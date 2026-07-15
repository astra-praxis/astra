use serde::{Deserialize, Serialize};
use std::collections::HashMap;
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
}

impl DownloadManager {
    pub fn new() -> Self {
        Self {
            tasks: std::sync::Mutex::new(HashMap::new()),
            cancel_flags: std::sync::Mutex::new(HashMap::new()),
        }
    }
}

// ===== Tauri 命令 =====

/// 获取下载列表
#[tauri::command]
pub async fn list_downloads(app: AppHandle) -> Result<Vec<DownloadTask>, String> {
    let manager = app.state::<DownloadManager>();
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

    // 从 URL 提取文件名
    let filename = filename.unwrap_or_else(|| {
        url.split('/')
            .last()
            .map(|s| {
                // 移除查询参数
                s.split('?').next().unwrap_or("download").to_string()
            })
            .unwrap_or_else(|| "download".to_string())
    });

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
        manager
            .tasks
            .lock()
            .unwrap()
            .insert(id.clone(), task.clone());
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
    let app_handle = app.clone();
    let cancel = cancel_flag.clone();
    let task_id_clone = task_id.clone();

    tokio::spawn(async move {
        if let Err(e) = download_file(app_handle, task_id_clone, task, cancel).await {
            eprintln!("下载失败: {}", e);
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

    // 删除文件
    if delete_file {
        if let Some(task) = task {
            tokio::spawn(async move {
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

// ===== 内部下载逻辑 =====

async fn download_file(
    app: AppHandle,
    _task_id: String,
    mut task: DownloadTask,
    cancel: Arc<AtomicBool>,
) -> Result<(), String> {
    let client = reqwest::Client::new();

    // 检查已下载的文件大小（断点续传）
    let existing_size = match async_fs::metadata(&task.file_path).await {
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
    {
        let manager = app.state::<DownloadManager>();
        let mut tasks = manager.tasks.lock().unwrap();
        if let Some(t) = tasks.get_mut(&task.id) {
            t.status = DownloadStatus::Completed;
            t.downloaded_size = downloaded;
            t.completed_at = Some(chrono::Utc::now().timestamp() as u64);
            t.speed = 0;
        }
    }

    // 发送完成事件
    let _ = app.emit(
        "download-complete",
        serde_json::json!({
            "id": task.id,
        }),
    );

    Ok(())
}
