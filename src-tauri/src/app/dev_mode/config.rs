use std::path::Path;

use super::parse::*;
use super::types::*;
// ===== 开发配置（Cookie / 代理地址） =====

/// 读取 master 目录下 vue.config.js 的开发配置
#[tauri::command]
pub fn read_dev_config(master_path: String) -> Result<DevConfig, String> {
    let config_path = Path::new(&master_path).join("vue.config.js");
    let content = std::fs::read_to_string(&config_path)
        .map_err(|e| format!("读取 vue.config.js 失败: {}", e))?;

    Ok(DevConfig {
        cookie: extract_cookie(&content),
        proxy_target: extract_backend_target(&content),
        port: extract_dev_server_port(&content),
    })
}

/// 保存 Cookie 到 master vue.config.js 的 cookie1 函数
/// cookie 参数应为完整值，如 "WISCPSID=a2acc58b-..."
#[tauri::command]
pub fn save_dev_cookie(master_path: String, cookie: String) -> Result<(), String> {
    let config_path = Path::new(&master_path).join("vue.config.js");
    let content = std::fs::read_to_string(&config_path)
        .map_err(|e| format!("读取 vue.config.js 失败: {}", e))?;

    let new_content = replace_cookie_value(&content, &cookie).ok_or("未找到 Cookie 配置位置")?;

    std::fs::write(&config_path, new_content)
        .map_err(|e| format!("写入 vue.config.js 失败: {}", e))?;
    Ok(())
}

/// 在系统默认浏览器中打开 URL（用于登录获取 Cookie）
#[tauri::command]
pub fn open_login_url(url: String) -> Result<(), String> {
    #[cfg(target_os = "macos")]
    {
        std::process::Command::new("open")
            .arg(&url)
            .spawn()
            .map_err(|e| format!("打开浏览器失败: {}", e))?;
    }
    #[cfg(target_os = "linux")]
    {
        std::process::Command::new("xdg-open")
            .arg(&url)
            .spawn()
            .map_err(|e| format!("打开浏览器失败: {}", e))?;
    }
    #[cfg(target_os = "windows")]
    {
        std::process::Command::new("cmd")
            .args(["/c", "start", "", &url])
            .spawn()
            .map_err(|e| format!("打开浏览器失败: {}", e))?;
    }
    Ok(())
}
