use std::path::PathBuf;
use std::sync::Arc;


use agent_core::astra_generation::openai_images::ImageSink;
use agent_core::astra_generation::{
    GenerationProvider, MediaKind, MediaLocation, MediaRef,
};
use agent_core::task::AssetStore;

use super::*;

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
