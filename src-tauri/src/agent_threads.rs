//! Host-owned stable IDs, SQLite assembly and persistence error reporting.
use std::{
    collections::{HashMap, HashSet},
    path::Path,
    sync::{Arc, Mutex},
};

use agent_core::{
    astra_store_sqlite::{SqliteCheckpointStore, SqliteThreadStore},
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
        }
        Ok(())
    }

    pub fn checkpoints(&self) -> Result<Arc<dyn CheckpointStore>, String> {
        self.checkpoints
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
    pub async fn bind_image_config(
        &self,
        id: &str,
        enabled: bool,
        endpoint: &str,
        model: &str,
    ) -> Result<(), String> {
        if id.trim().is_empty() || self.deleted.contains(id) {
            return Err("Invalid or deleted agent session ID".into());
        }
        let store = self.store()?;
        let existing = store.get(&id.to_owned()).await.map_err(|e| e.to_string())?;
        let requested = serde_json::json!({
            "enabled": enabled, "endpoint": endpoint, "model": model,
        });
        if let Some(mut record) = existing {
            match record.metadata.get("host_image_config") {
                Some(saved) if saved != &requested => {
                    return Err("图片识别或模型来源与本会话绑定配置不一致；请恢复原设置，或新建会话使设置生效。".into());
                }
                Some(_) => return Ok(()),
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
        }
        Ok(())
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
