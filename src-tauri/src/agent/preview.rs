
use tauri::{AppHandle, Manager, State};


use super::*;
// ===== 预览：从后台任务日志发现本地服务地址并打开预览窗口 =====

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
