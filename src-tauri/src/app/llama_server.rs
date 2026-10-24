use serde::{Deserialize, Serialize};
use std::process::{Child, Command, Stdio};
use std::sync::Mutex;
use tauri::Manager;

use crate::app::settings::{get_settings, save_settings};

// ===== llama-server 管理 =====

pub(crate) struct RunningServer {
    child: Child,
    port: u16,
    model_name: String,
    model_path: String,
    started_at: u64,
    pid: u32,
}

pub(crate) type ServerState = Mutex<Vec<RunningServer>>;

#[derive(Debug, Serialize, Deserialize, Clone)]
pub(crate) struct ModelConfig {
    id: String,
    name: String,
    model_path: String,
    server_path: String,
    port: u16,
    ngl: u32,
}

#[derive(Debug, Serialize)]
pub(crate) struct RunningModelInfo {
    #[serde(flatten)]
    config: ModelConfig,
    status: String,
    pid: Option<u32>,
    started_at: u64,
}

pub(crate) fn unix_timestamp() -> u64 {
    std::time::SystemTime::now()
        .duration_since(std::time::UNIX_EPOCH)
        .map(|d| d.as_secs())
        .unwrap_or(0)
}

pub(crate) fn default_port() -> u16 {
    8080
}

pub(crate) fn default_ngl() -> u32 {
    999
}

// ===== 命令： llama-server 管理 =====

#[tauri::command]
pub(crate) fn start_llama_server(
    app: tauri::AppHandle,
    server_path: String,
    model_path: String,
    port: Option<u16>,
    ngl: Option<u32>,
    model_name: String,
) -> Result<(u16, u32), String> {
    let actual_port = port.unwrap_or(default_port());
    let actual_ngl = ngl.unwrap_or(default_ngl());

    // 检查端口是否已被占用（操作系统层面检测）
    std::net::TcpListener::bind(format!("0.0.0.0:{}", actual_port))
        .map_err(|e| format!("端口 {} 已被占用：{}", actual_port, e))?;

    // 检查内部状态中是否有相同端口的进程（双重检查）
    let state = app.state::<ServerState>();
    let servers = state.lock().unwrap();
    for server in servers.iter() {
        if server.port == actual_port {
            return Err(format!("端口 {} 已被占用", actual_port));
        }
    }
    // 释放锁，以便后续可以再次获取
    drop(servers);

    // 构建命令：llama serve -m <model> --port <port> -ngl <ngl> --host 0.0.0.0
    // 空则默认使用 'llama'（依赖系统 PATH）
    let server_bin = if server_path.trim().is_empty() {
        "llama"
    } else {
        &server_path
    };

    // 补充 PATH（打包后可能不完整）：Windows 直接沿用，Unix 补常见目录，
    // join_paths 按平台分隔符拼接
    let existing = std::env::var_os("PATH").unwrap_or_default();
    #[cfg(windows)]
    let extra: Vec<std::path::PathBuf> = Vec::new();
    #[cfg(not(windows))]
    let extra: Vec<std::path::PathBuf> = [
        "/usr/local/bin",
        "/opt/homebrew/bin",
        "/opt/local/bin",
        "/usr/bin",
        "/bin",
    ]
    .iter()
    .map(std::path::PathBuf::from)
    .collect();
    let mut dirs: Vec<std::path::PathBuf> = extra;
    dirs.push(existing.into());
    let full_path = std::env::join_paths(&dirs).unwrap_or_default();

    let mut cmd = Command::new(server_bin);
    cmd.arg("serve")
        .arg("-m")
        .arg(&model_path)
        .arg("--port")
        .arg(actual_port.to_string())
        .arg("-ngl")
        .arg(actual_ngl.to_string())
        .arg("--host")
        .arg("0.0.0.0")
        .env("PATH", &full_path)
        .stdout(Stdio::null())
        .stderr(Stdio::null());
    crate::proc_util::hide_window(&mut cmd);

    let child = cmd
        .spawn()
        .map_err(|e| format!("启动 llama serve 失败：{}", e))?;

    // 记录运行中的服务
    let pid = child.id();
    let server = RunningServer {
        child,
        port: actual_port,
        model_name,
        model_path,
        started_at: unix_timestamp(),
        pid,
    };
    app.state::<ServerState>().lock().unwrap().push(server);

    Ok((actual_port, actual_ngl))
}

#[tauri::command]
pub(crate) fn stop_llama_server(app: tauri::AppHandle, port: u16) -> Result<bool, String> {
    let state = app.state::<ServerState>();
    let mut servers = state.lock().unwrap();
    let mut server_index = None;

    for (i, server) in servers.iter().enumerate() {
        if server.port == port {
            server_index = Some(i);
            break;
        }
    }

    if let Some(index) = server_index {
        let mut server = servers.remove(index);
        // 尝试优雅关闭
        match server.child.try_wait() {
            Ok(Some(_)) => {
                // 进程已经终止
                Ok(true)
            }
            Ok(None) => {
                // 进程仍在运行，强制终止
                server
                    .child
                    .kill()
                    .map_err(|e| format!("终止进程失败: {}", e))?;
                Ok(true)
            }
            Err(_) => Ok(false),
        }
    } else {
        Ok(false)
    }
}

#[tauri::command]
pub(crate) fn list_running_servers(app: tauri::AppHandle) -> Result<Vec<RunningModelInfo>, String> {
    let mut servers = Vec::new();
    let state = app.state::<ServerState>();
    let mut running_servers = state.lock().unwrap();
    let mut dead_servers = Vec::new();

    for (i, server) in running_servers.iter_mut().enumerate() {
        // 检查进程状态
        let status = match server.child.try_wait() {
            Ok(Some(_)) => "stopped",
            Ok(None) => "running",
            Err(_) => "unknown",
        };

        servers.push(RunningModelInfo {
            config: ModelConfig {
                id: format!("model-{}-{}", server.model_name, server.port),
                name: server.model_name.clone(),
                model_path: server.model_path.clone(),
                server_path: String::new(),
                port: server.port,
                ngl: 0,
            },
            status: status.to_string(),
            pid: Some(server.pid),
            started_at: server.started_at,
        });

        if status != "running" {
            dead_servers.push(i);
        }
    }

    // 清理已终止的服务
    for index in dead_servers.into_iter().rev() {
        let _ = running_servers.remove(index);
    }

    Ok(servers)
}

#[tauri::command]
pub(crate) fn check_server_status(
    app: tauri::AppHandle,
    port: u16,
) -> Result<Option<String>, String> {
    let state = app.state::<ServerState>();
    let mut servers = state.lock().unwrap();

    for server in servers.iter_mut() {
        if server.port == port {
            return Ok(Some(
                match server.child.try_wait() {
                    Ok(Some(_)) => "stopped",
                    Ok(None) => "running",
                    Err(_) => {
                        // 进程可能已崩溃，尝试通过 TCP 探测确认
                        if std::net::TcpListener::bind(format!("0.0.0.0:{}", port)).is_ok() {
                            // 可以绑定 = 端口没有进程在监听
                            "stopped"
                        } else {
                            // 仍然被占用 = 有外部进程
                            "running"
                        }
                    }
                }
                .to_string(),
            ));
        }
    }

    Ok(None)
}

#[tauri::command]
pub(crate) fn list_model_configs(app: tauri::AppHandle) -> Vec<ModelConfig> {
    let settings = get_settings(app);
    settings.models
}

/// 检查模型配置路径是否有效
/// 返回: (model_path_valid, model_path_error, server_path_valid, server_path_error)
#[tauri::command]
pub(crate) fn check_model_paths(
    model_path: String,
    server_path: String,
) -> Result<(bool, String, bool, String), String> {
    let mut model_path_valid = false;
    let mut model_path_error = String::new();
    let mut server_path_valid = false;
    let mut server_path_error = String::new();

    // 检查模型文件路径
    if model_path.trim().is_empty() {
        model_path_error = "模型文件路径不能为空".to_string();
    } else {
        let path = std::path::Path::new(model_path.trim());
        if path.exists() {
            if path.is_file() {
                // 检查是否是 .gguf 文件（可选）
                if let Some(ext) = path.extension() {
                    if ext == "gguf" {
                        model_path_valid = true;
                    } else {
                        model_path_error =
                            format!("文件扩展名不是 .gguf，当前为: {}", ext.to_string_lossy());
                    }
                } else {
                    model_path_valid = true; // 没有扩展名也允许
                }
            } else {
                model_path_error = "路径指向的是目录，不是文件".to_string();
            }
        } else {
            model_path_error = "文件不存在".to_string();
        }
    }

    // 检查服务器路径
    let server_bin = server_path.trim();
    if server_bin.is_empty() {
        // 空则默认使用 'llama'，检查是否在 PATH 中
        if which::which("llama").is_ok() {
            server_path_valid = true;
        } else {
            server_path_error =
                "未找到 'llama' 命令，请确保已安装 llama.cpp 并添加到 PATH".to_string();
        }
    } else {
        let path = std::path::Path::new(server_bin);
        if path.exists() {
            if path.is_file() {
                server_path_valid = true;
            } else {
                // 可能是目录，尝试在其中找 llama 或 llama-server
                let llama_path = path.join("llama");
                let llama_server_path = path.join("llama-server");
                if llama_path.exists() || llama_server_path.exists() {
                    server_path_valid = true;
                } else {
                    server_path_error = "目录中未找到 llama 或 llama-server 可执行文件".to_string();
                }
            }
        } else {
            // 尝试作为命令名在 PATH 中查找
            if which::which(server_bin).is_ok() {
                server_path_valid = true;
            } else {
                server_path_error = format!("未找到可执行文件: {}", server_bin);
            }
        }
    }

    Ok((
        model_path_valid,
        model_path_error,
        server_path_valid,
        server_path_error,
    ))
}

/// 扫描目录中的 .gguf 模型文件
/// 返回找到的模型文件列表：路径、文件名、大小
#[derive(Debug, Serialize, Deserialize, Clone)]
pub struct ModelFileInfo {
    pub path: String,
    pub filename: String,
    pub size_bytes: u64,
    pub size_display: String,
}

#[tauri::command]
pub(crate) fn scan_model_directory(dir: String) -> Result<Vec<ModelFileInfo>, String> {
    let dir_path = std::path::Path::new(&dir);
    if !dir_path.exists() {
        return Err("目录不存在".to_string());
    }
    if !dir_path.is_dir() {
        return Err("路径不是目录".to_string());
    }

    let mut results = Vec::new();
    let entries = std::fs::read_dir(dir_path).map_err(|e| format!("读取目录失败: {}", e))?;

    for entry in entries {
        let entry = entry.map_err(|e| format!("读取目录条目失败: {}", e))?;
        let path = entry.path();

        if path.is_file() {
            if let Some(ext) = path.extension() {
                if ext == "gguf" {
                    let filename = path
                        .file_name()
                        .map(|n| n.to_string_lossy().to_string())
                        .unwrap_or_default();
                    let size_bytes = std::fs::metadata(&path).map(|m| m.len()).unwrap_or(0);
                    let size_display = format_file_size(size_bytes);

                    results.push(ModelFileInfo {
                        path: path.to_string_lossy().to_string(),
                        filename,
                        size_bytes,
                        size_display,
                    });
                }
            }
        }
    }

    // 按文件名排序
    results.sort_by(|a, b| a.filename.cmp(&b.filename));

    Ok(results)
}

pub(crate) fn format_file_size(bytes: u64) -> String {
    const UNITS: &[&str] = &["B", "KB", "MB", "GB", "TB"];
    if bytes == 0 {
        return "0 B".to_string();
    }
    let i = (bytes as f64).log(1024.0).floor() as usize;
    if i >= UNITS.len() {
        return format!("{} TB", bytes as f64 / 1099511627776.0);
    }
    let value = bytes as f64 / (1024u64.pow(i as u32) as f64);
    format!("{:.1} {}", value, UNITS[i])
}

#[tauri::command]
pub(crate) fn save_model_config(app: tauri::AppHandle, model: ModelConfig) -> Result<(), String> {
    let mut settings = get_settings(app.clone());
    let model_id = model.id.clone();
    // 检查 ID 是否冲突（排除自己）
    if let Some(pos) = settings.models.iter().position(|m| m.id == model_id) {
        settings.models[pos] = model;
    } else {
        settings.models.push(model);
    }
    save_settings(app, settings)
}

#[tauri::command]
pub(crate) fn delete_model_config(app: tauri::AppHandle, id: String) -> Result<(), String> {
    let mut settings = get_settings(app.clone());
    settings.models.retain(|m| m.id != id);
    save_settings(app, settings)
}
