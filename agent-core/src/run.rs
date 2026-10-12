//! Run：一次任务执行。自定义 Agent Loop。
//!
//! 循环（每轮）：
//!   模型流式请求 → 文本增量发事件 → 收到拼装完成的工具调用 →
//!   静态权限判定 → Allow 直执行 / Confirm 经 PermissionBridge 挂起等批准 /
//!   Deny 回错误 → 执行结果写回消息序列 → 续轮，直到无工具调用或取消/预算耗尽。
//!
//! 竞态规则（评审定稿）：
//! - 取消后不再启动新的模型请求与工具调用
//! - 取消与批准同时发生：取消优先（select! 保证）
//! - 旧 Run 在新 Run 开始前被 cancel（Thread 层负责）
//! - 权限请求绑定具体 tool_call_id；批准不扩大到其他调用

use futures::StreamExt;
use std::path::PathBuf;
use std::sync::Arc;
use std::time::Duration;
use tokio::sync::mpsc;
use tokio_util::sync::CancellationToken;

use crate::error::{Error, Result};
use crate::events::{Envelope, RunEvent, RunStatus};
use crate::model::{ChatMessage, MessageContent, ModelBackend, ModelRequest, Role};
use crate::permissions::{decide, Decision, PermissionBridge, ToolPolicy};
use crate::texts::{SharedTextPack, TextPack};
use crate::tool::{ToolCall, ToolContext, ToolRegistry};

/// 每轮流式增量间的空闲超时（非整轮预算；防模型端挂死）
const STREAM_IDLE_TIMEOUT: Duration = Duration::from_secs(120);
/// 最大续轮数默认值（防止无限工具循环的预算上限之一，可在 ThreadConfig 覆盖）
pub const DEFAULT_MAX_TURNS: u32 = 50;
/// 上下文自动压缩默认阈值（估算 token 数；参考 Zed：服务端 usage 不可靠时用估算触发）
pub const DEFAULT_AUTO_COMPACT_TOKENS: u64 = 60_000;
/// 压缩点之前保留的用户消息原文预算（字节，≈2万 token；对齐 Zed 的 80KB 启发式）
const RETAINED_USER_BYTE_BUDGET: usize = 80_000;

pub struct RunConfig {
    pub session_id: String,
    pub run_id: String,
    pub system_prompt: String,
    pub temperature: Option<f32>,
    pub max_tokens: Option<u32>,
    pub max_turns: u32,
    /// 上下文自动压缩阈值（估算 token 数）；None = 关闭
    pub auto_compact_tokens: Option<u64>,
    pub policy: ToolPolicy,
    /// 文案模板包（模型上下文与前端事件的可见文案；默认中文）
    pub texts: SharedTextPack,
}

impl RunConfig {
    pub fn new(session_id: impl Into<String>, system_prompt: impl Into<String>) -> Self {
        Self {
            session_id: session_id.into(),
            run_id: format!("run_{}", chrono_ts()),
            system_prompt: system_prompt.into(),
            temperature: None,
            max_tokens: None,
            max_turns: DEFAULT_MAX_TURNS,
            auto_compact_tokens: Some(DEFAULT_AUTO_COMPACT_TOKENS),
            policy: ToolPolicy::default(),
            texts: Arc::new(TextPack::zh()),
        }
    }
}

fn chrono_ts() -> u64 {
    std::time::SystemTime::now()
        .duration_since(std::time::UNIX_EPOCH)
        .unwrap_or_default()
        .as_millis() as u64
}

/// Run 句柄：前端/Thread 持有，用于取消
#[derive(Clone)]
pub struct RunHandle {
    token: CancellationToken,
}

impl RunHandle {
    pub(crate) fn from_token(token: CancellationToken) -> Self {
        Self { token }
    }

    pub fn cancel(&self) {
        self.token.cancel();
    }

    pub fn is_cancelled(&self) -> bool {
        self.token.is_cancelled()
    }
}

/// 执行一次 Run。
///
/// - `messages`：进入 Run 前的完整消息序列（本轮用户输入已在最后）。
///   Run 结束后返回追加了助手回复/工具调用/工具结果的完整序列。
/// - 事件经 `event_tx` 发出（Envelope 带 session_id / run_id / sequence）。
/// - `cancel`：取消令牌；Thread 层旧 Run 先 cancel 再开新 Run。
#[allow(clippy::too_many_arguments)]
pub async fn execute(
    backend: Arc<dyn ModelBackend>,
    registry: ToolRegistry,
    config: RunConfig,
    project_root: Option<PathBuf>,
    bridge: Arc<dyn PermissionBridge>,
    mut messages: Vec<ChatMessage>,
    event_tx: mpsc::Sender<Envelope>,
    cancel: CancellationToken,
) -> Result<Vec<ChatMessage>> {
    let ctx = ToolContext {
        project_root,
        bridge,
        cancelled: cancel.clone(),
    };

    tracing::info!(
        session_id = %config.session_id,
        run_id = %config.run_id,
        turns = messages.len(),
        tools = registry.names().len(),
        "Run 开始"
    );

    let result = run_loop(
        backend.as_ref(),
        &registry,
        &config,
        &mut messages,
        &event_tx,
        &ctx,
        &cancel,
    )
    .await;

    match result {
        Ok(()) => {
            tracing::info!(session_id = %config.session_id, run_id = %config.run_id, "Run 完成");
            Ok(messages)
        }
        Err(e) => {
            tracing::warn!(session_id = %config.session_id, run_id = %config.run_id, error = %e, "Run 失败");
            Err(e)
        }
    }
}

async fn run_loop(
    backend: &dyn ModelBackend,
    registry: &ToolRegistry,
    config: &RunConfig,
    messages: &mut Vec<ChatMessage>,
    event_tx: &mpsc::Sender<Envelope>,
    ctx: &ToolContext,
    cancel: &CancellationToken,
) -> Result<()> {
    let mut sequence: u64 = 0;
    macro_rules! emit {
        ($event:expr) => {{
            sequence += 1;
            let _ = event_tx
                .send($event.with_meta(&config.session_id, &config.run_id, sequence))
                .await;
        }};
    }

    emit!(RunEvent::StatusChanged {
        status: RunStatus::Analyzing
    });

    // 压缩失败后本 Run 不再重试（下轮继续会再次触发判定，避免每轮重复请求摘要）
    let mut compaction_disabled = false;

    for turn in 0..config.max_turns {
        if cancel.is_cancelled() {
            emit!(RunEvent::RunCancelled);
            return Err(Error::Cancelled);
        }

        // ===== 0. 上下文自动压缩（对齐 Zed：每轮循环顶部检查，工具轮次间也可触发） =====
        if let Some(threshold) = config.auto_compact_tokens {
            if !compaction_disabled && should_compact(messages, threshold) {
                emit!(RunEvent::StatusChanged {
                    status: RunStatus::Compacting
                });
                match compact_context(backend, config, messages, cancel).await {
                    Ok(summary_len) => {
                        tracing::info!(
                            session_id = %config.session_id,
                            run_id = %config.run_id,
                            summary_len,
                            "上下文压缩完成"
                        );
                        emit!(RunEvent::CompactionCompleted);
                    }
                    Err(Error::Cancelled) => {
                        emit!(RunEvent::RunCancelled);
                        return Err(Error::Cancelled);
                    }
                    Err(e) => {
                        tracing::warn!(
                            session_id = %config.session_id,
                            run_id = %config.run_id,
                            error = %e,
                            "上下文压缩失败（本 Run 不再重试，继续原上下文）"
                        );
                        compaction_disabled = true;
                    }
                }
                emit!(RunEvent::StatusChanged {
                    status: RunStatus::Analyzing
                });
            }
        }

        // ===== 1. 模型流式请求（按压缩点裁剪后的视图发请求） =====
        let request = ModelRequest {
            system: config.system_prompt.clone(),
            messages: request_view(messages),
            tools: registry.definitions(),
            temperature: config.temperature,
            max_tokens: config.max_tokens,
        };
        let mut stream = tokio::select! {
            _ = cancel.cancelled() => {
                emit!(RunEvent::RunCancelled);
                return Err(Error::Cancelled);
            }
            s = backend.stream_completion(request) => match s {
                Ok(s) => {
                    tracing::debug!(session_id = %config.session_id, run_id = %config.run_id, turn = turn, "模型流已建立");
                    s
                }
                Err(e) => {
                    tracing::warn!(session_id = %config.session_id, run_id = %config.run_id, turn = turn, error = %e, "模型请求失败");
                    emit!(RunEvent::RunFailed {
                        error: e.to_string()
                    });
                    return Err(e);
                }
            },
        };

        // ===== 2. 消费增量：文本直发事件；工具调用聚合后统一处理 =====
        let mut turn_text = String::new();
        let mut pending_calls: Vec<ToolCall> = vec![];

        loop {
            let item = tokio::select! {
                _ = cancel.cancelled() => {
                    // drop stream 即中断底层连接（探针已验证）
                    emit!(RunEvent::RunCancelled);
                    return Err(Error::Cancelled);
                }
                item = tokio::time::timeout(STREAM_IDLE_TIMEOUT, stream.next()) => {
                    match item {
                        Err(_) => {
                            let e = Error::BudgetExceeded {
                                budget_secs: STREAM_IDLE_TIMEOUT.as_secs(),
                            };
                            emit!(RunEvent::RunFailed {
                                error: config.texts.budget_exceeded(STREAM_IDLE_TIMEOUT.as_secs())
                            });
                            return Err(e);
                        }
                        Ok(None) => break,
                        Ok(Some(v)) => match v {
                            Ok(v) => v,
                            Err(e) => {
                                emit!(RunEvent::RunFailed {
                                    error: e.to_string()
                                });
                                return Err(e);
                            }
                        },
                    }
                }
            };

            match item {
                crate::model::Delta::Text { text } => {
                    turn_text.push_str(&text);
                    emit!(RunEvent::MessageDelta { text });
                }
                crate::model::Delta::Reasoning { text } => {
                    // 思考过程透出（参考 Zed 的 thinking 块）；无 reasoning 的模型不会产生此事件
                    emit!(RunEvent::ThinkingDelta { text });
                }
                crate::model::Delta::ToolCallStart { id, name } => {
                    // 参数开始生成（首片段带 id+name）；fragment 为空表示首帧
                    emit!(RunEvent::ToolCallArgsDelta {
                        tool_call_id: id,
                        tool_name: name,
                        fragment: String::new(),
                    });
                }
                crate::model::Delta::ToolCallArgsDelta { id, name, fragment } => {
                    emit!(RunEvent::ToolCallArgsDelta {
                        tool_call_id: id,
                        tool_name: name,
                        fragment,
                    });
                }
                crate::model::Delta::ToolCalls { calls } => {
                    pending_calls.extend(calls);
                }
                crate::model::Delta::TurnEnd => {
                    // 单轮流结束；是否续轮由 pending_calls 决定
                }
            }
        }

        // ===== 3. 无工具调用 → 本 Run 结束 =====
        if pending_calls.is_empty() {
            messages.push(ChatMessage {
                role: Role::Assistant,
                content: vec![MessageContent::Text {
                    text: turn_text.clone(),
                }],
            });
            emit!(RunEvent::RunCompleted {
                final_text: turn_text
            });
            return Ok(());
        }

        // ===== 4. 有工具调用：助手消息（text + tool_use 块）入序列 =====
        let mut assistant_content = vec![];
        if !turn_text.is_empty() {
            assistant_content.push(MessageContent::Text {
                text: turn_text.clone(),
            });
        }
        for call in &pending_calls {
            assistant_content.push(MessageContent::ToolUse { call: call.clone() });
        }
        messages.push(ChatMessage {
            role: Role::Assistant,
            content: assistant_content,
        });

        // ===== 5. 逐个执行工具（权限三态） =====
        for call in pending_calls {
            if cancel.is_cancelled() {
                emit!(RunEvent::RunCancelled);
                return Err(Error::Cancelled);
            }

            let tool = match registry.get(&call.name) {
                Some(t) => t,
                None => {
                    // 未注册工具：错误回传模型，不中断 Run
                    let msg = config.texts.unregistered_tool(&call.name);
                    emit!(RunEvent::ToolCallFailed {
                        tool_call_id: call.id.clone(),
                        tool_name: call.name.clone(),
                        error: msg.clone(),
                    });
                    push_tool_result(messages, &call.id, msg, true);
                    continue;
                }
            };

            emit!(RunEvent::ToolCallStarted {
                tool_call_id: call.id.clone(),
                tool_name: call.name.clone(),
                args_summary: summarize_args(&call.arguments),
                args: Some(cap_args(&call.arguments, &config.texts)),
            });

            // 5.1 静态权限判定（纯函数）
            let decision = decide(&config.policy, &call.name);
            let approved = match decision {
                Decision::Allow => true,
                Decision::Deny => false,
                Decision::Confirm => {
                    // 5.2 运行时批准：挂起等用户；取消优先
                    emit!(RunEvent::StatusChanged {
                        status: RunStatus::AwaitingPermission,
                    });
                    emit!(RunEvent::PermissionRequested {
                        tool_call_id: call.id.clone(),
                        tool_name: call.name.clone(),
                        summary: summarize_args(&call.arguments),
                    });

                    let req = crate::permissions::PermissionRequest {
                        tool_call_id: call.id.clone(),
                        tool_name: call.name.clone(),
                        summary: summarize_args(&call.arguments),
                        arguments: call.arguments.clone(),
                    };
                    let d = tokio::select! {
                        _ = cancel.cancelled() => {
                            emit!(RunEvent::RunCancelled);
                            return Err(Error::Cancelled);
                        }
                        d = ctx.bridge.request(req) => d,
                    };
                    tracing::info!(
                        session_id = %config.session_id,
                        run_id = %config.run_id,
                        tool = %call.name,
                        decision = ?d,
                        "权限决定"
                    );
                    emit!(RunEvent::PermissionDecided {
                        tool_call_id: call.id.clone(),
                        decision: d,
                    });
                    d == Decision::Allow
                }
            };

            if !approved {
                let msg = if decision == Decision::Deny {
                    config.texts.policy_denied(&call.name)
                } else {
                    config.texts.user_denied()
                };
                emit!(RunEvent::ToolCallFailed {
                    tool_call_id: call.id.clone(),
                    tool_name: call.name.clone(),
                    error: msg.clone(),
                });
                push_tool_result(messages, &call.id, msg, true);
                continue;
            }

            // 5.3 执行（参数校验 → 调用；错误回传模型不中断 Run）
            emit!(RunEvent::StatusChanged {
                status: RunStatus::ExecutingTool,
            });

            let exec_result = match crate::permissions::validate_args_with(
                &tool.parameters_schema(),
                &call.arguments,
                &config.texts,
            ) {
                Err(e) => Err(e),
                Ok(()) => tool.call(call.arguments.clone(), ctx).await,
            };

            match exec_result {
                Ok(output) => {
                    tracing::debug!(
                        session_id = %config.session_id,
                        run_id = %config.run_id,
                        tool = %call.name,
                        "工具执行成功"
                    );
                    emit!(RunEvent::ToolCallCompleted {
                        tool_call_id: call.id.clone(),
                        tool_name: call.name.clone(),
                        result_summary: summarize_text(&output.content, 200),
                        artifacts: output.artifacts,
                    });
                    push_tool_result(messages, &call.id, output.content, false);
                }
                Err(e) => {
                    let msg = e.to_string();
                    tracing::warn!(
                        session_id = %config.session_id,
                        run_id = %config.run_id,
                        tool = %call.name,
                        error = %msg,
                        "工具执行失败"
                    );
                    emit!(RunEvent::ToolCallFailed {
                        tool_call_id: call.id.clone(),
                        tool_name: call.name.clone(),
                        error: msg.clone(),
                    });
                    push_tool_result(messages, &call.id, msg, true);
                }
            }
        }
        // 续轮：messages 已含工具结果，回到循环顶部
        emit!(RunEvent::StatusChanged {
            status: RunStatus::Analyzing
        });
    }

    let e = Error::MaxTurnsExceeded {
        max_turns: config.max_turns,
    };
    emit!(RunEvent::RunFailed {
        error: config.texts.max_turns_exceeded(config.max_turns)
    });
    Err(e)
}

fn push_tool_result(
    messages: &mut Vec<ChatMessage>,
    call_id: &str,
    content: String,
    is_error: bool,
) {
    messages.push(ChatMessage {
        role: Role::User,
        content: vec![MessageContent::ToolResult {
            call_id: call_id.to_string(),
            content,
            is_error,
        }],
    });
}

// ===== 上下文自动压缩（参考 Zed agent thread.rs 的 compact 系列） =====

/// 压缩提示词由 TextPack 提供（texts.compaction_prompt），可随宿主注入替换。
/// 估算消息序列的 token 数：CJK 字符≈1 token/字，其他内容≈1 token/4 字节。
/// 模型端 usage 不可靠（本地 llama.cpp 常缺失），故全链路用同一估算器，宁可可预测。
fn estimate_tokens(messages: &[ChatMessage]) -> u64 {
    fn is_cjk(c: char) -> bool {
        matches!(c as u32,
            0x3000..=0x303F // CJK 标点
            | 0x3400..=0x4DBF // CJK 扩展 A
            | 0x4E00..=0x9FFF // CJK 基本区
            | 0xF900..=0xFAFF // 兼容表意
            | 0xFF00..=0xFFEF) // 全角
    }

    let text_of = |c: &MessageContent| -> String {
        match c {
            MessageContent::Text { text } => text.clone(),
            MessageContent::ToolResult { content, .. } => content.clone(),
            MessageContent::ToolUse { call } => call.arguments.to_string(),
        }
    };

    let mut cjk: u64 = 0;
    let mut other_bytes: u64 = 0;
    for m in messages {
        for c in &m.content {
            for ch in text_of(c).chars() {
                if is_cjk(ch) {
                    cjk += 1;
                } else {
                    other_bytes += ch.len_utf8() as u64;
                }
            }
        }
    }
    cjk + other_bytes.div_ceil(4)
}

/// 最后一条压缩摘要消息的下标
fn last_compaction_ix(messages: &[ChatMessage]) -> Option<usize> {
    messages.iter().rposition(|m| m.role == Role::Compaction)
}

/// 发模型用的裁剪视图：无压缩点 → 全量；有 → 保留预算内的近期用户消息原文 +
/// 压缩摘要 + 压缩点之后的全部消息（对齐 Zed 的 extend_request_history_until）
fn request_view(messages: &[ChatMessage]) -> Vec<ChatMessage> {
    let Some(ix) = last_compaction_ix(messages) else {
        return messages.to_vec();
    };
    let mut view = retained_user_messages_before(messages, ix);
    view.extend_from_slice(&messages[ix..]);
    view
}

/// 压缩点之前、按字节预算从新到旧保留的含文本用户消息
fn retained_user_messages_before(
    messages: &[ChatMessage],
    compaction_ix: usize,
) -> Vec<ChatMessage> {
    fn text_bytes(m: &ChatMessage) -> usize {
        m.content
            .iter()
            .map(|c| match c {
                MessageContent::Text { text } => text.len(),
                _ => 0,
            })
            .sum()
    }
    let mut remaining = RETAINED_USER_BYTE_BUDGET;
    let mut retained = Vec::new();
    for m in messages[..compaction_ix].iter().rev() {
        if m.role != Role::User {
            continue;
        }
        // 只保留含文本的用户输入；纯 tool_result 消息属于已完成轮次，进摘要即可
        let bytes = text_bytes(m);
        if bytes == 0 {
            continue;
        }
        if bytes <= remaining {
            remaining -= bytes;
            retained.push(m.clone());
        } else {
            if remaining > 0 {
                let mut truncated = truncate_messages_text(m, remaining);
                truncated.content.truncate(1); // 截断只保留首个文本块
                if text_bytes(&truncated) > 0 {
                    retained.push(truncated);
                }
            }
            break;
        }
    }
    retained.reverse();
    retained
}

/// 把消息内文本块截断到 max_bytes（超预算时保留近期原文的兜底）
fn truncate_messages_text(m: &ChatMessage, max_bytes: usize) -> ChatMessage {
    let mut out = m.clone();
    for c in &mut out.content {
        if let MessageContent::Text { text } = c {
            if text.len() > max_bytes {
                let mut end = max_bytes;
                while end > 0 && !text.is_char_boundary(end) {
                    end -= 1;
                }
                *text = text[..end].to_string();
            }
        }
    }
    out
}

/// 是否需要压缩：超阈值、内容足够、且不是刚压缩过（防连压循环）
fn should_compact(messages: &[ChatMessage], threshold: u64) -> bool {
    if messages.len() < 3 {
        return false;
    }
    if matches!(messages.last().map(|m| m.role), Some(Role::Compaction)) {
        return false;
    }
    estimate_tokens(&request_view(messages)) >= threshold
}

/// 生成摘要并把 Compaction 消息追加到历史末尾（不删除原消息，裁剪发生在组装请求时）。
/// 返回摘要字节数；取消透传 Cancelled；空摘要视为失败。
async fn compact_context(
    backend: &dyn ModelBackend,
    config: &RunConfig,
    messages: &mut Vec<ChatMessage>,
    cancel: &CancellationToken,
) -> Result<usize> {
    let mut req_messages = request_view(messages);
    req_messages.push(ChatMessage {
        role: Role::User,
        content: vec![MessageContent::text(
            config.texts.compaction_prompt.as_str(),
        )],
    });
    let request = ModelRequest {
        system: config.system_prompt.clone(),
        messages: req_messages,
        tools: vec![],
        temperature: config.temperature,
        max_tokens: config.max_tokens,
    };

    let mut stream = tokio::select! {
        _ = cancel.cancelled() => return Err(Error::Cancelled),
        s = backend.stream_completion(request) => s?,
    };

    let mut summary = String::new();
    loop {
        let item = tokio::select! {
            _ = cancel.cancelled() => return Err(Error::Cancelled),
            item = tokio::time::timeout(STREAM_IDLE_TIMEOUT, stream.next()) => match item {
                Err(_) => {
                    return Err(Error::BudgetExceeded {
                        budget_secs: STREAM_IDLE_TIMEOUT.as_secs(),
                    });
                }
                Ok(None) => break,
                Ok(Some(v)) => v?,
            },
        };
        if let crate::model::Delta::Text { text } = item {
            summary.push_str(&text);
        }
    }

    if summary.trim().is_empty() {
        return Err(Error::CompactionFailed(config.texts.compaction_empty()));
    }
    let len = summary.len();
    messages.push(ChatMessage {
        role: Role::Compaction,
        content: vec![MessageContent::Text { text: summary }],
    });
    Ok(len)
}

/// 单个参数字符串值上限（超出截断并标注，防止大文件内容撑爆事件流/前端）
const ARGS_VALUE_CAP: usize = 20_000;

/// 完整参数快照：字符串值超长时截断。供前端渲染写文件内容 / 完整命令预览。
fn cap_args(args: &serde_json::Value, texts: &TextPack) -> serde_json::Value {
    match args {
        serde_json::Value::Object(map) => {
            let mut out = serde_json::Map::new();
            for (k, v) in map {
                let capped = match v {
                    serde_json::Value::String(s) if s.chars().count() > ARGS_VALUE_CAP => {
                        let t: String = s.chars().take(ARGS_VALUE_CAP).collect();
                        serde_json::Value::String(format!("{t}{}", texts.arg_value_truncated()))
                    }
                    other => other.clone(),
                };
                out.insert(k.clone(), capped);
            }
            serde_json::Value::Object(out)
        }
        other => other.clone(),
    }
}

fn summarize_args(args: &serde_json::Value) -> String {
    match args {
        serde_json::Value::Object(map) => map
            .iter()
            .map(|(k, v)| {
                let v_str = match v {
                    serde_json::Value::String(s) => s.clone(),
                    other => other.to_string(),
                };
                format!("{k}={}", summarize_text(&v_str, 60))
            })
            .collect::<Vec<_>>()
            .join(", "),
        other => summarize_text(&other.to_string(), 120),
    }
}

fn summarize_text(text: &str, max: usize) -> String {
    let trimmed = text.trim();
    if trimmed.chars().count() <= max {
        trimmed.to_string()
    } else {
        let cut: String = trimmed.chars().take(max).collect();
        format!("{cut}…")
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn summarize_args_flat_object() {
        let args = serde_json::json!({"path": "docs/report.md", "n": 3});
        let s = summarize_args(&args);
        assert!(s.contains("path=docs/report.md"));
        assert!(s.contains("n=3"));
    }

    #[test]
    fn summarize_text_truncates() {
        let long = "a".repeat(300);
        let s = summarize_text(&long, 100);
        assert!(s.ends_with('…'));
        assert!(s.chars().count() <= 101);
    }

    // ===== Run 循环集成测试（MockBackend，无网络） =====
    mod loop_tests {
        use super::super::*;
        use crate::model::{Delta, DeltaStream};
        use crate::test_support::*;
        use crate::tool::ToolRegistry as R;
        use async_trait::async_trait;
        use std::sync::Mutex;

        fn policy_all_deny() -> ToolPolicy {
            ToolPolicy {
                allowed_readonly: vec![],
                confirm_tools: vec![],
            }
        }

        #[tokio::test]
        async fn sequence_starts_at_one_and_monotonically_increases() {
            // Contract: sequence is 1-based, strictly increasing, no gaps,
            // and event_id matches the derived format (docs/TESTS.md §2).
            let backend = Arc::new(MockBackend::new(vec![text_turn("ok")]));
            let (result, events) = run_execute(
                backend,
                R::new(),
                text_config("s1", "", policy_all_deny()),
                None,
                deny_bridge(),
                vec![user_msg("hi")],
            )
            .await;
            assert!(result.is_ok());
            assert!(!events.is_empty());
            for (i, env) in events.iter().enumerate() {
                assert_eq!(env.sequence, (i + 1) as u64, "sequence 必须 1 起严格递增");
                assert_eq!(env.event_id, format!("evt_{}_{}", env.run_id, env.sequence));
            }
            let ids: std::collections::HashSet<_> =
                events.iter().map(|e| e.event_id.clone()).collect();
            assert_eq!(ids.len(), events.len(), "event_id 不得重复");
        }

        #[tokio::test]
        async fn text_only_run_completes_with_events() {
            let backend = Arc::new(MockBackend::new(vec![text_turn("你好，世界")]));
            let (result, events) = run_execute(
                backend,
                R::new(),
                text_config("s1", "系统提示", policy_all_deny()),
                None,
                deny_bridge(),
                vec![user_msg("打个招呼")],
            )
            .await;

            let messages = result.expect("应成功");
            assert_eq!(messages.len(), 2);
            assert!(matches!(&messages[0].role, Role::User));
            assert!(matches!(&messages[1].role, Role::Assistant));
            assert_eq!(messages[1].content.len(), 1);
            match &messages[1].content[0] {
                MessageContent::Text { text } => assert_eq!(text, "你好，世界"),
                other => panic!("预期文本块，实际: {other:?}"),
            }

            // 事件序：状态 → 文本增量 → 完成
            assert_sequence_contiguous(&events);
            assert!(
                matches!(
                    events[0].event,
                    RunEvent::StatusChanged {
                        status: RunStatus::Analyzing
                    }
                ),
                "首个事件应为 Analyzing，实际: {:?}",
                events[0].event
            );
            assert!(events.iter().any(
                |e| matches!(&e.event, RunEvent::MessageDelta { text } if text == "你好，世界")
            ));
            assert!(events.iter().any(|e| matches!(
                &e.event,
                RunEvent::RunCompleted { final_text } if final_text == "你好，世界"
            )));
        }

        #[tokio::test]
        async fn tool_loop_executes_and_returns_final_text() {
            let log = Arc::new(Mutex::new(Vec::new()));
            let tool = EchoTool::ok("read_file", "文件内容", log.clone());
            let backend = Arc::new(MockBackend::new(vec![
                vec![tool_call_delta("t1", "read_file")],
                text_turn("搞定了"),
            ]));
            let registry = R::new().register(tool);

            let (result, events) = run_execute(
                backend,
                registry,
                text_config("s1", "", ToolPolicy::default()),
                None,
                deny_bridge(),
                vec![user_msg("读取")],
            )
            .await;

            let messages = result.expect("应成功");
            assert_eq!(
                messages.len(),
                4,
                "user → assistant(tool_use) → user(tool_result) → assistant(text)"
            );
            assert!(matches!(
                &messages[1].content[0],
                MessageContent::ToolUse { .. }
            ));
            match &messages[2].content[0] {
                MessageContent::ToolResult {
                    call_id,
                    content,
                    is_error,
                } => {
                    assert_eq!(call_id, "t1");
                    assert_eq!(content, "文件内容");
                    assert!(!*is_error);
                }
                other => panic!("预期 tool_result，实际: {other:?}"),
            }
            assert!(matches!(
                &messages[3].content[0],
                MessageContent::Text { text } if text == "搞定了"
            ));

            // 工具确实执行了
            assert_eq!(log.lock().unwrap().len(), 1);

            // 事件序：started 在 completed 之前，最终 RunCompleted
            assert_sequence_contiguous(&events);
            let started_idx = events
                .iter()
                .position(|e| matches!(&e.event, RunEvent::ToolCallStarted { tool_call_id, .. } if tool_call_id == "t1"))
                .expect("应有 ToolCallStarted");
            let completed_idx = events
                .iter()
                .position(|e| matches!(&e.event, RunEvent::ToolCallCompleted { tool_call_id, .. } if tool_call_id == "t1"))
                .expect("应有 ToolCallCompleted");
            assert!(started_idx < completed_idx);
            assert!(events.iter().any(|e| matches!(
                &e.event,
                RunEvent::RunCompleted { final_text } if final_text == "搞定了"
            )));
        }

        #[tokio::test]
        async fn unregistered_tool_fails_but_run_continues() {
            let backend = Arc::new(MockBackend::new(vec![
                vec![tool_call_delta("t1", "不存在工具")],
                text_turn("收到错误"),
            ]));
            let registry = R::new(); // 空：任何调用都未注册

            let (result, events) = run_execute(
                backend,
                registry,
                text_config("s1", "", policy_all_deny()),
                None,
                deny_bridge(),
                vec![user_msg("调用")],
            )
            .await;

            let messages = result.expect("Run 不应中断");
            assert_eq!(messages.len(), 4);
            match &messages[2].content[0] {
                MessageContent::ToolResult {
                    is_error, content, ..
                } => {
                    assert!(*is_error);
                    assert!(content.contains("未注册的工具"));
                }
                other => panic!("预期 tool_result，实际: {other:?}"),
            }
            assert!(events.iter().any(|e| matches!(
                &e.event,
                RunEvent::ToolCallFailed { tool_name, .. } if tool_name == "不存在工具"
            )));
        }

        #[tokio::test]
        async fn tool_error_is_returned_to_model_not_abort_run() {
            let log = Arc::new(Mutex::new(Vec::new()));
            let tool = EchoTool::failing("read_file", "磁盘故障", log.clone());
            let backend = Arc::new(MockBackend::new(vec![
                vec![tool_call_delta("t1", "read_file")],
                text_turn("已处理错误"),
            ]));

            let (result, events) = run_execute(
                backend,
                R::new().register(tool),
                text_config("s1", "", ToolPolicy::default()),
                None,
                deny_bridge(),
                vec![user_msg("读取")],
            )
            .await;

            assert!(result.is_ok(), "工具失败不中断 Run");
            let messages = result.unwrap();
            match &messages[2].content[0] {
                MessageContent::ToolResult {
                    content, is_error, ..
                } => {
                    assert!(*is_error);
                    assert!(content.contains("磁盘故障"), "错误文本应回传模型");
                }
                other => panic!("预期 tool_result，实际: {other:?}"),
            }
            assert!(events.iter().any(|e| matches!(
                &e.event,
                RunEvent::ToolCallFailed { error, .. } if error.contains("磁盘故障")
            )));
        }

        #[tokio::test]
        async fn deny_policy_blocks_execution_without_confirm() {
            let log = Arc::new(Mutex::new(Vec::new()));
            let tool = EchoTool::ok("write_file", "不应执行", log.clone());
            let backend = Arc::new(MockBackend::new(vec![
                vec![tool_call_delta("t1", "write_file")],
                text_turn("好"),
            ]));

            let (result, events) = run_execute(
                backend,
                R::new().register(tool),
                text_config("s1", "", policy_all_deny()),
                None,
                deny_bridge(),
                vec![user_msg("写")],
            )
            .await;

            assert!(result.is_ok());
            // 工具从未执行
            assert!(log.lock().unwrap().is_empty(), "Deny 策略下不应执行");
            // 失败事件带策略拒绝文案，且未发 PermissionRequested
            assert!(events.iter().any(|e| matches!(
                &e.event,
                RunEvent::ToolCallFailed { error, .. } if error.contains("安全策略拒绝")
            )));
            assert!(!events
                .iter()
                .any(|e| matches!(&e.event, RunEvent::PermissionRequested { .. })));
        }

        #[tokio::test]
        async fn confirm_allow_executes_tool() {
            let log = Arc::new(Mutex::new(Vec::new()));
            let tool = EchoTool::ok("write_file", "已写入", log.clone());
            let backend = Arc::new(MockBackend::new(vec![
                vec![tool_call_delta("t1", "write_file")],
                text_turn("完成"),
            ]));

            let (result, events) = run_execute(
                backend,
                R::new().register(tool),
                text_config("s1", "", ToolPolicy::default()), // write_file = Confirm
                None,
                allow_bridge(),
                vec![user_msg("写")],
            )
            .await;

            assert!(result.is_ok());
            assert_eq!(log.lock().unwrap().len(), 1, "批准后应执行");
            // 确认流程事件完整：requested → decided(allow) → completed
            let requested_idx = events
                .iter()
                .position(|e| matches!(&e.event, RunEvent::PermissionRequested { tool_call_id, .. } if tool_call_id == "t1"))
                .expect("应有 PermissionRequested");
            let decided_idx = events
                .iter()
                .position(|e| {
                    matches!(
                        &e.event,
                        RunEvent::PermissionDecided {
                            decision: Decision::Allow,
                            ..
                        }
                    )
                })
                .expect("应有 PermissionDecided(Allow)");
            let completed_idx = events
                .iter()
                .position(|e| matches!(&e.event, RunEvent::ToolCallCompleted { tool_call_id, .. } if tool_call_id == "t1"))
                .expect("应有 ToolCallCompleted");
            assert!(requested_idx < decided_idx && decided_idx < completed_idx);
        }

        #[tokio::test]
        async fn confirm_deny_blocks_execution() {
            let log = Arc::new(Mutex::new(Vec::new()));
            let tool = EchoTool::ok("write_file", "不应执行", log.clone());
            let backend = Arc::new(MockBackend::new(vec![
                vec![tool_call_delta("t1", "write_file")],
                text_turn("好"),
            ]));

            let (result, events) = run_execute(
                backend,
                R::new().register(tool),
                text_config("s1", "", ToolPolicy::default()),
                None,
                deny_bridge(),
                vec![user_msg("写")],
            )
            .await;

            assert!(result.is_ok());
            assert!(log.lock().unwrap().is_empty(), "用户拒绝后不应执行");
            assert!(events.iter().any(|e| matches!(
                &e.event,
                RunEvent::ToolCallFailed { error, .. } if error.contains("用户拒绝")
            )));
            assert!(events.iter().any(|e| matches!(
                &e.event,
                RunEvent::PermissionDecided {
                    decision: Decision::Deny,
                    ..
                }
            )));
        }

        #[tokio::test]
        async fn cancelled_before_start_never_calls_backend() {
            let backend = Arc::new(MockBackend::new(vec![text_turn("不应到达")]));
            let token = tokio_util::sync::CancellationToken::new();
            token.cancel();

            let (result, events) = run_execute_with_cancel(
                backend,
                R::new(),
                text_config("s1", "", policy_all_deny()),
                None,
                deny_bridge(),
                vec![user_msg("你好")],
                token,
            )
            .await;

            assert!(matches!(result, Err(Error::Cancelled)));
            assert!(
                events
                    .iter()
                    .any(|e| matches!(&e.event, RunEvent::RunCancelled)),
                "应有 RunCancelled 事件"
            );
        }

        #[tokio::test]
        async fn cancel_mid_stream_stops_and_emits_event() {
            let backend: Arc<dyn ModelBackend> = Arc::new(HangingBackend);
            let token = tokio_util::sync::CancellationToken::new();
            let config = text_config("s1", "", policy_all_deny());
            let messages = vec![user_msg("写长文")];

            let (tx, mut rx) = mpsc::channel(1024);
            let runner = tokio::spawn(execute(
                backend,
                R::new(),
                config,
                None,
                deny_bridge(),
                messages,
                tx,
                token.clone(),
            ));

            // 等首个增量事件到达（HangingBackend 先产出 1 个文本增量后挂起）
            wait_until(
                || {
                    rx.try_recv()
                        .ok()
                        .map(|e| matches!(e.event, RunEvent::MessageDelta { .. }))
                        .unwrap_or(false)
                },
                "等待首个文本增量",
            )
            .await;
            token.cancel();

            let result = runner.await.unwrap();
            assert!(matches!(result, Err(Error::Cancelled)));
            // 排干剩余事件确认 RunCancelled
            let mut cancelled = false;
            while let Some(e) = rx.recv().await {
                if matches!(e.event, RunEvent::RunCancelled) {
                    cancelled = true;
                }
            }
            assert!(cancelled, "取消后应有 RunCancelled 终态事件");
        }

        #[tokio::test]
        async fn budget_exceeded_after_max_turns() {
            // 每轮都返回工具调用 → max_turns 后报 MaxTurnsExceeded（显式配置，不依赖默认值）
            let log = Arc::new(Mutex::new(Vec::new()));
            let tool = EchoTool::ok("read_file", "ok", log.clone());
            let turn = vec![tool_call_delta("t", "read_file")];
            let backend = Arc::new(MockBackend::new(vec![turn; 30]));

            let mut cfg = text_config("s1", "", ToolPolicy::default());
            cfg.max_turns = 20;

            let (result, events) = run_execute(
                backend,
                R::new().register(tool),
                cfg,
                None,
                deny_bridge(),
                vec![user_msg("循环")],
            )
            .await;

            match &result {
                Err(Error::MaxTurnsExceeded { max_turns }) => {
                    assert_eq!(*max_turns, 20);
                }
                other => panic!("预期 MaxTurnsExceeded，实际: {other:?}"),
            }
            assert_eq!(log.lock().unwrap().len(), 20, "应恰好跑满 max_turns 轮");
            assert!(events
                .iter()
                .any(|e| matches!(&e.event, RunEvent::RunFailed { .. })));
        }

        #[tokio::test]
        async fn envelopes_carry_session_and_run_ids() {
            let backend = Arc::new(MockBackend::new(vec![text_turn("ok")]));
            let (result, events) = run_execute(
                backend,
                R::new(),
                text_config("sess-x", "", policy_all_deny()),
                None,
                deny_bridge(),
                vec![user_msg("hi")],
            )
            .await;
            assert!(result.is_ok());
            for env in &events {
                assert_eq!(env.session_id, "sess-x");
                assert!(
                    env.run_id.starts_with("run_"),
                    "run_id 形如 run_*: {}",
                    env.run_id
                );
            }
            // 同一 Run 内 run_id 一致
            let run_ids: std::collections::HashSet<_> =
                events.iter().map(|e| e.run_id.clone()).collect();
            assert_eq!(run_ids.len(), 1);
        }

        #[tokio::test]
        async fn multiple_tool_calls_in_one_turn_execute_sequentially() {
            let log = Arc::new(Mutex::new(Vec::new()));
            let tool = EchoTool::ok("read_file", "ok", log.clone());
            let backend = Arc::new(MockBackend::new(vec![
                vec![
                    tool_call_delta("t1", "read_file"),
                    tool_call_delta("t2", "read_file"),
                ],
                text_turn("都完成了"),
            ]));

            let (result, events) = run_execute(
                backend,
                R::new().register(tool),
                text_config("s1", "", ToolPolicy::default()),
                None,
                deny_bridge(),
                vec![user_msg("两次读取")],
            )
            .await;

            let messages = result.expect("应成功");
            // assistant(2 tool_use) → user(result t1) → user(result t2) → assistant(text)
            assert_eq!(messages.len(), 5);
            assert_eq!(
                messages[1].content.len(),
                2,
                "两个 tool_use 同属一条助手消息"
            );
            match &messages[2].content[0] {
                MessageContent::ToolResult { call_id, .. } => assert_eq!(call_id, "t1"),
                other => panic!("预期 tool_result，实际: {other:?}"),
            }
            match &messages[3].content[0] {
                MessageContent::ToolResult { call_id, .. } => assert_eq!(call_id, "t2"),
                other => panic!("预期 tool_result，实际: {other:?}"),
            }
            assert_eq!(log.lock().unwrap().len(), 2);
            // 两个 started 事件都存在
            let started: Vec<_> = events
                .iter()
                .filter_map(|e| match &e.event {
                    RunEvent::ToolCallStarted { tool_call_id, .. } => Some(tool_call_id.clone()),
                    _ => None,
                })
                .collect();
            assert_eq!(started, vec!["t1".to_string(), "t2".to_string()]);
        }

        #[tokio::test]
        async fn assistant_turn_text_before_tool_use_is_preserved() {
            // 模型先说话再调工具：文本块与 tool_use 同存于助手消息
            let log = Arc::new(Mutex::new(Vec::new()));
            let tool = EchoTool::ok("read_file", "ok", log.clone());
            let backend = Arc::new(MockBackend::new(vec![
                vec![
                    Delta::Text {
                        text: "让我先看看文件".into(),
                    },
                    tool_call_delta("t1", "read_file"),
                ],
                text_turn("完成"),
            ]));

            let (result, _) = run_execute(
                backend,
                R::new().register(tool),
                text_config("s1", "", ToolPolicy::default()),
                None,
                deny_bridge(),
                vec![user_msg("读")],
            )
            .await;

            let messages = result.expect("应成功");
            assert_eq!(messages[1].content.len(), 2);
            assert!(
                matches!(&messages[1].content[0], MessageContent::Text { text } if text == "让我先看看文件")
            );
            assert!(matches!(
                &messages[1].content[1],
                MessageContent::ToolUse { .. }
            ));
        }

        #[tokio::test]
        async fn backend_error_propagates_as_run_failed() {
            struct FailingBackend;
            #[async_trait]
            impl ModelBackend for FailingBackend {
                async fn stream_completion(
                    &self,
                    _req: ModelRequest,
                ) -> crate::Result<DeltaStream> {
                    Err(Error::Model("网关 502".into()))
                }
            }

            let (result, events) = run_execute(
                Arc::new(FailingBackend),
                R::new(),
                text_config("s1", "", policy_all_deny()),
                None,
                deny_bridge(),
                vec![user_msg("hi")],
            )
            .await;

            match &result {
                Err(Error::Model(m)) => assert!(m.contains("502")),
                other => panic!("预期 Model 错误，实际: {other:?}"),
            }
            assert!(events.iter().any(|e| matches!(
                &e.event,
                RunEvent::RunFailed { error } if error.contains("502")
            )));
        }

        #[tokio::test]
        async fn mid_stream_error_yields_run_failed() {
            struct ErrMidStream;
            #[async_trait]
            impl ModelBackend for ErrMidStream {
                async fn stream_completion(
                    &self,
                    _req: ModelRequest,
                ) -> crate::Result<DeltaStream> {
                    let stream = futures::stream::iter(vec![
                        Ok(Delta::Text {
                            text: "开头".into(),
                        }),
                        Err(Error::Model("流中断".into())),
                    ]);
                    Ok(Box::pin(stream))
                }
            }

            let (result, events) = run_execute(
                Arc::new(ErrMidStream),
                R::new(),
                text_config("s1", "", policy_all_deny()),
                None,
                deny_bridge(),
                vec![user_msg("hi")],
            )
            .await;

            match &result {
                Err(Error::Model(m)) => assert!(m.contains("流中断"), "实际错误: {m}"),
                other => panic!("期望 Err(Error::Model(\"流中断\"))，实际为 {other:?}"),
            }
            assert!(events
                .iter()
                .any(|e| matches!(&e.event, RunEvent::RunFailed { .. })));
        }
    }

    // ===== 上下文自动压缩 =====
    mod compaction_tests {
        use super::super::*;
        use crate::test_support::*;
        use crate::tool::ToolRegistry as R;

        fn msg(role: Role, text: &str) -> ChatMessage {
            ChatMessage {
                role,
                content: vec![MessageContent::text(text)],
            }
        }

        // ----- estimate_tokens -----

        #[test]
        fn estimate_tokens_cjk_counts_one_per_char() {
            // 5 个汉字 + 1 个全角标点 ≈ 6 token
            let msgs = vec![msg(Role::User, "你好世界呀！")];
            assert_eq!(estimate_tokens(&msgs), 6);
        }

        #[test]
        fn estimate_tokens_ascii_counts_one_per_four_bytes() {
            let msgs = vec![msg(Role::User, "abcdefgh")]; // 8 字节 → 2 token
            assert_eq!(estimate_tokens(&msgs), 2);
        }

        #[test]
        fn estimate_tokens_counts_tool_result_and_tool_use() {
            let call_id = "c1";
            let mut messages = vec![
                msg(Role::User, "读取文件"),
                ChatMessage {
                    role: Role::Assistant,
                    content: vec![MessageContent::ToolUse {
                        call: ToolCall {
                            id: call_id.into(),
                            call_id: None,
                            name: "read_file".into(),
                            arguments: serde_json::json!({"path": "abcdefgh"}), // 18 字节 JSON
                        },
                    }],
                },
                ChatMessage {
                    role: Role::User,
                    content: vec![MessageContent::ToolResult {
                        call_id: call_id.into(),
                        content: "12345678".into(), // 8 字节 → 2
                        is_error: false,
                    }],
                },
            ];
            let _ = &mut messages;
            // user "读取文件"=4；tool_use 参数 JSON ≈ {"path":"abcdefgh"} 18 字节；
            // JSON 序列化键名/引号也算字节 → 至少 4（ceil(18/4)=5）
            let est = estimate_tokens(&messages);
            assert!(est >= 4 + 5, "估算应覆盖工具参数与结果，实际 {est}");
        }

        // ----- request_view -----

        #[test]
        fn request_view_without_compaction_point_is_full_copy() {
            let msgs = vec![msg(Role::User, "一"), msg(Role::Assistant, "二")];
            let view = request_view(&msgs);
            assert_eq!(view.len(), 2);
        }

        #[test]
        fn request_view_trims_history_before_compaction_point() {
            let mut msgs = vec![
                msg(Role::User, &"旧".repeat(40_000)), // 120KB，超预算，被丢弃
                msg(Role::Assistant, "旧回复"),
                msg(Role::User, "近期问题"), // 预算内，保留
                msg(Role::Assistant, "近期回复"),
                msg(Role::Compaction, "【摘要】 everything"), // 压缩点
                msg(Role::User, "压缩后的问题"),
            ];
            msgs[1].content.clear(); // assistant 文本不影响保留逻辑
            let view = request_view(&msgs);
            // 保留：旧消息截断到剩余预算 79_988 字节 + 近期用户原文 + 摘要 + 压缩点之后全部
            assert_eq!(view.len(), 4);
            match &view[0].content[0] {
                MessageContent::Text { text } => {
                    // 截到剩余预算 79_988 字节内的字符边界（"旧"=3字节 → 79_986）
                    assert_eq!(text.len(), 79_986);
                }
                other => panic!("应为文本块，实际 {other:?}"),
            }
            assert!(matches!(view[1].role, Role::User));
            assert!(matches!(view[2].role, Role::Compaction));
            assert!(matches!(view[3].role, Role::User));
            match &view[2].content[0] {
                MessageContent::Text { text } => assert!(text.contains("【摘要】")),
                other => panic!("摘要应为文本块，实际 {other:?}"),
            }
        }

        #[test]
        fn request_view_retains_budget_and_truncates_overflow() {
            // 近期用户消息超过 80KB 预算：前一条被丢弃，后一条被截断保留
            let big1 = "a".repeat(60_000);
            let big2 = "b".repeat(60_000);
            let msgs = vec![
                msg(Role::User, &big1),
                msg(Role::Assistant, "x"),
                msg(Role::User, &big2),
                msg(Role::Compaction, "摘要"),
            ];
            let view = request_view(&msgs);
            // 预算 80_000：big2(60_000) 放入后剩 20_000，big1 超出 → 截断到 20_000 保留
            assert_eq!(view.len(), 3); // big1截断 + big2 + 摘要
            match &view[0].content[0] {
                MessageContent::Text { text } => assert_eq!(text.len(), 20_000),
                other => panic!("应为文本块，实际 {other:?}"),
            }
            match &view[1].content[0] {
                MessageContent::Text { text } => assert_eq!(text.len(), 60_000),
                other => panic!("应为文本块，实际 {other:?}"),
            }
        }

        // ----- should_compact -----

        #[test]
        fn should_compact_below_threshold_is_false() {
            let msgs = vec![
                msg(Role::User, "短"),
                msg(Role::Assistant, "答"),
                msg(Role::User, "再问"),
            ];
            assert!(!should_compact(&msgs, 1_000_000));
        }

        #[test]
        fn should_compact_above_threshold_is_true() {
            let msgs = vec![
                msg(Role::User, "长".repeat(100).as_str()),
                msg(Role::Assistant, "答"),
                msg(Role::User, "再问"),
            ];
            assert!(should_compact(&msgs, 10));
        }

        #[test]
        fn should_compact_skips_when_last_is_compaction() {
            let msgs = vec![
                msg(Role::User, "长".repeat(100).as_str()),
                msg(Role::Assistant, "答"),
                msg(Role::Compaction, "摘要"),
            ];
            assert!(!should_compact(&msgs, 10));
        }

        #[test]
        fn should_compact_needs_at_least_three_messages() {
            let msgs = vec![msg(Role::User, "长".repeat(100).as_str())];
            assert!(!should_compact(&msgs, 10));
        }

        // ----- 集成：run_loop 中触发压缩 -----

        #[tokio::test]
        async fn run_loop_compacts_when_threshold_exceeded() {
            // 队列：工具调用 → 压缩摘要 → 最终回答
            let backend = Arc::new(MockBackend::new(vec![
                vec![tool_call_delta("t1", "read_file")],
                text_turn("【压缩摘要】用户在读文件"),
                text_turn("最终回答"),
            ]));
            let registry = R::new().register(EchoTool::ok(
                "read_file",
                "文件内容",
                Arc::new(std::sync::Mutex::new(Vec::new())),
            ));

            let mut cfg = text_config("s1", "", ToolPolicy::default());
            cfg.auto_compact_tokens = Some(1); // 强制触发

            let (result, events) = run_execute(
                backend.clone(),
                registry,
                cfg,
                None,
                deny_bridge(),
                vec![user_msg("读取 a.txt")],
            )
            .await;

            let messages = result.expect("应成功");
            // user → assistant(tool_use) → user(tool_result) → compaction → assistant(final)
            assert_eq!(messages.len(), 5, "实际: {messages:?}");
            assert!(matches!(&messages[3].role, Role::Compaction));
            match &messages[3].content[0] {
                MessageContent::Text { text } => assert!(text.contains("压缩摘要")),
                other => panic!("压缩消息应为文本块，实际 {other:?}"),
            }
            assert!(matches!(&messages[4].role, Role::Assistant));

            // 3 次模型请求：正常轮 + 压缩轮 + 最终轮
            assert_eq!(
                backend.call_count.load(std::sync::atomic::Ordering::SeqCst),
                3
            );

            assert!(events.iter().any(|e| matches!(
                &e.event,
                RunEvent::StatusChanged {
                    status: RunStatus::Compacting
                }
            )));
            assert!(events
                .iter()
                .any(|e| matches!(&e.event, RunEvent::CompactionCompleted)));
            assert!(events
                .iter()
                .any(|e| matches!(&e.event, RunEvent::RunCompleted { final_text } if final_text == "最终回答")));
            assert_sequence_contiguous(&events);
        }

        #[tokio::test]
        async fn run_loop_disables_compaction_after_failure_but_run_continues() {
            // 压缩轮返回空文本 → CompactionFailed → 本 Run 禁用压缩，继续正常执行
            let backend = Arc::new(MockBackend::new(vec![
                vec![tool_call_delta("t1", "read_file")],
                text_turn("   "), // 空摘要 → 失败
                text_turn("最终回答"),
            ]));
            let registry = R::new().register(EchoTool::ok(
                "read_file",
                "文件内容",
                Arc::new(std::sync::Mutex::new(Vec::new())),
            ));

            let mut cfg = text_config("s1", "", ToolPolicy::default());
            cfg.auto_compact_tokens = Some(1);

            let (result, events) = run_execute(
                backend.clone(),
                registry,
                cfg,
                None,
                deny_bridge(),
                vec![user_msg("读取 a.txt")],
            )
            .await;

            let messages = result.expect("压缩失败不应中断 Run");
            // 无压缩消息，照常完成
            assert!(
                !messages.iter().any(|m| matches!(m.role, Role::Compaction)),
                "不应有压缩消息"
            );
            assert!(matches!(&messages.last().unwrap().role, Role::Assistant));
            assert_eq!(
                backend.call_count.load(std::sync::atomic::Ordering::SeqCst),
                3
            );
            // 有 Compacting 状态但无 CompactionCompleted
            assert!(events.iter().any(|e| matches!(
                &e.event,
                RunEvent::StatusChanged {
                    status: RunStatus::Compacting
                }
            )));
            assert!(!events
                .iter()
                .any(|e| matches!(&e.event, RunEvent::CompactionCompleted)));
        }
    }
}
