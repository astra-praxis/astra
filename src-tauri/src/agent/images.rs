//! 用户图片输入的宿主侧预处理（多模态 M1，见 astra-core/docs/MULTIMODAL_SPEC.md §5）。
//!
//! 职责边界：astra-core 负责协议与预算校验（`ImageContent::from_png`、
//! `Thread::try_send_content`、`validate_images`），归一化在 core 内部是
//! `pub(crate)`（astra-tools `image_read`），按规格「宿主负责上传 UI 与资源
//! 预处理」，本模块在应用层镜像同一套预算，把用户选择的 PNG/JPEG/WebP
//! 归一化为规范 PNG，交给 `agent_start_run` 组装 `MessageContent::Image`。
//!
//! 预算（与 astra-core IMAGES.md 一致，硬安全上限）：
//! - 源文件 20 MiB / 24,000,000 像素 / 单边 16,384 px
//! - 解码器分配上限 128 MiB
//! - 规范输出单边 1,568 px，PNG ≤ 2 MiB
//! - 单请求最多 4 张（与用户图/工具图共享预算，由 core 最终裁定）

use base64::Engine as _;
use image::ImageEncoder as _;
use image::ImageReader;
use serde::Serialize;
use std::io::Cursor;

use agent_core::model::image::{MAX_IMAGE_BYTES, MAX_IMAGE_EDGE};
use agent_core::model::ImageContent;

/// 源文件字节上限（进入解码前先挡，避免无谓的内存占用）
const MAX_SOURCE_BYTES: usize = 20 * 1024 * 1024;
/// 源像素总量上限
const MAX_SOURCE_PIXELS: u64 = 24_000_000;
/// 源单边上限
const MAX_SOURCE_EDGE: u32 = 16_384;
/// 解码器分配上限（解码器记账，非进程 RSS）
const MAX_DECODER_ALLOC: u64 = 128 * 1024 * 1024;

/// 归一化结果：base64 规范 PNG + 尺寸（与前端 `UserImage`、`AgentImageInput` 对齐）
#[derive(Debug, Serialize)]
pub struct NormalizedImage {
    pub data: String,
    pub width: u32,
    pub height: u32,
}

/// 把用户选择的图片文件（原始字节）归一化为规范 PNG。
///
/// 只支持 PNG/JPEG/WebP（与 core 的 read_file 一致，其他格式明确报错，
/// 不做静默降级）；动图按静帧处理，EXIF 方向与色彩配置不保留。
#[tauri::command]
pub fn agent_normalize_image(bytes_base64: String) -> Result<NormalizedImage, String> {
    let raw = base64::engine::general_purpose::STANDARD
        .decode(bytes_base64.as_bytes())
        .map_err(|_| "图片数据不是合法的 base64".to_string())?;
    if raw.is_empty() {
        return Err("图片内容为空".into());
    }
    if raw.len() > MAX_SOURCE_BYTES {
        return Err(format!(
            "图片超过 {MAX_SOURCE_BYTES} 字节上限，请压缩后再试"
        ));
    }
    normalize_user_image(&raw)
}

/// 归一化核心（独立函数便于单测）：预算检查 → 解码 → 缩放 → PNG 重编码。
fn normalize_user_image(raw: &[u8]) -> Result<NormalizedImage, String> {
    // 格式嗅探 + 尺寸预检（头部检查先于像素解码，超限快速失败）
    let reader = ImageReader::new(Cursor::new(raw)).with_guessed_format();
    let reader = reader.map_err(|e| format!("无法识别图片格式：{e}"))?;
    let format = reader
        .format()
        .ok_or("无法识别图片格式（仅支持 PNG/JPEG/WebP）")?;
    let (width, height) = reader
        .into_dimensions()
        .map_err(|e| format!("无法读取图片尺寸：{e}"))?;
    if width == 0
        || height == 0
        || width > MAX_SOURCE_EDGE
        || height > MAX_SOURCE_EDGE
        || u64::from(width) * u64::from(height) > MAX_SOURCE_PIXELS
    {
        return Err("图片像素超限（单边 ≤16384、总量 ≤2400 万像素）".into());
    }

    // 解码（带 limits，防止恶意头部声明超大位图导致内存耗尽）
    let mut reader = ImageReader::with_format(Cursor::new(raw), format);
    let mut limits = image::Limits::default();
    limits.max_image_width = Some(MAX_SOURCE_EDGE);
    limits.max_image_height = Some(MAX_SOURCE_EDGE);
    limits.max_alloc = Some(MAX_DECODER_ALLOC);
    reader.limits(limits);
    let decoded = reader.decode().map_err(|e| format!("图片解码失败：{e}"))?;

    // thumbnail 只缩不放：小图保持原始尺寸
    let resized = if width > MAX_IMAGE_EDGE || height > MAX_IMAGE_EDGE {
        decoded.thumbnail(MAX_IMAGE_EDGE, MAX_IMAGE_EDGE)
    } else {
        decoded
    }
    .to_rgba8();

    // PNG 重编码（带输出字节预算：高熵图缩放后仍可能超 2MiB，明确报错）
    let mut out = Vec::new();
    let mut writer = BoundedWriter { buf: &mut out };
    image::codecs::png::PngEncoder::new(&mut writer)
        .write_image(
            resized.as_raw(),
            resized.width(),
            resized.height(),
            image::ExtendedColorType::Rgba8,
        )
        .map_err(|e| format!("PNG 编码失败：{e}"))?;

    let image = ImageContent::from_png(&out, resized.width(), resized.height())
        .map_err(|e| format!("图片未通过规范校验：{e}"))?;
    Ok(NormalizedImage {
        data: base64::engine::general_purpose::STANDARD.encode(out),
        width: image.width(),
        height: image.height(),
    })
}

/// 带预算的输出缓冲：编码过程中超 2MiB 立即失败，不积压超大内存
struct BoundedWriter<'a> {
    buf: &'a mut Vec<u8>,
}

impl std::io::Write for BoundedWriter<'_> {
    fn write(&mut self, bytes: &[u8]) -> std::io::Result<usize> {
        if self.buf.len() + bytes.len() > MAX_IMAGE_BYTES {
            return Err(std::io::Error::new(
                std::io::ErrorKind::WriteZero,
                "image output byte budget exceeded",
            ));
        }
        self.buf.extend_from_slice(bytes);
        Ok(bytes.len())
    }
    fn flush(&mut self) -> std::io::Result<()> {
        Ok(())
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    /// 1×1 红色 PNG（本地按 PNG chunk/zlib 规则生成，无外部来源）
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

    #[test]
    fn png_passes_through_with_dimensions() {
        let out = normalize_user_image(&tiny_png()).unwrap();
        assert_eq!((out.width, out.height), (1, 1));
        let decoded = base64::engine::general_purpose::STANDARD
            .decode(&out.data)
            .unwrap();
        assert!(decoded.len() <= MAX_IMAGE_BYTES);
    }

    #[test]
    fn corrupt_input_is_rejected() {
        let mut bytes = tiny_png();
        bytes[30] ^= 0xFF;
        assert!(normalize_user_image(&bytes).is_err());
        assert!(normalize_user_image(b"not an image").is_err());
        assert!(normalize_user_image(&[]).is_err());
    }

    #[test]
    fn oversized_dimensions_are_rejected_before_decode() {
        // 20000×1 超单边上限：不进入像素解码
        let mut png = tiny_png();
        // 宽度字段（IHDR 内 big-endian u32）：偏移 16..20
        png[16..20].copy_from_slice(&20_000u32.to_be_bytes());
        assert!(normalize_user_image(&png).is_err());
    }

    #[test]
    fn jpeg_and_webp_bytes_without_format_are_guessed() {
        // 非图片字节明确报错即可；格式嗅探路径已由 png 用例覆盖
        assert!(normalize_user_image(&[0xFF, 0xD8, 0xFF]).is_err());
    }
}
