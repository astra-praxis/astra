//! P0-3 敏感数据保护：账户密钥（KEK）与信封加密封装。
//!
//! 职责边界：
//! - KEK 解析优先级（进程内缓存 → 密码文件 → 本地文件 → 一次性 keychain 迁移）：
//!   1. 进程内缓存（解锁后或首次生成时写入，每进程最多触碰一次磁盘/keychain）
//!   2. 访问密码（`.astra-password.json` 存在时锁定，需 `account_unlock` 解锁；
//!      Argon2id 派生，默认不启用——用户可选设置，无需记住即可用）
//!   3. 本地文件 `<dir>/.astra-kek`（0600，随机 32 字节，**默认方案，零弹窗**）
//!   4. 遗产迁移：早期版本曾把 KEK 存 OS keychain（每次访问都弹授权框，已弃
//!      用）；首次启动时一次性读出迁入本地文件并写 marker，之后不再触碰
//!      keychain。迁移被用户取消时不写 marker，下次启动重试。
//! - `encrypt_to_envelope` / `decrypt_envelope` / `looks_like_envelope`
//!   提供统一的信封格式 `{"v":1,"alg":"xchacha20poly1305","data":"<b64(nonce||ct)>"}`，
//!   供 providers.json 加密、memory.key 密钥包装与密码文件复用。
//!
//! 边界说明：KEK 防的是"敏感数据明文落盘"，不宣称能抵御同一 OS 用户下的
//! 恶意代码；密码为本地验证，不做跨设备/云同步。

use chacha20poly1305::aead::{Aead, KeyInit};
use chacha20poly1305::{XChaCha20Poly1305, XNonce};
use serde::{Deserialize, Serialize};
use std::path::Path;
use std::sync::OnceLock;

const ENVELOPE_VERSION: u32 = 1;
const ENVELOPE_ALG: &str = "xchacha20poly1305";
const KEK_SERVICE: &str = "com.astra.app";
const KEK_USER: &str = "astra-machine-kek";
const KEK_FILE: &str = ".astra-kek";
/// keychain 遗产迁移 marker：写过后不再触碰 keychain（不再弹授权框）
const KEK_IMPORT_MARKER: &str = ".astra-kek-imported";

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

/// 取账户 KEK（进程内缓存，最多触碰一次磁盘/keychain）。解析顺序见模块注释。
/// 失败不降级、不静默换新密钥——旧密文可能全部不可读。
pub fn machine_kek(fallback_dir: &Path) -> Result<[u8; 32], String> {
    if let Some(kek) = KEK_CACHE.get() {
        return Ok(*kek);
    }
    let kek = machine_kek_uncached(fallback_dir)?;
    let _ = KEK_CACHE.set(kek);
    Ok(kek)
}

fn machine_kek_uncached(fallback_dir: &Path) -> Result<[u8; 32], String> {
    // 1. 访问密码已设置且本进程未解锁：锁定，拒绝一切解密/加密
    if password_file(fallback_dir).exists() {
        return Err("账户已启用访问密码，处于锁定状态（等待解锁）".into());
    }
    // 2. 本地文件密钥（默认方案）
    let kek_path = fallback_dir.join(KEK_FILE);
    if let Ok(bytes) = std::fs::read(&kek_path) {
        if bytes.len() == 32 {
            let mut kek = [0u8; 32];
            kek.copy_from_slice(&bytes);
            return Ok(kek);
        }
        return Err(format!(
            "{} exists but has {} bytes, expected 32; refusing to silently rotate the KEK",
            kek_path.display(),
            bytes.len()
        ));
    }
    // 3. 遗产迁移：旧版本曾把 KEK 存 keychain（会反复弹授权框，已弃用）。
    //    仅首次（marker 不存在）尝试一次性读出迁入本地文件；取消则下次重试。
    let marker = fallback_dir.join(KEK_IMPORT_MARKER);
    if !marker.exists() {
        match keyring::Entry::new(KEK_SERVICE, KEK_USER) {
            Ok(entry) => match entry.get_password() {
                Ok(b64) => {
                    if let Some(bytes) = b64_decode(&b64) {
                        if bytes.len() == 32 {
                            let mut legacy = [0u8; 32];
                            legacy.copy_from_slice(&bytes);
                            write_kek_file(&kek_path, &legacy)?;
                            let _ = std::fs::write(&marker, "1");
                            app_log!(
                                info,
                                "secret",
                                "legacy keychain KEK imported to {}",
                                kek_path.display()
                            );
                            return Ok(legacy);
                        }
                    }
                    app_log!(
                        warn,
                        "secret",
                        "legacy keychain KEK unreadable, generating new"
                    );
                    let _ = std::fs::write(&marker, "1");
                }
                Err(keyring::Error::NoEntry) => {
                    // 本机从未用过 keychain：写 marker，以后不再尝试
                    let _ = std::fs::write(&marker, "1");
                }
                Err(e) => {
                    // 可能是用户取消了授权框：不写 marker，下次启动重试
                    return Err(format!(
                        "legacy keychain migration interrupted (denied?): {e}"
                    ));
                }
            },
            Err(e) => {
                let _ = std::fs::write(&marker, "1");
                app_log!(warn, "secret", "keyring unavailable: {e}");
            }
        }
    }
    // 4. 生成新文件密钥（0600）
    let kek = random_bytes_32();
    write_kek_file(&kek_path, &kek)?;
    app_log!(
        info,
        "secret",
        "generated account KEK at {}",
        kek_path.display()
    );
    Ok(kek)
}

fn write_kek_file(path: &Path, kek: &[u8; 32]) -> Result<(), String> {
    if let Some(parent) = path.parent() {
        std::fs::create_dir_all(parent).map_err(|e| format!("create kek dir: {e}"))?;
    }
    std::fs::write(path, kek).map_err(|e| format!("write {}: {e}", path.display()))?;
    #[cfg(unix)]
    {
        use std::os::unix::fs::PermissionsExt;
        let _ = std::fs::set_permissions(path, std::fs::Permissions::from_mode(0o600));
    }
    Ok(())
}

// ===== 可选访问密码（Argon2id 派生，本地验证，不跨设备同步） =====

const PASSWORD_FILE: &str = ".astra-password.json";
const PASSWORD_MIN_LEN: usize = 6;
const PASSWORD_MAX_LEN: usize = 128;
/// 验证明文：解锁时用它校验密码是否正确（不需要真的解开 KEK 才知道）
const PASSWORD_VERIFY_PLAINTEXT: &[u8] = b"astra-kek-verify1";

#[derive(Debug, Serialize, Deserialize)]
struct PasswordKekFile {
    v: u32,
    alg: String,
    /// b64(16B salt)
    salt: String,
    /// KEK 的信封加密（用密码派生密钥包装）
    wrapped_kek: String,
    /// 验证信封（固定明文，用于快速校验密码对错）
    verifier: String,
}

fn password_file(dir: &Path) -> std::path::PathBuf {
    dir.join(PASSWORD_FILE)
}

/// 账户是否设置了访问密码
pub fn password_is_set(dir: &Path) -> bool {
    password_file(dir).exists()
}

/// 是否处于锁定态：设了密码且本进程尚未解锁。
/// 解锁后 KEK 进程内缓存，无需重复输入。
pub fn is_locked(dir: &Path) -> bool {
    password_is_set(dir) && KEK_CACHE.get().is_none()
}

/// Argon2id 派生 32 字节密钥（Params::default：m=19MiB, t=2, p=1）。
fn derive_kek_from_password(password: &str, salt: &[u8]) -> Result<[u8; 32], String> {
    use argon2::{Algorithm, Argon2, Params, Version};
    let a2 = Argon2::new(Algorithm::Argon2id, Version::V0x13, Params::default());
    let mut out = [0u8; 32];
    a2.hash_password_into(password.as_bytes(), salt, &mut out)
        .map_err(|e| format!("argon2 derive: {e}"))?;
    Ok(out)
}

fn validate_password(password: &str) -> Result<(), String> {
    let n = password.chars().count();
    if n < PASSWORD_MIN_LEN || n > PASSWORD_MAX_LEN {
        return Err(format!(
            "密码长度需为 {PASSWORD_MIN_LEN}-{PASSWORD_MAX_LEN} 个字符"
        ));
    }
    Ok(())
}

/// 设置/更换访问密码：用密码派生密钥包装当前 KEK，写密码文件并删除
/// 明文 KEK 文件。要求当前处于可用状态（未锁定），否则会拿不到 KEK。
pub fn password_set(dir: &Path, password: &str) -> Result<(), String> {
    let kek = machine_kek(dir)?; // 锁定态会在此报错，不会静默重包装
    password_set_with_kek(dir, password, &kek)
}

/// 指定 KEK 的版本（单测用，避免触碰进程级缓存）。
pub(crate) fn password_set_with_kek(
    dir: &Path,
    password: &str,
    kek: &[u8; 32],
) -> Result<(), String> {
    validate_password(password)?;
    let salt = *uuid::Uuid::new_v4().as_bytes();
    let kek_p = derive_kek_from_password(password, &salt)?;
    let file = PasswordKekFile {
        v: 1,
        alg: "argon2id".into(),
        salt: b64_encode(&salt),
        wrapped_kek: encrypt_to_envelope(&kek_p, kek)?,
        verifier: encrypt_to_envelope(&kek_p, PASSWORD_VERIFY_PLAINTEXT)?,
    };
    let json =
        serde_json::to_string_pretty(&file).map_err(|e| format!("serialize password file: {e}"))?;
    std::fs::write(password_file(dir), json).map_err(|e| format!("write {PASSWORD_FILE}: {e}"))?;
    // 明文 KEK 文件使命完成，删除（密码即唯一解锁凭证）
    let _ = std::fs::remove_file(dir.join(KEK_FILE));
    Ok(())
}

/// 验证密码并返回被包装的 KEK（不写缓存；公开入口在此基础上缓存）。
fn unlock_inner(dir: &Path, password: &str) -> Result<[u8; 32], String> {
    let raw = std::fs::read_to_string(password_file(dir))
        .map_err(|_| "此账户未设置访问密码".to_string())?;
    let file: PasswordKekFile =
        serde_json::from_str(&raw).map_err(|e| format!("password file corrupt: {e}"))?;
    let salt = b64_decode(&file.salt).ok_or("password salt corrupt")?;
    let kek_p = derive_kek_from_password(password, &salt)?;
    let verify = decrypt_envelope(&kek_p, &file.verifier).ok_or_else(|| "密码错误".to_string())?;
    if verify != PASSWORD_VERIFY_PLAINTEXT {
        return Err("密码错误".into());
    }
    let wrapped = decrypt_envelope(&kek_p, &file.wrapped_kek)
        .ok_or("password file corrupt: cannot unwrap KEK")?;
    if wrapped.len() != 32 {
        return Err(format!(
            "unwrapped KEK has {} bytes, expected 32",
            wrapped.len()
        ));
    }
    let mut kek = [0u8; 32];
    kek.copy_from_slice(&wrapped);
    Ok(kek)
}

/// 解锁：验证密码，KEK 写入进程内缓存。
pub fn account_unlock(dir: &Path, password: &str) -> Result<(), String> {
    let kek = unlock_inner(dir, password)?;
    let _ = KEK_CACHE.set(kek);
    Ok(())
}

/// 移除访问密码：验证后删除密码文件，并把当前 KEK 写回明文文件
/// （回到默认文件密钥方案）。要求本进程已解锁。
pub fn password_remove(dir: &Path, password: &str) -> Result<(), String> {
    // 必须已解锁（缓存里有 KEK），否则移除后无法继续解密数据
    let kek = KEK_CACHE
        .get()
        .ok_or_else(|| "账户处于锁定状态，请先解锁".to_string())?;
    password_remove_with(dir, password, kek)
}

/// 指定 KEK 的版本（单测用）。
pub(crate) fn password_remove_with(
    dir: &Path,
    password: &str,
    kek: &[u8; 32],
) -> Result<(), String> {
    unlock_inner(dir, password)?; // 校验密码正确才允许移除
    std::fs::remove_file(password_file(dir)).map_err(|e| format!("remove {PASSWORD_FILE}: {e}"))?;
    write_kek_file(&dir.join(KEK_FILE), kek)?;
    Ok(())
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
        let path = dir.path().join(KEK_FILE);
        let kek = random_bytes_32();
        write_kek_file(&path, &kek).unwrap();
        let reread = std::fs::read(&path).unwrap();
        assert_eq!(reread, kek);

        std::fs::write(&path, b"short").unwrap();
        let bad = machine_kek_uncached(dir.path()).unwrap_err();
        assert!(bad.contains("refusing to silently rotate"), "{bad}");
    }

    #[test]
    fn password_set_unlock_remove_roundtrip() {
        let dir = tempfile::tempdir().unwrap();
        let kek = random_bytes_32();

        // 设密码：密码文件生成，明文 KEK 被删除
        password_set_with_kek(dir.path(), "hunter2!", &kek).unwrap();
        assert!(password_is_set(dir.path()));
        assert!(!dir.path().join(KEK_FILE).exists());
        // 解锁验证正确/错误密码（unlock_inner 纯验证，不写进程级缓存）
        assert!(unlock_inner(dir.path(), "wrong-password").is_err());
        assert_eq!(unlock_inner(dir.path(), "hunter2!").unwrap(), kek);

        // 移除密码：验证后 KEK 写回明文文件
        password_remove_with(dir.path(), "hunter2!", &kek).unwrap();
        assert!(!password_is_set(dir.path()));
        assert_eq!(std::fs::read(dir.path().join(KEK_FILE)).unwrap(), kek);
    }

    #[test]
    fn password_change_rewraps_kek() {
        let dir = tempfile::tempdir().unwrap();
        let kek = random_bytes_32();
        password_set_with_kek(dir.path(), "first-pass", &kek).unwrap();
        password_set_with_kek(dir.path(), "second-pass!", &kek).unwrap(); // 更换 = 重新包装
        assert!(unlock_inner(dir.path(), "first-pass").is_err());
        assert_eq!(unlock_inner(dir.path(), "second-pass!").unwrap(), kek);
    }

    #[test]
    fn password_length_validated() {
        let dir = tempfile::tempdir().unwrap();
        let kek = random_bytes_32();
        assert!(password_set_with_kek(dir.path(), "12345", &kek).is_err()); // < 6
        assert!(password_set_with_kek(dir.path(), &"x".repeat(129), &kek).is_err()); // > 128
        assert!(!password_is_set(dir.path()));
    }
}
