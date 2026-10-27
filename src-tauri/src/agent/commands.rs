use agent_core::Decision;
use tauri::State;

use super::*;

/// 取消当前 Run（停止不再启动新工具调用；进行中的模型流被中断）。
/// `session_id` 指定会话（tab 的停止按钮，只取消该会话，不影响其他
/// 并行运行）；None 取消全部活跃 Run（旧的全局入口语义）。返回是否
/// 确实取消了运行：发送后的启动准备阶段尚未登记 active 时取消是空
/// 操作，前端据此给出可见反馈而不是静默失败。
#[tauri::command]
pub fn agent_cancel(state: State<'_, AgentState>, session_id: Option<String>) -> bool {
    agent_cancel_ref(&state, session_id)
}

/// agent_cancel 的纯引用版本（热切换账户时由 accounts 模块调用）。
pub(crate) fn agent_cancel_ref(state: &AgentState, session_id: Option<String>) -> bool {
    // Do not clear active until the receiver has drained the persistence tail.
    let active = state.active.lock().unwrap_or_else(|e| e.into_inner());
    match session_id {
        Some(sid) => {
            let Some(run) = active.get(&sid) else {
                return false;
            };
            run.handle.cancel();
            state.hub.drop_session(&sid);
            state.questions.drop_session(&sid);
            true
        }
        None => {
            if active.is_empty() {
                return false;
            }
            for run in active.values() {
                run.handle.cancel();
            }
            state.hub.drop_all();
            state.questions.drop_all();
            true
        }
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
    // P3: an unload while the session's Run is streaming would orphan the
    // live run and let the next send build a diverging second Thread.
    if state
        .active
        .lock()
        .unwrap_or_else(|e| e.into_inner())
        .contains_key(&session_id)
    {
        return Err("会话正在运行，请先停止再重置".into());
    }
    let mut threads = state.threads.lock().await;
    threads.unload(&session_id, &state.persistence_errors)
}

/// 扩展开关变更后批量卸载内存 Thread：所有空闲会话在下次对话时按新配置
/// 重新装配 MCP/ACP。正在运行的会话跳过（P3：不得卸载有活跃 Run 的
/// Thread，否则孤儿 Run + 双实例），由前端提示「将在下次对话时生效」；
/// 带未落盘错误的会话跳过（unload 返回 Err），不影响其余会话。
#[derive(serde::Serialize)]
pub struct UnloadAllReport {
    pub unloaded: usize,
    pub skipped: Vec<String>,
}

#[tauri::command]
pub async fn agent_unload_all_sessions(
    state: State<'_, AgentState>,
) -> Result<UnloadAllReport, String> {
    let mut threads = state.threads.lock().await;
    let running: std::collections::HashSet<String> = state
        .active
        .lock()
        .unwrap_or_else(|e| e.into_inner())
        .keys()
        .cloned()
        .collect();
    let ids: Vec<String> = threads.threads.keys().cloned().collect();
    let mut report = UnloadAllReport {
        unloaded: 0,
        skipped: Vec::new(),
    };
    for id in ids {
        if running.contains(&id) {
            report
                .skipped
                .push(format!("{id}: 会话正在运行，将在结束后生效"));
            continue;
        }
        match threads.unload(&id, &state.persistence_errors) {
            Ok(()) => report.unloaded += 1,
            Err(e) => report.skipped.push(format!("{id}: {e}")),
        }
    }
    Ok(report)
}
