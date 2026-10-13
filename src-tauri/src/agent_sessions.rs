use crate::agent::AgentState;
use serde::{Deserialize, Serialize};
use std::path::PathBuf;
use tauri::{AppHandle, Manager, State};

use crate::session_store;

// ===== 类型 =====

/// Agent 会话历史记录（与 chat_sessions.rs 同模式：JSON 文件全量读写）。
/// entries 存前端条目流的原始 JSON（消息/工具卡/权限卡），恢复时原样回放。
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct AgentSession {
    pub id: String,
    /// 绑定的项目路径（空 = 纯聊天）
    pub project_path: String,
    pub project_name: String,
    /// 首条用户消息截断，作列表标题
    pub title: String,
    pub entries: serde_json::Value,
    pub created_at: u64,
    pub updated_at: u64,
}

// ===== 辅助 =====

fn ensure_sessions_file(app: &AppHandle) -> Result<PathBuf, String> {
    session_store::ensure_json_file(app, "agent_sessions.json", "创建会话记录文件")
}

// ===== Tauri 命令 =====

/// 列出所有 Agent 会话（按更新时间倒序）
#[tauri::command]
pub async fn list_agent_sessions(
    app: AppHandle,
    state: State<'_, AgentState>,
) -> Result<Vec<AgentSession>, String> {
    let _files = state.session_files.lock().await;
    let path = ensure_sessions_file(&app)?;
    let mut sessions = session_store::read_json_vec::<AgentSession>(&path);
    sessions.sort_by(|a, b| b.updated_at.cmp(&a.updated_at));
    Ok(sessions)
}

/// 保存 Agent 会话（按 id upsert；id 由前端生成并保持稳定）
#[tauri::command]
pub async fn save_agent_session(
    app: AppHandle,
    state: State<'_, AgentState>,
    id: String,
    project_path: String,
    project_name: String,
    title: String,
    entries: serde_json::Value,
) -> Result<AgentSession, String> {
    let deleted = state.session_files.lock().await;
    if deleted.contains(&id) {
        return Err("Cannot save a deleted agent session".into());
    }
    let path = ensure_sessions_file(&app)?;
    let mut sessions = session_store::read_json_vec::<AgentSession>(&path);

    let now = session_store::unix_timestamp();
    if let Some(session) = sessions.iter_mut().find(|s| s.id == id) {
        // 内容未变时不刷新 updated_at：切换会话/驻留存档会反复落库，
        // 若无条件更新时间戳，历史列表（按 updated_at 倒序）顺序会跳变
        if session.project_path == project_path
            && session.project_name == project_name
            && session.title == title
            && session.entries == entries
        {
            return Ok(session.clone());
        }
        session.project_path = project_path;
        session.project_name = project_name;
        session.title = title;
        session.entries = entries;
        session.updated_at = now;
        let result = session.clone();
        session_store::write_json_vec(&path, &sessions)?;
        return Ok(result);
    }

    let session = AgentSession {
        id,
        project_path,
        project_name,
        title,
        entries,
        created_at: now,
        updated_at: now,
    };
    sessions.push(session.clone());
    session_store::write_json_vec(&path, &sessions)?;
    Ok(session)
}

/// 删除 Agent 会话
#[tauri::command]
pub async fn delete_agent_session(
    app: AppHandle,
    state: State<'_, AgentState>,
    id: String,
) -> Result<(), String> {
    // Reject rather than race any active Run, including its cancelled write tail.
    let mut threads = state
        .threads
        .try_lock()
        .map_err(|_| "Agent is running or draining; wait before deleting".to_string())?;
    let mut deleted = state.session_files.lock().await;
    let path = ensure_sessions_file(&app)?;
    let mut sessions = session_store::read_json_vec::<AgentSession>(&path);
    let directory = app.path().app_data_dir().map_err(|e| e.to_string())?;
    threads
        .open(&directory, std::sync::Arc::clone(&state.persistence_errors))
        .await?;
    threads.delete(&id).await?;
    // A failed JSON write can be retried; never permit a delayed save to resurrect it.
    deleted.insert(id.clone());
    state
        .persistence_errors
        .lock()
        .unwrap_or_else(|e| e.into_inner())
        .remove(&id);
    sessions.retain(|s| s.id != id);
    session_store::write_json_vec(&path, &sessions)
}
