//! 共享测试设施（仅测试构建编译）。
//!
//! 提供：
//! - `MockBackend`：按轮次脚本化返回 Delta（无网络），可统计模型调用次数
//! - `HangingBackend`：产出 1 个文本增量后永久挂起（测中途取消）
//! - `EchoTool`：可配置成功/失败的工具，记录收到的参数（测执行与回传）
//! - `SleepingBridge` / `PendingBridge`：权限桥的时序控制（测取消优先）
//! - 便捷构造器与事件断言

use std::path::PathBuf;
use std::sync::atomic::{AtomicBool, AtomicUsize, Ordering};
use std::sync::{Arc, Mutex};
use std::time::Duration;

use async_trait::async_trait;
use futures::StreamExt;
use serde_json::Value;
use tokio::sync::mpsc;
use tokio_util::sync::CancellationToken;

use crate::error::{Error, Result};
use crate::events::Envelope;
use crate::model::{
    ChatMessage, Delta, DeltaStream, MessageContent, ModelBackend, ModelRequest, Role,
};
use crate::permissions::{AutoBridge, Decision, PermissionBridge, PermissionRequest, ToolPolicy};
use crate::run::{execute, RunConfig};
use crate::tool::{AgentTool, ToolCall, ToolContext, ToolOutput, ToolRegistry};

// ===== Mock 模型后端 =====

/// 按调用次序返回脚本化轮次；脚本耗尽后返回 fallback 文本轮。
pub struct MockBackend {
    turns: Mutex<Vec<Vec<Delta>>>,
    pub call_count: AtomicUsize,
}

impl MockBackend {
    pub fn new(turns: Vec<Vec<Delta>>) -> Self {
        Self {
            turns: Mutex::new(turns),
            call_count: AtomicUsize::new(0),
        }
    }
}

#[async_trait]
impl ModelBackend for MockBackend {
    async fn stream_completion(&self, _req: ModelRequest) -> Result<DeltaStream> {
        let n = self.call_count.fetch_add(1, Ordering::SeqCst);
        let script = self
            .turns
            .lock()
            .unwrap()
            .get(n)
            .cloned()
            .unwrap_or_else(|| {
                vec![Delta::Text {
                    text: "fallback".into(),
                }]
            });
        Ok(Box::pin(futures::stream::iter(script.into_iter().map(Ok))))
    }
}

/// 产出 1 个文本增量后永久挂起（用于流式中途取消测试）
pub struct HangingBackend;

#[async_trait]
impl ModelBackend for HangingBackend {
    async fn stream_completion(&self, _req: ModelRequest) -> Result<DeltaStream> {
        let stream =
            futures::stream::once(async { Ok::<Delta, Error>(Delta::Text { text: "首".into() }) })
                .chain(futures::stream::pending::<Result<Delta>>());
        Ok(Box::pin(stream))
    }
}

// ===== 权限桥 =====

/// 延迟后返回固定决定，并记录是否被调用过（测取消优先/请求计数）
pub struct SleepingBridge {
    pub millis: u64,
    pub decision: Decision,
    pub called: AtomicBool,
}

impl SleepingBridge {
    pub fn new(millis: u64, decision: Decision) -> Self {
        Self {
            millis,
            decision,
            called: AtomicBool::new(false),
        }
    }

    /// 是否被请求过（测取消优先时验证桥未被执行）
    #[allow(dead_code)]
    pub fn was_called(&self) -> bool {
        self.called.load(Ordering::SeqCst)
    }
}

#[async_trait]
impl PermissionBridge for SleepingBridge {
    async fn request(&self, _req: PermissionRequest) -> Decision {
        self.called.store(true, Ordering::SeqCst);
        tokio::time::sleep(Duration::from_millis(self.millis)).await;
        self.decision
    }
}

/// 永不返回的桥（测已取消令牌下 select! 只能走取消分支）
pub struct PendingBridge;

#[async_trait]
impl PermissionBridge for PendingBridge {
    async fn request(&self, _req: PermissionRequest) -> Decision {
        std::future::pending().await
    }
}

// ===== 测试工具 =====

/// 可配置成功/失败的回声工具，记录每次收到的参数
pub struct EchoTool {
    pub name: String,
    pub output: std::result::Result<String, String>,
    pub log: Arc<Mutex<Vec<String>>>,
}

impl EchoTool {
    pub fn ok(name: &str, output: &str, log: Arc<Mutex<Vec<String>>>) -> Self {
        Self {
            name: name.to_string(),
            output: Ok(output.to_string()),
            log,
        }
    }

    pub fn failing(name: &str, error: &str, log: Arc<Mutex<Vec<String>>>) -> Self {
        Self {
            name: name.to_string(),
            output: Err(error.to_string()),
            log,
        }
    }

    /// 收到的全部参数记录（按调用顺序）
    #[allow(dead_code)]
    pub fn recorded(&self) -> Vec<String> {
        self.log.lock().unwrap().clone()
    }
}

#[async_trait]
impl AgentTool for EchoTool {
    fn name(&self) -> &str {
        &self.name
    }
    fn description(&self) -> &str {
        "测试用回声工具"
    }
    fn parameters_schema(&self) -> Value {
        serde_json::json!({
            "type": "object",
            "properties": { "path": { "type": "string" } },
            "required": ["path"]
        })
    }
    async fn call(&self, args: Value, _ctx: &ToolContext) -> Result<ToolOutput> {
        self.log.lock().unwrap().push(args.to_string());
        match &self.output {
            Ok(s) => Ok(ToolOutput::from(s.clone())),
            Err(e) => Err(Error::Tool {
                tool: self.name.clone(),
                message: e.clone(),
            }),
        }
    }
}

// ===== 便捷构造器 =====

pub fn user_msg(text: &str) -> ChatMessage {
    ChatMessage {
        role: Role::User,
        content: vec![MessageContent::text(text)],
    }
}

/// 单轮纯文本增量脚本
pub fn text_turn(text: &str) -> Vec<Delta> {
    vec![Delta::Text {
        text: text.to_string(),
    }]
}

/// 单轮工具调用增量脚本（参数固定 path=a.txt，满足 schema required）
pub fn tool_call_delta(id: &str, name: &str) -> Delta {
    Delta::ToolCalls {
        calls: vec![ToolCall {
            id: id.to_string(),
            call_id: None,
            name: name.to_string(),
            arguments: serde_json::json!({"path": "a.txt"}),
        }],
    }
}

pub fn text_config(session: &str, system: &str, policy: ToolPolicy) -> RunConfig {
    let mut config = RunConfig::new(session, system);
    config.policy = policy;
    config
}

/// 跑一次 execute 并同步收集全部事件（容量足够大，不会因未消费而死锁）
pub async fn run_execute(
    backend: Arc<dyn ModelBackend>,
    registry: ToolRegistry,
    config: RunConfig,
    project_root: Option<PathBuf>,
    bridge: Arc<dyn PermissionBridge>,
    messages: Vec<ChatMessage>,
) -> (Result<Vec<ChatMessage>>, Vec<Envelope>) {
    run_execute_with_cancel(
        backend,
        registry,
        config,
        project_root,
        bridge,
        messages,
        CancellationToken::new(),
    )
    .await
}

pub async fn run_execute_with_cancel(
    backend: Arc<dyn ModelBackend>,
    registry: ToolRegistry,
    config: RunConfig,
    project_root: Option<PathBuf>,
    bridge: Arc<dyn PermissionBridge>,
    messages: Vec<ChatMessage>,
    cancel: CancellationToken,
) -> (Result<Vec<ChatMessage>>, Vec<Envelope>) {
    let (tx, mut rx) = mpsc::channel(4096);
    let result = execute(
        backend,
        registry,
        config,
        project_root,
        bridge,
        messages,
        tx,
        cancel,
    )
    .await;
    let mut events = Vec::new();
    while let Some(env) = rx.recv().await {
        events.push(env);
    }
    (result, events)
}

// ===== 断言辅助 =====

pub fn assert_sequence_contiguous(envelopes: &[Envelope]) {
    assert!(!envelopes.is_empty(), "事件不应为空");
    for (i, env) in envelopes.iter().enumerate() {
        assert_eq!(
            env.sequence as usize,
            i + 1,
            "sequence 必须从 1 连续递增，第 {i} 个事件实际为 {}",
            env.sequence
        );
    }
}

/// 等待条件成立（每 10ms 轮询，超时 panic）
pub async fn wait_until(mut cond: impl FnMut() -> bool, what: &str) {
    for _ in 0..300 {
        if cond() {
            return;
        }
        tokio::time::sleep(Duration::from_millis(10)).await;
    }
    panic!("等待超时: {what}");
}

/// AutoBridge 的显式别名，测试里语义更清晰
pub fn allow_bridge() -> Arc<dyn PermissionBridge> {
    Arc::new(AutoBridge(Decision::Allow))
}

pub fn deny_bridge() -> Arc<dyn PermissionBridge> {
    Arc::new(AutoBridge(Decision::Deny))
}
