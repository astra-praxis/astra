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

use astra_memory::cme::engine::{MemoryEngine, RememberRequest};
use astra_memory::cme::model::MemoryKind;
use astra_memory::cme::store_sqlite::CmeSqliteStore;
use astra_memory_provider::CmeMemoryProvider;

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

impl MemoryHost {
    /// 打开（或首次创建）加密存储并装配引擎。任何失败都由调用方降级为
    /// memory-off（agent 照常运行，只是没有 recall 注入）。
    pub fn init(dir: &Path) -> Result<Self, String> {
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
        let engine = Arc::new(MemoryEngine::new(store));
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

/// 硬删除单条（级联关系与版本链引用）。
#[tauri::command]
pub async fn memory_forget(
    state: tauri::State<'_, crate::agent::AgentState>,
    id: String,
) -> Result<(), String> {
    let host = host_or_err(&state)?;
    if !host.engine.forget(&id).await.map_err(|e| e.to_string())? {
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

        let host = MemoryHost::init(&dir).unwrap();
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
