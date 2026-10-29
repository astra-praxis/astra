use std::sync::Arc;

use serde::Serialize;
use tauri::State;

use super::*;
// ===== ChangeSet 查询（FileChanges 卡片：宿主提供事实，前端渲染）=====

/// 前端可见的单文件变更摘要（同一路径在一次 Run 内的多次修改已合并：
/// kind 取最后一次，增删行数累加）。
#[derive(Debug, Serialize)]
pub struct AgentFileChangeInfo {
    /// 相对项目根的路径（工具输出的 POSIX 风格）
    pub path: String,
    /// created | modified | deleted（最后一次操作的类型）
    pub kind: String,
    /// 产生该变更的工具名（最后一次）
    pub tool: String,
    pub insertions: u32,
    pub deletions: u32,
    /// move/copy 的来源路径
    pub from: Option<String>,
    /// 宿主是否有恢复所需数据（快照/哈希）；无则前端隐藏恢复入口
    pub restorable: bool,
    pub updated_at: u64,
}

#[derive(Debug, Serialize)]
pub struct AgentChangeSetInfo {
    pub run_id: String,
    pub started_at: u64,
    pub ended_at: u64,
    pub files: Vec<AgentFileChangeInfo>,
}

/// 单文件 diff（变更前快照 vs 当前磁盘内容）
#[derive(Debug, Serialize)]
pub struct AgentFileDiff {
    pub path: String,
    pub diff: String,
    pub insertions: u32,
    pub deletions: u32,
}

#[derive(Debug, Serialize)]
pub struct AgentRestoreResult {
    pub restored: Vec<agent_core::change_tracking::RestoredFile>,
    pub conflicts: Vec<agent_core::change_tracking::RestoreConflict>,
}

fn tracker_of(
    state: &AgentState,
    session_id: &str,
) -> Result<Arc<agent_core::ChangeTracker>, String> {
    state
        .change_trackers
        .lock()
        .unwrap_or_else(|e| e.into_inner())
        .get(session_id)
        .cloned()
        .ok_or_else(|| "该会话没有变更记录（可能是旧会话或追踪器未初始化）".to_owned())
}

fn change_info(f: &agent_core::change_tracking::FileChangeSummary) -> AgentFileChangeInfo {
    AgentFileChangeInfo {
        path: f.path.clone(),
        kind: f.kind.as_str().to_owned(),
        tool: f.tool.clone(),
        insertions: f.insertions,
        deletions: f.deletions,
        from: f.from.clone(),
        restorable: f.restorable,
        updated_at: f.updated_at,
    }
}

/// 列出会话全部 Run 的文件变更集（started_at 升序）。
#[tauri::command]
pub async fn agent_change_sets(
    state: State<'_, AgentState>,
    session_id: String,
) -> Result<Vec<AgentChangeSetInfo>, String> {
    let tracker = tracker_of(&state, &session_id)?;
    let sets = tokio::task::spawn_blocking(move || tracker.change_sets())
        .await
        .map_err(|e| e.to_string())?;
    Ok(sets
        .into_iter()
        .map(|s| AgentChangeSetInfo {
            run_id: s.run_id,
            started_at: s.started_at,
            ended_at: s.ended_at,
            files: s.files.iter().map(change_info).collect(),
        })
        .collect())
}

/// 读取一次 Run 内某文件的真实差异（修改前快照 vs 当前磁盘内容）。
#[tauri::command]
pub async fn agent_change_diff(
    state: State<'_, AgentState>,
    session_id: String,
    run_id: String,
    path: String,
) -> Result<AgentFileDiff, String> {
    let tracker = tracker_of(&state, &session_id)?;
    let root = {
        let threads = state.threads.lock().await;
        threads
            .threads
            .get(&session_id)
            .and_then(|t| t.project_root().cloned())
            .ok_or_else(|| "会话未加载或未绑定项目目录".to_owned())?
    };
    let diff = tokio::task::spawn_blocking(move || tracker.diff_for(&root, &run_id, &path))
        .await
        .map_err(|e| e.to_string())??;
    Ok(AgentFileDiff {
        path: diff.path,
        diff: diff.diff,
        insertions: diff.insertions,
        deletions: diff.deletions,
    })
}

/// 恢复一次 Run 内指定文件的变更前状态。宿主校验：
/// - 会话不得正在运行（变更可能仍在产生）
/// - 当前内容与记录的修改后哈希不一致时拒绝（除非 force），
///   不静默覆盖用户或其他 Agent 的后续修改
#[tauri::command]
pub async fn agent_change_restore(
    state: State<'_, AgentState>,
    session_id: String,
    run_id: String,
    paths: Vec<String>,
    force: Option<bool>,
) -> Result<AgentRestoreResult, String> {
    if paths.is_empty() {
        return Err("未选择要恢复的文件".into());
    }
    {
        let active = state.active.lock().unwrap_or_else(|e| e.into_inner());
        if active.contains_key(&session_id) {
            return Err("会话正在运行，请先取消当前任务再恢复文件".into());
        }
    }
    let tracker = tracker_of(&state, &session_id)?;
    let root = {
        let threads = state.threads.lock().await;
        threads
            .threads
            .get(&session_id)
            .and_then(|t| t.project_root().cloned())
            .ok_or_else(|| "会话未加载或未绑定项目目录".to_owned())?
    };
    let force = force.unwrap_or(false);
    let run_for_task = run_id.clone();
    let outcome =
        tokio::task::spawn_blocking(move || tracker.restore(&root, &run_for_task, &paths, force))
            .await
            .map_err(|e| e.to_string())??;
    if !outcome.conflicts.is_empty() {
        app_log!(
            warn,
            "agent",
            "change restore conflicts: session={session_id} run={run_id} n={}",
            outcome.conflicts.len()
        );
    }
    Ok(AgentRestoreResult {
        restored: outcome.restored,
        conflicts: outcome.conflicts,
    })
}
