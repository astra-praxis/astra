use std::sync::Arc;

use agent_core::{ChatMessage, Role};
use serde::Serialize;
use serde_json::json;
use tauri::{AppHandle, Emitter, State};

use super::*;
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

pub(crate) fn checkpoint_info(record: agent_core::CheckpointRecord) -> AgentCheckpointInfo {
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
    let directory = crate::app::accounts::data_dir(&app)?;
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
    let directory = crate::app::accounts::data_dir(&app)?;
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
        if active.contains_key(&session_id) {
            return Err("会话正在运行，请先取消当前任务再恢复 checkpoint".into());
        }
    }
    let mut threads = state.threads.lock().await;
    let directory = crate::app::accounts::data_dir(&app)?;
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

/// 回退结果：回退后的锚点游标、是否还原了工作区、回退后的完整消息序列
/// （前端用它重建会话视图，同 agent_checkpoint_restore.messages）。
#[derive(Debug, Serialize)]
pub struct AgentRewindResult {
    pub thread_seq: u64,
    pub workspace_restored: bool,
    pub messages: Vec<ChatMessage>,
}

/// 回退到倒数第 N 条用户消息之前（1 = 最后一条）。
///
/// 无感快照层的恢复原语，服务两个入口：
/// - 失败卡片「重试 / 编辑后重发」：`restore_workspace = true`，回退对话并
///   尝试把项目文件还原到该轮发送前的快照（找不到快照时降级为仅回退对话）；
/// - 历史消息「编辑重发」：`restore_workspace = false`，只回退对话（成功轮次
///   的文件产物属于用户工作，不能随对话回退被静默覆盖）。
///
/// 工作区还原走最近一次自动锚定的 BeforeTurn checkpoint（agent_start_run
/// 每次发送前创建）；同一锚点的快照记录可能因早先的回退被清理，此时降级。
#[tauri::command]
pub async fn agent_rewind_to_user_message(
    app: AppHandle,
    state: State<'_, AgentState>,
    session_id: String,
    turns_back: u32,
    restore_workspace: bool,
) -> Result<AgentRewindResult, String> {
    // 进行中的 Run 会持有 threads 生命周期锁；先给出明确提示而不是静默排队。
    {
        let active = state.active.lock().unwrap_or_else(|e| e.into_inner());
        if active.contains_key(&session_id) {
            return Err("会话正在运行，请先取消当前任务再回退".into());
        }
    }
    let mut threads = state.threads.lock().await;
    let directory = crate::app::accounts::data_dir(&app)?;
    threads
        .open(&directory, Arc::clone(&state.persistence_errors))
        .await?;
    let thread = threads
        .threads
        .get(&session_id)
        .ok_or_else(|| "会话未加载，无法回退".to_string())?;

    // 定位目标用户消息前的锚点（候选消息不足时明确报错）。
    let anchor = thread
        .anchor_before_user_turn(turns_back)
        .await
        .map_err(|e| e.to_string())?;

    // 工作区还原：找锚点上的快照记录走完整 restore（对话回退 + 文件还原）。
    let mut workspace_restored = false;
    if restore_workspace {
        let checkpoints = threads.checkpoints()?;
        let records = checkpoints
            .list(&session_id)
            .await
            .map_err(|e| e.to_string())?;
        let record = records.iter().find(|r| r.thread_seq == anchor);
        if let Some(record) = record {
            let provider = agent_core::astra_workspace::GitWorkspaceCheckpointProvider::new();
            match thread
                .restore_checkpoint(checkpoints, &record.id, Some(Arc::new(provider)), None)
                .await
            {
                Ok(report) => workspace_restored = report.workspace_restored,
                Err(error) => {
                    // 快照还原失败不阻断回退：对话本身必须能回去。
                    eprintln!(
                        "[checkpoint] workspace restore failed (session {session_id}): {error}"
                    );
                    thread.rewind_to(anchor).await.map_err(|e| e.to_string())?;
                }
            }
        }
    }

    // 仅对话回退（或快照缺失 / 还原失败后的降级路径）。
    if !workspace_restored {
        thread.rewind_to(anchor).await.map_err(|e| e.to_string())?;
    }

    let _ = app.emit(
        "agent-checkpoint-restored",
        json!({ "session_id": session_id, "checkpoint_id": null }),
    );
    Ok(AgentRewindResult {
        thread_seq: anchor,
        workspace_restored,
        messages: thread.messages(),
    })
}
