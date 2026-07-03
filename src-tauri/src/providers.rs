use serde::{Deserialize, Serialize};
use std::fs;
use std::path::PathBuf;
use tauri::{AppHandle, Manager};

// ===== 类型定义 =====

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct ProviderConfig {
    pub id: String,
    pub name: String,
    pub base_url: String,
    pub api_key: String,
    pub active_model: Option<String>,
    pub created_at: u64,
    pub updated_at: u64,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct ModelInfo {
    pub id: String,
    pub object: Option<String>,
    pub created: Option<i64>,
    pub owned_by: Option<String>,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct ModelsResponse {
    pub object: String,
    pub data: Vec<ModelInfo>,
}

// ===== 辅助函数 =====

fn get_providers_path(app: &AppHandle) -> Result<PathBuf, String> {
    let app_data_dir = app
        .path()
        .app_data_dir()
        .map_err(|e| format!("获取应用数据目录失败: {}", e))?;
    fs::create_dir_all(&app_data_dir).map_err(|e| format!("创建配置目录失败: {}", e))?;
    Ok(app_data_dir.join("providers.json"))
}

fn ensure_providers_file(app: &AppHandle) -> Result<PathBuf, String> {
    let path = get_providers_path(app)?;
    if !path.exists() {
        fs::write(&path, "[]").map_err(|e| format!("创建配置文件失败: {}", e))?;
    }
    Ok(path)
}

fn unix_timestamp() -> u64 {
    std::time::SystemTime::now()
        .duration_since(std::time::UNIX_EPOCH)
        .unwrap_or_default()
        .as_secs()
}

fn generate_id() -> String {
    format!("provider_{}", unix_timestamp())
}

// ===== Tauri 命令 =====

#[tauri::command]
pub fn list_providers(app: AppHandle) -> Result<Vec<ProviderConfig>, String> {
    let path = ensure_providers_file(&app)?;

    match fs::read_to_string(&path) {
        Ok(content) => {
            let providers: Vec<ProviderConfig> = serde_json::from_str(&content).unwrap_or_default();
            Ok(providers)
        }
        Err(e) => Err(format!("读取配置失败: {}", e)),
    }
}

#[tauri::command]
pub fn save_provider(
    app: AppHandle,
    id: Option<String>,
    name: String,
    base_url: String,
    api_key: String,
    active_model: Option<String>,
) -> Result<ProviderConfig, String> {
    let path = ensure_providers_file(&app)?;

    // 读取现有配置
    let content = fs::read_to_string(&path).unwrap_or_else(|_| "[]".to_string());
    let mut providers: Vec<ProviderConfig> = serde_json::from_str(&content).unwrap_or_default();

    let now = unix_timestamp();

    if let Some(existing_id) = id {
        // 更新现有配置
        if let Some(provider) = providers.iter_mut().find(|p| p.id == existing_id) {
            provider.name = name;
            provider.base_url = base_url;
            provider.api_key = api_key;
            provider.active_model = active_model;
            provider.updated_at = now;

            let result = provider.clone();
            let _ = fs::write(
                &path,
                serde_json::to_string_pretty(&providers).unwrap_or_default(),
            );
            return Ok(result);
        }
        return Err("找不到要更新的配置".to_string());
    } else {
        // 创建新配置
        let provider = ProviderConfig {
            id: generate_id(),
            name,
            base_url,
            api_key,
            active_model,
            created_at: now,
            updated_at: now,
        };

        providers.push(provider.clone());
        let _ = fs::write(
            &path,
            serde_json::to_string_pretty(&providers).unwrap_or_default(),
        );
        Ok(provider)
    }
}

#[tauri::command]
pub fn delete_provider(app: AppHandle, id: String) -> Result<(), String> {
    let path = ensure_providers_file(&app)?;

    let content = fs::read_to_string(&path).unwrap_or_else(|_| "[]".to_string());
    let mut providers: Vec<ProviderConfig> = serde_json::from_str(&content).unwrap_or_default();

    let len_before = providers.len();
    providers.retain(|p| p.id != id);

    if providers.len() == len_before {
        return Err("找不到要删除的配置".to_string());
    }

    let _ = fs::write(
        &path,
        serde_json::to_string_pretty(&providers).unwrap_or_default(),
    );
    Ok(())
}

#[tauri::command]
pub async fn fetch_provider_models(
    base_url: String,
    api_key: String,
) -> Result<Vec<ModelInfo>, String> {
    // 第一步：优先请求标准的 /models 接口
    let step1 = try_get_models_list(&base_url, &api_key).await;
    if let Ok(models) = &step1 {
        if !models.is_empty() {
            return Ok(models.clone());
        }
    }

    // 第二步：models 为空 / 请求失败时，回降为探测 chat/completions
    // 一些 LLM 网关（如本例的 domain=gateway）不在 /models 暴露模型，
    // 但在请求非法模型名时，错误信息里会给出可用模型列表。
    let step2 = try_probe_models_via_chat(&base_url, &api_key).await;
    if let Ok(models) = &step2 {
        if !models.is_empty() {
            return Ok(models.clone());
        }
    }

    // 任一步成功但都没数据 → 返回空（让前端提示手动添加）
    if step1.is_ok() || step2.is_ok() {
        return Ok(vec![]);
    }

    // 两步都真正失败 → 返回错误信息（优先第一步的错误）
    Err(step1.err().unwrap_or_else(|| "获取模型失败".to_string()))
}

// ===== 内部辅助 =====

async fn try_get_models_list(base_url: &str, api_key: &str) -> Result<Vec<ModelInfo>, String> {
    let url = if base_url.ends_with('/') {
        format!("{}models", base_url)
    } else {
        format!("{}/models", base_url)
    };

    let client = reqwest::Client::new();
    let response = client
        .get(&url)
        .header("Authorization", format!("Bearer {}", api_key))
        .send()
        .await
        .map_err(|e| format!("/models 请求失败: {}", e))?;

    if !response.status().is_success() {
        let status = response.status();
        let body = response.text().await.unwrap_or_default();
        return Err(format!("/models 返回错误 ({}): {}", status, body));
    }

    let models_response: ModelsResponse = response
        .json()
        .await
        .map_err(|e| format!("/models 解析响应失败: {}", e))?;

    Ok(models_response.data)
}

async fn try_probe_models_via_chat(
    base_url: &str,
    api_key: &str,
) -> Result<Vec<ModelInfo>, String> {
    let url = if base_url.ends_with('/') {
        format!("{}chat/completions", base_url)
    } else {
        format!("{}/chat/completions", base_url)
    };

    let client = reqwest::Client::new();
    // 故意使用非法模型名，期望网关返回包含可用模型列表的错误响应
    let payload = serde_json::json!({
        "model": "__probe_invalid_model__",
        "messages": [{"role": "user", "content": "hi"}],
        "max_tokens": 1,
    });

    let response = client
        .post(&url)
        .header("Authorization", format!("Bearer {}", api_key))
        .header("Content-Type", "application/json")
        .json(&payload)
        .send()
        .await
        .map_err(|e| format!("chat/completions 探测失败: {}", e))?;

    // 不关心状态码，直接读取正文试图从中解析模型列表
    let text = response
        .text()
        .await
        .map_err(|e| format!("读取响应失败: {}", e))?;

    Ok(parse_models_from_text(&text))
}

/// 从一段文本（通常是错误响应正文）里提取形如 ['a','b',"c"] 的模型名列表。
/// 优先解析为 JSON 并扫描常见的错误字段；找不到再回退到全文搜索。
fn parse_models_from_text(text: &str) -> Vec<ModelInfo> {
    if let Ok(value) = serde_json::from_str::<serde_json::Value>(text) {
        // 直接命中标准 OpenAI 的 /models 响应也算解析成功
        if let Some(data) = value.get("data").and_then(|v| v.as_array()) {
            let models: Vec<ModelInfo> = data
                .iter()
                .filter_map(|m| {
                    let id = m.get("id").and_then(|v| v.as_str())?.to_string();
                    Some(ModelInfo {
                        object: m.get("object").and_then(|v| v.as_str()).map(String::from),
                        created: m.get("created").and_then(|v| v.as_i64()),
                        owned_by: m.get("owned_by").and_then(|v| v.as_str()).map(String::from),
                        id,
                    })
                })
                .collect();
            if !models.is_empty() {
                return models;
            }
        }

        // 扫描常见的错误信息字段
        for field in ["msg", "message", "error", "detail", "reason"] {
            if let Some(s) = value.get(field).and_then(|v| v.as_str()) {
                let models = parse_bracket_list(s, "gateway");
                if !models.is_empty() {
                    return models;
                }
            }
        }
    }

    // 最后回退到全文搜索
    parse_bracket_list(text, "gateway")
}

/// 解析字符串里第一对方括号内的逗号分隔列表。
/// 元素两侧可能包围着单引号或双引号，全部去除。
fn parse_bracket_list(msg: &str, owned_by: &str) -> Vec<ModelInfo> {
    let start = match msg.find('[') {
        Some(i) => i,
        None => return vec![],
    };
    let end = match msg[start..].find(']') {
        Some(i) => start + i,
        None => return vec![],
    };
    let list_str = &msg[start + 1..end];

    list_str
        .split(',')
        .filter_map(|s| {
            let trimmed = s
                .trim()
                .trim_matches('\'')
                .trim_matches('"')
                .trim()
                .to_string();
            if trimmed.is_empty() {
                None
            } else {
                Some(ModelInfo {
                    id: trimmed,
                    object: Some("list".to_string()),
                    created: None,
                    owned_by: Some(owned_by.to_string()),
                })
            }
        })
        .collect()
}

#[tauri::command]
pub fn set_active_model(
    app: AppHandle,
    provider_id: String,
    model_id: String,
) -> Result<(), String> {
    let path = ensure_providers_file(&app)?;

    let content = fs::read_to_string(&path).unwrap_or_else(|_| "[]".to_string());
    let mut providers: Vec<ProviderConfig> = serde_json::from_str(&content).unwrap_or_default();

    if let Some(provider) = providers.iter_mut().find(|p| p.id == provider_id) {
        provider.active_model = Some(model_id);
        provider.updated_at = unix_timestamp();
        let _ = fs::write(
            &path,
            serde_json::to_string_pretty(&providers).unwrap_or_default(),
        );
        Ok(())
    } else {
        Err("找不到配置".to_string())
    }
}

#[tauri::command]
pub fn get_provider(app: AppHandle, id: String) -> Result<ProviderConfig, String> {
    let path = ensure_providers_file(&app)?;

    let content = fs::read_to_string(&path).unwrap_or_else(|_| "[]".to_string());
    let providers: Vec<ProviderConfig> = serde_json::from_str(&content).unwrap_or_default();

    providers
        .into_iter()
        .find(|p| p.id == id)
        .ok_or_else(|| "找不到配置".to_string())
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn test_parse_gateway_error_message() {
        // 模拟本例网关返回的真实错误结构
        let text = r#"{"req_id":"abc","code":403,"domain":"gateway","error_code":"model_not_available","msg":"模型 gpt-3.5-turbo 无权限，可用模型：['MiniMax-M2.7-highspeed', 'minimax-auto', 'glm-5.2','deepseek-v4-pro']}"}刘海"#;
        let models = parse_models_from_text(text);
        assert_eq!(models.len(), 4);
        assert_eq!(models[0].id, "MiniMax-M2.7-highspeed");
        assert_eq!(models[1].id, "minimax-auto");
        assert_eq!(models[2].id, "glm-5.2");
        assert_eq!(models[3].id, "deepseek-v4-pro");
        assert_eq!(models[0].owned_by.as_deref(), Some("gateway"));
    }

    #[test]
    fn test_parse_plain_brackets() {
        let text = "可用：['a', 'b', 'c']结尾";
        let models = parse_models_from_text(text);
        assert_eq!(models.len(), 3);
        assert_eq!(models[0].id, "a");
        assert_eq!(models[1].id, "b");
        assert_eq!(models[2].id, "c");
    }

    #[test]
    fn test_parse_double_quoted_list() {
        let text = r#"["model-a", "model-b"]"#;
        let models = parse_models_from_text(text);
        assert_eq!(models.len(), 2);
        assert_eq!(models[0].id, "model-a");
        assert_eq!(models[1].id, "model-b");
    }

    #[test]
    fn test_parse_standard_models_response() {
        // 同时能识别标准 OpenAI /models 响应，用于后续进一步扩展
        let text =
            r#"{"object":"list","data":[{"id":"gpt-4o","object":"model","owned_by":"openai"}]}"#;
        let models = parse_models_from_text(text);
        assert_eq!(models.len(), 1);
        assert_eq!(models[0].id, "gpt-4o");
    }

    #[test]
    fn test_parse_no_models_returns_empty() {
        let text = "no models here";
        let models = parse_models_from_text(text);
        assert!(models.is_empty());
    }

    #[test]
    fn test_parse_real_gateway_response_e2e() {
        // 2026-07-03 从 http://gateway.internal:8002/v1/chat/completions 用
        // 非法模型名 __probe_invalid_model__ 实际拿到的网关响应
        let text = "{\"req_id\":\"f4abd85d-0e07-49f0-8048-de45e6f3981f\",\"code\":403,\"domain\":\"gateway\",\"error_code\":\"model_not_available\",\"msg\":\"模型 __probe_invalid_model__ 无权限，可用模型：['MiniMax-M2.7-highspeed', 'minimax-auto', 'MiniMax-M2.5', 'MiniMax-M2.5-highspeed', 'MiniMax-M2.7', 'MiniMax-M3', 'glm-4.7', 'glm-auto', 'glm-5.1', 'kimi-k2.5', 'kimi-auto', 'glm-5', 'glm-5.2', 'ark-code-latest', 'minimax-m2.5', 'qwen3-coder-plus', 'qwen-coder-auto', 'qwen3.5-plus', 'qwen-plus-auto', 'qwen3.7-plus', 'qwen3-coder-next', 'qwen3.6-plus', 'MiniMax-auto', 'deepseek-v4-pro', 'deepseek-v4-flash']\"}";
        let models = parse_models_from_text(text);
        assert_eq!(models.len(), 25);
        assert_eq!(models[0].id, "MiniMax-M2.7-highspeed");
        assert_eq!(models[12].id, "glm-5.2");
        assert_eq!(models[23].id, "deepseek-v4-pro");
        assert_eq!(models[24].id, "deepseek-v4-flash");
    }
}
