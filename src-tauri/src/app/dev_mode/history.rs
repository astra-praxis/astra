use tauri::AppHandle;

use super::types::*;
// ===== 开发服务历史记录 =====

/// 获取历史记录文件路径
pub(crate) fn get_dev_history_path(app: &AppHandle) -> Result<std::path::PathBuf, String> {
    let config_dir = crate::app::accounts::data_dir(app)?;
    std::fs::create_dir_all(&config_dir).map_err(|e| format!("创建配置目录失败: {}", e))?;
    Ok(config_dir.join("dev_history.json"))
}

/// 读取历史记录
pub(crate) fn read_dev_history(app: &AppHandle) -> Vec<DevHistoryEntry> {
    let path = match get_dev_history_path(app) {
        Ok(p) => p,
        Err(_) => return Vec::new(),
    };
    match std::fs::read_to_string(&path) {
        Ok(content) => serde_json::from_str(&content).unwrap_or_default(),
        Err(_) => Vec::new(),
    }
}

/// 写入历史记录
pub(crate) fn write_dev_history(
    app: &AppHandle,
    history: &[DevHistoryEntry],
) -> Result<(), String> {
    let path = get_dev_history_path(app)?;
    let json =
        serde_json::to_string_pretty(history).map_err(|e| format!("序列化历史记录失败: {}", e))?;
    std::fs::write(&path, json).map_err(|e| format!("写入历史记录失败: {}", e))?;
    Ok(())
}

/// 查询指定项目的历史启动记录
#[tauri::command]
pub fn list_dev_history(
    app: AppHandle,
    project_path: String,
) -> Result<Vec<DevHistoryEntry>, String> {
    let history = read_dev_history(&app);
    let filtered: Vec<DevHistoryEntry> = history
        .into_iter()
        .filter(|h| h.project_path == project_path)
        .collect();
    Ok(filtered)
}

/// 清除指定项目的历史启动记录
#[tauri::command]
pub fn clear_dev_history(app: AppHandle, project_path: String) -> Result<(), String> {
    let mut history = read_dev_history(&app);
    history.retain(|h| h.project_path != project_path);
    write_dev_history(&app, &history)
}
