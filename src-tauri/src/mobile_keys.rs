//! 手机遥控桥的持久设备 Key 管理。
//!
//! 与临时配对 token（内存随机串，重启失效）互补：持久 key 是「绑定这台桌面端
//! 的长期钥匙」（GitHub PAT 逻辑）——secret 只在创建时出现在二维码里，服务端
//! 只存 SHA-256 hash，丢失即撤销重建。支持失效时间与手动撤销。
//!
//! 职责边界：本模块只做 key 的存储与校验（纯函数 + 文件 IO），认证协议
//!（ticket 换发 / 限速 / scope 过滤）在 mobile_bridge.rs。
//!
//! 存储位置：账户私有 data_dir（`mobile_bridge/keys.json`）。绑定的是该账户
//! 数据目录里的会话，语义一致；账户热切换后需重新绑定。

use std::path::PathBuf;

use serde::{Deserialize, Serialize};
use serde_json::Value;
use sha2::{Digest, Sha256};

/// key 的可见范围：全部会话，或仅绑定的单个会话
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(tag = "kind", rename_all = "snake_case")]
pub enum KeyScope {
    All,
    Session { session_id: String },
}

impl KeyScope {
    /// 供协议层使用的标签（hello 的 scope 字段：'all' | 'one'）
    pub fn tag(&self) -> &'static str {
        match self {
            KeyScope::All => "all",
            KeyScope::Session { .. } => "one",
        }
    }

    /// scoped 时返回绑定的会话 id
    pub fn session_id(&self) -> Option<&str> {
        match self {
            KeyScope::All => None,
            KeyScope::Session { session_id } => Some(session_id),
        }
    }
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct BridgeKey {
    pub key_id: String,
    /// 设备名（创建时指定，列表展示用）
    pub name: String,
    /// SHA-256(secret) hex；secret 绝不落盘
    pub key_hash: String,
    pub scope: KeyScope,
    /// 创建时间（ms）
    pub created_at: u64,
    /// 过期时间（ms）；None = 永久
    pub expires_at: Option<u64>,
    pub revoked_at: Option<u64>,
    pub last_used_at: Option<u64>,
    pub last_ip: Option<String>,
}

impl BridgeKey {
    /// 当前是否可用（未撤销、未过期）
    pub fn usable(&self, now_ms: u64) -> bool {
        self.revoked_at.is_none() && self.expires_at.map(|e| e > now_ms).unwrap_or(true)
    }

    /// 列表视图：剥掉 hash（列表界面不需要，少一处敏感数据暴露）
    pub fn to_view(&self) -> Value {
        serde_json::json!({
            "id": self.key_id,
            "name": self.name,
            "scope": self.scope,
            "scope_tag": self.scope.tag(),
            "created_at": self.created_at,
            "expires_at": self.expires_at,
            "revoked_at": self.revoked_at,
            "last_used_at": self.last_used_at,
            "last_ip": self.last_ip,
        })
    }
}

// ===== 纯函数（单测友好，不依赖 AppHandle） =====

/// 生成新 secret：`mbk_` + 64 位 hex（两个 uuid 拼接）。只出现在创建时的返回值与二维码里
pub fn new_secret() -> String {
    format!(
        "mbk_{}{}",
        uuid::Uuid::new_v4().simple(),
        uuid::Uuid::new_v4().simple()
    )
}

pub fn hash_secret(secret: &str) -> String {
    let mut hasher = Sha256::new();
    hasher.update(secret.as_bytes());
    let digest = hasher.finalize();
    digest.iter().map(|b| format!("{b:02x}")).collect()
}

pub fn now_ms() -> u64 {
    std::time::SystemTime::now()
        .duration_since(std::time::UNIX_EPOCH)
        .map(|d| d.as_millis() as u64)
        .unwrap_or(0)
}

/// 按指数退避计算认证失败的封禁时长（秒）：第 n 次连续失败封 2^n 秒，封顶 300s
pub fn backoff_secs(fails: u32) -> u64 {
    if fails == 0 {
        return 0;
    }
    let exp = 2u64.saturating_pow(fails.min(30));
    exp.min(300)
}

/// 校验 secret 在 key 列表中的合法性：找到对应 hash 且未撤销未过期。
/// 返回命中 key 的克隆（供调用方更新 last_used / 取 scope）
pub fn validate_in_list(keys: &[BridgeKey], secret: &str, now_ms: u64) -> Option<BridgeKey> {
    let hash = hash_secret(secret);
    keys.iter()
        .find(|k| k.key_hash == hash && k.usable(now_ms))
        .cloned()
}

/// 创建一条 key（不落盘）：返回 (key, secret)
pub fn create_in_list(
    keys: &mut Vec<BridgeKey>,
    name: &str,
    scope: KeyScope,
    expires_at: Option<u64>,
) -> (BridgeKey, String) {
    let secret = new_secret();
    let key = BridgeKey {
        key_id: uuid::Uuid::new_v4().simple().to_string(),
        name: name.trim().to_string(),
        key_hash: hash_secret(&secret),
        scope,
        created_at: now_ms(),
        expires_at,
        revoked_at: None,
        last_used_at: None,
        last_ip: None,
    };
    keys.push(key.clone());
    (key, secret)
}

/// 撤销：已撤销的返回 false（幂等语义交由调用方），未撤销的置位并返回 true
pub fn revoke_in_list(keys: &mut [BridgeKey], key_id: &str) -> bool {
    let now = now_ms();
    keys.iter_mut()
        .find(|k| k.key_id == key_id)
        .map(|k| {
            if k.revoked_at.is_some() {
                false
            } else {
                k.revoked_at = Some(now);
                true
            }
        })
        .unwrap_or(false)
}

/// 彻底删除一条已撤销的 key（需求：撤销列表可清理）。未撤销或不存在返回 false
pub fn delete_in_list(keys: &mut Vec<BridgeKey>, key_id: &str) -> bool {
    let at = keys.iter().position(|k| k.key_id == key_id);
    match at {
        Some(i) if keys[i].revoked_at.is_some() => {
            keys.remove(i);
            true
        }
        _ => false,
    }
}

// ===== 文件 IO（账户私有 data_dir） =====

fn keys_dir(app: &tauri::AppHandle) -> Result<PathBuf, String> {
    let base = crate::app::accounts::data_dir(app)?;
    let dir = base.join("mobile_bridge");
    std::fs::create_dir_all(&dir).map_err(|e| format!("创建 mobile_bridge 目录失败: {e}"))?;
    Ok(dir)
}

fn keys_path(app: &tauri::AppHandle) -> Result<PathBuf, String> {
    Ok(keys_dir(app)?.join("keys.json"))
}

pub fn load_keys(app: &tauri::AppHandle) -> Result<Vec<BridgeKey>, String> {
    let path = keys_path(app)?;
    if !path.exists() {
        return Ok(Vec::new());
    }
    let text = std::fs::read_to_string(&path).map_err(|e| format!("读取 key 文件失败: {e}"))?;
    serde_json::from_str(&text).map_err(|e| format!("解析 key 文件失败: {e}"))
}

pub fn save_keys(app: &tauri::AppHandle, keys: &[BridgeKey]) -> Result<(), String> {
    let path = keys_path(app)?;
    let text = serde_json::to_string_pretty(keys).map_err(|e| format!("序列化 key 失败: {e}"))?;
    // 原子写：先写临时文件再改名，避免中途崩溃留下半截 JSON
    let tmp = path.with_extension("json.tmp");
    std::fs::write(&tmp, text).map_err(|e| format!("写入 key 文件失败: {e}"))?;
    std::fs::rename(&tmp, &path).map_err(|e| format!("替换 key 文件失败: {e}"))
}

/// 端口持久化：成功绑定过的端口写回，重启后优先复用（URL 稳定，持久 key 才有意义）
pub fn load_saved_port(app: &tauri::AppHandle) -> Option<u16> {
    let path = keys_dir(app).ok()?.join("port.txt");
    std::fs::read_to_string(path).ok()?.trim().parse().ok()
}

pub fn save_port(app: &tauri::AppHandle, port: u16) {
    if let Ok(dir) = keys_dir(app) {
        let _ = std::fs::write(dir.join("port.txt"), port.to_string());
    }
}

// ===== 单测 =====

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn secret_roundtrip_and_lookup() {
        let mut keys = Vec::new();
        let (k, secret) = create_in_list(&mut keys, "我的手机", KeyScope::All, None);
        assert!(secret.starts_with("mbk_"));
        assert_eq!(secret.len(), 68); // mbk_ + 64 hex
        assert!(k.name == "我的手机");
        // 命中
        let hit = validate_in_list(&keys, &secret, now_ms()).expect("合法 secret 应命中");
        assert_eq!(hit.key_id, k.key_id);
        assert_eq!(hit.scope, KeyScope::All);
        // 错误 secret 不命中
        assert!(validate_in_list(&keys, "mbk_wrong", now_ms()).is_none());
        // hash 不落明文
        assert!(!keys[0].key_hash.contains(&secret));
    }

    #[test]
    fn scope_session_roundtrip() {
        let mut keys = Vec::new();
        let (k, secret) = create_in_list(
            &mut keys,
            "单会话分享",
            KeyScope::Session {
                session_id: "sess-1".into(),
            },
            None,
        );
        assert_eq!(k.scope.tag(), "one");
        assert_eq!(k.scope.session_id(), Some("sess-1"));
        let hit = validate_in_list(&keys, &secret, now_ms()).unwrap();
        assert_eq!(hit.scope.session_id(), Some("sess-1"));
        // serde 往返
        let v = serde_json::to_value(&k.scope).unwrap();
        assert_eq!(v["kind"], "session");
        assert_eq!(v["session_id"], "sess-1");
        let back: KeyScope = serde_json::from_value(v).unwrap();
        assert_eq!(back, k.scope);
    }

    #[test]
    fn expiry_and_revocation() {
        let mut keys = Vec::new();
        let now = now_ms();
        // 已过期
        let (k1, s1) = create_in_list(&mut keys, "过期", KeyScope::All, Some(now - 1));
        assert!(k1.expires_at.is_some());
        assert!(
            validate_in_list(&keys, &s1, now).is_none(),
            "过期 key 不应命中"
        );
        // 未过期（边界：恰好等于 now 视为过期）
        let (_k2, s2) = create_in_list(&mut keys, "没过期", KeyScope::All, Some(now + 60_000));
        assert!(validate_in_list(&keys, &s2, now).is_some());
        // 永久
        let (_k3, s3) = create_in_list(&mut keys, "永久", KeyScope::All, None);
        assert!(validate_in_list(&keys, &s3, now + 365 * 86_400_000).is_some());
        // 撤销后不再命中
        let (k4, s4) = create_in_list(&mut keys, "将撤销", KeyScope::All, None);
        assert!(revoke_in_list(&mut keys, &k4.key_id));
        assert!(validate_in_list(&keys, &s4, now).is_none());
        // 重复撤销返回 false（幂等）
        assert!(!revoke_in_list(&mut keys, &k4.key_id));
        // 未知 id 返回 false
        assert!(!revoke_in_list(&mut keys, "no-such"));
    }

    #[test]
    fn delete_only_revoked() {
        let mut keys = Vec::new();
        let (k1, _s1) = create_in_list(&mut keys, "未撤销不可删", KeyScope::All, None);
        let (k2, _s2) = create_in_list(&mut keys, "已撤销可删", KeyScope::All, None);
        assert!(revoke_in_list(&mut keys, &k2.key_id));
        // 未撤销 / 未知 id：拒绝
        assert!(!delete_in_list(&mut keys, &k1.key_id));
        assert!(!delete_in_list(&mut keys, "no-such"));
        assert_eq!(keys.len(), 2);
        // 已撤销：删除成功且列表缩短
        assert!(delete_in_list(&mut keys, &k2.key_id));
        assert_eq!(keys.len(), 1);
        assert_eq!(keys[0].key_id, k1.key_id);
    }

    #[test]
    fn backoff_growth_capped() {
        assert_eq!(backoff_secs(0), 0);
        assert_eq!(backoff_secs(1), 2);
        assert_eq!(backoff_secs(3), 8);
        assert_eq!(backoff_secs(10), 300, "应封顶 300s");
        assert_eq!(backoff_secs(100), 300);
    }

    #[test]
    fn view_strips_hash() {
        let mut keys = Vec::new();
        let (k, _s) = create_in_list(&mut keys, "x", KeyScope::All, None);
        let v = k.to_view();
        assert!(v.get("key_hash").is_none());
        assert_eq!(v["id"], k.key_id);
        assert_eq!(v["scope_tag"], "all");
    }
}
