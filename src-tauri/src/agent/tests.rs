use std::collections::HashMap;
use std::sync::{Arc, Mutex};

use agent_core::{
    Decision, Envelope, MessageContent, ModelBackend, PermissionBridge, PermissionRequest,
    RigBackend, ThreadConfig, ToolPolicy, DEFAULT_AUTO_COMPACT_TOKENS,
};
use base64::Engine as _;
use serde_json::json;
use std::path::Path;

use super::events::*;
use super::permissions::*;
use super::questions::*;
use super::registry::*;
use super::run::*;
use super::state::*;
use super::status::*;

/// 多级放权：级别解析
#[test]
fn auto_mode_parse_roundtrip() {
    assert_eq!(AutoMode::parse("off"), Ok(AutoMode::Off));
    assert_eq!(AutoMode::parse("project"), Ok(AutoMode::Project));
    assert_eq!(AutoMode::parse("workspace"), Ok(AutoMode::Workspace));
    assert_eq!(AutoMode::parse("all"), Ok(AutoMode::All));
    assert!(AutoMode::parse("true").is_err());
    assert!(AutoMode::parse("").is_err());
    for m in [
        AutoMode::Off,
        AutoMode::Project,
        AutoMode::Workspace,
        AutoMode::All,
    ] {
        assert_eq!(AutoMode::parse(m.as_str()), Ok(m));
        assert_eq!(AutoMode::from_u8(m as u8), m);
    }
}

/// 多级放权：路径参数提取（已知键、非字符串与未知键忽略）
#[test]
fn candidate_paths_extracts_known_string_keys() {
    let args = serde_json::json!({
        "path": "src/main.rs",
        "output": "out.docx",
        "content": "text",
        "n": 3,
        "command": "rm -rf /",
        "from": "a.md",
        "to": 123
    });
    let mut got = candidate_paths(&args);
    got.sort();
    assert_eq!(got, vec!["a.md", "out.docx", "src/main.rs"]);
    assert!(candidate_paths(&serde_json::json!({"command": "ls"})).is_empty());
    assert!(candidate_paths(&serde_json::json!("not-an-object")).is_empty());
}

/// 多级放权：路径解析（相对路径相对项目根、~ 展开、词法归一化）
#[test]
fn resolve_arg_path_joins_and_normalizes() {
    let root = std::path::Path::new("/home/u/proj");
    assert_eq!(
        resolve_arg_path("src/main.rs", Some(root)).unwrap(),
        std::path::PathBuf::from("/home/u/proj/src/main.rs")
    );
    assert_eq!(
        resolve_arg_path("./a/../b.txt", Some(root)).unwrap(),
        std::path::PathBuf::from("/home/u/proj/b.txt")
    );
    assert_eq!(
        resolve_arg_path("/abs/x.txt", Some(root)).unwrap(),
        std::path::PathBuf::from("/abs/x.txt")
    );
    // 无项目根时相对路径不可解析
    assert!(resolve_arg_path("rel.txt", None).is_none());
    // ~ 展开（读真实 HOME 动态断言，避免进程全局环境变量竞争）
    if let Some(home) = std::env::var_os("HOME") {
        assert_eq!(
            resolve_arg_path("~/notes/a.md", Some(root)).unwrap(),
            std::path::PathBuf::from(home).join("notes/a.md")
        );
    }
}

/// 多级放权：纯判定 —— Off 永不，All 恒真，Project/Workspace 按边界，
/// 无路径参数（shell）或边界为空时回退确认
#[test]
fn auto_allow_respects_level_boundaries() {
    let root = std::path::PathBuf::from("/home/u/proj");
    let other_ws = std::path::PathBuf::from("/home/u/other-ws");
    let inside = serde_json::json!({"path": "src/main.rs"});
    let escape = serde_json::json!({"path": "../outside.txt"});
    let absolute_outside = serde_json::json!({"path": "/etc/passwd"});
    let shell = serde_json::json!({"command": "cargo build"});
    let ws_roots = vec![root.clone(), other_ws];

    // Off：永不放行
    assert!(!auto_allow(AutoMode::Off, &inside, Some(&root), &ws_roots));
    // All：无路径参数也放行（shell 等）
    assert!(auto_allow(AutoMode::All, &shell, Some(&root), &ws_roots));
    // Project：项目内相对路径放行
    assert!(auto_allow(
        AutoMode::Project,
        &inside,
        Some(&root),
        &ws_roots
    ));
    // Project：逃逸（..）与项目外绝对路径不放行
    assert!(!auto_allow(
        AutoMode::Project,
        &escape,
        Some(&root),
        &ws_roots
    ));
    assert!(!auto_allow(
        AutoMode::Project,
        &absolute_outside,
        Some(&root),
        &ws_roots
    ));
    // Project：另一工作空间内的绝对路径不放行（越出项目边界）
    assert!(!auto_allow(
        AutoMode::Project,
        &serde_json::json!({"path": "/home/u/other-ws/a.txt"}),
        Some(&root),
        &ws_roots
    ));
    // Project：无项目根（纯聊天）不放行
    assert!(!auto_allow(AutoMode::Project, &inside, None, &ws_roots));
    // Workspace：任一工作空间内的绝对路径放行
    assert!(auto_allow(
        AutoMode::Workspace,
        &serde_json::json!({"path": "/home/u/other-ws/a.txt"}),
        Some(&root),
        &ws_roots
    ));
    // Workspace：项目内相对路径放行（项目根在工作空间内）
    assert!(auto_allow(
        AutoMode::Workspace,
        &inside,
        Some(&root),
        &ws_roots
    ));
    // Workspace：边界外不放行
    assert!(!auto_allow(
        AutoMode::Workspace,
        &escape,
        Some(&root),
        &ws_roots
    ));
    assert!(!auto_allow(
        AutoMode::Workspace,
        &absolute_outside,
        Some(&root),
        &ws_roots
    ));
    // shell 等无路径参数在任何路径级别都不放行
    assert!(!auto_allow(
        AutoMode::Project,
        &shell,
        Some(&root),
        &ws_roots
    ));
    assert!(!auto_allow(
        AutoMode::Workspace,
        &shell,
        Some(&root),
        &ws_roots
    ));
    // 未配置工作空间时 Workspace 级别回退确认
    assert!(!auto_allow(AutoMode::Workspace, &inside, Some(&root), &[]));
}

/// 多级放权：Hub 级别存取
#[test]
fn hub_stores_auto_mode() {
    let hub = PermissionHub::default();
    assert_eq!(hub.auto_mode(), AutoMode::Off);
    hub.set_auto_mode(AutoMode::Workspace);
    assert_eq!(hub.auto_mode(), AutoMode::Workspace);
    hub.set_auto_mode(AutoMode::All);
    assert_eq!(hub.auto_mode(), AutoMode::All);
}

/// 多模态 M1：内容块组装（文本在前图片随后，纯图片合法，空输入拒绝）
#[test]
fn build_user_content_orders_text_and_images() {
    let png: &[u8] = &[
        0x89, 0x50, 0x4E, 0x47, 0x0D, 0x0A, 0x1A, 0x0A, 0x00, 0x00, 0x00, 0x0D, 0x49, 0x48, 0x44,
        0x52, 0x00, 0x00, 0x00, 0x01, 0x00, 0x00, 0x00, 0x01, 0x08, 0x02, 0x00, 0x00, 0x00, 0x90,
        0x77, 0x53, 0xDE, 0x00, 0x00, 0x00, 0x0C, 0x49, 0x44, 0x41, 0x54, 0x08, 0xD7, 0x63, 0xF8,
        0xCF, 0xC0, 0x00, 0x00, 0x03, 0x01, 0x01, 0x00, 0x18, 0xDD, 0x8D, 0xB0, 0x00, 0x00, 0x00,
        0x00, 0x49, 0x45, 0x4E, 0x44, 0xAE, 0x42, 0x60, 0x82,
    ];
    let img = |data: &str| AgentImageInput {
        data: data.into(),
        width: 1,
        height: 1,
    };
    let b64 = base64::engine::general_purpose::STANDARD.encode(png);
    let no_dir: Option<std::path::PathBuf> = None;

    // 纯文本：单 Text 块（与旧 try_send 行为一致）
    let content = build_user_content("hello", &[], None, &no_dir).unwrap();
    assert_eq!(content.len(), 1);
    assert!(matches!(&content[0], MessageContent::Text { text } if text == "hello"));

    // 文本 + 图片：顺序保持，图片为合法 Image 块
    let content = build_user_content("看图", &[img(&b64)], None, &no_dir).unwrap();
    assert_eq!(content.len(), 2);
    assert!(matches!(&content[0], MessageContent::Text { .. }));
    assert!(matches!(&content[1], MessageContent::Image { .. }));

    // 纯图片：允许（无空文本块）
    let content = build_user_content("", &[img(&b64)], None, &no_dir).unwrap();
    assert_eq!(content.len(), 1);
    assert!(matches!(&content[0], MessageContent::Image { .. }));

    // 空输入拒绝；非法 base64 / 坏 PNG 拒绝
    assert!(build_user_content("", &[], None, &no_dir).is_err());
    assert!(build_user_content("hi", &[img("!!!")], None, &no_dir).is_err());
    let bad = base64::engine::general_purpose::STANDARD.encode(b"broken png bytes");
    assert!(build_user_content("hi", &[img(&bad)], None, &no_dir).is_err());

    // 技能无技能库时明确报错（不静默降级为纯文本）
    assert!(build_user_content("hi", &[], Some("demo"), &no_dir).is_err());
}

#[test]
fn image_recognition_defaults_off_and_configures_capability() {
    let req: AgentStartReq = serde_json::from_value(json!({
        "session_id": "test", "user_text": "read image"
    }))
    .unwrap();
    assert!(!req.image_recognition);
    for enabled in [false, true] {
        let backend = RigBackend::new("http://localhost/v1", "", "arbitrary-model")
            .with_image_encoding(image_encoding(enabled));
        assert_eq!(backend.capabilities().vision, enabled);
    }
}

#[test]
fn question_payload_contains_session_id() {
    let payload = question_payload(
        "sess-1",
        "q-123",
        "选择哪种方案？",
        vec!["方案A".into(), "方案B".into()],
        true,
    );
    assert_eq!(payload["session_id"], "sess-1");
    assert_eq!(payload["question_id"], "q-123");
    assert_eq!(payload["question"], "选择哪种方案？");
    assert_eq!(payload["options"], json!(["方案A", "方案B"]));
    assert_eq!(payload["allow_free_text"], true);
}

#[test]
fn question_payload_empty_options() {
    let payload = question_payload("sess-2", "q-1", "一个问题？", vec![], false);
    assert_eq!(payload["session_id"], "sess-2");
    assert_eq!(payload["options"], json!([]));
    assert_eq!(payload["allow_free_text"], false);
}

/// 多模态 M5：工具内部权限事件的 Envelope 载荷（前端复用既有确认卡）
#[test]
fn in_tool_permission_event_carries_session_and_call_id() {
    let id = in_tool_permission_id();
    assert!(id.starts_with("perm-tool-"));
    let env = in_tool_permission_event("sess-9", &id, "generate_image", "生成 1 张图");
    assert_eq!(env["session_id"], "sess-9");
    assert_eq!(env["type"], "permission_requested");
    assert_eq!(env["tool_call_id"], id);
    assert_eq!(env["tool_name"], "generate_image");
    assert_eq!(env["summary"], "生成 1 张图");
    assert!(env["event_id"]
        .as_str()
        .unwrap()
        .starts_with("evt_in_tool_"));

    let decided = in_tool_permission_decided_event("sess-9", &id, Decision::Allow);
    assert_eq!(decided["type"], "permission_decided");
    assert_eq!(decided["tool_call_id"], id);
    assert_eq!(decided["decision"], "allow");
}

/// 多模态 M5：build_registry 在 enabled/disabled 下的 generate_image
/// 注册差异（disabled/缺配置 → 工具不出现，其余工具不受影响）
#[test]
fn build_registry_registers_generate_image_only_when_enabled() {
    struct NoQuestions2;
    #[async_trait::async_trait]
    impl agent_core::UserQuestionBridge for NoQuestions2 {
        async fn ask(
            &self,
            _question: String,
            _options: Vec<String>,
            _allow_free_text: bool,
        ) -> std::result::Result<String, String> {
            Err("no questions in test".into())
        }
    }
    struct StubProvider;
    #[async_trait::async_trait]
    impl agent_core::astra_generation::GenerationProvider for StubProvider {
        fn id(&self) -> &str {
            "stub"
        }
        fn descriptor(
            &self,
            _model_id: &str,
        ) -> Option<agent_core::model::capability::ModelDescriptor> {
            None
        }
        async fn submit(
            &self,
            _request: &agent_core::astra_generation::GenerationRequest,
            _client_request_id: &str,
        ) -> std::result::Result<
            agent_core::astra_generation::SubmitOutcome,
            agent_core::astra_generation::GenerationError,
        > {
            Err(
                agent_core::astra_generation::GenerationError::ProviderRejected {
                    message: "stub".into(),
                },
            )
        }
        async fn status(
            &self,
            _handle: &agent_core::astra_generation::ProviderTaskHandle,
        ) -> std::result::Result<
            agent_core::astra_generation::ProviderObservation,
            agent_core::astra_generation::GenerationError,
        > {
            Err(
                agent_core::astra_generation::GenerationError::ProviderRejected {
                    message: "stub".into(),
                },
            )
        }
        async fn result(
            &self,
            _handle: &agent_core::astra_generation::ProviderTaskHandle,
        ) -> std::result::Result<
            Vec<agent_core::astra_generation::GenerationOutput>,
            agent_core::astra_generation::GenerationError,
        > {
            Err(
                agent_core::astra_generation::GenerationError::ProviderRejected {
                    message: "stub".into(),
                },
            )
        }
        async fn cancel(
            &self,
            _handle: &agent_core::astra_generation::ProviderTaskHandle,
        ) -> std::result::Result<
            agent_core::astra_generation::CancelOutcome,
            agent_core::astra_generation::GenerationError,
        > {
            Err(
                agent_core::astra_generation::GenerationError::ProviderRejected {
                    message: "stub".into(),
                },
            )
        }
    }
    let registry_with = |generation| {
        build_registry(
            "sess-gen",
            None,
            None,
            Arc::new(NoQuestions2),
            agent_core::net_tools::SearchEngine::default(),
            Arc::new(agent_core::shell_background::ProcessManager::new()),
            Arc::new(agent_core::TaskManager::new(8, None)),
            generation,
            None,
        )
    };

    // disabled：不注册 generate_image，任务查询工具不受影响
    let reg = registry_with(None);
    assert!(reg.get("generate_image").is_none());
    assert!(reg.get("task_status").is_some());

    // enabled：generate_image 注册，归到 Confirm 之外的本工具付费确认
    let backend = Arc::new(agent_core::task::GenerationTaskBackend::new(
        Arc::new(StubProvider) as Arc<dyn agent_core::astra_generation::GenerationProvider>,
        Arc::new(crate::agent_generation::MediaStore::new(
            std::env::temp_dir().join("astra-agent-gen-test"),
        )) as Arc<dyn agent_core::task::AssetStore>,
    ));
    let provider: Arc<dyn agent_core::astra_generation::GenerationProvider> =
        Arc::new(StubProvider);
    let reg = registry_with(Some((Arc::clone(&backend), provider)));
    assert!(reg.get("generate_image").is_some());
}

/// 白名单完备性：registry 里每个工具（memory-off 形状）都必须在
/// host_tool_policy 里点名（Allow/Confirm）。权限是严格模型：漏了
/// 白名单 = 注册成功但执行被拒（memory 工具踩过：agent 看得到、调不了）。
#[test]
fn every_registered_tool_is_listed_in_host_policy() {
    struct NoQuestions3;
    #[async_trait::async_trait]
    impl agent_core::UserQuestionBridge for NoQuestions3 {
        async fn ask(
            &self,
            _question: String,
            _options: Vec<String>,
            _allow_free_text: bool,
        ) -> Result<String, String> {
            Err("test bridge".into())
        }
    }
    let reg = build_registry(
        "sess-policy",
        None,
        None,
        Arc::new(NoQuestions3),
        agent_core::net_tools::SearchEngine::default(),
        Arc::new(agent_core::shell_background::ProcessManager::new()),
        Arc::new(agent_core::TaskManager::new(8, None)),
        None,
        None,
    );
    let policy = host_tool_policy();
    for name in reg.names() {
        let listed = policy.allowed_readonly.iter().any(|n| n == &name)
            || policy.confirm_tools.iter().any(|n| n == &name);
        assert!(
            listed,
            "tool '{name}' 未列入 host_tool_policy，执行会被 Deny"
        );
    }
}

/// 宿主层闭环（TASK_SPEC §六）：AgentState::default 的共享 ProcessManager
/// 接线 → build_registry → shell_background spawn → adopt → task_status
/// 轮询 → task_result。全程走 registry 里的真实工具，不碰 manager 内部。
#[tokio::test]
async fn shell_background_adopts_into_task_tools_end_to_end() {
    struct AllowBridge;
    #[async_trait::async_trait]
    impl PermissionBridge for AllowBridge {
        async fn request(&self, _req: PermissionRequest) -> Decision {
            Decision::Allow
        }
    }

    struct NoQuestions;
    #[async_trait::async_trait]
    impl agent_core::UserQuestionBridge for NoQuestions {
        async fn ask(
            &self,
            _question: String,
            _options: Vec<String>,
            _allow_free_text: bool,
        ) -> Result<String, String> {
            Err("test bridge".into())
        }
    }

    let state = AgentState::default();
    let root = std::env::temp_dir().join(format!(
        "astra-host-e2e-{}-{:?}",
        std::process::id(),
        std::thread::current().id()
    ));
    let _ = std::fs::remove_dir_all(&root);
    std::fs::create_dir_all(&root).unwrap();

    let registry = build_registry(
        "sess-e2e",
        Some(&root),
        None,
        Arc::new(NoQuestions) as Arc<dyn agent_core::UserQuestionBridge>,
        agent_core::net_tools::SearchEngine::default(),
        Arc::clone(&state.processes),
        Arc::clone(&state.tasks),
        None,
        None,
    );
    let names = registry.names();
    for required in [
        "shell_background",
        "task_status",
        "task_result",
        "task_cancel",
    ] {
        assert!(names.contains(&required.into()), "missing tool: {required}");
    }

    let ctx = agent_core::ToolContext {
        project_root: Some(root.clone()),
        bridge: Arc::new(AllowBridge),
        cancelled: tokio_util::sync::CancellationToken::new(),
        deadline: None,
        run_id: None,
    };

    // 1. shell_background 启动短命令
    let bg = registry.get("shell_background").expect("bg tool");
    let out = bg
        .call(json!({ "command": "echo adopted-e2e; exit 0" }), &ctx)
        .await
        .expect("shell_background call");
    assert!(
        out.content
            .contains("Adopted into unified task tracking as task_id=task_"),
        "summary should surface unified task id: {}",
        out.content
    );
    let unified_id = out
        .content
        .split("Adopted into unified task tracking as task_id=")
        .nth(1)
        .and_then(|s| s.split(&['：', ':', ' '][..]).next())
        .expect("unified task id")
        .to_string();

    // 2. task_status 轮询到终态
    let status_tool = registry.get("task_status").expect("task_status tool");
    let mut final_label = String::new();
    for _ in 0..50 {
        let out = status_tool
            .call(json!({ "task_id": unified_id }), &ctx)
            .await
            .expect("task_status call");
        if out.content.contains("(terminal") {
            final_label = out.content;
            break;
        }
        tokio::time::sleep(std::time::Duration::from_millis(50)).await;
    }
    assert!(
        final_label.contains("completed"),
        "task_status should reach completed: {final_label}"
    );

    // 3. task_result 取回进程输出
    let result_tool = registry.get("task_result").expect("task_result tool");
    let out = result_tool
        .call(json!({ "task_id": unified_id }), &ctx)
        .await
        .expect("task_result call");
    assert!(
        out.content.contains("task completed") && out.content.contains("adopted-e2e"),
        "task_result should carry process output: {}",
        out.content
    );

    let _ = std::fs::remove_dir_all(&root);
}

// ===== 记忆采集端到端：宿主采集路径 + memory_search / 数据库两面 =====

use agent_core::memory::MemoryProvider as _;
use astra_memory::cme::engine::{MemoryEngine, RememberRequest};
use astra_memory::cme::model::MemoryKind;
use astra_memory::cme::store::MemoryStore as _;
use astra_memory::cme::store_sqlite::CmeSqliteStore;
use astra_memory_provider::{CmeMemoryProvider, ProjectBoundProvider};

fn envelope(event_id: &str, seq: u64, event: agent_core::RunEvent) -> Envelope {
    Envelope {
        schema_version: agent_core::events::EVENT_SCHEMA_VERSION,
        event_id: event_id.into(),
        timestamp: 1_700_000_000_000,
        session_id: "sess-mem-e2e".into(),
        run_id: "run-mem-e2e".into(),
        sequence: seq,
        event,
    }
}

/// 宿主采集路径端到端：模拟一次真实 Run 的完整事件流（用户提问 →
/// 工具失败 → 工具成功 → 助手总结），用与 observe spawn 相同的
/// MemoryRunCollector + 引擎调用写入真实加密 SQLite，然后验证两面：
/// memory_search 面（ProjectBoundProvider，即 core 实际调用路径）和
/// 数据库面（reopen + 摘要/游标 + 静态加密）。
#[tokio::test]
async fn memory_collection_end_to_end_both_surfaces() {
    let dir = tempfile::tempdir().unwrap();
    let db = dir.path().join("memory-cme.sqlite");
    let key = [7u8; 32];
    let engine = std::sync::Arc::new(MemoryEngine::new(CmeSqliteStore::open(&db, &key).unwrap()));
    let provider = CmeMemoryProvider::new(std::sync::Arc::clone(&engine));

    // ---- 1. 真实 Run 事件流（顺序与真实发射顺序一致）----
    let events = vec![
        envelope(
            "evt_run_1",
            1,
            agent_core::RunEvent::StatusChanged {
                status: agent_core::RunStatus::ExecutingTool,
            },
        ),
        envelope(
            "evt_run_2",
            2,
            agent_core::RunEvent::MessageDelta {
                text: "让我先查一下".into(),
            },
        ),
        envelope(
            "evt_run_3",
            3,
            agent_core::RunEvent::ToolCallStarted {
                tool_call_id: "tc1".into(),
                tool_name: "shell_execute".into(),
                args_summary: "which pandoc".into(),
                args: None,
            },
        ),
        // 失败：原因进摘要，永不直接成记忆
        envelope(
            "evt_run_4",
            4,
            agent_core::RunEvent::ToolCallFailed {
                tool_call_id: "tc1".into(),
                tool_name: "shell_execute".into(),
                error: "command not found: pandoc".into(),
            },
        ),
        // 成功：已验证路径，应提炼为长期记忆（experience）
        envelope(
            "evt_run_5",
            5,
            agent_core::RunEvent::ToolCallCompleted {
                tool_call_id: "tc2".into(),
                tool_name: "shell_execute".into(),
                result_summary: "转换完成".into(),
                raw_output: Some("成功，已验证实际路径 /opt/homebrew/bin/pandoc 可用".into()),
                artifacts: vec![],
            },
        ),
        // 助手总结：含未验证猜测，只进摘要永不成记忆
        envelope(
            "evt_run_6",
            6,
            agent_core::RunEvent::RunCompleted {
                final_text: "我觉得 pandoc 应该在 /usr/bin/pandoc，大概就是这个位置啦。".into(),
            },
        ),
    ];

    // ---- 2. 与 observe spawn 逐字相同的采集 + 提交路径 ----
    let mut collector = MemoryRunCollector::new();
    for env in &events {
        collector.observe(env);
    }
    let session = "sess-mem-e2e";
    let bound_project: Option<String> = Some("astra".into());
    engine
        .set_thread_scope(session, bound_project.as_deref())
        .await
        .unwrap();
    let now_ms = 1_700_000_100_000u64;
    let messages = collector.into_messages("帮我确认系统里 pandoc 的实际路径", now_ms);
    assert_eq!(messages.len(), 4); // user + failed + completed + assistant
    engine.ingest(session, messages);
    let force = engine.has_priority_pending();
    assert!(force, "verified tool result must trip the priority gate");
    engine.process_pending(force).await.unwrap();

    // ---- 3. memory_search 面：ProjectBoundProvider（core 实际调用形状）----
    let bound = ProjectBoundProvider::new(std::sync::Arc::new(provider), bound_project.clone());

    // 已验证工具路径可召回，kind = experience，内容含实际路径。
    let hits = bound
        .search("/opt/homebrew/bin/pandoc", None, 10)
        .await
        .unwrap();
    assert_eq!(hits.len(), 1, "verified tool path should be recallable");
    assert!(hits[0].content.contains("/opt/homebrew/bin/pandoc"));
    assert_eq!(hits[0].metadata["kind"], serde_json::json!("experience"));
    assert_eq!(hits[0].scope, agent_core::memory::MemoryScope::Project);

    // 助手猜测与失败原因不成为记忆（错误注入检查）。
    assert!(
        bound
            .search("/usr/bin/pandoc", None, 10)
            .await
            .unwrap()
            .is_empty(),
        "assistant guess must never become a memory"
    );
    assert!(
        bound
            .search("command not found", None, 10)
            .await
            .unwrap()
            .is_empty(),
        "raw failure must never become a memory"
    );

    // ---- 4. 数据库面：reopen + 摘要/游标持久化 + 静态加密 ----
    drop(bound);
    drop(engine);
    let reopened = CmeSqliteStore::open(&db, &key).unwrap();

    // 摘要保留完整会话流（含失败原因与助手文本），游标推进到最后一条
    // 采集事件（evt_run_6），下轮增量压缩从这里继续。
    let summary = reopened.load_summary(session).await.unwrap().unwrap();
    assert_eq!(summary.project_id.as_deref(), Some("astra"));
    assert!(
        summary.summary.contains("not found"),
        "failure goes to summary"
    );
    assert!(
        summary.summary.contains("/usr/bin/pandoc"),
        "assistant text goes to summary"
    );
    assert!(summary.summary.contains("/opt/homebrew/bin/pandoc"));
    assert_eq!(summary.covered_until.as_deref(), Some("evt_run_6"));

    // reopen 后记忆仍在（重启持久化）。
    let hits = reopened
        .search_text("/opt/homebrew/bin/pandoc", Some("astra"), 10)
        .await
        .unwrap();
    assert_eq!(hits.len(), 1);

    // 静态加密：落盘字节不含任何明文正文。
    drop(reopened);
    let raw = std::fs::read(&db).unwrap();
    let raw_str = String::from_utf8_lossy(&raw);
    assert!(!raw_str.contains("pandoc"), "plaintext leaked at rest");
    assert!(!raw_str.contains("帮我确认"), "plaintext leaked at rest");
}

/// 真实 Run 回放：从宿主 `agent_events.log` 提取的一次完整真实 Run
/// （641 个事件：9 次工具成功、1 次失败、8 次权限、用户取消收尾）。
/// 事件 JSON 原样反序列化成 Envelope，走与 observe spawn 相同的
/// MemoryRunCollector → engine 路径，验证真实世界数据形态下的
/// memory_search / 数据库两面。fixture：tests/fixtures/cme_replay_run.jsonl。
#[tokio::test]
async fn memory_collection_replays_real_run_event_stream() {
    const REAL_RUN: &str = include_str!("../../tests/fixtures/cme_replay_run.jsonl");

    let dir = tempfile::tempdir().unwrap();
    let db = dir.path().join("memory-cme.sqlite");
    let key = [9u8; 32];
    let engine = std::sync::Arc::new(MemoryEngine::new(CmeSqliteStore::open(&db, &key).unwrap()));
    let provider = CmeMemoryProvider::new(std::sync::Arc::clone(&engine));

    // ---- 1. 真实事件流 → Envelope → 采集器（与 spawn 逐字相同）----
    let mut collector = MemoryRunCollector::new();
    let mut event_count = 0usize;
    for line in REAL_RUN.lines().filter(|l| !l.trim().is_empty()) {
        let env: Envelope = serde_json::from_str(line).expect("real event must deserialize");
        collector.observe(&env);
        event_count += 1;
    }
    assert_eq!(event_count, 641, "fixture must replay the full real run");

    // ---- 2. Run 收尾路径（与 spawn 相同；该 Run 被用户取消，无 RunCompleted）----
    let session = "agent-mvf245e8-ab5odl"; // 真实 session id
    let bound_project: Option<String> = Some("astra".into());
    engine
        .set_thread_scope(session, bound_project.as_deref())
        .await
        .unwrap();
    // 真实用户提问（来自 agent_threads.sqlite3 同一 session）。
    let user_text = "Claude Code 是一款存在于终端中的代理编码工具 开源了?";
    let messages = collector.into_messages(user_text, 1_799_240_400_000);
    // user + 9 completed + 1 failed = 11（真实 Run 无 RunCompleted）
    assert_eq!(messages.len(), 11);
    engine.ingest(session, messages);
    let force = engine.has_priority_pending();
    assert!(force, "9 verified tool results must trip the priority gate");
    engine.process_pending(force).await.unwrap();

    // ---- 3. memory_search 面：真实工具结果可召回 ----
    let bound = ProjectBoundProvider::new(std::sync::Arc::new(provider), bound_project.clone());

    // 真实 fetch 结果（GitHub API JSON）被采集并提炼；搜索真实出现过的
    // 工具名应命中（experience，来自工具成功 cue）。
    let hits = bound.search("fetch", None, 10).await.unwrap();
    assert!(
        !hits.is_empty(),
        "verified fetch results should be recallable"
    );
    assert!(hits
        .iter()
        .all(|h| h.metadata["kind"] == serde_json::json!("experience")));
    assert!(hits
        .iter()
        .all(|h| h.scope == agent_core::memory::MemoryScope::Project));

    // 真实失败（HTTP 403）不成为记忆，但进摘要。token-overlap 检索是
    // 模糊匹配，这里直接断言不变量：没有任何已存记忆的正文携带失败
    // 前缀或失败原因。
    assert!(
        !bound
            .search("HTTP 403", None, 10)
            .await
            .unwrap()
            .iter()
            .any(|h| h.content.contains("[tool failed]") || h.content.contains("HTTP 403"),),
        "real tool failure must never become a memory"
    );

    // ---- 4. 数据库面：reopen + 摘要/游标 + 静态加密 ----
    drop(bound);
    drop(engine);
    let reopened = CmeSqliteStore::open(&db, &key).unwrap();

    let summary = reopened.load_summary(session).await.unwrap().unwrap();
    assert_eq!(summary.project_id.as_deref(), Some("astra"));
    // 摘要覆盖真实会话内容：用户提问 + 工具结果 + 失败原因。
    assert!(
        summary.summary.contains("Claude Code"),
        "user text in summary"
    );
    assert!(
        summary.summary.contains("HTTP 403"),
        "failure reason in summary"
    );
    // 游标推进到最后一条被采集的事件（工具完成 seq 628），而非流的最后
    // 一条（run_cancelled 不被采集）。
    assert_eq!(
        summary.covered_until.as_deref(),
        Some("evt_run_fe7f6199-047e-4907-b321-2d216d300fb0_628")
    );

    // reopen 后记忆仍在。
    let hits = reopened
        .search_text("fetch", Some("astra"), 10)
        .await
        .unwrap();
    assert!(!hits.is_empty());

    // 静态加密：真实正文（GitHub JSON / 用户提问）不以明文落盘。
    drop(reopened);
    let raw = std::fs::read(&db).unwrap();
    let raw_str = String::from_utf8_lossy(&raw);
    assert!(!raw_str.contains("Claude Code"), "plaintext leaked at rest");
    assert!(!raw_str.contains("node_id"), "plaintext leaked at rest");
}

// ===== 真实模型 Run：端到端验证宿主采集与跨会话召回注入 =====

struct NoQuestions;
#[async_trait::async_trait]
impl agent_core::UserQuestionBridge for NoQuestions {
    async fn ask(
        &self,
        _question: String,
        _options: Vec<String>,
        _allow_free_text: bool,
    ) -> std::result::Result<String, String> {
        Err("no questions in test".into())
    }
}

/// 从 providers.json 取一个可用的真实 provider（api_key + active_model）。
/// 路径可用 ASTRA_REAL_RUN_PROVIDERS 覆盖，默认读本机应用数据目录。
fn load_real_provider() -> (String, String, String) {
    let path = std::env::var_os("ASTRA_REAL_RUN_PROVIDERS")
        .map(std::path::PathBuf::from)
        .unwrap_or_else(|| {
            let home = std::env::var("HOME").expect("HOME not set");
            std::path::PathBuf::from(home)
                .join("Library/Application Support/com.astra.app/providers.json")
        });
    let raw = std::fs::read_to_string(&path)
        .unwrap_or_else(|e| panic!("real-run test needs providers config: {path:?}: {e}"));
    let providers: serde_json::Value = serde_json::from_str(&raw).unwrap();
    for p in providers.as_array().expect("providers must be an array") {
        let base_url = p["base_url"].as_str().unwrap_or("").trim().to_string();
        let api_key = p["api_key"].as_str().unwrap_or("").trim().to_string();
        let model = p["active_model"].as_str().unwrap_or("").trim().to_string();
        if !base_url.is_empty() && !api_key.is_empty() && !model.is_empty() {
            return (base_url, api_key, model);
        }
    }
    panic!("no provider with base_url + api_key + active_model in {path:?}");
}

/// 一次真实 Run：与 observe spawn 同形状组装 Thread（真实 backend /
/// registry / policy / ProjectBoundProvider 记忆），跑一轮并采集事件。
/// 返回（助手最终回答，采集器，全部事件）。权限桥全放行（等价 all 级放权）。
/// `max_tokens` 透传给模型（None = 网关默认；测试用它压低输出上限以
/// 确定性触发 finish_reason=length 截断）。
#[allow(clippy::too_many_arguments)]
async fn real_turn(
    threads: &mut crate::agent::threads::AgentThreads,
    session: &str,
    project_root: &Path,
    memory: Arc<dyn agent_core::memory::MemoryProvider>,
    base_url: &str,
    api_key: &str,
    model_name: &str,
    user_text: &str,
    max_tokens: Option<u32>,
) -> (String, MemoryRunCollector, Vec<agent_core::RunEvent>) {
    // 与 agent_start_run 相同的策略形状：本测试只需 read_file（默认
    // Allow）+ shell_execute（Confirm，经 AutoBridge 全放行）。
    let mut policy = ToolPolicy::default();
    policy.allowed_readonly.push("find_path".into());
    policy.confirm_tools.push("shell_execute".into());

    let mut config = ThreadConfig::new(
        session.to_string(),
        crate::agent::prompts::agent_system_prompt(),
    );
    config.project_root = Some(project_root.to_path_buf());
    config.max_turns = 6;
    config.max_tokens = max_tokens;
    config.auto_compact_tokens = Some(DEFAULT_AUTO_COMPACT_TOKENS).filter(|&t| t > 0);
    config.policy = policy;
    config.memory = Some(memory);
    config.texts = Arc::new(agent_core::TextPack::zh());

    let backend: Arc<dyn ModelBackend> = Arc::new(RigBackend::new(
        base_url.to_string(),
        api_key.to_string(),
        model_name.to_string(),
    ));
    let root = project_root.to_path_buf();
    let registry = build_registry(
        session,
        Some(&root),
        None,
        Arc::new(NoQuestions),
        agent_core::net_tools::SearchEngine::default(),
        Arc::new(agent_core::shell_background::ProcessManager::new()),
        Arc::new(agent_core::TaskManager::new(8, None)),
        None,
        None,
    );
    threads
        .load_or_create(
            config,
            backend,
            registry,
            Arc::new(agent_core::AutoBridge(Decision::Allow)),
        )
        .await
        .unwrap();

    let (tx, mut rx) = tokio::sync::mpsc::channel::<Envelope>(1024);
    let content = vec![MessageContent::text(user_text)];
    let handle = threads.threads[session]
        .try_send_content(content, tx)
        .unwrap();
    let mut collector = MemoryRunCollector::new();
    let mut final_text = String::new();
    let mut events: Vec<agent_core::RunEvent> = Vec::new();
    let wait = async {
        while let Some(env) = rx.recv().await {
            if let agent_core::RunEvent::RunCompleted { final_text: text } = &env.event {
                final_text = text.clone();
            }
            events.push(env.event.clone());
            collector.observe(&env);
        }
    };
    if tokio::time::timeout(std::time::Duration::from_secs(300), wait)
        .await
        .is_err()
    {
        handle.cancel();
        panic!("real run timed out after 300s");
    }
    (final_text, collector, events)
}

/// 真实模型 Run 验证宿主采集路径（唯一依赖外部模型 API 的测试，默认
/// 忽略）：`cargo test --lib memory_collection_real_model_run -- --ignored --nocapture`
///
/// 链路：真实模型 + 真实工具执行（read_file + shell_execute）→
/// 与 observe spawn 逐字相同的 MemoryRunCollector + commit_collected_memory
/// → 真实 CME 加密库。验证两面 + 跨会话召回注入：
/// 1) memory_search 面：已验证 shell 输出成为可召回记忆（experience / Project）；
/// 2) 数据库面：摘要含工具输出、游标推进、静态加密；
/// 3) 注入面：全新会话（无对话历史）问探针输出，只有记忆召回注入
///    一条路能提供 token —— 模型答对即证明工具经验真实进入上下文。
#[tokio::test]
#[ignore = "real model API + network; run with --ignored --nocapture"]
async fn memory_collection_real_model_run() {
    let (base_url, api_key, model_name) = load_real_provider();
    eprintln!("[real-run] model: {model_name}");

    // 临时项目：README 供 read_file；探针 token 不可推算，只能来自
    // 工具结果 → 记忆 → 跨会话召回注入。
    const PROBE: &str = "ASTRA_MEMORY_PROBE_X7K9";
    let project = tempfile::tempdir().unwrap();
    std::fs::write(
        project.path().join("README.md"),
        "# memory probe\n\nThis project uses pnpm as its package manager.\n",
    )
    .unwrap();

    let mem_dir = tempfile::tempdir().unwrap();
    let db = mem_dir.path().join("memory-cme.sqlite");
    let key = [11u8; 32];
    let engine = Arc::new(MemoryEngine::new(CmeSqliteStore::open(&db, &key).unwrap()));
    let bound_project: Option<String> = Some("astra-real-run-probe".into());

    let mut threads = crate::agent::threads::AgentThreads::default();
    let threads_dir = tempfile::tempdir().unwrap();
    threads
        .open(threads_dir.path(), Arc::new(Mutex::new(HashMap::new())))
        .await
        .unwrap();

    // ---- Run 1：真实模型 + 真实工具，采集 → 提炼 ----
    let prompt1 = "请先读取项目根目录的 README.md，告诉我这个项目用的包管理器；然后执行 shell 命令 `echo ASTRA_MEMORY_PROBE_X7K9`，把命令的完整原始输出告诉我。";
    let memory1 = Arc::new(ProjectBoundProvider::new(
        Arc::new(CmeMemoryProvider::new(Arc::clone(&engine))),
        bound_project.clone(),
    ));
    let (final1, collector1, _events1) = real_turn(
        &mut threads,
        "sess-real-run-1",
        project.path(),
        memory1,
        &base_url,
        &api_key,
        &model_name,
        prompt1,
        None,
    )
    .await;
    eprintln!("[real-run] run1 final: {final1}");
    assert!(
        !final1.trim().is_empty(),
        "run 1 must complete with a final answer"
    );

    // Run 收尾（与 observe spawn 相同的共用提交路径）。
    let force = commit_collected_memory(
        &engine,
        collector1,
        "sess-real-run-1",
        bound_project.as_deref(),
        prompt1,
        1_800_000_000_000,
        None,
    )
    .await;
    assert!(force, "verified shell result must trip the priority gate");

    // ---- 1. memory_search 面：已验证 shell 输出成为可召回记忆 ----
    let bound = ProjectBoundProvider::new(
        Arc::new(CmeMemoryProvider::new(Arc::clone(&engine))),
        bound_project.clone(),
    );
    let hits = bound.search(PROBE, None, 10).await.unwrap();
    assert!(
        hits.iter().any(|h| h.content.contains(PROBE)),
        "verified shell output must become a recallable memory; got: {:?}",
        hits.iter().map(|h| h.content.clone()).collect::<Vec<_>>()
    );
    assert!(hits
        .iter()
        .filter(|h| h.content.contains(PROBE))
        .all(|h| h.metadata["kind"] == serde_json::json!("experience")));
    assert!(hits
        .iter()
        .filter(|h| h.content.contains(PROBE))
        .all(|h| h.scope == agent_core::memory::MemoryScope::Project));

    // ---- 1b. 作用域隔离与显式记住（确定性断言，无需额外模型调用）----
    // 显式记住（host memory_remember 同路径）：立即持久化可召回，
    // 不经过 process_pending 阈值；project_id=None = 全局（User 级）。
    let preference = "用户偏好：回答保持简洁";
    engine
        .remember(RememberRequest {
            content: preference.into(),
            kind: MemoryKind::Preference,
            project_id: None,
            thread_id: None,
        })
        .await
        .unwrap();
    let hits = bound.search("保持简洁", None, 10).await.unwrap();
    assert!(
        hits.iter().any(|h| h.content.contains("保持简洁")),
        "explicit remember must be immediately recallable without threshold"
    );
    assert!(hits
        .iter()
        .any(|h| h.scope == agent_core::memory::MemoryScope::User));

    // 反向隔离：项目 B 绑定召回不到项目 A 的探针记忆，但全局偏好
    // （User 级）跨项目共享。
    let bound_b = ProjectBoundProvider::new(
        Arc::new(CmeMemoryProvider::new(Arc::clone(&engine))),
        Some("astra-other-project".into()),
    );
    let hits_b = bound_b.search(PROBE, None, 10).await.unwrap();
    assert!(
        hits_b.iter().all(|h| !h.content.contains(PROBE)),
        "project A probe must never leak into project B recall"
    );
    let hits_b = bound_b.search("保持简洁", None, 10).await.unwrap();
    assert!(
        hits_b.iter().any(|h| h.content.contains("保持简洁")),
        "user-level memory must be shared across projects"
    );

    // 对称隔离：写入项目 B 的记忆对项目 A 不可见。
    const BUN: &str = "BUN_PROBE_Y4M8";
    engine
        .remember(RememberRequest {
            content: format!("astra-other-project 的构建工具是 {BUN}"),
            kind: MemoryKind::Fact,
            project_id: Some("astra-other-project".into()),
            thread_id: None,
        })
        .await
        .unwrap();
    let hits_b = bound_b.search(BUN, None, 10).await.unwrap();
    assert!(hits_b.iter().any(|h| h.content.contains(BUN)));
    let hits_a = bound.search(BUN, None, 10).await.unwrap();
    assert!(
        hits_a.iter().all(|h| !h.content.contains(BUN)),
        "project B memory must never leak into project A recall"
    );

    // ---- 2. 数据库面：摘要含工具输出、游标推进、静态加密 ----
    let reopened = CmeSqliteStore::open(&db, &key).unwrap();
    let summary = reopened
        .load_summary("sess-real-run-1")
        .await
        .unwrap()
        .expect("run 1 summary must exist");
    assert_eq!(summary.project_id.as_deref(), Some("astra-real-run-probe"));
    assert!(
        summary.summary.contains(PROBE),
        "tool output must reach the session summary"
    );
    assert!(
        summary.covered_until.is_some(),
        "cursor must advance past collected events"
    );
    // SQL 级作用域过滤证明：隔离不是元数据标记，而是 store 的 WHERE
    // 子句（Some(p) → 全局+p；None → 仅全局）。
    let hits = reopened
        .search_text(PROBE, Some("astra-real-run-probe"), 10)
        .await
        .unwrap();
    assert!(hits.iter().any(|h| h.content.contains(PROBE)));
    let hits = reopened
        .search_text(PROBE, Some("astra-other-project"), 10)
        .await
        .unwrap();
    assert!(
        hits.is_empty(),
        "wrong-project SQL filter must exclude the probe memory"
    );
    let hits = reopened.search_text(PROBE, None, 10).await.unwrap();
    assert!(
        hits.is_empty(),
        "project memory must never be visible as global"
    );
    drop(reopened);
    let raw = std::fs::read(&db).unwrap();
    assert!(
        !String::from_utf8_lossy(&raw).contains(PROBE),
        "plaintext leaked at rest"
    );

    // ---- 3. 注入面：全新会话（无对话历史），只有记忆召回注入一条路
    // 能提供探针 token —— 模型答对即证明工具经验真实进入上下文。 ----
    // 注意问题措辞：必须含检索命中词（shell_execute，与记忆正文
    // 「shell_execute: …」重叠）但绝不能含 token 本身。
    let prompt2 = "不要执行任何命令，不要读任何文件。我们之前用 shell_execute 执行过一个 echo 探针命令，请根据你记忆中的信息回答：它输出的完整 token 是什么？只回答 token 本身。";
    let memory2 = Arc::new(ProjectBoundProvider::new(
        Arc::new(CmeMemoryProvider::new(Arc::clone(&engine))),
        bound_project.clone(),
    ));
    let (final2, collector2, _events2) = real_turn(
        &mut threads,
        "sess-real-run-2",
        project.path(),
        memory2,
        &base_url,
        &api_key,
        &model_name,
        prompt2,
        None,
    )
    .await;
    eprintln!("[real-run] run2 final: {final2}");
    assert!(
        final2.contains("X7K9"),
        "cross-session recall must inject the verified tool memory into a fresh session; final: {final2}"
    );
    // Run 2 的采集提交也走一遍共用路径（不应 panic / 破坏状态）。
    commit_collected_memory(
        &engine,
        collector2,
        "sess-real-run-2",
        bound_project.as_deref(),
        prompt2,
        1_800_000_100_000,
        None,
    )
    .await;
}

/// 真实模型大文件截断续跑验证（默认忽略）：
/// `cargo test --lib truncation_real_model_large_file -- --ignored --nocapture`
///
/// 复现用户报告的场景：大文件 + 需要长输出的任务 → 模型响应被单次输出
/// 上限截断（finish_reason=length）。修复前：截断发生在 tool-call 参数
/// JSON 流到一半时，调用被静默丢弃 → Run 直接 RunCompleted（「任务完成」
/// 但什么都没做）。修复后：Delta::Truncated → 保留已完整的调用继续执行 +
/// 自动续跑（≤3 次）。
///
/// 确定性触发：max_tokens 压到 300，任何「把全部条目写入文件」的任务都
/// 必然超出单次输出上限，不依赖网关的默认 max_tokens。
#[tokio::test]
#[ignore = "real model API + network; run with --ignored --nocapture"]
async fn truncation_real_model_large_file() {
    let (base_url, api_key, model_name) = load_real_provider();
    eprintln!("[trunc-run] model: {model_name}");

    // 临时项目：大文件（400 条编号记录，~24KB）+ 全放行权限
    let project = tempfile::tempdir().unwrap();
    let mut data = String::new();
    for i in 1..=400 {
        data.push_str(&format!(
            "ITEM {:04} | token-{:04}-XK{:02} | status=active | weight={}\n",
            i,
            i,
            i % 100,
            (i * 7) % 997
        ));
    }
    std::fs::write(project.path().join("data.txt"), &data).unwrap();

    let mem_dir = tempfile::tempdir().unwrap();
    let db = mem_dir.path().join("memory-cme.sqlite");
    let key = [12u8; 32];
    let engine = Arc::new(MemoryEngine::new(CmeSqliteStore::open(&db, &key).unwrap()));
    let memory = Arc::new(ProjectBoundProvider::new(
        Arc::new(CmeMemoryProvider::new(Arc::clone(&engine))),
        Some("astra-trunc-run".to_string()),
    ));

    let mut threads = crate::agent::threads::AgentThreads::default();
    let threads_dir = tempfile::tempdir().unwrap();
    threads
        .open(
            threads_dir.path(),
            Arc::new(std::sync::Mutex::new(HashMap::new())),
        )
        .await
        .unwrap();
    let prompt = "请读取项目根目录的 data.txt，把其中全部 400 条记录按原样逐行写入项目根目录的 output.md（一行一条，不许省略、不许截断）。写完文件后回复「已写入 N 条」。";
    let (final_text, _collector, events) = real_turn(
        &mut threads,
        "sess-trunc-run",
        project.path(),
        memory,
        &base_url,
        &api_key,
        &model_name,
        prompt,
        Some(300), // 确定性触发 finish_reason=length
    )
    .await;

    eprintln!("[trunc-run] final: {final_text:?}");
    for ev in &events {
        let brief = match ev {
            agent_core::RunEvent::StatusChanged { status } => format!("status={status:?}"),
            agent_core::RunEvent::ToolCallStarted { tool_name, .. } => {
                format!("tool_start={tool_name}")
            }
            agent_core::RunEvent::ToolCallCompleted {
                tool_name,
                result_summary,
                ..
            } => format!("tool_end={tool_name} summary={:.80}", result_summary),
            agent_core::RunEvent::ToolCallFailed {
                tool_name, error, ..
            } => format!("tool_fail={tool_name} err={error}"),
            agent_core::RunEvent::MessageDelta { .. } => "delta".to_string(),
            agent_core::RunEvent::UsageUpdated {
                input_tokens,
                output_tokens,
                ..
            } => format!("usage in={input_tokens:?} out={output_tokens:?}"),
            agent_core::RunEvent::RunCompleted { final_text } => {
                format!("COMPLETED final={:.120}", final_text)
            }
            agent_core::RunEvent::RunFailed { error } => format!("FAILED error={error}"),
            other => format!("{other:?}"),
        };
        eprintln!("[trunc-run] event: {brief}");
    }

    // ===== 断言 1：Run 必须完成且有实质回复（不能静默空答） =====
    assert!(
        !final_text.trim().is_empty(),
        "run must end with a non-empty final answer (silent empty completion was the bug)"
    );

    // ===== 断言 2：截断后续跑必须实际执行了工具（写文件发生） =====
    // 修复前：截断把未完成的 tool-call 静默丢弃 → 没有任何工具执行就「完成」。
    let output_path = project.path().join("output.md");
    let output = std::fs::read_to_string(&output_path).unwrap_or_default();
    eprintln!("[trunc-run] output.md bytes: {}", output.len());
    assert!(
        !output.trim().is_empty(),
        "write_file must have executed at least once (dropped tool calls were the bug)"
    );
    let written_lines = output.lines().filter(|l| l.contains("ITEM")).count();
    eprintln!("[trunc-run] output.md ITEM lines: {written_lines}/400");
    eprintln!("[trunc-run] OK: no silent completion; tool work was done");
}
