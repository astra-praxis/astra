//! 权限判定：纯函数静态层 + 运行时批准通道。
//!
//! 对齐 skill 原则：
//! - 判定是纯函数（工具 + 参数 + 上下文 → Allow/Deny/Confirm），可单测
//! - 静态判定不信任模型输出；Confirm 交由前端收集决定，
//!   **后端只执行已批准的具体操作**（批准绑定 tool_call_id，不扩大范围）
//! - 路径校验必须规范化（含符号链接），不能用字符串前缀

use serde::{Deserialize, Serialize};
use serde_json::Value;
use std::path::{Path, PathBuf};

use crate::error::{Error, Result};

/// 静态判定结果
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum Decision {
    Allow,
    Confirm,
    Deny,
}

/// 权限请求（发给前端确认卡的数据，宿主层负责呈现与收集决定）
#[derive(Debug, Clone)]
pub struct PermissionRequest {
    pub tool_call_id: String,
    pub tool_name: String,
    /// 展示用参数摘要（如目标路径）
    pub summary: String,
    /// 完整参数（宿主桥实现智能放行策略时使用，如项目内 shell 自动允许）
    pub arguments: serde_json::Value,
}

/// 运行时批准桥：核心库不依赖 Tauri/UI，宿主层实现此 trait。
/// 实现约定：向 UI 发出确认请求并等待用户决定（内部用 oneshot）。
/// 用户拒绝或超时 → Deny；核心库保证取消时不会消费过期授权。
#[async_trait::async_trait]
pub trait PermissionBridge: Send + Sync {
    async fn request(&self, req: PermissionRequest) -> Decision;
}

/// 测试/探针用：自动放行或自动拒绝的桥
pub struct AutoBridge(pub Decision);

#[async_trait::async_trait]
impl PermissionBridge for AutoBridge {
    async fn request(&self, _req: PermissionRequest) -> Decision {
        self.0
    }
}

/// 工具策略：按工具名给出静态判定。
/// MVP 规则（确认过的设计）：
/// - list/read/search：Allow（只读）
/// - write_file：Confirm（写文件）
/// - 未注册工具：Deny（未声明参数不得静默接受）
pub fn decide(policy: &ToolPolicy, tool_name: &str) -> Decision {
    match policy
        .allowed_readonly
        .iter()
        .find(|n| n.as_str() == tool_name)
    {
        Some(_) => Decision::Allow,
        None => match policy
            .confirm_tools
            .iter()
            .find(|n| n.as_str() == tool_name)
        {
            Some(_) => Decision::Confirm,
            None => Decision::Deny,
        },
    }
}

/// 工具策略配置（后续从 settings 读取）
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct ToolPolicy {
    pub allowed_readonly: Vec<String>,
    pub confirm_tools: Vec<String>,
}

impl Default for ToolPolicy {
    fn default() -> Self {
        Self {
            allowed_readonly: vec![
                "list_files".into(),
                "read_file".into(),
                "search_files".into(),
                "list_skills".into(),
                "read_skill".into(),
            ],
            confirm_tools: vec!["write_file".into()],
        }
    }
}

/// 路径边界校验：canonicalize（符号链接在此拦截）后必须是 root 子路径。
/// 目标尚不存在时校验其最近存在祖先（覆盖 `mkdir -p` 式写入）。
/// 相对路径先 join 到 root（canonicalize 是相对进程 cwd 解析的，不能直接用）。
pub fn ensure_within_root(root: &Path, target: &Path) -> Result<PathBuf> {
    let canonical_root = root
        .canonicalize()
        .map_err(|e| Error::PathEscape(format!("项目根目录无效: {e}")))?;

    // 绝对路径原样校验；相对路径基于项目根（并归一化掉 ./ 等分量）
    let joined: PathBuf = if target.is_absolute() {
        target.to_path_buf()
    } else {
        root.join(target).components().collect()
    };

    let mut probe = joined.clone();
    let canonical_target = loop {
        match probe.canonicalize() {
            Ok(p) => break p,
            Err(_) => {
                // 不存在的部分：若当前探针本身是符号链接，必须解析链接目标。
                // canonicalize 只能解析“目标已存在”的链接；链接指向尚未创建的
                // 文件时会在此失败，若直接 pop 会越过链接层漏掉逃逸。
                let is_link = std::fs::symlink_metadata(&probe)
                    .map(|m| m.file_type().is_symlink())
                    .unwrap_or(false);
                if is_link {
                    let link_dir = probe
                        .parent()
                        .unwrap_or(std::path::Path::new("/"))
                        .to_path_buf();
                    let link_target = std::fs::read_link(&probe)
                        .map_err(|e| Error::PathEscape(format!("{}: {e}", target.display())))?;
                    // 递归校验链接目标（read_link 可能返回相对路径，join 统一处理；
                    // 绝对路径会被 join 替换）
                    probe = ensure_within_root(root, &link_dir.join(link_target))?;
                    continue;
                }
                // 普通不存在路径：向上找最近的存在祖先
                if !probe.pop() {
                    return Err(Error::PathEscape(target.display().to_string()));
                }
            }
        }
    };

    if !canonical_target.starts_with(&canonical_root) {
        return Err(Error::PathEscape(target.display().to_string()));
    }
    Ok(joined)
}

/// 参数白名单校验：schema required 字段必须存在且类型正确（MVP 只查 required + string）
pub fn validate_args(schema: &Value, args: &Value) -> Result<()> {
    let required = schema
        .get("required")
        .and_then(|v| v.as_array())
        .cloned()
        .unwrap_or_default();
    for key in required.iter().filter_map(|v| v.as_str()) {
        let present = args.get(key).is_some_and(|v| !v.is_null());
        if !present {
            return Err(Error::Tool {
                tool: "validate_args".into(),
                message: format!("缺少必填参数: {key}"),
            });
        }
    }
    Ok(())
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn readonly_tools_allow() {
        let policy = ToolPolicy::default();
        assert_eq!(decide(&policy, "read_file"), Decision::Allow);
        assert_eq!(decide(&policy, "list_files"), Decision::Allow);
    }

    #[test]
    fn write_tools_confirm() {
        let policy = ToolPolicy::default();
        assert_eq!(decide(&policy, "write_file"), Decision::Confirm);
    }

    #[test]
    fn unknown_tools_deny() {
        let policy = ToolPolicy::default();
        assert_eq!(decide(&policy, "exec"), Decision::Deny);
        assert_eq!(decide(&policy, ""), Decision::Deny);
    }

    #[test]
    fn path_escape_detected() {
        let dir = std::env::temp_dir().join("agent-core-test-root");
        std::fs::create_dir_all(&dir).unwrap();
        std::fs::write(dir.join("a.txt"), "x").unwrap();

        // 项目内 OK（绝对路径）
        assert!(ensure_within_root(&dir, &dir.join("a.txt")).is_ok());
        // 项目内 OK（相对路径，含 ./ 前缀）
        let rel = ensure_within_root(&dir, std::path::Path::new("./a.txt")).unwrap();
        assert_eq!(rel, dir.join("a.txt"));
        // 项目外拒绝
        assert!(ensure_within_root(&dir, std::path::Path::new("/etc/passwd")).is_err());
        // 前缀欺骗拒绝
        let sibling_prefix = dir.parent().unwrap().join(format!(
            "{}-evil",
            dir.file_name().unwrap().to_string_lossy()
        ));
        std::fs::create_dir_all(&sibling_prefix).unwrap();
        assert!(ensure_within_root(&dir, &sibling_prefix.join("f.txt")).is_err());
        // 相对路径跨目录逃逸拒绝
        assert!(ensure_within_root(&dir, std::path::Path::new("../evil/f.txt")).is_err());

        std::fs::remove_dir_all(&sibling_prefix).unwrap();
        std::fs::remove_dir_all(&dir).unwrap();
    }

    #[test]
    fn symlink_escape_detected() {
        let dir = std::env::temp_dir().join("agent-core-test-symlink");
        let _ = std::fs::remove_dir_all(&dir); // 清理上次运行残留，避免 symlink 重复创建失败
        std::fs::create_dir_all(&dir).unwrap();
        #[cfg(unix)]
        {
            std::os::unix::fs::symlink("/etc", dir.join("link")).unwrap();
            assert!(ensure_within_root(&dir, &dir.join("link/passwd")).is_err());
        }
        std::fs::remove_dir_all(&dir).unwrap();
    }

    #[test]
    fn args_validation() {
        let schema = serde_json::json!({
            "type": "object",
            "properties": { "path": { "type": "string" } },
            "required": ["path"]
        });
        assert!(validate_args(&schema, &serde_json::json!({"path": "a.txt"})).is_ok());
        assert!(validate_args(&schema, &serde_json::json!({})).is_err());
    }

    #[test]
    fn custom_policy_overrides_defaults() {
        // 策略可由宿主层配置：write_file 也可降为只读名单（不推荐，但机制上允许）
        let policy = ToolPolicy {
            allowed_readonly: vec!["write_file".into()],
            confirm_tools: vec![],
        };
        assert_eq!(decide(&policy, "write_file"), Decision::Allow);
        assert_eq!(decide(&policy, "read_file"), Decision::Deny);
    }

    #[test]
    fn decision_serializes_snake_case() {
        assert_eq!(serde_json::to_value(Decision::Allow).unwrap(), "allow");
        assert_eq!(serde_json::to_value(Decision::Confirm).unwrap(), "confirm");
        assert_eq!(serde_json::to_value(Decision::Deny).unwrap(), "deny");
    }

    #[test]
    fn nested_nonexistent_path_within_root_is_allowed() {
        // mkdir -p 式写入：多级不存在目录也在边界内
        let dir = std::env::temp_dir().join("agent-core-test-nested");
        std::fs::create_dir_all(&dir).unwrap();
        let target = dir.join("docs/2026/new/report.md");
        let resolved = ensure_within_root(&dir, &target).unwrap();
        assert_eq!(resolved, target);
        std::fs::remove_dir_all(&dir).unwrap();
    }

    #[test]
    fn root_itself_is_within_root() {
        let dir = std::env::temp_dir().join("agent-core-test-rootself");
        std::fs::create_dir_all(&dir).unwrap();
        assert!(ensure_within_root(&dir, &dir).is_ok());
        // 相对路径 "" 与 "." 归一化后即根
        assert!(ensure_within_root(&dir, std::path::Path::new(".")).is_ok());
        std::fs::remove_dir_all(&dir).unwrap();
    }

    #[test]
    fn invalid_root_errors() {
        assert!(ensure_within_root(
            std::path::Path::new("/nonexistent-root-for-agent-core-test"),
            std::path::Path::new("a.txt")
        )
        .is_err());
    }

    #[test]
    fn empty_required_list_accepts_any_object() {
        let schema = serde_json::json!({"type": "object", "required": []});
        assert!(validate_args(&schema, &serde_json::json!({"anything": 1})).is_ok());
    }

    #[test]
    fn schema_without_required_key_accepts_anything() {
        let schema = serde_json::json!({"type": "object"});
        assert!(validate_args(&schema, &serde_json::json!({})).is_ok());
    }

    #[test]
    fn null_required_value_counts_as_missing() {
        let schema = serde_json::json!({
            "type": "object",
            "required": ["path"]
        });
        assert!(validate_args(&schema, &serde_json::json!({"path": null})).is_err());
    }

    #[test]
    fn symlink_escape_detected_for_write_target() {
        // 写目标是通过符号链接指向项目外的不存在路径：最近存在祖先是链接本身
        let dir = std::env::temp_dir().join("agent-core-test-symlink-write");
        let _ = std::fs::remove_dir_all(&dir); // 清理上次运行残留，避免 symlink 重复创建失败
        std::fs::create_dir_all(&dir).unwrap();
        #[cfg(unix)]
        {
            std::os::unix::fs::symlink("/tmp/outside-agent-core", dir.join("out")).unwrap();
            assert!(ensure_within_root(&dir, &dir.join("out/file.txt")).is_err());
        }
        std::fs::remove_dir_all(&dir).unwrap();
    }
}
