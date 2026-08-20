//! 开发模式 - 管理 casp-portal 项目下的卡片前端服务
//!
//! 扫描项目下的子目录（卡片），检测 web/ / mobile/ / component/ 目录。
//! 支持并行启动/停止 pnpm dev 服务。

use serde::{Deserialize, Serialize};
use std::collections::HashMap;
use std::io::{BufRead, BufReader};
use std::path::Path;
use std::process::{Child, Command, Stdio};
use std::sync::Mutex;
use tauri::{AppHandle, Emitter, Manager};
use uuid::Uuid;

// ===== 数据结构 =====

/// 卡片子目录信息
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct DevCardInfo {
    /// 显示名称（优先取 README.md 首行标题，否则同 folder_name）
    pub display_name: String,
    /// 文件夹名称
    pub folder_name: String,
    /// 卡片完整路径
    pub path: String,
    /// 前端子目录列表（可直接执行 pnpm dev 的目录）
    pub sub_dirs: Vec<DevSubDir>,
    /// 卡片类别: template / main / card
    pub category: String,
}

/// 卡片的前端子目录
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct DevSubDir {
    /// 显示标签: Web / Mobile / Component / PC
    pub label: String,
    /// 类型 key: web / mobile / component / pc
    pub key: String,
    /// 完整工作目录路径（pnpm dev 的工作目录）
    pub work_dir: String,
    /// 是否有 package.json（确认是前端项目）
    pub has_package_json: bool,
}

/// 正在运行的开发服务器
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct DevServerInfo {
    /// 服务器唯一 ID
    pub id: String,
    /// 卡片名称
    pub card_name: String,
    /// 子目录类型 (web/mobile/component)
    pub subdir: String,
    /// 工作目录（卡片下的具体子目录完整路径）
    pub work_dir: String,
    /// 命令名称 (dev / install)
    pub command: String,
    /// 启动时间戳
    pub started_at: u64,
    /// 状态: running / stopped / error
    pub status: String,
    /// PID
    pub pid: Option<u32>,
    /// 监听端口（从输出日志解析）
    pub port: Option<u16>,
}

/// 内部管理的运行中进程
pub struct RunningDevProcess {
    #[allow(dead_code)]
    pub id: String,
    pub card_name: String,
    pub subdir: String,
    pub work_dir: String,
    pub command: String,
    pub child: Child,
    pub started_at: u64,
    pub pid: u32,
    /// 从输出日志解析的端口
    pub port: Option<u16>,
}

pub type DevServerState = Mutex<HashMap<String, RunningDevProcess>>;

/// 开发配置（从 master vue.config.js 读取）
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct DevConfig {
    /// Cookie 值，如 "WISCPSID=a2acc58b-..."
    pub cookie: String,
    /// 后端代理地址，如 "https://i.ygu.edu.cn/"
    pub proxy_target: String,
    /// master devServer 端口
    pub port: Option<u16>,
}

/// 开发服务器历史记录条目
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct DevHistoryEntry {
    /// 工作目录（唯一标识）
    pub work_dir: String,
    /// 卡片名称
    pub card_name: String,
    /// 子目录 key (web/mobile/component)
    pub subdir_key: String,
    /// 启动命令
    pub command: String,
    /// 所属项目路径
    pub project_path: String,
    /// 最后启动时间戳
    pub last_started_at: u64,
}

// ===== vue.config.js 解析辅助 =====

/// 从 vue.config.js 内容中提取 Cookie 值（）
/// 匹配: setHeader("Cookie", "VALUE")
fn extract_cookie(content: &str) -> String {
    let key = "\"Cookie\"";
    let pos = match content.find(key) {
        Some(p) => p,
        None => return String::new(),
    };
    let rest = &content[pos + key.len()..];
    // 找第一个 "（Cookie 值的起始引号）
    let q1 = match rest.find('"') {
        Some(p) => p,
        None => return String::new(),
    };
    let rest2 = &rest[q1 + 1..];
    // 找第二个 "（Cookie 值的结束引号）
    let q2 = match rest2.find('"') {
        Some(p) => p,
        None => return String::new(),
    };
    rest2[..q2].to_string()
}

/// 从 vue.config.js 中提取后端代理地址（第一个非 localhost 的 target）
fn extract_backend_target(content: &str) -> String {
    for line in content.lines() {
        let trimmed = line.trim_start();
        // 跳过注释行
        if trimmed.starts_with("//") {
            continue;
        }
        if trimmed.contains("target:") && trimmed.contains('"') {
            if let Some(start) = trimmed.find('"') {
                let after = &trimmed[start + 1..];
                if let Some(end) = after.find('"') {
                    let url = &after[..end];
                    // 跳过 localhost / 127.0.0.1（卡片代理）
                    if !url.contains("localhost") && !url.contains("127.0.0.1") {
                        return url.to_string();
                    }
                }
            }
        }
    }
    String::new()
}

/// 从 vue.config.js 中提取 devServer.port
fn extract_dev_server_port(content: &str) -> Option<u16> {
    for line in content.lines() {
        let trimmed = line.trim();
        if trimmed.starts_with("//") {
            continue;
        }
        if trimmed.contains("port:") && !trimmed.contains("false") {
            // 提取 port: 后面的数字
            if let Some(idx) = trimmed.find("port:") {
                let after = &trimmed[idx + 5..];
                let num_str: String = after
                    .trim()
                    .chars()
                    .take_while(|c| c.is_ascii_digit())
                    .collect();
                if let Ok(port) = num_str.parse::<u16>() {
                    return Some(port);
                }
            }
        }
    }
    None
}

/// 从 dev server 输出行中解析监听端口
/// 匹配 localhost:PORT / 127.0.0.1:PORT / 0.0.0.0:PORT 等常见格式
fn extract_port_from_output(line: &str) -> Option<u16> {
    for pattern in &[
        "localhost:",
        "127.0.0.1:",
        "0.0.0.0:",
        "http://[::]:",
        "http://[::1]:",
        "[::1]:",
    ] {
        if let Some(idx) = line.find(pattern) {
            let after = &line[idx + pattern.len()..];
            let num_str: String = after.chars().take_while(|c| c.is_ascii_digit()).collect();
            if let Ok(port) = num_str.parse::<u16>() {
                if port > 0 {
                    return Some(port);
                }
            }
        }
    }
    None
}

/// 替换 vue.config.js 中的 Cookie 值
fn replace_cookie_value(content: &str, new_cookie: &str) -> Option<String> {
    let key = "\"Cookie\"";
    let pos = content.find(key)?;
    let rest = &content[pos + key.len()..];
    let q1 = rest.find('"')?;
    let val_start = pos + key.len() + q1 + 1;
    let rest2 = &content[val_start..];
    let q2 = rest2.find('"')?;

    let mut result = String::with_capacity(content.len() + new_cookie.len());
    result.push_str(&content[..val_start]);
    result.push_str(new_cookie);
    result.push_str(&content[val_start + q2..]);
    Some(result)
}

/// 构建 PATH 环境变量，补充打包后可能缺失的路径（nvm/fnm/volta/homebrew 等）
fn build_full_path() -> String {
    let existing = std::env::var("PATH").unwrap_or_default();
    let home = std::env::var("HOME").unwrap_or_default();

    let mut extra: Vec<String> = vec![
        "/usr/local/bin".into(),
        "/opt/homebrew/bin".into(),
        "/opt/homebrew/sbin".into(),
        "/usr/bin".into(),
        "/bin".into(),
    ];

    if !home.is_empty() {
        // npm global
        extra.push(format!("{}/.npm-global/bin", home));

        // ===== fnm (Fast Node Manager) =====
        // 现代版 fnm 默认目录: ~/.local/share/fnm/
        // default alias: ~/.local/share/fnm/aliases/default → node-versions/vXX.X.X/installation
        // 传统路径: ~/Library/fnm/
        extra.push(format!("{}/.local/share/fnm/aliases/default/bin", home));
        extra.push(format!("{}/Library/fnm/aliases/default/bin", home));
        // glob 所有版本目录（最新开发的 fallback）
        for base in &[
            format!("{}/.local/share/fnm/node-versions", home),
            format!("{}/Library/fnm/node-versions", home),
        ] {
            if let Ok(entries) = std::fs::read_dir(base) {
                for entry in entries.flatten() {
                    let bin = entry.path().join("installation/bin");
                    if bin.is_dir() {
                        extra.push(bin.to_string_lossy().to_string());
                    }
                }
            }
        }

        // ===== nvm =====
        // nvm 没有 current 软链，需 glob 版本目录
        let nvm_dir_base = format!("{}/.nvm/versions/node", home);
        if let Ok(entries) = std::fs::read_dir(&nvm_dir_base) {
            for entry in entries.flatten() {
                let bin = entry.path().join("bin");
                if bin.is_dir() {
                    extra.push(bin.to_string_lossy().to_string());
                }
            }
        }
        // nvm 也可能用 NVM_HOME 环境变量（$HOME/.nvm）
        // 允许用户自定义 NVM_DIR
        if let Ok(nvm_dir) = std::env::var("NVM_DIR") {
            let nvm_versions = format!("{}/versions/node", nvm_dir);
            if let Ok(entries) = std::fs::read_dir(&nvm_versions) {
                for entry in entries.flatten() {
                    let bin = entry.path().join("bin");
                    if bin.is_dir() {
                        extra.push(bin.to_string_lossy().to_string());
                    }
                }
            }
        }

        // ===== volta =====
        extra.push(format!("{}/.volta/bin", home));

        // ===== pnpm global =====
        extra.push(format!("{}/Library/pnpm", home));
    }

    // 现有 PATH 放最后（优先使用上面稳定的路径）
    extra.push(existing);
    extra.join(":")
}

/// 终止进程树（包括 pnpm/node 等子孙进程）
///
/// 启动时通过 process_group(0) 让每个服务自成进程组，
/// 停止时对整个进程组发送 SIGTERM（优雅退出），等待短暂时间后
/// 再发送 SIGKILL（强制），确保端口被释放。
///
/// 注意：不能以“直接子进程是否退出”作为是否强杀的判断依据，
/// 因为 sh 退出后 node 等子孙进程可能仍存活并占用端口。
#[cfg(unix)]
pub fn terminate_process_tree(child: &mut Child) {
    let pid = child.id() as i32;
    if pid <= 1 {
        let _ = child.kill();
        return;
    }
    unsafe {
        libc::kill(-pid, libc::SIGTERM);
    }
    // 短暂等待，让进程组优雅退出
    std::thread::sleep(std::time::Duration::from_millis(400));
    // 强制结束整个进程组（若组已不存在，kill 返回 ESRCH，无副作用）
    unsafe {
        libc::kill(-pid, libc::SIGKILL);
    }
    let _ = child.wait();
}

#[cfg(not(unix))]
pub fn terminate_process_tree(child: &mut Child) {
    let _ = child.kill();
}

/// 按端口强制结束占用进程（兜底措施）
///
/// 当进程组终止后端口仍被占用时（如脱离进程组的遗留 node 进程），
/// 通过 lsof 找到监听该端口的进程并强杀，确保端口释放。
#[cfg(any(target_os = "macos", target_os = "linux"))]
pub fn kill_processes_on_port(port: u16) {
    let port_spec = format!("TCP:{}", port);
    let out = std::process::Command::new("lsof")
        .args(["-t", "-i", &port_spec, "-sTCP:LISTEN"])
        .output();
    if let Ok(out) = out {
        let text = String::from_utf8_lossy(&out.stdout);
        for line in text.lines() {
            if let Ok(pid) = line.trim().parse::<i32>() {
                if pid > 1 {
                    unsafe {
                        libc::kill(pid, libc::SIGKILL);
                    }
                }
            }
        }
    }
}

#[cfg(not(any(target_os = "macos", target_os = "linux")))]
pub fn kill_processes_on_port(_port: u16) {}

/// 让子进程独立成进程组（Unix），便于后续整组终止
#[cfg(unix)]
fn setup_process_group(cmd: &mut Command) {
    use std::os::unix::process::CommandExt;
    cmd.process_group(0);
}

#[cfg(not(unix))]
fn setup_process_group(_cmd: &mut Command) {}

// ===== 命令 =====

/// 扫描项目目录下的所有卡片子目录
///
/// 目录结构:
/// 1. casp-portal-webapps: 主项目，只保留 master 子目录
/// 2. cus-card-* / cus-comp-card-*: web/mobile/component-web 在 src/main/ 下
/// 3. cus-template-*: 模板卡片
/// 4. casp-common-components / casp-portal-core: 跳过
#[tauri::command]
pub fn scan_dev_dirs(project_path: String) -> Result<Vec<DevCardInfo>, String> {
    let dir = Path::new(&project_path);
    if !dir.is_dir() {
        return Err("项目目录不存在".to_string());
    }

    let mut main_cards: Vec<DevCardInfo> = Vec::new();
    let mut template_cards: Vec<DevCardInfo> = Vec::new();
    let mut regular_cards: Vec<DevCardInfo> = Vec::new();

    let entries = std::fs::read_dir(dir).map_err(|e| format!("读取目录失败: {}", e))?;

    for entry in entries.flatten() {
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

        // 跳过非前端目录
        if folder_name == "target"
            || folder_name == "casp-common-components"
            || folder_name == "casp-portal-core"
            || is_date_folder(&folder_name)
        {
            continue;
        }

        // casp-portal-webapps: 主项目，只保留 master
        if folder_name == "casp-portal-webapps" {
            let master_dir = entry_path.join("src/main/master");
            if master_dir.is_dir() && master_dir.join("package.json").exists() {
                main_cards.push(DevCardInfo {
                    display_name: "Portal 主站点".to_string(),
                    folder_name: "casp-portal-webapps/master".to_string(),
                    path: master_dir.to_string_lossy().to_string(),
                    sub_dirs: vec![DevSubDir {
                        label: "Master".to_string(),
                        key: "master".to_string(),
                        work_dir: master_dir.to_string_lossy().to_string(),
                        has_package_json: true,
                    }],
                    category: "main".to_string(),
                });
            }
            continue;
        }

        // 普通卡片目录: 检测 src/main/ 下的 web / mobile / component-web
        let sub_dirs = scan_card_subdirs(&entry_path, &folder_name);
        if !sub_dirs.is_empty() {
            let display_name = read_readme_title(&entry_path, &folder_name);
            let category = if folder_name.contains("template") {
                "template"
            } else {
                "card"
            };
            let card = DevCardInfo {
                display_name,
                folder_name,
                path: entry_path.to_string_lossy().to_string(),
                sub_dirs,
                category: category.to_string(),
            };
            if category == "template" {
                template_cards.push(card);
            } else {
                regular_cards.push(card);
            }
        }
    }

    // 排序: 主项目 → 模板 → 常规卡片
    template_cards.sort_by(|a, b| a.display_name.cmp(&b.display_name));
    regular_cards.sort_by(|a, b| a.display_name.cmp(&b.display_name));

    let mut cards = main_cards;
    cards.append(&mut template_cards);
    cards.append(&mut regular_cards);

    Ok(cards)
}

/// 检测是否为日期目录（如 20260717）
fn is_date_folder(name: &str) -> bool {
    name.len() == 8 && name.chars().all(|c| c.is_ascii_digit())
}

/// 读取目录下 README.md 的首行标题作为显示名
/// 如果没有 README 或首行为空，返回 fallback
fn read_readme_title(dir: &Path, fallback: &str) -> String {
    let readme = dir.join("README.md");
    if let Ok(content) = std::fs::read_to_string(&readme) {
        if let Some(first_line) = content.lines().next() {
            let title = first_line.trim().trim_start_matches('#').trim();
            if !title.is_empty() {
                return title.to_string();
            }
        }
    }
    fallback.to_string()
}

/// 扫描卡片目录下的前端子目录
/// 检测 src/main/ 下的 web / mobile / component-web
/// 对于 webapps 下的模板目录，检测根级 web / mobile
fn scan_card_subdirs(card_path: &Path, _folder_name: &str) -> Vec<DevSubDir> {
    let mut sub_dirs: Vec<DevSubDir> = Vec::new();

    // 标准卡片: src/main/ 下的 web / mobile / component-web
    let src_main = card_path.join("src/main");
    if src_main.is_dir() {
        check_subdir(&mut sub_dirs, &src_main.join("web"), "Web", "web");
        check_subdir(&mut sub_dirs, &src_main.join("mobile"), "Mobile", "mobile");
        check_subdir(
            &mut sub_dirs,
            &src_main.join("component-web"),
            "Component",
            "component-web",
        );
    }

    // 如果 src/main 下没找到，检测根级 web / mobile（模板项目可能如此）
    if sub_dirs.is_empty() {
        check_subdir(&mut sub_dirs, &card_path.join("web"), "Web", "web");
        check_subdir(&mut sub_dirs, &card_path.join("mobile"), "Mobile", "mobile");
    }

    sub_dirs
}

/// 辅助: 检查单个子目录是否存在，存在则加入列表
fn check_subdir(sub_dirs: &mut Vec<DevSubDir>, dir: &Path, label: &str, key: &str) {
    if dir.is_dir() {
        let has_pkg = dir.join("package.json").exists();
        if has_pkg {
            sub_dirs.push(DevSubDir {
                label: label.to_string(),
                key: key.to_string(),
                work_dir: dir.to_string_lossy().to_string(),
                has_package_json: true,
            });
        }
    }
}

/// 在指定工作目录下执行命令（pnpm install / pnpm dev）
/// 返回实时输出
#[tauri::command]
pub async fn run_card_command(
    app: AppHandle,
    work_dir: String,
    subdir_key: String,
    command_name: String,
) -> Result<String, String> {
    let work_dir_path = Path::new(&work_dir);
    if !work_dir_path.is_dir() {
        return Err(format!("目录不存在: {}", work_dir));
    }

    let run_cmd = if command_name == "install" {
        "pnpm install".to_string()
    } else if command_name == "dev" {
        "pnpm run dev".to_string()
    } else if command_name == "dev:shell" {
        "pnpm run dev:shell".to_string()
    } else {
        return Err(format!("不支持的命令: {}", command_name));
    };

    // 构建完整 shell 命令
    let shell = if cfg!(target_os = "windows") {
        "cmd"
    } else {
        "sh"
    };
    let shell_flag = if cfg!(target_os = "windows") {
        "/C"
    } else {
        "-c"
    };

    let mut cmd = Command::new(shell);
    cmd.arg(shell_flag)
        .arg(&run_cmd)
        .current_dir(&work_dir)
        .env("PATH", build_full_path())
        .stdout(Stdio::piped())
        .stderr(Stdio::piped());
    setup_process_group(&mut cmd);
    let mut child = cmd.spawn().map_err(|e| format!("启动命令失败: {}", e))?;

    let stdout = child.stdout.take().ok_or("无法获取 stdout")?;
    let stderr = child.stderr.take().ok_or("无法获取 stderr")?;

    // 提前 clone，避免 move 后无法使用
    let wd_for_stdout = work_dir.clone();
    let key_for_stdout = subdir_key.clone();
    let cmd_for_stdout = command_name.clone();
    let wd_for_stderr = work_dir.clone();
    let key_for_stderr = subdir_key.clone();
    let cmd_for_stderr = command_name.clone();

    // 读取 stdout
    let app_for_stdout = app.clone();
    let stdout_handle = tokio::task::spawn_blocking(move || {
        let reader = BufReader::new(stdout);
        for line in reader.lines() {
            if let Ok(line) = line {
                let _ = app_for_stdout.emit(
                    "dev-cmd-line",
                    serde_json::json!({
                        "work_dir": wd_for_stdout,
                        "subdir_key": key_for_stdout,
                        "command": cmd_for_stdout,
                        "stream": "stdout",
                        "line": line,
                    }),
                );
            }
        }
    });

    // 读取 stderr
    let app_for_stderr = app.clone();
    let stderr_handle = tokio::task::spawn_blocking(move || {
        let reader = BufReader::new(stderr);
        for line in reader.lines() {
            if let Ok(line) = line {
                let _ = app_for_stderr.emit(
                    "dev-cmd-line",
                    serde_json::json!({
                        "work_dir": wd_for_stderr,
                        "subdir_key": key_for_stderr,
                        "command": cmd_for_stderr,
                        "stream": "stderr",
                        "line": line,
                    }),
                );
            }
        }
    });

    // 等待子进程（在 blocking 线程等待，不阻塞异步运行时）
    let status = tokio::task::spawn_blocking(move || child.wait())
        .await
        .map_err(|e| format!("等待命令任务失败: {}", e))?
        .map_err(|e| format!("等待命令完成失败: {}", e))?;

    // 等待输出读取完成
    let _ = stdout_handle.await;
    let _ = stderr_handle.await;

    let exit_code = status.code().unwrap_or(-1);
    let success = status.success();

    let _ = app.emit(
        "dev-cmd-done",
        serde_json::json!({
            "work_dir": work_dir,
            "subdir_key": subdir_key,
            "command": command_name,
            "exit_code": exit_code,
            "success": success,
        }),
    );

    if success {
        Ok(format!("命令执行完成，退出码: {}", exit_code))
    } else {
        Err(format!("命令执行失败，退出码: {}", exit_code))
    }
}

/// 启动开发服务器（后台运行）
/// command: 完整启动命令，如 "pnpm run dev" 或 "pnpm run dev:shell"
/// project_path: 所属项目路径，用于历史记录
/// 返回服务器 ID
#[tauri::command]
pub fn start_dev_server(
    app: AppHandle,
    work_dir: String,
    card_name: String,
    subdir_key: String,
    command: Option<String>,
    project_path: Option<String>,
) -> Result<String, String> {
    let work_dir_path = Path::new(&work_dir);
    if !work_dir_path.is_dir() {
        return Err(format!("目录不存在: {}", work_dir));
    }

    let run_command = command.unwrap_or_else(|| "pnpm dev".to_string());

    let shell = if cfg!(target_os = "windows") {
        "cmd"
    } else {
        "sh"
    };
    let shell_flag = if cfg!(target_os = "windows") {
        "/C"
    } else {
        "-c"
    };

    let mut cmd = Command::new(shell);
    cmd.arg(shell_flag)
        .arg(&run_command)
        .current_dir(&work_dir)
        .env("PATH", build_full_path())
        .stdout(Stdio::piped())
        .stderr(Stdio::piped());
    setup_process_group(&mut cmd);
    let mut child = cmd.spawn().map_err(|e| format!("启动服务器失败: {}", e))?;

    let pid = child.id();
    let server_id = Uuid::new_v4().to_string();
    let started_at = std::time::SystemTime::now()
        .duration_since(std::time::UNIX_EPOCH)
        .map(|d| d.as_secs())
        .unwrap_or(0);

    // 记录历史
    if let Some(ref proj_path) = project_path {
        let mut history = read_dev_history(&app);
        history.retain(|h| h.work_dir != work_dir);
        history.push(DevHistoryEntry {
            work_dir: work_dir.clone(),
            card_name: card_name.clone(),
            subdir_key: subdir_key.clone(),
            command: run_command.clone(),
            project_path: proj_path.clone(),
            last_started_at: started_at,
        });
        history.sort_by(|a, b| b.last_started_at.cmp(&a.last_started_at));
        history.truncate(50);
        let _ = write_dev_history(&app, &history);
    }

    let stdout = child.stdout.take().ok_or("无法获取 stdout")?;
    let stderr = child.stderr.take().ok_or("无法获取 stderr")?;

    // 先注册到全局状态（把 child 移入状态管理）
    let state = app.state::<DevServerState>();
    let mut servers = state.lock().unwrap();
    servers.insert(
        server_id.clone(),
        RunningDevProcess {
            id: server_id.clone(),
            card_name: card_name.clone(),
            subdir: subdir_key.clone(),
            work_dir: work_dir.clone(),
            command: run_command.clone(),
            child,
            started_at,
            pid,
            port: None,
        },
    );
    drop(servers); // 释放锁

    // 用 std::thread 读取输出（不依赖 Tokio 运行时）
    let app_bg = app.clone();
    let sid_bg = server_id.clone();

    // stdout 读取线程（stdout EOF 代表进程退出）
    std::thread::spawn(move || {
        let reader = BufReader::new(stdout);
        for line in reader.lines() {
            if let Ok(line) = line {
                // 检测端口
                if let Some(port) = extract_port_from_output(&line) {
                    let state = app_bg.state::<DevServerState>();
                    let mut servers = state.lock().unwrap();
                    if let Some(proc) = servers.get_mut(&sid_bg) {
                        proc.port = Some(port);
                    }
                    drop(servers);
                    let _ = app_bg.emit(
                        "dev-server-port",
                        serde_json::json!({
                            "server_id": sid_bg,
                            "port": port,
                        }),
                    );
                }
                let _ = app_bg.emit(
                    "dev-server-log",
                    serde_json::json!({
                        "server_id": sid_bg,
                        "card_name": card_name,
                        "subdir_key": subdir_key,
                        "work_dir": work_dir,
                        "stream": "stdout",
                        "line": line,
                    }),
                );
            }
        }
        // stdout EOF → 启动等待线程获取真实退出码
        let app_wait = app_bg.clone();
        let sid_wait = sid_bg.clone();
        let state = app_wait.state::<DevServerState>();
        let mut servers = state.lock().unwrap();
        if let Some(mut proc) = servers.remove(&sid_wait) {
            let status = proc.child.wait().ok();
            drop(servers);
            let (exit_code, success) = match status {
                Some(s) => (s.code().unwrap_or(-1), s.success()),
                None => (-1, false),
            };
            let _ = app_wait.emit(
                "dev-server-stopped",
                serde_json::json!({
                    "server_id": sid_wait,
                    "exit_code": exit_code,
                    "success": success,
                }),
            );
        }
    });

    // stderr 读取线程
    let app_err = app.clone();
    let sid_err = server_id.clone();
    std::thread::spawn(move || {
        let reader = BufReader::new(stderr);
        for line in reader.lines() {
            if let Ok(line) = line {
                // stderr 也可能输出了端口信息
                if let Some(port) = extract_port_from_output(&line) {
                    let state = app_err.state::<DevServerState>();
                    let mut servers = state.lock().unwrap();
                    if let Some(proc) = servers.get_mut(&sid_err) {
                        proc.port = Some(port);
                    }
                    drop(servers);
                    let _ = app_err.emit(
                        "dev-server-port",
                        serde_json::json!({
                            "server_id": sid_err,
                            "port": port,
                        }),
                    );
                }
                let _ = app_err.emit(
                    "dev-server-log",
                    serde_json::json!({
                        "server_id": sid_err,
                        "stream": "stderr",
                        "line": line,
                    }),
                );
            }
        }
    });

    Ok(server_id)
}

/// 停止开发服务器
#[tauri::command]
pub fn stop_dev_server(app: AppHandle, server_id: String) -> Result<bool, String> {
    let state = app.state::<DevServerState>();
    let mut servers = state.lock().unwrap();

    if let Some(mut server) = servers.remove(&server_id) {
        let port = server.port;
        match server.child.try_wait() {
            Ok(Some(_)) => {
                // 已终止，但可能遗留占用端口的 node 进程，兜底清理
                if let Some(p) = port {
                    kill_processes_on_port(p);
                }
                Ok(true)
            }
            Ok(None) => {
                // 仍在运行，终止整个进程树（含 pnpm/node 子孙进程）
                terminate_process_tree(&mut server.child);
                // 兜底：若端口仍被占用，按端口强杀，确保释放
                if let Some(p) = port {
                    kill_processes_on_port(p);
                }
                let _ = app.emit(
                    "dev-server-stopped",
                    serde_json::json!({
                        "server_id": server_id,
                        "exit_code": -1,
                        "success": true,
                    }),
                );
                Ok(true)
            }
            Err(_) => Ok(false),
        }
    } else {
        Err("服务器未找到".to_string())
    }
}

/// 列出正在运行的开发服务器
#[tauri::command]
pub fn list_dev_servers(app: AppHandle) -> Result<Vec<DevServerInfo>, String> {
    let state = app.state::<DevServerState>();
    let mut servers = state.lock().unwrap();

    let mut result = Vec::new();
    let mut dead_ids = Vec::new();

    for (id, server) in servers.iter_mut() {
        let status = match server.child.try_wait() {
            Ok(Some(_)) => "stopped",
            Ok(None) => "running",
            Err(_) => "error",
        };

        if status != "running" {
            dead_ids.push(id.clone());
        }

        result.push(DevServerInfo {
            id: id.clone(),
            card_name: server.card_name.clone(),
            subdir: server.subdir.clone(),
            work_dir: server.work_dir.clone(),
            command: server.command.clone(),
            started_at: server.started_at,
            status: status.to_string(),
            pid: Some(server.pid),
            port: server.port,
        });
    }

    // 清理已停止的服务
    for id in dead_ids {
        servers.remove(&id);
    }

    Ok(result)
}

/// 一键停止所有运行中的开发服务器
/// 返回停止的服务器数量
#[tauri::command]
pub fn stop_all_dev_servers(app: AppHandle) -> Result<usize, String> {
    let state = app.state::<DevServerState>();
    let mut servers = state.lock().unwrap();
    let count = servers.len();
    for (_, mut server) in servers.drain() {
        let port = server.port;
        match server.child.try_wait() {
            Ok(None) => {
                terminate_process_tree(&mut server.child);
            }
            _ => {}
        }
        // 兜底：按端口清理遗留进程，确保端口释放
        if let Some(p) = port {
            kill_processes_on_port(p);
        }
    }
    drop(servers);
    // 发送单一事件，前端统一清理（不逐个发 dev-server-stopped 避免误标记为失败）
    let _ = app.emit("dev-all-stopped", serde_json::json!({ "count": count }));
    Ok(count)
}

// ===== 开发服务历史记录 =====

/// 获取历史记录文件路径
fn get_dev_history_path(app: &AppHandle) -> Result<std::path::PathBuf, String> {
    let config_dir = app
        .path()
        .app_data_dir()
        .map_err(|e| format!("获取应用数据目录失败: {}", e))?;
    std::fs::create_dir_all(&config_dir).map_err(|e| format!("创建配置目录失败: {}", e))?;
    Ok(config_dir.join("dev_history.json"))
}

/// 读取历史记录
fn read_dev_history(app: &AppHandle) -> Vec<DevHistoryEntry> {
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
fn write_dev_history(app: &AppHandle, history: &[DevHistoryEntry]) -> Result<(), String> {
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
