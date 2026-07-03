use serde::{Deserialize, Serialize};
use std::sync::atomic::{AtomicBool, Ordering};
use tauri::{AppHandle, Emitter, State};

use crate::providers;

// ===== 类型 =====

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct ChatMessage {
    pub role: String,
    pub content: String,
}

/// 聊天的来源选择。两种模式统一走 OpenAI 兼容接口。
#[derive(Debug, Clone, Serialize, Deserialize)]
#[serde(tag = "type")]
pub enum ChatSource {
    /// 本地 llama.cpp 服务器（从模型管理页面启动）
    #[serde(rename = "model")]
    Model { port: u16, model_name: String },
    /// API 提供者（Provider 配置）
    #[serde(rename = "provider")]
    Provider {
        provider_id: String,
        /// 可选覆盖；不传则用 provider 的 active_model
        model: Option<String>,
    },
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct ChatRequest {
    pub source: ChatSource,
    pub messages: Vec<ChatMessage>,
    pub max_tokens: Option<u32>,
    pub temperature: Option<f32>,
}

/// 全局停止标志（同时只能进行一个会话）
pub struct ChatStopFlag(pub AtomicBool);

// ===== Tauri 命令 =====

/// 发起流式聊天。结果通过事件推送：
/// - `chat-chunk`: payload = 文本片段（String）
/// - `chat-done`: payload = ()，无论正常结束还是被停止
/// - `chat-error`: payload = 错误描述（String）
#[tauri::command]
pub async fn send_chat(
    app: AppHandle,
    stop_flag: State<'_, ChatStopFlag>,
    req: ChatRequest,
) -> Result<(), String> {
    // 重置停止标志
    stop_flag.0.store(false, Ordering::SeqCst);

    // 1). 解析来源 → (url, api_key, model_name)
    let (url, api_key, model_name) = match resolve_source(&app, &req.source) {
        Ok(v) => v,
        Err(e) => {
            let _ = app.emit("chat-error", e.clone());
            return Err(e);
        }
    };

    // 2). 构造 OpenAI 兼容请求体（强制 stream）
    let mut body = serde_json::json!({
        "model": model_name,
        "messages": req.messages,
        "stream": true,
    });
    if let Some(t) = req.temperature {
        body["temperature"] = serde_json::json!(t);
    }
    if let Some(m) = req.max_tokens {
        body["max_tokens"] = serde_json::json!(m);
    }

    // 3). 发送请求
    let client = reqwest::Client::new();
    let mut response = client
        .post(&url)
        .header("Content-Type", "application/json")
        .header("Authorization", format!("Bearer {}", api_key))
        .json(&body)
        .send()
        .await
        .map_err(|e| {
            let msg = format!("请求失败: {}", e);
            let _ = app.emit("chat-error", msg.clone());
            msg
        })?;

    if !response.status().is_success() {
        let status = response.status();
        let text = response.text().await.unwrap_or_default();
        let msg = format!("HTTP {}: {}", status, text);
        let _ = app.emit("chat-error", msg.clone());
        return Err(msg);
    }

    // 4). 流式读取并解析 SSE
    let mut buf: Vec<u8> = Vec::new();

    while let Some(chunk) = response.chunk().await.map_err(|e| {
        let msg = format!("读取流失败: {}", e);
        let _ = app.emit("chat-error", msg.clone());
        msg
    })? {
        // 检查停止标志
        if stop_flag.0.load(Ordering::SeqCst) {
            let _ = app.emit("chat-done", ());
            return Ok(());
        }

        buf.extend_from_slice(&chunk);

        // 处理完整的每一行
        while let Some(pos) = buf.iter().position(|b| *b == b'\n') {
            let line_bytes = buf[..pos].to_vec();
            buf = buf[pos + 1..].to_vec();

            let Ok(line) = String::from_utf8(line_bytes) else {
                continue;
            };
            let line = line.trim();

            // 空行或注释
            if line.is_empty() || line.starts_with(':') {
                continue;
            }

            // 提取 data: 之后的负载
            let Some(json_str) = line.strip_prefix("data:").map(|s| s.trim()) else {
                continue;
            };

            if json_str == "[DONE]" {
                let _ = app.emit("chat-done", ());
                return Ok(());
            }

            // 解析 JSON，提取增量文本
            if let Ok(value) = serde_json::from_str::<serde_json::Value>(json_str) {
                if let Some(content) = value
                    .pointer("/choices/0/delta/content")
                    .and_then(|v| v.as_str())
                {
                    if !content.is_empty() {
                        let _ = app.emit("chat-chunk", content);
                    }
                }
                // 某些后端使用 reasoning_content 字段传递推理内容（可选展示）
                if let Some(reasoning) = value
                    .pointer("/choices/0/delta/reasoning_content")
                    .and_then(|v| v.as_str())
                {
                    if !reasoning.is_empty() {
                        let _ = app.emit("chat-chunk-reasoning", reasoning);
                    }
                }
            }
        }
    }

    // 流自然结束（部分服务不发 [DONE]）
    let _ = app.emit("chat-done", ());
    Ok(())
}

/// 停止当前流式回话
#[tauri::command]
pub fn stop_chat(stop_flag: State<'_, ChatStopFlag>) {
    stop_flag.0.store(true, Ordering::SeqCst);
}

// ===== 内部辅助 =====

fn resolve_source(
    app: &AppHandle,
    source: &ChatSource,
) -> Result<(String, String, String), String> {
    match source {
        ChatSource::Model { port, model_name } => {
            // 本地 llama.cpp 默认 OpenAI 兼容端点
            let url = format!("http://localhost:{}/v1/chat/completions", port);
            Ok((url, String::new(), model_name.clone()))
        }
        ChatSource::Provider { provider_id, model } => {
            let provider = providers::get_provider_inner(app, provider_id)?;
            let url = join_url(&provider.base_url, "chat/completions");
            let model_name = model
                .clone()
                .or(provider.active_model)
                .ok_or_else(|| format!("provider '{}' 没有设置活动模型", provider_id))?;
            Ok((url, provider.api_key, model_name))
        }
    }
}

fn join_url(base: &str, path: &str) -> String {
    if base.ends_with('/') {
        format!("{}{}", base, path)
    } else {
        format!("{}/{}", base, path)
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn test_join_url() {
        assert_eq!(join_url("http://x/v1", "models"), "http://x/v1/models");
        assert_eq!(join_url("http://x/v1/", "models"), "http://x/v1/models");
    }

    #[test]
    fn test_source_serialization() {
        // 验证 tag 形式
        let model_src = ChatSource::Model {
            port: 8080,
            model_name: "llama".into(),
        };
        let s = serde_json::to_string(&model_src).unwrap();
        assert!(s.contains("\"type\":\"model\""));
        assert!(s.contains("\"port\":8080"));
        assert!(s.contains("\"model_name\":\"llama\""));

        let prov_src = ChatSource::Provider {
            provider_id: "p1".into(),
            model: Some("glm-5.2".into()),
        };
        let s = serde_json::to_string(&prov_src).unwrap();
        assert!(s.contains("\"type\":\"provider\""));
        assert!(s.contains("\"provider_id\":\"p1\""));
    }
}
