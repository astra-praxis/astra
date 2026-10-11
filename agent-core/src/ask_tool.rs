//! ask_user 工具：任务中途向用户提问（问题卡 + 选项 / 自由文本）。
//!
//! 参考 Zed ask_user_tool 的语义：答案 verbatim 回传模型；选项 ≥2 或
//! 允许自由文本，二者必须有其一。核心库不依赖 UI——提问通道由宿主实现
//! `UserQuestionBridge`（Tauri 事件 + oneshot），取消优先于等待回答。

use crate::error::{Error, Result};
use crate::tool::{AgentTool, ToolContext, ToolOutput};
use async_trait::async_trait;
use serde_json::{json, Value};
use std::sync::Arc;

/// 提问桥：宿主层实现（发出问题卡事件 + 等待用户回答）。
/// 返回 Err 表示用户取消/拒绝/通道失效——工具会把错误信息回传模型。
#[async_trait]
pub trait UserQuestionBridge: Send + Sync {
    async fn ask(
        &self,
        question: String,
        options: Vec<String>,
        allow_free_text: bool,
    ) -> std::result::Result<String, String>;
}

/// 测试用：自动回答的桥
pub struct AutoAnswerBridge(pub String);

#[async_trait]
impl UserQuestionBridge for AutoAnswerBridge {
    async fn ask(
        &self,
        _question: String,
        _options: Vec<String>,
        _allow_free_text: bool,
    ) -> std::result::Result<String, String> {
        Ok(self.0.clone())
    }
}

pub struct AskUserTool {
    bridge: Arc<dyn UserQuestionBridge>,
}

impl AskUserTool {
    pub fn new(bridge: Arc<dyn UserQuestionBridge>) -> Self {
        Self { bridge }
    }
}

#[async_trait]
impl AgentTool for AskUserTool {
    fn name(&self) -> &str {
        "ask_user"
    }
    fn description(&self) -> &str {
        "向用户提问并等待回答（问题卡形式）。需要用户做决定、确认假设或补充信息时使用。\
         提供至少两个 options（可点击选项），或把 allow_free_text 设为 true（用户自由输入），\
         也可两者同时提供"
    }
    fn parameters_schema(&self) -> Value {
        json!({
            "type": "object",
            "properties": {
                "question": { "type": "string", "description": "要问的问题，简短明确" },
                "options": { "type": "array", "items": { "type": "string" },
                    "description": "可选：展示给用户的选项（每项即按钮文案，选中后原样返回）。至少两项，或改用 allow_free_text" },
                "allow_free_text": { "type": "boolean",
                    "description": "可选：是否允许用户自由输入回答。选项可能不全面时设为 true" }
            },
            "required": ["question"]
        })
    }
    async fn call(&self, args: Value, ctx: &ToolContext) -> Result<ToolOutput> {
        let question = args
            .get("question")
            .and_then(|v| v.as_str())
            .ok_or_else(|| Error::tool("ask_user", "缺少 question"))?;
        let options: Vec<String> = args
            .get("options")
            .and_then(|v| v.as_array())
            .map(|a| {
                a.iter()
                    .filter_map(|v| v.as_str().map(|s| s.to_string()))
                    .collect()
            })
            .unwrap_or_default();
        let allow_free_text = args
            .get("allow_free_text")
            .and_then(|v| v.as_bool())
            .unwrap_or(false);

        // 对齐 Zed：至少两个选项，或允许自由文本；否则模型应直接用自然语言问
        if !allow_free_text && options.len() < 2 {
            return Err(Error::tool(
                "ask_user",
                "ask_user 需要至少两个 options，或把 allow_free_text 设为 true；\
                 开放式问题请直接在回复中用自然语言提问",
            ));
        }

        // 取消优先：Run 被取消时问题作废，不等用户回答
        let question = question.to_string();
        let bridge = Arc::clone(&self.bridge);
        let answer = tokio::select! {
            _ = ctx.cancelled.cancelled() => {
                return Err(Error::tool("ask_user", "任务已取消，问题作废"));
            }
            r = bridge.ask(question, options, allow_free_text) => r,
        };
        match answer {
            Ok(selected) => Ok(ToolOutput::from(format!("用户回答: {selected}"))),
            Err(reason) => Err(Error::tool("ask_user", reason)),
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::time::Duration;
    use tokio_util::sync::CancellationToken;

    struct DelayedBridge {
        delay_ms: u64,
        answer: std::result::Result<String, String>,
    }

    #[async_trait]
    impl UserQuestionBridge for DelayedBridge {
        async fn ask(
            &self,
            _question: String,
            _options: Vec<String>,
            _allow_free_text: bool,
        ) -> std::result::Result<String, String> {
            tokio::time::sleep(Duration::from_millis(self.delay_ms)).await;
            self.answer.clone()
        }
    }

    fn ctx(token: CancellationToken) -> ToolContext {
        ToolContext {
            project_root: None,
            bridge: Arc::new(crate::permissions::AutoBridge(
                crate::permissions::Decision::Allow,
            )),
            cancelled: token,
        }
    }

    #[tokio::test]
    async fn answer_returned_verbatim_with_prefix() {
        let tool = AskUserTool::new(Arc::new(AutoAnswerBridge("方案 B".into())));
        let out = tool
            .call(
                json!({"question": "选哪个方案？", "options": ["方案 A", "方案 B"]}),
                &ctx(CancellationToken::new()),
            )
            .await
            .unwrap();
        assert_eq!(out.content, "用户回答: 方案 B");
    }

    #[tokio::test]
    async fn requires_two_options_or_free_text() {
        let tool = AskUserTool::new(Arc::new(AutoAnswerBridge("x".into())));
        let token = CancellationToken::new();
        // 只有一个选项且不允许自由文本 → 参数错误（不触桥）
        let err = tool
            .call(
                json!({"question": "?", "options": ["仅一项"]}),
                &ctx(token.clone()),
            )
            .await
            .unwrap_err();
        assert!(err.to_string().contains("至少两个 options"));
        // 两者皆无 → 同样报错
        let err = tool
            .call(json!({"question": "?"}), &ctx(token.clone()))
            .await
            .unwrap_err();
        assert!(err.to_string().contains("至少两个 options"));
        // 无选项但允许自由文本 → 合法（触桥返回）
        let out = tool
            .call(
                json!({"question": "?", "allow_free_text": true}),
                &ctx(token),
            )
            .await
            .unwrap();
        assert_eq!(out.content, "用户回答: x");
    }

    #[tokio::test]
    async fn missing_question_errors() {
        let tool = AskUserTool::new(Arc::new(AutoAnswerBridge("x".into())));
        let err = tool
            .call(
                json!({"options": ["a", "b"]}),
                &ctx(CancellationToken::new()),
            )
            .await
            .unwrap_err();
        assert!(err.to_string().contains("question"));
    }

    #[tokio::test]
    async fn bridge_error_is_tool_error() {
        let tool = AskUserTool::new(Arc::new(DelayedBridge {
            delay_ms: 1,
            answer: Err("用户拒绝回答".into()),
        }));
        let err = tool
            .call(
                json!({"question": "?", "allow_free_text": true}),
                &ctx(CancellationToken::new()),
            )
            .await
            .unwrap_err();
        assert!(err.to_string().contains("用户拒绝回答"));
    }

    #[tokio::test]
    async fn cancel_beats_pending_question() {
        // 桥 300ms 后才返回；100ms 时取消 → 立即报「任务已取消」
        let tool = AskUserTool::new(Arc::new(DelayedBridge {
            delay_ms: 300,
            answer: Ok("太迟了".into()),
        }));
        let token = CancellationToken::new();
        let t = token.clone();
        let task = tokio::spawn(async move {
            tool.call(json!({"question": "?", "allow_free_text": true}), &ctx(t))
                .await
        });
        tokio::time::sleep(Duration::from_millis(100)).await;
        token.cancel();
        let started = std::time::Instant::now();
        let err = task.await.unwrap().unwrap_err();
        assert!(err.to_string().contains("已取消"));
        assert!(
            started.elapsed() < Duration::from_millis(250),
            "取消应立即返回，不等桥"
        );
    }
}
