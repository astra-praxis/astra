use std::path::Path;
use std::sync::{Arc, RwLock};

use agent_core::astra_generation::error::rejected;
use agent_core::astra_generation::openai_images::ImageSink;
use agent_core::astra_generation::{
    GenerationError, GenerationProvider, OpenAIImageModel, OpenAIImagesConfig, OpenAIImagesProvider,
};
use agent_core::model::capability::{ModelDescriptor, OperationConstraints};
use agent_core::task::{AssetStore, GenerationTaskBackend};
use async_trait::async_trait;

use super::*;

// ===== 供应商装配（按配置懒构建 / 懒切换） =====

/// 当前生效的 provider 及其构建时的配置指纹（用于「配置没变就不重建」）
struct ActiveProvider {
    config: GenerationConfig,
    provider: Arc<dyn GenerationProvider>,
}

/// 可切换的 provider 壳：TaskManager 与 generate_image 工具持有同一份
/// Arc；配置变更（新会话生效）时只替换内部 provider，TaskManager 的
/// generation 后端注册保持不变（其 provider 协议是纯只读组合）。
pub(crate) struct SwitchableProvider {
    id: String,
    inner: RwLock<Option<ActiveProvider>>,
}

impl SwitchableProvider {
    pub(crate) fn new() -> Self {
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
    pub(crate) media: Arc<MediaStore>,
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
