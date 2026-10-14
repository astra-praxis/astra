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

use agent_core::skill_tools::truncate_desc;
use agent_core::{
    ChatMessage, Decision, Envelope, ModelBackend, PermissionBridge, PermissionRequest, RigBackend,
    Role, RunHandle, ThreadConfig, ToolPolicy, ToolRegistry, DEFAULT_AUTO_COMPACT_TOKENS,
    DEFAULT_MAX_TURNS,
};
use serde::{Deserialize, Serialize};
use serde_json::{json, Value};
use std::path::{Path, PathBuf};
use tauri::{AppHandle, Emitter, Manager, State};
use tokio::sync::oneshot;

use crate::providers;

// ===== 事件日志（JSONL，一行一个 Envelope，供排查对话问题） =====

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

/// 等待用户决定的权限请求集合（tool_call_id → oneshot 发送端）
#[derive(Default)]
pub struct PermissionHub {
    pending: Mutex<HashMap<String, oneshot::Sender<Decision>>>,
    /// 「放权模式」：开启后所有 Confirm 类工具直接 Allow，不再弹确认卡。
    /// Deny 类硬规则（未注册工具、提权命令、路径逃逸）不受影响，仍在执行层拦截
    auto_all: std::sync::atomic::AtomicBool,
}

impl PermissionHub {
    pub fn set_auto_all(&self, enabled: bool) {
        use std::sync::atomic::Ordering;
        self.auto_all.store(enabled, Ordering::Relaxed);
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
/// 开启「放权模式」时所有 Confirm 直接 Allow 跳过弹窗（Deny 不经此桥，不受影响）。
struct HubBridge(Arc<PermissionHub>);

#[async_trait::async_trait]
impl PermissionBridge for HubBridge {
    async fn request(&self, req: PermissionRequest) -> Decision {
        use std::sync::atomic::Ordering;
        if self.0.auto_all.load(Ordering::Relaxed) {
            return Decision::Allow;
        }
        // tool_call_id 由 run_loop 生成并已通过 PermissionRequested 事件告知前端
        let id = req.tool_call_id;
        let rx = self.0.register(id);
        match rx.await {
            Ok(d) => d,
            Err(_) => Decision::Deny,
        }
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
}

impl Default for AgentState {
    fn default() -> Self {
        Self {
            hub: Arc::new(PermissionHub::default()),
            questions: Arc::new(QuestionHub::default()),
            active: Arc::new(Mutex::new(None)),
            threads: Arc::new(tokio::sync::Mutex::new(
                crate::agent_threads::AgentThreads::default(),
            )),
            persistence_errors: Arc::new(Mutex::new(HashMap::new())),
            session_files: tokio::sync::Mutex::new(std::collections::HashSet::new()),
            processes: Arc::new(agent_core::shell_background::ProcessManager::new()),
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
/// 从应用设置读取 search_web 的 SearXNG 实例地址（未配置 → None，用 DDG）。
/// 地址会 trim 并去掉末尾的 /，再拼 /search?q=&format=json。
fn settings_search_endpoint(app: &AppHandle) -> Option<String> {
    let path = crate::get_settings_path(app).ok()?;
    let content = std::fs::read_to_string(path).ok()?;
    let v: serde_json::Value = serde_json::from_str(&content).ok()?;
    v.get("search_endpoint")
        .and_then(|s| s.as_str())
        .map(|s| s.trim().trim_end_matches('/').to_string())
        .filter(|s| !s.is_empty())
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

fn build_registry(
    project_root: Option<&PathBuf>,
    skills_dir: Option<&Path>,
    question_bridge: Arc<dyn agent_core::UserQuestionBridge>,
    search_endpoint: Option<String>,
    processes: Arc<agent_core::shell_background::ProcessManager>,
) -> ToolRegistry {
    // 网络工具 + ask_user 不依赖项目根，纯聊天也可用
    let reg = agent_core::net_tools::register_with_search(ToolRegistry::new(), search_endpoint)
        .register(agent_core::AskUserTool::new(question_bridge));
    let reg = match skills_dir {
        Some(dir) => agent_core::skill_tools::register(reg, dir),
        None => reg,
    };
    match project_root {
        Some(root) => {
            let reg = agent_core::fs_basic::register(reg, root);
            let reg = agent_core::doc_tools::register(reg, root);
            let reg = agent_core::shell_tool::register(reg, root);
            // Background task management (dev servers / watchers): manager is
            // host-owned and shared across sessions, so tasks outlive a chat.
            let reg = agent_core::shell_background::register(reg, root, processes);
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
        let manager = astra_mcp::McpManager::new(cfg.mcp.to_manager_configs())
            .map_err(|e| format!("MCP 配置无效: {e}"))?;
        let manager = std::sync::Arc::new(manager);
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
        // manager 由 rule_provider 持有保活；权限规则按发现结果逐工具生成，
        // 未列出的默认 Confirm（MCP_SPEC §9.2），经 ThreadConfig 进入权限管线。
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
        // manager 随作用域释放：adapter 持有连接，调用不受影响；
        // 仅崩溃降级/懒重连不再可用（v1 简化）。
    }

    Ok(registry)
}

fn default_system_prompt() -> String {
    "你是星野的 Agent 助手，可以读取、检索和写入项目内文件来完成任务。\
     用户消息中的「@相对路径」是对当前项目内文件的引用，处理前可先用 read_file 查看内容。\
     检索文件内容用 search_files（正则，可配 include 文件名过滤），按文件名找文件用 find_path（glob）。\
     处理 .docx 等二进制文档时，先用 extract_document 转成 Markdown，\
     再用 read_file 的 offset/limit 分段阅读（大文件不要一次全读，按截断提示续读）。\
     修改已有文件优先用 edit_file（old_text/new_text 精确替换），新建文件或整体重写用 write_file；\
     复制/移动/建目录用 copy_path / move_path / create_directory（目标已存在会报错，不会覆盖）。\
     需要交付 docx 时用 build_docx（会请求用户确认）。\
     需要运行命令（pandoc/python/构建工具等）时用 shell_execute（在项目根目录执行，每次都请求用户确认）。\
     需要启动长驻进程（dev server / watch / 长任务）时用 shell_background 后台启动，\
     然后用 shell_status 增量查看输出（可从中发现监听端口和构建报错并自行修复），不再需要时用 shell_kill 停止。\
     需要查阅网页资料时用 fetch 抓取页面（返回 Markdown），需要实时信息时用 search_web 搜索后再 fetch 具体页面。\
     任务需要用户决定、确认或补充信息时用 ask_user 提问（问题会以卡片形式展示给用户）。\
     系统提示末尾列出了用户的技能库：任务与某技能相关时，先用 read_skill 读取该技能说明再严格按其执行；\
     用户消息中出现「/技能名」时优先遵循对应技能。\
     执行写操作前先用一句话说明要做什么。回答使用中文，保持简洁。"
        .into()
}

fn image_encoding(enabled: bool) -> agent_core::model::ImageEncoding {
    if enabled {
        agent_core::model::ImageEncoding::UserImageUrl
    } else {
        agent_core::model::ImageEncoding::Disabled
    }
}

// ===== 命令 =====

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

    // 2.5 技能库：注册发现/读取工具 + system prompt 注入摘要（跨会话复用，建 Thread 时生效）
    let skills_dir = crate::user_skills::app_skills_dir(&app).ok();
    let skills_summary = skills_dir
        .as_deref()
        .map(crate::user_skills::summary_in)
        .unwrap_or_default();

    threads
        .bind_image_config(&session_id, req.image_recognition, &base_url, &model_name)
        .await?;

    // 3. 取或建 Thread（绑定 root/backend/policy；历史保留在消息序列里）
    {
        if !threads.threads.contains_key(&session_id) {
            // 在默认策略上追加 pandoc 工具：转换是只读产物生成（Allow），
            // 构建 docx 会覆盖/创建交付文件（Confirm）
            let mut policy = ToolPolicy::default();
            // fs_tools（Zed 移植）：find_path 只读；edit/copy/move/mkdir 产生变更走 Confirm
            policy.allowed_readonly.push("find_path".into());
            // pandoc 转换是只读产物生成（Allow）；ask_user / search_web 无副作用（Allow）
            policy.allowed_readonly.push("extract_document".into());
            policy.allowed_readonly.push("ask_user".into());
            policy.allowed_readonly.push("search_web".into());
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
            // fetch 涉及外部网络访问，走确认弹窗（SSRF 门在工具内部始终生效）
            policy.confirm_tools.push("fetch".into());
            let mut system_prompt = req.system_prompt.unwrap_or_else(default_system_prompt);
            if !skills_summary.is_empty() {
                system_prompt.push_str("\n\n## 可用技能\n以下是用户的技能库（用户可用 /技能名 直接调用）。\
                    如果当前任务与某个技能相关，先用 read_skill 读取该技能的完整说明，然后严格按说明执行：\n");
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
            let bridge: Arc<dyn PermissionBridge> = Arc::new(HubBridge(Arc::clone(&state.hub)));
            let question_bridge: Arc<dyn agent_core::UserQuestionBridge> =
                Arc::new(QuestionBridge {
                    hub: Arc::clone(&state.questions),
                    app: app.clone(),
                    session_id: session_id.clone(),
                });
            let mut registry = build_registry(
                project_root.as_ref(),
                skills_dir.as_deref(),
                question_bridge,
                settings_search_endpoint(&app),
                Arc::clone(&state.processes),
            );
            // MCP/ACP 装配（MCP_SPEC §14 / ACP_SPEC §11 宿主清单）：
            // ~/.astra/config.toml 的 [mcp]/[acp] 段加载外部能力并注册进本次
            // registry；required 失败阻断启动，其余失败降级（工具不出现）。
            registry = attach_extensions(registry, &mut config).await?;
            threads
                .load_or_create(config, backend, registry, bridge)
                .await?;
        }
    }

    let (tx, mut rx) = tokio::sync::mpsc::channel::<Envelope>(1024);
    // /技能名 指定：技能内容注入本次用户消息（Thread 按会话复用，不能靠 system prompt）
    let user_text = match &req.skill {
        None => user_text,
        Some(skill_name) => {
            let dir = skills_dir
                .as_ref()
                .ok_or_else(|| format!("技能库不可用，无法加载技能 '{skill_name}'"))?;
            let content = crate::user_skills::read_skill_content(dir, skill_name, None)?;
            format!(
                "【用户通过 /{skill_name} 指定了技能，请先阅读以下技能说明并严格按其执行】\n\n{content}\n\n---\n\n用户任务：{user_text}"
            )
        }
    };
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
    let handle = match thread.try_send(user_text, tx) {
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
    tokio::spawn(async move {
        while let Some(env) = rx.recv().await {
            log_event(&forward_app, &env);
            let _ = forward_app.emit("agent-event", &env);
        }
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

/// 放权模式：开启后所有 Confirm 类工具（写文件 / 编辑 / shell / fetch / build_docx 等）
/// 直接 Allow，不再弹确认卡；Deny 类硬规则不受影响，仍在执行层拦截。
#[tauri::command]
pub fn agent_set_auto_all(state: State<'_, AgentState>, enabled: bool) {
    state.hub.set_auto_all(enabled);
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

#[cfg(test)]
mod tests {
    use super::*;

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
}
