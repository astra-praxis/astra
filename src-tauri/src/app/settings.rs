use serde::{Deserialize, Serialize};
use std::fs;
use std::process::Command;
use tauri::Manager;

use crate::app::llama_server::ModelConfig;

#[derive(Debug, Serialize, Deserialize, Clone)]
pub(crate) struct EditorSetting {
    name: String,
    command: String,
}

#[derive(Debug, Serialize, Deserialize, Clone)]
pub(crate) struct WorkspaceConfig {
    name: String,
    path: String,
}

pub(crate) fn default_scan_depth() -> u32 {
    3
}

#[derive(Debug, Serialize, Deserialize, Clone)]
pub(crate) struct DevModeFavorite {
    path: String,
    name: String,
}

#[derive(Debug, Serialize, Deserialize, Clone)]
pub(crate) struct AppSettings {
    pub(crate) editor: EditorSetting,
    pub(crate) workspaces: Vec<WorkspaceConfig>,
    pub(crate) active_workspace: Option<String>,
    #[serde(default = "default_scan_depth")]
    pub(crate) scan_depth: u32,
    #[serde(default)]
    pub(crate) models: Vec<ModelConfig>,
    #[serde(default)]
    pub(crate) favorite_dev_mode: Option<DevModeFavorite>,
    /// Agent search_web 的搜索引擎（bing | duckduckgo | searxng；默认 bing）
    #[serde(default)]
    pub(crate) search_engine: Option<String>,
    /// SearXNG 实例地址（search_engine = searxng 时使用；仅配置了它而未选
    /// 引擎的旧设置仍走 SearXNG）
    #[serde(default)]
    pub(crate) search_endpoint: Option<String>,
    /// 记忆 agent 路径总开关：true = 宿主 agent LLM 负责会话总结、记忆
    /// 提炼（W1）与 supersedes 语义确认（W2），各自失败自动降级（规则
    /// 合并 / 规则提炼 / 不建链）；false = 全部纯规则（默认）。重启后生效。
    #[serde(default)]
    pub(crate) memory_agent_summary: bool,
    /// agent 摘要使用的 provider id；None = 自动选第一个配置齐全的
    #[serde(default)]
    pub(crate) memory_summary_provider_id: Option<String>,
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
            favorite_dev_mode: None,
            search_engine: None,
            search_endpoint: None,
            memory_agent_summary: false,
            memory_summary_provider_id: None,
        }
    }
}

// ===== 设置文件路径 =====

pub(crate) fn get_settings_path(app: &tauri::AppHandle) -> Result<std::path::PathBuf, String> {
    let config_dir = app
        .path()
        .app_data_dir()
        .map_err(|e| format!("获取应用数据目录失败: {}", e))?;
    fs::create_dir_all(&config_dir).map_err(|e| format!("创建配置目录失败: {}", e))?;
    Ok(config_dir.join("settings.json"))
}
// ===== 命令：获取设置 =====

#[tauri::command]
pub(crate) fn get_settings(app: tauri::AppHandle) -> AppSettings {
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

/// 已配置的工作空间根目录（多级放权 Workspace 级别的判定边界）
pub(crate) fn workspace_roots(app: &tauri::AppHandle) -> Vec<std::path::PathBuf> {
    get_settings(app.clone())
        .workspaces
        .into_iter()
        .map(|w| std::path::PathBuf::from(w.path))
        .collect()
}
// ===== 命令：保存设置 =====

#[tauri::command]
pub(crate) fn save_settings(app: tauri::AppHandle, settings: AppSettings) -> Result<(), String> {
    let path = get_settings_path(&app)?;
    let json =
        serde_json::to_string_pretty(&settings).map_err(|e| format!("序列化设置失败: {}", e))?;
    fs::write(&path, json).map_err(|e| format!("写入设置文件失败: {}", e))?;
    Ok(())
}

// ===== 命令：设置收藏的开发模式 =====

#[tauri::command]
pub(crate) fn set_favorite_dev_mode(
    app: tauri::AppHandle,
    path: Option<String>,
    name: Option<String>,
) -> Result<AppSettings, String> {
    let mut settings = get_settings(app.clone());
    settings.favorite_dev_mode = path.map(|p| DevModeFavorite {
        path: p,
        name: name.unwrap_or_else(|| "开发模式".to_string()),
    });
    let path = get_settings_path(&app)?;
    let json =
        serde_json::to_string_pretty(&settings).map_err(|e| format!("序列化设置失败: {}", e))?;
    fs::write(&path, json).map_err(|e| format!("写入设置文件失败: {}", e))?;
    Ok(settings)
}

#[tauri::command]
pub(crate) fn add_workspace(
    app: tauri::AppHandle,
    name: String,
    path: String,
) -> Result<AppSettings, String> {
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
pub(crate) fn remove_workspace(app: tauri::AppHandle, path: String) -> Result<AppSettings, String> {
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
pub(crate) fn set_active_workspace(
    app: tauri::AppHandle,
    path: String,
) -> Result<AppSettings, String> {
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
pub(crate) fn open_in_editor(path: String, editor_command: String) -> Result<(), String> {
    // 补充 PATH（打包后可能不完整）：Windows 直接沿用（分隔符/路径不同，
    // Unix 目录无意义），Unix 补常见目录并用 join_paths 按平台拼接
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

#[tauri::command]
pub(crate) fn get_preset_editors() -> Vec<EditorSetting> {
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
