//! Agent Runtime 桥接层：把 agent-core 的事件流接到 Tauri 前端。
//!
//! 职责边界（对齐 build-agent-runtime skill）：
//! - agent-core 拥有运行循环、权限判定与工具执行；本层只做「宿主适配」：
//!   模型来源解析（provider / 本地 llama.cpp）、事件转发（Envelope → agent-event）、
//!   权限决定回传（oneshot hub）与运行取消
//! - 前端只消费 Envelope（type 标签 + snake_case 载荷），不依赖 agent-core 具体类型
//! - 权限批准绑定 tool_call_id：批准只放行该次具体调用，不扩大范围
//!
//! 会话模型（MVP）：
//! - session_id 由前端生成；Thread 按 session_id 缓存（含消息历史与项目绑定）
//! - 切换项目 / 模型来源后需新会话（Thread 创建时绑定 root 与 backend）
//! - `agent_reset_session` unloads memory only; SQLite owns restart history.

use std::collections::HashMap;
use std::sync::{Arc, Mutex};

use agent_core::model::ImageContent;
use agent_core::skill_tools::truncate_desc;
use agent_core::{
    ChatMessage, Decision, Envelope, MessageContent, ModelBackend, PermissionBridge,
    PermissionRequest, RigBackend, Role, RunHandle, ThreadConfig, ToolPolicy, ToolRegistry,
    DEFAULT_AUTO_COMPACT_TOKENS, DEFAULT_MAX_TURNS,
};
use base64::Engine as _;
use serde::{Deserialize, Serialize};
use serde_json::{json, Value};
use std::path::{Path, PathBuf};
use tauri::{AppHandle, Emitter, Manager, State};
use tokio::sync::oneshot;

use crate::providers;

// ===== 事件日志（JSONL，一行一个 Envelope，供排查对话问题） =====

/// 截断记忆采集用的长文本（按字符，不劈开 UTF-8 序列），附带长度标记。
fn truncate_for_memory(s: &str, max_chars: usize) -> String {
    if s.chars().count() <= max_chars {
        return s.to_owned();
    }
    let head: String = s.chars().take(max_chars).collect();
    format!("{head}…[truncated]")
}

/// 记忆采集器：从 Run 事件流中提取值得进入记忆管线的内容，纯逻辑、
/// 无 AppHandle 依赖——宿主 observe spawn 与集成测试共用同一段代码。
///
/// 采集边界：助手最终回答（unverified，只进摘要不成记忆）、已验证成功的
/// 工具结果、工具失败原因（进摘要，解决方案需验证后才进记忆）。内容截断，
/// 记忆模块不需要完整工具输出。MessageDelta / Started / Permission 等忽略。
struct MemoryRunCollector {
    collected: Vec<(astra_memory::cme::compactor::MessageRole, String, String)>,
}

impl MemoryRunCollector {
    fn new() -> Self {
        Self {
            collected: Vec::new(),
        }
    }

    fn observe(&mut self, env: &Envelope) {
        match &env.event {
            agent_core::RunEvent::RunCompleted { final_text } => {
                let text = final_text.trim();
                if text.chars().count() >= 6 {
                    self.collected.push((
                        astra_memory::cme::compactor::MessageRole::Assistant,
                        truncate_for_memory(text, 2000),
                        env.event_id.clone(),
                    ));
                }
            }
            agent_core::RunEvent::ToolCallCompleted {
                tool_name,
                result_summary,
                raw_output,
                ..
            } => {
                let body = match raw_output {
                    Some(raw) if !raw.trim().is_empty() => truncate_for_memory(raw.trim(), 2000),
                    _ => truncate_for_memory(result_summary.trim(), 2000),
                };
                if !body.is_empty() {
                    self.collected.push((
                        astra_memory::cme::compactor::MessageRole::Tool,
                        format!("{tool_name}: {body}"),
                        env.event_id.clone(),
                    ));
                }
            }
            agent_core::RunEvent::ToolCallFailed {
                tool_name, error, ..
            } => {
                let body = error.trim();
                if !body.is_empty() {
                    self.collected.push((
                        astra_memory::cme::compactor::MessageRole::Tool,
                        format!(
                            "[tool failed] {tool_name}: {}",
                            truncate_for_memory(body, 2000)
                        ),
                        env.event_id.clone(),
                    ));
                }
            }
            _ => {}
        }
    }

    /// 组装 ingest 消息：用户输入在前，采集到的运行事件随后。id 用事件
    /// event_id（重放稳定）；空则回退到时间序合成 id。
    fn into_messages(
        self,
        user_text: &str,
        now_ms: u64,
    ) -> Vec<astra_memory::cme::compactor::ConversationMessage> {
        let mut messages: Vec<astra_memory::cme::compactor::ConversationMessage> = Vec::new();
        let text = user_text.trim();
        if !text.is_empty() {
            messages.push(astra_memory::cme::compactor::ConversationMessage {
                id: format!("u_{now_ms}"),
                thread_id: String::new(), // ingest 按 session 盖章
                role: astra_memory::cme::compactor::MessageRole::User,
                content: text.to_owned(),
                created_at: now_ms as i64,
            });
        }
        for (role, content, event_id) in self.collected {
            let id = if event_id.is_empty() {
                format!("{now_ms}_{}", messages.len())
            } else {
                event_id
            };
            messages.push(astra_memory::cme::compactor::ConversationMessage {
                id,
                thread_id: String::new(),
                role,
                content,
                created_at: now_ms as i64,
            });
        }
        messages
    }
}

/// Run 结束后的记忆提交（observe spawn 与真实 Run 测试共用）：登记采集
/// 消息进 CME 待处理队列，含优先级线索时立即压缩提炼；有待确认的
/// supersedes 意图且语义确认通道可用时，逐条裁决建链（docs/13 W2）。
/// 任何失败只记日志：记忆故障永不阻断 agent（D21）。返回是否走了立即
/// 处理路径（priority gate 触发），测试用于断言。
async fn commit_collected_memory(
    engine: &std::sync::Arc<
        astra_memory::cme::engine::MemoryEngine<astra_memory::cme::store_sqlite::CmeSqliteStore>,
    >,
    collector: MemoryRunCollector,
    session_id: &str,
    project: Option<&str>,
    user_text: &str,
    now_ms: u64,
    reviewer: Option<&dyn astra_memory::cme::consolidate::SupersedeReviewer>,
) -> bool {
    if let Err(error) = engine.set_thread_scope(session_id, project).await {
        eprintln!("[memory] cme set_thread_scope failed (session {session_id}): {error}");
    }
    let messages = collector.into_messages(user_text, now_ms);
    if messages.is_empty() {
        return false;
    }
    engine.ingest(session_id, messages);
    // 含重要线索（决策/纠正/偏好/已验证工具结果）立即处理，不让关键信息
    // 滞留在待处理队列；普通对话走阈值批处理。
    let force = engine.has_priority_pending();
    if let Err(error) = engine.process_pending(force).await {
        eprintln!("[memory] cme process_pending failed (session {session_id}): {error}");
    }
    // 语义确认（docs/13 W2）：规则发现的非 Decision supersedes 意图只有
    // 宿主 agent 裁决后才建链；拒绝/失败都保守并存，意图不重复入队。
    if engine.pending_supersede_count() > 0 {
        match reviewer {
            Some(reviewer) => match engine.resolve_pending_supersedes(reviewer).await {
                Ok(r) => eprintln!(
                    "[memory] supersede resolved (session {session_id}): chained={} rejected={} unresolved={} vanished={}",
                    r.chained, r.rejected, r.unresolved, r.vanished
                ),
                Err(error) => {
                    eprintln!(
                        "[memory] cme resolve_supersedes failed (session {session_id}): {error}"
                    )
                }
            },
            None => eprintln!(
                "[memory] {} supersede intent(s) pending (semantic reviewer disabled)",
                engine.pending_supersede_count()
            ),
        }
    }
    force
}

/// 把事件追加写入 app_data_dir/agent_events.log（JSONL）。
/// 失败只忽略——日志不能影响对话本身。
fn log_event(app: &AppHandle, env: &Envelope) {
    let Ok(dir) = app.path().app_data_dir() else {
        return;
    };
    let path = dir.join("agent_events.log");
    let Ok(mut f) = std::fs::OpenOptions::new()
        .create(true)
        .append(true)
        .open(&path)
    else {
        return;
    };
    if let Ok(mut line) = serde_json::to_value(env) {
        // 时间戳便于离线量化每步耗时（事件流本身无时序信息）
        line["logged_at"] = Value::String(chrono::Local::now().to_rfc3339());
        if let Ok(s) = serde_json::to_string(&line) {
            use std::io::Write;
            let _ = writeln!(f, "{s}");
        }
    }
}

/// 读取最近 N 条事件日志（最新在后），供 /logs 页或前端排查
#[tauri::command]
pub fn agent_recent_events(app: AppHandle, limit: Option<u32>) -> Result<Vec<Value>, String> {
    let dir = app
        .path()
        .app_data_dir()
        .map_err(|e| format!("无法定位应用数据目录: {e}"))?;
    let path = dir.join("agent_events.log");
    let content = std::fs::read_to_string(&path).unwrap_or_default();
    let lines: Vec<&str> = content.lines().collect();
    let start = lines.len().saturating_sub(limit.unwrap_or(200) as usize);
    Ok(lines[start..]
        .iter()
        .filter_map(|l| serde_json::from_str(l).ok())
        .collect())
}

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
    fn from_u8(v: u8) -> Self {
        match v {
            1 => Self::Project,
            2 => Self::Workspace,
            3 => Self::All,
            _ => Self::Off,
        }
    }

    #[cfg(test)]
    fn as_str(self) -> &'static str {
        match self {
            Self::Off => "off",
            Self::Project => "project",
            Self::Workspace => "workspace",
            Self::All => "all",
        }
    }

    fn parse(s: &str) -> Result<Self, String> {
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
    pending: Mutex<HashMap<String, oneshot::Sender<Decision>>>,
    /// 多级放权级别（AutoMode as u8），见 AutoMode 文档
    auto_mode: std::sync::atomic::AtomicU8,
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

    fn register(&self, tool_call_id: String) -> oneshot::Receiver<Decision> {
        let (tx, rx) = oneshot::channel();
        self.pending.lock().unwrap().insert(tool_call_id, tx);
        rx
    }

    fn decide(&self, tool_call_id: &str, d: Decision) -> Result<(), String> {
        let tx = self
            .pending
            .lock()
            .unwrap()
            .remove(tool_call_id)
            .ok_or_else(|| format!("没有等待中的权限请求: {tool_call_id}"))?;
        let _ = tx.send(d);
        Ok(())
    }

    /// 取消 / 重置时丢弃全部挂起请求（对应 receiver 会收到 Err → Deny）
    fn drop_all(&self) {
        self.pending.lock().unwrap().clear();
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
struct HubBridge {
    hub: Arc<PermissionHub>,
    app: AppHandle,
    session_id: String,
    /// 本会话绑定的项目根（None = 纯聊天）；路径级放权的判定基准
    project_root: Option<std::path::PathBuf>,
}

/// 已知携带路径参数的工具参数键（覆盖宿主注册的全部路径类工具：
/// fs_tools 的 path/from/to、doc_tools 的 path/output）。MCP 等外部
/// 工具的任意参数不在内：含未知键时回退确认，保守不放行。
const PATH_ARG_KEYS: &[&str] = &["path", "from", "to", "output", "file_path", "directory"];

/// 提取工具参数中的候选路径（只取已知键下的字符串值；无路径参数返回空）
fn candidate_paths(args: &serde_json::Value) -> Vec<String> {
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
fn resolve_arg_path(
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
fn within_any(resolved: &std::path::Path, boundaries: &[std::path::PathBuf]) -> bool {
    boundaries.iter().any(|b| resolved.starts_with(b))
}

/// 多级放权的纯判定：该请求是否可自动 Allow。
/// 规则：Off 永不；All 恒真；Project / Workspace 要求至少提取到一个路径参数，
/// 且全部解析成功并落在对应边界内。无路径参数（shell 等）或含未知参数形状
/// 的请求一律回退确认（false）。
fn auto_allow(
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
fn paths_in(
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

fn in_tool_permission_id() -> String {
    let seq = IN_TOOL_PERM_SEQ.fetch_add(1, std::sync::atomic::Ordering::Relaxed);
    format!("perm-tool-{seq}")
}

/// 工具内部权限事件的 Envelope 载荷（纯函数便于单测）。run_id 为空：
/// 请求发生在工具执行中，宿主侧无法拿到 run 上下文；sequence 用独立
/// 大序号避免与 run_loop 每 Run 从 1 重启的序号混淆。
fn permission_envelope(session_id: &str, event: Value) -> Value {
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

fn in_tool_permission_event(
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

fn decision_str(d: Decision) -> &'static str {
    match d {
        Decision::Allow => "allow",
        Decision::Confirm => "confirm",
        Decision::Deny => "deny",
    }
}

fn in_tool_permission_decided_event(
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
        let rx = self.hub.register(id.clone());
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

// ===== 全局状态 =====

pub struct AgentState {
    hub: Arc<PermissionHub>,
    questions: Arc<QuestionHub>,
    active: Arc<Mutex<Option<(String, RunHandle)>>>,
    pub(crate) threads: Arc<tokio::sync::Mutex<crate::agent_threads::AgentThreads>>,
    pub(crate) persistence_errors: crate::agent_threads::PersistenceErrors,
    // Serialize UI JSON updates and reject delayed saves after deletion.
    pub(crate) session_files: tokio::sync::Mutex<std::collections::HashSet<String>>,
    /// Background task registry shared by every session's tool registry.
    /// Project-level lifetime: dev servers survive session switches; killed
    /// on app exit (ProcessManager::drop → process-group teardown).
    pub(crate) processes: Arc<agent_core::shell_background::ProcessManager>,
    /// 统一异步任务运行时（TASK_SPEC）：shell_background spawn 的进程经
    /// adopt 收编进来，task_status/task_result/task_cancel 跨会话跟踪。
    /// v1 仅内存态（无 store、无事件通道），Run 永不挂起（D1）。
    pub(crate) tasks: Arc<agent_core::TaskManager>,
    /// MCP 任务后端（TASK_SPEC D6 修订版）：MCP server 物化的任务经
    /// adapter 收编进 tasks，路由回各会话的 McpManager 连接轮询。
    pub(crate) mcp_tasks: Arc<astra_mcp::McpTaskBackend>,
    /// 运行中的扩展 manager（每会话一份，attach_extensions 时登记）：
    /// 保活让 ACP 崩溃降级 / 懒重连继续可用，并支撑运行状态查询。
    pub(crate) extensions: Arc<tokio::sync::Mutex<HashMap<String, SessionExtensions>>>,
    /// 多模态 M5（MULTIMODAL_SPEC §10）：generation 栈。backend 在构造期
    /// 一次性注册进 tasks；provider 按配置懒构建/懒切换（agent_start_run
    /// 每次 sync，新会话生效），disabled 时工具不注册、不阻塞会话。
    pub(crate) generation: crate::agent_generation::GenerationStack,
    /// 记忆宿主（astra-memory 集成）：setup 阶段初始化（需要 app_data_dir），
    /// 失败降级为 memory-off（agent 照常运行，无 recall 注入）。
    pub(crate) memory: Arc<std::sync::OnceLock<crate::memory::MemoryHost>>,
}

/// 一次会话装配产生的扩展 manager 句柄（Arc 保活）。
#[derive(Default)]
pub(crate) struct SessionExtensions {
    pub mcp: Vec<std::sync::Arc<astra_mcp::McpManager>>,
    pub acp: Vec<std::sync::Arc<astra_acp::AcpManager>>,
}

impl Default for AgentState {
    fn default() -> Self {
        let processes = Arc::new(agent_core::shell_background::ProcessManager::new());
        let mcp_tasks = Arc::new(astra_mcp::McpTaskBackend::new());
        let generation = crate::agent_generation::GenerationStack::new();
        Self {
            hub: Arc::new(PermissionHub::default()),
            questions: Arc::new(QuestionHub::default()),
            active: Arc::new(Mutex::new(None)),
            threads: Arc::new(tokio::sync::Mutex::new(
                crate::agent_threads::AgentThreads::default(),
            )),
            persistence_errors: Arc::new(Mutex::new(HashMap::new())),
            session_files: tokio::sync::Mutex::new(std::collections::HashSet::new()),
            extensions: Arc::new(tokio::sync::Mutex::new(HashMap::new())),
            processes: Arc::clone(&processes),
            tasks: Arc::new(
                agent_core::TaskManager::new(agent_core::task::DEFAULT_MAX_TASKS, None)
                    .with_backend(Arc::clone(&mcp_tasks) as Arc<dyn agent_core::TaskBackend>)
                    .with_backend(Arc::new(agent_core::ShellTaskBackend::new(
                        // root only serves direct backend.start (host path is
                        // adoption via shell_background); temp dir is a safe stub.
                        std::env::temp_dir(),
                        processes,
                    )))
                    // 多模态 M5：generation 后端一次性注册（TASK_SOURCE kind
                    // "generation" 路由）；provider 是否可用由配置决定，
                    // disabled 时无工具入口，后端闲置不产生任何任务。
                    .with_backend(generation.task_backend() as Arc<dyn agent_core::TaskBackend>),
            ),
            mcp_tasks,
            generation,
            memory: Arc::new(std::sync::OnceLock::new()),
        }
    }
}

// ===== ask_user 提问桥：问题卡事件 + oneshot 回答 =====

/// 等待用户回答的问题集合（question_id → oneshot 发送端）
#[derive(Default)]
pub struct QuestionHub {
    pending: Mutex<HashMap<String, oneshot::Sender<Option<String>>>>,
}

static QUESTION_SEQ: std::sync::atomic::AtomicU64 = std::sync::atomic::AtomicU64::new(0);

impl QuestionHub {
    fn register(&self) -> (String, oneshot::Receiver<Option<String>>) {
        let seq = QUESTION_SEQ.fetch_add(1, std::sync::atomic::Ordering::Relaxed);
        let id = format!(
            "q-{}-{seq}",
            std::time::SystemTime::now()
                .duration_since(std::time::UNIX_EPOCH)
                .map(|d| d.as_nanos())
                .unwrap_or(0)
        );
        let (tx, rx) = oneshot::channel();
        self.pending.lock().unwrap().insert(id.clone(), tx);
        (id, rx)
    }

    /// 用户回答（None = 取消/拒绝）
    fn answer(&self, question_id: &str, answer: Option<String>) -> Result<(), String> {
        let tx = self
            .pending
            .lock()
            .unwrap()
            .remove(question_id)
            .ok_or_else(|| format!("没有等待中的问题: {question_id}"))?;
        let _ = tx.send(answer);
        Ok(())
    }

    /// 取消 / 重置时丢弃全部挂起问题（对应 receiver 收到 Err → 问题作废）
    fn drop_all(&self) {
        self.pending.lock().unwrap().clear();
    }
}

/// UserQuestionBridge 宿主实现：发 agent-question 事件给前端，
/// oneshot 等待 agent_question_answer 回传。发送端被清理（取消/重置）
/// 时返回 Err，问题作废，工具把错误回传模型。
/// 从应用设置读取 search_web 的搜索配置：`search_engine` 选后端（bing |
/// duckduckgo | searxng，未设置时默认 bing；仅配置了 search_endpoint 的旧
/// 设置保持 SearXNG 语义，由 agent-core 的 SearchEngine::resolve 统一裁定）。
fn settings_search(app: &AppHandle) -> agent_core::net_tools::SearchEngine {
    let read = |key: &str| -> Option<String> {
        let path = crate::get_settings_path(app).ok()?;
        let content = std::fs::read_to_string(path).ok()?;
        let v: serde_json::Value = serde_json::from_str(&content).ok()?;
        v.get(key)
            .and_then(|s| s.as_str())
            .map(|s| s.trim().to_string())
            .filter(|s| !s.is_empty())
    };
    let endpoint = read("search_endpoint").map(|s| s.trim_end_matches('/').to_string());
    agent_core::net_tools::SearchEngine::resolve(read("search_engine").as_deref(), endpoint)
}

struct QuestionBridge {
    hub: Arc<QuestionHub>,
    app: AppHandle,
    session_id: String,
}

/// 构造 agent-question 事件载荷。抽出纯函数便于单测：
/// session_id 必须随事件下发，前端靠它过滤本会话的问题卡。
fn question_payload(
    session_id: &str,
    question_id: &str,
    question: &str,
    options: Vec<String>,
    allow_free_text: bool,
) -> serde_json::Value {
    json!({
        "session_id": session_id,
        "question_id": question_id,
        "question": question,
        "options": options,
        "allow_free_text": allow_free_text,
    })
}

#[async_trait::async_trait]
impl agent_core::UserQuestionBridge for QuestionBridge {
    async fn ask(
        &self,
        question: String,
        options: Vec<String>,
        allow_free_text: bool,
    ) -> std::result::Result<String, String> {
        let (qid, rx) = self.hub.register();
        self.app
            .emit(
                "agent-question",
                question_payload(&self.session_id, &qid, &question, options, allow_free_text),
            )
            .map_err(|e| format!("发送问题失败: {e}"))?;
        match rx.await {
            Ok(Some(answer)) => Ok(answer),
            Ok(None) => Err("用户取消回答".into()),
            Err(_) => Err("问题已失效（任务取消或会话重置）".into()),
        }
    }
}

// ===== 工具注册（全部实现在 agent-core，本层只做组装） =====

/// 宿主工具权限白名单（严格模型：不在名单内一律 Deny）。每次新增内置
/// 工具时必须同步这里，否则注册成功但执行被拒（memory 工具踩过一次：
/// agent 看得到、调不了）。
fn host_tool_policy() -> ToolPolicy {
    // 在默认策略上追加 pandoc 工具：转换是只读产物生成（Allow），
    // 构建 docx 会覆盖/创建交付文件（Confirm）
    let mut policy = ToolPolicy::default();
    // fs_tools（Zed 移植）：find_path 只读；edit/copy/move/mkdir 产生变更走 Confirm
    policy.allowed_readonly.push("find_path".into());
    // pandoc 转换是只读产物生成（Allow）；ask_user / search_web 无副作用（Allow）
    policy.allowed_readonly.push("extract_document".into());
    policy.allowed_readonly.push("ask_user".into());
    policy.allowed_readonly.push("search_web".into());
    // MCP 配置自助管理：读无副作用 Allow；写入配置 Confirm（放权级别
    // 为 all 时随全放行放行，装扩展是用户主动要求的行为）
    policy.allowed_readonly.push("mcp_config_read".into());
    policy.confirm_tools.push("mcp_config_write".into());
    policy.confirm_tools.push("edit_file".into());
    policy.confirm_tools.push("copy_path".into());
    policy.confirm_tools.push("move_path".into());
    policy.confirm_tools.push("create_directory".into());
    policy.confirm_tools.push("build_docx".into());
    policy.confirm_tools.push("shell_execute".into());
    // 后台任务：启动/停止有副作用走 Confirm；shell_status 只读（Allow）
    policy.confirm_tools.push("shell_background".into());
    policy.confirm_tools.push("shell_kill".into());
    policy.allowed_readonly.push("shell_status".into());
    // 统一任务查询（TASK_SPEC D3）：status/result 只读 Allow，
    // cancel 有副作用走 Confirm
    policy.allowed_readonly.push("task_status".into());
    policy.allowed_readonly.push("task_result".into());
    policy.confirm_tools.push("task_cancel".into());
    // 付费生成（MULTIMODAL_SPEC §10）：循环层直接放行，付费确认由
    // 工具内部 request_permission 完成（批准仅针对固定请求，换参数
    // 会重新弹窗）——避免循环层 Confirm + 工具内确认双重弹窗。
    policy.allowed_readonly.push("generate_image".into());
    // fetch 涉及外部网络访问，走确认弹窗（SSRF 门在工具内部始终生效）
    policy.confirm_tools.push("fetch".into());
    // Agent 记忆工具：search 只读 Allow；remember 写的是应用自己的
    // 记忆库（非文件系统/系统变更，可经 engine.forget 回退），且设计
    // 要求「显式记住立即生效」——走 Confirm 弹窗会破坏这个语义，
    // 故 Allow（memory-off 时工具根本不注册，不存在未声明即放行）。
    // forget 仅逻辑遗忘（valid_until=now，可逆：重新 remember 同内容
    // 即恢复），物理删除不暴露给 agent，故同为 Allow（docs/13 D2）。
    policy.allowed_readonly.push("memory_search".into());
    policy.allowed_readonly.push("memory_remember".into());
    policy.allowed_readonly.push("memory_forget".into());
    policy
}

fn build_registry(
    session_id: &str,
    project_root: Option<&PathBuf>,
    skills_dir: Option<&Path>,
    question_bridge: Arc<dyn agent_core::UserQuestionBridge>,
    search: agent_core::net_tools::SearchEngine,
    processes: Arc<agent_core::shell_background::ProcessManager>,
    tasks: Arc<agent_core::TaskManager>,
    generation: Option<(
        Arc<agent_core::task::GenerationTaskBackend>,
        Arc<dyn agent_core::astra_generation::GenerationProvider>,
    )>,
    memory: Option<
        std::sync::Arc<
            astra_memory::cme::engine::MemoryEngine<
                astra_memory::cme::store_sqlite::CmeSqliteStore,
            >,
        >,
    >,
) -> ToolRegistry {
    // 网络工具 + ask_user 不依赖项目根，纯聊天也可用；任务查询工具同全局
    let reg = agent_core::net_tools::register_with_search(ToolRegistry::new(), search)
        .register(agent_core::AskUserTool::new(question_bridge))
        // MCP 配置自助管理：用户在对话里说「装个 xx MCP」时，agent 用它写
        // ~/.astra/config.toml（工具集合建会话时冻结，写入后新会话生效）
        .register(crate::mcp_config_tool::McpConfigRead)
        .register(crate::mcp_config_tool::McpConfigWrite);
    let reg = agent_core::task_tools::register(reg, tasks.clone());
    // 多模态 M5：enabled 且配置完整时注册 generate_image（付费生成，
    // MULTIMODAL_SPEC §10）；disabled / 缺配置时不注册、不报错。
    let reg = match generation {
        Some((backend, provider)) => agent_core::generation_tools::register(
            reg,
            tasks.clone(),
            backend,
            provider,
            session_id,
        ),
        None => reg,
    };
    let reg = match skills_dir {
        Some(dir) => agent_core::skill_tools::register(reg, dir),
        None => reg,
    };
    // Agent 记忆工具（memory_remember / memory_search）：memory-off 时不注册，
    // agent 不会看到无法调用的工具。
    let reg = match memory {
        Some(engine) => crate::memory_tool::register(reg, engine),
        None => reg,
    };
    match project_root {
        Some(root) => {
            let reg = agent_core::fs_basic::register(reg, root);
            let reg = agent_core::doc_tools::register(reg, root);
            let reg = agent_core::shell_tool::register(reg, root);
            // Background task management (dev servers / watchers): manager is
            // host-owned and shared across sessions, so tasks outlive a chat.
            // Spawned processes are adopted into the unified task runtime
            // (TASK_SPEC §六: same TaskManager as task_status/task_result).
            let reg = agent_core::shell_background::register(reg, root, processes, Some(tasks));
            // fs_tools（Zed 移植）：搜索/编辑/移动类
            agent_core::fs_tools::register(reg, root)
        }
        None => reg,
    }
}

/// MCP/ACP 扩展装配（MCP_SPEC §14 / ACP_SPEC §11 宿主清单）：读
/// `~/.astra/config.toml`（`ASTRA_HOME` 可覆盖）的 [mcp]/[acp] 段，
/// 加载外部能力并注册进本次 registry。规则：
/// - required 失败阻断本次启动；其余失败降级（工具不出现，日志报错）
/// - 配置文件缺失或无效 = 无扩展，不阻断普通对话（配置错误经日志暴露）
/// - 工具集合建 Thread 时冻结：仅新会话生效，与「切项目/模型后需新会话」一致
/// - 每个 Thread 一份独立 manager，避免共享 manager 重载时拆掉其他会话
///   仍在用的连接（v1 简化，代价是同 server 随会话数多开进程）
async fn attach_extensions(
    mut registry: ToolRegistry,
    config: &mut ThreadConfig,
    handles: &mut SessionExtensions,
    session_id: &str,
    mcp_tasks: Arc<astra_mcp::McpTaskBackend>,
    tasks: Arc<agent_core::TaskManager>,
) -> Result<ToolRegistry, String> {
    let cfg = match agent_core::astra_config::load() {
        Ok(c) => c,
        Err(e) => {
            eprintln!("[extensions] 配置无效，本次运行不加载 MCP/ACP: {e}");
            return Ok(registry);
        }
    };

    // ===== MCP =====
    if cfg.mcp.servers.values().any(|s| s.enabled) {
        // D6 修订版：接线统一任务收编——server 物化的任务（resultType:
        // "task"）经 adapter adopt 进全局 TaskManager，模型拿统一 task_id
        // 用 task_status/task_result 轮询；后端按 session 路由回本 manager。
        let manager = astra_mcp::McpManager::new(cfg.mcp.to_manager_configs())
            .map_err(|e| format!("MCP 配置无效: {e}"))?
            .with_task_tracking(session_id.to_string(), tasks);
        let manager = std::sync::Arc::new(manager);
        mcp_tasks.register_session(session_id, &manager).await;
        let (reg, report) = manager.load_and_register(registry).await;
        registry = reg;
        for failed in &report.failed {
            eprintln!(
                "[extensions] MCP server '{}' 加载失败: {}",
                failed.id, failed.error
            );
            if failed.required {
                return Err(format!(
                    "MCP server '{}' 为 required，加载失败: {}",
                    failed.id, failed.error
                ));
            }
        }
        for dropped in &report.dropped_tools {
            eprintln!(
                "[extensions] MCP 工具被丢弃: {}/{} ({})",
                dropped.server, dropped.tool, dropped.reason
            );
        }
        // manager 由 rule_provider + state.extensions 保活；权限规则按发现
        // 结果逐工具生成，未列出的默认 Confirm（MCP_SPEC §9.2），经
        // ThreadConfig 进入权限管线。
        handles.mcp.push(std::sync::Arc::clone(&manager));
        let permissions: std::collections::HashMap<
            String,
            agent_core::astra_config::McpPermissions,
        > = cfg
            .mcp
            .servers
            .iter()
            .filter(|(_, e)| e.enabled)
            .map(|(id, e)| (id.clone(), e.permissions.clone()))
            .collect();
        config.rule_provider = Some(std::sync::Arc::new(
            agent_core::astra_config::McpRuleProvider::new(manager, permissions),
        ));
        eprintln!(
            "[extensions] MCP: {} server(s) 已注册（见上方失败日志，如有）",
            report.loaded.len()
        );
    }

    // ===== ACP =====
    if cfg.acp.agents.values().any(|a| a.enabled) {
        let manager = astra_acp::AcpManager::new(cfg.acp.to_specs())
            .map_err(|e| format!("ACP 配置无效: {e}"))?;
        let (reg, report) = manager.load_and_register(registry).await;
        registry = reg;
        for failed in &report.failed {
            eprintln!(
                "[extensions] ACP agent '{}' 加载失败: {}",
                failed.id, failed.error
            );
            if failed.required {
                return Err(format!(
                    "ACP agent '{}' 为 required，加载失败: {}",
                    failed.id, failed.error
                ));
            }
        }
        eprintln!(
            "[extensions] ACP: {} agent(s) 已注册（见上方失败日志，如有）",
            report.loaded.len()
        );
        // D7.1：调用外部 Agent 本身复用既有 decide 管线——装配期对每个注册
        // 成功的 agent 生成 Confirm 规则（risk=High 不降级，未配置即 Confirm）。
        // [acp.agents.*].permission 只管第二层 request_permission 应答，
        // 不驱动本层。名字必须与 adapter 注册名一致（normalize_tool_name）。
        for id in &report.loaded {
            config
                .policy
                .confirm_tools
                .push(astra_acp::normalize_tool_name(id));
        }
        // manager 保活在 state.extensions：adapter 持有连接不影响调用，
        // 保活额外让崩溃降级 / 懒重连 / 运行状态查询可用。
        handles.acp.push(std::sync::Arc::new(manager));
    }

    Ok(registry)
}

// ===== 运行状态查询 =====

#[derive(Debug, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct McpRuntimeStatus {
    pub id: String,
    /// stopped | starting | ready | failed
    pub status: String,
    pub tool_count: usize,
    pub tools: Vec<String>,
    pub error: Option<String>,
    pub attempts: u32,
}

#[derive(Debug, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct AcpRuntimeStatus {
    pub id: String,
    /// stopped | starting | ready | failed
    pub status: String,
    pub tool: Option<String>,
    pub error: Option<String>,
    pub attempts: u32,
}

#[derive(Debug, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct SessionRuntimeStatus {
    pub session_id: String,
    pub mcp: Vec<McpRuntimeStatus>,
    pub acp: Vec<AcpRuntimeStatus>,
}

/// MCP/ACP 运行状态（agent_start_run 装配时登记的 manager 快照）。
/// `session_id = None` 返回所有活跃会话；会话存在但未配置扩展 → 空 vec。
#[tauri::command]
pub async fn extensions_runtime_status(
    state: State<'_, AgentState>,
    session_id: Option<String>,
) -> Result<Vec<SessionRuntimeStatus>, String> {
    let extensions = state.extensions.lock().await;
    let mut out = Vec::new();
    for (sid, ext) in extensions.iter() {
        if session_id.as_ref().is_some_and(|s| s != sid) {
            continue;
        }
        let mut mcp = Vec::new();
        for manager in &ext.mcp {
            for s in manager.snapshot().await {
                mcp.push(McpRuntimeStatus {
                    id: s.id,
                    status: format!("{:?}", s.status).to_lowercase(),
                    tool_count: s.tools.len(),
                    tools: s.tools.into_iter().map(|t| t.remote).collect(),
                    error: s.error,
                    attempts: s.attempts,
                });
            }
        }
        let mut acp = Vec::new();
        for manager in &ext.acp {
            for s in manager.snapshot().await {
                acp.push(AcpRuntimeStatus {
                    id: s.id,
                    status: format!("{:?}", s.status).to_lowercase(),
                    tool: s.tool,
                    error: s.error,
                    attempts: s.attempts,
                });
            }
        }
        out.push(SessionRuntimeStatus {
            session_id: sid.clone(),
            mcp,
            acp,
        });
    }
    Ok(out)
}

fn image_encoding(enabled: bool) -> agent_core::model::ImageEncoding {
    if enabled {
        agent_core::model::ImageEncoding::UserImageUrl
    } else {
        agent_core::model::ImageEncoding::Disabled
    }
}

// ===== 命令 =====

/// 用户附带图片的传输形状（与 astra-core `ImageContent` 对齐：base64 规范 PNG）
#[derive(Debug, Clone, Deserialize)]
pub struct AgentImageInput {
    pub data: String,
    pub width: u32,
    pub height: u32,
}

#[derive(Debug, Deserialize)]
pub struct AgentStartReq {
    /// API 来源：provider_id 与 model_port 二选一
    pub provider_id: Option<String>,
    pub model_port: Option<u16>,
    /// 模型名覆盖；不传则用 provider 的 active_model
    pub model: Option<String>,
    /// 项目根目录（None = 纯聊天，不注册文件工具）
    pub project_root: Option<String>,
    pub system_prompt: Option<String>,
    /// 最大续轮数（不传用 agent-core 默认）
    pub max_turns: Option<u32>,
    /// 上下文自动压缩阈值（估算 token 数；0 或 None = 关闭）
    pub auto_compact_tokens: Option<u64>,
    /// 用户通过 /技能名 指定的技能（内容注入本次用户消息）
    pub skill: Option<String>,
    /// 前端生成的会话 id（同会话复用 Thread 与历史）
    pub session_id: String,
    pub user_text: String,
    /// 用户附带的图片（规范 PNG 的 base64 与尺寸，经 agent_normalize_image 归一化）。
    /// 需要后端 vision 能力（image_recognition 开启）；与文本保持顺序（文本在前）。
    #[serde(default)]
    pub images: Vec<AgentImageInput>,
    /// Explicit consent for image-capable model/gateway requests. Never inferred from names.
    #[serde(default)]
    pub image_recognition: bool,
    /// Thinking mode for reasoning models; None leaves gateway defaults untouched.
    #[serde(default)]
    pub thinking_enabled: Option<bool>,
    /// Reasoning effort level (e.g. low/medium/high); requires a thinking-capable model.
    #[serde(default)]
    pub thinking_effort: Option<String>,
}

/// 单条消息最多附带图片数（与 core 的 MAX_IMAGES_PER_REQUEST 一致；
/// 工具图与本图共享该预算，超限时由 core 二次校验明确拒绝）
const MAX_ATTACHED_IMAGES: usize = agent_core::model::image::MAX_IMAGES_PER_REQUEST;

/// 组装用户输入内容块（多模态 M1）。
///
/// 顺序：文本块在前（技能包装只影响文本），图片块随后。纯图片消息允许
/// （文本为空时不产生空文本块）；core 的 `try_send_content` 做最终准入校验
/// （vision 能力、预算、PNG 有效性），此处只做宿主侧的快速失败。
fn build_user_content(
    user_text: &str,
    images: &[AgentImageInput],
    skill: Option<&str>,
    skills_dir: &Option<std::path::PathBuf>,
) -> Result<Vec<MessageContent>, String> {
    if images.len() > MAX_ATTACHED_IMAGES {
        return Err(format!("单条消息最多附带 {MAX_ATTACHED_IMAGES} 张图片"));
    }
    let text = match skill {
        None => user_text.to_string(),
        Some(skill_name) => {
            let dir = skills_dir
                .as_ref()
                .ok_or_else(|| format!("技能库不可用，无法加载技能 '{skill_name}'"))?;
            let content = crate::user_skills::read_skill_content(dir, skill_name, None)
                .map_err(|e| e.to_string())?;
            format!(
                "【用户通过 /{skill_name} 指定了技能，请先阅读以下技能说明并严格按其执行】\n\n{content}\n\n---\n\n用户任务：{user_text}"
            )
        }
    };
    let mut content = Vec::new();
    if !text.trim().is_empty() {
        content.push(MessageContent::text(text));
    }
    for img in images {
        let bytes = base64::engine::general_purpose::STANDARD
            .decode(img.data.as_bytes())
            .map_err(|_| "图片数据不是合法的 base64".to_string())?;
        let image = ImageContent::from_png(&bytes, img.width, img.height)
            .map_err(|e| format!("图片未通过规范校验：{e}"))?;
        content.push(MessageContent::Image { image });
    }
    if content.is_empty() {
        return Err("消息内容为空".into());
    }
    Ok(content)
}

/// 启动一次 Run（多轮对话中的一轮）。事件经 `agent-event` 流出。
/// 返回会话 id。
#[tauri::command]
pub async fn agent_start_run(
    app: AppHandle,
    state: State<'_, AgentState>,
    req: AgentStartReq,
) -> Result<String, String> {
    // Keep admission exclusive through stream closure, including write-through.
    let mut threads = Arc::clone(&state.threads)
        .try_lock_owned()
        .map_err(|_| "Agent is running, draining or changing sessions".to_string())?;
    let directory = app.path().app_data_dir().map_err(|e| e.to_string())?;
    threads
        .open(&directory, Arc::clone(&state.persistence_errors))
        .await?;

    // 1. 模型来源（URL 约定与 chat.rs 一致）
    let (base_url, api_key, model_name) = if let Some(port) = req.model_port {
        (
            format!("http://localhost:{port}/v1"),
            String::new(),
            req.model.unwrap_or_else(|| "local".into()),
        )
    } else if let Some(pid) = &req.provider_id {
        let p = providers::get_provider_inner(&app, pid)?;
        let model = req
            .model
            .or(p.active_model)
            .ok_or_else(|| format!("provider '{}' 没有设置活动模型", pid))?;
        (p.base_url, p.api_key, model)
    } else {
        return Err("请先选择模型来源".into());
    };

    // 2. 项目根（提供了但无效 → 明确报错，不静默降级为纯聊天）
    let project_root = match &req.project_root {
        None => None,
        Some(p) => {
            let pb = PathBuf::from(p);
            if !pb.is_dir() {
                return Err(format!("项目目录不存在: {p}"));
            }
            Some(pb)
        }
    };

    let session_id = req.session_id;
    let user_text = req.user_text;

    // 记忆作用域键：取项目根目录名（稳定、跨机器可复现）。ingest 侧
    // set_thread_scope 与 recall 侧 ProjectBoundProvider 必须用同一个键，
    // 项目记忆才能对上。
    let bound_project = project_root.as_ref().map(|p| {
        p.file_name()
            .map(|n| n.to_string_lossy().into_owned())
            .unwrap_or_else(|| p.to_string_lossy().into_owned())
    });

    // 2.5 技能库：注册发现/读取工具 + system prompt 注入摘要（跨会话复用，建 Thread 时生效）
    let skills_dir = crate::user_skills::app_skills_dir(&app).ok();
    let skills_summary = skills_dir
        .as_deref()
        .map(crate::user_skills::summary_in)
        .unwrap_or_default();

    let dest_changed = threads
        .bind_image_config(&session_id, req.image_recognition, &base_url, &model_name)
        .await?;

    // 2.6 多模态 M5（MULTIMODAL_SPEC §10）：读取生成配置并按需构建 provider。
    // enabled 且配置完整 → build_registry 注册 generate_image；
    // disabled / 缺配置 / 构建失败 → None（不注册、不报错、不阻塞会话）。
    // 配置变更只对新会话生效（已开 Thread 的工具集合冻结）。
    let generation = state
        .generation
        .sync(&directory, crate::agent_generation::load_config(&app));

    // 3. 取或建 Thread（绑定 root/backend/policy；历史保留在消息序列里）
    {
        if dest_changed && threads.threads.contains_key(&session_id) {
            // 模型来源切换：卸载缓存 Thread 后按新 backend 重建。历史消息
            // 已持久化在 SQLite，Thread::load 会恢复；存在持久化错误（未
            // 落盘尾部）时卸载会失败，此处明确报错而不是静默丢消息。
            threads
                .unload(&session_id, &state.persistence_errors)
                .map_err(|e| format!("切换模型来源失败：{e}"))?;
        }
        if !threads.threads.contains_key(&session_id) {
            let policy = host_tool_policy();
            let mut system_prompt = req
                .system_prompt
                .unwrap_or_else(crate::prompts::agent_system_prompt);
            if !skills_summary.is_empty() {
                system_prompt.push_str(&crate::prompts::agent_skills_section());
                for s in &skills_summary {
                    system_prompt.push_str(&format!(
                        "- {}: {}\n",
                        s.name,
                        truncate_desc(&s.description)
                    ));
                }
            }
            let mut config = ThreadConfig::new(session_id.clone(), system_prompt);
            config.project_root = project_root.clone();
            config.max_turns = req.max_turns.unwrap_or(DEFAULT_MAX_TURNS);
            config.auto_compact_tokens = req
                .auto_compact_tokens
                .or(Some(DEFAULT_AUTO_COMPACT_TOKENS))
                .filter(|&t| t > 0);
            config.policy = policy;
            // 记忆集成：注入 MemoryProvider，每个 Run 由 agent-core 自动
            // recall 最新用户输入进 system prompt（REMEMBERED CONTEXT）。
            // memory-off（init 失败）时保持 None，prompt 逐字节不变。
            // 项目绑定：core 的 recall 恒传 scope=None（冻结接口），这里用
            // ProjectBoundProvider 把本次 Run 的项目绑定进搜索范围
            // （全局偏好 + 本项目记忆），项目私有记忆不会跨项目串线。
            if let Some(memory) = state.memory.get() {
                config.memory = Some(Arc::new(astra_memory_provider::ProjectBoundProvider::new(
                    memory.provider(),
                    bound_project.clone(),
                )));
            }
            // Artifact records share the threads sqlite file; a failure to
            // open it keeps artifacts event-only (same degradation as before
            // this wiring) instead of blocking the run.
            if let Ok(artifacts) = threads.artifacts() {
                config.artifact_registry = Some(artifacts);
            }
            config.texts = std::sync::Arc::new(agent_core::TextPack::zh());
            config.thinking_enabled = req.thinking_enabled;
            config.thinking_effort = req.thinking_effort.clone();
            let backend: Arc<dyn ModelBackend> = Arc::new(
                RigBackend::new(base_url, api_key, model_name)
                    .with_image_encoding(image_encoding(req.image_recognition)),
            );
            let bridge: Arc<dyn PermissionBridge> = Arc::new(HubBridge {
                hub: Arc::clone(&state.hub),
                app: app.clone(),
                session_id: session_id.clone(),
                project_root: project_root.clone(),
            });
            let question_bridge: Arc<dyn agent_core::UserQuestionBridge> =
                Arc::new(QuestionBridge {
                    hub: Arc::clone(&state.questions),
                    app: app.clone(),
                    session_id: session_id.clone(),
                });
            let mut registry = build_registry(
                &session_id,
                project_root.as_ref(),
                skills_dir.as_deref(),
                question_bridge,
                settings_search(&app),
                Arc::clone(&state.processes),
                Arc::clone(&state.tasks),
                generation,
                state.memory.get().map(|m| m.engine().clone()),
            );
            // MCP/ACP 装配（MCP_SPEC §14 / ACP_SPEC §11 宿主清单）：
            // ~/.astra/config.toml 的 [mcp]/[acp] 段加载外部能力并注册进本次
            // registry；required 失败阻断启动，其余失败降级（工具不出现）。
            // manager 句柄登记到 state.extensions：运行状态查询 + ACP 懒重连。
            let mut ext = SessionExtensions::default();
            registry = attach_extensions(
                registry,
                &mut config,
                &mut ext,
                &session_id,
                Arc::clone(&state.mcp_tasks),
                Arc::clone(&state.tasks),
            )
            .await?;
            state
                .extensions
                .lock()
                .await
                .insert(session_id.clone(), ext);
            threads
                .load_or_create(config, backend, registry, bridge)
                .await?;
        }
    }

    let (tx, mut rx) = tokio::sync::mpsc::channel::<Envelope>(1024);
    // 多模态 M1：组装用户输入内容块（文本在前、图片随后，保持原始顺序）。
    // /技能名 指定：技能内容只包装文本块（技能匹配从不接触图片数据）。
    let content = build_user_content(&user_text, &req.images, req.skill.as_deref(), &skills_dir)?;
    let thread = threads
        .threads
        .get(&session_id)
        .ok_or_else(|| "Agent thread is unavailable".to_string())?;
    // Apply explicit request settings after restore, before admitting the Run.
    // Await persistence and preserve the error instead of starting with stale settings.
    if req.thinking_enabled.is_some() || req.thinking_effort.is_some() {
        thread
            .set_thinking(req.thinking_enabled, req.thinking_effort)
            .await
            .map_err(|error| error.to_string())?;
    }
    let previous_error = state
        .persistence_errors
        .lock()
        .unwrap_or_else(|e| e.into_inner())
        .remove(&session_id);
    // 3.5 无感快照（隐形保险层）：每个发送前自动锚定 BeforeTurn checkpoint
    // （对话游标 + git 工作区快照），供「回到出错前 / 编辑重发」回退使用。
    // 用户不感知、无常驻 UI；捕获失败按 CHECKPOINT spec §5.3 降级为仅锚点，
    // 打点本身失败只记日志，绝不阻断 Run。
    if project_root.is_some() {
        match threads.checkpoints() {
            Ok(checkpoints) => {
                let provider = agent_core::astra_workspace::GitWorkspaceCheckpointProvider::new();
                if let Err(error) = thread
                    .create_checkpoint(
                        checkpoints,
                        Some(Arc::new(provider)),
                        agent_core::CheckpointKind::BeforeTurn,
                        None,
                        None,
                    )
                    .await
                {
                    eprintln!(
                        "[checkpoint] auto before_turn failed (session {session_id}): {error}"
                    );
                }
            }
            // 快照存储不可用只损失回退能力，不阻断发送
            Err(error) => {
                eprintln!("[checkpoint] store unavailable (session {session_id}): {error}")
            }
        }
    }
    let handle = match thread.try_send_content(content, tx) {
        Ok(handle) => handle,
        Err(error) => {
            if let Some(previous_error) = previous_error {
                state
                    .persistence_errors
                    .lock()
                    .unwrap_or_else(|e| e.into_inner())
                    .insert(session_id.clone(), previous_error);
            }
            return Err(error.to_string());
        }
    };
    *state.active.lock().unwrap_or_else(|e| e.into_inner()) = Some((session_id.clone(), handle));

    // 4. 事件转发任务：Envelope → agent-event + JSONL 日志；Run 结束（tx 全部 drop）后自然退出
    let forward_app = app.clone();
    let active = Arc::clone(&state.active);
    let errors = Arc::clone(&state.persistence_errors);
    let settled_session = session_id.clone();
    // 记忆采集（宿主能力，不暴露给 agent 模型）：Run 结束后把用户输入、
    // 助手最终回答和已完成的工具结果 ingest 进 CME 待处理队列，达到阈值时
    // 压缩会话并提炼长期记忆。提炼是确定性规则，无模型调用；异步 spawn，
    // 不阻塞 Run 结束路径。任何失败只记日志：记忆故障永不阻断 agent（D21）。
    let observe_engine = state.memory.get().map(|m| m.engine().clone());
    let observe_reviewer = state.memory.get().and_then(|m| m.supersede_reviewer());
    let observe_text = user_text.clone();
    let observe_project = bound_project.clone();
    tokio::spawn(async move {
        // 边转发边采集（采集边界见 MemoryRunCollector 文档）。任何失败只记
        // 日志：记忆故障永不阻断 agent（D21）。
        let mut collector = MemoryRunCollector::new();
        while let Some(env) = rx.recv().await {
            collector.observe(&env);
            log_event(&forward_app, &env);
            let _ = forward_app.emit("agent-event", &env);
        }
        // 先结算后记忆：流关闭即释放准入并通知 UI（底部 loading 随渲染
        // 完成立即消失），记忆提交放在 settled 之后后台执行——开 agent
        // 提炼时 process_pending 要调 LLM（可达十几秒），不能让用户看着
        // loading 等记忆。与下一 Run 的记忆提交并发时由 CME 游标 CAS 兜底
        // （冲突只记日志，不丢消息）。
        let persistence_error = errors
            .lock()
            .unwrap_or_else(|e| e.into_inner())
            .get(&settled_session)
            .cloned();
        *active.lock().unwrap_or_else(|e| e.into_inner()) = None;
        // Release admission before notifying the UI that another send is safe.
        drop(threads);
        let _ = forward_app.emit(
            "agent-run-settled",
            json!({
                "session_id": settled_session,
                "persistence_error": persistence_error,
            }),
        );
        if let Some(engine) = observe_engine {
            // 项目绑定：本 Run 提炼的记忆落到对应项目作用域，避免跨项目串线。
            // best-effort：失败只记日志。键与 recall 侧 ProjectBoundProvider
            // 同源（bound_project），否则项目记忆对不上。
            let now_ms = std::time::SystemTime::now()
                .duration_since(std::time::UNIX_EPOCH)
                .map(|d| d.as_millis() as u64)
                .unwrap_or(0);
            commit_collected_memory(
                &engine,
                collector,
                &settled_session,
                observe_project.as_deref(),
                &observe_text,
                now_ms,
                observe_reviewer.as_deref(),
            )
            .await;
        }
    });

    Ok(session_id)
}

/// 取消当前 Run（停止不再启动新工具调用；进行中的模型流被中断）
#[tauri::command]
pub fn agent_cancel(state: State<'_, AgentState>) {
    // Do not clear active until the receiver has drained the persistence tail.
    let active = state.active.lock().unwrap_or_else(|e| e.into_inner());
    if let Some((_, handle)) = active.as_ref() {
        handle.cancel();
        state.hub.drop_all();
        state.questions.drop_all();
    }
}

/// 回传用户对某次权限请求的决定（批准只对该 tool_call_id 生效）
#[tauri::command]
pub fn agent_permission_decide(
    state: State<'_, AgentState>,
    tool_call_id: String,
    decision: Decision,
) -> Result<(), String> {
    state.hub.decide(&tool_call_id, decision)
}

/// 回传用户对 ask_user 问题卡的回答（None = 取消/拒绝）
#[tauri::command]
pub fn agent_question_answer(
    state: State<'_, AgentState>,
    question_id: String,
    answer: Option<String>,
) -> Result<(), String> {
    state.questions.answer(&question_id, answer)
}

/// 多级放权：设置自动批准级别。
/// - off：全部弹确认卡
/// - project：当前项目目录内的路径类操作自动批准
/// - workspace：全部已配置工作空间内的路径类操作自动批准
/// - all：所有确认类操作自动批准（含 shell 命令）
/// Deny 类硬规则（未注册工具、提权命令、路径逃逸）不受任何级别影响；
/// shell 等无路径参数的操作仅在 all 级别自动批准。
#[tauri::command]
pub fn agent_set_auto_mode(state: State<'_, AgentState>, mode: String) -> Result<(), String> {
    let parsed = AutoMode::parse(&mode)?;
    state.hub.set_auto_mode(parsed);
    Ok(())
}

/// Unload memory only. Cancel and wait for agent-run-settled before resetting.
#[tauri::command]
pub async fn agent_reset_session(
    state: State<'_, AgentState>,
    session_id: String,
) -> Result<(), String> {
    let mut threads = state.threads.try_lock().map_err(|_| {
        "Agent is running or draining; cancel and wait before unloading".to_string()
    })?;
    threads.unload(&session_id, &state.persistence_errors)
}

/// 扩展开关变更后批量卸载内存 Thread：所有空闲会话在下次对话时按新配置
/// 重新装配 MCP/ACP。正在运行的会话持有 threads 锁 → 整体失败，由前端
/// 提示「将在下次对话时生效」；带未落盘错误的会话跳过（unload 返回 Err），
/// 不影响其余会话。
#[derive(serde::Serialize)]
pub struct UnloadAllReport {
    pub unloaded: usize,
    pub skipped: Vec<String>,
}

#[tauri::command]
pub async fn agent_unload_all_sessions(
    state: State<'_, AgentState>,
) -> Result<UnloadAllReport, String> {
    let mut threads = state.threads.try_lock().map_err(|_| {
        "Agent is running or draining; the change applies when that conversation continues"
            .to_string()
    })?;
    let ids: Vec<String> = threads.threads.keys().cloned().collect();
    let mut report = UnloadAllReport {
        unloaded: 0,
        skipped: Vec::new(),
    };
    for id in ids {
        match threads.unload(&id, &state.persistence_errors) {
            Ok(()) => report.unloaded += 1,
            Err(e) => report.skipped.push(format!("{id}: {e}")),
        }
    }
    Ok(report)
}

// ===== 预览：从后台任务日志发现本地服务地址并打开预览窗口 =====

/// 可预览任务信息（前端展示用）
#[derive(serde::Serialize)]
pub struct AgentPreviewTask {
    pub task_id: u64,
    pub command: String,
    pub status: String,
    pub exit_code: Option<i32>,
    pub urls: Vec<String>,
}

/// 列出后台任务及从其日志中检测到的本地服务地址
#[tauri::command]
pub async fn agent_preview_tasks(
    state: State<'_, AgentState>,
) -> Result<Vec<AgentPreviewTask>, String> {
    let urls_of = |id: u64| {
        state
            .processes
            .full_log(id)
            .map(|log| agent_core::shell_background::detect_local_urls(&log))
            .unwrap_or_default()
    };
    Ok(state
        .processes
        .snapshot()
        .into_iter()
        .map(|t| AgentPreviewTask {
            task_id: t.id,
            command: t.command.clone(),
            urls: urls_of(t.id),
            status: t.status,
            exit_code: t.exit_code,
        })
        .collect())
}

/// 为后台任务打开预览窗口：优先用显式传入的 url，否则用日志中检测到的第一个。
/// 同一 url 复用已存在的窗口（避免点出一片窗口海）。
#[tauri::command]
pub async fn agent_open_preview(
    app: AppHandle,
    state: State<'_, AgentState>,
    task_id: u64,
    url: Option<String>,
) -> Result<String, String> {
    let target = match url {
        Some(u) if !u.trim().is_empty() => u,
        _ => {
            let log = state
                .processes
                .full_log(task_id)
                .ok_or_else(|| format!("后台任务不存在: {task_id}"))?;
            agent_core::shell_background::detect_local_urls(&log)
                .into_iter()
                .next()
                .ok_or_else(|| {
                    "日志中未检测到本地服务地址（服务可能还没就绪，稍后再试）".to_string()
                })?
        }
    };

    // 同 url 复用现有窗口并聚焦；label 由 url 的简单 FNV-1a 摘要派生（稳定且唯一性足够）
    let mut hash: u64 = 0xcbf29ce484222325;
    for b in target.as_bytes() {
        hash ^= *b as u64;
        hash = hash.wrapping_mul(0x100000001b3);
    }
    let label = format!("preview-{hash:016x}");
    if let Some(existing) = app.get_webview_window(&label) {
        let _ = existing.set_focus();
        return Ok(existing.label().to_string());
    }

    use tauri::WebviewUrl;
    use tauri::WebviewWindowBuilder;
    let parsed: tauri::Url = target
        .parse()
        .map_err(|e| format!("无效的服务地址 {target}: {e}"))?;
    let title = format!(
        "预览 · {}",
        state
            .processes
            .snapshot()
            .into_iter()
            .find(|t| t.id == task_id)
            .map(|t| t.command.chars().take(40).collect::<String>())
            .unwrap_or_else(|| format!("任务 {task_id}"))
    );
    WebviewWindowBuilder::new(&app, label.clone(), WebviewUrl::External(parsed))
        .title(title)
        .inner_size(1200.0, 800.0)
        .build()
        .map_err(|e| format!("打开预览窗口失败: {e}"))?;
    Ok(label)
}

/// 当前 Agent 是否有进行中的 Run（页面刷新后恢复状态用）
#[derive(Debug, Serialize)]
pub struct AgentStatus {
    pub running: bool,
    pub session_id: Option<String>,
    pub persistence_errors: HashMap<String, String>,
}

#[tauri::command]
pub fn agent_status(state: State<'_, AgentState>) -> AgentStatus {
    let active = state.active.lock().unwrap_or_else(|e| e.into_inner());
    AgentStatus {
        running: active.is_some(),
        session_id: active.as_ref().map(|(id, _)| id.clone()),
        persistence_errors: state
            .persistence_errors
            .lock()
            .unwrap_or_else(|e| e.into_inner())
            .clone(),
    }
}

// ===== Artifacts（Phase 4，ARTIFACT_SPEC：location-based 元数据登记）=====

/// 前端可见的 artifact 记录。registry 只证明「写入时产物存在于 location」，
/// 不保证文件现在仍在（spec §1：无内容快照/校验/回滚）。
#[derive(Debug, Serialize)]
pub struct AgentArtifactInfo {
    pub id: String,
    pub thread_id: String,
    /// 逻辑身份，版本链锚点（v1 = location）
    pub artifact_key: String,
    pub kind: String,
    /// 写入时刻的路径（相对项目根），不是存活声明
    pub location: String,
    pub preview: Option<String>,
    pub metadata: Value,
    pub version: u32,
    pub previous_id: Option<String>,
    pub created_at: u64,
    pub status: String,
}

fn artifact_info(record: agent_core::artifact_registry::ArtifactRecord) -> AgentArtifactInfo {
    AgentArtifactInfo {
        id: record.id,
        thread_id: record.thread_id,
        artifact_key: record.artifact_key,
        kind: serde_json::to_value(record.kind)
            .ok()
            .and_then(|v| v.as_str().map(str::to_owned))
            .unwrap_or_else(|| "generic".into()),
        location: record.location,
        preview: record.preview,
        metadata: record.metadata,
        version: record.version,
        previous_id: record.previous_id,
        created_at: record.created_at,
        status: serde_json::to_value(record.status)
            .ok()
            .and_then(|v| v.as_str().map(str::to_owned))
            .unwrap_or_else(|| "ready".into()),
    }
}

/// 列出一个会话的全部 artifact 记录（created_at 升序）。
/// 版本链由前端按 artifact_key 自行分组（registry 无分页，spec §4）。
#[tauri::command]
pub async fn agent_artifact_list(
    app: AppHandle,
    state: State<'_, AgentState>,
    session_id: String,
) -> Result<Vec<AgentArtifactInfo>, String> {
    let mut threads = state.threads.lock().await;
    let directory = app.path().app_data_dir().map_err(|e| e.to_string())?;
    threads
        .open(&directory, Arc::clone(&state.persistence_errors))
        .await?;
    let artifacts = threads.artifacts()?;
    drop(threads);
    let records = artifacts
        .list(&session_id)
        .await
        .map_err(|e| e.to_string())?;
    Ok(records.into_iter().map(artifact_info).collect())
}

// ===== Checkpoint（Phase 11.6，CHECKPOINT_SPEC §7/§8）=====

/// 前端可见的 checkpoint 摘要（不含 workspace/artifact 内部结构）
#[derive(Debug, Serialize)]
pub struct AgentCheckpointInfo {
    pub id: String,
    pub kind: String,
    pub state: String,
    pub thread_seq: u64,
    pub created_at: u64,
    pub parent_id: Option<String>,
    pub has_workspace: bool,
}

fn checkpoint_info(record: agent_core::CheckpointRecord) -> AgentCheckpointInfo {
    let kind = serde_json::to_value(record.kind)
        .ok()
        .and_then(|v| v.as_str().map(str::to_owned))
        .unwrap_or_default();
    let state = serde_json::to_value(record.state)
        .ok()
        .and_then(|v| v.as_str().map(str::to_owned))
        .unwrap_or_default();
    AgentCheckpointInfo {
        id: record.id,
        kind,
        state,
        thread_seq: record.thread_seq,
        created_at: record.created_at,
        parent_id: record.parent_id,
        has_workspace: record.workspace.is_some(),
    }
}

/// 列出会话的 checkpoint（created_at 升序）。会话未加载也能列出（游标在 SQLite）。
#[tauri::command]
pub async fn agent_checkpoint_list(
    app: AppHandle,
    state: State<'_, AgentState>,
    session_id: String,
) -> Result<Vec<AgentCheckpointInfo>, String> {
    let mut threads = state.threads.lock().await;
    let directory = app.path().app_data_dir().map_err(|e| e.to_string())?;
    threads
        .open(&directory, Arc::clone(&state.persistence_errors))
        .await?;
    let checkpoints = threads.checkpoints()?;
    drop(threads);
    let records = checkpoints
        .list(&session_id)
        .await
        .map_err(|e| e.to_string())?;
    Ok(records.into_iter().map(checkpoint_info).collect())
}

/// 在当前会话上创建 checkpoint（默认 Manual）。游标取最后一次落盘的消息序号，
/// workspace 捕获失败不阻断（spec §5.3）。
#[tauri::command]
pub async fn agent_checkpoint_create(
    app: AppHandle,
    state: State<'_, AgentState>,
    session_id: String,
    kind: Option<String>,
) -> Result<AgentCheckpointInfo, String> {
    let mut threads = state.threads.lock().await;
    let directory = app.path().app_data_dir().map_err(|e| e.to_string())?;
    threads
        .open(&directory, Arc::clone(&state.persistence_errors))
        .await?;
    let checkpoints = threads.checkpoints()?;
    let thread = threads
        .threads
        .get(&session_id)
        .ok_or_else(|| "会话未加载，无法创建 checkpoint".to_string())?;
    let parsed_kind = match kind.as_deref() {
        Some("before_turn") => agent_core::CheckpointKind::BeforeTurn,
        Some("stage") => agent_core::CheckpointKind::Stage,
        _ => agent_core::CheckpointKind::Manual,
    };
    let provider = agent_core::astra_workspace::GitWorkspaceCheckpointProvider::new();
    let record = thread
        .create_checkpoint(
            checkpoints,
            Some(Arc::new(provider)),
            parsed_kind,
            None,
            None,
        )
        .await
        .map_err(|e| e.to_string())?;
    Ok(checkpoint_info(record))
}

/// 恢复到指定 checkpoint：对话回退到锚点（含工作区还原），返回回退后的
/// 完整消息序列供前端重建会话视图。
#[derive(Debug, Serialize)]
pub struct AgentCheckpointRestoreResult {
    pub checkpoint_id: String,
    pub thread_seq: u64,
    pub workspace_restored: bool,
    pub artifact_report: Vec<agent_core::ArtifactReconcileEntry>,
    pub messages: Vec<ChatMessage>,
}

#[tauri::command]
pub async fn agent_checkpoint_restore(
    app: AppHandle,
    state: State<'_, AgentState>,
    session_id: String,
    checkpoint_id: String,
) -> Result<AgentCheckpointRestoreResult, String> {
    // 进行中的 Run 会持有 threads 生命周期锁；先给出明确提示而不是静默排队。
    {
        let active = state.active.lock().unwrap_or_else(|e| e.into_inner());
        if active.is_some() {
            return Err("Agent 正在运行，请先取消当前任务再恢复 checkpoint".into());
        }
    }
    let mut threads = state.threads.lock().await;
    let directory = app.path().app_data_dir().map_err(|e| e.to_string())?;
    threads
        .open(&directory, Arc::clone(&state.persistence_errors))
        .await?;
    let checkpoints = threads.checkpoints()?;
    let thread = threads
        .threads
        .get(&session_id)
        .ok_or_else(|| "会话未加载，无法恢复 checkpoint".to_string())?;
    let provider = agent_core::astra_workspace::GitWorkspaceCheckpointProvider::new();
    let report = thread
        .restore_checkpoint(checkpoints, &checkpoint_id, Some(Arc::new(provider)), None)
        .await
        .map_err(|e| e.to_string())?;
    let messages = thread.messages();
    let _ = app.emit(
        "agent-checkpoint-restored",
        json!({ "session_id": session_id, "checkpoint_id": checkpoint_id }),
    );
    Ok(AgentCheckpointRestoreResult {
        checkpoint_id: report.checkpoint_id,
        thread_seq: report.thread_seq,
        workspace_restored: report.workspace_restored,
        artifact_report: report.artifact_report,
        messages,
    })
}

// 保留 ChatMessage / Role 引用（类型用于将来直接序列化快照）
#[allow(dead_code)]
fn _assert_types(_: ChatMessage, _: Role) {}

/// 回退结果：回退后的锚点游标、是否还原了工作区、回退后的完整消息序列
/// （前端用它重建会话视图，同 agent_checkpoint_restore.messages）。
#[derive(Debug, Serialize)]
pub struct AgentRewindResult {
    pub thread_seq: u64,
    pub workspace_restored: bool,
    pub messages: Vec<ChatMessage>,
}

/// 回退到倒数第 N 条用户消息之前（1 = 最后一条）。
///
/// 无感快照层的恢复原语，服务两个入口：
/// - 失败卡片「重试 / 编辑后重发」：`restore_workspace = true`，回退对话并
///   尝试把项目文件还原到该轮发送前的快照（找不到快照时降级为仅回退对话）；
/// - 历史消息「编辑重发」：`restore_workspace = false`，只回退对话（成功轮次
///   的文件产物属于用户工作，不能随对话回退被静默覆盖）。
///
/// 工作区还原走最近一次自动锚定的 BeforeTurn checkpoint（agent_start_run
/// 每次发送前创建）；同一锚点的快照记录可能因早先的回退被清理，此时降级。
#[tauri::command]
pub async fn agent_rewind_to_user_message(
    app: AppHandle,
    state: State<'_, AgentState>,
    session_id: String,
    turns_back: u32,
    restore_workspace: bool,
) -> Result<AgentRewindResult, String> {
    // 进行中的 Run 会持有 threads 生命周期锁；先给出明确提示而不是静默排队。
    {
        let active = state.active.lock().unwrap_or_else(|e| e.into_inner());
        if active.is_some() {
            return Err("Agent 正在运行，请先取消当前任务再回退".into());
        }
    }
    let mut threads = state.threads.lock().await;
    let directory = app.path().app_data_dir().map_err(|e| e.to_string())?;
    threads
        .open(&directory, Arc::clone(&state.persistence_errors))
        .await?;
    let thread = threads
        .threads
        .get(&session_id)
        .ok_or_else(|| "会话未加载，无法回退".to_string())?;

    // 定位目标用户消息前的锚点（候选消息不足时明确报错）。
    let anchor = thread
        .anchor_before_user_turn(turns_back)
        .await
        .map_err(|e| e.to_string())?;

    // 工作区还原：找锚点上的快照记录走完整 restore（对话回退 + 文件还原）。
    let mut workspace_restored = false;
    if restore_workspace {
        let checkpoints = threads.checkpoints()?;
        let records = checkpoints
            .list(&session_id)
            .await
            .map_err(|e| e.to_string())?;
        let record = records.iter().find(|r| r.thread_seq == anchor);
        if let Some(record) = record {
            let provider = agent_core::astra_workspace::GitWorkspaceCheckpointProvider::new();
            match thread
                .restore_checkpoint(checkpoints, &record.id, Some(Arc::new(provider)), None)
                .await
            {
                Ok(report) => workspace_restored = report.workspace_restored,
                Err(error) => {
                    // 快照还原失败不阻断回退：对话本身必须能回去。
                    eprintln!(
                        "[checkpoint] workspace restore failed (session {session_id}): {error}"
                    );
                    thread.rewind_to(anchor).await.map_err(|e| e.to_string())?;
                }
            }
        }
    }

    // 仅对话回退（或快照缺失 / 还原失败后的降级路径）。
    if !workspace_restored {
        thread.rewind_to(anchor).await.map_err(|e| e.to_string())?;
    }

    let _ = app.emit(
        "agent-checkpoint-restored",
        json!({ "session_id": session_id, "checkpoint_id": null }),
    );
    Ok(AgentRewindResult {
        thread_seq: anchor,
        workspace_restored,
        messages: thread.messages(),
    })
}

#[cfg(test)]
mod tests {
    use super::*;

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
            0x89, 0x50, 0x4E, 0x47, 0x0D, 0x0A, 0x1A, 0x0A, 0x00, 0x00, 0x00, 0x0D, 0x49, 0x48,
            0x44, 0x52, 0x00, 0x00, 0x00, 0x01, 0x00, 0x00, 0x00, 0x01, 0x08, 0x02, 0x00, 0x00,
            0x00, 0x90, 0x77, 0x53, 0xDE, 0x00, 0x00, 0x00, 0x0C, 0x49, 0x44, 0x41, 0x54, 0x08,
            0xD7, 0x63, 0xF8, 0xCF, 0xC0, 0x00, 0x00, 0x03, 0x01, 0x01, 0x00, 0x18, 0xDD, 0x8D,
            0xB0, 0x00, 0x00, 0x00, 0x00, 0x49, 0x45, 0x4E, 0x44, 0xAE, 0x42, 0x60, 0x82,
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
        let engine =
            std::sync::Arc::new(MemoryEngine::new(CmeSqliteStore::open(&db, &key).unwrap()));
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
        const REAL_RUN: &str = include_str!("../tests/fixtures/cme_replay_run.jsonl");

        let dir = tempfile::tempdir().unwrap();
        let db = dir.path().join("memory-cme.sqlite");
        let key = [9u8; 32];
        let engine =
            std::sync::Arc::new(MemoryEngine::new(CmeSqliteStore::open(&db, &key).unwrap()));
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
    /// 返回（助手最终回答，采集器）。权限桥全放行（等价 all 级放权）。
    #[allow(clippy::too_many_arguments)]
    async fn real_turn(
        threads: &mut crate::agent_threads::AgentThreads,
        session: &str,
        project_root: &Path,
        memory: Arc<dyn agent_core::memory::MemoryProvider>,
        base_url: &str,
        api_key: &str,
        model_name: &str,
        user_text: &str,
    ) -> (String, MemoryRunCollector) {
        // 与 agent_start_run 相同的策略形状：本测试只需 read_file（默认
        // Allow）+ shell_execute（Confirm，经 AutoBridge 全放行）。
        let mut policy = ToolPolicy::default();
        policy.allowed_readonly.push("find_path".into());
        policy.confirm_tools.push("shell_execute".into());

        let mut config =
            ThreadConfig::new(session.to_string(), crate::prompts::agent_system_prompt());
        config.project_root = Some(project_root.to_path_buf());
        config.max_turns = 6;
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
        let wait = async {
            while let Some(env) = rx.recv().await {
                if let agent_core::RunEvent::RunCompleted { final_text: text } = &env.event {
                    final_text = text.clone();
                }
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
        (final_text, collector)
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

        let mut threads = crate::agent_threads::AgentThreads::default();
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
        let (final1, collector1) = real_turn(
            &mut threads,
            "sess-real-run-1",
            project.path(),
            memory1,
            &base_url,
            &api_key,
            &model_name,
            prompt1,
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
        let (final2, collector2) = real_turn(
            &mut threads,
            "sess-real-run-2",
            project.path(),
            memory2,
            &base_url,
            &api_key,
            &model_name,
            prompt2,
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
}
