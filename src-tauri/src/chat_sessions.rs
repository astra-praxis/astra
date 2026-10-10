use serde::{Deserialize, Serialize};
use std::path::PathBuf;
use tauri::AppHandle;

use crate::chat::{ChatMessage, ChatSource};
use crate::session_store;

// ===== 类型 =====

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct ChatSession {
    pub id: String,
    pub title: String,
    pub source: ChatSource,
    pub messages: Vec<ChatMessage>,
    pub created_at: u64,
    pub updated_at: u64,
}

// ===== 辅助 =====

fn ensure_sessions_file(app: &AppHandle) -> Result<PathBuf, String> {
    session_store::ensure_json_file(app, "chat_sessions.json", "创建聊天记录文件")
}

fn generate_id() -> String {
    format!("chat_{}", session_store::unix_timestamp())
}

// ===== Tauri 命令 =====

/// 列出所有聊天记录（按更新时间倒序）
#[tauri::command]
pub fn list_chat_sessions(app: AppHandle) -> Result<Vec<ChatSession>, String> {
    let path = ensure_sessions_file(&app)?;
    let mut sessions = session_store::read_json_vec::<ChatSession>(&path);

    // 按更新时间倒序排列
    sessions.sort_by(|a, b| b.updated_at.cmp(&a.updated_at));
    Ok(sessions)
}

/// 保存聊天记录（新建或更新）
#[tauri::command]
pub fn save_chat_session(
    app: AppHandle,
    id: Option<String>,
    title: String,
    source: ChatSource,
    messages: Vec<ChatMessage>,
) -> Result<ChatSession, String> {
    let path = ensure_sessions_file(&app)?;
    let mut sessions = session_store::read_json_vec::<ChatSession>(&path);

    let now = session_store::unix_timestamp();

    if let Some(existing_id) = id {
        // 更新现有记录
        let session = sessions
            .iter_mut()
            .find(|s| s.id == existing_id)
            .ok_or_else(|| "找不到要更新的聊天记录".to_string())?;
        session.title = title;
        session.source = source;
        session.messages = messages;
        session.updated_at = now;

        let result = session.clone();
        session_store::write_json_vec(&path, &sessions)?;
        Ok(result)
    } else {
        // 新建记录
        let session = ChatSession {
            id: generate_id(),
            title,
            source,
            messages,
            created_at: now,
            updated_at: now,
        };

        sessions.push(session.clone());
        session_store::write_json_vec(&path, &sessions)?;
        Ok(session)
    }
}

/// 更新聊天记录标题
#[tauri::command]
pub fn update_chat_session_title(app: AppHandle, id: String, title: String) -> Result<(), String> {
    let path = ensure_sessions_file(&app)?;
    let mut sessions = session_store::read_json_vec::<ChatSession>(&path);

    let session = sessions
        .iter_mut()
        .find(|s| s.id == id)
        .ok_or_else(|| "找不到聊天记录".to_string())?;
    session.title = title;
    session.updated_at = session_store::unix_timestamp();
    session_store::write_json_vec(&path, &sessions)
}

/// 删除聊天记录
#[tauri::command]
pub fn delete_chat_session(app: AppHandle, id: String) -> Result<(), String> {
    let path = ensure_sessions_file(&app)?;
    let mut sessions = session_store::read_json_vec::<ChatSession>(&path);

    let len_before = sessions.len();
    sessions.retain(|s| s.id != id);

    if sessions.len() == len_before {
        return Err("找不到要删除的聊天记录".to_string());
    }

    session_store::write_json_vec(&path, &sessions)
}

/// 获取单个聊天记录
#[tauri::command]
pub fn get_chat_session(app: AppHandle, id: String) -> Result<ChatSession, String> {
    let path = ensure_sessions_file(&app)?;
    let sessions = session_store::read_json_vec::<ChatSession>(&path);

    sessions
        .into_iter()
        .find(|s| s.id == id)
        .ok_or_else(|| "找不到聊天记录".to_string())
}
