use std::path::{Path, PathBuf};
use std::sync::RwLock;

use base64::Engine as _;

use agent_core::astra_generation::openai_images::ImageSink;
use agent_core::astra_generation::{MediaKind, MediaLocation, MediaRef};
use agent_core::task::AssetStore;
use agent_core::Error;
use async_trait::async_trait;
use serde::Serialize;
use tauri::{AppHandle, Manager};

/// 单个媒体资产字节上限（§8：下载前先看 Content-Length，流式读取再复核）
pub(crate) const MAX_ASSET_BYTES: usize = 100 * 1024 * 1024;
/// 下载超时（整体请求，含读体）
const DOWNLOAD_TIMEOUT: std::time::Duration = std::time::Duration::from_secs(120);
/// PNG 魔数（ImageSink 只接受 PNG：OpenAI Images 适配器默认产出 image/png）
const PNG_MAGIC: &[u8] = &[0x89, 0x50, 0x4E, 0x47, 0x0D, 0x0A, 0x1A, 0x0A];

// ===== 媒体存储：AssetStore（下载侧） + ImageSink（生成侧） =====

/// 宿主媒体存储：所有生成资产统一落盘 `media/{asset_id}`（asset_id 为 UUID，
/// MIME 保存在 MediaRef 里，不落在文件名上）。
pub struct MediaStore {
    pub(crate) dir: RwLock<PathBuf>,
    http: reqwest::Client,
}

impl MediaStore {
    pub fn new(dir: PathBuf) -> Self {
        Self {
            dir: RwLock::new(dir),
            // 与 astra-core 的 OpenAIImagesAdapter 同理：loopback 媒体地址
            // （自托管网关/测试服务）不能被系统代理劫持。
            http: reqwest::Client::builder()
                .timeout(DOWNLOAD_TIMEOUT)
                .no_proxy()
                .build()
                .unwrap_or_default(),
        }
    }

    pub(crate) fn current_dir(&self) -> PathBuf {
        self.dir.read().unwrap().clone()
    }

    /// 落盘字节（AssetStore 与 ImageSink 共用）：写入即成功，返回 Stored 引用。
    /// 描述性字段（kind / 宽高 / 时长）从来源 MediaRef 保留。
    async fn store_bytes(
        &self,
        bytes: Vec<u8>,
        mime: &str,
        source: &MediaRef,
    ) -> agent_core::Result<MediaRef> {
        if bytes.len() > MAX_ASSET_BYTES {
            return Err(Error::Storage(format!(
                "media asset exceeds the {} MiB limit",
                MAX_ASSET_BYTES / (1024 * 1024)
            )));
        }
        let asset_id = uuid::Uuid::new_v4().to_string();
        let dir = self.current_dir();
        tokio::fs::create_dir_all(&dir)
            .await
            .map_err(|e| Error::Storage(format!("创建媒体目录失败: {e}")))?;
        tokio::fs::write(dir.join(&asset_id), &bytes)
            .await
            .map_err(|e| Error::Storage(format!("写入媒体文件失败: {e}")))?;
        let mime_type = if mime.is_empty() {
            source.mime_type.clone()
        } else {
            mime.to_string()
        };
        Ok(MediaRef {
            asset_id: asset_id.clone(),
            kind: source.kind,
            mime_type,
            byte_size: bytes.len() as u64,
            width: source.width,
            height: source.height,
            duration_ms: source.duration_ms,
            location: MediaLocation::Stored { asset_id },
        })
    }
}

/// MIME 宽松校验：缺失或通用二进制类型放行（CDN 常见），明确非媒体类型拒绝
pub(crate) fn mime_acceptable(mime: &str, kind: MediaKind) -> bool {
    let base = mime
        .split(';')
        .next()
        .unwrap_or("")
        .trim()
        .to_ascii_lowercase();
    if base.is_empty() || base == "application/octet-stream" {
        return true;
    }
    match kind {
        MediaKind::Image => base.starts_with("image/"),
        MediaKind::Video => base.starts_with("video/"),
    }
}

pub(crate) fn ensure_within_limit(len: usize) -> agent_core::Result<()> {
    if len > MAX_ASSET_BYTES {
        return Err(Error::Storage(format!(
            "media asset exceeds the {} MiB limit",
            MAX_ASSET_BYTES / (1024 * 1024)
        )));
    }
    Ok(())
}

#[async_trait]
impl AssetStore for MediaStore {
    /// 把 TemporaryUrl 下载落盘为 Stored（§8：下载失败 = 未持久化，可安全重试，
    /// 绝不触发重新生成）；已是 Stored 的输入原样返回（幂等）。
    async fn persist(&self, media: &MediaRef) -> agent_core::Result<MediaRef> {
        if !media.needs_download() {
            return Ok(media.clone());
        }
        let url = match &media.location {
            MediaLocation::TemporaryUrl { url } => url,
            MediaLocation::Stored { .. } => return Ok(media.clone()),
        };
        if !(url.starts_with("http://") || url.starts_with("https://")) {
            return Err(Error::Storage(
                "media download URL must be http(s)".to_string(),
            ));
        }
        let resp = self
            .http
            .get(url)
            .send()
            .await
            .map_err(|e| Error::Storage(format!("媒体下载请求失败: {e}")))?;
        if !resp.status().is_success() {
            return Err(Error::Storage(format!(
                "媒体下载失败：HTTP {}",
                resp.status()
            )));
        }
        // 大小预检：Content-Length 超限直接拒绝，不进入读体
        if let Some(len) = resp.content_length() {
            ensure_within_limit(len as usize)?;
        }
        let mime = resp
            .headers()
            .get(reqwest::header::CONTENT_TYPE)
            .and_then(|v| v.to_str().ok())
            .unwrap_or("")
            .to_string();
        if !mime_acceptable(&mime, media.kind) {
            return Err(Error::Storage(format!(
                "媒体下载被拒绝：非预期的 Content-Type '{mime}'"
            )));
        }
        // 流式读体并复核大小（Content-Length 可能缺失或撒谎）
        use futures_util::StreamExt;
        let mut stream = resp.bytes_stream();
        let mut bytes: Vec<u8> = Vec::new();
        while let Some(chunk) = stream.next().await {
            let chunk = chunk.map_err(|e| Error::Storage(format!("媒体下载中断: {e}")))?;
            ensure_within_limit(bytes.len() + chunk.len())?;
            bytes.extend_from_slice(&chunk);
        }
        self.store_bytes(bytes, &mime, media).await
    }
}

#[async_trait]
impl ImageSink for MediaStore {
    /// b64 解码后的生成结果落盘：校验 PNG 头，返回 asset id。
    /// `Err` = 未持久化；调用方只报失败，绝不静默重提交（Images API 无幂等键，
    /// 重提交 = 第二次付费任务）。
    async fn persist(
        &self,
        bytes: Vec<u8>,
        mime_type: &str,
    ) -> std::result::Result<String, String> {
        if !bytes.starts_with(PNG_MAGIC) {
            return Err("生成结果不是 PNG 图片，已拒绝落盘".to_string());
        }
        if let Err(e) = ensure_within_limit(bytes.len()) {
            return Err(e.to_string());
        }
        let source = MediaRef::stored("", MediaKind::Image, mime_type, 0);
        let stored = self
            .store_bytes(bytes, mime_type, &source)
            .await
            .map_err(|e| e.to_string())?;
        Ok(stored.asset_id)
    }
}

// ===== 媒体资产读取/打开（前端预览与系统查看，MULTIMODAL_SPEC §10）=====

/// 预览 data URL 的字节上限：生成图片通常几 MB；超过 20 MiB 的资产仍可
/// 「打开」查看，但不再内联预览（避免 IPC 大字符串拖慢 WebView）。
const PREVIEW_MAX_BYTES: usize = 20 * 1024 * 1024;

impl MediaStore {
    /// 校验 asset_id 并解析到媒体目录下的文件路径。
    /// asset_id 一律由本进程 UUID 生成；外部传入的值必须是不含路径分隔符
    /// 的短字母数字串，防止路径穿越。
    fn resolve_asset(&self, asset_id: &str) -> Result<PathBuf, String> {
        let valid = !asset_id.is_empty()
            && asset_id.len() <= 64
            && asset_id
                .chars()
                .all(|c| c.is_ascii_alphanumeric() || c == '-' || c == '_');
        if !valid {
            return Err("非法的 asset_id".to_string());
        }
        Ok(self.current_dir().join(asset_id))
    }

    /// 供测试断言路径解析（命令层走 read_asset）。
    #[cfg(test)]
    pub fn asset_path(&self, asset_id: &str) -> Result<PathBuf, String> {
        self.resolve_asset(asset_id)
    }

    /// 读取资产字节（落盘时已限 100 MiB，此处再按上限复核）。
    pub async fn read_asset(&self, asset_id: &str) -> Result<Vec<u8>, String> {
        let path = self.resolve_asset(asset_id)?;
        let meta = tokio::fs::metadata(&path)
            .await
            .map_err(|_| format!("媒体资产不存在: {asset_id}"))?;
        if !meta.is_file() {
            return Err(format!("媒体资产不是文件: {asset_id}"));
        }
        if meta.len() as usize > MAX_ASSET_BYTES {
            return Err(format!(
                "媒体资产超过 {} MiB 上限",
                MAX_ASSET_BYTES / (1024 * 1024)
            ));
        }
        tokio::fs::read(&path)
            .await
            .map_err(|e| format!("读取媒体资产失败: {e}"))
    }
}

/// 魔数嗅探 MIME（资产文件名无扩展名，MIME 存在 MediaRef 里不落盘）。
/// 仅识别本系统产出的类型；未知返回 octet-stream。
pub(crate) fn sniff_mime(bytes: &[u8]) -> &'static str {
    if bytes.starts_with(PNG_MAGIC) {
        "image/png"
    } else if bytes.starts_with(&[0xFF, 0xD8, 0xFF]) {
        "image/jpeg"
    } else if bytes.starts_with(b"GIF87a") || bytes.starts_with(b"GIF89a") {
        "image/gif"
    } else if bytes.len() >= 12 && bytes.starts_with(b"RIFF") && &bytes[8..12] == b"WEBP" {
        "image/webp"
    } else if bytes.len() >= 12 && bytes.starts_with(b"RIFF") && &bytes[8..12] == b"AVI " {
        "video/x-msvideo"
    } else if bytes.len() >= 12 && &bytes[4..8] == b"ftyp" {
        "video/mp4"
    } else if bytes.starts_with(&[0x1A, 0x45, 0xDF, 0xA3]) {
        "video/webm"
    } else {
        "application/octet-stream"
    }
}

#[derive(Serialize)]
pub struct MediaAssetPayload {
    pub data_url: String,
    pub mime_type: String,
    pub byte_size: usize,
}

/// 读取资产并编码为 data URL（前端 <img> 内联预览用）。
#[tauri::command]
pub async fn media_read_asset(
    app: AppHandle,
    asset_id: String,
) -> Result<MediaAssetPayload, String> {
    let media = app.state::<crate::agent::AgentState>().generation.media();
    let bytes = media.read_asset(&asset_id).await?;
    if bytes.len() > PREVIEW_MAX_BYTES {
        return Err(format!(
            "资产超过预览上限（{} MiB），请用「打开」查看",
            PREVIEW_MAX_BYTES / (1024 * 1024)
        ));
    }
    let mime_type = sniff_mime(&bytes).to_string();
    Ok(MediaAssetPayload {
        data_url: format!(
            "data:{mime_type};base64,{}",
            base64::engine::general_purpose::STANDARD.encode(&bytes)
        ),
        mime_type,
        byte_size: bytes.len(),
    })
}

/// 用系统默认应用打开资产：文件无扩展名，先按嗅探到的 MIME 复制到临时
/// 文件（带扩展名）再交给系统打开。
#[tauri::command]
pub async fn media_open_asset(app: AppHandle, asset_id: String) -> Result<(), String> {
    let media = app.state::<crate::agent::AgentState>().generation.media();
    let bytes = media.read_asset(&asset_id).await?;
    let mime = sniff_mime(&bytes);
    let ext = match mime {
        "image/png" => "png",
        "image/jpeg" => "jpg",
        "image/gif" => "gif",
        "image/webp" => "webp",
        "video/mp4" => "mp4",
        "video/webm" => "webm",
        "video/x-msvideo" => "avi",
        _ => "bin",
    };
    let tmp = std::env::temp_dir().join(format!("astra-asset-{asset_id}.{ext}"));
    tokio::fs::write(&tmp, &bytes)
        .await
        .map_err(|e| format!("写入临时文件失败: {e}"))?;
    open_with_system(&tmp)
}

/// 跨平台系统打开（与 downloader.rs 同一套模式）。
fn open_with_system(path: &Path) -> Result<(), String> {
    #[cfg(target_os = "macos")]
    {
        std::process::Command::new("open")
            .arg(path)
            .spawn()
            .map_err(|e| format!("打开文件失败: {e}"))?
            .wait()
            .map_err(|e| format!("等待打开进程失败: {e}"))?;
        Ok(())
    }
    #[cfg(target_os = "windows")]
    {
        std::process::Command::new("explorer")
            .arg(path)
            .spawn()
            .map_err(|e| format!("打开文件失败: {e}"))?;
        Ok(())
    }
    #[cfg(target_os = "linux")]
    {
        std::process::Command::new("xdg-open")
            .arg(path)
            .spawn()
            .map_err(|e| format!("打开文件失败: {e}"))?;
        Ok(())
    }
    #[cfg(not(any(target_os = "macos", target_os = "windows", target_os = "linux")))]
    {
        let _ = path;
        Err("当前平台不支持打开文件".to_string())
    }
}
