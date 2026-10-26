//! P0-3 敏感数据保护：机器级 KEK 与信封加密封装。
//!
//! 职责边界：
//! - `machine_kek` 提供 32 字节机器级密钥加密密钥（KEK）：优先 OS keychain
//!   （macOS Keychain / Windows Credential Manager / Linux Secret Service），
//!   失败回退 `<fallback_dir>/.astra-kek` 文件（0600）。
//! - `encrypt_to_envelope` / `decrypt_envelope` / `looks_like_envelope`
//!   提供统一的信封格式 `{"v":1,"alg":"xchacha20poly1305","data":"<b64(nonce||ct)>"}`，
//!   供 providers.json 加密与 memory.key 密钥包装复用。
//!
//! 边界说明：KEK 是机器级而非账户级，防的是"敏感数据明文落盘"，
//! 不宣称能抵御同一 OS 用户下的恶意代码；per-account DEK（账户口令派生）
//! 留给 P2 阶段。解密失败一律返回 None/Err，绝不静默降级为明文写入。

use chacha20poly1305::aead::{Aead, KeyInit};
use chacha20poly1305::{XChaCha20Poly1305, XNonce};
use std::path::Path;
use std::sync::OnceLock;

const ENVELOPE_VERSION: u32 = 1;
const ENVELOPE_ALG: &str = "xchacha20poly1305";
const KEK_SERVICE: &str = "com.astra.app";
const KEK_USER: &str = "astra-machine-kek";
const KEK_FILE: &str = ".astra-kek";

/// 进程内 KEK 缓存：keychain 访问在 macOS 上可能弹授权框，
/// 必须每进程最多访问一次（冷切换架构下一进程只服务一个账户，
/// KEK 是机器级而非账户级，缓存不跨进程、不破坏隔离）。
static KEK_CACHE: OnceLock<[u8; 32]> = OnceLock::new();

fn b64_encode(data: &[u8]) -> String {
    use base64::Engine;
    base64::engine::general_purpose::STANDARD.encode(data)
}

fn b64_decode(data: &str) -> Option<Vec<u8>> {
    use base64::Engine;
    base64::engine::general_purpose::STANDARD.decode(data).ok()
}

/// 32 字节随机数：两个 UUIDv4 拼接（uuid crate 无 getrandom 特性也可用，
/// 与 memory/host.rs 生成主密钥的既有做法一致）。
fn random_bytes_32() -> [u8; 32] {
    let mut b = [0u8; 32];
    b[..16].copy_from_slice(uuid::Uuid::new_v4().as_bytes());
    b[16..].copy_from_slice(uuid::Uuid::new_v4().as_bytes());
    b
}

/// XChaCha20-Poly1305 的 24 字节随机 nonce。
fn random_nonce_24() -> [u8; 24] {
    let a = *uuid::Uuid::new_v4().as_bytes();
    let b = *uuid::Uuid::new_v4().as_bytes();
    let mut n = [0u8; 24];
    n[..16].copy_from_slice(&a);
    n[16..].copy_from_slice(&b[..8]);
    n
}

/// 取机器级 KEK（进程内缓存，最多触碰一次 keychain）：优先 OS keychain；
/// keychain 不可用时回退 `<fallback_dir>/.astra-kek` 文件（0600）。
/// 回退文件已存在但长度异常时拒绝静默轮换（旧数据会全部不可读）。
pub fn machine_kek(fallback_dir: &Path) -> Result<[u8; 32], String> {
    if let Some(kek) = KEK_CACHE.get() {
        return Ok(*kek);
    }
    let kek = machine_kek_uncached(fallback_dir)?;
    let _ = KEK_CACHE.set(kek);
    Ok(kek)
}

fn machine_kek_uncached(fallback_dir: &Path) -> Result<[u8; 32], String> {
    match try_kek_from_keychain() {
        Ok(kek) => Ok(kek),
        Err(e) => {
            app_log!(
                warn,
                "secret",
                "keychain unavailable, fallback to file: {}",
                e
            );
            kek_from_file(fallback_dir)
        }
    }
}

fn try_kek_from_keychain() -> Result<[u8; 32], String> {
    let entry =
        keyring::Entry::new(KEK_SERVICE, KEK_USER).map_err(|e| format!("keyring entry: {e}"))?;
    match entry.get_password() {
        Ok(b64) => {
            let bytes = b64_decode(&b64).ok_or("keychain kek is not valid base64")?;
            if bytes.len() != 32 {
                return Err(format!(
                    "keychain kek has {} bytes, expected 32",
                    bytes.len()
                ));
            }
            let mut kek = [0u8; 32];
            kek.copy_from_slice(&bytes);
            Ok(kek)
        }
        Err(keyring::Error::NoEntry) => {
            let kek = random_bytes_32();
            entry
                .set_password(&b64_encode(&kek))
                .map_err(|e| format!("keyring set: {e}"))?;
            Ok(kek)
        }
        Err(e) => Err(format!("keyring get: {e}")),
    }
}

fn kek_from_file(fallback_dir: &Path) -> Result<[u8; 32], String> {
    let path = fallback_dir.join(KEK_FILE);
    if let Ok(bytes) = std::fs::read(&path) {
        if bytes.len() == 32 {
            let mut kek = [0u8; 32];
            kek.copy_from_slice(&bytes);
            return Ok(kek);
        }
        return Err(format!(
            "{} exists but has {} bytes, expected 32; refusing to silently rotate the KEK",
            path.display(),
            bytes.len()
        ));
    }
    std::fs::create_dir_all(fallback_dir).map_err(|e| format!("create kek dir: {e}"))?;
    let kek = random_bytes_32();
    std::fs::write(&path, kek).map_err(|e| format!("write {}: {e}", path.display()))?;
    #[cfg(unix)]
    {
        use std::os::unix::fs::PermissionsExt;
        let _ = std::fs::set_permissions(&path, std::fs::Permissions::from_mode(0o600));
    }
    app_log!(
        warn,
        "secret",
        "generated fallback KEK at {}",
        path.display()
    );
    Ok(kek)
}

/// 判断内容是否为信封格式（供读取方区分密文与 legacy 明文）。
pub fn looks_like_envelope(content: &str) -> bool {
    serde_json::from_str::<serde_json::Value>(content)
        .ok()
        .and_then(|v| {
            let alg = v.get("alg").and_then(|a| a.as_str());
            let data = v.get("data").and_then(|d| d.as_str());
            match (v.get("v").and_then(|n| n.as_u64()), alg, data) {
                (Some(1), Some(a), Some(d)) if a == ENVELOPE_ALG && !d.is_empty() => Some(true),
                _ => None,
            }
        })
        .unwrap_or(false)
}

/// 加密为信封 JSON：`{"v":1,"alg":"xchacha20poly1305","data":"<b64(nonce||ct)>"}`。
pub fn encrypt_to_envelope(kek: &[u8; 32], plaintext: &[u8]) -> Result<String, String> {
    let cipher = XChaCha20Poly1305::new(kek.into());
    let nonce_bytes = random_nonce_24();
    let ct = cipher
        .encrypt(XNonce::from_slice(&nonce_bytes), plaintext)
        .map_err(|e| format!("encrypt: {e}"))?;

    let mut packed = Vec::with_capacity(nonce_bytes.len() + ct.len());
    packed.extend_from_slice(&nonce_bytes);
    packed.extend_from_slice(&ct);

    let envelope = serde_json::json!({
        "v": ENVELOPE_VERSION,
        "alg": ENVELOPE_ALG,
        "data": b64_encode(&packed),
    });
    serde_json::to_string(&envelope).map_err(|e| format!("serialize envelope: {e}"))
}

/// 解密信封 JSON；任何一步失败都返回 None（调用方自行决定降级策略）。
pub fn decrypt_envelope(kek: &[u8; 32], envelope: &str) -> Option<Vec<u8>> {
    if !looks_like_envelope(envelope) {
        return None;
    }
    let v: serde_json::Value = serde_json::from_str(envelope).ok()?;
    let packed = b64_decode(v.get("data")?.as_str()?)?;
    if packed.len() < 25 {
        return None; // 24B nonce + 至少 1B 密文
    }
    let (nonce_bytes, ct) = packed.split_at(24);
    let cipher = XChaCha20Poly1305::new(kek.into());
    cipher.decrypt(XNonce::from_slice(nonce_bytes), ct).ok()
}

#[cfg(test)]
mod tests {
    use super::*;

    fn test_kek() -> [u8; 32] {
        random_bytes_32()
    }

    #[test]
    fn envelope_roundtrip() {
        let kek = test_kek();
        let plain = b"api-key-sk-123 hello".to_vec();
        let envelope = encrypt_to_envelope(&kek, &plain).unwrap();
        assert!(looks_like_envelope(&envelope));
        assert!(!envelope.contains("api-key-sk-123"));
        assert_eq!(decrypt_envelope(&kek, &envelope).unwrap(), plain);
    }

    #[test]
    fn envelope_rejects_wrong_kek_and_tampered_data() {
        let kek = test_kek();
        let envelope = encrypt_to_envelope(&kek, b"secret").unwrap();

        assert!(decrypt_envelope(&test_kek(), &envelope).is_none());

        let mut v: serde_json::Value = serde_json::from_str(&envelope).unwrap();
        v["data"] = serde_json::Value::String("AAAA".into());
        assert!(decrypt_envelope(&kek, &v.to_string()).is_none());
    }

    #[test]
    fn envelope_detected_but_plaintext_json_not() {
        assert!(looks_like_envelope(
            r#"{"v":1,"alg":"xchacha20poly1305","data":"AAAA"}"#
        ));
        assert!(!looks_like_envelope(r#"[{"id":"p1"}]"#));
        assert!(!looks_like_envelope("[]"));
    }

    #[test]
    fn kek_file_roundtrip_and_corrupt_rejected() {
        let dir = tempfile::tempdir().unwrap();
        let kek1 = kek_from_file(dir.path()).unwrap();
        let kek2 = kek_from_file(dir.path()).unwrap();
        assert_eq!(kek1, kek2, "fallback KEK must be stable across calls");

        std::fs::write(dir.path().join(KEK_FILE), b"short").unwrap();
        assert!(kek_from_file(dir.path()).is_err());
    }
}
