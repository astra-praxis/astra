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
//! - `agent_reset_session` 显式丢弃会话上下文

use std::collections::HashMap;
use std::sync::{Arc, Mutex};

use agent_core::skill_tools::truncate_desc;
use agent_core::{
    ChatMessage, Decision, Envelope, ModelBackend, PermissionBridge, PermissionRequest, RigBackend,
    Role, RunHandle, Thread, ThreadConfig, ToolPolicy, ToolRegistry, DEFAULT_AUTO_COMPACT_TOKENS,
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
    active: Mutex<Option<RunHandle>>,
    threads: Mutex<HashMap<String, Thread>>,
}

impl Default for AgentState {
    fn default() -> Self {
        Self {
            hub: Arc::new(PermissionHub::default()),
            questions: Arc::new(QuestionHub::default()),
            active: Mutex::new(None),
            threads: Mutex::new(HashMap::new()),
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
            // fs_tools（Zed 移植）：搜索/编辑/移动类
            agent_core::fs_tools::register(reg, root)
        }
        None => reg,
    }
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
     需要查阅网页资料时用 fetch 抓取页面（返回 Markdown），需要实时信息时用 search_web 搜索后再 fetch 具体页面。\
     任务需要用户决定、确认或补充信息时用 ask_user 提问（问题会以卡片形式展示给用户）。\
     系统提示末尾列出了用户的技能库：任务与某技能相关时，先用 read_skill 读取该技能说明再严格按其执行；\
     用户消息中出现「/技能名」时优先遵循对应技能。\
     执行写操作前先用一句话说明要做什么。回答使用中文，保持简洁。"
        .into()
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
}

/// 启动一次 Run（多轮对话中的一轮）。事件经 `agent-event` 流出。
/// 返回会话 id。
#[tauri::command]
pub async fn agent_start_run(
    app: AppHandle,
    state: State<'_, AgentState>,
    req: AgentStartReq,
) -> Result<String, String> {
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

    // 3. 取或建 Thread（绑定 root/backend/policy；历史保留在消息序列里）
    {
        let mut threads = state.threads.lock().unwrap();
        if !threads.contains_key(&session_id) {
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
            let config = ThreadConfig {
                session_id: session_id.clone(),
                system_prompt,
                project_root: project_root.clone(),
                temperature: None,
                max_tokens: None,
                max_turns: req.max_turns.unwrap_or(DEFAULT_MAX_TURNS),
                auto_compact_tokens: req
                    .auto_compact_tokens
                    .or(Some(DEFAULT_AUTO_COMPACT_TOKENS))
                    .filter(|&t| t > 0),
                policy,
                texts: std::sync::Arc::new(agent_core::TextPack::zh()),
            };
            let backend: Arc<dyn ModelBackend> =
                Arc::new(RigBackend::new(base_url, api_key, model_name));
            let bridge: Arc<dyn PermissionBridge> = Arc::new(HubBridge(Arc::clone(&state.hub)));
            let question_bridge: Arc<dyn agent_core::UserQuestionBridge> =
                Arc::new(QuestionBridge {
                    hub: Arc::clone(&state.questions),
                    app: app.clone(),
                    session_id: session_id.clone(),
                });
            threads.insert(
                session_id.clone(),
                Thread::new(
                    config,
                    backend,
                    build_registry(
                        project_root.as_ref(),
                        skills_dir.as_deref(),
                        question_bridge,
                        settings_search_endpoint(&app),
                    ),
                    bridge,
                ),
            );
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
    let handle = {
        let threads = state.threads.lock().unwrap();
        threads
            .get(&session_id)
            .expect("Thread 刚插入必须存在")
            .send(user_text, tx)
    };
    *state.active.lock().unwrap() = Some(handle);

    // 4. 事件转发任务：Envelope → agent-event + JSONL 日志；Run 结束（tx 全部 drop）后自然退出
    let forward_app = app.clone();
    tokio::spawn(async move {
        while let Some(env) = rx.recv().await {
            log_event(&forward_app, &env);
            let _ = forward_app.emit("agent-event", &env);
        }
    });

    Ok(session_id)
}

/// 取消当前 Run（停止不再启动新工具调用；进行中的模型流被中断）
#[tauri::command]
pub fn agent_cancel(state: State<'_, AgentState>) {
    if let Some(h) = state.active.lock().unwrap().take() {
        h.cancel();
    }
    state.hub.drop_all();
    state.questions.drop_all();
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

/// 丢弃会话上下文（新对话 / 切换项目时调用）：取消运行、清挂起权限、删 Thread
#[tauri::command]
pub fn agent_reset_session(state: State<'_, AgentState>, session_id: String) {
    if let Some(h) = state.active.lock().unwrap().take() {
        h.cancel();
    }
    state.hub.drop_all();
    state.questions.drop_all();
    state.threads.lock().unwrap().remove(&session_id);
}

/// 当前 Agent 是否有进行中的 Run（页面刷新后恢复状态用）
#[derive(Debug, Serialize)]
pub struct AgentStatus {
    pub running: bool,
}

#[tauri::command]
pub fn agent_status(state: State<'_, AgentState>) -> AgentStatus {
    AgentStatus {
        running: state
            .active
            .lock()
            .unwrap()
            .as_ref()
            .map(|h| !h.is_cancelled())
            .unwrap_or(false),
    }
}

// 保留 ChatMessage / Role 引用（类型用于将来直接序列化快照）
#[allow(dead_code)]
fn _assert_types(_: ChatMessage, _: Role) {}

#[cfg(test)]
mod tests {
    use super::*;

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
