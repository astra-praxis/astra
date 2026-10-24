use std::io::{BufRead, BufReader};
use std::path::Path;
use std::process::{Command, Stdio};
use tauri::{AppHandle, Emitter, Manager};
use uuid::Uuid;

use super::history::*;
use super::parse::*;
use super::process::*;
use super::types::*;
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
        app_log!(
            error,
            "dev_mode",
            "start_dev_server: dir not found: {}",
            work_dir
        );
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
    let mut child = match cmd.spawn() {
        Ok(c) => c,
        Err(e) => {
            app_log!(
                error,
                "dev_mode",
                "start_dev_server spawn failed: {} ({})",
                run_command,
                e
            );
            return Err(format!("启动服务器失败: {}", e));
        }
    };

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
    app_log!(
        info,
        "dev_mode",
        "start_dev_server ok: id={} pid={} {} (cwd: {})",
        server_id,
        pid,
        run_command,
        work_dir
    );

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
                // 已终止，但可能遗留占用端口的 node 进程，兑底清理
                if let Some(p) = port {
                    kill_processes_on_port(p);
                }
                app_log!(
                    info,
                    "dev_mode",
                    "stop_dev_server ok (already exited): id={}",
                    server_id
                );
                Ok(true)
            }
            Ok(None) => {
                // 仍在运行，终止整个进程树（含 pnpm/node 子孙进程）
                terminate_process_tree(&mut server.child);
                // 兑底：若端口仍被占用，按端口强杀，确保释放
                if let Some(p) = port {
                    kill_processes_on_port(p);
                }
                app_log!(
                    info,
                    "dev_mode",
                    "stop_dev_server ok (terminated): id={} port={:?}",
                    server_id,
                    port
                );
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
            Err(e) => {
                app_log!(
                    error,
                    "dev_mode",
                    "stop_dev_server wait failed: id={} ({})",
                    server_id,
                    e
                );
                Ok(false)
            }
        }
    } else {
        app_log!(
            error,
            "dev_mode",
            "stop_dev_server: not found: id={}",
            server_id
        );
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
