mod chat;
mod chat_sessions;
mod command_runner;
mod providers;

use serde::{Deserialize, Serialize};
use std::fs;
use std::path::Path;
use std::process::{Child, Command, Stdio};
use std::sync::Mutex;
use tauri::Manager;

// ===== llama-server 管理 =====

struct RunningServer {
    child: Child,
    port: u16,
    model_name: String,
    model_path: String,
    started_at: u64,
    pid: u32,
}

type ServerState = Mutex<Vec<RunningServer>>;

#[derive(Debug, Serialize, Deserialize, Clone)]
struct ModelConfig {
    id: String,
    name: String,
    model_path: String,
    server_path: String,
    port: u16,
    ngl: u32,
}

#[derive(Debug, Serialize)]
struct RunningModelInfo {
    #[serde(flatten)]
    config: ModelConfig,
    status: String,
    pid: Option<u32>,
    started_at: u64,
}

fn unix_timestamp() -> u64 {
    std::time::SystemTime::now()
        .duration_since(std::time::UNIX_EPOCH)
        .map(|d| d.as_secs())
        .unwrap_or(0)
}

fn default_port() -> u16 {
    8080
}

fn default_ngl() -> u32 {
    999
}

// ===== 类型定义（保持原有）=====

#[derive(Debug, Serialize, Clone)]
struct SubProject {
    name: String,
    path: String,
}

#[derive(Debug, Serialize)]
struct ProjectCard {
    name: String,
    path: String,
    has_readme: bool,
    sub_projects: Vec<SubProject>,
}

#[derive(Debug, Serialize, Deserialize, Clone)]
struct EditorSetting {
    name: String,
    command: String,
}

#[derive(Debug, Serialize, Deserialize, Clone)]
struct WorkspaceConfig {
    name: String,
    path: String,
}

fn default_scan_depth() -> u32 {
    3
}

#[derive(Debug, Serialize, Deserialize, Clone)]
struct AppSettings {
    editor: EditorSetting,
    workspaces: Vec<WorkspaceConfig>,
    active_workspace: Option<String>,
    #[serde(default = "default_scan_depth")]
    scan_depth: u32,
    #[serde(default)]
    models: Vec<ModelConfig>,
}

impl Default for AppSettings {
    fn default() -> Self {
        Self {
            editor: EditorSetting {
                name: "VS Code".to_string(),
                command: "code".to_string(),
            },
            workspaces: Vec::new(),
            active_workspace: None,
            scan_depth: 3,
            models: Vec::new(),
        }
    }
}

// ===== 设置文件路径 =====

fn get_settings_path(app: &tauri::AppHandle) -> Result<std::path::PathBuf, String> {
    let config_dir = app
        .path()
        .app_data_dir()
        .map_err(|e| format!("获取应用数据目录失败: {}", e))?;
    fs::create_dir_all(&config_dir).map_err(|e| format!("创建配置目录失败: {}", e))?;
    Ok(config_dir.join("settings.json"))
}

// ===== 命令：扫描工作空间 =====

#[tauri::command]
fn scan_workspace(path: String) -> Result<Vec<ProjectCard>, String> {
    let dir = Path::new(&path);

    if !dir.is_dir() {
        return Err(format!("路径不是有效的目录: {}", path));
    }

    let entries = fs::read_dir(dir).map_err(|e| format!("读取目录失败: {}", e))?;

    let mut projects = Vec::new();

    for entry in entries {
        let entry = entry.map_err(|e| format!("读取条目失败: {}", e))?;
        let entry_path = entry.path();

        if !entry_path.is_dir() {
            continue;
        }

        // 跳过隐藏目录（以 . 开头）
        let folder_name = entry_path
            .file_name()
            .unwrap_or_default()
            .to_string_lossy()
            .to_string();

        if folder_name.starts_with('.') {
            continue;
        }

        // 检查 README.md
        let readme_path = entry_path.join("README.md");
        let has_readme = readme_path.exists();

        let project_name = if has_readme {
            // 读取 README.md 的第一行作为项目名称
            match fs::read_to_string(&readme_path) {
                Ok(content) => content
                    .lines()
                    .next()
                    .unwrap_or(&folder_name)
                    .trim()
                    .trim_start_matches("# ")
                    .trim()
                    .to_string(),
                Err(_) => folder_name.clone(),
            }
        } else {
            // 如果没有 README.md，自动生成一个
            let readme_content =
                format!("# {}\n\nThis is the {} project.", folder_name, folder_name);
            if let Err(e) = fs::write(&readme_path, &readme_content) {
                eprintln!("创建 README.md 失败: {}", e);
            }
            folder_name.clone()
        };

        projects.push(ProjectCard {
            name: project_name,
            path: entry_path.to_string_lossy().to_string(),
            has_readme,
            sub_projects: scan_sub_projects(&entry_path),
        });
    }

    // 按名称排序
    projects.sort_by(|a, b| a.name.to_lowercase().cmp(&b.name.to_lowercase()));

    Ok(projects)
}

// ===== 扫描项目子目录 =====

fn scan_sub_projects(project_dir: &Path) -> Vec<SubProject> {
    let mut subs = Vec::new();

    let entries = match fs::read_dir(project_dir) {
        Ok(e) => e,
        Err(_) => return subs,
    };

    for entry in entries.flatten() {
        let entry_path = entry.path();

        if !entry_path.is_dir() {
            continue;
        }

        let folder_name = match entry_path.file_name() {
            Some(n) => n.to_string_lossy().to_string(),
            None => continue,
        };

        if folder_name.starts_with('.') || folder_name == "node_modules" {
            continue;
        }

        subs.push(SubProject {
            name: folder_name,
            path: entry_path.to_string_lossy().to_string(),
        });
    }

    subs.sort_by(|a, b| a.name.to_lowercase().cmp(&b.name.to_lowercase()));
    subs
}

// ===== 获取 Git 远程地址 =====

fn get_git_remote_url(repo_path: &Path) -> Option<String> {
    let output = Command::new("git")
        .args([
            "-C",
            &repo_path.to_string_lossy(),
            "remote",
            "get-url",
            "origin",
        ])
        .output()
        .ok()?;
    if output.status.success() {
        let url = String::from_utf8_lossy(&output.stdout).trim().to_string();
        if !url.is_empty() {
            Some(url)
        } else {
            None
        }
    } else {
        None
    }
}

// ===== 项目详情 =====

#[derive(Debug, Serialize)]
struct GitRepo {
    name: String,
    path: String,
    remote_url: Option<String>,
}

#[derive(Debug, Serialize)]
struct SubDetail {
    name: String,
    path: String,
    git_repo: Option<GitRepo>,
    children: Vec<SubDetail>,
    readme_preview: String,
    depth: u32,
}

#[derive(Debug, Serialize)]
struct ProjectDetail {
    name: String,
    path: String,
    has_readme: bool,
    readme_preview: String,
    sub_items: Vec<SubDetail>,
}

fn read_readme_preview(dir: &Path) -> String {
    let readme_path = dir.join("README.md");
    if !readme_path.exists() {
        return String::new();
    }
    match fs::read_to_string(&readme_path) {
        Ok(content) => content.chars().take(1000).collect::<String>(),
        Err(_) => String::new(),
    }
}

// ===== 递归扫描子目录（用于详情视图）=====

fn scan_subdirectory(dir: &Path, current_depth: u32, max_depth: u32) -> Vec<SubDetail> {
    if current_depth >= max_depth {
        return Vec::new();
    }

    let mut items = Vec::new();
    let entries = match fs::read_dir(dir) {
        Ok(e) => e,
        Err(_) => return items,
    };

    for entry in entries.flatten() {
        let entry_path = entry.path();
        if !entry_path.is_dir() {
            continue;
        }

        let name = match entry_path.file_name() {
            Some(n) => n.to_string_lossy().to_string(),
            None => continue,
        };

        if name.starts_with('.') || name == "node_modules" {
            continue;
        }

        let git_repo = get_git_remote_url(&entry_path).map(|url| GitRepo {
            name: name.clone(),
            path: entry_path.to_string_lossy().to_string(),
            remote_url: Some(url),
        });

        // 如果目录本身有 .git 子目录，说明这是一个 git 仓库根目录，不再遍历其子目录
        let has_git_dir = entry_path.join(".git").exists();
        let children = if has_git_dir {
            Vec::new()
        } else {
            scan_subdirectory(&entry_path, current_depth + 1, max_depth)
        };

        items.push(SubDetail {
            name,
            path: entry_path.to_string_lossy().to_string(),
            git_repo,
            children,
            readme_preview: read_readme_preview(&entry_path),
            depth: current_depth + 1,
        });
    }

    items.sort_by(|a, b| a.name.to_lowercase().cmp(&b.name.to_lowercase()));
    items
}

#[tauri::command]
fn get_project_detail(app: tauri::AppHandle, path: String) -> Result<ProjectDetail, String> {
    let settings = get_settings(app);
    let max_depth = settings.scan_depth;

    let dir = Path::new(&path);
    if !dir.is_dir() {
        return Err("项目目录不存在".to_string());
    }

    let folder_name = dir
        .file_name()
        .unwrap_or_default()
        .to_string_lossy()
        .to_string();

    let readme_path = dir.join("README.md");
    let (project_name, readme_preview, has_readme) = if readme_path.exists() {
        match fs::read_to_string(&readme_path) {
            Ok(content) => {
                let first_line = content
                    .lines()
                    .next()
                    .unwrap_or(&folder_name)
                    .trim()
                    .trim_start_matches("# ")
                    .trim()
                    .to_string();
                let preview = content.chars().take(300).collect::<String>();
                (
                    if first_line.is_empty() {
                        folder_name.clone()
                    } else {
                        first_line
                    },
                    preview,
                    true,
                )
            }
            Err(_) => (folder_name.clone(), String::new(), false),
        }
    } else {
        (folder_name.clone(), String::new(), false)
    };

    // 如果项目根目录本身就有 .git，说明整个项目就是一个 git 仓库，不遍历子目录
    let sub_items = if dir.join(".git").exists() {
        Vec::new()
    } else {
        scan_subdirectory(&dir, 0, max_depth)
    };

    Ok(ProjectDetail {
        name: project_name,
        path: dir.to_string_lossy().to_string(),
        has_readme,
        readme_preview,
        sub_items,
    })
}

// ===== 命令：创建新项目 =====

#[tauri::command]
fn create_project(
    workspace_path: String,
    folder_name: String,
    project_name: String,
) -> Result<ProjectCard, String> {
    // 验证文件夹名称
    if folder_name.trim().is_empty() {
        return Err("文件夹名称不能为空".to_string());
    }

    let folder_name = folder_name.trim();

    // 不允许以 . 开头（隐藏目录）
    if folder_name.starts_with('.') {
        return Err("文件夹名称不能以 . 开头".to_string());
    }

    // 不允许包含路径分隔符
    if folder_name.contains('/') || folder_name.contains('\\') {
        return Err("文件夹名称不能包含路径分隔符".to_string());
    }

    let project_dir = Path::new(&workspace_path).join(folder_name);

    // 检查目录是否已存在
    if project_dir.exists() {
        return Err(format!("文件夹 '{}' 已存在", folder_name));
    }

    // 创建目录
    fs::create_dir_all(&project_dir).map_err(|e| format!("创建目录失败: {}", e))?;

    let project_name = if project_name.trim().is_empty() {
        folder_name.to_string()
    } else {
        project_name.trim().to_string()
    };

    // 写入 README.md
    let readme_content = format!("# {}\n\nThis is the {} project.", project_name, folder_name);
    let readme_path = project_dir.join("README.md");
    fs::write(&readme_path, &readme_content).map_err(|e| format!("创建 README.md 失败: {}", e))?;

    Ok(ProjectCard {
        name: project_name,
        path: project_dir.to_string_lossy().to_string(),
        has_readme: true,
        sub_projects: scan_sub_projects(&project_dir),
    })
}

// ===== 命令：获取设置 =====

#[tauri::command]
fn get_settings(app: tauri::AppHandle) -> AppSettings {
    let path = match get_settings_path(&app) {
        Ok(p) => p,
        Err(_) => return AppSettings::default(),
    };

    if !path.exists() {
        return AppSettings::default();
    }

    match fs::read_to_string(&path) {
        Ok(content) => serde_json::from_str(&content).unwrap_or_default(),
        Err(_) => AppSettings::default(),
    }
}

// ===== 命令：保存设置 =====

#[tauri::command]
fn save_settings(app: tauri::AppHandle, settings: AppSettings) -> Result<(), String> {
    let path = get_settings_path(&app)?;
    let json =
        serde_json::to_string_pretty(&settings).map_err(|e| format!("序列化设置失败: {}", e))?;
    fs::write(&path, json).map_err(|e| format!("写入设置文件失败: {}", e))?;
    Ok(())
}

// ===== 工作空间管理 =====

#[tauri::command]
fn add_workspace(app: tauri::AppHandle, name: String, path: String) -> Result<AppSettings, String> {
    let mut settings = get_settings(app.clone());
    // 检查是否已存在相同路径
    if settings.workspaces.iter().any(|w| w.path == path) {
        return Err("该路径已是工作空间".to_string());
    }
    settings.workspaces.push(WorkspaceConfig {
        name,
        path: path.clone(),
    });
    // 如果是第一个工作空间，自动设为活跃
    if settings.active_workspace.is_none() {
        settings.active_workspace = Some(path);
    }
    let path = get_settings_path(&app)?;
    let json = serde_json::to_string_pretty(&settings).map_err(|e| e.to_string())?;
    fs::write(&path, json).map_err(|e| e.to_string())?;
    Ok(settings)
}

#[tauri::command]
fn remove_workspace(app: tauri::AppHandle, path: String) -> Result<AppSettings, String> {
    let mut settings = get_settings(app.clone());
    settings.workspaces.retain(|w| w.path != path);
    if settings.active_workspace.as_ref() == Some(&path) {
        settings.active_workspace = settings.workspaces.first().map(|w| w.path.clone());
    }
    let save_path = get_settings_path(&app)?;
    let json = serde_json::to_string_pretty(&settings).map_err(|e| e.to_string())?;
    fs::write(&save_path, json).map_err(|e| e.to_string())?;
    Ok(settings)
}

#[tauri::command]
fn set_active_workspace(app: tauri::AppHandle, path: String) -> Result<AppSettings, String> {
    let mut settings = get_settings(app.clone());
    if !settings.workspaces.iter().any(|w| w.path == path) {
        return Err("工作空间不存在".to_string());
    }
    settings.active_workspace = Some(path);
    let save_path = get_settings_path(&app)?;
    let json = serde_json::to_string_pretty(&settings).map_err(|e| e.to_string())?;
    fs::write(&save_path, json).map_err(|e| e.to_string())?;
    Ok(settings)
}

// ===== 命令：在编辑器中打开项目 =====

#[tauri::command]
fn open_in_editor(path: String, editor_command: String) -> Result<(), String> {
    // 打包后 PATH 可能不完整，补充常见编辑器安装路径
    let existing_path = std::env::var("PATH").unwrap_or_default();
    let extra_paths = [
        "/usr/local/bin",
        "/opt/homebrew/bin",
        "/opt/local/bin",
        "/usr/bin",
        "/bin",
    ];
    let full_path = if existing_path.is_empty() {
        extra_paths.join(":")
    } else {
        let mut parts: Vec<&str> = extra_paths.iter().map(|s| *s).collect();
        parts.push(&existing_path);
        parts.join(":")
    };

    Command::new(&editor_command)
        .arg(&path)
        .env("PATH", &full_path)
        .spawn()
        .map_err(|e| {
            format!(
                "启动编辑器失败: {}（请确认 {} 已安装且在系统 PATH 中）",
                e, editor_command
            )
        })?;
    Ok(())
}

// ===== 命令：最小化到托盘 =====

#[tauri::command]
fn minimize_to_tray(window: tauri::WebviewWindow) -> Result<(), String> {
    window.hide().map_err(|e| e.to_string())?;
    Ok(())
}

// ===== 命令： llama-server 管理 =====

#[tauri::command]
fn start_llama_server(
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

    // 打包后 PATH 可能不完整，补充常见安装路径
    let existing_path = std::env::var("PATH").unwrap_or_default();
    let extra_paths = [
        "/usr/local/bin",
        "/opt/homebrew/bin",
        "/opt/local/bin",
        "/usr/bin",
        "/bin",
        "/Users/hanbiao/.llama-app",
    ];
    let full_path = if existing_path.is_empty() {
        extra_paths.join(":")
    } else {
        let mut parts: Vec<&str> = extra_paths.iter().map(|s| *s).collect();
        parts.push(&existing_path);
        parts.join(":")
    };

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
fn stop_llama_server(app: tauri::AppHandle, port: u16) -> Result<bool, String> {
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
fn list_running_servers(app: tauri::AppHandle) -> Result<Vec<RunningModelInfo>, String> {
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
fn check_server_status(app: tauri::AppHandle, port: u16) -> Result<Option<String>, String> {
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
fn list_model_configs(app: tauri::AppHandle) -> Vec<ModelConfig> {
    let settings = get_settings(app);
    settings.models
}

#[tauri::command]
fn save_model_config(app: tauri::AppHandle, model: ModelConfig) -> Result<(), String> {
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
fn delete_model_config(app: tauri::AppHandle, id: String) -> Result<(), String> {
    let mut settings = get_settings(app.clone());
    settings.models.retain(|m| m.id != id);
    save_settings(app, settings)
}

// ===== 命令：窗口拖拽 =====

#[tauri::command]
fn drag_window(app: tauri::AppHandle) -> Result<(), String> {
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
fn set_window_background(app: tauri::AppHandle, is_dark: bool) -> Result<(), String> {
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

#[tauri::command]
fn get_preset_editors() -> Vec<EditorSetting> {
    vec![
        EditorSetting {
            name: "VS Code".to_string(),
            command: "code".to_string(),
        },
        EditorSetting {
            name: "Zed".to_string(),
            command: "zed".to_string(),
        },
        EditorSetting {
            name: "Cursor".to_string(),
            command: "cursor".to_string(),
        },
        EditorSetting {
            name: "WebStorm".to_string(),
            command: "webstorm".to_string(),
        },
        EditorSetting {
            name: "IntelliJ IDEA".to_string(),
            command: "idea".to_string(),
        },
        EditorSetting {
            name: "Sublime Text".to_string(),
            command: "subl".to_string(),
        },
        EditorSetting {
            name: "Atom".to_string(),
            command: "atom".to_string(),
        },
        EditorSetting {
            name: "Windsurf".to_string(),
            command: "windsurf".to_string(),
        },
    ]
}

// ===== 应用入口 =====

#[cfg_attr(mobile, tauri::mobile_entry_point)]
pub fn run() {
    tauri::Builder::default()
        .manage(Mutex::new(Vec::<RunningServer>::new()))
        .manage(chat::ChatStopFlag(std::sync::atomic::AtomicBool::new(
            false,
        )))
        .plugin(tauri_plugin_opener::init())
        .plugin(tauri_plugin_dialog::init())
        .plugin(tauri_plugin_fs::init())
        .plugin(
            tauri_plugin_global_shortcut::Builder::new()
                .with_handler(|app, _shortcut, event| {
                    if event.state == tauri_plugin_global_shortcut::ShortcutState::Pressed {
                        if let Some(window) = app.get_webview_window("main") {
                            let _ = window.show();
                            let _ = window.set_focus();
                        }
                    }
                })
                .build(),
        )
        .setup(|app| {
            use tauri::{
                menu::{Menu, MenuItem},
                tray::TrayIconBuilder,
                Manager,
            };
            use tauri_plugin_global_shortcut::GlobalShortcutExt;

            let show_item = MenuItem::with_id(app, "show", "显示窗口", true, Some("Alt+Space"))?;
            let separator = tauri::menu::PredefinedMenuItem::separator(app)?;
            let quit_item = MenuItem::with_id(app, "quit", "退出", true, Some("CmdOrCtrl+Q"))?;
            let menu = Menu::with_items(app, &[&show_item, &separator, &quit_item])?;

            let _tray = TrayIconBuilder::new()
                .icon(app.default_window_icon().unwrap().clone())
                .tooltip("星野")
                .menu(&menu)
                .on_menu_event(|app, event| match event.id.as_ref() {
                    "show" => {
                        if let Some(window) = app.get_webview_window("main") {
                            let _ = window.show();
                            let _ = window.set_focus();
                        }
                    }
                    "quit" => {
                        app.exit(0);
                    }
                    _ => {}
                })
                .on_tray_icon_event(|tray, event| {
                    use tauri::tray::{MouseButton, MouseButtonState, TrayIconEvent};
                    if let TrayIconEvent::Click {
                        button: MouseButton::Left,
                        button_state: MouseButtonState::Up,
                        ..
                    } = event
                    {
                        let app = tray.app_handle();
                        if let Some(window) = app.get_webview_window("main") {
                            let _ = window.show();
                            let _ = window.set_focus();
                        }
                    }
                })
                .build(app)?;

            // 注册全局快捷键 Alt+Space 唤醒窗口
            app.global_shortcut().register("Alt+Space")?;

            Ok(())
        })
        .on_window_event(|window, event| {
            if let tauri::WindowEvent::CloseRequested { api, .. } = event {
                // 点击关闭按钮时不退出，而是隐藏到托盘
                window.hide().ok();
                api.prevent_close();
            }
        })
        .invoke_handler(tauri::generate_handler![
            greet,
            scan_workspace,
            create_project,
            get_settings,
            save_settings,
            open_in_editor,
            command_runner::run_command,
            get_preset_editors,
            list_skills,
            delete_skill,
            read_skill_doc,
            get_project_detail,
            add_workspace,
            remove_workspace,
            set_active_workspace,
            minimize_to_tray,
            drag_window,
            set_window_background,
            start_llama_server,
            stop_llama_server,
            list_running_servers,
            check_server_status,
            list_model_configs,
            save_model_config,
            delete_model_config,
            // Providers
            providers::list_providers,
            providers::save_provider,
            providers::delete_provider,
            providers::fetch_provider_models,
            providers::set_active_model,
            providers::get_provider,
            // Chat
            chat::send_chat,
            chat::stop_chat,
            // Chat Sessions
            chat_sessions::list_chat_sessions,
            chat_sessions::save_chat_session,
            chat_sessions::update_chat_session_title,
            chat_sessions::delete_chat_session,
            chat_sessions::get_chat_session,
        ])
        .run(tauri::generate_context!())
        .expect("error while running tauri application");
}

// ===== 保留默认 greet =====

#[tauri::command]
fn greet(name: &str) -> String {
    format!("Hello, {}! You've been greeted from Rust!", name)
}

// ===== Skills 管理 =====

#[derive(Debug, Serialize)]
struct SkillCard {
    name: String,
    description: String,
    path: String,
    version: String,
}

/// 解析 SKILL.md 的 YAML frontmatter
fn parse_skill_frontmatter(content: &str) -> (String, String, String) {
    let mut name = String::new();
    let mut description = String::new();
    let mut version = String::new();

    let content = content.trim();
    if !content.starts_with("---") {
        return (name, description, version);
    }

    let after_first = content.strip_prefix("---").unwrap_or("");
    if let Some(end) = after_first.find("---") {
        let frontmatter = &after_first[..end];
        let mut in_description = false;
        let mut desc_parts: Vec<String> = Vec::new();

        for line in frontmatter.lines() {
            if in_description {
                let trimmed = line.trim();
                if trimmed.is_empty()
                    || !trimmed.starts_with(' ')
                        && !trimmed.starts_with(|c: char| c.is_whitespace())
                {
                    // check if this is a new key
                    if trimmed.contains(':') && !trimmed.starts_with(' ') {
                        in_description = false;
                        // parse as key... but just fall through
                    } else {
                        desc_parts.push(trimmed.to_string());
                    }
                } else {
                    desc_parts.push(trimmed.to_string());
                }
                continue;
            }

            if let Some(val) = line.strip_prefix("name:") {
                name = val.trim().trim_matches('"').to_string();
            } else if let Some(val) = line.strip_prefix("description:") {
                let val = val.trim();
                if val.starts_with('"') {
                    description = val.trim_matches('"').to_string();
                } else if val.starts_with(">-") || val.starts_with('|') {
                    in_description = true;
                } else if !val.is_empty() {
                    description = val.to_string();
                } else {
                    in_description = true;
                }
            } else if let Some(val) = line.strip_prefix("version:") {
                version = val.trim().trim_matches('"').to_string();
            }
        }

        if !desc_parts.is_empty() {
            description = desc_parts.join(" ");
        }
    }

    (name, description, version)
}

#[tauri::command]
fn list_skills() -> Result<Vec<SkillCard>, String> {
    let home = dirs::home_dir().ok_or("无法获取用户主目录")?;
    let skills_dir = home.join(".agents").join("skills");

    if !skills_dir.exists() {
        return Ok(Vec::new());
    }

    let entries = fs::read_dir(&skills_dir).map_err(|e| format!("读取 skills 目录失败: {}", e))?;

    let mut skills = Vec::new();

    for entry in entries {
        let entry = entry.map_err(|e| format!("读取条目失败: {}", e))?;
        let entry_path = entry.path();

        if !entry_path.is_dir() {
            continue;
        }

        let folder_name = entry_path
            .file_name()
            .unwrap_or_default()
            .to_string_lossy()
            .to_string();

        if folder_name.starts_with('.') {
            continue;
        }

        let skill_md_path = entry_path.join("SKILL.md");
        let (name, description, version) = if skill_md_path.exists() {
            match fs::read_to_string(&skill_md_path) {
                Ok(content) => parse_skill_frontmatter(&content),
                Err(_) => (folder_name.clone(), String::new(), String::new()),
            }
        } else {
            (folder_name.clone(), String::new(), String::new())
        };

        skills.push(SkillCard {
            name: if name.is_empty() { folder_name } else { name },
            description,
            path: entry_path.to_string_lossy().to_string(),
            version,
        });
    }

    skills.sort_by(|a, b| a.name.to_lowercase().cmp(&b.name.to_lowercase()));
    Ok(skills)
}

#[tauri::command]
fn delete_skill(path: String) -> Result<(), String> {
    let dir = Path::new(&path);
    if !dir.exists() {
        return Err("技能目录不存在".to_string());
    }
    fs::remove_dir_all(dir).map_err(|e| format!("删除失败: {}", e))?;
    Ok(())
}

#[tauri::command]
fn read_skill_doc(path: String) -> Result<String, String> {
    let dir = Path::new(&path);
    if !dir.exists() || !dir.is_dir() {
        return Err("技能目录不存在".to_string());
    }

    // 按优先级尝试读取文档文件
    let candidates = ["SKILL.md", "README.md", "readme.md", "README", "Readme.md"];
    for candidate in &candidates {
        let file_path = dir.join(candidate);
        if file_path.exists() && file_path.is_file() {
            return fs::read_to_string(&file_path).map_err(|e| format!("读取文件失败: {}", e));
        }
    }

    // 尝试任意 .md 文件
    if let Ok(entries) = fs::read_dir(dir) {
        for entry in entries.flatten() {
            let entry_path = entry.path();
            if entry_path.is_file() {
                if let Some(ext) = entry_path.extension() {
                    if ext == "md" {
                        return fs::read_to_string(&entry_path)
                            .map_err(|e| format!("读取文件失败: {}", e));
                    }
                }
            }
        }
    }

    Err("未找到文档文件（SKILL.md / README.md）".to_string())
}
