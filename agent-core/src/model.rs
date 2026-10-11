//! 模型适配层：`ModelBackend` trait + OpenAI 兼容实现。
//!
//! 抽象边界：后端只需要把“请求（消息 + 工具定义）”变成“增量流”。
//! 流式解析在 openai_sse 模块（自研 SSE 客户端）：文本/思考实时透出，
//! 工具调用按 openai 协议以片段形式透出（首片段带 id+name，后续只带参数片段），
//! 同一调用的片段拼接即完整参数 JSON。

use async_trait::async_trait;
use futures::{Stream, StreamExt};
use serde::{Deserialize, Serialize};
use serde_json::json;
use std::collections::VecDeque;
use std::pin::Pin;

use crate::error::Result;
use crate::openai_sse::{self, SseDelta};
use crate::tool::{ToolCall, ToolDefinition};

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

/// 模型流式产出的增量。
/// 工具调用以片段形式透出：`ToolCallStart`（首片段，带 id+name）→
/// 0..n 个 `ToolCallArgsDelta`（参数 JSON 片段）→ 流结束时的
/// `ToolCalls`（拼装后的完整调用，上层消费入口不变）。
#[derive(Debug, Clone)]
pub enum Delta {
    Text {
        text: String,
    },
    Reasoning {
        text: String,
    },
    /// 工具调用首片段：id + name 已知（此后同 id 的参数片段跟在后面）
    ToolCallStart {
        id: String,
        name: String,
    },
    /// 工具调用参数增量（原始 JSON 片段；name 随事件携带，前端无需自维护映射）
    ToolCallArgsDelta {
        id: String,
        name: String,
        fragment: String,
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

// ===== 自研 OpenAI 兼容实现（替代 rig 流式层） =====

/// OpenAI 兼容后端（自研 SSE 流式客户端）。
/// 覆盖：云端 OpenAI 兼容 API、第三方网关、本地 llama.cpp server。
/// 与 rig 实现的区别：工具调用参数片段实时透出（写文件流式可见）。
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
pub fn should_retry_model_error(msg: &str) -> bool {
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

/// 工具调用片段聚合器：按 openai 协议的 index 聚合同一调用的片段，
/// 流结束时产出完整 ToolCall（对齐原 rig 行为，上层消费入口不变）。
#[derive(Default)]
struct ToolCallAssembler {
    /// index → (id, name, args 累计)
    parts: std::collections::BTreeMap<u32, (String, String, String)>,
}

impl ToolCallAssembler {
    fn ingest(
        &mut self,
        index: u32,
        id: Option<String>,
        name: Option<String>,
        args_fragment: &str,
    ) -> Option<Delta> {
        let entry = self.parts.entry(index).or_default();
        if let Some(i) = id {
            entry.0 = i;
        }
        if let Some(n) = name {
            entry.1 = n;
        }
        entry.2.push_str(args_fragment);
        let (id, name) = (entry.0.clone(), entry.1.clone());
        if id.is_empty() || name.is_empty() {
            return None; // id/name 未到齐，不发事件
        }
        if args_fragment.is_empty() {
            Some(Delta::ToolCallStart { id, name })
        } else {
            Some(Delta::ToolCallArgsDelta {
                id,
                name,
                fragment: args_fragment.to_string(),
            })
        }
    }

    fn finish(self) -> Vec<ToolCall> {
        self.parts
            .into_values()
            .filter(|(id, name, _)| !id.is_empty() && !name.is_empty())
            .filter_map(|(id, name, args)| {
                serde_json::from_str(&args)
                    .map(|arguments| ToolCall {
                        id,
                        call_id: None,
                        name,
                        arguments,
                    })
                    .map_err(|e| tracing::warn!("工具参数 JSON 拼装失败（已回传错误给模型）: {e}"))
                    .ok()
            })
            .collect()
    }
}

#[async_trait]
impl ModelBackend for RigBackend {
    async fn stream_completion(&self, request: ModelRequest) -> Result<DeltaStream> {
        let client = reqwest::Client::new();
        let mut body = openai_sse::build_request_body(&request);
        body["model"] = json!(self.model_name);
        if !request.system.is_empty() {
            // system 提示插入消息列表头部
            let mut msgs = vec![serde_json::json!({"role": "system", "content": request.system})];
            msgs.extend(body["messages"].as_array().cloned().unwrap_or_default());
            body["messages"] = serde_json::json!(msgs);
        }
        let byte_stream =
            openai_sse::open_stream(&client, &self.base_url, &self.api_key, &body).await?;

        // SSE 解析状态机：字节流 → 行 → data 载荷 → SseDelta → Delta
        Ok(Box::pin(stream_state_machine(byte_stream)))
    }
}

/// SSE 解析状态机：把字节流转成 Delta 流。
/// 跨 chunk 保持缓冲（不完整行）与工具参数聚合；
/// 流自然结束时（无论是否收到 finish_reason）都 flush 完整 ToolCall + TurnEnd。
fn stream_state_machine(
    byte_stream: impl futures::Stream<Item = Result<bytes::Bytes>> + Unpin,
) -> impl futures::Stream<Item = Result<Delta>> {
    #[derive(Default)]
    struct Queue(VecDeque<Result<Delta>>);

    futures::stream::unfold(
        (
            byte_stream,
            String::new(),
            Some(ToolCallAssembler::default()),
            Queue::default(),
        ),
        |(mut stream, mut buf, mut assembler, mut queue)| async move {
            loop {
                // 队列非空先吐存量
                if let Some(d) = queue.0.pop_front() {
                    return Some((d, (stream, buf, assembler, queue)));
                }
                let Some(item) = stream.next().await else {
                    // 字节流结束：flush 完整 ToolCall + TurnEnd（只一次）
                    if let Some(asm) = assembler.take() {
                        let mut out: VecDeque<Result<Delta>> = asm
                            .finish()
                            .into_iter()
                            .map(|call| Ok(Delta::ToolCalls { calls: vec![call] }))
                            .collect();
                        out.push_back(Ok(Delta::TurnEnd));
                        if let Some(d) = out.pop_front() {
                            queue = Queue(out);
                            let _ = d;
                            continue;
                        }
                    }
                    return None;
                };
                match item {
                    Err(e) => return Some((Err(e), (stream, buf, assembler, queue))),
                    Ok(bytes) => {
                        buf.push_str(&String::from_utf8_lossy(&bytes));
                        let (lines, rest) = {
                            let (l, r) = openai_sse::split_sse_lines(&buf);
                            (l, r)
                        };
                        buf = rest;
                        let Some(asm) = assembler.as_mut() else {
                            continue; // 已 flush 过（异常尾包），丢弃
                        };
                        for line in lines {
                            let Some(data) = openai_sse::sse_data_of(&line) else {
                                continue;
                            };
                            match openai_sse::parse_sse_data(data) {
                                Ok(Some(SseDelta::Text { text })) => {
                                    queue.0.push_back(Ok(Delta::Text { text }))
                                }
                                Ok(Some(SseDelta::Reasoning { text })) => {
                                    queue.0.push_back(Ok(Delta::Reasoning { text }))
                                }
                                Ok(Some(SseDelta::ToolCallFragment {
                                    index,
                                    id,
                                    name,
                                    args_fragment,
                                })) => {
                                    if let Some(d) = asm.ingest(index, id, name, &args_fragment) {
                                        queue.0.push_back(Ok(d));
                                    }
                                }
                                Ok(Some(SseDelta::Finish)) | Ok(None) => {}
                                Err(e) => queue.0.push_back(Err(e)),
                            }
                        }
                    }
                }
            }
        },
    )
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
