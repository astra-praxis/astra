//! 事件模型：单一类型化事件流，描述"实际发生的事情"。
//!
//! 修订记录（评审后定稿）：
//! - 去掉 reasoning_delta（模型私有推理不作正式事件，仅可选 status 提示）
//! - 事件带 session_id / run_id / 递增 sequence，防错序、支持恢复
//! - 工具状态拆 started/completed/failed，产物结构化
//! - 1.2：Envelope 增加 schema_version；全部变体 JSON 形状由快照测试锁定
//!   （`events_testdata/`，破坏性变更：版本 +1 → 重写快照 → 同步 docs/EVENTS.md）
//!   事件契约文档：`docs/EVENTS.md`
//! - 1.3：ToolCallCompleted 直接携带结构化 Artifact（id/kind/location/
//!   metadata/preview），ArtifactRef 移除 → schema_version 2
//! - 2 修订（additive，不 bump）：Envelope 增加 event_id（派生自
//!   run_id+sequence，重放稳定）与 timestamp（毫秒）；未采纳项见 PLAN 决策记录

use serde::{Deserialize, Serialize};

/// 事件协议版本（Envelope.schema_version）。
/// 破坏性 JSON 形状变更必须 +1，并重写快照（`write_event_snapshot`）+ 更新 docs/EVENTS.md。
pub const EVENT_SCHEMA_VERSION: u32 = 2;

fn default_schema_version() -> u32 {
    EVENT_SCHEMA_VERSION
}

/// 全局递增序号（每个 Run 内从 1 开始）
pub type Sequence = u64;

/// Run 级事件（跨轮）。
#[derive(Debug, Clone, Serialize, Deserialize)]
#[serde(tag = "type", rename_all = "snake_case")]
pub enum RunEvent {
    /// 正在生成的回复文本（增量）
    MessageDelta { text: String },

    /// 模型思考过程增量（reasoning 模型的 reasoning_content）。
    /// 前端渲染为可折叠的「思考过程」块（参考 Zed 的 thinking 块）。
    ThinkingDelta { text: String },

    /// 工具调用参数流式增量（写大文件时前端实时可见，参考 Zed 的流式 diff）。
    /// fragment 是原始 JSON 参数片段；同一 tool_call_id 的片段拼接后即完整参数 JSON。
    ToolCallArgsDelta {
        tool_call_id: String,
        tool_name: String,
        fragment: String,
    },

    /// 运行状态变化（ analyzing / executing_tool / awaiting_permission / ... ）
    StatusChanged { status: RunStatus },

    /// 任务计划更新（Agent 运行时维护的结构化状态，不是模型生成的 markdown）
    PlanUpdated { plan: Plan },

    /// 工具调用开始（参数已校验；Confirm 时发出后等待用户）
    /// args 为完整参数（超长字符串截断），供前端渲染写文件内容/完整命令预览
    ToolCallStarted {
        tool_call_id: String,
        tool_name: String,
        args_summary: String,
        #[serde(default, skip_serializing_if = "Option::is_none")]
        args: Option<serde_json::Value>,
    },

    /// 工具执行成功（含结构化产物）
    ToolCallCompleted {
        tool_call_id: String,
        tool_name: String,
        result_summary: String,
        #[serde(default, skip_serializing_if = "Vec::is_empty")]
        artifacts: Vec<crate::tool::Artifact>,
    },

    /// 工具执行失败（错误字符串同时回传模型）
    ToolCallFailed {
        tool_call_id: String,
        tool_name: String,
        error: String,
    },

    /// 等待用户确认（前端展示确认卡；决定经 channel 回传）
    PermissionRequested {
        tool_call_id: String,
        tool_name: String,
        summary: String,
    },

    /// 权限已决定（Allow/Deny，用于前端更新确认卡状态）
    PermissionDecided {
        tool_call_id: String,
        decision: crate::permissions::Decision,
    },

    /// 当前任务完成
    RunCompleted { final_text: String },

    /// 上下文自动压缩完成（历史已插入摘要消息，后续请求自动裁剪）
    CompactionCompleted,

    /// 当前任务失败
    RunFailed { error: String },

    /// 当前任务已取消
    RunCancelled,
}

impl RunEvent {
    pub fn with_meta(self, session_id: &str, run_id: &str, sequence: Sequence) -> Envelope {
        self.with_meta_at(session_id, run_id, sequence, now_ms())
    }

    /// Same as `with_meta` but with an explicit timestamp — used by tests
    /// (deterministic snapshots) and hosts that carry their own clock.
    pub fn with_meta_at(
        self,
        session_id: &str,
        run_id: &str,
        sequence: Sequence,
        timestamp_ms: u64,
    ) -> Envelope {
        // Derived composite id: stable under replay (same event → same id,
        // unlike a counter), unique across runs by construction.
        let event_id = format!("evt_{run_id}_{sequence}");
        Envelope {
            schema_version: EVENT_SCHEMA_VERSION,
            event_id,
            timestamp: timestamp_ms,
            session_id: session_id.to_string(),
            run_id: run_id.to_string(),
            sequence,
            event: self,
        }
    }
}

/// Wall-clock milliseconds since UNIX epoch (0 if clock is before epoch).
fn now_ms() -> u64 {
    std::time::SystemTime::now()
        .duration_since(std::time::UNIX_EPOCH)
        .map(|d| d.as_millis() as u64)
        .unwrap_or(0)
}

/// 带元数据的事件信封（Tauri event payload）
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct Envelope {
    /// 事件协议版本；缺省按当前版本处理（兼容旧载荷反序列化）
    #[serde(default = "default_schema_version")]
    pub schema_version: u32,
    /// 全局事件 ID（`evt_{run_id}_{sequence}`）：重放稳定、跨 Run 唯一，
    /// 供 WebSocket 重连去重 / 日志追踪 / replay 定位
    #[serde(default)]
    pub event_id: String,
    /// Runtime 侧产生时间（ms since epoch）；旧载荷缺省 0。性能分析、
    /// Agent Trace 用，避免依赖消费方收到事件的时钟
    #[serde(default)]
    pub timestamp: u64,
    pub session_id: String,
    pub run_id: String,
    pub sequence: Sequence,
    // NOTE: agent_id field slot reserved — add when multi-agent scheduling
    // (Phase 5) actually introduces agent identity, not before.
    #[serde(flatten)]
    pub event: RunEvent,
}

/// 运行状态（status_changed 的载荷）
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum RunStatus {
    /// 正在分析 / 模型思考中
    Analyzing,
    /// 正在执行工具
    ExecutingTool,
    /// 等待用户确认
    AwaitingPermission,
    /// 正在压缩上下文（历史摘要生成中）
    Compacting,
    /// 收尾中
    Finalizing,
}

/// 任务计划：轻量结构化状态机（不是工作流编辑器）。
/// 由 Run 循环维护：模型每轮输出后可更新计划（通过专用轻量机制或
/// 工具调用前后钩子）；步骤状态由后端真实事件驱动。
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct Plan {
    pub steps: Vec<PlanStep>,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct PlanStep {
    pub id: String,
    pub title: String,
    pub state: PlanStepState,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum PlanStepState {
    Pending,
    InProgress,
    Completed,
    Failed,
}

impl Plan {
    pub fn empty() -> Self {
        Self { steps: vec![] }
    }

    /// 插入或更新（幂等：同一 id 重复 upsert 只改标题与状态）
    pub fn upsert(
        &mut self,
        id: impl Into<String>,
        title: impl Into<String>,
        state: PlanStepState,
    ) {
        let id = id.into();
        if let Some(step) = self.steps.iter_mut().find(|s| s.id == id) {
            step.title = title.into();
            step.state = state;
        } else {
            self.steps.push(PlanStep {
                id,
                title: title.into(),
                state,
            });
        }
    }

    /// 按步骤 id 查询状态（供 Run 循环驱动步骤流转）
    pub fn state_of(&self, id: &str) -> Option<PlanStepState> {
        self.steps.iter().find(|s| s.id == id).map(|s| s.state)
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn plan_upsert_inserts_then_updates_in_place() {
        let mut plan = Plan::empty();
        plan.upsert("step1", "读取文件", PlanStepState::InProgress);
        assert_eq!(plan.steps.len(), 1);
        assert_eq!(plan.state_of("step1"), Some(PlanStepState::InProgress));

        // 同 id 再 upsert：更新而不是新增
        plan.upsert("step1", "读取文件", PlanStepState::Completed);
        assert_eq!(plan.steps.len(), 1);
        assert_eq!(plan.state_of("step1"), Some(PlanStepState::Completed));
        // 标题也被更新
        assert_eq!(plan.steps[0].title, "读取文件");
    }

    #[test]
    fn plan_upsert_appends_new_steps_in_order() {
        let mut plan = Plan::empty();
        plan.upsert("a", "一", PlanStepState::Pending);
        plan.upsert("b", "二", PlanStepState::InProgress);
        plan.upsert("c", "三", PlanStepState::Pending);
        let ids: Vec<&str> = plan.steps.iter().map(|s| s.id.as_str()).collect();
        assert_eq!(ids, vec!["a", "b", "c"]);
    }

    #[test]
    fn plan_state_of_unknown_id_is_none() {
        let mut plan = Plan::empty();
        plan.upsert("a", "一", PlanStepState::Pending);
        assert_eq!(plan.state_of("missing"), None);
        assert_eq!(Plan::empty().state_of("a"), None);
    }

    #[test]
    fn envelope_carries_meta_and_event_tag() {
        let env = RunEvent::MessageDelta {
            text: "你好".into(),
        }
        .with_meta_at("sess-1", "run-1", 3, 1_700_000_000_000);
        assert_eq!(env.schema_version, EVENT_SCHEMA_VERSION);
        assert_eq!(env.session_id, "sess-1");
        assert_eq!(env.run_id, "run-1");
        assert_eq!(env.sequence, 3);
        assert_eq!(env.timestamp, 1_700_000_000_000);
        assert_eq!(env.event_id, "evt_run-1_3");
    }

    #[test]
    fn envelope_event_id_is_replay_stable() {
        // Same (run_id, sequence) must yield the same event_id — dedup on
        // reconnect/replay depends on it.
        let a = RunEvent::RunCancelled.with_meta_at("s", "run_x", 7, 1);
        let b = RunEvent::RunFailed { error: "e".into() }.with_meta_at("s", "run_x", 7, 2);
        assert_eq!(a.event_id, b.event_id);
        assert_ne!(
            a.event_id,
            RunEvent::RunCancelled
                .with_meta_at("s", "run_x", 8, 1)
                .event_id
        );
    }

    #[test]
    fn envelope_tolerates_legacy_payload_without_v2_fields() {
        // v1 payloads have no event_id/timestamp — must deserialize with
        // defaults (contract for old hosts; see docs/TESTS.md §2).
        let legacy = r#"{"schema_version":1,"session_id":"s","run_id":"r","sequence":2,"type":"run_cancelled"}"#;
        let env: Envelope = serde_json::from_str(legacy).unwrap();
        assert_eq!(env.schema_version, 1);
        assert_eq!(env.sequence, 2);
        assert_eq!(env.event_id, "");
        assert_eq!(env.timestamp, 0);
        assert!(matches!(env.event, RunEvent::RunCancelled));
    }

    #[test]
    fn envelope_ignores_unknown_fields() {
        // Forward compatibility: extra fields from future versions are
        // silently ignored (locked as a contract here).
        let future = r#"{
            "schema_version": 2,
            "event_id": "evt_r_1",
            "timestamp": 42,
            "session_id": "s",
            "run_id": "r",
            "sequence": 1,
            "type": "message_delta",
            "text": "hi",
            "future_field": {"nested": true}
        }"#;
        let env: Envelope = serde_json::from_str(future).unwrap();
        assert_eq!(env.sequence, 1);
        assert!(matches!(env.event, RunEvent::MessageDelta { .. }));
    }

    #[test]
    fn envelope_serializes_with_snake_case_type_tag() {
        // 前端依赖这个 JSON 形状：type 字段 + flatten 的事件载荷 + schema_version
        let env = RunEvent::MessageDelta { text: "hi".into() }.with_meta_at("s", "r", 1, 42);
        let v = serde_json::to_value(&env).unwrap();
        assert_eq!(v["type"], "message_delta");
        assert_eq!(v["schema_version"], EVENT_SCHEMA_VERSION);
        assert_eq!(v["event_id"], "evt_r_1");
        assert_eq!(v["timestamp"], 42);
        assert_eq!(v["text"], "hi");
        assert_eq!(v["session_id"], "s");
        assert_eq!(v["run_id"], "r");
        assert_eq!(v["sequence"], 1);
    }

    // ===== 快照契约（PLAN 1.2）：全部变体 JSON 形状锁定 =====

    /// 穷尽映射：新增 RunEvent 变体时此处编译报错，
    /// 提醒同步更新 all_events()、快照文件与 docs/EVENTS.md。
    fn tag_of(ev: &RunEvent) -> &'static str {
        match ev {
            RunEvent::MessageDelta { .. } => "message_delta",
            RunEvent::ThinkingDelta { .. } => "thinking_delta",
            RunEvent::ToolCallArgsDelta { .. } => "tool_call_args_delta",
            RunEvent::StatusChanged { .. } => "status_changed",
            RunEvent::PlanUpdated { .. } => "plan_updated",
            RunEvent::ToolCallStarted { .. } => "tool_call_started",
            RunEvent::ToolCallCompleted { .. } => "tool_call_completed",
            RunEvent::ToolCallFailed { .. } => "tool_call_failed",
            RunEvent::PermissionRequested { .. } => "permission_requested",
            RunEvent::PermissionDecided { .. } => "permission_decided",
            RunEvent::RunCompleted { .. } => "run_completed",
            RunEvent::CompactionCompleted => "compaction_completed",
            RunEvent::RunFailed { .. } => "run_failed",
            RunEvent::RunCancelled => "run_cancelled",
        }
    }

    const ALL_EVENT_TAGS: &[&str] = &[
        "message_delta",
        "thinking_delta",
        "tool_call_args_delta",
        "status_changed",
        "plan_updated",
        "tool_call_started",
        "tool_call_completed",
        "tool_call_failed",
        "permission_requested",
        "permission_decided",
        "run_completed",
        "compaction_completed",
        "run_failed",
        "run_cancelled",
    ];

    /// 快照样本：每个变体构造一次（含代表性载荷）。
    /// 新增变体必须在此同步加入，否则 all_events_covers_every_variant 失败。
    fn all_events() -> Vec<RunEvent> {
        vec![
            RunEvent::MessageDelta {
                text: "增量文本".into(),
            },
            RunEvent::ThinkingDelta {
                text: "思考增量".into(),
            },
            RunEvent::ToolCallArgsDelta {
                tool_call_id: "tc1".into(),
                tool_name: "write_file".into(),
                fragment: "{\"path\":".into(),
            },
            RunEvent::StatusChanged {
                status: RunStatus::Analyzing,
            },
            RunEvent::PlanUpdated {
                plan: Plan {
                    steps: vec![PlanStep {
                        id: "s1".into(),
                        title: "示例步骤".into(),
                        state: PlanStepState::InProgress,
                    }],
                },
            },
            RunEvent::ToolCallStarted {
                tool_call_id: "tc1".into(),
                tool_name: "write_file".into(),
                args_summary: "path=a.txt".into(),
                args: Some(serde_json::json!({"path": "a.txt"})),
            },
            RunEvent::ToolCallCompleted {
                tool_call_id: "tc1".into(),
                tool_name: "write_file".into(),
                result_summary: "已写入 a.txt".into(),
                artifacts: vec![crate::tool::Artifact {
                    id: "art_fix_1".into(),
                    kind: crate::tool::ArtifactKind::FileChange,
                    location: "a.txt".into(),
                    metadata: serde_json::json!({"mode": "create", "bytes": 128}),
                    preview: None,
                }],
            },
            RunEvent::ToolCallFailed {
                tool_call_id: "tc1".into(),
                tool_name: "read_file".into(),
                error: "文件不存在".into(),
            },
            RunEvent::PermissionRequested {
                tool_call_id: "tc1".into(),
                tool_name: "write_file".into(),
                summary: "写入 a.txt".into(),
            },
            RunEvent::PermissionDecided {
                tool_call_id: "tc1".into(),
                decision: crate::permissions::Decision::Allow,
            },
            RunEvent::RunCompleted {
                final_text: "任务完成".into(),
            },
            RunEvent::CompactionCompleted,
            RunEvent::RunFailed {
                error: "网关超时".into(),
            },
            RunEvent::RunCancelled,
        ]
    }

    fn render_snapshot() -> String {
        let envelopes: Vec<Envelope> = all_events()
            .into_iter()
            .enumerate()
            .map(|(i, ev)| ev.with_meta_at("sess-1", "run-1", (i + 1) as u64, 1_700_000_000_000))
            .collect();
        let mut json = serde_json::to_string_pretty(&envelopes).unwrap();
        json.push('\n');
        json
    }

    #[test]
    fn all_events_covers_every_variant() {
        let tags: std::collections::HashSet<&str> = all_events().iter().map(tag_of).collect();
        assert_eq!(
            tags.len(),
            ALL_EVENT_TAGS.len(),
            "all_events() 未覆盖全部变体，新增变体必须加入快照构造"
        );
        for t in ALL_EVENT_TAGS {
            assert!(tags.contains(t), "快照构造缺少事件 {t}");
        }
    }

    #[test]
    fn event_envelope_json_matches_schema_v2_snapshot() {
        for ev in all_events() {
            let _ = tag_of(&ev); // 编译期穷尽保证
        }
        let actual = render_snapshot();
        let expected = include_str!("events_testdata/events_schema_v2.snapshot.json");
        assert_eq!(
            actual, expected,
            "事件 JSON 形状发生变化——这是破坏性契约变更：\
             schema_version +1 → cargo test -p agent-core write_event_snapshot -- --ignored \
             重写快照 → 同步 docs/EVENTS.md 与前端消费方"
        );
    }

    /// 破坏性变更后的快照再生成：
    /// `cargo test -p agent-core write_event_snapshot -- --ignored`
    #[test]
    #[ignore]
    fn write_event_snapshot() {
        let dir = std::path::Path::new(env!("CARGO_MANIFEST_DIR"))
            .join("src")
            .join("events_testdata");
        std::fs::create_dir_all(&dir).unwrap();
        let path = dir.join("events_schema_v2.snapshot.json");
        std::fs::write(&path, render_snapshot()).unwrap();
        println!("已写入 {}", path.display());
    }

    #[test]
    fn run_status_serializes_snake_case() {
        assert_eq!(
            serde_json::to_value(RunStatus::Analyzing).unwrap(),
            "analyzing"
        );
        assert_eq!(
            serde_json::to_value(RunStatus::AwaitingPermission).unwrap(),
            "awaiting_permission"
        );
        assert_eq!(
            serde_json::to_value(RunStatus::ExecutingTool).unwrap(),
            "executing_tool"
        );
        assert_eq!(
            serde_json::to_value(RunStatus::Finalizing).unwrap(),
            "finalizing"
        );
        assert_eq!(
            serde_json::to_value(RunStatus::Compacting).unwrap(),
            "compacting"
        );
    }

    #[test]
    fn empty_artifacts_are_skipped_in_completed_event() {
        let with = serde_json::to_value(&RunEvent::ToolCallCompleted {
            tool_call_id: "t".into(),
            tool_name: "w".into(),
            result_summary: "".into(),
            artifacts: vec![crate::tool::Artifact {
                id: "art_fix_2".into(),
                kind: crate::tool::ArtifactKind::FileChange,
                location: "a.txt".into(),
                metadata: serde_json::Value::Null,
                preview: None,
            }],
        })
        .unwrap();
        assert!(with["artifacts"].is_array());
        assert_eq!(with["artifacts"][0]["location"], "a.txt");
        assert_eq!(with["artifacts"][0]["kind"], "file_change");

        let without = serde_json::to_value(&RunEvent::ToolCallCompleted {
            tool_call_id: "t".into(),
            tool_name: "w".into(),
            result_summary: "".into(),
            artifacts: vec![],
        })
        .unwrap();
        assert!(without.get("artifacts").is_none(), "空产物应省略字段");
    }

    #[test]
    fn plan_step_state_serializes_snake_case() {
        assert_eq!(
            serde_json::to_value(PlanStepState::InProgress).unwrap(),
            "in_progress"
        );
    }
}
