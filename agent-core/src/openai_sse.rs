//! OpenAI 兼容 chat completions 的 SSE 流式客户端（自研，替代 rig 0.22 流式层）。
//!
//! 为什么不用 rig：rig 0.22 的 openai streaming 在内部累积工具参数片段，
//! 只在流结束时一次性 yield 完整 ToolCall——写大文件期间前端拿不到任何增量。
//! 自研客户端直接消费 SSE chunk，把三种增量实时透出：
//! 文本（delta.content）、思考（delta.reasoning_content / delta.reasoning）、
//! 工具参数片段（delta.tool_calls[].function.arguments）。
//!
//! 协议范围：OpenAI chat completions 兼容端点（火山方舟 / DeepSeek / 第三方网关 /
//! 本地 llama.cpp server）。错误重试语义与原 rig 实现保持一致。

use serde_json::{json, Value};

use crate::error::{Error, Result};
use crate::model::{ChatMessage, MessageContent, ModelRequest, Role};

// ===== 请求构造 =====

/// 把平铺消息块序列转成 OpenAI messages 数组（单遍，严格保序）。
/// - User 文本 → user；User ToolResult → role:"tool"（openai 协议要求独立消息，
///   排在同一条 user 消息的文本之后）
/// - Assistant 文本/ToolUse → assistant（ToolUse 进 tool_calls 数组）
/// - Compaction → user + 前缀说明（与原 rig 实现一致）
/// - System → system；空消息列表回退空 user
pub fn to_openai_messages(messages: &[ChatMessage]) -> Vec<Value> {
    if messages.is_empty() {
        return vec![json!({"role": "user", "content": ""})];
    }
    let mut out: Vec<Value> = Vec::with_capacity(messages.len() + 2);
    for m in messages {
        match m.role {
            Role::System => {
                let text = texts_of(&m.content);
                out.push(json!({"role": "system", "content": text}));
            }
            Role::Compaction => {
                let summary = texts_of(&m.content);
                out.push(json!({"role": "user", "content": Role::compaction_prefixed(&summary)}));
            }
            Role::User => {
                let mut texts: Vec<String> = Vec::new();
                for c in &m.content {
                    if let MessageContent::Text { text } = c {
                        texts.push(text.clone());
                    }
                }
                // 文本部分（如有）在前
                if !texts.is_empty() {
                    out.push(json!({"role": "user", "content": texts.join("\n")}));
                }
                // ToolResult 展开为独立 tool 消息
                for c in &m.content {
                    if let MessageContent::ToolResult {
                        call_id,
                        content,
                        is_error,
                    } = c
                    {
                        let body = if *is_error {
                            format!("[错误] {content}")
                        } else {
                            content.clone()
                        };
                        out.push(json!({
                            "role": "tool",
                            "tool_call_id": call_id,
                            "content": body,
                        }));
                    }
                }
            }
            Role::Assistant => {
                let mut text = String::new();
                let mut tool_calls: Vec<Value> = Vec::new();
                for c in &m.content {
                    match c {
                        MessageContent::Text { text: t } => text.push_str(t),
                        MessageContent::ToolUse { call } => tool_calls.push(json!({
                            "id": call.id,
                            "type": "function",
                            "function": {
                                "name": call.name,
                                "arguments": call.arguments.to_string(),
                            },
                        })),
                        MessageContent::ToolResult { .. } => {}
                    }
                }
                if tool_calls.is_empty() {
                    out.push(json!({"role": "assistant", "content": text}));
                } else {
                    let mut msg = json!({"role": "assistant", "tool_calls": tool_calls});
                    if !text.is_empty() {
                        msg["content"] = json!(text);
                    }
                    out.push(msg);
                }
            }
        }
    }
    out
}

/// 提取内容块里的全部文本（拼接）
fn texts_of(content: &[MessageContent]) -> String {
    content
        .iter()
        .filter_map(|c| match c {
            MessageContent::Text { text } => Some(text.clone()),
            _ => None,
        })
        .collect::<Vec<_>>()
        .join("\n")
}

/// 构造 /chat/completions 请求体
pub fn build_request_body(request: &ModelRequest) -> Value {
    let tools: Vec<Value> = request
        .tools
        .iter()
        .map(|t| {
            json!({
                "type": "function",
                "function": {
                    "name": t.name,
                    "description": t.description,
                    "parameters": t.parameters,
                },
            })
        })
        .collect();
    let mut body = json!({
        "model": "", // 由调用方填
        "messages": to_openai_messages(&request.messages),
        "stream": true,
        // 流式模式下 usage 统计不可靠，不依赖
    });
    if !tools.is_empty() {
        body["tools"] = json!(tools);
    }
    if let Some(t) = request.temperature {
        body["temperature"] = json!(t);
    }
    if let Some(m) = request.max_tokens {
        body["max_tokens"] = json!(m);
    }
    body
}

// ===== SSE 解析 =====

/// 单个 SSE chunk 解析出的增量
#[derive(Debug, Clone, PartialEq)]
pub enum SseDelta {
    Text {
        text: String,
    },
    Reasoning {
        text: String,
    },
    /// 工具调用片段：index 用于聚合，id/name 只在首片段出现
    ToolCallFragment {
        index: u32,
        id: Option<String>,
        name: Option<String>,
        args_fragment: String,
    },
    /// finish_reason 非空（流正常收尾信号）
    Finish,
}

/// 解析一行 SSE data 载荷（已去掉 "data: " 前缀）。`[DONE]` 返回 None。
/// 结构见 OpenAI chat completions 流式协议；同时兼容
/// DeepSeek/Volcano 的 delta.reasoning_content 与 OpenAI o 系的 delta.reasoning。
pub fn parse_sse_data(data: &str) -> Result<Option<SseDelta>> {
    let data = data.trim();
    if data.is_empty() || data == "[DONE]" {
        return Ok(None);
    }
    let v: Value =
        serde_json::from_str(data).map_err(|e| Error::Model(format!("SSE chunk 非 JSON: {e}")))?;
    // 错误载荷：{"error": {...}}（网关/配额错误可能以 200 + error 体返回）
    if let Some(err) = v.get("error") {
        let msg = err
            .get("message")
            .and_then(|m| m.as_str())
            .unwrap_or("模型返回错误");
        return Err(Error::Model(msg.to_string()));
    }
    let Some(choice) = v.get("choices").and_then(|c| c.get(0)) else {
        return Ok(None); // usage-only chunk 等，忽略
    };
    let delta = choice.get("delta");

    // 工具调用片段（可能多个，按数组顺序产出）
    if let Some(tcs) = delta
        .and_then(|d| d.get("tool_calls"))
        .and_then(|t| t.as_array())
    {
        for tc in tcs {
            let index = tc.get("index").and_then(|i| i.as_u64()).unwrap_or(0) as u32;
            let id = tc
                .get("id")
                .and_then(|i| i.as_str())
                .filter(|s| !s.is_empty())
                .map(String::from);
            let function = tc.get("function");
            let name = function
                .and_then(|f| f.get("name"))
                .and_then(|n| n.as_str())
                .filter(|s| !s.is_empty())
                .map(String::from);
            let args_fragment = function
                .and_then(|f| f.get("arguments"))
                .and_then(|a| a.as_str())
                .unwrap_or("")
                .to_string();
            // name 与片段全空：无信息 chunk，跳过
            if id.is_none() && name.is_none() && args_fragment.is_empty() {
                continue;
            }
            return Ok(Some(SseDelta::ToolCallFragment {
                index,
                id,
                name,
                args_fragment,
            }));
        }
        if !tcs.is_empty() {
            return Ok(None); // 全是空片段
        }
    }

    // 思考增量（DeepSeek: reasoning_content；OpenAI o 系: reasoning）
    if let Some(d) = delta {
        for key in ["reasoning_content", "reasoning"] {
            if let Some(r) = d.get(key) {
                let text = match r {
                    Value::String(s) => s.clone(),
                    // OpenAI o 系为对象数组 {"reasoning": [{"text": ...}]} 或字符串
                    Value::Array(items) => items
                        .iter()
                        .filter_map(|i| i.get("text").and_then(|t| t.as_str()))
                        .collect::<Vec<_>>()
                        .join(""),
                    _ => String::new(),
                };
                if !text.is_empty() {
                    return Ok(Some(SseDelta::Reasoning { text }));
                }
            }
        }
    }

    // 文本增量
    if let Some(t) = delta
        .and_then(|d| d.get("content"))
        .and_then(|c| c.as_str())
    {
        if !t.is_empty() {
            return Ok(Some(SseDelta::Text {
                text: t.to_string(),
            }));
        }
    }

    // finish_reason
    if let Some(f) = choice.get("finish_reason") {
        if !f.is_null() {
            return Ok(Some(SseDelta::Finish));
        }
    }
    Ok(None)
}

/// 从字节流缓冲里按行拆 SSE 事件。SSE 规范：事件以空行分隔，
/// chat completions 的每个 chunk 通常是 "data: {...}\n\n"。
/// 返回 (完整行列表, 剩余不完整缓冲)；行为 owned（避免调用方借用冲突）。
pub fn split_sse_lines(buffer: &str) -> (Vec<String>, String) {
    let mut lines = Vec::new();
    let mut rest = String::new();
    for line in buffer.split_inclusive('\n') {
        if line.ends_with('\n') {
            lines.push(line.trim_end_matches(['\n', '\r']).to_string());
        } else {
            rest.push_str(line);
        }
    }
    (lines, rest)
}

/// 从 SSE 行提取 data 载荷（去掉 "data: " 前缀）；非 data 行返回 None
pub fn sse_data_of(line: &str) -> Option<&str> {
    let line = line.trim_start();
    line.strip_prefix("data:").map(|d| d.trim_start())
}

// ===== 流式请求 =====

/// 发起流式请求并返回原始字节流（调用方负责 SSE 解析）。
/// 保留原 rig 实现的初始请求重试语义：3 次尝试、指数退避（1s/2s），
/// 只覆盖连接建立阶段（流开始后出错不重试）。
pub async fn open_stream(
    client: &reqwest::Client,
    base_url: &str,
    api_key: &str,
    body: &Value,
) -> Result<impl futures::Stream<Item = Result<bytes::Bytes>>> {
    let url = format!("{}/chat/completions", base_url.trim_end_matches('/'));
    let mut last_err = String::new();
    for attempt in 0..3 {
        if attempt > 0 {
            tracing::warn!(
                "模型请求失败，{}ms 后重试（第 {attempt}/2 次）: {last_err}",
                1000 * attempt as u64
            );
            tokio::time::sleep(std::time::Duration::from_millis(1000 * attempt as u64)).await;
        }
        let resp = client
            .post(&url)
            .bearer_auth(api_key)
            .json(body)
            .send()
            .await;
        match resp {
            Ok(r) if r.status().is_success() => {
                let stream = r.bytes_stream();
                return Ok(futures::StreamExt::map(stream, |item| {
                    item.map_err(|e| Error::Model(format!("流中断: {e}")))
                }));
            }
            Ok(r) => {
                let status = r.status();
                let body_text = r.text().await.unwrap_or_default();
                last_err = format!("HTTP {status}: {}", truncate(&body_text, 500));
                if !crate::model::should_retry_model_error(&last_err) {
                    return Err(Error::Model(last_err));
                }
            }
            Err(e) => {
                last_err = e.to_string();
                if !crate::model::should_retry_model_error(&last_err) {
                    return Err(Error::Model(last_err));
                }
            }
        }
    }
    Err(Error::Model(last_err))
}

fn truncate(s: &str, max: usize) -> &str {
    match s.char_indices().nth(max) {
        Some((ix, _)) => &s[..ix],
        None => s,
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn user(t: &str) -> ChatMessage {
        ChatMessage {
            role: Role::User,
            content: vec![MessageContent::text(t)],
        }
    }

    fn assistant_tool_use(id: &str, name: &str, args: Value) -> ChatMessage {
        ChatMessage {
            role: Role::Assistant,
            content: vec![MessageContent::ToolUse {
                call: crate::tool::ToolCall {
                    id: id.into(),
                    call_id: None,
                    name: name.into(),
                    arguments: args,
                },
            }],
        }
    }

    fn tool_result(call_id: &str, content: &str) -> ChatMessage {
        ChatMessage {
            role: Role::User,
            content: vec![MessageContent::ToolResult {
                call_id: call_id.into(),
                content: content.into(),
                is_error: false,
            }],
        }
    }

    // ----- 消息转换 -----

    #[test]
    fn empty_messages_fall_back_to_empty_user() {
        let msgs = to_openai_messages(&[]);
        assert_eq!(msgs.len(), 1);
        assert_eq!(msgs[0]["role"], "user");
        assert_eq!(msgs[0]["content"], "");
    }

    #[test]
    fn tool_result_becomes_tool_role_message() {
        let msgs = vec![tool_result("t1", "文件内容")];
        let out = to_openai_messages(&msgs);
        assert_eq!(out.len(), 1);
        assert_eq!(out[0]["role"], "tool");
        assert_eq!(out[0]["tool_call_id"], "t1");
        assert_eq!(out[0]["content"], "文件内容");
    }

    #[test]
    fn assistant_tool_use_carries_tool_calls_array() {
        let msgs = vec![assistant_tool_use(
            "t1",
            "read_file",
            json!({"path": "a.txt"}),
        )];
        let out = to_openai_messages(&msgs);
        assert_eq!(out.len(), 1);
        assert_eq!(out[0]["role"], "assistant");
        let tc = &out[0]["tool_calls"][0];
        assert_eq!(tc["id"], "t1");
        assert_eq!(tc["type"], "function");
        assert_eq!(tc["function"]["name"], "read_file");
        // arguments 必须是字符串（openai 协议）
        assert!(tc["function"]["arguments"].is_string());
        let args: Value =
            serde_json::from_str(tc["function"]["arguments"].as_str().unwrap()).unwrap();
        assert_eq!(args["path"], "a.txt");
    }

    #[test]
    fn mixed_user_text_and_tool_result_order_preserved() {
        let msgs = vec![ChatMessage {
            role: Role::User,
            content: vec![
                MessageContent::text("顺便"),
                MessageContent::ToolResult {
                    call_id: "t2".into(),
                    content: "结果".into(),
                    is_error: true,
                },
            ],
        }];
        let out = to_openai_messages(&msgs);
        assert_eq!(out.len(), 2, "文本 user 消息 + 错误 tool 消息");
        assert_eq!(out[0]["role"], "user");
        assert_eq!(out[0]["content"], "顺便");
        assert_eq!(out[1]["role"], "tool");
        assert_eq!(out[1]["content"], "[错误] 结果");
    }

    #[test]
    fn multi_turn_tool_loop_order_strictly_preserved() {
        // 回归测试：旧的两遍转换会把 user 消息重复、丢失尾部 assistant。
        // 场景：完整一轮工具调用 + 最终回答，共 5 条消息。
        let msgs = vec![
            user("hi"),
            assistant_tool_use("t1", "read_file", json!({"path": "a.txt"})),
            tool_result("t1", "文件内容"),
            user("继续"),
            ChatMessage {
                role: Role::Assistant,
                content: vec![MessageContent::text("done")],
            },
        ];
        let out = to_openai_messages(&msgs);
        let roles: Vec<&str> = out.iter().map(|m| m["role"].as_str().unwrap()).collect();
        assert_eq!(
            roles,
            vec!["user", "assistant", "tool", "user", "assistant"],
            "消息顺序必须严格保持"
        );
        assert_eq!(out[1]["tool_calls"][0]["id"], "t1");
        assert_eq!(out[2]["tool_call_id"], "t1");
        assert_eq!(out[3]["content"], "继续");
        assert_eq!(out[4]["content"], "done");
    }

    #[test]
    fn compaction_message_gets_prefix() {
        let msgs = vec![ChatMessage {
            role: Role::Compaction,
            content: vec![MessageContent::text("摘要内容")],
        }];
        let out = to_openai_messages(&msgs);
        assert_eq!(out[0]["role"], "user");
        let s = out[0]["content"].as_str().unwrap();
        assert!(s.contains("压缩"), "应带压缩前缀: {s}");
        assert!(s.contains("摘要内容"));
    }

    #[test]
    fn request_body_has_tools_and_stream() {
        let req = ModelRequest {
            system: "sys".into(),
            messages: vec![user("hi")],
            tools: vec![crate::tool::ToolDefinition {
                name: "read_file".into(),
                description: "读文件".into(),
                parameters: json!({"type": "object"}),
            }],
            temperature: Some(0.5),
            max_tokens: Some(1024),
        };
        let body = build_request_body(&req);
        assert_eq!(body["stream"], true);
        assert_eq!(body["temperature"], 0.5);
        assert_eq!(body["max_tokens"], 1024);
        assert_eq!(body["tools"][0]["function"]["name"], "read_file");
        assert_eq!(body["messages"][0]["role"], "user");
    }

    // ----- SSE 解析 -----

    #[test]
    fn sse_text_delta_parsed() {
        let line = r#"{"choices":[{"delta":{"content":"你好"},"index":0}]}"#;
        let d = parse_sse_data(line).unwrap().unwrap();
        assert_eq!(
            d,
            SseDelta::Text {
                text: "你好".into()
            }
        );
    }

    #[test]
    fn sse_done_and_empty_return_none() {
        assert!(parse_sse_data("[DONE]").unwrap().is_none());
        assert!(parse_sse_data("").unwrap().is_none());
        assert!(parse_sse_data("   ").unwrap().is_none());
    }

    #[test]
    fn sse_reasoning_content_parsed_deepseek_style() {
        let line = r#"{"choices":[{"delta":{"reasoning_content":"让我想想"}}]}"#;
        let d = parse_sse_data(line).unwrap().unwrap();
        assert_eq!(
            d,
            SseDelta::Reasoning {
                text: "让我想想".into()
            }
        );
    }

    #[test]
    fn sse_reasoning_object_style_parsed() {
        let line = r#"{"choices":[{"delta":{"reasoning":[{"text":"分析"}]}}]}"#;
        let d = parse_sse_data(line).unwrap().unwrap();
        assert_eq!(
            d,
            SseDelta::Reasoning {
                text: "分析".into()
            }
        );
    }

    #[test]
    fn sse_tool_call_start_fragment() {
        let line = r#"{"choices":[{"delta":{"tool_calls":[{"index":0,"id":"call_1","function":{"name":"write_file","arguments":""}}]}}]}"#;
        let d = parse_sse_data(line).unwrap().unwrap();
        match d {
            SseDelta::ToolCallFragment {
                index,
                id,
                name,
                args_fragment,
            } => {
                assert_eq!(index, 0);
                assert_eq!(id.as_deref(), Some("call_1"));
                assert_eq!(name.as_deref(), Some("write_file"));
                assert_eq!(args_fragment, "");
            }
            other => panic!("预期 ToolCallFragment，实际 {other:?}"),
        }
    }

    #[test]
    fn sse_tool_call_args_fragment() {
        let line = r#"{"choices":[{"delta":{"tool_calls":[{"index":0,"function":{"arguments":"{\"path\":"}}]}}]}"#;
        let d = parse_sse_data(line).unwrap().unwrap();
        match d {
            SseDelta::ToolCallFragment { args_fragment, .. } => {
                assert_eq!(args_fragment, "{\"path\":")
            }
            other => panic!("预期 ToolCallFragment，实际 {other:?}"),
        }
    }

    #[test]
    fn sse_finish_reason_parsed() {
        let line = r#"{"choices":[{"delta":{},"finish_reason":"tool_calls"}]}"#;
        let d = parse_sse_data(line).unwrap().unwrap();
        assert_eq!(d, SseDelta::Finish);
    }

    #[test]
    fn sse_error_payload_is_model_error() {
        let line = r#"{"error":{"message":"配额不足"}}"#;
        let err = parse_sse_data(line).unwrap_err();
        assert!(err.to_string().contains("配额不足"));
    }

    #[test]
    fn sse_usage_only_chunk_ignored() {
        let line = r#"{"choices":[],"usage":{"total_tokens":10}}"#;
        assert!(parse_sse_data(line).unwrap().is_none());
    }

    #[test]
    fn empty_content_chunk_skipped() {
        // 部分服务商夹带空 content chunk
        let line = r#"{"choices":[{"delta":{"content":""}}]}"#;
        assert!(parse_sse_data(line).unwrap().is_none());
    }

    // ----- SSE 行拆分 -----

    #[test]
    fn split_sse_lines_handles_partial_tail() {
        let buf = "data: {\"a\":1}\n\ndata: {\"b\":";
        let (lines, rest) = split_sse_lines(buf);
        assert_eq!(lines, vec!["data: {\"a\":1}".to_string(), String::new()]);
        assert_eq!(rest, "data: {\"b\":");
    }

    #[test]
    fn split_sse_lines_handles_crlf() {
        let (lines, rest) = split_sse_lines("data: x\r\ndata: y\r\n");
        assert_eq!(lines, vec!["data: x".to_string(), "data: y".to_string()]);
        assert!(rest.is_empty());
    }

    #[test]
    fn sse_data_of_extracts_payload() {
        assert_eq!(sse_data_of("data: hello"), Some("hello"));
        assert_eq!(sse_data_of("data:  spaced"), Some("spaced"));
        assert_eq!(sse_data_of("event: ping"), None);
        assert_eq!(sse_data_of(": comment"), None);
    }

    #[test]
    fn chunked_fragments_across_lines_assemble_to_full_call() {
        // 模拟真实流：首片段带 id+name，后续片段只带参数碎片
        let chunks = [
            r#"data: {"choices":[{"delta":{"tool_calls":[{"index":0,"id":"c1","function":{"name":"write_file","arguments":""}}]}}]}"#,
            r#"data: {"choices":[{"delta":{"tool_calls":[{"index":0,"function":{"arguments":"{\"path\":\"out.md\","}}]}}]}"#,
            r#"data: {"choices":[{"delta":{"tool_calls":[{"index":0,"function":{"arguments":"\"content\":\"第一段\"}"}}]}}]}"#,
            "data: [DONE]",
        ];
        let mut id = String::new();
        let mut name = String::new();
        let mut args = String::new();
        for c in chunks {
            if let Some(data) = sse_data_of(c) {
                if let Some(SseDelta::ToolCallFragment {
                    id: i,
                    name: n,
                    args_fragment,
                    ..
                }) = parse_sse_data(data).unwrap()
                {
                    if let Some(i) = i {
                        id = i;
                    }
                    if let Some(n) = n {
                        name = n;
                    }
                    args.push_str(&args_fragment);
                }
            }
        }
        assert_eq!(id, "c1");
        assert_eq!(name, "write_file");
        let parsed: Value = serde_json::from_str(&args).unwrap();
        assert_eq!(parsed["path"], "out.md");
        assert_eq!(parsed["content"], "第一段");
    }
}
