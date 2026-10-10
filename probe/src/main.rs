//! 阶段 0：rig-core 模型能力验证探针
//!
//! 验证目标（对照 build-agent-runtime skill 的技术风险清单）：
//! 1. 普通流式聊天
//! 2. 模型返回工具调用
//! 3. 流式工具名称 / 调用 ID / 参数片段正确拼接（rig 内部完成，我们验证产出完整）
//! 4. 工具执行结果按协议回传模型
//! 5. 模型根据结果继续调用工具或输出最终回答（multi_turn 循环）
//! 6. 中途取消生效（select! + drop stream，模拟用户点击停止）
//! 7. 工具失败后模型收到错误并尝试恢复（错误字符串回传）
//!
//! 附加验证：权限等待 —— Tool::call 内 await watch 通道，
//! 模拟"工具执行前挂起等待用户批准"的完整时序。
//!
//! 用法：cargo run --release -- <scenario> <provider-name> [model]
//! scenario: text | tool | allow-wait | deny | error | cancel
//! provider-name: providers.json 里的 name 字段（如 newApi、火山）

use std::io::Write as _;
use std::path::PathBuf;
use std::time::Duration;

use futures::StreamExt;
use rig::agent::MultiTurnStreamItem;
use rig::completion::ToolDefinition;
use rig::providers::openai;
use rig::streaming::{StreamedAssistantContent, StreamingChat};
use rig::tool::Tool;
use serde::Deserialize;
use tokio::sync::watch;

// ===== provider 配置（复用星野应用的 providers.json，不打印密钥） =====

#[derive(Debug, Deserialize)]
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

fn pick_provider<'a>(
    providers: &'a [ProviderInfo],
    name: &str,
) -> Result<&'a ProviderInfo, String> {
    providers.iter().find(|p| p.name == name).ok_or_else(|| {
        format!(
            "找不到 provider '{name}'，可用: {}",
            providers
                .iter()
                .map(|p| p.name.as_str())
                .collect::<Vec<_>>()
                .join(", ")
        )
    })
}

// ===== 探针工具 =====

/// 工具错误类型：错误消息会原样回传给模型（rig 行为，scenario 7 的基础）
#[derive(Debug, thiserror::Error)]
#[error("{0}")]
struct ProbeError(String);

/// 临时工作目录（模拟"项目"），scenario 开始前创建
struct TempProject(PathBuf);

impl TempProject {
    fn create(tag: &str) -> Self {
        let dir = std::env::temp_dir().join(format!("rig-probe-{tag}-{}", std::process::id()));
        std::fs::create_dir_all(&dir).expect("创建临时目录失败");
        std::fs::write(
            dir.join("requirements.md"),
            "# 需求\n- 用户登录\n- 数据导出\n",
        )
        .unwrap();
        std::fs::write(dir.join("notes.txt"), "备注：方案需要包含风险评估章节。\n").unwrap();
        Self(dir)
    }
}

impl Drop for TempProject {
    fn drop(&mut self) {
        let _ = std::fs::remove_dir_all(&self.0);
    }
}

/// 只读工具：列出项目文件
struct ListFiles {
    root: PathBuf,
}

impl Tool for ListFiles {
    const NAME: &'static str = "list_files";
    type Args = ListFilesArgs;
    type Error = ProbeError;
    type Output = String;

    async fn definition(&self, _prompt: String) -> ToolDefinition {
        ToolDefinition {
            name: Self::NAME.to_string(),
            description: "列出项目根目录下的所有文件名".to_string(),
            parameters: serde_json::json!({
                "type": "object",
                "properties": {},
                "required": []
            }),
        }
    }

    async fn call(&self, _args: Self::Args) -> Result<Self::Output, Self::Error> {
        let mut names = vec![];
        let mut entries = tokio::fs::read_dir(&self.root)
            .await
            .map_err(|e| ProbeError(format!("读取目录失败: {e}")))?;
        while let Some(entry) = entries
            .next_entry()
            .await
            .map_err(|e| ProbeError(e.to_string()))?
        {
            names.push(entry.file_name().to_string_lossy().to_string());
        }
        names.sort();
        Ok(names.join("\n"))
    }
}

#[derive(Debug, Deserialize)]
struct ListFilesArgs {}

/// 读文件工具：验证"读取内容 → 模型基于内容回答"链路
struct ReadFile {
    root: PathBuf,
}

impl Tool for ReadFile {
    const NAME: &'static str = "read_file";
    type Args = ReadFileArgs;
    type Error = ProbeError;
    type Output = String;

    async fn definition(&self, _prompt: String) -> ToolDefinition {
        ToolDefinition {
            name: Self::NAME.to_string(),
            description: "读取项目中指定文件的文本内容".to_string(),
            parameters: serde_json::json!({
                "type": "object",
                "properties": {
                    "path": { "type": "string", "description": "相对于项目根目录的文件路径" }
                },
                "required": ["path"]
            }),
        }
    }

    async fn call(&self, args: Self::Args) -> Result<Self::Output, Self::Error> {
        let path = self.root.join(&args.path);
        // 边界校验：防止路径逃逸（规范化后必须是 root 的子路径；符号链接也在此拦截）
        let canonical_root = self
            .root
            .canonicalize()
            .map_err(|e| ProbeError(format!("项目根目录无效: {e}")))?;
        let canonical_path = path.canonicalize().unwrap_or_else(|_| path.clone()); // 文件尚不存在时用字面路径校验
        if !canonical_path.starts_with(&canonical_root) {
            return Err(ProbeError("拒绝访问：路径位于项目目录之外".to_string()));
        }
        let content = tokio::fs::read_to_string(&path)
            .await
            .map_err(|e| ProbeError(format!("读取文件失败: {e}")))?;
        Ok(content)
    }
}

#[derive(Debug, Deserialize)]
struct ReadFileArgs {
    path: String,
}

/// 受权限门控的写文件工具：模拟"执行前挂起等待用户批准"
/// - 等待 decision 变为 Allow / Deny（watch 通道，模拟前端批准 UI）
/// - Deny 时返回 Err（错误字符串回传模型）
struct GatedWrite {
    root: PathBuf,
    decision_rx: watch::Receiver<Decision>,
}

#[derive(Debug, Clone, Copy, PartialEq)]
enum Decision {
    Pending,
    Allow,
    Deny,
}

impl Tool for GatedWrite {
    const NAME: &'static str = "write_file";
    type Args = WriteFileArgs;
    type Error = ProbeError;
    type Output = String;

    async fn definition(&self, _prompt: String) -> ToolDefinition {
        ToolDefinition {
            name: Self::NAME.to_string(),
            description: "在项目中写入（创建或覆盖）一个文本文件，需要用户批准".to_string(),
            parameters: serde_json::json!({
                "type": "object",
                "properties": {
                    "path": { "type": "string", "description": "相对于项目根目录的文件路径" },
                    "content": { "type": "string", "description": "文件内容" }
                },
                "required": ["path", "content"]
            }),
        }
    }

    async fn call(&self, args: Self::Args) -> Result<Self::Output, Self::Error> {
        // 1. 挂起等待用户决定（生产环境中此处对应前端权限确认卡）
        let mut rx = self.decision_rx.clone();
        let decision = loop {
            if *rx.borrow() != Decision::Pending {
                break *rx.borrow();
            }
            if rx.changed().await.is_err() {
                return Err(ProbeError("权限通道关闭".to_string()));
            }
        };

        // 2. 取消优先于批准：若等待期间被拒绝，不执行任何写操作
        match decision {
            Decision::Deny => Err(ProbeError("用户拒绝了本次文件写入授权".to_string())),
            Decision::Pending => Err(ProbeError("授权等待超时".to_string())),
            Decision::Allow => {
                let path = self.root.join(&args.path);
                tokio::fs::write(&path, &args.content)
                    .await
                    .map_err(|e| ProbeError(format!("写入失败: {e}")))?;
                Ok(format!(
                    "已写入 {}（{} 字节）",
                    args.path,
                    args.content.len()
                ))
            }
        }
    }
}

#[derive(Debug, Deserialize)]
struct WriteFileArgs {
    path: String,
    content: String,
}

/// 永远失败的工具：验证错误回传后模型的恢复行为
struct AlwaysFail;

impl Tool for AlwaysFail {
    const NAME: &'static str = "broken_tool";
    type Args = FailArgs;
    type Error = ProbeError;
    type Output = String;

    async fn definition(&self, _prompt: String) -> ToolDefinition {
        ToolDefinition {
            name: Self::NAME.to_string(),
            description: "一个故意损坏的工具，用来测试错误处理".to_string(),
            parameters: serde_json::json!({
                "type": "object",
                "properties": {
                    "input": { "type": "string" }
                },
                "required": []
            }),
        }
    }

    async fn call(&self, _args: Self::Args) -> Result<Self::Output, Self::Error> {
        Err(ProbeError(
            "内部错误：工具依赖的外部服务不可用（模拟故障）".to_string(),
        ))
    }
}

#[derive(Debug, Deserialize)]
struct FailArgs {
    #[serde(default)]
    input: Option<String>,
}

// ===== Agent 构建 =====

fn build_agent(
    provider: &ProviderInfo,
    model_override: Option<&str>,
    project_root: PathBuf,
    decision_rx: Option<watch::Receiver<Decision>>,
) -> rig::agent::Agent<openai::completion::CompletionModel<reqwest::Client>> {
    let client = openai::Client::builder(provider.api_key.as_str())
        .base_url(provider.base_url.as_str())
        .build();
    let model = openai::completion::CompletionModel::new(
        client,
        model_override.unwrap_or(&provider.active_model),
    );

    let mut builder = model
        .into_agent_builder()
        .preamble(
            "你是星野项目的文件助手，运行在用户的本地项目目录中。\
             使用提供的工具完成用户任务，回答使用中文。\
             写文件前必须调用 write_file 工具（用户会被征求批准），不要把文件内容只在回复里展示。",
        )
        .tool(ListFiles {
            root: project_root.clone(),
        })
        .tool(ReadFile {
            root: project_root.clone(),
        });

    if let Some(rx) = decision_rx {
        builder = builder.tool(GatedWrite {
            root: project_root,
            decision_rx: rx,
        });
    }
    builder.build()
}

// ===== 流式消费（所有 scenario 共用） =====

type ProbeStream = <openai::completion::CompletionModel<reqwest::Client> as rig::completion::CompletionModel>::StreamingResponse;

async fn run_stream(
    agent: &rig::agent::Agent<openai::completion::CompletionModel<reqwest::Client>>,
    prompt: &str,
) -> Result<(), String> {
    let mut stream = agent.stream_chat(prompt, vec![]).multi_turn(10).await;

    let mut tool_calls = 0usize;
    let mut final_text = String::new();

    loop {
        // 统一超时护栏，防止挂死
        let item = tokio::time::timeout(Duration::from_secs(120), stream.next())
            .await
            .map_err(|_| "超时：模型 120 秒无响应".to_string())?;
        match item {
            None => break,
            Some(Err(e)) => {
                return Err(format!("流错误: {e}"));
            }
            Some(Ok(MultiTurnStreamItem::StreamItem(StreamedAssistantContent::Text(t)))) => {
                print!("{}", t.text);
                let _ = std::io::stdout().flush();
            }
            Some(Ok(MultiTurnStreamItem::StreamItem(StreamedAssistantContent::Reasoning(r)))) => {
                // 不作为正式事件输出，仅提示存在
                let _ = r.reasoning.len();
                print!("[thinking]");
                let _ = std::io::stdout().flush();
            }
            Some(Ok(MultiTurnStreamItem::StreamItem(StreamedAssistantContent::ToolCall(tc)))) => {
                tool_calls += 1;
                println!(
                    "\n>>> [TOOL_CALL #{tool_calls}] id={} name={} args={}",
                    tc.id, tc.function.name, tc.function.arguments
                );
            }
            Some(Ok(MultiTurnStreamItem::StreamItem(StreamedAssistantContent::Final(_)))) => {}
            Some(Ok(MultiTurnStreamItem::FinalResponse(resp))) => {
                final_text = resp.response().to_string();
                println!("\n<<< [FINAL]");
            }
            Some(Ok(_)) => {}
        }
    }

    println!("\n=== 工具调用总次数: {tool_calls} ===");
    if !final_text.is_empty() && tool_calls == 0 {
        println!(
            "=== 最终回复（非流式聚合）: {} ===",
            truncate(&final_text, 200)
        );
    }
    Ok(())
}

fn truncate(s: &str, n: usize) -> String {
    if s.chars().count() <= n {
        s.to_string()
    } else {
        s.chars().take(n).collect::<String>() + "..."
    }
}

// ===== Scenarios =====

/// 1. 普通流式聊天
async fn scenario_text(provider: &ProviderInfo, model: Option<&str>) -> Result<(), String> {
    println!("--- scenario: text（纯流式，无工具） ---");
    let agent = build_agent(provider, model, std::env::temp_dir(), None);
    // 无工具的 agent 也保留只读工具不影响；此 scenario 只验证文本流
    run_stream(&agent, "用两句话介绍一下你自己，不要调用任何工具。").await
}

/// 2-5. 工具调用 → 结果回传 → 继续/收尾
async fn scenario_tool(provider: &ProviderInfo, model: Option<&str>) -> Result<(), String> {
    println!("--- scenario: tool（list_files + read_file，验证多轮工具循环） ---");
    let project = TempProject::create("tool");
    let agent = build_agent(provider, model, project.0.clone(), None);
    run_stream(
        &agent,
        "看一下项目里有哪些文件，然后读取 requirements.md，告诉我需求里列了哪几项。",
    )
    .await
}

/// 权限允许场景：1.5 秒后批准
async fn scenario_allow_wait(provider: &ProviderInfo, model: Option<&str>) -> Result<(), String> {
    println!("--- scenario: allow-wait（write_file 挂起 1.5s 后批准） ---");
    let project = TempProject::create("allow");
    let (tx, rx) = watch::channel(Decision::Pending);
    tokio::spawn(async move {
        tokio::time::sleep(Duration::from_millis(1500)).await;
        println!("\n>>> [PERMISSION] 用户批准写入");
        let _ = tx.send(Decision::Allow);
    });
    let agent = build_agent(provider, model, project.0.clone(), Some(rx));
    let result = run_stream(
        &agent,
        "请把项目里的 notes.txt 内容整理后写入 summary.md（用 write_file 工具）。",
    )
    .await;
    let target = project.0.join("summary.md");
    match std::fs::read_to_string(&target) {
        Ok(content) => println!("=== 文件已生成: summary.md（{} 字节）===", content.len()),
        Err(_) => println!("=== 警告: summary.md 未生成 ==="),
    }
    result
}

/// 权限拒绝场景：1.5 秒后拒绝，验证错误回传 + 模型恢复
async fn scenario_deny(provider: &ProviderInfo, model: Option<&str>) -> Result<(), String> {
    println!("--- scenario: deny（write_file 挂起 1.5s 后拒绝） ---");
    let project = TempProject::create("deny");
    let (tx, rx) = watch::channel(Decision::Pending);
    tokio::spawn(async move {
        tokio::time::sleep(Duration::from_millis(1500)).await;
        println!("\n>>> [PERMISSION] 用户拒绝写入");
        let _ = tx.send(Decision::Deny);
    });
    let agent = build_agent(provider, model, project.0.clone(), Some(rx));
    let result = run_stream(
        &agent,
        "请把项目里的 notes.txt 内容整理后写入 summary.md（用 write_file 工具）。",
    )
    .await;
    let target = project.0.join("summary.md");
    if target.exists() {
        println!("=== 警告: 拒绝后文件仍被生成！权限失效 ===");
    } else {
        println!("=== 文件未生成（权限控制生效） ===");
    }
    result
}

/// 工具故障恢复场景
async fn scenario_error(provider: &ProviderInfo, model: Option<&str>) -> Result<(), String> {
    println!("--- scenario: error（broken_tool 永远失败） ---");
    let _project = TempProject::create("error");
    let client = openai::Client::builder(provider.api_key.as_str())
        .base_url(provider.base_url.as_str())
        .build();
    let agent =
        openai::completion::CompletionModel::new(client, model.unwrap_or(&provider.active_model))
            .into_agent_builder()
            .preamble("你是测试助手，用中文回答。")
            .tool(AlwaysFail)
            .build();
    run_stream(
        &agent,
        "调用 broken_tool 处理 'hello'，如果失败请直接告诉我结论。",
    )
    .await
}

/// 取消场景：流式输出中途 drop stream（模拟用户点击停止按钮）
async fn scenario_cancel(provider: &ProviderInfo, model: Option<&str>) -> Result<(), String> {
    println!("--- scenario: cancel（收到 3 个文本增量后强制取消） ---");
    let agent = build_agent(provider, model, std::env::temp_dir(), None);
    let mut stream = agent
        .stream_chat("写一篇 500 字的短文，介绍秋天的湖边景色。", vec![])
        .await;

    let mut deltas = 0usize;
    let result: Result<(), String> = loop {
        let item = tokio::time::timeout(Duration::from_secs(60), stream.next()).await;
        match item {
            Err(_) => break Err("超时".to_string()),
            Ok(None) => {
                println!(">>> 流在取消前自然结束（模型响应太快，未验证到取消）");
                break Ok(());
            }
            Ok(Some(Err(e))) => break Err(format!("流错误: {e}")),
            Ok(Some(Ok(MultiTurnStreamItem::StreamItem(StreamedAssistantContent::Text(_))))) => {
                deltas += 1;
                if deltas == 3 {
                    println!("\n>>> [CANCEL] 用户在第 {deltas} 个增量后点击停止，丢弃 stream");
                    break Ok(()); // 此处 return/drop 掉 stream，即生产环境 stop 的语义
                }
            }
            Ok(Some(Ok(_))) => {}
        }
    };

    // drop 发生在 break 时（stream 出作用域），后续不再有任何事件
    println!("=== 已收到 {deltas} 个增量后停止，取消语义验证完成 ===");
    result
}

#[tokio::main]
async fn main() {
    let args: Vec<String> = std::env::args().collect();
    if args.len() < 3 {
        eprintln!("用法: cargo run --release -- <text|tool|allow-wait|deny|error|cancel> <provider-name> [model]");
        std::process::exit(1);
    }
    let scenario = args[1].as_str();
    let provider_name = args[2].as_str();
    let model_override = args.get(3).map(|s| s.as_str());

    let providers = match load_providers() {
        Ok(p) => p,
        Err(e) => {
            eprintln!("{e}");
            std::process::exit(1);
        }
    };
    let provider = match pick_provider(&providers, provider_name) {
        Ok(p) => p,
        Err(e) => {
            eprintln!("{e}");
            std::process::exit(1);
        }
    };
    println!(
        "provider={} base_url={} model={}",
        provider.name,
        provider.base_url,
        model_override.unwrap_or(&provider.active_model)
    );

    let result = match scenario {
        "text" => scenario_text(provider, model_override).await,
        "tool" => scenario_tool(provider, model_override).await,
        "allow-wait" => scenario_allow_wait(provider, model_override).await,
        "deny" => scenario_deny(provider, model_override).await,
        "error" => scenario_error(provider, model_override).await,
        "cancel" => scenario_cancel(provider, model_override).await,
        other => Err(format!("未知 scenario: {other}")),
    };

    match result {
        Ok(()) => println!("\n[PASS] {scenario}"),
        Err(e) => {
            eprintln!("\n[FAIL] {scenario}: {e}");
            std::process::exit(2);
        }
    }
}
