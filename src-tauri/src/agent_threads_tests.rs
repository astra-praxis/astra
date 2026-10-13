use super::*;
use agent_core::{
    AutoBridge, Decision, Delta, Envelope, MessageContent, ModelCapability, ModelRequest, Role,
    ToolCall,
};
use serde_json::json;

struct Database(std::path::PathBuf);
impl Database {
    fn new() -> Self {
        let path = std::env::temp_dir().join(format!("astra-host-{}", uuid::Uuid::new_v4()));
        Self(path)
    }
}
impl Drop for Database {
    fn drop(&mut self) {
        let _ = std::fs::remove_dir_all(&self.0);
    }
}

#[derive(Default)]
struct Backend(Mutex<Vec<ModelRequest>>, Mutex<Option<ToolCall>>, bool);
#[async_trait::async_trait]
impl ModelBackend for Backend {
    fn capabilities(&self) -> ModelCapability {
        ModelCapability {
            tool_call: true,
            vision: self.2,
            ..ModelCapability::default()
        }
    }
    async fn stream_completion(
        &self,
        request: ModelRequest,
    ) -> agent_core::Result<agent_core::model::DeltaStream> {
        self.0.lock().unwrap().push(request);
        if let Some(call) = self.1.lock().unwrap().take() {
            return Ok(Box::pin(futures_util::stream::iter(vec![Ok(
                Delta::ToolCalls { calls: vec![call] },
            )])));
        }
        Ok(Box::pin(futures_util::stream::iter(vec![Ok(
            Delta::Text {
                text: "reply".into(),
            },
        )])))
    }
}

struct Echo;
#[async_trait::async_trait]
impl agent_core::AgentTool for Echo {
    fn name(&self) -> &str {
        "test_echo"
    }
    fn description(&self) -> &str {
        "Return a deterministic test result"
    }
    fn parameters_schema(&self) -> serde_json::Value {
        json!({"type":"object"})
    }
    fn category(&self) -> agent_core::ToolCategory {
        agent_core::ToolCategory::Filesystem
    }
    fn risk(&self) -> agent_core::ToolRisk {
        agent_core::ToolRisk::Low
    }
    async fn call(
        &self,
        _: serde_json::Value,
        _: &agent_core::ToolContext,
    ) -> agent_core::Result<agent_core::ToolOutput> {
        Ok(agent_core::ToolOutput::from("complete\noutput".to_owned()))
    }
}

async fn load(host: &mut AgentThreads, id: &str, backend: Arc<Backend>) -> Result<(), String> {
    let mut config = ThreadConfig::new(id, "Test host");
    config.auto_compact_tokens = None;
    config.policy.allowed_readonly.push("test_echo".into());
    host.load_or_create(
        config,
        backend,
        ToolRegistry::new().register(Echo),
        Arc::new(AutoBridge(Decision::Allow)),
    )
    .await
}
async fn drain(thread: &Thread) {
    let (tx, mut rx) = tokio::sync::mpsc::channel::<Envelope>(32);
    let _handle = thread.try_send("continue", tx).unwrap();
    tokio::time::timeout(std::time::Duration::from_secs(5), async {
        while rx.recv().await.is_some() {}
    })
    .await
    .unwrap();
}
fn errors() -> PersistenceErrors {
    Arc::new(Mutex::new(HashMap::new()))
}

#[tokio::test]
async fn project_image_read_reaches_model_and_sqlite_without_event_payloads() {
    // Locally generated SDK PNG fixture, not a remote image.
    let database = Database::new();
    let mut host = AgentThreads::default();
    host.open(&database.0, errors()).await.unwrap();
    std::fs::write(
        database.0.join("pixel.png"),
        include_bytes!("image_test.png"),
    )
    .unwrap();
    host.bind_image_config("image-read", true, "https://gateway/v1", "vision")
        .await
        .unwrap();
    let backend = Arc::new(Backend(
        Mutex::new(vec![]),
        Mutex::new(Some(ToolCall {
            id: "image-call".into(),
            call_id: None,
            name: "read_file".into(),
            arguments: json!({"path": "pixel.png"}),
        })),
        true,
    ));
    let mut config = ThreadConfig::new("image-read", "Read the project image");
    config.project_root = Some(database.0.clone());
    host.load_or_create(
        config,
        backend.clone(),
        agent_core::fs_basic::register(ToolRegistry::new(), &database.0),
        Arc::new(AutoBridge(Decision::Allow)),
    )
    .await
    .unwrap();
    let (tx, mut rx) = tokio::sync::mpsc::channel::<Envelope>(32);
    let _handle = host.threads["image-read"]
        .try_send("Read pixel.png", tx)
        .unwrap();
    let events = tokio::time::timeout(std::time::Duration::from_secs(5), async {
        let mut events = vec![];
        while let Some(event) = rx.recv().await {
            events.push(event);
        }
        events
    })
    .await
    .unwrap();
    let serialized = serde_json::to_string(&events).unwrap();
    assert!(!serialized.contains("data:image"));
    assert!(!serialized.contains("iVBOR"));
    assert!(serialized.contains("normalized_mime"));
    assert!(backend
        .0
        .lock()
        .unwrap()
        .iter()
        .any(|request| request.messages.iter().any(|message| message
            .content
            .iter()
            .any(|content| matches!(content, MessageContent::ToolImage { .. })))));
    drop(host);
    std::fs::remove_file(database.0.join("pixel.png")).unwrap();
    let mut host = AgentThreads::default();
    host.open(&database.0, errors()).await.unwrap();
    host.bind_image_config("image-read", true, "https://gateway/v1", "vision")
        .await
        .unwrap();
    let backend = Arc::new(Backend(Mutex::new(vec![]), Mutex::new(None), true));
    load(&mut host, "image-read", backend.clone())
        .await
        .unwrap();
    drain(&host.threads["image-read"]).await;
    assert!(backend.0.lock().unwrap()[0]
        .messages
        .iter()
        .any(|message| message
            .content
            .iter()
            .any(|content| matches!(content, MessageContent::ToolImage { .. }))));
}

#[tokio::test]
async fn image_consent_is_bound_before_cache_and_survives_restart() {
    let database = Database::new();
    let mut host = AgentThreads::default();
    host.open(&database.0, errors()).await.unwrap();
    host.bind_image_config("images", true, "https://gateway/v1", "vision")
        .await
        .unwrap();
    load(&mut host, "images", Arc::new(Backend::default()))
        .await
        .unwrap();
    drain(&host.threads["images"]).await;
    for (enabled, endpoint, model) in [
        (false, "https://gateway/v1", "vision"),
        (true, "https://other/v1", "vision"),
        (true, "https://gateway/v1", "other"),
    ] {
        assert!(host
            .bind_image_config("images", enabled, endpoint, model)
            .await
            .is_err());
    }
    drop(host);
    let mut host = AgentThreads::default();
    host.open(&database.0, errors()).await.unwrap();
    host.bind_image_config("images", true, "https://gateway/v1", "vision")
        .await
        .unwrap();
    assert!(host
        .bind_image_config("images", false, "https://gateway/v1", "vision")
        .await
        .is_err());
    host.bind_image_config("new", false, "https://gateway/v1", "vision")
        .await
        .unwrap();
    load(&mut host, "legacy", Arc::new(Backend::default()))
        .await
        .unwrap();
    assert!(host
        .bind_image_config("legacy", true, "https://gateway/v1", "vision")
        .await
        .is_err());
    host.bind_image_config("legacy", false, "https://gateway/v1", "vision")
        .await
        .unwrap();
}

#[tokio::test]
async fn restart_restores_complete_tool_history_to_model_request() {
    let database = Database::new();
    let mut host = AgentThreads::default();
    host.open(&database.0, errors()).await.unwrap();
    let first_backend = Arc::new(Backend::default());
    *first_backend.1.lock().unwrap() = Some(ToolCall {
        id: "call-1".into(),
        call_id: None,
        name: "test_echo".into(),
        arguments: json!({"path":"a"}),
    });
    load(&mut host, "stable-ui-id", first_backend)
        .await
        .unwrap();
    let id = "stable-ui-id".to_owned();
    assert_eq!(host.threads[&id].thread_id(), Some(&id));
    let history = vec![
        ChatMessage {
            role: Role::User,
            content: vec![MessageContent::text("continue")],
        },
        ChatMessage {
            role: Role::Assistant,
            content: vec![MessageContent::ToolUse {
                call: ToolCall {
                    id: "call-1".into(),
                    call_id: None,
                    name: "test_echo".into(),
                    arguments: json!({"path":"a"}),
                },
            }],
        },
        ChatMessage {
            role: Role::User,
            content: vec![MessageContent::ToolResult {
                call_id: "call-1".into(),
                content: "complete\noutput".into(),
                is_error: false,
            }],
        },
        ChatMessage {
            role: Role::Assistant,
            content: vec![MessageContent::text("reply")],
        },
    ];
    drain(&host.threads[&id]).await;
    assert_eq!(json!(host.threads[&id].messages()), json!(history));
    drop(host);
    let mut host = AgentThreads::default();
    host.open(&database.0, errors()).await.unwrap();
    let backend = Arc::new(Backend::default());
    load(&mut host, &id, backend.clone()).await.unwrap();
    assert_eq!(json!(host.threads[&id].messages()), json!(history));
    drain(&host.threads[&id]).await;
    let mut expected = history;
    expected.push(ChatMessage {
        role: Role::User,
        content: vec![MessageContent::text("continue")],
    });
    assert_eq!(
        json!(backend.0.lock().unwrap()[0].messages),
        json!(expected)
    );
    assert_eq!(host.store().unwrap().messages(&id).await.unwrap().len(), 6);
}

#[tokio::test]
async fn unload_preserves_history_delete_removes_it_and_blocks_recreation() {
    let database = Database::new();
    let errors = errors();
    let mut host = AgentThreads::default();
    host.open(&database.0, errors.clone()).await.unwrap();
    load(&mut host, "a", Arc::new(Backend::default()))
        .await
        .unwrap();
    drain(&host.threads["a"]).await;
    host.unload("a", &errors).unwrap();
    load(&mut host, "a", Arc::new(Backend::default()))
        .await
        .unwrap();
    assert_eq!(host.threads["a"].messages().len(), 2);
    load(&mut host, "b", Arc::new(Backend::default()))
        .await
        .unwrap();
    assert!(host.threads["b"].messages().is_empty());
    host.delete("a").await.unwrap();
    host.delete("a").await.unwrap();
    assert!(host
        .store()
        .unwrap()
        .get(&"a".into())
        .await
        .unwrap()
        .is_none());
    assert!(load(&mut host, "a", Arc::new(Backend::default()))
        .await
        .is_err());
    assert!(host
        .store()
        .unwrap()
        .get(&"b".into())
        .await
        .unwrap()
        .is_some());
}

#[tokio::test]
async fn storage_errors_are_reported_and_prevent_unload() {
    let database = Database::new();
    let errors = errors();
    let mut host = AgentThreads::default();
    host.open(&database.0, errors.clone()).await.unwrap();
    load(&mut host, "a", Arc::new(Backend::default()))
        .await
        .unwrap();
    // Remove the row underneath the runtime to force SDK write-through failure.
    host.store().unwrap().delete(&"a".into()).await.unwrap();
    drain(&host.threads["a"]).await;
    assert!(errors.lock().unwrap().contains_key("a"));
    assert!(host.unload("a", &errors).is_err());
    assert_eq!(host.threads["a"].messages().len(), 2);
}

#[tokio::test]
async fn invalid_database_path_and_empty_session_fail_without_memory_fallback() {
    let database = Database::new();
    std::fs::write(&database.0, "not a directory").unwrap();
    let mut host = AgentThreads::default();
    assert!(host.open(&database.0, errors()).await.is_err());
    assert!(host.store.is_none());
    std::fs::remove_file(&database.0).unwrap();
    host.open(&database.0, errors()).await.unwrap();
    assert!(load(&mut host, "", Arc::new(Backend::default()))
        .await
        .is_err());
}

#[tokio::test]
async fn lifecycle_gate_remains_exclusive_through_cancelled_drain() {
    let database = Database::new();
    let host = Arc::new(tokio::sync::Mutex::new(AgentThreads::default()));
    let mut guard = host.clone().lock_owned().await;
    guard.open(&database.0, errors()).await.unwrap();
    load(&mut guard, "a", Arc::new(Backend::default()))
        .await
        .unwrap();
    let (tx, mut rx) = tokio::sync::mpsc::channel::<Envelope>(1);
    let handle = guard.threads["a"].try_send("cancelled", tx).unwrap();
    handle.cancel();
    assert!(
        host.try_lock().is_err(),
        "send, unload and delete must reject admission"
    );
    tokio::time::timeout(std::time::Duration::from_secs(5), async {
        while rx.recv().await.is_some() {}
    })
    .await
    .unwrap();
    assert!(host.try_lock().is_err());
    drop(guard);
    let mut guard = host.try_lock().unwrap();
    guard.delete("a").await.unwrap();
    assert!(guard
        .store()
        .unwrap()
        .get(&"a".into())
        .await
        .unwrap()
        .is_none());
}
