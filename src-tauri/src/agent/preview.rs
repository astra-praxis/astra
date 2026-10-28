use tauri::{AppHandle, Manager, State};

use super::*;
// ===== 预览：从后台任务日志发现本地服务地址并打开预览窗口 =====

// ===== 运行中子任务（Agent 页执行时间胶囊）：后台子进程 + TaskManager 各后端 =====

/// 运行中的子任务（前端展示 + 取消路由）
#[derive(serde::Serialize)]
pub struct AgentRunningTask {
    /// 取消路由：process = 后台子进程（id 为 u64，走 ProcessManager::kill）；
    /// 其余走 TaskManager::cancel
    pub kind: String,
    pub id: String,
    /// 展示名（命令前缀 / 子 Agent / 生成 · 模型 / MCP · server/tool）
    pub label: String,
    /// 已运行秒数（前端每秒本地递增 + 定时重拉校准）
    pub elapsed_secs: u64,
    /// 子 Agent 实时活动：最新 RunStatus（serde 名，前端 STATUS_LABEL 可渲染）
    #[serde(skip_serializing_if = "Option::is_none")]
    pub detail: Option<String>,
    /// 子 Agent 最近一次工具调用（"tool_name · args_summary"）
    #[serde(skip_serializing_if = "Option::is_none")]
    pub last_tool: Option<String>,
    /// 子 Agent 所属会话（旧字段：并行委托下 session 匹配有歧义，仅保留
    /// 兼容；卡片关联一律用 parent_tool_call_id）
    #[serde(skip_serializing_if = "Option::is_none")]
    pub session_id: Option<String>,
    /// 发起本次委托的 delegate_task 调用 id（v1.6 精确卡片关联：前端按
    /// conversation 里的 tool_call_id 匹配，不做 session 模糊回退）
    #[serde(skip_serializing_if = "Option::is_none")]
    pub parent_tool_call_id: Option<String>,
    /// 任务所用模型名（子 Agent = 父会话当前模型，v1 与父 Run 同源；
    /// 进程 / MCP 任务无模型概念，为 None）
    #[serde(skip_serializing_if = "Option::is_none")]
    pub model: Option<String>,
}

/// 列出当前运行中的子任务：后台 shell 子进程（真相源 ProcessManager）+
/// TaskManager 中的子 Agent / 生成 / MCP 任务。Shell 任务在 TaskManager
/// 里是 adopt 视图，跳过避免同进程重复显示；Internal 是宿主内建/测试，不面向用户
#[tauri::command]
pub async fn agent_tasks_running(
    state: State<'_, AgentState>,
) -> Result<Vec<AgentRunningTask>, String> {
    let mut out: Vec<AgentRunningTask> = state
        .processes
        .snapshot()
        .into_iter()
        .filter(|t| t.status == "running")
        .map(|t| AgentRunningTask {
            kind: "process".into(),
            id: t.id.to_string(),
            label: t.command.chars().take(48).collect(),
            elapsed_secs: t.runtime_secs,
            detail: None,
            last_tool: None,
            session_id: None,
            parent_tool_call_id: None,
            model: None,
        })
        .collect();
    for view in state.tasks.list().await {
        let running = matches!(
            view.status,
            agent_core::TaskStatus::Created
                | agent_core::TaskStatus::Running
                | agent_core::TaskStatus::Waiting
                | agent_core::TaskStatus::SubmissionUnknown
                | agent_core::TaskStatus::Cancelling
        );
        if !running {
            continue;
        }
        let (kind, label) = match &view.source {
            agent_core::TaskSource::Subagent { .. } => ("subagent", "子 Agent".to_string()),
            agent_core::TaskSource::Generation {
                model_id,
                operation,
                ..
            } => ("generation", format!("生成 · {model_id} ({operation})")),
            agent_core::TaskSource::Mcp { server, tool } => {
                ("mcp", format!("MCP · {server}/{tool}"))
            }
            // Shell 已经由 ProcessManager 展示；Internal 不面向用户
            _ => continue,
        };
        let elapsed_secs = (crate::agent::session_store::unix_timestamp()
            .saturating_sub(view.created_at_ms)
            / 1000)
            .max(0);
        // 子 Agent：用 backend 执行真相里的 external_id（= child_run_id）查
        // 实时活动快照（v1.5 可观测性：子事件被吞，仅保留最新一步；v1.6 加
        // parent_tool_call_id 供前端精确卡片关联）
        let (detail, last_tool, session_id, parent_tool_call_id) = if kind == "subagent" {
            match &view.external_id {
                Some(child_run_id) => {
                    let act = state.subagent_backend.activity(child_run_id).await;
                    (
                        act.as_ref().map(|a| a.status.clone()),
                        act.as_ref().and_then(|a| a.last_tool.clone()),
                        act.as_ref().map(|a| a.session_id.clone()),
                        act.as_ref().and_then(|a| a.parent_tool_call_id.clone()),
                    )
                }
                None => (None, None, None, None),
            }
        } else {
            (None, None, None, None)
        };
        // 子 Agent 与父 Run 同模型（v1 不做模型路由），按 activity 带回的
        // 父会话 id 查模型名；查不到（旧 Run / 快照缺失）不标注，前端隐趈
        let model = if kind == "subagent" {
            session_id.as_deref().and_then(|sid| {
                state
                    .session_models
                    .lock()
                    .unwrap_or_else(|e| e.into_inner())
                    .get(sid)
                    .cloned()
            })
        } else {
            None
        };
        out.push(AgentRunningTask {
            kind: kind.into(),
            id: view.id.0.clone(),
            label,
            elapsed_secs,
            detail,
            last_tool,
            session_id,
            parent_tool_call_id,
            model,
        });
    }
    Ok(out)
}

/// 取消一个运行中子任务：process 走 ProcessManager::kill（进程组终止），
/// 其余走 TaskManager::cancel（Requested = 已受理等提供方确认，不算失败）
#[tauri::command]
pub async fn agent_task_cancel(
    state: State<'_, AgentState>,
    kind: String,
    id: String,
) -> Result<(), String> {
    if kind == "process" {
        let task_id = id
            .parse::<u64>()
            .map_err(|_| format!("无效的后台进程 id: {id}"))?;
        if state.processes.kill(task_id) {
            return Ok(());
        }
        return Err(format!("后台进程不存在或已退出: {id}"));
    }
    match state.tasks.cancel(&agent_core::TaskId(id.clone())).await {
        Ok(agent_core::task::CancelOutcome::Confirmed)
        | Ok(agent_core::task::CancelOutcome::Requested) => Ok(()),
        Ok(agent_core::task::CancelOutcome::Unsupported) => {
            Err("该任务不支持取消，请等待其自然结束".into())
        }
        Err(e) => Err(e.to_string()),
    }
}

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
