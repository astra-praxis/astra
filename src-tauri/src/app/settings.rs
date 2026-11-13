use serde::{Deserialize, Serialize};
use std::fs;
use std::process::Command;
// AppHandle 以完全限定路径使用（tauri::AppHandle），无需 use 导入

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
    let config_dir = crate::app::accounts::data_dir(app)?;
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
        Ok(content) => match serde_json::from_str::<AppSettings>(&content) {
            Ok(settings) => settings,
            Err(e) => {
                // 解析失败降级为默认值 = 用户此前配置全丢（数据丢失盲点），
                // 这里只记日志不改行为，方便事后从日志发现。
                app_log!(
                    error,
                    "settings",
                    "get_settings: parse failed, fallback to default: {} ({})",
                    path.display(),
                    e
                );
                AppSettings::default()
            }
        },
        Err(e) => {
            app_log!(
                error,
                "settings",
                "get_settings: read failed, fallback to default: {} ({})",
                path.display(),
                e
            );
            AppSettings::default()
        }
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
    fs::write(&path, json).map_err(|e| {
        app_log!(
            error,
            "settings",
            "save_settings: write failed {}: {}",
            path.display(),
            e
        );
        format!("写入设置文件失败: {}", e)
    })?;
    app_log!(
        info,
        "settings",
        "save_settings ok: {} workspaces, active: {:?}",
        settings.workspaces.len(),
        settings.active_workspace
    );
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
        name: name.clone(),
        path: path.clone(),
    });
    // 如果是第一个工作空间，自动设为活跃
    if settings.active_workspace.is_none() {
        settings.active_workspace = Some(path.clone());
    }
    let save_path = get_settings_path(&app)?;
    let json = serde_json::to_string_pretty(&settings).map_err(|e| e.to_string())?;
    fs::write(&save_path, json).map_err(|e| e.to_string())?;
    app_log!(info, "settings", "add_workspace ok: {} ({})", name, path);
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
    app_log!(info, "settings", "remove_workspace ok: {}", path);
    Ok(settings)
}

#[tauri::command]
pub(crate) fn set_active_workspace(
    app: tauri::AppHandle,
    path: String,
) -> Result<AppSettings, String> {
    let mut settings = get_settings(app.clone());
    if !settings.workspaces.iter().any(|w| w.path == path) {
        app_log!(
            error,
            "settings",
            "set_active_workspace: not found: {}",
            path
        );
        return Err("工作空间不存在".to_string());
    }
    settings.active_workspace = Some(path.clone());
    let save_path = get_settings_path(&app)?;
    let json = serde_json::to_string_pretty(&settings).map_err(|e| e.to_string())?;
    fs::write(&save_path, json).map_err(|e| e.to_string())?;
    app_log!(info, "settings", "set_active_workspace ok: {}", path);
    Ok(settings)
}
// ===== 命令：在编辑器中打开项目 =====

/// 在候选目录中把编辑器命令解析为绝对路径。
/// 不信任 std 的 PATH 查找：macOS 打包后 GUI 应用走 posix_spawnp，
/// 可执行文件搜索用父进程 PATH（.env 修改的子进程 PATH 不参与）；
/// Windows 上 CreateProcess 同样只看父进程 PATH。两平台都自己解析最稳。
fn resolve_editor_command(command: &str, dirs: &[std::path::PathBuf]) -> std::path::PathBuf {
    // 已含路径分隔符：视为明确路径，直接交给 std
    if command.contains('/') || command.contains('\\') {
        let direct = std::path::PathBuf::from(command);
        // 用户可能指到 VS Code 的 bin/code（POSIX sh 脚本），同名 .cmd 才是可执行入口
        if cfg!(windows) {
            if let Some(fixed) = windows_executable_sibling(&direct) {
                return fixed;
            }
        }
        return direct;
    }
    let names = editor_candidate_names(command, cfg!(windows));
    choose_editor_candidate(&names, dirs, cfg!(windows))
        .unwrap_or_else(|| std::path::PathBuf::from(command))
}

/// 在候选目录中按优先级挑选编辑器入口；挑不到返回 None。
///
/// `windows` 作为参数而非 `cfg!` 内联，是为了让 Windows 挑选规则
/// （跳过同名 POSIX 脚本）在任意平台都能被单测覆盖。
fn choose_editor_candidate(
    names: &[String],
    dirs: &[std::path::PathBuf],
    windows: bool,
) -> Option<std::path::PathBuf> {
    for dir in dirs {
        for name in names {
            let candidate = dir.join(name);
            if !candidate.is_file() {
                continue;
            }
            // Windows 下无扩展名候选只接受真正的 PE 镜像，否则继续找：
            // 同名的 sh 脚本无法 CreateProcess
            if windows && candidate.extension().is_none() && !is_pe_image(&candidate) {
                continue;
            }
            return Some(candidate);
        }
    }
    None
}

/// 候选文件名（按优先级）。
///
/// Windows 必须把无扩展名放最后：VS Code / Cursor 的 `bin` 目录里
/// `code`（POSIX sh 脚本）、`code.cmd`、`code.ps1` 并存，先命中脚本会被
/// CreateProcess 拒绝（193「%1 不是有效的 Win32 应用程序」）。
fn editor_candidate_names(command: &str, windows: bool) -> Vec<String> {
    if windows {
        [".exe", ".cmd", ".bat", ""]
            .iter()
            .map(|ext| format!("{command}{ext}"))
            .collect()
    } else {
        vec![command.to_string()]
    }
}

/// Windows 可执行镜像判定：PE 文件以 `MZ` 开头。
/// 用于排除同名非 PE 文件——VS Code / Cursor 的 `bin/code` 是 POSIX sh 脚本，
/// 直接 CreateProcess 会失败（193「%1 不是有效的 Win32 应用程序」）。
fn is_pe_image(path: &std::path::Path) -> bool {
    use std::io::Read;
    std::fs::File::open(path)
        .and_then(|mut f| {
            let mut head = [0u8; 2];
            f.read_exact(&mut head)?;
            Ok(head)
        })
        .map(|head| head == *b"MZ")
        .unwrap_or(false)
}

/// 把「无扩展名且不是 PE」的同名入口纠正为可执行的兄弟文件
/// （`bin/code` → `bin/code.cmd`）。找不到时返回 None，由调用方原样使用。
fn windows_executable_sibling(path: &std::path::Path) -> Option<std::path::PathBuf> {
    if !path.is_file() || path.extension().is_some() || is_pe_image(path) {
        return None;
    }
    for ext in [".cmd", ".bat", ".exe"] {
        let mut sibling = path.as_os_str().to_os_string();
        sibling.push(ext);
        let sibling = std::path::PathBuf::from(sibling);
        if sibling.is_file() {
            return Some(sibling);
        }
    }
    None
}

/// Windows 批处理入口（code.cmd / zed.cmd 等）不能被 CreateProcess 直接执行
/// （Rust std 自 CVE-2024-24576 修复后拒绝 spawn .bat/.cmd，报错 193：
/// 「%1 不是有效的 Win32 应用程序」），必须经 PowerShell 调用解释执行。
/// 唯一调用点在 cfg(windows) 分支内，非 Windows 平台仅由单测使用。
#[cfg_attr(not(windows), allow(dead_code))]
fn needs_cmd_interpreter(editor: &std::path::Path) -> bool {
    editor
        .extension()
        .and_then(|e| e.to_str())
        .map(|e| e.eq_ignore_ascii_case("cmd") || e.eq_ignore_ascii_case("bat"))
        .unwrap_or(false)
}

/// 构造 PowerShell 单引号参数：内部字符全部字面（& % ^ ! 空格不再特殊），
/// 唯一的转义是 `'` 翻倍为 `''`。
/// 唯一调用点在 cfg(windows) 分支内，非 Windows 平台仅由单测使用。
#[cfg_attr(not(windows), allow(dead_code))]
fn powershell_quote(arg: &str) -> String {
    format!("'{}'", arg.replace('\'', "''"))
}

/// 路径解析：绝对路径原样返回；`~` / `~/…` 展开为用户主目录；相对路径以
/// base_dir 为基准 join 并做词法规范化（处理 . 与 ..，不触碰文件系统，目标
/// 不存在也返回预期绝对路径）。无 base_dir 的相对路径拒绝解析——静默落在
/// 进程/编辑器 cwd 的行为不可预测，由调用方传入明确基准（当前工作空间根目录等）。
pub(crate) fn resolve_open_path(path: &str, base_dir: Option<&str>) -> Result<String, String> {
    // `~` / `~/…` 是 shell 惯例，GUI 进程没有 shell 代为展开，手动展开为主目录；
    // `~user` 用户名形式不支持，按普通相对路径走后续逻辑
    if let Some(abs) = expand_tilde(path) {
        return Ok(abs);
    }
    // Windows 盘符 C:\、UNC \\server\share、POSIX / 开头都由 std 判定，不手写字符串判断
    if std::path::Path::new(path).is_absolute() {
        return Ok(path.to_string());
    }
    // file:// 等带协议前缀的不是文件系统路径，拒绝解析（URL 由浏览器链路处理）
    if path.contains("://") {
        return Err(format!("不是本地文件路径（含协议前缀）: {path}"));
    }
    let base = match base_dir {
        Some(b) if !b.trim().is_empty() => std::path::PathBuf::from(b.trim()),
        _ => {
            return Err(format!(
                "相对路径缺少解析基准（base_dir），无法定位: {path}"
            ))
        }
    };
    let mut normalized = base;
    for comp in std::path::Path::new(path).components() {
        match comp {
            std::path::Component::CurDir => {}
            std::path::Component::ParentDir => {
                normalized.pop();
            }
            other => normalized.push(other),
        }
    }
    Ok(normalized.to_string_lossy().into_owned())
}

/// `~` 与 `~/…`（含 Windows 的 `~\`）展开为用户主目录绝对路径；
/// 不是 `~` 开头或主目录不可用时返回 None（`~user` 用户名形式不支持）。
fn expand_tilde(path: &str) -> Option<String> {
    let rest = if path == "~" {
        ""
    } else {
        path.strip_prefix("~/")
            .or_else(|| path.strip_prefix("~\\"))?
    };
    let home = dirs::home_dir()?;
    let buf = if rest.is_empty() {
        home
    } else {
        home.join(rest)
    };
    Some(buf.to_string_lossy().into_owned())
}

#[tauri::command]
pub(crate) fn open_in_editor(
    path: String,
    editor_command: String,
    base_dir: Option<String>,
) -> Result<(), String> {
    // 相对路径先以 base_dir 解析成绝对路径；缺基准直接报错，不再静默交给编辑器 cwd
    let path = resolve_open_path(&path, base_dir.as_deref())?;
    // 候选目录（打包后 GUI 应用继承的 PATH 只有 /usr/bin:/bin 等系统目录）
    let mut dirs: Vec<std::path::PathBuf> = Vec::new();
    #[cfg(not(windows))]
    {
        for dir in [
            "/usr/local/bin",
            "/opt/homebrew/bin",
            "/opt/local/bin",
            "/usr/bin",
            "/bin",
        ] {
            dirs.push(std::path::PathBuf::from(dir));
        }
        // zed 官方安装脚本默认装到 ~/.local/bin，部分工具在 ~/bin
        if let Ok(home) = std::env::var("HOME") {
            dirs.push(std::path::PathBuf::from(&home).join(".local/bin"));
            dirs.push(std::path::PathBuf::from(home).join("bin"));
        }
    }
    // 用户继承的 PATH 排在自定义目录后（自定义目录优先命中）
    if let Some(existing) = std::env::var_os("PATH") {
        dirs.extend(std::env::split_paths(&existing));
    }
    // 拼装失败时退回继承的 PATH，绝不返回空串（空 PATH 会让后续 spawn 全部报 os error 2）
    let full_path = std::env::join_paths(&dirs)
        .unwrap_or_else(|_| std::env::var_os("PATH").unwrap_or_default());

    let editor = resolve_editor_command(&editor_command, &dirs);
    #[cfg(windows)]
    let mut spawn_cmd: Command = if needs_cmd_interpreter(&editor) {
        // 批处理入口（code.cmd 等）不能直接 CreateProcess（错误 193），
        // 经 cmd /C 时路径里的 & % ^ ! 会被 cmd 二次解析（%VAR% 引号内也展开），
        // 所以改用 PowerShell：单引号参数内全部字面，只需把 ' 翻倍转义。
        let script = format!(
            "& {} {}",
            powershell_quote(&editor.display().to_string()),
            powershell_quote(&path)
        );
        let mut cmd = Command::new("powershell");
        cmd.args(["-NoProfile", "-NonInteractive", "-Command", &script])
            .env("PATH", &full_path);
        cmd
    } else {
        let mut cmd = Command::new(&editor);
        cmd.arg(&path).env("PATH", &full_path);
        cmd
    };
    #[cfg(not(windows))]
    let mut spawn_cmd: Command = {
        let mut cmd = Command::new(&editor);
        cmd.arg(&path).env("PATH", &full_path);
        cmd
    };
    // .cmd 包装入口（code.cmd）会先弹 cmd 黑框，统一抑制；
    // 编辑器自身 GUI 窗口不受影响
    crate::proc_util::hide_window(&mut spawn_cmd);
    if let Err(e) = spawn_cmd.spawn() {
        app_log!(
            error,
            "settings",
            "open_in_editor failed: {} via {}: {}",
            path,
            editor_command,
            e
        );
        // Windows 193 = ERROR_BAD_EXE_FORMAT（「%1 不是有效的 Win32 应用程序」）：
        // 典型原因是入口指到了 VS Code 的 bin/code（POSIX sh 脚本），或 .ps1 等
        // 解释器文件。原文只有占位符，对用户没有信息量，换成可操作中文提示。
        let hint = if cfg!(windows) && e.raw_os_error() == Some(193) {
            format!(
                "{} 不是 Windows 可执行程序（多半是 POSIX 脚本或无扩展名入口），请在设置里改指向 code.cmd 或 code.exe",
                editor.display()
            )
        } else {
            format!("请确认 {} 已安装且在系统 PATH 中", editor_command)
        };
        return Err(format!("启动编辑器失败: {}（{}）", e, hint));
    }
    app_log!(
        info,
        "settings",
        "open_in_editor ok: {} via {}",
        path,
        editor_command
    );
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

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn needs_cmd_interpreter_matches_windows_batch_extensions() {
        // Windows 批处理入口需要经 cmd 解释器执行
        assert!(needs_cmd_interpreter(std::path::Path::new("code.cmd")));
        assert!(needs_cmd_interpreter(std::path::Path::new("code.CMD")));
        assert!(needs_cmd_interpreter(std::path::Path::new("zed.BAT")));
        assert!(needs_cmd_interpreter(std::path::Path::new("tool.bat")));
        assert!(needs_cmd_interpreter(std::path::Path::new(
            "C:\\Program Files\\...\\code.cmd"
        )));
        // 可执行文件与无扩展名命令不需要
        assert!(!needs_cmd_interpreter(std::path::Path::new("Code.exe")));
        assert!(!needs_cmd_interpreter(std::path::Path::new("zed")));
        assert!(!needs_cmd_interpreter(std::path::Path::new("/usr/bin/zed")));
        assert!(!needs_cmd_interpreter(std::path::Path::new("")));
        // 同名但非批处理扩展
        assert!(!needs_cmd_interpreter(std::path::Path::new("code.bat.exe")));
        assert!(!needs_cmd_interpreter(std::path::Path::new("cmder")));
    }

    #[test]
    fn powershell_quote_wraps_and_escapes_literals() {
        // 普通路径：单引号包裹，内部字符全部字面
        assert_eq!(
            powershell_quote(r"C:\foo & bar\100%\proj"),
            "'C:\\foo & bar\\100%\\proj'"
        );
        // 引号翻倍：路径里的单引号 -> 两个单引号
        assert_eq!(powershell_quote("a'b"), "'a''b'");
        // 空参数
        assert_eq!(powershell_quote(""), "''");
        // cmd 特殊字符（& % ^ ! 空格）都不需要额外转义
        assert_eq!(powershell_quote("x&y%z^w!q "), "'x&y%z^w!q '");
    }

    #[test]
    fn resolve_open_path_absolute_passthrough() {
        // 绝对路径原样返回，无 base_dir 也不报错
        let abs = std::env::temp_dir().join("astra-open-test.txt");
        let abs_str = abs.to_string_lossy().into_owned();
        assert_eq!(resolve_open_path(&abs_str, None).unwrap(), abs_str);
    }

    #[test]
    fn resolve_open_path_relative_joins_base() {
        let base = std::env::temp_dir();
        let got = resolve_open_path("src/main.rs", Some(base.to_str().unwrap())).unwrap();
        assert!(std::path::Path::new(&got).is_absolute());
        assert!(std::path::Path::new(&got).ends_with("src/main.rs"));
        // ./ 前缀与直接写等价
        let got2 = resolve_open_path("./src/main.rs", Some(base.to_str().unwrap())).unwrap();
        assert_eq!(got, got2);
    }

    #[test]
    fn resolve_open_path_parent_dir_resolves_lexically() {
        let base = std::env::temp_dir();
        let got = resolve_open_path("../README.md", Some(base.to_str().unwrap())).unwrap();
        let expect = base.parent().unwrap().join("README.md");
        assert_eq!(got, expect.to_string_lossy().into_owned());
    }

    #[test]
    fn resolve_open_path_missing_base_is_error() {
        // 相对路径无基准：报错而非静默落到 cwd
        assert!(resolve_open_path("src/main.rs", None).is_err());
        assert!(resolve_open_path("src/main.rs", Some("")).is_err());
        assert!(resolve_open_path("src/main.rs", Some("  ")).is_err());
    }

    #[test]
    fn resolve_open_path_expands_tilde_home() {
        let home = dirs::home_dir().expect("test machine has home dir");
        // 裸 ~ 展开为主目录，无需 base_dir
        assert_eq!(
            resolve_open_path("~", None).unwrap(),
            home.to_string_lossy().into_owned()
        );
        // ~/ 前缀接子路径（Unix 风格）
        let got = resolve_open_path("~/src/main.rs", None).unwrap();
        assert_eq!(
            std::path::Path::new(&got),
            home.join("src/main.rs").as_path()
        );
        // ~/ 优先于 base_dir：即使给了基准也展开到主目录
        let got2 = resolve_open_path("~/notes.txt", Some("/tmp")).unwrap();
        assert_eq!(
            std::path::Path::new(&got2),
            home.join("notes.txt").as_path()
        );
        // ~user 用户名形式不支持：按相对路径处理，无基准时报错
        assert!(resolve_open_path("~someone/x", None).is_err());
        // 路径中间的 ~ 不是前缀，不展开
        assert!(resolve_open_path("a/~b", None).is_err());
    }

    #[test]
    fn resolve_open_path_rejects_url() {
        assert!(resolve_open_path("file:///tmp/a.txt", None).is_err());
        assert!(resolve_open_path("https://example.com/a.txt", Some("/tmp")).is_err());
    }

    #[cfg(not(windows))]
    #[test]
    fn resolve_open_path_clamps_above_root() {
        // .. 超出基准根：词法 pop 在根处钳制（不会拼出越界路径，也不会报错）
        let got = resolve_open_path("../../../../etc/passwd", Some("/tmp")).unwrap();
        assert_eq!(got, "/etc/passwd");
    }

    #[cfg(windows)]
    #[test]
    fn resolve_open_path_windows_drive_and_unc() {
        // 盘符 / UNC 都是绝对路径，无 base_dir 原样透传
        assert_eq!(
            resolve_open_path("C:\\a\\b.txt", None).unwrap(),
            "C:\\a\\b.txt"
        );
        assert_eq!(
            resolve_open_path("\\\\server\\share\\f.txt", None).unwrap(),
            "\\\\server\\share\\f.txt"
        );
        // 相对路径 + Windows 基准：反斜杠分隔正确 join
        let got = resolve_open_path("src\\main.rs", Some("C:\\proj")).unwrap();
        assert_eq!(got, "C:\\proj\\src\\main.rs");
    }

    #[cfg(not(windows))]
    #[test]
    fn resolve_editor_finds_binary_in_candidate_dirs() {
        let dir = tempfile::tempdir().unwrap();
        let bin = dir.path().join("zed");
        std::fs::write(&bin, "#!/bin/sh\n").unwrap();
        #[cfg(unix)]
        {
            use std::os::unix::fs::PermissionsExt;
            std::fs::set_permissions(&bin, std::fs::Permissions::from_mode(0o755)).unwrap();
        }
        let dirs = vec![dir.path().to_path_buf()];
        assert_eq!(
            resolve_editor_command("zed", &dirs),
            bin,
            "候选目录中的命令应解析为绝对路径"
        );
        // 带路径分隔符的命令原样返回，不做目录拼接
        assert_eq!(
            resolve_editor_command("/opt/homebrew/bin/zed", &dirs),
            std::path::PathBuf::from("/opt/homebrew/bin/zed")
        );
        // 解析不到时原样返回（让 std 报 ENOENT）
        assert_eq!(
            resolve_editor_command("not-installed", &dirs),
            std::path::PathBuf::from("not-installed")
        );
    }

    /// VS Code / Cursor 的 bin 布局：无扩展名的 `code` 是 POSIX sh 脚本，与
    /// code.cmd / code.ps1 并存。Windows 上必须命中 code.cmd——选到脚本会被
    /// CreateProcess 拒绝（193「%1 不是有效的 Win32 应用程序」）。
    #[test]
    fn windows_candidate_selection_skips_posix_script() {
        let dir = tempfile::tempdir().unwrap();
        std::fs::write(dir.path().join("code"), "#!/usr/bin/env sh\n").unwrap();
        std::fs::write(dir.path().join("code.ps1"), "# ps1\n").unwrap();
        let code_cmd = dir.path().join("code.cmd");
        std::fs::write(&code_cmd, "@echo off\r\n").unwrap();
        let dirs = vec![dir.path().to_path_buf()];

        let names = editor_candidate_names("code", true);
        assert_eq!(
            names,
            vec![
                "code.exe".to_string(),
                "code.cmd".to_string(),
                "code.bat".to_string(),
                "code".to_string()
            ],
            "无扩展名必须排最后"
        );
        assert_eq!(choose_editor_candidate(&names, &dirs, true), Some(code_cmd));

        // 只有脚本、没有同名可执行入口：Windows 上不能挑脚本，回退为原命令名
        let script_only = tempfile::tempdir().unwrap();
        std::fs::write(script_only.path().join("code"), "#!/usr/bin/env sh\n").unwrap();
        let only = vec![script_only.path().to_path_buf()];
        assert_eq!(choose_editor_candidate(&names, &only, true), None);

        // 真正的 PE 镜像（MZ 头）即使无扩展名也可用
        let pe_dir = tempfile::tempdir().unwrap();
        std::fs::write(pe_dir.path().join("zed"), b"MZ\x90\x00").unwrap();
        let zed_names = editor_candidate_names("zed", true);
        let pe_dirs = vec![pe_dir.path().to_path_buf()];
        assert_eq!(
            choose_editor_candidate(&zed_names, &pe_dirs, true),
            Some(pe_dir.path().join("zed"))
        );
    }

    /// 明确路径指到脚本本身时，纠正为同名 .cmd
    #[test]
    fn windows_explicit_script_path_resolves_to_sibling() {
        let dir = tempfile::tempdir().unwrap();
        std::fs::write(dir.path().join("code"), "#!/usr/bin/env sh\n").unwrap();
        let code_cmd = dir.path().join("code.cmd");
        std::fs::write(&code_cmd, "@echo off\r\n").unwrap();
        assert_eq!(
            windows_executable_sibling(&dir.path().join("code")),
            Some(code_cmd)
        );
        // 已带扩展名或已是 PE：不纠正
        assert_eq!(
            windows_executable_sibling(&dir.path().join("code.cmd")),
            None
        );
        std::fs::write(dir.path().join("zed"), b"MZ\x90\x00").unwrap();
        assert_eq!(windows_executable_sibling(&dir.path().join("zed")), None);
    }
}
