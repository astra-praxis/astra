//! 多模态 M5 宿主接线（astra-core/docs/MULTIMODAL_SPEC.md §8/§10）。
//!
//! 职责边界：生成协议、供应商适配器与任务状态机都在 astra-core
//! （astra-generation / astra-task / astra-tools::generation_tools），
//! 本模块只做「宿主适配」（MULTIMODAL_SPEC §3 宿主清单）：
//! - 配置与密钥：generation.json（app_data_dir）的读写与命令暴露
//!   （enabled / base_url / api_key / model_id）
//! - 资源存储实现：[`MediaStore`] 同时实现下载侧 [`AssetStore`]
//!   （TemporaryUrl → Stored，落盘 `app_data_dir/media/{asset_id}`，幂等）
//!   与生成侧 [`ImageSink`]（b64 解码字节落盘，PNG 头校验，返回 asset id）
//! - 供应商装配：按配置构建 [`OpenAIImagesProvider`]，经 [`SwitchableProvider`]
//!   接入 [`GenerationTaskBackend`]；TaskManager 的后端注册在 AgentState
//!   构造期一次性完成，配置变更（新会话生效）只替换内部 provider
//! - disabled / 配置缺失 / 构建失败：不注册工具、不报错、不阻塞会话
//!
//! 安全边界（§8）：下载有大小上限（100 MiB）与超时；MIME 宽松校验（缺失或
//! 通用类型放行，明确的非媒体类型拒绝）；二进制不进入事件、任务 JSON 或日志。
//! 供应商临时 URL 视为受信来源（SSRF 内网策略暂不在此实现，后续接入任意
//! URL 下载时再加固）。

use std::path::{Path, PathBuf};
use std::sync::{Arc, RwLock};

use base64::Engine as _;

use agent_core::astra_generation::error::rejected;
use agent_core::astra_generation::openai_images::ImageSink;
use agent_core::astra_generation::{
    GenerationError, GenerationProvider, MediaKind, MediaLocation, MediaRef, OpenAIImageModel,
    OpenAIImagesConfig, OpenAIImagesProvider,
};
use agent_core::model::capability::{ModelDescriptor, OperationConstraints};
use agent_core::task::{AssetStore, GenerationTaskBackend};
use agent_core::Error;
use async_trait::async_trait;
use serde::{Deserialize, Serialize};
use tauri::{AppHandle, Manager};

/// 单个媒体资产字节上限（§8：下载前先看 Content-Length，流式读取再复核）
const MAX_ASSET_BYTES: usize = 100 * 1024 * 1024;
/// 下载超时（整体请求，含读体）
const DOWNLOAD_TIMEOUT: std::time::Duration = std::time::Duration::from_secs(120);
/// PNG 魔数（ImageSink 只接受 PNG：OpenAI Images 适配器默认产出 image/png）
const PNG_MAGIC: &[u8] = &[0x89, 0x50, 0x4E, 0x47, 0x0D, 0x0A, 0x1A, 0x0A];

// ===== 配置（generation.json）=====

/// 生成供应商配置；文件缺失 = 默认值（disabled）
#[derive(Debug, Clone, Default, PartialEq, Eq, Serialize, Deserialize)]
#[serde(default)]
pub struct GenerationConfig {
    pub enabled: bool,
    pub base_url: String,
    pub api_key: String,
    pub model_id: String,
}

impl GenerationConfig {
    /// 归一化（trim + 去尾部 /），set_config 落盘前调用
    fn normalized(mut self) -> Self {
        self.base_url = self.base_url.trim().trim_end_matches('/').to_string();
        self.api_key = self.api_key.trim().to_string();
        self.model_id = self.model_id.trim().to_string();
        self
    }

    /// enabled 且配置完整（api_key / model_id 非空）才可用
    fn usable(&self) -> bool {
        self.enabled && !self.api_key.is_empty() && !self.model_id.is_empty()
    }
}

fn read_config_file(path: &Path) -> GenerationConfig {
    std::fs::read_to_string(path)
        .ok()
        .and_then(|s| serde_json::from_str(&s).ok())
        .unwrap_or_default()
}

fn write_config_file(path: &Path, config: &GenerationConfig) -> Result<(), String> {
    if let Some(parent) = path.parent() {
        std::fs::create_dir_all(parent).map_err(|e| format!("创建配置目录失败: {e}"))?;
    }
    let content = serde_json::to_string_pretty(&config.clone().normalized()).unwrap_or_default();
    std::fs::write(path, content).map_err(|e| format!("写入配置文件失败: {e}"))
}

fn generation_config_path(app: &AppHandle) -> Result<PathBuf, String> {
    let dir = app
        .path()
        .app_data_dir()
        .map_err(|e| format!("获取应用数据目录失败: {e}"))?;
    Ok(dir.join("generation.json"))
}

/// 读取生成配置（文件缺失/损坏 → 默认 disabled，不报错）
pub fn load_config(app: &AppHandle) -> GenerationConfig {
    match generation_config_path(app) {
        Ok(path) => read_config_file(&path),
        Err(_) => GenerationConfig::default(),
    }
}

/// 读取生成配置（可注入路径，便于测试与命令复用）
#[tauri::command]
pub fn generation_get_config(app: AppHandle) -> Result<GenerationConfig, String> {
    let path = generation_config_path(&app)?;
    Ok(read_config_file(&path))
}

/// 保存生成配置。新配置对之后新建的会话生效（已开 Thread 的工具集合冻结）。
#[tauri::command]
pub fn generation_set_config(app: AppHandle, config: GenerationConfig) -> Result<(), String> {
    let path = generation_config_path(&app)?;
    write_config_file(&path, &config)
}

// ===== 媒体存储：AssetStore（下载侧） + ImageSink（生成侧） =====

/// 宿主媒体存储：所有生成资产统一落盘 `media/{asset_id}`（asset_id 为 UUID，
/// MIME 保存在 MediaRef 里，不落在文件名上）。
pub struct MediaStore {
    dir: RwLock<PathBuf>,
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

    fn current_dir(&self) -> PathBuf {
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
fn mime_acceptable(mime: &str, kind: MediaKind) -> bool {
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

fn ensure_within_limit(len: usize) -> agent_core::Result<()> {
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
fn sniff_mime(bytes: &[u8]) -> &'static str {
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

// ===== 供应商装配（按配置懒构建 / 懒切换） =====

/// 当前生效的 provider 及其构建时的配置指纹（用于「配置没变就不重建」）
struct ActiveProvider {
    config: GenerationConfig,
    provider: Arc<dyn GenerationProvider>,
}

/// 可切换的 provider 壳：TaskManager 与 generate_image 工具持有同一份
/// Arc；配置变更（新会话生效）时只替换内部 provider，TaskManager 的
/// generation 后端注册保持不变（其 provider 协议是纯只读组合）。
struct SwitchableProvider {
    id: String,
    inner: RwLock<Option<ActiveProvider>>,
}

impl SwitchableProvider {
    fn new() -> Self {
        Self {
            id: "astra-generation".to_string(),
            inner: RwLock::new(None),
        }
    }

    fn set(&self, config: GenerationConfig, provider: Arc<dyn GenerationProvider>) {
        *self.inner.write().unwrap() = Some(ActiveProvider { config, provider });
    }

    fn clear(&self) {
        *self.inner.write().unwrap() = None;
    }

    fn matches(&self, config: &GenerationConfig) -> bool {
        self.inner
            .read()
            .unwrap()
            .as_ref()
            .map(|a| &a.config == config)
            .unwrap_or(false)
    }

    fn current(&self) -> Option<Arc<dyn GenerationProvider>> {
        self.inner
            .read()
            .unwrap()
            .as_ref()
            .map(|a| Arc::clone(&a.provider))
    }
}

#[async_trait]
impl GenerationProvider for SwitchableProvider {
    fn id(&self) -> &str {
        &self.id
    }

    fn descriptor(&self, model_id: &str) -> Option<ModelDescriptor> {
        self.current().and_then(|p| p.descriptor(model_id))
    }

    /// 未配置时的兜底：正常流程下 disabled = 工具不注册，此路径不可达；
    /// 配置被切换/清除后旧任务轮询到此错误即终态化（不重试、不重提交）。
    async fn submit(
        &self,
        request: &agent_core::astra_generation::GenerationRequest,
        client_request_id: &str,
    ) -> std::result::Result<agent_core::astra_generation::SubmitOutcome, GenerationError> {
        match self.current() {
            Some(p) => p.submit(request, client_request_id).await,
            None => Err(rejected("generation provider is not configured")),
        }
    }

    async fn status(
        &self,
        handle: &agent_core::astra_generation::ProviderTaskHandle,
    ) -> std::result::Result<agent_core::astra_generation::ProviderObservation, GenerationError>
    {
        match self.current() {
            Some(p) => p.status(handle).await,
            None => Err(rejected("generation provider is not configured")),
        }
    }

    async fn result(
        &self,
        handle: &agent_core::astra_generation::ProviderTaskHandle,
    ) -> std::result::Result<Vec<agent_core::astra_generation::GenerationOutput>, GenerationError>
    {
        match self.current() {
            Some(p) => p.result(handle).await,
            None => Err(rejected("generation provider is not configured")),
        }
    }

    async fn cancel(
        &self,
        handle: &agent_core::astra_generation::ProviderTaskHandle,
    ) -> std::result::Result<agent_core::astra_generation::CancelOutcome, GenerationError> {
        match self.current() {
            Some(p) => p.cancel(handle).await,
            None => Err(rejected("generation provider is not configured")),
        }
    }
}

/// 自定义模型的约束声明：与 OpenAIImagesProvider 的尺寸映射及
/// `sends_response_format=false`（gpt-image-1 风格网关）一致，镜像
/// `OpenAIImageModel::gpt_image_1` 预设；32k prompt 上限在装配时传入。
fn image_constraints() -> OperationConstraints {
    OperationConstraints {
        max_input_images: Some(0),
        max_output_images: Some(10),
        aspect_ratios: Some(vec!["1:1".into(), "3:2".into(), "2:3".into()]),
        resolutions: Some(vec![
            "1024x1024".into(),
            "1536x1024".into(),
            "1024x1536".into(),
            "auto".into(),
        ]),
        ..Default::default()
    }
}

/// generation 栈：backend 一次性注册进共享 TaskManager；provider 按配置
/// 懒构建/懒切换。media 目录占位（temp），首次 sync 时指向真实 app_data_dir。
pub struct GenerationStack {
    backend: Arc<GenerationTaskBackend>,
    switch: Arc<SwitchableProvider>,
    media: Arc<MediaStore>,
}

impl Default for GenerationStack {
    fn default() -> Self {
        Self::new()
    }
}

impl GenerationStack {
    pub fn new() -> Self {
        let switch = Arc::new(SwitchableProvider::new());
        let media = Arc::new(MediaStore::new(std::env::temp_dir().join("astra-media")));
        let backend = Arc::new(GenerationTaskBackend::new(
            Arc::clone(&switch) as Arc<dyn GenerationProvider>,
            Arc::clone(&media) as Arc<dyn AssetStore>,
        ));
        Self {
            backend,
            switch,
            media,
        }
    }

    /// TaskManager 构造期一次性注册用
    pub fn task_backend(&self) -> Arc<GenerationTaskBackend> {
        Arc::clone(&self.backend)
    }

    /// 资产读取/打开命令用（media_read_asset / media_open_asset）
    pub fn media(&self) -> Arc<MediaStore> {
        Arc::clone(&self.media)
    }

    /// 读取配置并按需构建/替换 provider。返回 Some((backend, provider)) 表示
    /// enabled 且配置完整，build_registry 应注册 generate_image；
    /// disabled / 配置缺失 / 构建失败 → None（不报错，不阻塞会话）。
    pub fn sync(
        &self,
        app_data_dir: &Path,
        config: GenerationConfig,
    ) -> Option<(Arc<GenerationTaskBackend>, Arc<dyn GenerationProvider>)> {
        // 媒体目录：app_data_dir/media（AssetStore 与 ImageSink 共用）
        *self.media.dir.write().unwrap() = app_data_dir.join("media");

        if !config.usable() {
            self.switch.clear();
            return None;
        }
        if self.switch.matches(&config) {
            let provider = self.switch.current()?;
            return Some((Arc::clone(&self.backend), provider));
        }

        let config = config.normalized();
        let model =
            OpenAIImageModel::custom(config.model_id.clone(), image_constraints(), 32_000, false);
        let mut cfg = OpenAIImagesConfig::new(config.api_key.clone(), vec![model]);
        if !config.base_url.is_empty() {
            cfg = cfg.with_base_url(config.base_url.clone());
        }
        match OpenAIImagesProvider::new(cfg, Arc::clone(&self.media) as Arc<dyn ImageSink>) {
            Ok(provider) => {
                let provider: Arc<dyn GenerationProvider> = Arc::new(provider);
                self.switch.set(config, Arc::clone(&provider));
                Some((Arc::clone(&self.backend), provider))
            }
            Err(e) => {
                // HTTP 客户端构建失败（环境级问题）：降级为不可用，不阻塞会话
                eprintln!("[generation] provider 构建失败，generate_image 未注册: {e}");
                self.switch.clear();
                None
            }
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn tmp_dir(tag: &str) -> PathBuf {
        let dir = std::env::temp_dir().join(format!(
            "astra-agent-generation-{tag}-{}",
            uuid::Uuid::new_v4()
        ));
        std::fs::create_dir_all(&dir).unwrap();
        dir
    }

    /// 1×1 红色 PNG（与 agent_images 测试同一份本地生成的字节）
    fn tiny_png() -> Vec<u8> {
        const PNG: &[u8] = &[
            0x89, 0x50, 0x4E, 0x47, 0x0D, 0x0A, 0x1A, 0x0A, 0x00, 0x00, 0x00, 0x0D, 0x49, 0x48,
            0x44, 0x52, 0x00, 0x00, 0x00, 0x01, 0x00, 0x00, 0x00, 0x01, 0x08, 0x02, 0x00, 0x00,
            0x00, 0x90, 0x77, 0x53, 0xDE, 0x00, 0x00, 0x00, 0x0C, 0x49, 0x44, 0x41, 0x54, 0x08,
            0xD7, 0x63, 0xF8, 0xCF, 0xC0, 0x00, 0x00, 0x03, 0x01, 0x01, 0x00, 0x18, 0xDD, 0x8D,
            0xB0, 0x00, 0x00, 0x00, 0x00, 0x49, 0x45, 0x4E, 0x44, 0xAE, 0x42, 0x60, 0x82,
        ];
        PNG.to_vec()
    }

    /// 本地一次性 HTTP 服务：返回 (url, 线程句柄)。
    /// content_length_override 用于模拟 Content-Length 撒谎场景。
    fn serve_once(content_type: &'static str, body: Vec<u8>) -> String {
        serve_once_with_len(content_type, body, None)
    }

    fn serve_once_with_len(
        content_type: &'static str,
        body: Vec<u8>,
        content_length_override: Option<u64>,
    ) -> String {
        let listener = std::net::TcpListener::bind("127.0.0.1:0").unwrap();
        let addr = listener.local_addr().unwrap();
        std::thread::spawn(move || {
            if let Ok((mut stream, _)) = listener.accept() {
                use std::io::{Read, Write};
                let mut buf = [0u8; 4096];
                let _ = stream.read(&mut buf);
                let len = content_length_override.unwrap_or(body.len() as u64);
                let head = format!(
                    "HTTP/1.1 200 OK\r\nContent-Type: {content_type}\r\n\
                     Content-Length: {len}\r\nConnection: close\r\n\r\n"
                );
                let _ = stream.write_all(head.as_bytes());
                let _ = stream.write_all(&body);
            }
        });
        format!("http://{addr}/asset.bin")
    }

    // ===== ImageSink =====

    #[tokio::test]
    async fn sink_persists_png_bytes_and_returns_asset_id() {
        let dir = tmp_dir("sink");
        let store = MediaStore::new(dir.clone());
        let asset_id = ImageSink::persist(&store, tiny_png(), "image/png")
            .await
            .expect("PNG 应落盘成功");
        assert!(!asset_id.is_empty());
        let path = dir.join(&asset_id);
        let written = tokio::fs::read(&path).await.unwrap();
        assert_eq!(written, tiny_png());
    }

    #[tokio::test]
    async fn sink_rejects_non_png_bytes() {
        let store = MediaStore::new(tmp_dir("sink-bad"));
        assert!(
            ImageSink::persist(&store, b"not a png".to_vec(), "image/png")
                .await
                .is_err()
        );
        // JPEG 头也不接受：ImageSink 契约只收 PNG（适配器默认产出 PNG）
        assert!(
            ImageSink::persist(&store, vec![0xFF, 0xD8, 0xFF, 0xE0], "image/jpeg")
                .await
                .is_err()
        );
    }

    // ===== AssetStore（下载落盘） =====

    #[tokio::test]
    async fn asset_store_downloads_temporary_url_to_stored() {
        let dir = tmp_dir("asset");
        let store = MediaStore::new(dir.clone());
        let url = serve_once("image/png", tiny_png());
        let media = MediaRef::temporary_image(
            "asset-1",
            url,
            "image/png",
            tiny_png().len() as u64,
            Some(1),
            Some(1),
        );
        let stored = AssetStore::persist(&store, &media)
            .await
            .expect("下载应成功");
        assert!(matches!(stored.location, MediaLocation::Stored { .. }));
        assert_eq!(stored.byte_size as usize, tiny_png().len());
        assert_eq!(stored.width, Some(1));
        assert_eq!(stored.height, Some(1));
        let written = tokio::fs::read(dir.join(&stored.asset_id)).await.unwrap();
        assert_eq!(written, tiny_png());

        // 幂等：Stored 输入原样返回，不重复下载
        let again = AssetStore::persist(&store, &stored).await.unwrap();
        assert_eq!(again, stored);
    }

    #[tokio::test]
    async fn asset_store_rejects_non_media_content_type() {
        let store = MediaStore::new(tmp_dir("asset-mime"));
        let url = serve_once("text/html", b"<html>login page</html>".to_vec());
        let media = MediaRef::temporary_image("asset-2", url, "image/png", 1, None, None);
        let err = AssetStore::persist(&store, &media)
            .await
            .unwrap_err()
            .to_string();
        assert!(err.contains("Content-Type"), "实际错误: {err}");
    }

    #[tokio::test]
    async fn asset_store_rejects_oversized_content_length() {
        let store = MediaStore::new(tmp_dir("asset-size"));
        // Content-Length 撒谎成 200MiB：预检直接拒绝，不进入读体
        let url = serve_once_with_len("image/png", tiny_png(), Some(200 * 1024 * 1024));
        let media = MediaRef::temporary_image("asset-3", url, "image/png", 1, None, None);
        let err = AssetStore::persist(&store, &media)
            .await
            .unwrap_err()
            .to_string();
        assert!(err.contains("limit"), "实际错误: {err}");
    }

    #[test]
    fn mime_checks_are_lenient_but_not_permissive() {
        let img = MediaKind::Image;
        // 宽松：缺失 / 通用二进制放行
        assert!(mime_acceptable("", img));
        assert!(mime_acceptable("application/octet-stream", img));
        assert!(mime_acceptable("IMAGE/PNG", img));
        assert!(mime_acceptable("image/jpeg; charset=binary", img));
        // 明确非媒体类型拒绝
        assert!(!mime_acceptable("text/html", img));
        assert!(!mime_acceptable("application/json", img));
        assert!(!mime_acceptable("text/html", MediaKind::Video));
        assert!(mime_acceptable("video/mp4", MediaKind::Video));
    }

    #[test]
    fn size_limit_rejects_over_cap() {
        assert!(ensure_within_limit(0).is_ok());
        assert!(ensure_within_limit(MAX_ASSET_BYTES).is_ok());
        assert!(ensure_within_limit(MAX_ASSET_BYTES + 1).is_err());
    }

    #[tokio::test]
    async fn read_asset_rejects_bad_ids_and_missing_files() {
        let store = MediaStore::new(tmp_dir("read-asset"));
        for bad in ["", "../etc/passwd", "a/b", "a b", "x../..", &"a".repeat(65)] {
            assert!(
                store.read_asset(bad).await.is_err(),
                "should reject {bad:?}"
            );
        }
        assert!(store
            .read_asset("aaaaaaaa-bbbb-cccc-dddd-eeeeeeeeeeee")
            .await
            .is_err());
    }

    #[tokio::test]
    async fn read_asset_round_trips_and_sniffs_mime() {
        let dir = tmp_dir("sniff");
        let store = MediaStore::new(std::path::PathBuf::clone(&dir));
        tokio::fs::write(dir.join("asset-1"), tiny_png())
            .await
            .unwrap();
        let bytes = store.read_asset("asset-1").await.expect("read");
        assert_eq!(bytes, tiny_png());
        assert_eq!(sniff_mime(&bytes), "image/png");
        assert_eq!(sniff_mime(b"GIF89a....."), "image/gif");
        assert_eq!(sniff_mime(&[0xFF, 0xD8, 0xFF, 0xE0]), "image/jpeg");
        let mut mp4 = vec![0u8; 16];
        mp4[4..8].copy_from_slice(b"ftyp");
        assert_eq!(sniff_mime(&mp4), "video/mp4");
        assert_eq!(sniff_mime(b"plain text"), "application/octet-stream");
    }

    // ===== 配置 round-trip =====

    #[test]
    fn config_round_trip_preserves_fields() {
        let dir = tmp_dir("config");
        let path = dir.join("generation.json");
        let config = GenerationConfig {
            enabled: true,
            base_url: "https://gw.example.com/v1/".into(),
            api_key: "sk-test".into(),
            model_id: "gpt-image-1".into(),
        };
        write_config_file(&path, &config).unwrap();
        let loaded = read_config_file(&path);
        assert_eq!(loaded, config.normalized());
        // 归一化：base_url 去尾部 /
        assert_eq!(loaded.base_url, "https://gw.example.com/v1");
    }

    #[test]
    fn missing_or_broken_config_file_defaults_to_disabled() {
        let dir = tmp_dir("config-missing");
        assert_eq!(
            read_config_file(&dir.join("absent.json")),
            GenerationConfig::default()
        );
        let broken = dir.join("broken.json");
        std::fs::write(&broken, "{ not json").unwrap();
        assert_eq!(read_config_file(&broken), GenerationConfig::default());
    }

    // ===== GenerationStack 懒构建 =====

    #[tokio::test]
    async fn sync_disabled_or_incomplete_returns_none() {
        let dir = tmp_dir("stack");
        let stack = GenerationStack::new();

        // disabled → None，且 provider 槽位被清空
        let disabled = GenerationConfig {
            enabled: false,
            base_url: "https://gw.example.com/v1".into(),
            api_key: "sk-test".into(),
            model_id: "gpt-image-1".into(),
        };
        assert!(stack.sync(&dir, disabled).is_none());

        // enabled 但缺 api_key / model_id → None（不报错）
        assert!(stack
            .sync(
                &dir,
                GenerationConfig {
                    enabled: true,
                    api_key: String::new(),
                    model_id: "m".into(),
                    base_url: String::new()
                }
            )
            .is_none());
        assert!(stack
            .sync(
                &dir,
                GenerationConfig {
                    enabled: true,
                    api_key: "sk".into(),
                    model_id: String::new(),
                    base_url: String::new()
                }
            )
            .is_none());
    }

    #[tokio::test]
    async fn sync_enabled_builds_provider_and_reuses_unchanged_config() {
        let dir = tmp_dir("stack-enabled");
        let stack = GenerationStack::new();
        let config = GenerationConfig {
            enabled: true,
            base_url: "https://gw.example.com/v1".into(),
            api_key: "sk-test".into(),
            model_id: "gpt-image-1".into(),
        };

        let (backend, provider) = stack.sync(&dir, config.clone()).expect("enabled 应注册");
        assert!(provider.descriptor("gpt-image-1").is_some());
        assert!(provider.descriptor("unknown-model").is_none());
        // 媒体目录被指向 app_data_dir/media
        assert_eq!(stack.media.current_dir(), dir.join("media"));

        // 配置未变：复用同一 provider（不重建）
        let (_, provider2) = stack.sync(&dir, config.clone()).unwrap();
        assert!(Arc::ptr_eq(&provider, &provider2));

        // 配置变更：重建新 provider，backend 保持同一实例
        let changed = GenerationConfig {
            model_id: "another-image-model".into(),
            ..config
        };
        let (backend2, provider3) = stack.sync(&dir, changed).unwrap();
        assert!(Arc::ptr_eq(&backend, &backend2));
        assert!(!Arc::ptr_eq(&provider2, &provider3));
        assert!(provider3.descriptor("another-image-model").is_some());
        assert!(provider3.descriptor("gpt-image-1").is_none());
    }

    #[tokio::test]
    async fn switchable_provider_errors_without_active_provider() {
        let switch = SwitchableProvider::new();
        let request = agent_core::astra_generation::GenerationRequest {
            model_id: "m".into(),
            operation: agent_core::model::capability::OperationKind::ImageGeneration,
            prompt: "p".into(),
            inputs: vec![],
            parameters: agent_core::astra_generation::RequestParameters::Image {
                image: agent_core::astra_generation::ImageParameters {
                    count: 1,
                    aspect_ratio: None,
                    resolution: None,
                },
            },
            provider_params: Default::default(),
        };
        let err = switch
            .submit(&request, "cr-1")
            .await
            .expect_err("未配置应报错");
        assert!(!err.is_retryable(), "未配置不是可重试的暂时性错误");
        assert!(switch.descriptor("m").is_none());
    }
}
