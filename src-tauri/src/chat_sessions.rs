use serde::{Deserialize, Serialize};
use std::fs;
use std::path::PathBuf;
use tauri::{AppHandle, Manager};

use crate::chat::{ChatMessage, ChatSource};

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

fn get_sessions_path(app: &AppHandle) -> Result<PathBuf, String> {
    let app_data_dir = app
        .path()
        .app_data_dir()
        .map_err(|e| format!("获取应用数据目录失败: {}", e))?;
    fs::create_dir_all(&app_data_dir).map_err(|e| format!("创建配置目录失败: {}", e))?;
    Ok(app_data_dir.join("chat_sessions.json"))
}

fn ensure_sessions_file(app: &AppHandle) -> Result<PathBuf, String> {
    let path = get_sessions_path(app)?;
    if !path.exists() {
        fs::write(&path, "[]").map_err(|e| format!("创建聊天记录文件失败: {}", e))?;
    }
    Ok(path)
}

fn unix_timestamp() -> u64 {
    std::time::SystemTime::now()
        .duration_since(std::time::UNIX_EPOCH)
        .unwrap_or_default()
        .as_secs()
}

fn generate_id() -> String {
    format!("chat_{}", unix_timestamp())
}

// ===== Tauri 命令 =====

/// 列出所有聊天记录（按更新时间倒序）
#[tauri::command]
pub fn list_chat_sessions(app: AppHandle) -> Result<Vec<ChatSession>, String> {
    let path = ensure_sessions_file(&app)?;

    let content = fs::read_to_string(&path).unwrap_or_else(|_| "[]".to_string());
    let mut sessions: Vec<ChatSession> = serde_json::from_str(&content).unwrap_or_default();

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

    let content = fs::read_to_string(&path).unwrap_or_else(|_| "[]".to_string());
    let mut sessions: Vec<ChatSession> = serde_json::from_str(&content).unwrap_or_default();

    let now = unix_timestamp();

    if let Some(existing_id) = id {
        // 更新现有记录
        if let Some(session) = sessions.iter_mut().find(|s| s.id == existing_id) {
            session.title = title;
            session.source = source;
            session.messages = messages;
            session.updated_at = now;

            let result = session.clone();
            let _ = fs::write(
                &path,
                serde_json::to_string_pretty(&sessions).unwrap_or_default(),
            );
            return Ok(result);
        }
        return Err("找不到要更新的聊天记录".to_string());
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
        let _ = fs::write(
            &path,
            serde_json::to_string_pretty(&sessions).unwrap_or_default(),
        );
        Ok(session)
    }
}

/// 更新聊天记录标题
#[tauri::command]
pub fn update_chat_session_title(app: AppHandle, id: String, title: String) -> Result<(), String> {
    let path = ensure_sessions_file(&app)?;

    let content = fs::read_to_string(&path).unwrap_or_else(|_| "[]".to_string());
    let mut sessions: Vec<ChatSession> = serde_json::from_str(&content).unwrap_or_default();

    if let Some(session) = sessions.iter_mut().find(|s| s.id == id) {
        session.title = title;
        session.updated_at = unix_timestamp();
        let _ = fs::write(
            &path,
            serde_json::to_string_pretty(&sessions).unwrap_or_default(),
        );
        Ok(())
    } else {
        Err("找不到聊天记录".to_string())
    }
}

/// 删除聊天记录
#[tauri::command]
pub fn delete_chat_session(app: AppHandle, id: String) -> Result<(), String> {
    let path = ensure_sessions_file(&app)?;

    let content = fs::read_to_string(&path).unwrap_or_else(|_| "[]".to_string());
    let mut sessions: Vec<ChatSession> = serde_json::from_str(&content).unwrap_or_default();

    let len_before = sessions.len();
    sessions.retain(|s| s.id != id);

    if sessions.len() == len_before {
        return Err("找不到要删除的聊天记录".to_string());
    }

    let _ = fs::write(
        &path,
        serde_json::to_string_pretty(&sessions).unwrap_or_default(),
    );
    Ok(())
}

/// 获取单个聊天记录
#[tauri::command]
pub fn get_chat_session(app: AppHandle, id: String) -> Result<ChatSession, String> {
    let path = ensure_sessions_file(&app)?;

    let content = fs::read_to_string(&path).unwrap_or_else(|_| "[]".to_string());
    let sessions: Vec<ChatSession> = serde_json::from_str(&content).unwrap_or_default();

    sessions
        .into_iter()
        .find(|s| s.id == id)
        .ok_or_else(|| "找不到聊天记录".to_string())
}
