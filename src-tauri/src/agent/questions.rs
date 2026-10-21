use std::collections::HashMap;
use std::sync::{Arc, Mutex};

use serde_json::json;
use tauri::{AppHandle, Emitter};
use tokio::sync::oneshot;


// ===== ask_user 提问桥：问题卡事件 + oneshot 回答 =====

/// 等待用户回答的问题集合（question_id → oneshot 发送端）
#[derive(Default)]
pub struct QuestionHub {
    pending: Mutex<HashMap<String, PendingQuestion>>,
}

/// Pending map entry with its owning session (同 PendingDecision：按会话取消
/// 不得波及其他并行运行会话的挂起问题卡)
struct PendingQuestion {
    pub(crate) session_id: String,
    tx: oneshot::Sender<Option<String>>,
}

static QUESTION_SEQ: std::sync::atomic::AtomicU64 = std::sync::atomic::AtomicU64::new(0);

impl QuestionHub {
    pub(crate) fn register(&self, session_id: &str) -> (String, oneshot::Receiver<Option<String>>) {
        let seq = QUESTION_SEQ.fetch_add(1, std::sync::atomic::Ordering::Relaxed);
        let id = format!(
            "q-{}-{seq}",
            std::time::SystemTime::now()
                .duration_since(std::time::UNIX_EPOCH)
                .map(|d| d.as_nanos())
                .unwrap_or(0)
        );
        let (tx, rx) = oneshot::channel();
        self.pending.lock().unwrap().insert(
            id.clone(),
            PendingQuestion {
                session_id: session_id.to_string(),
                tx,
            },
        );
        (id, rx)
    }

    /// 用户回答（None = 取消/拒绝）
    pub(crate) fn answer(&self, question_id: &str, answer: Option<String>) -> Result<(), String> {
        let pending = self
            .pending
            .lock()
            .unwrap()
            .remove(question_id)
            .ok_or_else(|| format!("没有等待中的问题: {question_id}"))?;
        let _ = pending.tx.send(answer);
        Ok(())
    }

    /// 取消 / 重置时丢弃全部挂起问题（对应 receiver 收到 Err → 问题作废）
    pub(crate) fn drop_all(&self) {
        self.pending.lock().unwrap().clear();
    }

    /// 取消单个会话时只丢弃该会话的挂起问题（P3 并发）
    pub(crate) fn drop_session(&self, session_id: &str) {
        self.pending
            .lock()
            .unwrap()
            .retain(|_, p| p.session_id != session_id);
    }
}

/// UserQuestionBridge 宿主实现：发 agent-question 事件给前端，
/// oneshot 等待 agent_question_answer 回传。发送端被清理（取消/重置）
/// 时返回 Err，问题作废，工具把错误回传模型。
/// 从应用设置读取 search_web 的搜索配置：`search_engine` 选后端（bing |
/// duckduckgo | searxng，未设置时默认 bing；仅配置了 search_endpoint 的旧
/// 设置保持 SearXNG 语义，由 agent-core 的 SearchEngine::resolve 统一裁定）。
pub(crate) fn settings_search(app: &AppHandle) -> agent_core::net_tools::SearchEngine {
    let read = |key: &str| -> Option<String> {
        let path = crate::get_settings_path(app).ok()?;
        let content = std::fs::read_to_string(path).ok()?;
        let v: serde_json::Value = serde_json::from_str(&content).ok()?;
        v.get(key)
            .and_then(|s| s.as_str())
            .map(|s| s.trim().to_string())
            .filter(|s| !s.is_empty())
    };
    let endpoint = read("search_endpoint").map(|s| s.trim_end_matches('/').to_string());
    agent_core::net_tools::SearchEngine::resolve(read("search_engine").as_deref(), endpoint)
}

pub(crate) struct QuestionBridge {
    pub(crate) hub: Arc<QuestionHub>,
    pub(crate) app: AppHandle,
    pub(crate) session_id: String,
}

/// 构造 agent-question 事件载荷。抽出纯函数便于单测：
/// session_id 必须随事件下发，前端靠它过滤本会话的问题卡。
pub(crate) fn question_payload(
    session_id: &str,
    question_id: &str,
    question: &str,
    options: Vec<String>,
    allow_free_text: bool,
) -> serde_json::Value {
    json!({
        "session_id": session_id,
        "question_id": question_id,
        "question": question,
        "options": options,
        "allow_free_text": allow_free_text,
    })
}

#[async_trait::async_trait]
impl agent_core::UserQuestionBridge for QuestionBridge {
    async fn ask(
        &self,
        question: String,
        options: Vec<String>,
        allow_free_text: bool,
    ) -> std::result::Result<String, String> {
        let (qid, rx) = self.hub.register(&self.session_id);
        self.app
            .emit(
                "agent-question",
                question_payload(&self.session_id, &qid, &question, options, allow_free_text),
            )
            .map_err(|e| format!("发送问题失败: {e}"))?;
        match rx.await {
            Ok(Some(answer)) => Ok(answer),
            Ok(None) => Err("用户取消回答".into()),
            Err(_) => Err("问题已失效（任务取消或会话重置）".into()),
        }
    }
}
