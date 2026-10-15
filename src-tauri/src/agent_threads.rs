//! Host-owned stable IDs, SQLite assembly and persistence error reporting.
use std::{
    collections::{HashMap, HashSet},
    path::Path,
    sync::{Arc, Mutex},
};

use agent_core::{
    artifact_registry::ArtifactRegistry,
    astra_store_sqlite::{SqliteArtifactRegistry, SqliteCheckpointStore, SqliteThreadStore},
    ChatMessage, CheckpointStore, MessageRecord, ModelBackend, PermissionBridge, Thread,
    ThreadConfig, ThreadQuery, ThreadRecord, ThreadStore, ToolRegistry,
};

pub type PersistenceErrors = Arc<Mutex<HashMap<String, String>>>;

/// The SDK intentionally keeps failed writes in memory. Retain those errors for
/// IPC status and refuse to unload a thread whose unsaved suffix would be lost.
pub struct ReportingStore {
    inner: Arc<dyn ThreadStore>,
    pub errors: PersistenceErrors,
}

impl ReportingStore {
    fn observe_write<T>(&self, id: &str, result: agent_core::Result<T>) -> agent_core::Result<T> {
        if let Err(error) = &result {
            self.errors
                .lock()
                .unwrap_or_else(|e| e.into_inner())
                .insert(id.to_owned(), error.to_string());
        }
        result
    }
}

#[async_trait::async_trait]
impl ThreadStore for ReportingStore {
    async fn create(&self, record: ThreadRecord) -> agent_core::Result<ThreadRecord> {
        self.inner.create(record).await
    }
    async fn get(&self, id: &String) -> agent_core::Result<Option<ThreadRecord>> {
        self.inner.get(id).await
    }
    async fn update(&self, record: ThreadRecord) -> agent_core::Result<()> {
        let id = record.id.clone();
        self.observe_write(&id, self.inner.update(record).await)
    }
    async fn append_message(
        &self,
        id: &String,
        message: ChatMessage,
    ) -> agent_core::Result<MessageRecord> {
        self.observe_write(id, self.inner.append_message(id, message).await)
    }
    async fn messages(&self, id: &String) -> agent_core::Result<Vec<MessageRecord>> {
        self.inner.messages(id).await
    }
    async fn rewind_messages_to(&self, id: &String, seq: u64) -> agent_core::Result<()> {
        // Write operation: observe for persistence-error accounting so the
        // host gate treats a failed rewind like any other failed write
        // (CHECKPOINT_SPEC §7 restore coordinator relies on this).
        self.observe_write(id, self.inner.rewind_messages_to(id, seq).await)
    }
    async fn delete(&self, id: &String) -> agent_core::Result<()> {
        self.observe_write(id, self.inner.delete(id).await)
    }
    async fn list(&self, query: ThreadQuery) -> agent_core::Result<Vec<ThreadRecord>> {
        self.inner.list(query).await
    }
}

#[derive(Default)]
pub struct AgentThreads {
    store: Option<Arc<dyn ThreadStore>>,
    /// Checkpoint persistence shares the same sqlite file as `store`
    /// (CHECKPOINT_SPEC §9: separate trait, separate tables, no cascade).
    checkpoints: Option<Arc<dyn CheckpointStore>>,
    /// Artifact registry shares the same sqlite file as `store`
    /// (ARTIFACT_SPEC §1.3: same crate, separate tables, no cascade).
    artifacts: Option<Arc<dyn ArtifactRegistry>>,
    pub threads: HashMap<String, Thread>,
    /// Terminal admission markers, including deletions whose cleanup must be retried.
    deleted: HashSet<String>,
}

impl AgentThreads {
    pub async fn open(
        &mut self,
        directory: &Path,
        errors: PersistenceErrors,
    ) -> Result<(), String> {
        if self.store.is_none() {
            tokio::fs::create_dir_all(directory)
                .await
                .map_err(|e| e.to_string())?;
            let path = directory.join("agent_threads.sqlite3");
            let inner = SqliteThreadStore::open(&path)
                .await
                .map_err(|e| e.to_string())?;
            self.store = Some(Arc::new(ReportingStore {
                inner: Arc::new(inner),
                errors,
            }));
            // Same database file, separate tables (spec §9). Opened once per
            // process; a failure here is fatal for checkpoint features.
            let checkpoints = SqliteCheckpointStore::open(&path)
                .await
                .map_err(|e| e.to_string())?;
            self.checkpoints = Some(Arc::new(checkpoints));
            // Artifact records share the file too (ARTIFACT_SPEC §1.3).
            // Fatal for artifact features only; threads/checkpoints above
            // stay usable.
            let artifacts = SqliteArtifactRegistry::open(&path)
                .await
                .map_err(|e| e.to_string())?;
            self.artifacts = Some(agent_core::astra_store_sqlite::shared_artifact_registry(
                artifacts,
            ));
        }
        Ok(())
    }

    pub fn checkpoints(&self) -> Result<Arc<dyn CheckpointStore>, String> {
        self.checkpoints
            .clone()
            .ok_or_else(|| "Agent thread store is not open".into())
    }

    pub fn artifacts(&self) -> Result<Arc<dyn ArtifactRegistry>, String> {
        self.artifacts
            .clone()
            .ok_or_else(|| "Agent thread store is not open".into())
    }

    fn store(&self) -> Result<Arc<dyn ThreadStore>, String> {
        self.store
            .clone()
            .ok_or_else(|| "Agent thread store is not open".into())
    }

    /// Bind consent and destination durably before consulting the cached Thread.
    /// Legacy sessions have no image consent and therefore remain disabled.
    ///
    /// 语义（图片隐私边界只保护"图片流向"，不钉死纯文本会话的模型来源）：
    /// - 开启图片识别：目的地（endpoint+model）必须与已记录一致，否则要求
    ///   新建会话——历史图片可能随上下文重放到新模型，属于 consent 违约；
    /// - 关闭图片识别的会话不允许关闭（同上，防"关-换-重开"绕过绑定）；
    /// - 纯文本会话（两侧均关闭）：目的地可自由切换，返回 true 表示
    ///   目的地相对已缓存 Thread 的 backend 发生变化（调用方需重建 Thread）。
    pub async fn bind_image_config(
        &self,
        id: &str,
        enabled: bool,
        endpoint: &str,
        model: &str,
    ) -> Result<bool, String> {
        if id.trim().is_empty() || self.deleted.contains(id) {
            return Err("Invalid or deleted agent session ID".into());
        }
        let store = self.store()?;
        let existing = store.get(&id.to_owned()).await.map_err(|e| e.to_string())?;
        let requested = serde_json::json!({
            "enabled": enabled, "endpoint": endpoint, "model": model,
        });
        if let Some(mut record) = existing {
            let mut dest_changed = false;
            match record.metadata.get("host_image_config") {
                Some(saved) if saved == &requested => return Ok(false),
                Some(saved) => {
                    let saved_enabled = saved["enabled"].as_bool().unwrap_or(false);
                    let same_dest = saved["endpoint"].as_str() == Some(endpoint)
                        && saved["model"].as_str() == Some(model);
                    if enabled {
                        // 图片将流向新目的地：必须与已记录绑定一致。
                        if !same_dest {
                            return Err("图片识别或模型来源与本会话绑定配置不一致；请恢复原设置，或新建会话使设置生效。".into());
                        }
                    } else if saved_enabled {
                        // 已开启的会话不允许关闭：防止"关闭→换模型→重开"
                        // 把历史图片随上下文送到新目的地。
                        return Err("图片识别或模型来源与本会话绑定配置不一致；请恢复原设置，或新建会话使设置生效。".into());
                    } else {
                        // 纯文本会话：图片从未流动，模型来源可自由切换。
                        dest_changed = !same_dest;
                    }
                }
                None if enabled => {
                    return Err(
                        "此历史会话未开启图片识别；请关闭图片识别后续聊，或新建会话开启。".into(),
                    );
                }
                None => {}
            }
            record
                .metadata
                .insert("host_image_config".into(), requested);
            store.update(record).await.map_err(|e| e.to_string())?;
            Ok(dest_changed)
        } else {
            let mut metadata = serde_json::Map::new();
            metadata.insert("host_image_config".into(), requested);
            store
                .create(ThreadRecord {
                    id: id.to_owned(),
                    title: String::new(),
                    created_at: 0,
                    updated_at: 0,
                    metadata,
                    thinking_enabled: None,
                    thinking_effort: None,
                })
                .await
                .map_err(|e| e.to_string())?;
            Ok(false)
        }
    }

    pub async fn load_or_create(
        &mut self,
        config: ThreadConfig,
        backend: Arc<dyn ModelBackend>,
        registry: ToolRegistry,
        bridge: Arc<dyn PermissionBridge>,
    ) -> Result<(), String> {
        let id = config.session_id.clone();
        if id.trim().is_empty() || self.deleted.contains(&id) {
            return Err("Invalid or deleted agent session ID".into());
        }
        if self.threads.contains_key(&id) {
            return Ok(());
        }
        let store = self.store()?;
        if store.get(&id).await.map_err(|e| e.to_string())?.is_none() {
            store
                .create(ThreadRecord {
                    id: id.clone(),
                    title: String::new(),
                    created_at: 0,
                    updated_at: 0,
                    metadata: serde_json::Map::new(),
                    thinking_enabled: None,
                    thinking_effort: None,
                })
                .await
                .map_err(|e| e.to_string())?;
        }
        let thread = Thread::load(store, &id, config, backend, registry, bridge)
            .await
            .map_err(|e| e.to_string())?;
        self.threads.insert(id, thread);
        Ok(())
    }

    pub fn unload(&mut self, id: &str, errors: &PersistenceErrors) -> Result<(), String> {
        if self.deleted.contains(id) && self.threads.contains_key(id) {
            return Err("Cannot unload agent session with pending deletion; retry deletion".into());
        }
        if let Some(error) = errors.lock().unwrap_or_else(|e| e.into_inner()).get(id) {
            return Err(format!("Cannot unload unsaved agent session: {error}"));
        }
        self.threads.remove(id);
        Ok(())
    }

    /// Caller holds the lifecycle gate, so no Run or write-through can race deletion.
    pub async fn delete(&mut self, id: &str) -> Result<(), String> {
        let store = self.store()?;
        // Match the SDK's terminal admission gate even if cleanup fails or is interrupted.
        // Keep the cached Thread until success so retries retain its artifact registry.
        self.deleted.insert(id.to_owned());
        if let Some(thread) = self.threads.get(id) {
            thread.delete().await.map_err(|e| e.to_string())?;
        } else {
            store
                .delete(&id.to_owned())
                .await
                .map_err(|e| e.to_string())?;
        }
        // Checkpoints are NOT cascaded from ThreadStore (spec §1.3/§9): the
        // teardown flow calls delete_thread explicitly. Best-effort — a failed
        // cleanup leaves soft-deleted rows for retention, never blocks removal.
        if let Some(checkpoints) = &self.checkpoints {
            if let Err(error) = checkpoints.delete_thread(&id.to_owned()).await {
                eprintln!("agent checkpoint cleanup failed on session delete: {error}");
            }
        }
        self.threads.remove(id);
        Ok(())
    }
}

#[cfg(test)]
#[path = "agent_threads_tests.rs"]
mod tests;
