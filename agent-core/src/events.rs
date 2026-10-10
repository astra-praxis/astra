//! 事件模型：单一类型化事件流，描述"实际发生的事情"。
//!
//! 修订记录（评审后定稿）：
//! - 去掉 reasoning_delta（模型私有推理不作正式事件，仅可选 status 提示）
//! - 事件带 session_id / run_id / 递增 sequence，防错序、支持恢复
//! - 工具状态拆 started/completed/failed，产物结构化

use serde::{Deserialize, Serialize};

/// 全局递增序号（每个 Run 内从 1 开始）
pub type Sequence = u64;

/// 工具执行产物引用
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct ArtifactRef {
    pub path: String,
    #[serde(rename = "kind")]
    pub kind: crate::tool::ArtifactKind,
}

/// Run 级事件（跨轮）。
#[derive(Debug, Clone, Serialize, Deserialize)]
#[serde(tag = "type", rename_all = "snake_case")]
pub enum RunEvent {
    /// 正在生成的回复文本（增量）
    MessageDelta { text: String },

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
        artifacts: Vec<ArtifactRef>,
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

    /// 当前任务失败
    RunFailed { error: String },

    /// 当前任务已取消
    RunCancelled,
}

impl RunEvent {
    pub fn with_meta(self, session_id: &str, run_id: &str, sequence: Sequence) -> Envelope {
        Envelope {
            session_id: session_id.to_string(),
            run_id: run_id.to_string(),
            sequence,
            event: self,
        }
    }
}

/// 带元数据的事件信封（Tauri event payload）
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct Envelope {
    pub session_id: String,
    pub run_id: String,
    pub sequence: Sequence,
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
        .with_meta("sess-1", "run-1", 3);
        assert_eq!(env.session_id, "sess-1");
        assert_eq!(env.run_id, "run-1");
        assert_eq!(env.sequence, 3);
    }

    #[test]
    fn envelope_serializes_with_snake_case_type_tag() {
        // 前端依赖这个 JSON 形状：type 字段 + flatten 的事件载荷
        let env = RunEvent::MessageDelta { text: "hi".into() }.with_meta("s", "r", 1);
        let v = serde_json::to_value(&env).unwrap();
        assert_eq!(v["type"], "message_delta");
        assert_eq!(v["text"], "hi");
        assert_eq!(v["session_id"], "s");
        assert_eq!(v["run_id"], "r");
        assert_eq!(v["sequence"], 1);
    }

    #[test]
    fn run_event_variants_serialize_with_expected_tags() {
        let cases: Vec<(RunEvent, &str)> = vec![
            (
                RunEvent::StatusChanged {
                    status: RunStatus::Analyzing,
                },
                "status_changed",
            ),
            (
                RunEvent::ToolCallStarted {
                    tool_call_id: "t".into(),
                    tool_name: "read_file".into(),
                    args_summary: "".into(),
                    args: None,
                },
                "tool_call_started",
            ),
            (
                RunEvent::ToolCallCompleted {
                    tool_call_id: "t".into(),
                    tool_name: "write_file".into(),
                    result_summary: "".into(),
                    artifacts: vec![],
                },
                "tool_call_completed",
            ),
            (
                RunEvent::ToolCallFailed {
                    tool_call_id: "t".into(),
                    tool_name: "x".into(),
                    error: "e".into(),
                },
                "tool_call_failed",
            ),
            (
                RunEvent::PermissionRequested {
                    tool_call_id: "t".into(),
                    tool_name: "write_file".into(),
                    summary: "s".into(),
                },
                "permission_requested",
            ),
            (
                RunEvent::PermissionDecided {
                    tool_call_id: "t".into(),
                    decision: crate::permissions::Decision::Allow,
                },
                "permission_decided",
            ),
            (
                RunEvent::RunCompleted {
                    final_text: "f".into(),
                },
                "run_completed",
            ),
            (RunEvent::RunFailed { error: "e".into() }, "run_failed"),
            (RunEvent::RunCancelled, "run_cancelled"),
            (
                RunEvent::PlanUpdated {
                    plan: Plan::empty(),
                },
                "plan_updated",
            ),
        ];
        for (event, expected_tag) in cases {
            let v = serde_json::to_value(&event).unwrap();
            assert_eq!(v["type"], expected_tag, "事件标签不符: {v}");
        }
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
    }

    #[test]
    fn empty_artifacts_are_skipped_in_completed_event() {
        let with = serde_json::to_value(&RunEvent::ToolCallCompleted {
            tool_call_id: "t".into(),
            tool_name: "w".into(),
            result_summary: "".into(),
            artifacts: vec![ArtifactRef {
                path: "a.txt".into(),
                kind: crate::tool::ArtifactKind::FileCreated,
            }],
        })
        .unwrap();
        assert!(with["artifacts"].is_array());
        assert_eq!(with["artifacts"][0]["path"], "a.txt");

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
