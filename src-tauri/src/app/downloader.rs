use serde::{Deserialize, Serialize};
use std::collections::HashMap;
use std::fs;
use std::fs::OpenOptions;
use std::io::Write;
use std::sync::atomic::{AtomicBool, Ordering};
use std::sync::Arc;
use std::time::{Duration, Instant};
use tauri::{AppHandle, Emitter, Manager};
use tokio::fs as async_fs;

// ===== 下载任务状态 =====
#[derive(Debug, Clone, Serialize, Deserialize, PartialEq)]
pub enum DownloadStatus {
    Pending,     // 等待中
    Downloading, // 下载中
    Paused,      // 已暂停
    Completed,   // 已完成
    Failed,      // 失败
    Cancelled,   // 已取消
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct DownloadTask {
    pub id: String,
    pub url: String,
    pub filename: String,
    pub save_path: String,
    pub file_path: String, // 完整文件路径
    pub total_size: u64,
    pub downloaded_size: u64,
    pub status: DownloadStatus,
    pub error: Option<String>,
    pub created_at: u64,
    pub started_at: Option<u64>,
    pub completed_at: Option<u64>,
    pub speed: u64, // bytes per second
}

// ===== 下载管理器状态 =====
pub struct DownloadManager {
    tasks: std::sync::Mutex<HashMap<String, DownloadTask>>,
    cancel_flags: std::sync::Mutex<HashMap<String, Arc<AtomicBool>>>,
    loaded: AtomicBool, // 是否已从磁盘加载下载记录
}

impl DownloadManager {
    pub fn new() -> Self {
        Self {
            tasks: std::sync::Mutex::new(HashMap::new()),
            cancel_flags: std::sync::Mutex::new(HashMap::new()),
            loaded: AtomicBool::new(false),
        }
    }
}

// ===== 持久化（下载记录） =====

/// 下载记录文件路径（应用数据目录下的 download_history.json）
fn get_history_path(app: &AppHandle) -> Result<std::path::PathBuf, String> {
    let app_data_dir = app
        .path()
        .app_data_dir()
        .map_err(|e| format!("获取应用数据目录失败: {}", e))?;
    Ok(app_data_dir.join("download_history.json"))
}

/// 从磁盘加载下载记录
fn load_history(app: &AppHandle) -> Vec<DownloadTask> {
    let path = match get_history_path(app) {
        Ok(p) => p,
        Err(_) => return Vec::new(),
    };
    if !path.exists() {
        return Vec::new();
    }
    let content = fs::read_to_string(&path).unwrap_or_default();
    serde_json::from_str::<Vec<DownloadTask>>(&content).unwrap_or_default()
}

/// 保存下载记录到磁盘
fn save_history(app: &AppHandle, tasks: &HashMap<String, DownloadTask>) {
    let path = match get_history_path(app) {
        Ok(p) => p,
        Err(_) => return,
    };
    if let Some(dir) = path.parent() {
        let _ = fs::create_dir_all(dir);
    }
    let tasks_vec: Vec<DownloadTask> = tasks.values().cloned().collect();
    let _ = fs::write(
        &path,
        serde_json::to_string_pretty(&tasks_vec).unwrap_or_default(),
    );
}

// ===== Tauri 命令 =====

/// 获取下载列表（首次调用时从磁盘加载历史记录）
#[tauri::command]
pub async fn list_downloads(app: AppHandle) -> Result<Vec<DownloadTask>, String> {
    let manager = app.state::<DownloadManager>();

    // 首次调用时从磁盘加载下载记录
    if !manager.loaded.load(Ordering::SeqCst) {
        manager.loaded.store(true, Ordering::SeqCst);
        let mut tasks = manager.tasks.lock().unwrap();
        if tasks.is_empty() {
            let history = load_history(&app);
            // 应用重启后，把下载中/等待中的任务标记为已暂停（可继续下载）
            for mut t in history {
                if t.status == DownloadStatus::Downloading || t.status == DownloadStatus::Pending {
                    t.status = DownloadStatus::Paused;
                }
                tasks.insert(t.id.clone(), t);
            }
        }
    }

    let tasks = manager.tasks.lock().unwrap();
    Ok(tasks.values().cloned().collect())
}

/// 添加下载任务
#[tauri::command]
pub async fn add_download(
    app: AppHandle,
    url: String,
    filename: Option<String>,
    save_path: Option<String>,
) -> Result<DownloadTask, String> {
    // 生成任务 ID
    let id = format!("dl-{}", chrono::Utc::now().timestamp_millis());

    // 获取默认下载目录
    let download_dir = save_path.unwrap_or_else(|| {
        dirs::download_dir()
            .map(|p| p.to_string_lossy().to_string())
            .unwrap_or_else(|| ".".to_string())
    });

    // 获取文件名：优先用户指定，其次尝试从服务器响应头获取真实文件名，最后从 URL 提取
    // 很多下载链接（S3 签名链接 / CDN / REST API）URL 最后一段是 id/token 长串，
    // 真实文件名在响应头的 Content-Disposition 里（浏览器下载正是依赖此机制）
    let filename = match filename {
        Some(f) => f,
        None => {
            let client = reqwest::Client::new();
            let from_server = fetch_filename_from_server(&client, &url).await;
            from_server.unwrap_or_else(|| {
                url.split('/')
                    .last()
                    .map(|s| {
                        // 移除查询参数
                        s.split('?').next().unwrap_or("download").to_string()
                    })
                    .unwrap_or_else(|| "download".to_string())
            })
        }
    };

    // 构建完整文件路径
    let file_path = format!("{}/{}", download_dir, filename);

    // 创建任务
    let task = DownloadTask {
        id: id.clone(),
        url: url.clone(),
        filename: filename.clone(),
        save_path: download_dir,
        file_path: file_path.clone(),
        total_size: 0,
        downloaded_size: 0,
        status: DownloadStatus::Pending,
        error: None,
        created_at: chrono::Utc::now().timestamp() as u64,
        started_at: None,
        completed_at: None,
        speed: 0,
    };

    // 保存任务
    {
        let manager = app.state::<DownloadManager>();
        let mut tasks = manager.tasks.lock().unwrap();
        tasks.insert(id.clone(), task.clone());
        save_history(&app, &tasks);
    }

    // 开始下载
    start_download(app, id)?;

    Ok(task)
}

/// 开始/继续下载
#[tauri::command]
pub fn start_download(app: AppHandle, task_id: String) -> Result<(), String> {
    let task = {
        let manager = app.state::<DownloadManager>();
        let mut tasks = manager.tasks.lock().unwrap();

        let task = tasks.get(&task_id).cloned().ok_or("任务不存在")?;
        if task.status == DownloadStatus::Downloading {
            return Err("任务正在下载中".to_string());
        }

        // 更新状态
        if let Some(t) = tasks.get_mut(&task_id) {
            t.status = DownloadStatus::Downloading;
            t.started_at = Some(chrono::Utc::now().timestamp() as u64);
            t.error = None;
        }

        save_history(&app, &tasks);

        tasks.get(&task_id).cloned().unwrap()
    };

    // 创建取消标志
    let cancel_flag = Arc::new(AtomicBool::new(false));
    {
        let manager = app.state::<DownloadManager>();
        manager
            .cancel_flags
            .lock()
            .unwrap()
            .insert(task_id.clone(), cancel_flag.clone());
    }

    // 在后台线程执行下载
    // 注意：必须用 tauri::async_runtime::spawn 而不是 tokio::spawn。
    // start_download 是同步命令，运行在 WebKit 回调线程（主线程），
    // 该线程没有 tokio runtime 上下文，tokio::spawn 会 panic 导致应用闪退。
    let app_handle = app.clone();
    let cancel = cancel_flag.clone();
    let task_id_clone = task_id.clone();

    tauri::async_runtime::spawn(async move {
        if let Err(e) = download_file(app_handle.clone(), task_id_clone.clone(), task, cancel).await
        {
            eprintln!("下载失败: {}", e);
            // 更新任务状态为失败并发送最终事件（含失败时间）
            let manager = app_handle.state::<DownloadManager>();
            let mut tasks = manager.tasks.lock().unwrap();
            let failed = tasks.get_mut(&task_id_clone).map(|t| {
                t.status = DownloadStatus::Failed;
                t.error = Some(e.clone());
                t.clone()
            });
            save_history(&app_handle, &tasks);
            if let Some(f) = failed {
                let _ = app_handle.emit(
                    "download-final",
                    serde_json::json!({
                        "id": f.id,
                        "status": "Failed",
                        "filename": f.filename,
                        "file_path": f.file_path,
                        "finished_at": chrono::Utc::now().timestamp(),
                        "total": f.downloaded_size,
                        "error": e,
                    }),
                );
            }
        }
    });

    Ok(())
}

/// 暂停下载
#[tauri::command]
pub fn pause_download(app: AppHandle, task_id: String) -> Result<(), String> {
    let manager = app.state::<DownloadManager>();

    // 设置取消标志
    if let Some(flag) = manager.cancel_flags.lock().unwrap().get(&task_id) {
        flag.store(true, Ordering::SeqCst);
    }

    // 更新状态
    let mut tasks = manager.tasks.lock().unwrap();
    if let Some(task) = tasks.get_mut(&task_id) {
        task.status = DownloadStatus::Paused;
    }
    save_history(&app, &tasks);

    Ok(())
}

/// 取消并删除下载任务（可选删除文件）
#[tauri::command]
pub fn delete_download(app: AppHandle, task_id: String, delete_file: bool) -> Result<(), String> {
    let manager = app.state::<DownloadManager>();

    // 取消下载
    if let Some(flag) = manager.cancel_flags.lock().unwrap().get(&task_id) {
        flag.store(true, Ordering::SeqCst);
    }

    // 获取任务信息
    let task = manager.tasks.lock().unwrap().get(&task_id).cloned();

    // 删除任务
    manager.tasks.lock().unwrap().remove(&task_id);
    manager.cancel_flags.lock().unwrap().remove(&task_id);

    // 保存记录
    {
        let tasks = manager.tasks.lock().unwrap();
        save_history(&app, &tasks);
    }

    // 删除文件（后台异步执行；同 start_download，需用 tauri::async_runtime::spawn，
    // 因为 delete_download 是同步命令，运行在线程上没有 tokio runtime 上下文）
    if delete_file {
        if let Some(task) = task {
            tauri::async_runtime::spawn(async move {
                let _ = async_fs::remove_file(&task.file_path).await;
            });
        }
    }

    Ok(())
}

/// 重试下载
#[tauri::command]
pub fn retry_download(app: AppHandle, task_id: String) -> Result<(), String> {
    {
        let manager = app.state::<DownloadManager>();
        let mut tasks = manager.tasks.lock().unwrap();

        if let Some(task) = tasks.get_mut(&task_id) {
            if task.status != DownloadStatus::Failed && task.status != DownloadStatus::Cancelled {
                return Err("只能重试失败或取消的任务".to_string());
            }
            task.status = DownloadStatus::Pending;
            task.error = None;
            task.downloaded_size = 0;
            task.speed = 0;
        }

        save_history(&app, &tasks);
    }

    start_download(app, task_id)
}

/// 打开下载文件夹
#[tauri::command]
pub fn open_download_folder(path: String) -> Result<(), String> {
    #[cfg(target_os = "macos")]
    {
        std::process::Command::new("open")
            .arg(&path)
            .spawn()
            .map_err(|e| format!("打开文件夹失败: {}", e))?;
    }

    #[cfg(target_os = "windows")]
    {
        std::process::Command::new("explorer")
            .arg(&path)
            .spawn()
            .map_err(|e| format!("打开文件夹失败: {}", e))?;
    }

    #[cfg(target_os = "linux")]
    {
        std::process::Command::new("xdg-open")
            .arg(&path)
            .spawn()
            .map_err(|e| format!("打开文件夹失败: {}", e))?;
    }

    Ok(())
}

/// 打开下载文件
#[tauri::command]
pub fn open_download_file(path: String) -> Result<(), String> {
    #[cfg(target_os = "macos")]
    {
        std::process::Command::new("open")
            .arg(&path)
            .spawn()
            .map_err(|e| format!("打开文件失败: {}", e))?;
    }

    #[cfg(target_os = "windows")]
    {
        std::process::Command::new("cmd")
            .args(["/C", "start", &path])
            .spawn()
            .map_err(|e| format!("打开文件失败: {}", e))?;
    }

    #[cfg(target_os = "linux")]
    {
        std::process::Command::new("xdg-open")
            .arg(&path)
            .spawn()
            .map_err(|e| format!("打开文件失败: {}", e))?;
    }

    Ok(())
}

/// 清空下载记录
#[tauri::command]
pub fn clear_download_history(app: AppHandle) -> Result<(), String> {
    let manager = app.state::<DownloadManager>();
    let mut tasks = manager.tasks.lock().unwrap();
    tasks.clear();
    save_history(&app, &tasks);
    Ok(())
}

// ===== 内部下载逻辑 =====

async fn download_file(
    app: AppHandle,
    _task_id: String,
    mut task: DownloadTask,
    cancel: Arc<AtomicBool>,
) -> Result<(), String> {
    let client = reqwest::Client::new();

    // 检查已下载的文件大小（断点续传）
    let mut existing_size = match async_fs::metadata(&task.file_path).await {
        Ok(meta) => meta.len(),
        Err(_) => 0,
    };

    task.downloaded_size = existing_size;

    // 发送带 Range 的请求
    let mut request = client.get(&task.url);
    if existing_size > 0 {
        request = request.header("Range", format!("bytes={}-", existing_size));
    }

    let response = request
        .send()
        .await
        .map_err(|e| format!("请求失败: {}", e))?;

    // 从响应头解析真实文件名（浏览器下载的标准做法）
    // 很多下载链接（REST API / 签名链接 / 带 token 的 CDN）URL 中不含真实文件名，
    // 但服务器会在 Content-Disposition 响应头里给出，例如：
    //   attachment; filename="report.pdf"
    //   attachment; filename*=UTF-8''%E6%96%87%E4%BB%B6.pdf
    let header_filename = response
        .headers()
        .get(reqwest::header::CONTENT_DISPOSITION)
        .and_then(|v| v.to_str().ok())
        .and_then(parse_content_disposition);

    if let Some(real_name) = header_filename {
        if real_name != task.filename {
            let old_path = task.file_path.clone();
            task.filename = real_name;
            task.file_path = format!("{}/{}", task.save_path, task.filename);

            // 旧路径已有部分文件（暂停/续传场景）时，重命名以保留进度
            if old_path != task.file_path && async_fs::metadata(&old_path).await.is_ok() {
                let _ = async_fs::rename(&old_path, &task.file_path).await;
            }

            // 同步更新管理器中的任务信息
            {
                let manager = app.state::<DownloadManager>();
                let mut tasks = manager.tasks.lock().unwrap();
                if let Some(t) = tasks.get_mut(&task.id) {
                    t.filename = task.filename.clone();
                    t.file_path = task.file_path.clone();
                }
            }

            // 重新检查新路径的已下载大小
            existing_size = match async_fs::metadata(&task.file_path).await {
                Ok(meta) => meta.len(),
                Err(_) => 0,
            };
            task.downloaded_size = existing_size;
        }
    }

    // 检查服务器是否支持断点续传
    let supports_range = response.headers().get("accept-ranges").is_some()
        || response.status() == reqwest::StatusCode::PARTIAL_CONTENT;

    // 获取文件总大小
    let total_size = if supports_range && response.status() == reqwest::StatusCode::PARTIAL_CONTENT
    {
        response.content_length().unwrap_or(0) + existing_size
    } else {
        response.content_length().unwrap_or(0)
    };

    // 更新任务信息
    {
        let manager = app.state::<DownloadManager>();
        let mut tasks = manager.tasks.lock().unwrap();
        if let Some(t) = tasks.get_mut(&task.id) {
            t.total_size = total_size;
            t.downloaded_size = existing_size;
        }
    }

    // 如果服务器返回 200 但本地已有文件，服务器不支持断点续传，需要重新下载
    if existing_size > 0 && response.status() == reqwest::StatusCode::OK {
        // 服务器不支持断点续传，重新开始
        let _ = async_fs::remove_file(&task.file_path).await;
        task.downloaded_size = 0;

        let manager = app.state::<DownloadManager>();
        let mut tasks = manager.tasks.lock().unwrap();
        if let Some(t) = tasks.get_mut(&task.id) {
            t.downloaded_size = 0;
        }
    }

    // 创建/打开文件（追加模式）
    let mut file = OpenOptions::new()
        .create(true)
        .write(true)
        .append(existing_size > 0)
        .open(&task.file_path)
        .map_err(|e| format!("创建文件失败: {}", e))?;

    // 下载流
    let mut stream = response.bytes_stream();
    let mut last_update = Instant::now();
    let mut last_bytes = existing_size;
    let mut downloaded = existing_size;

    use futures_util::StreamExt;

    while let Some(chunk) = stream.next().await {
        // 检查是否取消
        if cancel.load(Ordering::SeqCst) {
            // 保存已下载进度
            let manager = app.state::<DownloadManager>();
            let mut tasks = manager.tasks.lock().unwrap();
            if let Some(t) = tasks.get_mut(&task.id) {
                t.downloaded_size = downloaded;
            }
            save_history(&app, &tasks);
            return Ok(());
        }

        let chunk = chunk.map_err(|e| format!("读取数据失败: {}", e))?;

        file.write_all(&chunk)
            .map_err(|e| format!("写入文件失败: {}", e))?;
        downloaded += chunk.len() as u64;

        // 计算速度并发送进度更新（每 200ms）
        let now = Instant::now();
        if now.duration_since(last_update) > Duration::from_millis(200) {
            let elapsed = now.duration_since(last_update).as_millis() as u64;
            let bytes_diff = downloaded - last_bytes;
            let speed = if elapsed > 0 {
                (bytes_diff * 1000) / elapsed
            } else {
                0
            };

            // 更新任务状态
            {
                let manager = app.state::<DownloadManager>();
                let mut tasks = manager.tasks.lock().unwrap();
                if let Some(t) = tasks.get_mut(&task.id) {
                    t.downloaded_size = downloaded;
                    t.speed = speed;
                }
            }

            // 发送进度事件
            let _ = app.emit("download-progress", serde_json::json!({
                "id": task.id,
                "downloaded": downloaded,
                "total": total_size,
                "speed": speed,
                "progress": if total_size > 0 { (downloaded as f64 / total_size as f64 * 100.0) as u8 } else { 0 },
            }));

            last_update = now;
            last_bytes = downloaded;
        }
    }

    // 下载完成
    let final_task = {
        let manager = app.state::<DownloadManager>();
        let mut tasks = manager.tasks.lock().unwrap();
        let mut done = None;
        if let Some(t) = tasks.get_mut(&task.id) {
            t.status = DownloadStatus::Completed;
            t.downloaded_size = downloaded;
            t.completed_at = Some(chrono::Utc::now().timestamp() as u64);
            t.speed = 0;
            done = Some(t.clone());
        }
        save_history(&app, &tasks);
        done
    };

    // 发送最终事件（含完成时间）
    if let Some(t) = final_task {
        let _ = app.emit(
            "download-final",
            serde_json::json!({
                "id": t.id,
                "status": "Completed",
                "filename": t.filename,
                "file_path": t.file_path,
                "finished_at": t.completed_at,
                "total": t.downloaded_size,
                "error": null,
            }),
        );
    }

    // 发送完成事件（兼容旧版）
    let _ = app.emit(
        "download-complete",
        serde_json::json!({
            "id": task.id,
        }),
    );

    Ok(())
}

// ===== 文件名解析 =====

/// 通过 HEAD 请求获取服务器返回的真实文件名（解析 Content-Disposition 响应头）
///
/// 很多下载链接（S3 签名链接 / CDN / REST API）URL 中不含真实文件名，
/// 但响应头会携带。浏览器下载正是依赖此机制。
/// 获取失败（HEAD 不支持、无响应头、网络错误）时返回 None，由调用方 fallback。
async fn fetch_filename_from_server(client: &reqwest::Client, url: &str) -> Option<String> {
    // 设置超时，避免添加任务卡住
    let resp = client
        .head(url)
        .timeout(Duration::from_secs(5))
        .send()
        .await
        .ok()?;
    if !resp.status().is_success() {
        return None;
    }
    let value = resp
        .headers()
        .get(reqwest::header::CONTENT_DISPOSITION)?
        .to_str()
        .ok()?;
    parse_content_disposition(value)
}

/// 从 Content-Disposition 响应头解析真实文件名
///
/// 浏览器下载时正是依赖此响应头获取文件名。支持两种格式：
/// - `filename="xxx.zip"`（旧式，ASCII/百分号编码）
/// - `filename*=UTF-8''%E6%96%87%E4%BB%B6.zip`（RFC 5987，支持 UTF-8）
fn parse_content_disposition(header: &str) -> Option<String> {
    // 优先 filename*（RFC 5987，支持中文等 UTF-8 文件名）
    for part in header.split(';') {
        let part = part.trim();
        if let Some(value) = part.strip_prefix("filename*=") {
            let value = value.trim().trim_matches('"');
            // 跳过编码声明，如 UTF-8''xxx 或 UTF-8'zh-CN'xxx
            if let Some(first) = value.find('\'') {
                if let Some(second) = value[first + 1..].find('\'') {
                    let encoded = &value[first + 1 + second + 1..];
                    return Some(sanitize_filename(&percent_decode(encoded)));
                }
            }
            return Some(sanitize_filename(&value));
        }
    }

    // 其次 filename="xxx.zip" 或 filename=xxx.zip
    for part in header.split(';') {
        let part = part.trim();
        if let Some(value) = part.strip_prefix("filename=") {
            let value = value.trim().trim_matches('"');
            if !value.is_empty() {
                return Some(sanitize_filename(&percent_decode(value)));
            }
        }
    }

    None
}

/// 百分号解码（如 %E6%96%87 → 文）
fn percent_decode(s: &str) -> String {
    let bytes = s.as_bytes();
    let mut out = Vec::with_capacity(bytes.len());
    let mut i = 0;
    while i < bytes.len() {
        if bytes[i] == b'%' && i + 2 < bytes.len() {
            if let Ok(hex) = std::str::from_utf8(&bytes[i + 1..i + 3]) {
                if let Ok(v) = u8::from_str_radix(hex, 16) {
                    out.push(v);
                    i += 3;
                    continue;
                }
            }
        }
        out.push(bytes[i]);
        i += 1;
    }
    String::from_utf8_lossy(&out).to_string()
}

/// 清理文件名：去除路径分隔符和首尾空白/点，防止路径穿越
fn sanitize_filename(name: &str) -> String {
    let name = name.replace("/", "_").replace("\\", "_");
    let name = name.trim().trim_matches('.');
    if name.is_empty() {
        "download".to_string()
    } else {
        name.to_string()
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn test_parse_huggingface_cdn_header() {
        // HuggingFace / S3 签名链接返回的头，与用户提供的 URL 对应
        let header = "attachment; filename*=UTF-8''Qwen3.8-27B-UD-Q4_K_M.gguf; filename=\"Qwen3.8-27B-UD-Q4_K_M.gguf\";";
        assert_eq!(
            parse_content_disposition(header).unwrap(),
            "Qwen3.8-27B-UD-Q4_K_M.gguf"
        );
    }

    #[test]
    fn test_parse_utf8_encoded_filename() {
        // filename* 带百分号编码（中文文件名）
        let header = "attachment; filename*=UTF-8''%E6%96%87%E4%BB%B6%E5%90%8D.zip";
        assert_eq!(parse_content_disposition(header).unwrap(), "文件名.zip");
    }

    #[test]
    fn test_parse_plain_filename() {
        // 旧式 filename=，带引号
        let header = "attachment; filename=\"report.pdf\"";
        assert_eq!(parse_content_disposition(header).unwrap(), "report.pdf");
    }

    #[test]
    fn test_parse_filename_without_quotes() {
        // 无引号
        let header = "attachment; filename=report.pdf";
        assert_eq!(parse_content_disposition(header).unwrap(), "report.pdf");
    }

    #[test]
    fn test_parse_prefers_star_over_plain() {
        // 同时存在 filename* 和 filename 时优先 filename*
        let header =
            "attachment; filename*=UTF-8''%E6%96%87%E4%BB%B6.zip; filename=\"fallback.zip\"";
        assert_eq!(parse_content_disposition(header).unwrap(), "文件.zip");
    }

    #[test]
    fn test_sanitize_path_traversal() {
        // 恶意文件名：路径穿越。`/` 被替换为 `_`，前导点被去除，
        // 结果不可能包含路径分隔符，也不会以 `.`/`..` 开头
        let header = "attachment; filename=\"../../evil.sh\"";
        assert_eq!(parse_content_disposition(header).unwrap(), "_.._evil.sh");
    }

    #[test]
    fn test_no_filename_header() {
        assert_eq!(parse_content_disposition("inline"), None);
        assert_eq!(parse_content_disposition(""), None);
    }
}
