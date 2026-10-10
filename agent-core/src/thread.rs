//! Thread：长期会话层。
//!
//! 职责（对齐 skill 双层线程模型）：
//! - 持有消息序列（面向 LLM 的上下文）与项目绑定上下文
//! - `send()` 启动一次 Run；旧 Run 先 cancel 再开新 Run
//! - UI 只消费事件流（Envelope）与 Thread 的消息快照，不依赖具体后端
//!
//! Phase 1 最小实现：不做持久化（阶段 4）、不做上下文压缩（阶段 5）。

use std::path::PathBuf;
use std::sync::{Arc, Mutex};

use tokio::sync::mpsc;
use tokio_util::sync::CancellationToken;

use crate::events::Envelope;
use crate::model::{ChatMessage, MessageContent, ModelBackend, Role};
use crate::permissions::{PermissionBridge, ToolPolicy};
use crate::run::{execute, RunConfig, RunHandle, DEFAULT_AUTO_COMPACT_TOKENS, DEFAULT_MAX_TURNS};
use crate::tool::ToolRegistry;

pub struct ThreadConfig {
    pub session_id: String,
    pub system_prompt: String,
    /// None = 纯聊天模式（不注册文件工具的空注册表由调用方控制）
    pub project_root: Option<PathBuf>,
    pub temperature: Option<f32>,
    pub max_tokens: Option<u32>,
    /// 最大续轮数（防无限工具循环预算），默认 DEFAULT_MAX_TURNS
    pub max_turns: u32,
    /// 上下文自动压缩阈值（估算 token 数）；None = 关闭
    pub auto_compact_tokens: Option<u64>,
    pub policy: ToolPolicy,
}

impl ThreadConfig {
    pub fn new(session_id: impl Into<String>, system_prompt: impl Into<String>) -> Self {
        Self {
            session_id: session_id.into(),
            system_prompt: system_prompt.into(),
            project_root: None,
            temperature: None,
            max_tokens: None,
            max_turns: DEFAULT_MAX_TURNS,
            auto_compact_tokens: Some(DEFAULT_AUTO_COMPACT_TOKENS),
            policy: ToolPolicy::default(),
        }
    }
}

pub struct Thread {
    config: ThreadConfig,
    backend: Arc<dyn ModelBackend>,
    registry: ToolRegistry,
    bridge: Arc<dyn PermissionBridge>,
    messages: Arc<Mutex<Vec<ChatMessage>>>,
    active: Mutex<Option<CancellationToken>>,
}

impl Thread {
    pub fn new(
        config: ThreadConfig,
        backend: Arc<dyn ModelBackend>,
        registry: ToolRegistry,
        bridge: Arc<dyn PermissionBridge>,
    ) -> Self {
        Self {
            config,
            backend,
            registry,
            bridge,
            messages: Arc::new(Mutex::new(Vec::new())),
            active: Mutex::new(None),
        }
    }

    /// 发送用户输入并启动一次 Run（立即返回，事件经 event_tx 流出）。
    /// 旧的进行中 Run 会被先取消（竞态规则：取消优先）。
    pub fn send(
        &self,
        user_text: impl Into<String>,
        event_tx: mpsc::Sender<Envelope>,
    ) -> RunHandle {
        // 1. 取消旧 Run（竞态规则：取消优先）
        if let Some(old) = self.active.lock().unwrap().take() {
            tracing::info!(session_id = %self.config.session_id, "取消进行中的旧 Run");
            old.cancel();
        }

        // 2. 用户消息入序列
        {
            let mut msgs = self.messages.lock().unwrap();
            msgs.push(ChatMessage {
                role: Role::User,
                content: vec![MessageContent::text(user_text)],
            });
        }
        let snapshot = self.messages.lock().unwrap().clone();

        // 3. 组装 Run 并 spawn
        let token = CancellationToken::new();
        *self.active.lock().unwrap() = Some(token.clone());

        let mut config = RunConfig::new(&self.config.session_id, &self.config.system_prompt);
        config.temperature = self.config.temperature;
        config.max_tokens = self.config.max_tokens;
        config.max_turns = self.config.max_turns;
        config.auto_compact_tokens = self.config.auto_compact_tokens;
        config.policy = self.config.policy.clone();

        let backend = Arc::clone(&self.backend);
        let registry = self.registry.clone();
        let bridge = Arc::clone(&self.bridge);
        let project_root = self.config.project_root.clone();
        let messages_arc = Arc::clone(&self.messages);
        let spawn_token = token.clone();

        tokio::spawn(async move {
            match execute(
                backend,
                registry,
                config,
                project_root,
                bridge,
                snapshot,
                event_tx,
                spawn_token,
            )
            .await
            {
                Ok(updated) => {
                    // 成功：以 Run 产出的完整序列为准（含助手消息与工具结果）
                    *messages_arc.lock().unwrap() = updated;
                }
                Err(_) => {
                    // 失败/取消：保留已入序列的部分（用户输入、已完成的工具调用）。
                    // 中断点之后的恢复由阶段 4 持久化处理。
                }
            }
        });

        RunHandle::from_token(token)
    }

    /// 当前消息序列快照（供持久化与 UI 恢复）
    pub fn messages(&self) -> Vec<ChatMessage> {
        self.messages.lock().unwrap().clone()
    }

    pub fn history_len(&self) -> usize {
        self.messages.lock().unwrap().len()
    }

    pub fn session_id(&self) -> &str {
        &self.config.session_id
    }

    pub fn project_root(&self) -> Option<&PathBuf> {
        self.config.project_root.as_ref()
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::model::Delta;
    use crate::permissions::{AutoBridge, Decision, ToolPolicy};
    use crate::test_support::{wait_until, HangingBackend, MockBackend};
    use crate::tool::ToolRegistry as R;
    use async_trait::async_trait;
    use std::time::Duration;

    fn deny_policy() -> ToolPolicy {
        ToolPolicy {
            allowed_readonly: vec![],
            confirm_tools: vec![],
        }
    }

    fn make_thread(
        turns: Vec<Vec<Delta>>,
    ) -> (Thread, mpsc::Sender<Envelope>, mpsc::Receiver<Envelope>) {
        let backend: Arc<dyn ModelBackend> = Arc::new(MockBackend::new(turns));
        let mut config = ThreadConfig::new("t1", "系统提示");
        config.policy = deny_policy();
        let thread = Thread::new(
            config,
            backend,
            R::new(),
            Arc::new(AutoBridge(Decision::Deny)),
        );
        let (tx, rx) = mpsc::channel(4096);
        (thread, tx, rx)
    }

    #[tokio::test]
    async fn send_accumulates_messages_after_run_completes() {
        let (thread, tx, mut rx) = make_thread(vec![vec![Delta::Text {
            text: "回复".into(),
        }]]);
        let _handle = thread.send("问题", tx);
        wait_until(|| thread.history_len() == 2, "等待 Run 写回消息序列").await;

        let messages = thread.messages();
        assert!(matches!(&messages[0].role, Role::User));
        assert!(matches!(
            &messages[1].content[0],
            MessageContent::Text { text } if text == "回复"
        ));
        // 排干事件通道（spawn 的 execute 持有 sender，直到返回）
        while rx.recv().await.is_some() {}
    }

    #[tokio::test]
    async fn send_twice_runs_sequential_turns() {
        let (thread, tx, mut rx) = make_thread(vec![
            vec![Delta::Text {
                text: "一答".into(),
            }],
            vec![Delta::Text {
                text: "二答".into(),
            }],
        ]);
        let _h1 = thread.send("一问", tx.clone());
        wait_until(|| thread.history_len() == 2, "第一轮完成").await;

        let _h2 = thread.send("二问", tx.clone());
        wait_until(|| thread.history_len() == 4, "第二轮完成").await;

        let messages = thread.messages();
        assert!(matches!(
            &messages[1].content[0],
            MessageContent::Text { text } if text == "一答"
        ));
        assert!(matches!(
            &messages[3].content[0],
            MessageContent::Text { text } if text == "二答"
        ));
        drop(tx); // 测试持有 tx 会导致 recv() 永不返回 None
        while rx.recv().await.is_some() {}
    }

    #[tokio::test]
    async fn new_send_cancels_previous_active_run() {
        // 旧 Run 挂在 HangingBackend 上；第二次 send 应先取消它
        let backend: Arc<dyn ModelBackend> = Arc::new(HangingBackend);
        let mut config = ThreadConfig::new("t1", "");
        config.policy = deny_policy();
        let thread = Thread::new(
            config,
            backend,
            R::new(),
            Arc::new(AutoBridge(Decision::Deny)),
        );
        let (tx, mut rx) = mpsc::channel(1024);

        let h1 = thread.send("第一个长问题", tx.clone());
        // 等旧 Run 确实启动（收到首个事件）
        wait_until(|| rx.try_recv().is_ok(), "旧 Run 启动").await;
        assert!(!h1.is_cancelled());

        let h2 = thread.send("第二个问题", tx.clone());
        assert!(h1.is_cancelled(), "旧 Run 应被取消");
        assert!(!h2.is_cancelled(), "新 Run 不受影响");

        // 旧 Run 取消终态事件会流出来
        let mut saw_cancelled = false;
        let deadline = tokio::time::Instant::now() + Duration::from_secs(5);
        while tokio::time::Instant::now() < deadline {
            match rx.try_recv() {
                Ok(env) => {
                    if matches!(env.event, crate::events::RunEvent::RunCancelled) {
                        saw_cancelled = true;
                    }
                }
                Err(_) => tokio::time::sleep(Duration::from_millis(10)).await,
            }
            if saw_cancelled {
                break;
            }
        }
        assert!(saw_cancelled, "旧 Run 应发出 RunCancelled");
    }

    #[tokio::test]
    async fn failed_run_keeps_partial_messages_but_next_send_continues() {
        // 第一轮后端直接报错 → 消息序列只留用户输入；新一轮仍能继续
        struct FailingBackend;
        #[async_trait]
        impl ModelBackend for FailingBackend {
            async fn stream_completion(
                &self,
                _req: crate::model::ModelRequest,
            ) -> crate::Result<crate::model::DeltaStream> {
                Err(crate::Error::Model("网关不可用".into()))
            }
        }

        let backend: Arc<dyn ModelBackend> = Arc::new(FailingBackend);
        let mut config = ThreadConfig::new("t1", "");
        config.policy = deny_policy();
        let thread = Thread::new(
            config,
            backend,
            R::new(),
            Arc::new(AutoBridge(Decision::Deny)),
        );
        let (tx, mut rx) = mpsc::channel(1024);

        let _h = thread.send("问题", tx.clone());
        wait_until(
            || {
                rx.try_recv()
                    .is_ok_and(|e| matches!(e.event, crate::events::RunEvent::RunFailed { .. }))
            },
            "等待 RunFailed",
        )
        .await;
        assert_eq!(thread.history_len(), 1, "失败只保留用户输入");
        assert_eq!(thread.session_id(), "t1");
        assert!(thread.project_root().is_none());
    }
}
