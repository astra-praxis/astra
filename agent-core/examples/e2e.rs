//! agent-core 端到端探针（阶段 1 验收）。
//!
//! 用法：cargo run --release --example e2e -- <scenario> [provider] [model]
//! 场景：
//!   text       纯文本流式（无工具）
//!   tool       工具调用 + 结果回传（验证工具调用在事件流中**可见**）
//!   allow      Confirm 工具自动批准（AutoBridge(Allow)，文件应生成）
//!   deny       Confirm 工具自动拒绝（AutoBridge(Deny)，文件不应生成，模型优雅恢复）
//!   cancel     流式输出中途取消
//!
//! provider 默认「火山」（内网 newApi 可能不通），读Astra应用真实 providers.json。

use agent_core::events::RunEvent;
use agent_core::model::{ChatMessage, MessageContent, ModelBackend, RigBackend, Role};
use agent_core::permissions::{AutoBridge, Decision, ToolPolicy};
use agent_core::run::{execute, RunConfig};
use agent_core::tool::{AgentTool, ToolContext, ToolOutput, ToolRegistry};
use serde_json::Value;
use std::path::PathBuf;
use std::sync::Arc;
use std::time::Duration;
use tokio::sync::mpsc;
use tokio_util::sync::CancellationToken;

#[derive(Debug, serde::Deserialize)]
struct ProviderInfo {
    name: String,
    base_url: String,
    api_key: String,
    active_model: String,
}

fn load_providers() -> Result<Vec<ProviderInfo>, String> {
    let home = std::env::var("HOME").map_err(|_| "无法获取 HOME".to_string())?;
    let path = PathBuf::from(home).join("Library/Application Support/com.astra.app/providers.json");
    let content =
        std::fs::read_to_string(&path).map_err(|e| format!("读取 providers.json 失败: {e}"))?;
    serde_json::from_str(&content).map_err(|e| format!("解析 providers.json 失败: {e}"))
}

// ===== 探针工具 =====

struct ReadFile {
    root: PathBuf,
}

#[async_trait::async_trait]
impl AgentTool for ReadFile {
    fn name(&self) -> &str {
        "read_file"
    }
    fn description(&self) -> &str {
        "读取项目内的文本文件"
    }
    fn parameters_schema(&self) -> Value {
        serde_json::json!({
            "type": "object",
            "properties": { "path": { "type": "string", "description": "相对项目根的路径" } },
            "required": ["path"]
        })
    }
    async fn call(&self, args: Value, ctx: &ToolContext) -> agent_core::Result<ToolOutput> {
        let path =
            args.get("path")
                .and_then(|v| v.as_str())
                .ok_or_else(|| agent_core::Error::Tool {
                    tool: "read_file".into(),
                    message: "缺少 path".into(),
                })?;
        let full = agent_core::ensure_within_root(&self.root, &PathBuf::from(path))?;
        let _ = &ctx.cancelled; // 示意：耗时工具应监听取消
        let content = tokio::fs::read_to_string(self.root.join(full))
            .await
            .map_err(|e| agent_core::Error::Tool {
                tool: "read_file".into(),
                message: e.to_string(),
            })?;
        Ok(ToolOutput::from(content))
    }
}

struct WriteFile {
    root: PathBuf,
}

#[async_trait::async_trait]
impl AgentTool for WriteFile {
    fn name(&self) -> &str {
        "write_file"
    }
    fn description(&self) -> &str {
        "写入文本文件（相对项目根）"
    }
    fn parameters_schema(&self) -> Value {
        serde_json::json!({
            "type": "object",
            "properties": {
                "path": { "type": "string" },
                "content": { "type": "string" }
            },
            "required": ["path", "content"]
        })
    }
    async fn call(&self, args: Value, ctx: &ToolContext) -> agent_core::Result<ToolOutput> {
        let path = args.get("path").and_then(|v| v.as_str()).unwrap_or("");
        let content = args.get("content").and_then(|v| v.as_str()).unwrap_or("");
        let full = agent_core::ensure_within_root(&self.root, &PathBuf::from(path))?;
        let _ = &ctx.cancelled;
        tokio::fs::write(self.root.join(&full), content)
            .await
            .map_err(|e| agent_core::Error::Tool {
                tool: "write_file".into(),
                message: e.to_string(),
            })?;
        Ok(ToolOutput::with_artifacts(
            format!("已写入 {path}（{} 字节）", content.len()),
            vec![
                agent_core::Artifact::new(agent_core::ArtifactKind::FileChange, path)
                    .with_metadata(serde_json::json!({
                        "mode": "overwrite",
                        "bytes": content.len(),
                    })),
            ],
        ))
    }
}

// ===== 事件打印 =====

fn print_event(seq: u64, ev: &RunEvent) {
    match ev {
        RunEvent::MessageDelta { text } => print!("{text}"),
        RunEvent::ThinkingDelta { text } => print!("{text}"),
        RunEvent::ToolCallArgsDelta {
            tool_name,
            fragment,
            ..
        } => {
            if !fragment.is_empty() {
                print!("[seq {seq}] {tool_name} 参数流: {fragment}")
            }
        }
        RunEvent::StatusChanged { status } => println!("\n[seq {seq}] 状态 → {status:?}"),
        RunEvent::ToolCallStarted {
            tool_name,
            args_summary,
            ..
        } => println!("[seq {seq}] 工具开始: {tool_name}({args_summary})"),
        RunEvent::ToolCallCompleted {
            tool_name,
            result_summary,
            artifacts,
            ..
        } => println!(
            "[seq {seq}] 工具完成: {tool_name} → {result_summary}{}",
            if artifacts.is_empty() {
                String::new()
            } else {
                format!(
                    " (产物: {:?})",
                    artifacts
                        .iter()
                        .map(|a| format!("{} ({:?})", a.location, a.kind))
                        .collect::<Vec<_>>()
                )
            }
        ),
        RunEvent::ToolCallFailed {
            tool_name, error, ..
        } => println!("[seq {seq}] 工具失败: {tool_name} → {error}"),
        RunEvent::PermissionRequested {
            tool_name, summary, ..
        } => println!("[seq {seq}] 等待确认: {tool_name}({summary})"),
        RunEvent::PermissionDecided { decision, .. } => {
            println!("[seq {seq}] 权限决定: {decision:?}")
        }
        RunEvent::RunCompleted { final_text } => {
            println!("\n[seq {seq}] ✓ Run 完成，最终回复 {} 字", final_text.len())
        }
        RunEvent::RunFailed { error } => println!("\n[seq {seq}] ✗ Run 失败: {error}"),
        RunEvent::RunCancelled => println!("\n[seq {seq}] ⏹ Run 已取消"),
        RunEvent::PlanUpdated { plan } => {
            println!("[seq {seq}] 计划更新: {} 步", plan.steps.len())
        }
        RunEvent::CompactionCompleted => {
            println!("[seq {seq}] 上下文已压缩")
        }
    }
}

async fn drain_events(mut rx: mpsc::Receiver<agent_core::Envelope>) {
    while let Some(env) = rx.recv().await {
        print_event(env.sequence, &env.event);
    }
}

struct TempProject(PathBuf);
impl TempProject {
    fn new(tag: &str) -> Self {
        let dir = std::env::temp_dir().join(format!("agent-core-e2e-{tag}"));
        let _ = std::fs::remove_dir_all(&dir);
        std::fs::create_dir_all(&dir).unwrap();
        Self(dir)
    }
}
impl Drop for TempProject {
    fn drop(&mut self) {
        let _ = std::fs::remove_dir_all(&self.0);
    }
}

#[tokio::main]
async fn main() {
    let args: Vec<String> = std::env::args().collect();
    let scenario = args.get(1).map(|s| s.as_str()).unwrap_or("text");
    let provider_name = args.get(2).map(|s| s.as_str()).unwrap_or("火山");
    let model_override = args.get(3).map(|s| s.as_str());

    let providers = load_providers().expect("providers.json");
    let p = providers
        .iter()
        .find(|p| p.name == provider_name)
        .unwrap_or_else(|| panic!("找不到 provider {provider_name}"));
    let model = model_override.unwrap_or(&p.active_model).to_string();
    println!("=== 场景 {scenario} | provider={provider_name} model={model} ===");

    let backend: Arc<dyn ModelBackend> = Arc::new(RigBackend::new(
        p.base_url.clone(),
        p.api_key.clone(),
        model,
    ));

    match scenario {
        "text" => scenario_text(backend).await,
        "tool" => scenario_tool(backend).await,
        "allow" => scenario_confirm(backend, Decision::Allow).await,
        "deny" => scenario_confirm(backend, Decision::Deny).await,
        "cancel" => scenario_cancel(backend).await,
        other => panic!("未知场景: {other}"),
    }
}

async fn scenario_text(backend: Arc<dyn ModelBackend>) {
    let (tx, rx) = mpsc::channel(256);
    let drain = tokio::spawn(drain_events(rx));
    let mut config = RunConfig::new("s1", "你是简洁的中文助手。");
    config.policy = ToolPolicy {
        allowed_readonly: vec![],
        confirm_tools: vec![],
    };
    let token = CancellationToken::new();
    let messages = vec![ChatMessage {
        role: Role::User,
        content: vec![MessageContent::text("用两句话介绍秋天的湖边景色。")],
    }];
    let result = execute(
        backend,
        ToolRegistry::new(),
        config,
        None,
        Arc::new(AutoBridge(Decision::Deny)),
        messages,
        tx,
        token,
    )
    .await;
    match &result {
        Ok(m) => println!("\n[消息序列 {} 条]", m.len()),
        Err(e) => println!("\n[失败] {e}"),
    }
    drain.await.unwrap();
}

async fn scenario_tool(backend: Arc<dyn ModelBackend>) {
    let tmp = TempProject::new("tool");
    std::fs::write(
        tmp.0.join("需求.txt"),
        "项目要求：输出一份三行以内的产品口号。",
    )
    .unwrap();

    let registry = ToolRegistry::new()
        .register(ReadFile {
            root: tmp.0.clone(),
        })
        .register(WriteFile {
            root: tmp.0.clone(),
        });
    println!("注册工具: {:?}", registry.names());

    let (tx, rx) = mpsc::channel(256);
    let drain = tokio::spawn(drain_events(rx));
    let mut config = RunConfig::new("s2", "你是项目助手，只能操作项目内文件。回答保持简短。");
    config.policy = ToolPolicy::default();
    let token = CancellationToken::new();
    let messages = vec![ChatMessage {
        role: Role::User,
        content: vec![MessageContent::text(
            "读取项目里的「需求.txt」，按需求把口号写入「口号.txt」，然后告诉我口号是什么。",
        )],
    }];
    let result = execute(
        backend,
        registry,
        config,
        Some(tmp.0.clone()),
        Arc::new(AutoBridge(Decision::Allow)),
        messages,
        tx,
        token,
    )
    .await;
    match &result {
        Ok(m) => println!("\n[消息序列 {} 条]", m.len()),
        Err(e) => println!("\n[失败] {e}"),
    }
    drain.await.unwrap();
    let written = std::fs::read_to_string(tmp.0.join("口号.txt"));
    println!(
        "[验证] 口号.txt: {}",
        written
            .map(|c| format!("{} 行", c.lines().count()))
            .unwrap_or("不存在".into())
    );
}

async fn scenario_confirm(backend: Arc<dyn ModelBackend>, decision: Decision) {
    let tmp = TempProject::new(match decision {
        Decision::Allow => "allow",
        _ => "deny",
    });
    let registry = ToolRegistry::new().register(WriteFile {
        root: tmp.0.clone(),
    });
    let (tx, rx) = mpsc::channel(256);
    let drain = tokio::spawn(drain_events(rx));
    let mut config = RunConfig::new("s3", "你是文件助手。");
    config.policy = ToolPolicy::default(); // write_file = Confirm
    let token = CancellationToken::new();
    let messages = vec![ChatMessage {
        role: Role::User,
        content: vec![MessageContent::text(
            "把「agent-core 权限探针运行成功」写入 result.txt。",
        )],
    }];
    let result = execute(
        backend,
        registry,
        config,
        Some(tmp.0.clone()),
        Arc::new(AutoBridge(decision)),
        messages,
        tx,
        token,
    )
    .await;
    match &result {
        Ok(m) => println!("\n[消息序列 {} 条]", m.len()),
        Err(e) => println!("\n[失败] {e}"),
    }
    drain.await.unwrap();
    let exists = tmp.0.join("result.txt").exists();
    println!(
        "[验证] result.txt {}（预期: {}）",
        if exists { "已生成" } else { "未生成" },
        if decision == Decision::Allow {
            "已生成"
        } else {
            "未生成"
        }
    );
}

async fn scenario_cancel(backend: Arc<dyn ModelBackend>) {
    let (tx, mut rx) = mpsc::channel(256);
    let token = CancellationToken::new();
    let mut config = RunConfig::new("s4", "你是写作助手。");
    config.policy = ToolPolicy {
        allowed_readonly: vec![],
        confirm_tools: vec![],
    };
    let messages = vec![ChatMessage {
        role: Role::User,
        content: vec![MessageContent::text(
            "写一篇 800 字的短文，介绍湖边的四季。",
        )],
    }];

    let mut runner = tokio::spawn(execute(
        backend,
        ToolRegistry::new(),
        config,
        None,
        Arc::new(AutoBridge(Decision::Deny)),
        messages,
        tx.clone(),
        token.clone(),
    ));

    // 收到 3 个文本增量后取消
    let mut deltas = 0;
    let mut cancelled_seen = false;
    let mut runner_done = false;
    let deadline = tokio::time::Instant::now() + Duration::from_secs(90);
    loop {
        tokio::select! {
            _ = tokio::time::sleep_until(deadline) => {
                println!("[超时] 未能在 90s 内观察到取消");
                token.cancel();
                break;
            }
            env = rx.recv() => match env {
                Some(env) => {
                    if matches!(env.event, RunEvent::MessageDelta { .. }) {
                        deltas += 1;
                        if deltas == 3 {
                            println!("\n[触发] 第 3 个增量 → cancel()");
                            token.cancel();
                        }
                    }
                    if matches!(env.event, RunEvent::RunCancelled) {
                        cancelled_seen = true;
                    }
                    print_event(env.sequence, &env.event);
                }
                None => break,
            },
            result = &mut runner, if !runner_done => {
                match result {
                    Ok(Ok(m)) => println!("\n[Run 返回] 正常完成 {} 条消息", m.len()),
                    Ok(Err(e)) => println!("\n[Run 返回] Err({e})"),
                    Err(e) => println!("\n[Join 错误] {e}"),
                }
                runner_done = true;
                break;
            }
        }
    }
    if !runner_done {
        let _ = runner.await;
    }
    println!(
        "[验证] 3 个增量后取消 → RunCancelled 事件{}",
        if cancelled_seen {
            " ✓"
        } else {
            " ✗（未观察到）"
        }
    );
}
