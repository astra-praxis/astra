use reqwest::Client;
use serde::{Deserialize, Serialize};
use tauri::AppHandle;

use crate::providers;

/// 嵌入来源配置
#[derive(Debug, Clone, Serialize, Deserialize)]
#[serde(tag = "type")]
pub enum EmbeddingSource {
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

/// 嵌入响应
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct EmbedResult {
    pub embeddings: Vec<Vec<f32>>,
    pub model: String,
    pub dimension: usize,
}

/// OpenAI embeddings API 响应结构
#[derive(Debug, Deserialize)]
struct OpenAIEmbeddingResponse {
    data: Vec<OpenAIEmbeddingData>,
    model: String,
}

#[derive(Debug, Deserialize)]
struct OpenAIEmbeddingData {
    embedding: Vec<f32>,
    index: usize,
}

/// 获取文本的嵌入向量
///
/// 参数：
/// - source: 嵌入来源（本地 llama.cpp 或 provider）
/// - texts: 待嵌入的文本列表
///
/// 返回：
/// - embeddings: 嵌入向量列表
/// - model: 使用的模型名称
/// - dimension: 向量维度
#[tauri::command]
pub async fn embed_text(
    app: AppHandle,
    source: EmbeddingSource,
    texts: Vec<String>,
) -> Result<EmbedResult, String> {
    if texts.is_empty() {
        return Err("文本列表不能为空".to_string());
    }

    // 解析来源 → (url, api_key, model_name)
    let (url, api_key, model_name) = resolve_embedding_source(&app, &source)?;

    // 构造 OpenAI 兼容请求体
    let body = serde_json::json!({
        "model": model_name,
        "input": texts,
    });

    // 发送请求
    let client = Client::new();
    let mut request = client.post(&url).header("Content-Type", "application/json");

    if !api_key.is_empty() {
        request = request.header("Authorization", format!("Bearer {}", api_key));
    }

    let response = request
        .json(&body)
        .send()
        .await
        .map_err(|e| format!("嵌入请求失败: {}", e))?;

    if !response.status().is_success() {
        let status = response.status();
        let text = response.text().await.unwrap_or_default();
        return Err(format!("嵌入请求失败 HTTP {}: {}", status, text));
    }

    // 解析响应
    let result: OpenAIEmbeddingResponse = response
        .json()
        .await
        .map_err(|e| format!("解析嵌入响应失败: {}", e))?;

    // 提取嵌入向量（按 index 排序）
    let mut data = result.data;
    data.sort_by_key(|d| d.index);

    let embeddings: Vec<Vec<f32>> = data.into_iter().map(|d| d.embedding).collect();

    if embeddings.is_empty() {
        return Err("嵌入结果为空".to_string());
    }

    let dimension = embeddings[0].len();

    Ok(EmbedResult {
        embeddings,
        model: result.model,
        dimension,
    })
}

/// 测试嵌入来源是否可用
#[tauri::command]
pub async fn test_embedding_source(
    app: AppHandle,
    source: EmbeddingSource,
) -> Result<bool, String> {
    let result = embed_text(app, source, vec!["test".to_string()]).await?;
    Ok(!result.embeddings.is_empty())
}

/// 解析嵌入来源，返回 (url, api_key, model_name)
fn resolve_embedding_source(
    app: &AppHandle,
    source: &EmbeddingSource,
) -> Result<(String, String, String), String> {
    match source {
        EmbeddingSource::Model { port, model_name } => {
            // 本地 llama.cpp 的 OpenAI 兼容嵌入端点
            let url = format!("http://localhost:{}/v1/embeddings", port);
            Ok((url, String::new(), model_name.clone()))
        }
        EmbeddingSource::Provider { provider_id, model } => {
            let provider = providers::get_provider_inner(app, provider_id)?;
            let url = join_url(&provider.base_url, "embeddings");
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
        assert_eq!(
            join_url("http://x/v1", "embeddings"),
            "http://x/v1/embeddings"
        );
        assert_eq!(
            join_url("http://x/v1/", "embeddings"),
            "http://x/v1/embeddings"
        );
    }
}
