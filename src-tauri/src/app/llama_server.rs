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
    /// true = 不是 astra 拉起的（进程在应用之外，应用内无法停止）
    #[serde(default)]
    external: bool,
    /// llama.cpp /props 报告的视觉能力（mmproj 已加载时 true）
    #[serde(default)]
    vision: bool,
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
        // 未被跟踪：可能是应用之外启动的 llama-server（用户脚本/终端），无法停止
        if is_port_listening(port) {
            Err("该模型服务由 astra 之外启动，请在外部停止".to_string())
        } else {
            Ok(false)
        }
    }
}

/// 端口上是否有进程在监听（用于区分“外部启动的服务”和“早就停了”）
fn is_port_listening(port: u16) -> bool {
    use std::net::{SocketAddr, TcpStream};
    use std::time::Duration;
    let addr = SocketAddr::from(([127, 0, 0, 1], port));
    TcpStream::connect_timeout(&addr, Duration::from_millis(200)).is_ok()
}

#[tauri::command]
pub(crate) async fn list_running_servers(
    app: tauri::AppHandle,
) -> Result<Vec<RunningModelInfo>, String> {
    // 1) 应用自己拉起的 subprocess（能停能查 pid）——同步取完就释放锁，不能跨 await
    let mut servers = Vec::new();
    let mut tracked_ports: Vec<u16> = Vec::new();
    let mut dead_servers = Vec::new();
    {
        let state = app.state::<ServerState>();
        let mut running_servers = state.lock().unwrap();
        for (i, server) in running_servers.iter_mut().enumerate() {
            let status = match server.child.try_wait() {
                Ok(Some(_)) => "stopped",
                Ok(None) => "running",
                Err(_) => "unknown",
            };
            tracked_ports.push(server.port);
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
                external: false,
                vision: false,
            });
            if status != "running" {
                dead_servers.push(i);
            }
        }
        for index in dead_servers.into_iter().rev() {
            let _ = running_servers.remove(index);
        }
    }

    // 2) 应用之外启动的 llama-server（用户脚本 / 终端）——探本机监听端口
    for info in discover_external_servers(&app, &tracked_ports).await {
        servers.push(info);
    }
    // 同端口只保留一条（外部实例与已跟踪实例重合时以跟踪的为准，它在前面）
    let mut deduped: Vec<RunningModelInfo> = Vec::new();
    for info in servers {
        if deduped.iter().any(|s| s.config.port == info.config.port) {
            continue;
        }
        deduped.push(info);
    }
    crate::app_log!(
        debug,
        "models",
        "list_running_servers: {} entries, tracked={:?}, found={:?}",
        deduped.len(),
        tracked_ports,
        deduped
            .iter()
            .map(|s| {
                format!(
                    "{}@{}{}",
                    s.config.name,
                    s.config.port,
                    if s.external { ":external" } else { "" }
                )
            })
            .collect::<Vec<_>>()
    );
    Ok(deduped)
}

// ===== 外部 llama-server 发现 =====
// astra 只能跟踪自己 spawn 的子进程；用户脚本/终端里起的 llama-server 不在 ServerState
// 里，模型选择器就看不到。这里扫本机监听端口 + llama.cpp 的 /props 探测，把外部实例
// 一并列出来（external = true，应用内不可停止，但可直接选用聊天）。

#[derive(Debug, Deserialize)]
struct LlamaProps {
    #[serde(default)]
    model_alias: Option<String>,
    #[serde(default)]
    model_path: Option<String>,
    #[serde(default)]
    modalities: Option<LlamaModalities>,
}

#[derive(Debug, Deserialize)]
struct LlamaModalities {
    #[serde(default)]
    vision: bool,
}

/// OpenAI 兼容 `/v1/models`（无 /props 的旧版 llama.cpp 的退路）
#[derive(Debug, Deserialize)]
struct OpenAiModels {
    #[serde(default)]
    data: Vec<OpenAiModel>,
}

#[derive(Debug, Deserialize)]
struct OpenAiModel {
    #[serde(default)]
    id: String,
    #[serde(default)]
    details: Option<OpenAiModelDetails>,
}

#[derive(Debug, Deserialize)]
struct OpenAiModelDetails {
    #[serde(default)]
    format: Option<String>,
}

/// 探测结果（统一成 /props 的形状，便于命名与能力判断）
struct ProbedServer {
    alias: Option<String>,
    model_path: Option<String>,
    vision: bool,
}

/// 本机监听端口（用于发现任意端口上的外部实例）。取不到时返回空（退化为候选端口）。
/// 性能：lsof 约几十毫秒，结果缓存 8s（ModelsPage 每 3s 轮询一次）。
fn listening_ports() -> Vec<u16> {
    use std::sync::OnceLock;
    use std::time::Instant;
    static CACHE: OnceLock<Mutex<Option<(Instant, Vec<u16>)>>> = OnceLock::new();
    const TTL: std::time::Duration = std::time::Duration::from_secs(8);

    let cache = CACHE.get_or_init(|| Mutex::new(None));
    if let Ok(guard) = cache.lock() {
        if let Some((at, ports)) = guard.as_ref() {
            if at.elapsed() < TTL {
                return ports.clone();
            }
        }
    }

    #[allow(unused_mut)]
    let mut ports: Vec<u16> = Vec::new();
    #[cfg(any(target_os = "macos", target_os = "linux"))]
    {
        // lsof -nP -iTCP -sTCP:LISTEN -F n → 每行形如 "n127.0.0.1:8080" / "n*:8080"
        if let Ok(output) = std::process::Command::new("lsof")
            .args(["-nP", "-iTCP", "-sTCP:LISTEN", "-F", "n"])
            .output()
        {
            for line in String::from_utf8_lossy(&output.stdout).lines() {
                let Some(rest) = line.strip_prefix('n') else {
                    continue;
                };
                // 只收非特权端口（llama-server 无需 root 也能监听的区间）
                if let Some(port) = rest.rsplit(':').next().and_then(|p| p.parse::<u16>().ok()) {
                    if port >= 1024 {
                        ports.push(port);
                    }
                }
            }
        }
    }
    ports.sort_unstable();
    ports.dedup();

    if let Ok(mut guard) = cache.lock() {
        *guard = Some((Instant::now(), ports.clone()));
    }
    ports
}

/// 候选端口：已跟踪 ∪ 已配置模型 ∪ 常见默认 ∪ 本机监听端口
fn candidate_ports(app: &tauri::AppHandle, tracked: &[u16]) -> Vec<u16> {
    let mut ports: Vec<u16> = tracked.to_vec();
    for model in get_settings(app.clone()).models {
        if model.port >= 1024 {
            ports.push(model.port);
        }
    }
    ports.extend([8080u16, 8081, 8082, 8090, 8000, 1234]);
    ports.extend(listening_ports());
    ports.sort_unstable();
    ports.dedup();
    ports
}

/// 探一个端口：优先 /props（llama.cpp 专有，信息最全），失败则退到 OpenAI
/// `/v1/models` 且仅接受 gguf 格式（避免把 vLLM / LM Studio / 普通网关误判成 llama）。
async fn probe_port(client: &reqwest::Client, port: u16) -> Option<ProbedServer> {
    if let Ok(resp) = client
        .get(format!("http://127.0.0.1:{port}/props"))
        .send()
        .await
    {
        if resp.status().is_success() {
            if let Ok(props) = resp.json::<LlamaProps>().await {
                return Some(ProbedServer {
                    alias: props.model_alias,
                    model_path: props.model_path,
                    vision: props.modalities.map(|m| m.vision).unwrap_or(false),
                });
            }
        }
    }

    let resp = client
        .get(format!("http://127.0.0.1:{port}/v1/models"))
        .send()
        .await
        .ok()?;
    if !resp.status().is_success() {
        return None;
    }
    let models = resp.json::<OpenAiModels>().await.ok()?;
    let model = models.data.into_iter().next()?;
    let is_gguf = model
        .details
        .as_ref()
        .and_then(|d| d.format.as_deref())
        .map(|f| f.eq_ignore_ascii_case("gguf"))
        .unwrap_or(false);
    if !is_gguf {
        return None;
    }
    Some(ProbedServer {
        alias: (!model.id.is_empty()).then_some(model.id),
        model_path: None,
        vision: false,
    })
}

/// 扫描候选端口，返回应用之外启动的 llama-server 列表
async fn discover_external_servers(
    app: &tauri::AppHandle,
    tracked: &[u16],
) -> Vec<RunningModelInfo> {
    let ports = candidate_ports(app, tracked);
    if ports.is_empty() {
        return Vec::new();
    }
    let client = match reqwest::Client::builder()
        .timeout(std::time::Duration::from_millis(400))
        // 本机探测：绝不能被 HTTP_PROXY / ALL_PROXY 环境变量带跑
        .no_proxy()
        .build()
    {
        Ok(client) => client,
        Err(_) => return Vec::new(),
    };

    let client_ref = &client;
    let probes = ports
        .iter()
        .map(|&port| async move { (port, probe_port(client_ref, port).await) });
    let results = futures_util::future::join_all(probes).await;

    ports_dedup(results, tracked)
}

/// 把探测结果收敛成外部服务条目（跳过已跟踪端口）
fn ports_dedup(
    results: Vec<(u16, Option<ProbedServer>)>,
    tracked: &[u16],
) -> Vec<RunningModelInfo> {
    results
        .into_iter()
        .filter_map(|(port, probed)| probed.map(|p| (port, p)))
        .filter(|(port, _)| !tracked.contains(port))
        .map(|(port, probed)| {
            let name = probed
                .alias
                .filter(|a| !a.trim().is_empty())
                .or_else(|| {
                    probed.model_path.as_deref().and_then(|path| {
                        std::path::Path::new(path)
                            .file_stem()
                            .map(|s| s.to_string_lossy().into_owned())
                    })
                })
                .unwrap_or_else(|| format!("llama:{port}"));
            RunningModelInfo {
                config: ModelConfig {
                    id: format!("external-{port}"),
                    name,
                    model_path: probed.model_path.unwrap_or_default(),
                    server_path: String::new(),
                    port,
                    ngl: 0,
                },
                status: "running".to_string(),
                pid: None,
                started_at: unix_timestamp(),
                external: true,
                vision: probed.vision,
            }
        })
        .collect()
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

#[cfg(test)]
mod tests {
    use super::*;

    fn probed(alias: Option<&str>, path: Option<&str>, vision: bool) -> ProbedServer {
        ProbedServer {
            alias: alias.map(str::to_string),
            model_path: path.map(str::to_string),
            vision,
        }
    }

    /// 别名优先：llama.cpp /props 的 model_alias 就是用户在模型选择器里看到的名字
    #[test]
    fn dedup_names_external_server_by_alias() {
        let out = ports_dedup(
            vec![(
                8080,
                Some(probed(Some("astra-local"), Some("/m/a.gguf"), true)),
            )],
            &[],
        );
        assert_eq!(out.len(), 1);
        assert_eq!(out[0].config.name, "astra-local");
        assert_eq!(out[0].config.port, 8080);
        assert_eq!(out[0].config.id, "external-8080");
        assert!(out[0].external);
        assert!(out[0].vision);
        assert!(out[0].pid.is_none());
        assert_eq!(out[0].status, "running");
    }

    /// 没有别名（旧版 llama-server）时退回模型文件名
    #[test]
    fn dedup_falls_back_to_model_file_stem() {
        let out = ports_dedup(
            vec![(
                8081,
                Some(probed(None, Some("/models/qwen-7b.gguf"), false)),
            )],
            &[],
        );
        assert_eq!(out[0].config.name, "qwen-7b");
        assert!(out[0].external);
    }

    /// 两者都没有时用 llama:{port}，避免空白条目
    #[test]
    fn dedup_uses_port_placeholder_as_last_resort() {
        let out = ports_dedup(vec![(8090, Some(probed(Some("  "), None, false)))], &[]);
        assert_eq!(out[0].config.name, "llama:8090");
    }

    /// 应用自己拉起的端口（tracked）不再重复列为外部实例；探测失败的端口直接丢弃
    #[test]
    fn dedup_skips_tracked_and_unresponsive_ports() {
        let out = ports_dedup(
            vec![
                (8080, Some(probed(Some("tracked"), None, false))),
                (8081, None),
                (8082, Some(probed(Some("external"), None, false))),
            ],
            &[8080],
        );
        assert_eq!(out.len(), 1);
        assert_eq!(out[0].config.port, 8082);
        assert_eq!(out[0].config.name, "external");
    }

    /// 未跟踪端口停止前用 TCP 连接判断“是否真的有服务在跑”：连得上才算外部启动
    #[test]
    fn is_port_listening_detects_bound_socket() {
        let listener = std::net::TcpListener::bind(("127.0.0.1", 0)).unwrap();
        let port = listener.local_addr().unwrap().port();
        assert!(is_port_listening(port));
        drop(listener);
    }

    #[test]
    fn listening_ports_is_sorted_deduped_and_non_privileged() {
        let ports = listening_ports();
        assert!(
            ports.windows(2).all(|w| w[0] < w[1]),
            "应升序且无重复: {ports:?}"
        );
        assert!(
            ports.iter().all(|p| *p >= 1024),
            "只收非特权端口: {ports:?}"
        );
    }
}
