//! Serve 控制台（astra-serve 宿主的桌面端入口）。
//!
//! 职责边界：桌面端是 Serve 的 Client，不在本进程内托管 Entity。
//! 这里只做三件事：
//! - 探测本机 Serve 是否在运行（TCP 探测）
//! - 拉起独立常驻的 `astra-serve-host` 进程（不随桌面端退出而停止）
//! - 用独立 WebviewWindow 打开 Serve 自带的 Web 管理页（Entity 列表 /
//!   工作区 / 对话），窗口关了 Serve 照常跑
//!
//! Serve 二进制定位顺序：显式传入 → `~/.astra/bin/astra-serve-host` →
//! app_data_dir/bin → PATH（`which`）→ 自动从分发仓库下载到 app_data_dir/bin。
//! 分发仓库：https://github.com/astra-praxis/astra-bot（latest.json + Release
//! 资产，sha256 校验）。token 持久化在 `<data>/serve/token`，首次自动生成
//! （32 hex），之后复用。

use std::net::TcpStream;
use std::path::PathBuf;
use std::process::{Command, Stdio};
use std::time::Duration;

use serde::Serialize;
use tauri::{AppHandle, Manager};

pub(crate) const DEFAULT_PORT: u16 = 8800;

fn port_of(port: Option<u16>) -> u16 {
    port.unwrap_or(DEFAULT_PORT)
}

fn is_listening(port: u16) -> bool {
    TcpStream::connect_timeout(
        &format!("127.0.0.1:{port}").parse().expect("valid addr"),
        Duration::from_millis(400),
    )
    .is_ok()
}

/// serve 数据目录：`<app_data_dir>/serve`（机器级，不随账户隔离——
/// Serve 是本机常驻服务，不属于任何一个账户）。
fn serve_dir(app: &AppHandle) -> Result<PathBuf, String> {
    let dir = app
        .path()
        .app_data_dir()
        .map_err(|e| format!("获取应用数据目录失败: {e}"))?
        .join("serve");
    std::fs::create_dir_all(&dir).map_err(|e| format!("创建 serve 目录失败: {e}"))?;
    Ok(dir)
}

/// token：显式传入 > `<serve_dir>/token` 现值 > 自动生成并落盘。
fn ensure_token(app: &AppHandle, explicit: Option<&str>) -> Result<String, String> {
    let path = serve_dir(app)?.join("token");
    if let Some(t) = explicit {
        let t = t.trim().to_string();
        if t.is_empty() {
            return Err("token 不能为空".into());
        }
        std::fs::write(&path, &t).map_err(|e| format!("写入 token 失败: {e}"))?;
        return Ok(t);
    }
    if let Ok(existing) = std::fs::read_to_string(&path) {
        let t = existing.trim().to_string();
        if !t.is_empty() {
            return Ok(t);
        }
    }
    let token = uuid::Uuid::new_v4().simple().to_string();
    std::fs::write(&path, &token).map_err(|e| format!("写入 token 失败: {e}"))?;
    Ok(token)
}

/// 宿主二进制文件名：Windows 带 `.exe`。
fn host_bin_name() -> &'static str {
    if std::env::consts::OS == "windows" {
        "astra-serve-host.exe"
    } else {
        "astra-serve-host"
    }
}

/// Serve 二进制定位：显式路径 → ~/.astra/bin → app_data_dir/bin → PATH。
/// 全部落空且非显式路径时，自动从分发仓库下载（见 `download_serve_bin`）。
fn resolve_bin(app: &AppHandle, explicit: Option<&str>) -> Result<PathBuf, String> {
    if let Some(p) = explicit {
        let p = PathBuf::from(p.trim());
        if p.is_file() {
            return Ok(p);
        }
        return Err(format!("指定的 Serve 程序不存在: {}", p.display()));
    }
    let exe_name = host_bin_name();
    let mut searched: Vec<String> = Vec::new();
    // `dirs::home_dir()`：HOME on unix，Windows 回落 USERPROFILE（HOME 常缺省）
    if let Some(home) = dirs::home_dir() {
        let candidate = home.join(".astra/bin").join(exe_name);
        searched.push(candidate.display().to_string());
        if candidate.is_file() {
            return Ok(candidate);
        }
    }
    if let Ok(data) = app.path().app_data_dir() {
        let candidate = data.join("bin").join(exe_name);
        searched.push(candidate.display().to_string());
        if candidate.is_file() {
            return Ok(candidate);
        }
    }
    if let Ok(found) = which::which("astra-serve-host") {
        return Ok(found);
    }
    searched.push("PATH 中的 astra-serve-host".to_string());
    let local_err = format!("未找到 astra-serve-host。已尝试: {}。", searched.join("、"));
    match download_serve_bin(app) {
        Ok(path) => {
            app_log!(info, "serve", "Serve 二进制已自动下载: {}", path.display());
            Ok(path)
        }
        Err(dl_err) => Err(format!(
            "{local_err}自动下载也失败：{dl_err}。可在下方填入程序路径后重试。"
        )),
    }
}

// ===== 二进制自动下载（astra-praxis/astra-bot 分发仓库）=====

const SERVE_DIST_LATEST: &str =
    "https://raw.githubusercontent.com/astra-praxis/astra-bot/main/latest.json";

/// 当前运行环境的分发仓库 target triple（与构建侧 rustc host triple 对齐）。
fn current_triple() -> &'static str {
    match (std::env::consts::OS, std::env::consts::ARCH) {
        ("macos", "aarch64") => "aarch64-apple-darwin",
        ("macos", "x86_64") => "x86_64-apple-darwin",
        ("windows", "x86_64") => "x86_64-pc-windows-msvc",
        ("linux", "x86_64") => "x86_64-unknown-linux-gnu",
        ("linux", "aarch64") => "aarch64-unknown-linux-gnu",
        _ => "unknown",
    }
}

fn sha256_hex(bytes: &[u8]) -> String {
    use sha2::{Digest, Sha256};
    let digest = Sha256::digest(bytes);
    let mut out = String::with_capacity(64);
    for b in digest {
        out.push_str(&format!("{b:02x}"));
    }
    out
}

/// 从分发仓库下载 serve 二进制到 app_data_dir/bin/。sha256 校验失败不落盘。
/// 阻塞调用（同步命令上下文，通过 tauri 的 tokio runtime 发请求）。
fn download_serve_bin(app: &AppHandle) -> Result<PathBuf, String> {
    let triple = current_triple();
    if triple == "unknown" {
        return Err(format!(
            "不支持的平台: {}/{}",
            std::env::consts::OS,
            std::env::consts::ARCH
        ));
    }
    let client = reqwest::Client::builder()
        // 公司网络直连 GitHub 常年黑洞：快速失败远好于长时间白屏。
        // reqwest 默认读 HTTP(S)_PROXY 环境变量；系统代理（非 TUN）读不到，
        // 失败信息里提示用户手动填路径。
        .connect_timeout(Duration::from_secs(8))
        .timeout(Duration::from_secs(180))
        .build()
        .map_err(|e| format!("HTTP 客户端创建失败: {e}"))?;

    let manifest = tauri::async_runtime::block_on(async {
        client
            .get(SERVE_DIST_LATEST)
            .send()
            .await
            .and_then(|r| r.error_for_status())
            .map_err(|e| format!("拉取 latest.json 失败（网络受限需代理?）: {e}"))?
            .json::<serde_json::Value>()
            .await
            .map_err(|e| format!("latest.json 解析失败: {e}"))
    })?;
    let entry = manifest
        .pointer(&format!("/files/{triple}"))
        .ok_or_else(|| format!("latest.json 里没有平台 {triple} 的产物"))?;
    let url = entry
        .get("url")
        .and_then(|v| v.as_str())
        .ok_or("latest.json 条目缺 url")?;
    let want = entry
        .get("sha256")
        .and_then(|v| v.as_str())
        .ok_or("latest.json 条目缺 sha256")?;

    let bytes = tauri::async_runtime::block_on(async {
        client
            .get(url)
            .send()
            .await
            .and_then(|r| r.error_for_status())
            .map_err(|e| format!("下载失败: {e}"))?
            .bytes()
            .await
            .map_err(|e| format!("读取响应失败: {e}"))
    })?;
    let got = sha256_hex(&bytes);
    if got != want.to_lowercase() {
        return Err(format!("sha256 校验失败：期望 {want}，实际 {got}"));
    }

    let data_dir = app
        .path()
        .app_data_dir()
        .map_err(|e| format!("获取应用数据目录失败: {e}"))?;
    let bin_dir = data_dir.join("bin");
    std::fs::create_dir_all(&bin_dir).map_err(|e| format!("创建 bin 目录失败: {e}"))?;
    let exe = host_bin_name();
    let path = bin_dir.join(exe);
    std::fs::write(&path, &bytes).map_err(|e| format!("写入二进制失败: {e}"))?;
    #[cfg(unix)]
    {
        use std::os::unix::fs::PermissionsExt;
        let _ = std::fs::set_permissions(&path, std::fs::Permissions::from_mode(0o755));
    }
    Ok(path)
}

#[derive(Debug, Serialize)]
pub(crate) struct ServeStatus {
    pub running: bool,
    pub port: u16,
}

/// 探测本机 Serve 是否在监听。
#[tauri::command]
pub(crate) fn serve_console_status(port: Option<u16>) -> ServeStatus {
    let port = port_of(port);
    ServeStatus {
        running: is_listening(port),
        port,
    }
}

#[derive(Debug, Serialize)]
pub(crate) struct ServeStartInfo {
    /// true = 本次拉起；false = 端口已有服务（视为已运行）
    pub started: bool,
    pub port: u16,
    pub token: String,
    pub bin_path: String,
    pub data_dir: String,
}

/// 把桌面端已配置的模型来源同步成宿主 `settings.json` 的 providers 段。
/// 只覆盖 `providers` 字段，其余（搜索源、profiles 等 Web 页配置）保留。
/// best-effort：providers 解密失败（未解锁等）或写盘失败只记日志，不阻塞启动。
fn sync_host_settings(app: &AppHandle, data_dir: &std::path::Path) {
    let providers = match crate::providers::list_providers(app.clone()) {
        Ok(p) if !p.is_empty() => p,
        Ok(_) => return, // 桌面端还没配模型来源：不动宿主文件
        Err(e) => {
            app_log!(warn, "serve", "同步宿主模型来源失败（跳过）: {e}");
            return;
        }
    };
    let items: Vec<serde_json::Value> = providers
        .iter()
        .map(|p| {
            serde_json::json!({
                "id": p.id,
                "name": p.name,
                "base_url": p.base_url,
                "api_key": p.api_key,
                "active_model": p.active_model,
                "protocol": if p.kind == crate::providers::ProviderKind::Anthropic {
                    "anthropic"
                } else {
                    "openai_compat"
                },
            })
        })
        .collect();
    let path = data_dir.join("settings.json");
    let mut root: serde_json::Value = std::fs::read_to_string(&path)
        .ok()
        .and_then(|c| serde_json::from_str(&c).ok())
        .unwrap_or_else(|| serde_json::json!({}));
    root["providers"] = serde_json::Value::Array(items);
    let content = match serde_json::to_string_pretty(&root) {
        Ok(c) => c,
        Err(e) => {
            app_log!(warn, "serve", "序列化宿主 settings 失败（跳过）: {e}");
            return;
        }
    };
    if let Err(e) = std::fs::write(&path, content) {
        app_log!(warn, "serve", "写宿主 settings.json 失败（跳过）: {e}");
    }
}

/// 拉起（或复用）本机 Serve。进程 detached：桌面端退出不影响它。
#[tauri::command]
pub(crate) fn serve_console_start(
    app: AppHandle,
    bin_path: Option<String>,
    port: Option<u16>,
    token: Option<String>,
) -> Result<ServeStartInfo, String> {
    let port = port_of(port);
    let data_dir = serve_dir(&app)?;
    let token = ensure_token(&app, token.as_deref())?;

    // 桌面端模型来源 → 宿主 settings.json（新建 Entity 的 providers 模板）。
    // 无论是否复用已有进程都同步，保持「Agent 页默认 provider」即建 bot 默认模型。
    sync_host_settings(&app, &data_dir);

    if is_listening(port) {
        // 已有服务在跑：不覆盖、不重复拉起（token 可能属于它，前端会校验）。
        return Ok(ServeStartInfo {
            started: false,
            port,
            token,
            bin_path: String::new(),
            data_dir: data_dir.display().to_string(),
        });
    }

    let bin = resolve_bin(&app, bin_path.as_deref())?;

    // 端口落盘：serve_tool 等无 app 句柄的模块从文件读连接信息
    let _ = std::fs::write(data_dir.join("port"), port.to_string());

    // stdout/stderr 追加进 serve/serve-host.log，方便排查启动失败。
    let log_path = data_dir.join("serve-host.log");
    let log = std::fs::OpenOptions::new()
        .create(true)
        .append(true)
        .open(&log_path)
        .map_err(|e| format!("打开日志文件失败: {e}"))?;
    let log_err = log
        .try_clone()
        .map_err(|e| format!("复制日志句柄失败: {e}"))?;

    let mut cmd = Command::new(&bin);
    cmd.arg("--dir")
        .arg(&data_dir)
        .arg("--token")
        .arg(&token)
        .stdout(Stdio::from(log))
        .stderr(Stdio::from(log_err));
    // 独立进程组：桌面端被强杀/终端退出时不连带杀掉常驻 Serve。
    // Windows 下 CREATE_NO_WINDOW 抑制控制台闪窗 + 脱离作业对象。
    #[cfg(unix)]
    {
        use std::os::unix::process::CommandExt;
        cmd.process_group(0);
    }
    #[cfg(windows)]
    {
        const CREATE_NO_WINDOW: u32 = 0x0800_0000;
        const CREATE_NEW_PROCESS_GROUP: u32 = 0x0000_0200;
        const CREATE_BREAKAWAY_FROM_JOB: u32 = 0x0100_0000;
        use std::os::windows::process::CommandExt;
        cmd.creation_flags(
            CREATE_NO_WINDOW | CREATE_NEW_PROCESS_GROUP | CREATE_BREAKAWAY_FROM_JOB,
        );
    }

    let child = cmd
        .spawn()
        .map_err(|e| format!("启动 {} 失败: {e}", bin.display()))?;
    // 刻意不保留 Child：Serve 是独立常驻服务，随桌面端退出被杀反而违背设计。
    drop(child);

    // 轮询最多 ~5s 等端口就绪。
    for _ in 0..25 {
        std::thread::sleep(Duration::from_millis(200));
        if is_listening(port) {
            return Ok(ServeStartInfo {
                started: true,
                port,
                token,
                bin_path: bin.display().to_string(),
                data_dir: data_dir.display().to_string(),
            });
        }
    }
    Err(format!(
        "Serve 进程已启动但端口 {port} 未就绪（5s），请查看日志: {}",
        log_path.display()
    ))
}

/// 用独立窗口打开 Serve 自带的 Web 管理页。同端口复用已有窗口并聚焦。
#[tauri::command]
pub(crate) fn serve_console_open(app: AppHandle, port: Option<u16>) -> Result<String, String> {
    let port = port_of(port);
    if !is_listening(port) {
        return Err(format!("Serve 未在端口 {port} 运行，请先启动"));
    }
    let label = format!("serve-console-{port}");
    if let Some(existing) = app.get_webview_window(&label) {
        let _ = existing.show();
        let _ = existing.set_focus();
        return Ok(label);
    }
    use tauri::{WebviewUrl, WebviewWindowBuilder};
    let url: tauri::Url = format!("http://127.0.0.1:{port}/")
        .parse()
        .map_err(|e| format!("无效的 Serve 地址: {e}"))?;
    WebviewWindowBuilder::new(&app, label.clone(), WebviewUrl::External(url))
        .title("Astra Serve")
        .inner_size(1180.0, 780.0)
        .build()
        .map_err(|e| format!("打开 Serve 控制台窗口失败: {e}"))?;
    Ok(label)
}
