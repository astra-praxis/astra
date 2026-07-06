//! Shell 命令执行模块
//!
//! 提供安全的命令执行能力，支持流式输出和超时控制。

use serde::{Deserialize, Serialize};
use std::io::{BufRead, BufReader};
use std::process::{Command, Stdio};
use std::time::Duration;
use tauri::{AppHandle, Emitter};

/// 执行结果（最终状态）
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct CommandResult {
    pub exit_code: i32,
    pub stdout: String,
    pub stderr: String,
    pub success: bool,
}

/// 执行 shell 命令，流式输出到前端
///
/// # 事件
/// - `command-stdout`: 标准输出行 (String)
/// - `command-stderr`: 标准错误行 (String)
/// - `command-done`: 执行完成，payload = CommandResult
/// - `command-error`: 执行失败，payload = 错误消息 (String)
///
/// # 参数
/// - `command`: 要执行的 shell 命令
/// - `cwd`: 工作目录（可选）
/// - `timeout_secs`: 超时秒数（默认 300 秒）
#[tauri::command]
pub async fn run_command(
    app: AppHandle,
    command: String,
    cwd: Option<String>,
    timeout_secs: Option<u64>,
) -> Result<String, String> {
    // 补充 PATH 环境变量（打包后可能不完整）
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

    // 使用 sh -c 执行命令（macOS/Linux）
    let mut cmd = Command::new("sh");
    cmd.arg("-c")
        .arg(&command)
        .env("PATH", &full_path)
        .stdout(Stdio::piped())
        .stderr(Stdio::piped());

    if let Some(ref dir) = cwd {
        cmd.current_dir(dir);
    }

    let mut child = match cmd.spawn() {
        Ok(c) => c,
        Err(e) => {
            let _ = app.emit("command-error", format!("启动命令失败: {}", e));
            return Err(format!("启动命令失败: {}", e));
        }
    };

    let pid = child.id();
    let stdout = child.stdout.take();
    let stderr = child.stderr.take();

    // 读取 stdout（异步）
    let app_stdout = app.clone();
    let stdout_handle = tokio::spawn(async move {
        if let Some(mut stdout) = stdout {
            let reader = BufReader::new(&mut stdout);
            for line in reader.lines() {
                if let Ok(line) = line {
                    let _ = app_stdout.emit("command-stdout", line);
                }
            }
        }
    });

    // 读取 stderr（异步）
    let app_stderr = app.clone();
    let stderr_handle = tokio::spawn(async move {
        if let Some(mut stderr) = stderr {
            let reader = BufReader::new(&mut stderr);
            for line in reader.lines() {
                if let Ok(line) = line {
                    let _ = app_stderr.emit("command-stderr", line);
                }
            }
        }
    });

    // 等待命令完成（带超时）
    let timeout = Duration::from_secs(timeout_secs.unwrap_or(300)); // 默认 5 分钟
    let start = std::time::Instant::now();

    loop {
        match child.try_wait() {
            Ok(Some(status)) => {
                // 命令完成
                let _ = stdout_handle.await;
                let _ = stderr_handle.await;

                let exit_code = status.code().unwrap_or(-1);
                let success = status.success();

                let result = CommandResult {
                    exit_code,
                    stdout: String::new(), // stdout 已流式输出
                    stderr: String::new(),
                    success,
                };

                let _ = app.emit("command-done", &result);
                return Ok(format!("pid:{}", pid));
            }
            Ok(None) => {
                // 还在运行，检查超时
                if start.elapsed() > timeout {
                    let _ = child.kill();
                    let _ = app.emit(
                        "command-error",
                        format!("命令执行超时（{}秒）", timeout.as_secs()),
                    );
                    return Err(format!("命令执行超时（{}秒）", timeout.as_secs()));
                }
                tokio::time::sleep(Duration::from_millis(100)).await;
            }
            Err(e) => {
                let _ = app.emit("command-error", format!("等待命令失败: {}", e));
                return Err(format!("等待命令失败: {}", e));
            }
        }
    }
}
