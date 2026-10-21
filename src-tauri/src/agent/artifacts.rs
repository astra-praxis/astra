use std::sync::Arc;

use serde::Serialize;
use serde_json::Value;
use tauri::{AppHandle, Manager, State};


use super::*;
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

pub(crate) fn artifact_info(record: agent_core::artifact_registry::ArtifactRecord) -> AgentArtifactInfo {
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

/// 粘贴落盘结果（spilled 恒为 true，保留字段位以防未来引入禁写场景）。
#[derive(serde::Serialize)]
pub struct SpillPasteResult {
    pub spilled: bool,
    pub path: Option<String>,
    pub size: u64,
}

/// 把用户粘贴的超长内容写入 `.astra/pastes/paste-<时间戳>.txt`，
/// 发送时消息正文只带「先 read_file 读取该文件再处理指令」，避免超长文本撑爆
/// 输入框与上下文。应用侧功能：只负责写文件，不参与 Agent 循环。
/// 有项目时写在 `<project>/.astra/pastes/`；纯聊天时写在 `~/.astra/pastes/`
/// （纯聊天的文件工具以主目录为 root，read_file 可直接读该绝对路径）。
#[tauri::command]
pub fn agent_spill_paste(
    project_root: Option<String>,
    content: String,
) -> Result<SpillPasteResult, String> {
    if content.is_empty() {
        return Err("粘贴内容为空".into());
    }
    let dir = match project_root.filter(|p| std::path::Path::new(p).is_dir()) {
        Some(root) => std::path::Path::new(&root).join(".astra").join("pastes"),
        None => dirs::home_dir()
            .ok_or_else(|| "无法定位用户主目录".to_string())?
            .join(".astra")
            .join("pastes"),
    };
    std::fs::create_dir_all(&dir).map_err(|e| format!("创建粘贴目录失败：{e}"))?;
    let stamp = chrono::Local::now().format("%Y%m%d-%H%M%S%3f");
    let path = dir.join(format!("paste-{stamp}.txt"));
    std::fs::write(&path, &content).map_err(|e| format!("写入粘贴文件失败：{e}"))?;
    Ok(SpillPasteResult {
        spilled: true,
        path: Some(path.display().to_string()),
        size: content.len() as u64,
    })
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
