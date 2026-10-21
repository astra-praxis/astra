use std::collections::HashMap;
use std::sync::{Arc, Mutex};

use agent_core::{
    Decision, PermissionBridge,
    PermissionRequest,
};
use serde_json::{json, Value};
use tauri::{AppHandle, Emitter};
use tokio::sync::oneshot;


// ===== 权限 hub：前端决定 → 运行循环 =====

/// 多级放权级别。Off = 全部弹确认卡；All = 全部自动批准（原「放权模式」）；
/// Project / Workspace 为路径级放权：仅当请求参数全部落在对应边界内才自动批准，
/// 否则回退到正常确认流程（shell 等无路径参数的 Confirm 类永远弹卡）。
/// Deny 类硬规则（未注册工具、提权命令、路径逃逸）不受任何级别影响。
#[derive(Debug, Clone, Copy, PartialEq, Eq, Default)]
#[repr(u8)]
pub enum AutoMode {
    #[default]
    Off = 0,
    Project = 1,
    Workspace = 2,
    All = 3,
}

impl AutoMode {
    pub(crate) fn from_u8(v: u8) -> Self {
        match v {
            1 => Self::Project,
            2 => Self::Workspace,
            3 => Self::All,
            _ => Self::Off,
        }
    }

    #[cfg(test)]
    pub(crate) fn as_str(self) -> &'static str {
        match self {
            Self::Off => "off",
            Self::Project => "project",
            Self::Workspace => "workspace",
            Self::All => "all",
        }
    }

    pub(crate) fn parse(s: &str) -> Result<Self, String> {
        match s {
            "off" => Ok(Self::Off),
            "project" => Ok(Self::Project),
            "workspace" => Ok(Self::Workspace),
            "all" => Ok(Self::All),
            other => Err(format!(
                "未知的放权级别: {other}（可选 off | project | workspace | all）"
            )),
        }
    }
}

/// 等待用户决定的权限请求集合（tool_call_id → oneshot 发送端）
#[derive(Default)]
pub struct PermissionHub {
    pending: Mutex<HashMap<String, PendingDecision>>,
    /// 多级放权级别（AutoMode as u8），见 AutoMode 文档
    auto_mode: std::sync::atomic::AtomicU8,
}

/// Pending map entry with its owning session: per-session cancel must deny
/// only this session's requests, never another concurrently running one.
struct PendingDecision {
    pub(crate) session_id: String,
    tx: oneshot::Sender<Decision>,
}

impl PermissionHub {
    pub fn set_auto_mode(&self, mode: AutoMode) {
        use std::sync::atomic::Ordering;
        self.auto_mode.store(mode as u8, Ordering::Relaxed);
    }

    pub fn auto_mode(&self) -> AutoMode {
        use std::sync::atomic::Ordering;
        AutoMode::from_u8(self.auto_mode.load(Ordering::Relaxed))
    }

    pub(crate) fn register(&self, session_id: &str, tool_call_id: String) -> oneshot::Receiver<Decision> {
        let (tx, rx) = oneshot::channel();
        self.pending.lock().unwrap().insert(
            tool_call_id,
            PendingDecision {
                session_id: session_id.to_string(),
                tx,
            },
        );
        rx
    }

    pub(crate) fn decide(&self, tool_call_id: &str, d: Decision) -> Result<(), String> {
        let pending = self
            .pending
            .lock()
            .unwrap()
            .remove(tool_call_id)
            .ok_or_else(|| format!("没有等待中的权限请求: {tool_call_id}"))?;
        let _ = pending.tx.send(d);
        Ok(())
    }

    /// 取消 / 重置时丢弃全部挂起请求（对应 receiver 会收到 Err → Deny）
    pub(crate) fn drop_all(&self) {
        self.pending.lock().unwrap().clear();
    }

    /// 取消单个会话时只丢弃该会话的挂起请求（P3 并发：其他会话的
    /// 确认卡不受影响，继续等待用户决策）
    pub(crate) fn drop_session(&self, session_id: &str) {
        self.pending
            .lock()
            .unwrap()
            .retain(|_, p| p.session_id != session_id);
    }
}

/// PermissionBridge 宿主实现：注册 oneshot 并等待前端决定。
/// 发送端被清理（重置会话）时返回 Deny，保证不会挂死运行循环。
/// 多级放权（AutoMode）：All 级别所有 Confirm 直接 Allow；Project / Workspace
/// 级别仅当参数路径全部落在对应边界内才 Allow，否则回退正常确认流程
/// （Deny 不经此桥，不受影响）。
///
/// 工具内部权限（ToolContext::request_permission，如 generate_image 的付费
/// 确认，MULTIMODAL_SPEC §10）不经 run_loop 发事件：本层补发一条可见的
/// agent-event Envelope，前端复用既有确认卡渲染与 agent_permission_decide
/// 回传（同 QuestionBridge 自己发 agent-question 的模式），无需前端改动。
pub(crate) struct HubBridge {
    pub(crate) hub: Arc<PermissionHub>,
    pub(crate) app: AppHandle,
    pub(crate) session_id: String,
    /// 本会话绑定的项目根（None = 纯聊天）；路径级放权的判定基准
    pub(crate) project_root: Option<std::path::PathBuf>,
}

/// 已知携带路径参数的工具参数键（覆盖宿主注册的全部路径类工具：
/// fs_tools 的 path/from/to、doc_tools 的 path/output）。MCP 等外部
/// 工具的任意参数不在内：含未知键时回退确认，保守不放行。
pub(crate) const PATH_ARG_KEYS: &[&str] = &["path", "from", "to", "output", "file_path", "directory"];

/// 提取工具参数中的候选路径（只取已知键下的字符串值；无路径参数返回空）
pub(crate) fn candidate_paths(args: &serde_json::Value) -> Vec<String> {
    let Some(obj) = args.as_object() else {
        return Vec::new();
    };
    PATH_ARG_KEYS
        .iter()
        .filter_map(|k| obj.get(*k))
        .filter_map(|v| v.as_str())
        .filter(|s| !s.is_empty())
        .map(|s| s.to_string())
        .collect()
}

/// 解析工具参数中的路径：支持 `~` 展开、相对路径相对项目根、
/// 词法归一化（消除 `.` / `..`）。无法解析（无项目根的相对路径等）返回 None。
pub(crate) fn resolve_arg_path(
    raw: &str,
    project_root: Option<&std::path::Path>,
) -> Option<std::path::PathBuf> {
    let expanded = if raw == "~" || raw.starts_with("~/") {
        let home = std::env::var_os("HOME")?;
        let rest = raw.strip_prefix("~").unwrap_or("");
        let rest = rest.strip_prefix('/').unwrap_or("");
        std::path::PathBuf::from(home).join(rest)
    } else {
        std::path::PathBuf::from(raw)
    };
    let p = if expanded.is_absolute() {
        expanded
    } else {
        project_root?.join(expanded)
    };
    let mut out = std::path::PathBuf::new();
    for comp in p.components() {
        match comp {
            std::path::Component::CurDir => {}
            std::path::Component::ParentDir => {
                out.pop();
            }
            c => out.push(c.as_os_str()),
        }
    }
    Some(out)
}

/// 解析后的路径是否落在任一边界内（组件级前缀匹配，非字符串前缀）
pub(crate) fn within_any(resolved: &std::path::Path, boundaries: &[std::path::PathBuf]) -> bool {
    boundaries.iter().any(|b| resolved.starts_with(b))
}

/// 多级放权的纯判定：该请求是否可自动 Allow。
/// 规则：Off 永不；All 恒真；Project / Workspace 要求至少提取到一个路径参数，
/// 且全部解析成功并落在对应边界内。无路径参数（shell 等）或含未知参数形状
/// 的请求一律回退确认（false）。
pub(crate) fn auto_allow(
    mode: AutoMode,
    args: &serde_json::Value,
    project_root: Option<&std::path::Path>,
    workspace_roots: &[std::path::PathBuf],
) -> bool {
    match mode {
        AutoMode::Off => false,
        AutoMode::All => true,
        AutoMode::Project => {
            let Some(root) = project_root else {
                return false;
            };
            paths_in(args, project_root, &[root.to_path_buf()])
        }
        AutoMode::Workspace => paths_in(args, project_root, workspace_roots),
    }
}

/// Project / Workspace 级别的公共判定：提取参数路径并逐个检查边界。
/// workspace_roots 为空（未配置工作空间）时回退确认。
pub(crate) fn paths_in(
    args: &serde_json::Value,
    project_root: Option<&std::path::Path>,
    boundaries: &[std::path::PathBuf],
) -> bool {
    if boundaries.is_empty() {
        return false;
    }
    let paths = candidate_paths(args);
    if paths.is_empty() {
        return false;
    }
    paths
        .iter()
        .all(|raw| match resolve_arg_path(raw, project_root) {
            Some(resolved) => within_any(&resolved, boundaries),
            None => false,
        })
}

/// 工具内部权限请求的可见 id / 序号（与 run_loop 的 tc-* call id 区分）
static IN_TOOL_PERM_SEQ: std::sync::atomic::AtomicU64 = std::sync::atomic::AtomicU64::new(0);

pub(crate) fn in_tool_permission_id() -> String {
    let seq = IN_TOOL_PERM_SEQ.fetch_add(1, std::sync::atomic::Ordering::Relaxed);
    format!("perm-tool-{seq}")
}

/// 工具内部权限事件的 Envelope 载荷（纯函数便于单测）。run_id 为空：
/// 请求发生在工具执行中，宿主侧无法拿到 run 上下文；sequence 用独立
/// 大序号避免与 run_loop 每 Run 从 1 重启的序号混淆。
pub(crate) fn permission_envelope(session_id: &str, event: Value) -> Value {
    let seq = IN_TOOL_PERM_SEQ.fetch_add(1, std::sync::atomic::Ordering::Relaxed) + 1;
    let mut env = json!({
        "schema_version": agent_core::events::EVENT_SCHEMA_VERSION,
        "event_id": format!("evt_in_tool_{seq}"),
        "timestamp": chrono::Utc::now().timestamp_millis(),
        "session_id": session_id,
        "run_id": "",
        "sequence": seq,
    });
    if let (Some(base), Some(obj)) = (env.as_object_mut(), event.as_object()) {
        for (k, v) in obj {
            base.insert(k.clone(), v.clone());
        }
    }
    env
}

pub(crate) fn in_tool_permission_event(
    session_id: &str,
    tool_call_id: &str,
    tool_name: &str,
    summary: &str,
) -> Value {
    permission_envelope(
        session_id,
        json!({
            "type": "permission_requested",
            "tool_call_id": tool_call_id,
            "tool_name": tool_name,
            "summary": summary,
        }),
    )
}

pub(crate) fn decision_str(d: Decision) -> &'static str {
    match d {
        Decision::Allow => "allow",
        Decision::Confirm => "confirm",
        Decision::Deny => "deny",
    }
}

pub(crate) fn in_tool_permission_decided_event(
    session_id: &str,
    tool_call_id: &str,
    decision: Decision,
) -> Value {
    permission_envelope(
        session_id,
        json!({
            "type": "permission_decided",
            "tool_call_id": tool_call_id,
            "decision": decision_str(decision),
        }),
    )
}

#[async_trait::async_trait]
impl PermissionBridge for HubBridge {
    async fn request(&self, req: PermissionRequest) -> Decision {
        // 多级放权：All 直接 Allow；Project / Workspace 仅当参数路径全部
        // 落在对应边界内才 Allow，否则回退正常确认流程。硬规则 Deny 在
        // 到达本桥之前已被 core 拦截，放权不影响安全边界。
        let mode = self.hub.auto_mode();
        if mode != AutoMode::Off {
            let workspace_roots = if mode == AutoMode::Workspace {
                crate::workspace_roots(&self.app)
            } else {
                Vec::new()
            };
            if auto_allow(
                mode,
                &req.arguments,
                self.project_root.as_deref(),
                &workspace_roots,
            ) {
                return Decision::Allow;
            }
        }
        // 循环级请求自带 call.id（PermissionRequested 事件已由 run_loop 发出）；
        // 工具内部请求（tool_call_id 为空）需要本层补发可见事件。
        let in_tool = req.tool_call_id.is_empty();
        let id = if in_tool {
            let id = in_tool_permission_id();
            let _ = self.app.emit(
                "agent-event",
                in_tool_permission_event(&self.session_id, &id, &req.tool_name, &req.summary),
            );
            id
        } else {
            req.tool_call_id
        };
        let rx = self.hub.register(&self.session_id, id.clone());
        let decision = match rx.await {
            Ok(d) => d,
            Err(_) => Decision::Deny,
        };
        // 回执 permission_decided：前端确认卡落定（复用既有渲染）
        if in_tool {
            let _ = self.app.emit(
                "agent-event",
                in_tool_permission_decided_event(&self.session_id, &id, decision),
            );
        }
        decision
    }
}
