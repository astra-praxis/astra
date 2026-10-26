//! 账户密码认证（Argon2id）+ 数据加密信封。
//!
//! 每个账户（或 legacy 数据目录）持有随机 32 字节 KEK，保护
//! providers.json / memory.key 等敏感数据；KEK 由**账户登录密码**经
//! Argon2id 派生密钥包装后存 `<data_dir>/.astra-auth.json`
//! （salt + wrapped_kek + verifier）。登录 = 选用户 → 输密码 → 验证 → 进应用。
//!
//! KEK 解析优先级（进程内缓存，每进程至多读一次盘）：
//! 1. 进程内缓存（登录/解锁后写入）
//! 2. `.astra-auth.json` 存在 → 锁定态，未验证密码前拒绝一切解密/加密
//! 3. 本地文件 `<dir>/.astra-kek`（0600，legacy/dev 兜底，零弹窗）
//!
//! 冷切换登录态交接：验证成功后 KEK 经一次性 token 文件（APPDATA 根，
//! 0600，即读即删）交给 `--account` 启动的新进程，避免在 argv/环境变量里
//! 传秘密，也免去新进程二次输密码。
//!
//! 边界说明：**不使用 OS keychain**（会反复弹授权框，用户明确要求弃用）；
//! 密码仅本地验证，不做云同步；防"敏感数据明文落盘"，不宣称能抵御同一
//! OS 用户下的恶意代码。解密失败一律返回 None/Err，绝不静默降级为明文写入。

use chacha20poly1305::{
    aead::{generic_array::typenum::U24, Aead, KeyInit},
    XChaCha20Poly1305,
};
use serde::{Deserialize, Serialize};
use std::path::Path;
use std::sync::OnceLock;

/// XChaCha20-Poly1305 nonce 长度
pub(crate) type XNonceSize = U24;
use chacha20poly1305::aead::generic_array::GenericArray;
/// 便捷别名
pub(crate) type XNonce = GenericArray<u8, XNonceSize>;

const ENVELOPE_VERSION: u32 = 1;
const ENVELOPE_ALG: &str = "xchacha20poly1305";
/// legacy/dev 兜底密钥文件（有登录密码的账户目录里不存在此文件）
const KEK_FILE: &str = ".astra-kek";
/// 登录凭据文件（salt + 被密码派生密钥包装的 KEK + 验证信封）
const AUTH_FILE: &str = ".astra-auth.json";
/// 冷切换登录态交接 token（APPDATA 根，0600，即读即删）
const TOKEN_FILE: &str = ".astra-auth-token";

/// 进程内 KEK 缓存：登录/解锁后写入，之后所有加解密走缓存，
/// 不再触碰磁盘上的密钥材料。
static KEK_CACHE: OnceLock<[u8; 32]> = OnceLock::new();

/// 登录/解锁成功后（或消费交接 token 后）把 KEK 写入进程内缓存。
pub fn cache_kek(kek: [u8; 32]) {
    let _ = KEK_CACHE.set(kek);
}

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

// ===== KEK 解析 =====

/// 取当前数据目录的 KEK（进程内缓存优先）。缓存未命中且目录处于锁定态
/// （有登录凭据、未验证密码）→ Err，调用方（providers/memory）降级等待解锁。
pub fn machine_kek(dir: &Path) -> Result<[u8; 32], String> {
    if let Some(kek) = KEK_CACHE.get() {
        return Ok(*kek);
    }
    machine_kek_uncached(dir)
}

fn machine_kek_uncached(dir: &Path) -> Result<[u8; 32], String> {
    // 锁定态：有登录凭据但本进程未验证密码，拒绝解密/加密
    if auth_file(dir).exists() {
        return Err("账户处于锁定状态（等待登录）".into());
    }
    // legacy/dev 兜底：本地文件密钥（0600，零弹窗）
    let kek_path = dir.join(KEK_FILE);
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
    let kek = random_bytes_32();
    write_kek_file(&kek_path, &kek)?;
    app_log!(
        info,
        "secret",
        "generated legacy KEK at {}",
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

// ===== 登录凭据（Argon2id 派生，本地验证，不跨设备同步） =====

/// 验证明文：登录时用它校验密码是否正确（无需先解开 KEK）
const AUTH_VERIFY_PLAINTEXT: &[u8] = b"astra-auth-verify1";
const PASSWORD_MIN_LEN: usize = 6;
const PASSWORD_MAX_LEN: usize = 128;

#[derive(Debug, Serialize, Deserialize)]
struct AuthFile {
    v: u32,
    alg: String,
    /// b64(16B salt)
    salt: String,
    /// KEK 的信封加密（用密码派生密钥包装）
    wrapped_kek: String,
    /// 验证信封（固定明文，用于快速校验密码对错）
    verifier: String,
}

fn auth_file(dir: &Path) -> std::path::PathBuf {
    dir.join(AUTH_FILE)
}

/// 数据目录是否已设置登录密码
pub fn has_auth(dir: &Path) -> bool {
    auth_file(dir).exists()
}

/// 是否处于锁定态：设了登录密码且本进程尚未验证
pub fn is_locked(dir: &Path) -> bool {
    has_auth(dir) && KEK_CACHE.get().is_none()
}

/// Argon2id 派生 32 字节密钥（Params::default：m=19MiB, t=2, p=1）。
fn derive_key_from_password(password: &str, salt: &[u8]) -> Result<[u8; 32], String> {
    use argon2::{Algorithm, Argon2, Params, Version};
    let a2 = Argon2::new(Algorithm::Argon2id, Version::V0x13, Params::default());
    let mut out = [0u8; 32];
    a2.hash_password_into(password.as_bytes(), salt, &mut out)
        .map_err(|e| format!("argon2 derive: {e}"))?;
    Ok(out)
}

pub fn validate_password(password: &str) -> Result<(), String> {
    let n = password.chars().count();
    if !(PASSWORD_MIN_LEN..=PASSWORD_MAX_LEN).contains(&n) {
        return Err(format!(
            "密码长度需为 {PASSWORD_MIN_LEN}-{PASSWORD_MAX_LEN} 个字符"
        ));
    }
    Ok(())
}

fn write_auth_file(dir: &Path, kek: &[u8; 32], password: &str) -> Result<(), String> {
    let salt = *uuid::Uuid::new_v4().as_bytes();
    let key_p = derive_key_from_password(password, &salt)?;
    let file = AuthFile {
        v: 1,
        alg: "argon2id".into(),
        salt: b64_encode(&salt),
        wrapped_kek: encrypt_to_envelope(&key_p, kek)?,
        verifier: encrypt_to_envelope(&key_p, AUTH_VERIFY_PLAINTEXT)?,
    };
    let json =
        serde_json::to_string_pretty(&file).map_err(|e| format!("serialize auth file: {e}"))?;
    std::fs::write(auth_file(dir), json).map_err(|e| format!("write {AUTH_FILE}: {e}"))?;
    Ok(())
}

/// 为账户初始化登录密码：优先承接目录里已存在的 legacy 文件密钥
/// （迁移场景：存量密文继续可解），否则生成新 KEK；然后按密码包装落盘。
/// 返回被保护的 KEK（供冷切换 token 交接）。
pub fn auth_init(dir: &Path, password: &str) -> Result<[u8; 32], String> {
    validate_password(password)?;
    let kek_path = dir.join(KEK_FILE);
    let kek: [u8; 32] = match std::fs::read(&kek_path) {
        Ok(bytes) if bytes.len() == 32 => {
            let mut k = [0u8; 32];
            k.copy_from_slice(&bytes);
            // 明文 KEK 使命完成，删除（密码即唯一登录凭证）
            let _ = std::fs::remove_file(&kek_path);
            k
        }
        Ok(_) => {
            return Err(format!(
                "{} exists but has unexpected length; refusing to silently rotate the KEK",
                kek_path.display()
            ));
        }
        Err(_) => random_bytes_32(),
    };
    write_auth_file(dir, &kek, password)?;
    Ok(kek)
}

/// 验证密码并返回被包装的 KEK（纯验证，不写进程缓存）。
pub fn auth_verify(dir: &Path, password: &str) -> Result<[u8; 32], String> {
    let raw =
        std::fs::read_to_string(auth_file(dir)).map_err(|_| "此账户未设置登录密码".to_string())?;
    let file: AuthFile =
        serde_json::from_str(&raw).map_err(|e| format!("auth file corrupt: {e}"))?;
    let salt = b64_decode(&file.salt).ok_or("auth salt corrupt")?;
    let key_p = derive_key_from_password(password, &salt)?;
    let verified = decrypt_envelope(&key_p, &file.verifier).ok_or("密码错误")?;
    if verified != AUTH_VERIFY_PLAINTEXT {
        return Err("密码错误".into());
    }
    let wrapped = decrypt_envelope(&key_p, &file.wrapped_kek)
        .ok_or("auth file corrupt: cannot unwrap KEK")?;
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

/// 登录/解锁：验证密码，KEK 写入进程内缓存。
pub fn account_unlock(dir: &Path, password: &str) -> Result<(), String> {
    let kek = auth_verify(dir, password)?;
    cache_kek(kek);
    Ok(())
}

/// 更换登录密码：验证旧密码后按新密码重新包装同一 KEK（数据不动）。
pub fn password_change(dir: &Path, old: &str, new: &str) -> Result<(), String> {
    validate_password(new)?;
    let kek = auth_verify(dir, old).map_err(|e| {
        if e == "密码错误" {
            "当前密码错误".into()
        } else {
            e
        }
    })?;
    write_auth_file(dir, &kek, new)?;
    Ok(())
}

// ===== 冷切换登录态交接 =====

#[derive(Serialize, Deserialize)]
struct AuthToken {
    account_id: String,
    /// b64(32B KEK)
    kek: String,
}

fn token_path(app_data_root: &Path) -> std::path::PathBuf {
    app_data_root.join(TOKEN_FILE)
}

/// 写一次性交接 token（0600）：验证成功后、spawn 新进程前调用。
/// 文件包含 KEK，即读即删；属本地临时暴露，与 .astra-kek 同级风险。
pub fn auth_token_write(
    app_data_root: &Path,
    account_id: &str,
    kek: &[u8; 32],
) -> Result<(), String> {
    let token = AuthToken {
        account_id: account_id.into(),
        kek: b64_encode(kek),
    };
    let json = serde_json::to_string(&token).map_err(|e| format!("serialize token: {e}"))?;
    let path = token_path(app_data_root);
    std::fs::write(&path, json).map_err(|e| format!("write {TOKEN_FILE}: {e}"))?;
    #[cfg(unix)]
    {
        use std::os::unix::fs::PermissionsExt;
        let _ = std::fs::set_permissions(&path, std::fs::Permissions::from_mode(0o600));
    }
    Ok(())
}

/// 消费交接 token（setup 阶段）：account_id 匹配则返回 KEK；
/// 无论是否匹配，读到的 token 都删除（一次性）。
pub fn auth_token_take(app_data_root: &Path, account_id: &str) -> Option<[u8; 32]> {
    let path = token_path(app_data_root);
    let raw = std::fs::read_to_string(&path).ok()?;
    let _ = std::fs::remove_file(&path);
    let token: AuthToken = serde_json::from_str(&raw).ok()?;
    if token.account_id != account_id {
        return None;
    }
    let kek = b64_decode(&token.kek)?;
    if kek.len() != 32 {
        return None;
    }
    let mut out = [0u8; 32];
    out.copy_from_slice(&kek);
    Some(out)
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
    fn locked_state_is_reported_while_auth_file_exists() {
        let dir = tempfile::tempdir().unwrap();
        auth_init(dir.path(), "hunter2!").unwrap();
        assert!(has_auth(dir.path()));
        // 锁定判断不含缓存维度（缓存是进程态），这里验证 auth 文件语义
        assert!(machine_kek_uncached(dir.path())
            .unwrap_err()
            .contains("锁定"));
    }

    #[test]
    fn auth_init_verifies_and_changes_password() {
        let dir = tempfile::tempdir().unwrap();
        let kek = auth_init(dir.path(), "hunter2!").unwrap();

        assert!(auth_verify(dir.path(), "wrong-password").is_err());
        assert_eq!(auth_verify(dir.path(), "hunter2!").unwrap(), kek);

        // 更换密码：同一 KEK 重新包装，数据不动
        password_change(dir.path(), "wrong", "new-pass!").unwrap_err();
        password_change(dir.path(), "hunter2!", "new-pass!").unwrap();
        assert!(auth_verify(dir.path(), "hunter2!").is_err());
        assert_eq!(auth_verify(dir.path(), "new-pass!").unwrap(), kek);
    }

    #[test]
    fn auth_init_adopts_legacy_file_kek() {
        let dir = tempfile::tempdir().unwrap();
        let legacy = random_bytes_32();
        write_kek_file(&dir.path().join(KEK_FILE), &legacy).unwrap();

        let kek = auth_init(dir.path(), "hunter2!").unwrap();
        assert_eq!(kek, legacy, "迁移场景必须承接存量 KEK，否则旧密文不可解");
        assert!(!dir.path().join(KEK_FILE).exists(), "明文 KEK 应被删除");
        assert_eq!(auth_verify(dir.path(), "hunter2!").unwrap(), legacy);
    }

    #[test]
    fn password_length_validated() {
        let dir = tempfile::tempdir().unwrap();
        assert!(auth_init(dir.path(), "12345").is_err()); // < 6
        assert!(auth_init(dir.path(), &"x".repeat(129)).is_err()); // > 128
        assert!(!has_auth(dir.path()));
    }

    #[test]
    fn auth_token_write_take_roundtrip() {
        let root = tempfile::tempdir().unwrap();
        let kek = test_kek();
        auth_token_write(root.path(), "acc1", &kek).unwrap();

        // id 不匹配：token 被消费但不返回 KEK
        auth_token_write(root.path(), "acc1", &kek).unwrap();
        assert!(auth_token_take(root.path(), "acc2").is_none());
        assert!(!token_path(root.path()).exists(), "token 必须即读即删");

        auth_token_write(root.path(), "acc1", &kek).unwrap();
        assert_eq!(auth_token_take(root.path(), "acc1").unwrap(), kek);
        assert!(!token_path(root.path()).exists());
        assert!(auth_token_take(root.path(), "acc1").is_none());
    }
}
