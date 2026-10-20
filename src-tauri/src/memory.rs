//! 宿主侧记忆集成（astra-memory CME → 星野桌面）。
//!
//! 分层（astra-memory docs/12 + docs/02 铁律）：
//! - `MemoryHost` 拥有 cme `MemoryEngine`：加密 SQLite 存储
//!   （app_data_dir/memory-cme.sqlite，主密钥在 memory.key，缺失时用两个
//!   UUIDv4 生成 32 字节后落盘）。旧版 memory.sqlite（legacy 引擎 schema）
//!   在 init 时删除——CME 与 legacy 表结构不兼容，历史数据已确认废弃。
//! - `CmeMemoryProvider` 适配 agent-core 冻结的 `MemoryProvider`，经
//!   `ThreadConfig.memory` 注入每个 Run —— core 侧自动做 recall 注入
//!   （REMEMBERED CONTEXT，见 astra-core run.rs）。
//! - 对话采集是宿主能力：Run 结束后把用户输入 ingest 进 CME 待处理队列
//!   （agent.rs），达到阈值后由 `process_pending` 压缩并提炼长期记忆。
//!   提炼是确定性规则（RuleExtractor），无模型依赖；judge / embedding /
//!   proposer 等 legacy 管线已随 legacy 引擎退役。

use std::path::{Path, PathBuf};
use std::sync::Arc;
use std::time::Duration;

use astra_memory::cme::compactor::{AgentSummary, AgentSummaryClient, ConversationMessage};
use astra_memory::cme::engine::{MemoryEngine, RememberRequest};
use astra_memory::cme::model::{MemoryKind, SessionSummary};
use astra_memory::cme::store::MemoryError;
use astra_memory::cme::store_sqlite::CmeSqliteStore;
use astra_memory_provider::CmeMemoryProvider;
use tauri::AppHandle;

const KEY_FILE: &str = "memory.key";
/// CME 独立的库文件：与 legacy 的 memory.sqlite schema 不兼容。
const DB_FILE: &str = "memory-cme.sqlite";
/// legacy 库文件（旧引擎数据，确认废弃，init 时删除）。
const LEGACY_DB_FILE: &str = "memory.sqlite";
/// 管理接口单次返回的条数上限
const DEFAULT_SEARCH_LIMIT: usize = 50;

pub struct MemoryHost {
    engine: Arc<MemoryEngine<CmeSqliteStore>>,
    provider: Arc<CmeMemoryProvider<CmeSqliteStore>>,
    db_path: PathBuf,
}

/// 记忆引擎初始化选项（lib.rs setup 按 settings.json 组装）。
#[derive(Default)]
pub struct MemoryInitOptions {
    /// 摘要路径开关的实体：Some = 宿主 agent LLM 做会话总结（客户端失败
    /// 或输出不可用时自动降级回规则合并）；None = 纯规则（默认）。
    /// client 的构造需要 AppHandle，由调用方完成，本模块不依赖 tauri 状态。
    pub agent_summary_client: Option<Box<dyn AgentSummaryClient>>,
}

impl MemoryHost {
    /// 打开（或首次创建）加密存储并装配引擎。任何失败都由调用方降级为
    /// memory-off（agent 照常运行，只是没有 recall 注入）。
    pub fn init(dir: &Path, options: MemoryInitOptions) -> Result<Self, String> {
        std::fs::create_dir_all(dir).map_err(|e| format!("create memory dir: {e}"))?;
        // 旧引擎的库文件直接删除（用户确认历史数据废弃）；失败不阻断。
        match std::fs::remove_file(dir.join(LEGACY_DB_FILE)) {
            Ok(()) => eprintln!("[memory] removed legacy memory.sqlite (deprecated schema)"),
            Err(e) if e.kind() == std::io::ErrorKind::NotFound => {}
            Err(e) => eprintln!("[memory] could not remove legacy memory.sqlite: {e}"),
        }
        let key = load_or_create_key(&dir.join(KEY_FILE))?;
        let db_path = dir.join(DB_FILE);
        let store =
            CmeSqliteStore::open(&db_path, &key).map_err(|e| format!("open memory store: {e}"))?;
        let engine = MemoryEngine::new(store);
        let engine = match options.agent_summary_client {
            Some(client) => {
                eprintln!("[memory] agent summary enabled");
                engine.with_agent_summary(client)
            }
            None => engine,
        };
        let engine = Arc::new(engine);
        let provider = Arc::new(CmeMemoryProvider::new(Arc::clone(&engine)));
        Ok(Self {
            engine,
            provider,
            db_path,
        })
    }

    /// 对话采集入口（宿主 ingest 用；agent 模型永不接触）。
    pub(crate) fn engine(&self) -> &Arc<MemoryEngine<CmeSqliteStore>> {
        &self.engine
    }

    /// agent-core `MemoryProvider` 适配器（注入 ThreadConfig.memory 用）。
    pub fn provider(&self) -> Arc<CmeMemoryProvider<CmeSqliteStore>> {
        Arc::clone(&self.provider)
    }

    pub fn db_path(&self) -> &Path {
        &self.db_path
    }
}

/// 32 字节主密钥：存在则读，不存在则生成并落盘（0600）。
/// 主密钥与库文件同目录——本地方便性取舍，密钥层级升级（Keychain /
/// 硬件绑定）留给 docs/07 的 KeyProvider 演进，接口已预留。
fn load_or_create_key(path: &Path) -> Result<Vec<u8>, String> {
    if let Ok(bytes) = std::fs::read(path) {
        if bytes.len() >= 32 {
            return Ok(bytes);
        }
        return Err(format!(
            "{} exists but is shorter than 32 bytes; refusing to silently rotate the key (existing data would be unreadable)",
            path.display()
        ));
    }
    // 两个 UUIDv4 = 32 字节随机（uuid crate 无 getrandom 特性也能用）
    let mut key = [0u8; 32];
    key[..16].copy_from_slice(&uuid::Uuid::new_v4().as_bytes()[..]);
    key[16..].copy_from_slice(&uuid::Uuid::new_v4().as_bytes()[..]);
    std::fs::write(path, key).map_err(|e| format!("write {KEY_FILE}: {e}"))?;
    #[cfg(unix)]
    {
        use std::os::unix::fs::PermissionsExt;
        let _ = std::fs::set_permissions(path, std::fs::Permissions::from_mode(0o600));
    }
    Ok(key.to_vec())
}

// ===== agent 摘要客户端（summary_mode = agent 时由引擎回调） =====

/// 单条消息在摘要 prompt 中的截断上限；与采集截断（agent.rs 2000 chars）
/// 同量级，防止长工具输出撑爆 prompt。
const SUMMARY_MSG_TRUNCATE: usize = 2000;
/// 单批送入模型的最大消息数。
const SUMMARY_MAX_MESSAGES: usize = 50;
/// 模型返回摘要的最大长度（超出截断，保护存储）。
const SUMMARY_OUTPUT_TRUNCATE: usize = 8000;

/// astra-memory `AgentSummaryClient` 的宿主实现：用 providers.json 里
/// 配置的模型直接调 OpenAI 兼容 /chat/completions（无工具循环）。任何
/// 失败（无可用 provider / HTTP 错误 / 解析失败 / 空摘要）都返回 Err，
/// 由引擎侧降级回规则合并 —— 摘要模型故障永不阻断记忆管线。
/// failure or unusable output (empty summary text). Fallbacks are counted
/// for observability.
pub struct HostAgentSummaryClient {
    app: AppHandle,
    provider_id: Option<String>,
    http: reqwest::Client,
}

impl HostAgentSummaryClient {
    pub fn new(app: AppHandle, provider_id: Option<String>) -> Self {
        Self {
            app,
            provider_id,
            http: reqwest::Client::builder()
                .timeout(Duration::from_secs(60))
                .build()
                .unwrap_or_default(),
        }
    }

    /// 选模型来源：显式配置的 provider 优先（不可用则报错，不静默换），
    /// 否则取第一个配置齐全（base_url + api_key + active_model）的。
    fn pick_provider(&self) -> Result<(String, String, String), String> {
        let providers = crate::providers::list_providers(self.app.clone())?;
        if let Some(pid) = &self.provider_id {
            let p = providers
                .iter()
                .find(|p| &p.id == pid)
                .ok_or_else(|| format!("memory summary provider '{pid}' 不存在"))?;
            let model = p
                .active_model
                .clone()
                .filter(|m| !m.trim().is_empty())
                .ok_or_else(|| format!("provider '{pid}' 没有设置活动模型"))?;
            return Ok((p.base_url.clone(), p.api_key.clone(), model));
        }
        providers
            .iter()
            .filter_map(|p| {
                let model = p.active_model.as_deref()?.trim();
                if model.is_empty() || p.base_url.trim().is_empty() || p.api_key.is_empty() {
                    return None;
                }
                Some((p.base_url.clone(), p.api_key.clone(), model.to_string()))
            })
            .next()
            .ok_or_else(|| {
                "没有可用的模型来源（需要在 Providers 页配置 base_url、api_key 和活动模型）"
                    .to_string()
            })
    }
}

#[async_trait::async_trait]
impl AgentSummaryClient for HostAgentSummaryClient {
    async fn summarize(
        &self,
        previous: Option<&SessionSummary>,
        new_messages: &[ConversationMessage],
    ) -> Result<AgentSummary, MemoryError> {
        let (base_url, api_key, model) = self.pick_provider().map_err(MemoryError::Storage)?;

        // 组装 prompt：已有摘要 + 新增消息，要求全量 JSON 状态。
        let mut conversation = String::new();
        for msg in new_messages.iter().take(SUMMARY_MAX_MESSAGES) {
            let role = match msg.role {
                astra_memory::cme::compactor::MessageRole::User => "用户",
                astra_memory::cme::compactor::MessageRole::Assistant => "助手",
                astra_memory::cme::compactor::MessageRole::Tool => "工具",
            };
            let content: String = msg.content.chars().take(SUMMARY_MSG_TRUNCATE).collect();
            conversation.push_str(&format!("[{role}] {content}\n"));
        }
        let previous_json = match previous {
            Some(p) => serde_json::to_string(p)
                .map_err(|e| MemoryError::Storage(format!("serialize previous summary: {e}")))?,
            None => "null".to_string(),
        };
        let prompt = format!(
            "已有摘要（JSON，可能为 null）：\n{previous_json}\n\n新增消息：\n{conversation}\n\n\n把已有摘要与新增消息合并为一份新的当前状态，输出严格 JSON（不要 markdown 代码块、不要任何额外文字）：\n{{\"summary\": \"一段连贯的摘要\", \"open_items\": [\"尚未完成的事项\"], \"decisions\": [\"已确认的决策\"]}}\n要求：\n1. open_items / decisions 是全量当前状态而非增量：已解决的从 open_items 移除，被推翻的决策不再出现在 decisions，新确认的加入\n2. summary 简洁连贯，保留关键上下文（项目名、路径、原因）\n3. 不要编造消息里没有的内容"
        );

        let url = format!("{}/chat/completions", base_url.trim_end_matches('/'));
        let resp = self
            .http
            .post(&url)
            .bearer_auth(&api_key)
            .json(&serde_json::json!({
                "model": model,
                "messages": [
                    {"role": "system", "content": "你是 Astra 的会话记忆摘要器，只输出严格 JSON。"},
                    {"role": "user", "content": prompt}
                ],
                "temperature": 0
            }))
            .send()
            .await
            .map_err(|e| MemoryError::Storage(format!("summary request failed: {e}")))?;
        if !resp.status().is_success() {
            let status = resp.status();
            let body = resp.text().await.unwrap_or_default();
            return Err(MemoryError::Storage(format!(
                "summary model HTTP {status}: {}",
                body.chars().take(300).collect::<String>()
            )));
        }
        let payload: serde_json::Value = resp
            .json()
            .await
            .map_err(|e| MemoryError::Storage(format!("summary response not json: {e}")))?;
        let text = payload["choices"][0]["message"]["content"]
            .as_str()
            .ok_or_else(|| {
                MemoryError::Storage("summary response missing choices[0].message.content".into())
            })?;

        // 容忍模型包一层 ```json 代码块。
        let stripped = text.trim();
        let stripped = stripped
            .strip_prefix("```json")
            .or_else(|| stripped.strip_prefix("```"))
            .unwrap_or(stripped)
            .trim()
            .strip_suffix("```")
            .unwrap_or(stripped)
            .trim();
        let parsed: AgentSummaryOutput = serde_json::from_str(stripped)
            .map_err(|e| MemoryError::Storage(format!("summary output not valid JSON: {e}")))?;

        if parsed.summary.trim().is_empty() {
            return Err(MemoryError::Storage(
                "summary model returned an empty summary".into(),
            ));
        }
        Ok(AgentSummary {
            summary: parsed
                .summary
                .chars()
                .take(SUMMARY_OUTPUT_TRUNCATE)
                .collect(),
            open_items: parsed.open_items,
            decisions: parsed.decisions,
        })
    }
}

/// 模型输出的严格形状（未知字段忽略，缺失字段默认空）。
#[derive(serde::Deserialize)]
struct AgentSummaryOutput {
    #[serde(default)]
    summary: String,
    #[serde(default)]
    open_items: Vec<String>,
    #[serde(default)]
    decisions: Vec<String>,
}

// ===== DTO（不把 astra-memory 内部结构直接暴露给前端） =====

#[derive(serde::Serialize)]
pub struct MemoryItem {
    pub id: String,
    pub content: String,
    pub kind: String,
    pub confidence: f32,
    pub created_at: u64,
}

#[derive(serde::Serialize)]
pub struct MemoryStatus {
    pub enabled: bool,
    /// 提炼管线描述（CME v1 为确定性规则，无决策模型）
    pub model: String,
    pub db_path: String,
}

fn item_from_memory(memory: astra_memory::cme::model::Memory) -> MemoryItem {
    MemoryItem {
        id: memory.id,
        content: memory.content,
        kind: serde_json::to_value(memory.kind)
            .ok()
            .and_then(|v| v.as_str().map(str::to_owned))
            .unwrap_or_else(|| "other".into()),
        confidence: memory.confidence,
        created_at: (memory.created_at.max(0) as u64) / 1000,
    }
}

// ===== Tauri 命令（memory 关闭时统一报错，不静默） =====

fn host_or_err<'a>(
    state: &'a tauri::State<'_, crate::agent::AgentState>,
) -> Result<&'a MemoryHost, String> {
    state
        .memory
        .get()
        .ok_or_else(|| "memory is unavailable (init failed or not started)".to_string())
}

#[tauri::command]
pub fn memory_status(state: tauri::State<'_, crate::agent::AgentState>) -> MemoryStatus {
    match state.memory.get() {
        Some(host) => MemoryStatus {
            enabled: true,
            model: "cme-rule".into(),
            db_path: host.db_path().display().to_string(),
        },
        None => MemoryStatus {
            enabled: false,
            model: "none".into(),
            db_path: String::new(),
        },
    }
}

/// 语义检索（全局作用域，最相关在前）；空查询明确拒绝。
#[tauri::command]
pub async fn memory_search(
    state: tauri::State<'_, crate::agent::AgentState>,
    query: String,
    limit: Option<usize>,
) -> Result<Vec<MemoryItem>, String> {
    let host = host_or_err(&state)?;
    let query = query.trim().to_owned();
    if query.is_empty() {
        return Err("query must not be empty".to_string());
    }
    let hits = host
        .engine
        .recall_in_scope(None, &query, limit.unwrap_or(DEFAULT_SEARCH_LIMIT).min(500))
        .await
        .map_err(|e| e.to_string())?;
    Ok(hits
        .into_iter()
        .map(|hit| item_from_memory(hit.memory))
        .collect())
}

/// 显式写入（UI「添加记忆」按钮）；置信度钉 1.0，重复内容自动去重强化。
#[tauri::command]
pub async fn memory_remember(
    state: tauri::State<'_, crate::agent::AgentState>,
    content: String,
) -> Result<MemoryItem, String> {
    let host = host_or_err(&state)?;
    let req = RememberRequest {
        content: content.trim().to_owned(),
        kind: MemoryKind::Preference,
        project_id: None,
        thread_id: None,
    };
    if req.content.is_empty() {
        return Err("content must not be empty".to_string());
    }
    let memory = host.engine.remember(req).await.map_err(|e| e.to_string())?;
    Ok(item_from_memory(memory))
}

/// 遗忘单条（docs/13 D2 双模式）：默认逻辑遗忘 —— `valid_until = now`，
/// 立即退出召回但保留行供审计；`purge = true` 才物理删除（级联关系与
/// 版本链引用）。未知 id 两种模式都返回错误，不静默。
#[tauri::command]
pub async fn memory_forget(
    state: tauri::State<'_, crate::agent::AgentState>,
    id: String,
    purge: Option<bool>,
) -> Result<(), String> {
    let host = host_or_err(&state)?;
    let forgotten = if purge.unwrap_or(false) {
        host.engine.forget(&id).await
    } else {
        host.engine.logically_forget(&id).await
    }
    .map_err(|e| e.to_string())?;
    if !forgotten {
        return Err("memory not found".to_string());
    }
    Ok(())
}

#[cfg(test)]
mod tests {
    use super::*;

    fn key_roundtrip_and_short_key_rejected(dir: &Path) {
        let key_path = dir.join(KEY_FILE);
        let first = load_or_create_key(&key_path).unwrap();
        assert_eq!(first.len(), 32);
        let second = load_or_create_key(&key_path).unwrap();
        assert_eq!(first, second, "key must be stable across restarts");

        std::fs::write(&key_path, b"short").unwrap();
        assert!(load_or_create_key(&key_path).is_err());
    }

    #[test]
    fn key_roundtrip_and_short_key_rejected_tmp() {
        let dir = std::env::temp_dir().join(format!("astra-mem-key-{}", std::process::id()));
        let _ = std::fs::remove_dir_all(&dir);
        std::fs::create_dir_all(&dir).unwrap();
        key_roundtrip_and_short_key_rejected(&dir);
        let _ = std::fs::remove_dir_all(&dir);
    }

    #[tokio::test]
    async fn init_creates_store_and_cme_roundtrip() {
        let dir = std::env::temp_dir().join(format!("astra-mem-cme-{}", std::process::id()));
        let _ = std::fs::remove_dir_all(&dir);
        std::fs::create_dir_all(&dir).unwrap();
        // 预置一个 legacy 库文件，验证 init 会清掉它。
        std::fs::write(dir.join(LEGACY_DB_FILE), b"old data").unwrap();

        let host = MemoryHost::init(&dir, MemoryInitOptions::default()).unwrap();
        assert!(
            !dir.join(LEGACY_DB_FILE).exists(),
            "legacy db must be removed"
        );
        assert!(dir.join(DB_FILE).exists(), "cme db must be created");

        // 显式写入 → 检索 → 删除 全链路（走 provider 的冻结通道之一 + 引擎动词）。
        let memory = host
            .engine
            .remember(RememberRequest {
                content: "用户偏好简洁回答".into(),
                kind: MemoryKind::Preference,
                project_id: None,
                thread_id: None,
            })
            .await
            .unwrap();
        let hits = host.engine.recall_in_scope(None, "简洁", 10).await.unwrap();
        assert_eq!(hits.len(), 1);
        assert_eq!(hits[0].memory.id, memory.id);
        assert!(host.engine.forget(&memory.id).await.unwrap());
        assert!(host
            .engine
            .recall_in_scope(None, "简洁", 10)
            .await
            .unwrap()
            .is_empty());

        let _ = std::fs::remove_dir_all(&dir);
    }
}
