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

pub(crate) mod config;
pub(crate) mod media;
pub(crate) mod stack;
#[cfg(test)]
mod tests;

pub(crate) use config::*;
pub(crate) use media::*;
pub(crate) use stack::*;
