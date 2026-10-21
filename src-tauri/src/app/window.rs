use tauri::Manager;

// ===== 命令：最小化到托盘 =====

#[tauri::command]
pub(crate) fn minimize_to_tray(window: tauri::WebviewWindow) -> Result<(), String> {
    window.hide().map_err(|e| e.to_string())?;
    Ok(())
}
// ===== 命令：窗口拖拽 =====

#[tauri::command]
pub(crate) fn drag_window(app: tauri::AppHandle) -> Result<(), String> {
    let window = app
        .get_webview_window("main")
        .ok_or_else(|| "主窗口未找到".to_string())?;
    window
        .start_dragging()
        .map_err(|e| format!("拖拽失败: {}", e))?;
    Ok(())
}

// ===== 命令：设置窗口背景色（主题切换时调用）=====

#[tauri::command]
pub(crate) fn set_window_background(app: tauri::AppHandle, is_dark: bool) -> Result<(), String> {
    let window = app
        .get_webview_window("main")
        .ok_or_else(|| "主窗口未找到".to_string())?;
    use tauri::window::Color;
    let color = if is_dark {
        Color(20, 20, 32, 255)
    } else {
        Color(240, 242, 245, 255)
    };
    window
        .set_background_color(Some(color))
        .map_err(|e| format!("设置背景色失败: {}", e))?;
    Ok(())
}
