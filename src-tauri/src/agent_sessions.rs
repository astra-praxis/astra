use serde::{Deserialize, Serialize};
use std::path::PathBuf;
use tauri::AppHandle;

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
pub fn list_agent_sessions(app: AppHandle) -> Result<Vec<AgentSession>, String> {
    let path = ensure_sessions_file(&app)?;
    let mut sessions = session_store::read_json_vec::<AgentSession>(&path);
    sessions.sort_by(|a, b| b.updated_at.cmp(&a.updated_at));
    Ok(sessions)
}

/// 保存 Agent 会话（按 id upsert；id 由前端生成并保持稳定）
#[tauri::command]
pub fn save_agent_session(
    app: AppHandle,
    id: String,
    project_path: String,
    project_name: String,
    title: String,
    entries: serde_json::Value,
) -> Result<AgentSession, String> {
    let path = ensure_sessions_file(&app)?;
    let mut sessions = session_store::read_json_vec::<AgentSession>(&path);

    let now = session_store::unix_timestamp();
    if let Some(session) = sessions.iter_mut().find(|s| s.id == id) {
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
pub fn delete_agent_session(app: AppHandle, id: String) -> Result<(), String> {
    let path = ensure_sessions_file(&app)?;
    let mut sessions = session_store::read_json_vec::<AgentSession>(&path);
    sessions.retain(|s| s.id != id);
    session_store::write_json_vec(&path, &sessions)
}
