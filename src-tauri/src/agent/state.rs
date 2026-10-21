use std::collections::HashMap;
use std::sync::{Arc, Mutex};

use agent_core::RunHandle;


use super::*;
// ===== 全局状态 =====

/// One live Run per session (SESSION_RUNTIME_SPEC P3). `generation` fences
/// cleanup: an interrupt-send starts a newer Run for the same session while
/// the old Run's forwarder is still draining, and only the forwarder whose
/// generation matches the stored entry may release the slot.
#[derive(Clone)]
pub(crate) struct ActiveRun {
    pub(crate) generation: u64,
    pub(crate) handle: RunHandle,
}

pub(crate) static RUN_SEQ: std::sync::atomic::AtomicU64 = std::sync::atomic::AtomicU64::new(0);

pub struct AgentState {
    pub(crate) hub: Arc<PermissionHub>,
    pub(crate) questions: Arc<QuestionHub>,
    pub(crate) active: Arc<Mutex<HashMap<String, ActiveRun>>>,
    pub(crate) threads: Arc<tokio::sync::Mutex<crate::agent::threads::AgentThreads>>,
    pub(crate) persistence_errors: crate::agent::threads::PersistenceErrors,
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
    pub(crate) memory: Arc<std::sync::OnceLock<crate::memory::host::MemoryHost>>,
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
            active: Arc::new(Mutex::new(HashMap::new())),
            threads: Arc::new(tokio::sync::Mutex::new(
                crate::agent::threads::AgentThreads::default(),
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
