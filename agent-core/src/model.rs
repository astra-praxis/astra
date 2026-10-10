//! 模型适配层：`ModelBackend` trait + rig 实现。
//!
//! 抽象边界：后端只需要把"请求（消息 + 工具定义）"变成"增量流"。
//! 工具调用的流式参数拼装（SSE `tool_calls` delta 按 index 聚合）
//! 属于后端职责——rig 的 chat-completions 流式实现已完成该工作并实测通过。

use async_trait::async_trait;
use futures::{Stream, StreamExt};
use serde::{Deserialize, Serialize};
use std::pin::Pin;

use crate::error::{Error, Result};
use crate::tool::{ToolCall, ToolDefinition};
use rig::completion::{CompletionModel, ToolDefinition as RigToolDefinition};

/// 消息内容块（enum 建模，对齐 skill 检查清单第 1 条）
#[derive(Debug, Clone, Serialize, Deserialize)]
#[serde(tag = "type", rename_all = "snake_case")]
pub enum MessageContent {
    Text {
        text: String,
    },
    ToolUse {
        call: ToolCall,
    },
    ToolResult {
        call_id: String,
        content: String,
        is_error: bool,
    },
}

impl MessageContent {
    pub fn text(t: impl Into<String>) -> Self {
        Self::Text { text: t.into() }
    }
}

/// 一条消息：role + 内容块列表
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct ChatMessage {
    pub role: Role,
    pub content: Vec<MessageContent>,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum Role {
    System,
    User,
    Assistant,
    /// 上下文压缩摘要（不发给前端展示；转 rig 请求时映射为 User + 前缀说明）
    Compaction,
}

impl Role {
    /// 压缩摘要消息发模型时附加的前缀（对齐 Zed 的注入方式）
    pub fn compaction_prefixed(summary: &str) -> String {
        format!("之前的对话已被压缩。请以下面的摘要作为上下文继续：\n\n{summary}")
    }
}

/// 一次模型请求
#[derive(Debug, Clone)]
pub struct ModelRequest {
    /// 系统提示（公共规则层 + 能力层由上层拼装）
    pub system: String,
    /// 完整消息序列（含本轮用户输入 / 工具结果）
    pub messages: Vec<ChatMessage>,
    /// 可用工具定义
    pub tools: Vec<ToolDefinition>,
    pub temperature: Option<f32>,
    pub max_tokens: Option<u32>,
}

/// 模型流式产出的增量。工具调用为**拼装后的完整调用**
/// （流式参数片段由后端内部聚合，上层无需处理半截 JSON）。
#[derive(Debug, Clone)]
pub enum Delta {
    Text {
        text: String,
    },
    Reasoning {
        text: String,
    },
    /// 后端拼装完成的工具调用（可能一次流到多个，按产出顺序处理）
    ToolCalls {
        calls: Vec<ToolCall>,
    },
    /// 单轮流结束（不代表 Run 结束；Run 循环根据是否产生工具调用决定续轮）
    TurnEnd,
}

pub type DeltaStream = Pin<Box<dyn Stream<Item = Result<Delta>> + Send>>;

/// 模型后端抽象。实现方必须是 Send + Sync（Run 在 tokio 任务里轮转）。
#[async_trait]
pub trait ModelBackend: Send + Sync {
    /// 发起一次流式补全，返回增量流。
    /// 取消语义：丢弃返回的 Stream 即中断底层连接。
    async fn stream_completion(&self, request: ModelRequest) -> Result<DeltaStream>;
}

// ===== rig 实现（OpenAI 兼容 chat completions） =====

/// 基于 rig-core 的 OpenAI 兼容后端。
/// 覆盖：云端 OpenAI 兼容 API、第三方网关、本地 llama.cpp server。
pub struct RigBackend {
    base_url: String,
    api_key: String,
    model_name: String,
}

impl RigBackend {
    pub fn new(
        base_url: impl Into<String>,
        api_key: impl Into<String>,
        model_name: impl Into<String>,
    ) -> Self {
        Self {
            base_url: base_url.into(),
            api_key: api_key.into(),
            model_name: model_name.into(),
        }
    }
}

/// 判断模型错误是否为瞬断（可安全重试初始请求）。
/// 只覆盖请求建立阶段：流已经开始后出错无法恢复（无法从断点续流），不重试。
fn should_retry_model_error(msg: &str) -> bool {
    let m = msg.to_lowercase();
    // 连接层瞬断（reqwest）
    m.contains("error sending request")
        || m.contains("connection refused")
        || m.contains("connection reset")
        || m.contains("timed out")
        || m.contains("timeout")
        // 网关侧瞬时错误：实测火山方舟出现过 502 Bad Gateway，
        // 这类错误秒级自愈，值得退避重试
        || m.contains("bad gateway")
        || m.contains("502")
        || m.contains("503")
        || m.contains("504")
        || m.contains("service unavailable")
}

/// rig 的裸模型低层流式入口：`CompletionModel::completion_request(prompt)`
/// 约定“prompt 是最后一条消息，其余为 chat_history”。
/// 我们的 Thread 消息序列是无角色的平铺块列表，此函数负责拆分 + 转换。
fn to_rig_messages(
    messages: &[ChatMessage],
) -> (Vec<rig::message::Message>, rig::message::Message) {
    let convert = |m: &ChatMessage| -> rig::message::Message {
        let empty_text = || rig::OneOrMany::one(rig::message::UserContent::text(String::new()));
        match m.role {
            Role::User => {
                let content: Vec<rig::message::UserContent> = m
                    .content
                    .iter()
                    .filter_map(|c| match c {
                        MessageContent::Text { text } => {
                            Some(rig::message::UserContent::text(text.clone()))
                        }
                        MessageContent::ToolResult {
                            call_id, content, ..
                        } => Some(rig::message::UserContent::tool_result(
                            call_id,
                            rig::OneOrMany::one(rig::message::ToolResultContent::text(
                                content.clone(),
                            )),
                        )),
                        MessageContent::ToolUse { .. } => None,
                    })
                    .collect();
                rig::message::Message::User {
                    content: rig::OneOrMany::many(content).unwrap_or_else(|_| empty_text()),
                }
            }
            Role::Assistant => {
                let content: Vec<rig::message::AssistantContent> = m
                    .content
                    .iter()
                    .filter_map(|c| match c {
                        MessageContent::Text { text } => {
                            Some(rig::message::AssistantContent::text(text.clone()))
                        }
                        MessageContent::ToolUse { call } => {
                            Some(rig::message::AssistantContent::tool_call(
                                &call.id,
                                &call.name,
                                call.arguments.clone(),
                            ))
                        }
                        MessageContent::ToolResult { .. } => None,
                    })
                    .collect();
                rig::message::Message::Assistant {
                    id: None,
                    content: rig::OneOrMany::many(content).unwrap_or_else(|_| {
                        rig::OneOrMany::one(rig::message::AssistantContent::text(String::new()))
                    }),
                }
            }
            Role::System => rig::message::Message::User {
                content: empty_text(),
            },
            Role::Compaction => {
                let summary = match m.content.first() {
                    Some(MessageContent::Text { text }) => text.clone(),
                    _ => String::new(),
                };
                rig::message::Message::User {
                    content: rig::OneOrMany::one(rig::message::UserContent::text(
                        Role::compaction_prefixed(&summary),
                    )),
                }
            }
        }
    };

    // rig 约定：最后一条是 prompt，其余是 history。
    let last = messages.last().cloned().unwrap_or(ChatMessage {
        role: Role::User,
        content: vec![MessageContent::text(String::new())],
    });
    let history: Vec<rig::message::Message> = messages[..messages.len().saturating_sub(1)]
        .iter()
        .map(&convert)
        .collect();
    (history, convert(&last))
}

#[async_trait]
impl ModelBackend for RigBackend {
    async fn stream_completion(&self, request: ModelRequest) -> Result<DeltaStream> {
        let client = rig::providers::openai::Client::builder(self.api_key.as_str())
            .base_url(self.base_url.as_str())
            .build();
        let model =
            rig::providers::openai::completion::CompletionModel::new(client, &self.model_name);

        // 裸模型低层流式入口（agent 无关）：completion_request → builder → stream。
        // 注意：不能用 `StreamingCompletion` trait —— rig 0.22 只为 Agent 实现，
        // 裸 CompletionModel 走 builder.stream()。
        // builder 被 stream() 消耗，闭包内每次重建请求。
        let build_request = || {
            let (history, prompt) = to_rig_messages(&request.messages);
            let mut builder = model.completion_request(prompt).messages(history).tools(
                request
                    .tools
                    .iter()
                    .map(|t| RigToolDefinition {
                        name: t.name.clone(),
                        description: t.description.clone(),
                        parameters: t.parameters.clone(),
                    })
                    .collect(),
            );
            if !request.system.is_empty() {
                builder = builder.preamble(request.system.clone());
            }
            builder = builder.temperature_opt(request.temperature.map(|t| t as f64));
            builder = builder.max_tokens_opt(request.max_tokens.map(|t| t as u64));
            builder
        };

        // 初始请求指数退避重试（共 3 次尝试，间隔 1s/2s）：
        // 连接层瞬断（reqwest "error sending request"）与网关 502 在实测中出现过；
        // 原先 300ms/600ms 的间隔太短，网关抖动往往持续几秒，
        // 3 次全撞在同一抖动窗口里，等于没重试。
        // 流一旦建立后出错不可恢复，不重试。
        let mut raw = None;
        let mut last_err = String::new();
        for attempt in 0..3 {
            if attempt > 0 {
                tracing::warn!(
                    "模型请求失败，{}ms 后重试（第 {attempt}/2 次）: {last_err}",
                    1000 * attempt as u64
                );
                tokio::time::sleep(std::time::Duration::from_millis(1000 * attempt as u64)).await;
            }
            match build_request().stream().await {
                Ok(s) => {
                    raw = Some(s);
                    break;
                }
                Err(e) => {
                    last_err = e.to_string();
                    if !should_retry_model_error(&last_err) {
                        return Err(Error::Model(last_err));
                    }
                }
            }
        }
        let raw = raw.ok_or_else(|| Error::Model(last_err))?;

        // 空文本增量过滤：部分服务商在流式输出 tool_call 参数时会夹带
        // 数千个空 Text chunk，照单全收会淹没事件流并拖垮前端。
        // 用 filter_map 跳过它们。
        Ok(Box::pin(raw.filter_map(|item| async move {
            match item {
                Ok(rig::streaming::StreamedAssistantContent::Text(t)) if t.text.is_empty() => None,
                Ok(rig::streaming::StreamedAssistantContent::Text(t)) => {
                    Some(Ok(Delta::Text { text: t.text }))
                }
                Ok(rig::streaming::StreamedAssistantContent::Reasoning(r)) => {
                    Some(Ok(Delta::Reasoning {
                        text: r.reasoning.join("\n"),
                    }))
                }
                Ok(rig::streaming::StreamedAssistantContent::ToolCall(tc)) => {
                    Some(Ok(Delta::ToolCalls {
                        calls: vec![ToolCall {
                            id: tc.id,
                            call_id: tc.call_id,
                            name: tc.function.name,
                            arguments: tc.function.arguments,
                        }],
                    }))
                }
                Ok(rig::streaming::StreamedAssistantContent::Final(_)) => Some(Ok(Delta::TurnEnd)),
                Err(e) => Some(Err(Error::Model(e.to_string()))),
            }
        })))
    }
}

#[cfg(test)]
mod to_rig_messages_tests {
    use super::*;
    use rig::message::{AssistantContent, Message, UserContent};

    fn user_text(t: &str) -> ChatMessage {
        ChatMessage {
            role: Role::User,
            content: vec![MessageContent::text(t)],
        }
    }

    fn assistant_text(t: &str) -> ChatMessage {
        ChatMessage {
            role: Role::Assistant,
            content: vec![MessageContent::text(t)],
        }
    }

    fn sample_tool_call(id: &str, name: &str) -> ToolCall {
        ToolCall {
            id: id.into(),
            call_id: None,
            name: name.into(),
            arguments: serde_json::json!({"path": "a.txt"}),
        }
    }

    fn user_content_of(msg: &Message) -> Vec<UserContent> {
        match msg {
            Message::User { content } => content.iter().cloned().collect(),
            other => panic!("预期 User 消息，实际: {other:?}"),
        }
    }

    fn assistant_content_of(msg: &Message) -> Vec<AssistantContent> {
        match msg {
            Message::Assistant { content, .. } => content.iter().cloned().collect(),
            other => panic!("预期 Assistant 消息，实际: {other:?}"),
        }
    }

    #[test]
    fn single_user_text_becomes_prompt_with_empty_history() {
        let (history, prompt) = to_rig_messages(&[user_text("你好")]);
        assert!(history.is_empty());
        let content = user_content_of(&prompt);
        assert_eq!(content.len(), 1);
        match &content[0] {
            UserContent::Text(t) => assert_eq!(t.text, "你好"),
            other => panic!("预期文本块，实际: {other:?}"),
        }
    }

    #[test]
    fn last_message_is_prompt_rest_is_history() {
        let msgs = vec![
            user_text("第一问"),
            assistant_text("第一答"),
            user_text("第二问"),
        ];
        let (history, prompt) = to_rig_messages(&msgs);
        assert_eq!(history.len(), 2);
        let first = user_content_of(&history[0]);
        match &first[0] {
            UserContent::Text(t) => assert_eq!(t.text, "第一问"),
            other => panic!("预期文本块，实际: {other:?}"),
        }
        let prompt_content = user_content_of(&prompt);
        match &prompt_content[0] {
            UserContent::Text(t) => assert_eq!(t.text, "第二问"),
            other => panic!("预期文本块，实际: {other:?}"),
        }
    }

    #[test]
    fn assistant_text_and_tool_use_both_preserved() {
        let msgs = vec![ChatMessage {
            role: Role::Assistant,
            content: vec![
                MessageContent::text("我来读取"),
                MessageContent::ToolUse {
                    call: sample_tool_call("t1", "read_file"),
                },
            ],
        }];
        let (_, prompt) = to_rig_messages(&msgs);
        let content = assistant_content_of(&prompt);
        assert_eq!(content.len(), 2);
        match &content[0] {
            AssistantContent::Text(t) => assert_eq!(t.text, "我来读取"),
            other => panic!("预期文本，实际: {other:?}"),
        }
        match &content[1] {
            AssistantContent::ToolCall(tc) => {
                assert_eq!(tc.id, "t1");
                assert_eq!(tc.function.name, "read_file");
                assert_eq!(tc.function.arguments["path"], "a.txt");
            }
            other => panic!("预期 tool_call，实际: {other:?}"),
        }
    }

    #[test]
    fn tool_result_id_matches_tool_call_id() {
        // openai 协议要求 tool_result 的 id 等于助手 tool_call 的 id
        let msgs = vec![ChatMessage {
            role: Role::User,
            content: vec![MessageContent::ToolResult {
                call_id: "t1".into(),
                content: "文件内容".into(),
                is_error: false,
            }],
        }];
        let (_, prompt) = to_rig_messages(&msgs);
        let content = user_content_of(&prompt);
        match &content[0] {
            UserContent::ToolResult(tr) => {
                assert_eq!(tr.id, "t1");
                let texts: Vec<String> = tr
                    .content
                    .iter()
                    .filter_map(|c| match c {
                        rig::message::ToolResultContent::Text(t) => Some(t.text.clone()),
                        _ => None,
                    })
                    .collect();
                assert_eq!(texts, vec!["文件内容".to_string()]);
            }
            other => panic!("预期 tool_result，实际: {other:?}"),
        }
    }

    #[test]
    fn tool_use_filtered_from_user_role() {
        // User 消息里的 ToolUse 块不应进入 rig 消息（防御性过滤）
        let msgs = vec![ChatMessage {
            role: Role::User,
            content: vec![
                MessageContent::text("问题"),
                MessageContent::ToolUse {
                    call: sample_tool_call("t9", "x"),
                },
            ],
        }];
        let (_, prompt) = to_rig_messages(&msgs);
        let content = user_content_of(&prompt);
        assert_eq!(content.len(), 1, "ToolUse 应被过滤");
        assert!(matches!(&content[0], UserContent::Text(_)));
    }

    #[test]
    fn empty_messages_fallback_to_empty_user_prompt() {
        let (history, prompt) = to_rig_messages(&[]);
        assert!(history.is_empty());
        let content = user_content_of(&prompt);
        match &content[0] {
            UserContent::Text(t) => assert_eq!(t.text, ""),
            other => panic!("预期空文本回退，实际: {other:?}"),
        }
    }

    #[test]
    fn mixed_text_and_tool_result_user_message_keeps_both() {
        let msgs = vec![ChatMessage {
            role: Role::User,
            content: vec![
                MessageContent::Text {
                    text: "顺便".into(),
                },
                MessageContent::ToolResult {
                    call_id: "t2".into(),
                    content: "结果".into(),
                    is_error: true,
                },
            ],
        }];
        let (_, prompt) = to_rig_messages(&msgs);
        let content = user_content_of(&prompt);
        assert_eq!(content.len(), 2, "文本与 tool_result 都应保留");
        assert!(matches!(&content[0], UserContent::Text(_)));
        assert!(matches!(&content[1], UserContent::ToolResult(_)));
    }

    #[test]
    fn multi_tool_use_assistant_message_preserves_order() {
        let msgs = vec![ChatMessage {
            role: Role::Assistant,
            content: vec![
                MessageContent::ToolUse {
                    call: sample_tool_call("t1", "read_file"),
                },
                MessageContent::ToolUse {
                    call: sample_tool_call("t2", "list_files"),
                },
            ],
        }];
        let (_, prompt) = to_rig_messages(&msgs);
        let content = assistant_content_of(&prompt);
        assert_eq!(content.len(), 2);
        match &content[0] {
            AssistantContent::ToolCall(tc) => assert_eq!(tc.id, "t1"),
            other => panic!("预期 tool_call，实际: {other:?}"),
        }
        match &content[1] {
            AssistantContent::ToolCall(tc) => assert_eq!(tc.id, "t2"),
            other => panic!("预期 tool_call，实际: {other:?}"),
        }
    }
}

#[cfg(test)]
mod retry_tests {
    use super::*;

    #[test]
    fn connection_layer_errors_are_retryable() {
        // 实测出现过的 reqwest 连接层瞬断
        assert!(should_retry_model_error(
            "error sending request for url (http://localhost:8080/v1/chat/completions)"
        ));
        assert!(should_retry_model_error("Connection refused (os error 61)"));
        assert!(should_retry_model_error("connection reset by peer"));
        assert!(should_retry_model_error("operation timed out"));
    }

    #[test]
    fn gateway_errors_are_retryable() {
        // 实测火山方舟出现过的网关瞬时错误
        assert!(should_retry_model_error(
            "Invalid status code: 502 Bad Gateway"
        ));
        assert!(should_retry_model_error(
            "Invalid status code: 503 Service Unavailable"
        ));
    }

    #[test]
    fn protocol_errors_are_not_retryable() {
        // 业务层错误重试无意义（401 重试还是 401）
        assert!(!should_retry_model_error("401 Unauthorized"));
        assert!(!should_retry_model_error("invalid api key"));
        assert!(!should_retry_model_error("model not found"));
        assert!(!should_retry_model_error(
            "stream disconnected before completion"
        ));
    }
}
