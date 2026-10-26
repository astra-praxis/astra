//! JSON 会话存储共享助手（chat_sessions.rs / agent_sessions.rs 共用）。
//!
//! 两类会话都是「单 JSON 文件全量读写」模式：
//! app_data_dir 下一个 `[]` 数组文件，读时损坏兜底为空数组，写时 pretty 序列化。

use serde::de::DeserializeOwned;
use serde::Serialize;
use std::fs;
use std::path::PathBuf;
use tauri::AppHandle;

/// 确保 app_data_dir 存在且目标 JSON 文件已初始化（不存在则写入 `[]`），返回文件路径。
///
/// `create_err_label` 用于拼接创建失败文案：`"{label}失败: {e}"`。
pub fn ensure_json_file(
    app: &AppHandle,
    filename: &str,
    create_err_label: &str,
) -> Result<PathBuf, String> {
    let app_data_dir = crate::app::accounts::data_dir(app)?;
    fs::create_dir_all(&app_data_dir).map_err(|e| format!("创建配置目录失败: {}", e))?;
    let path = app_data_dir.join(filename);
    if !path.exists() {
        fs::write(&path, "[]").map_err(|e| format!("{}失败: {}", create_err_label, e))?;
    }
    Ok(path)
}

/// 读取 JSON 数组文件；文件缺失或内容损坏时兜底为空数组。
pub fn read_json_vec<T: DeserializeOwned>(path: &PathBuf) -> Vec<T> {
    let content = fs::read_to_string(path).unwrap_or_else(|_| "[]".to_string());
    serde_json::from_str(&content).unwrap_or_default()
}

/// pretty 序列化并写回 JSON 数组文件。
pub fn write_json_vec<T: Serialize>(path: &PathBuf, items: &[T]) -> Result<(), String> {
    let json =
        serde_json::to_string_pretty(items).map_err(|e| format!("序列化会话记录失败: {}", e))?;
    fs::write(path, json).map_err(|e| format!("写入会话记录失败: {}", e))
}

/// 秒级 Unix 时间戳。
pub fn unix_timestamp() -> u64 {
    std::time::SystemTime::now()
        .duration_since(std::time::UNIX_EPOCH)
        .unwrap_or_default()
        .as_secs()
}

#[cfg(test)]
mod tests {
    use super::*;
    use serde::{Deserialize, Serialize};

    #[derive(Debug, Serialize, Deserialize, PartialEq)]
    struct Item {
        id: String,
    }

    #[test]
    fn read_json_vec_roundtrip() {
        let dir = std::env::temp_dir().join(format!("astra_ss_test_{}", std::process::id()));
        fs::create_dir_all(&dir).unwrap();
        let path = dir.join("items.json");

        // 文件不存在 → 空数组
        assert!(read_json_vec::<Item>(&path).is_empty());

        // 写入后可读回
        write_json_vec(&path, &[Item { id: "a".into() }, Item { id: "b".into() }]).unwrap();
        let items = read_json_vec::<Item>(&path);
        assert_eq!(items.len(), 2);
        assert_eq!(items[1].id, "b");

        // 内容损坏 → 兜底空数组
        fs::write(&path, "not json").unwrap();
        assert!(read_json_vec::<Item>(&path).is_empty());

        fs::remove_dir_all(&dir).ok();
    }
}
