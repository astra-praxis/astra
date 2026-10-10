//! 工具层：`AgentTool` trait 与运行上下文。
//!
//! 设计要点（对齐 skill）：
//! - 工具自己控制执行；如需追加权限，通过 `ToolContext::request_permission`
//!   挂起等待（`PermissionBridge` 由宿主层实现，取消优先）
//! - `definition` 是纯数据 JSON schema，供后端组装 tools 参数
//! - 工具错误以字符串回传模型（实测 rig/兼容服务该行为可让模型自恢复），
//!   同时由 Run 循环发出结构化失败事件

use crate::error::Result;
use crate::permissions::{Decision, PermissionBridge, PermissionRequest};
use async_trait::async_trait;
use serde::{Deserialize, Serialize};
use serde_json::Value;
use std::sync::Arc;
use tokio_util::sync::CancellationToken;

/// 传给 OpenAI 兼容接口的工具定义
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct ToolDefinition {
    pub name: String,
    pub description: String,
    pub parameters: Value,
}

/// 一次工具调用（模型侧 ID + 名称 + 参数）
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct ToolCall {
    pub id: String,
    pub call_id: Option<String>,
    pub name: String,
    pub arguments: Value,
}

/// 工具执行产物：结构化保存（不只是一段渲染文本），
/// 产物文件单独列出供前端展示与恢复渲染。
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct ToolOutput {
    pub content: String,
    #[serde(default, skip_serializing_if = "Vec::is_empty")]
    pub artifacts: Vec<Artifact>,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct Artifact {
    /// 相对项目根的路径
    pub path: String,
    pub kind: ArtifactKind,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum ArtifactKind {
    FileCreated,
    FileModified,
}

impl From<String> for ToolOutput {
    fn from(content: String) -> Self {
        Self {
            content,
            artifacts: vec![],
        }
    }
}

impl ToolOutput {
    pub fn with_artifacts(content: impl Into<String>, artifacts: Vec<Artifact>) -> Self {
        Self {
            content: content.into(),
            artifacts,
        }
    }
}

/// 工具运行上下文：权限桥 + 取消信号 + 项目根
pub struct ToolContext {
    /// 项目根目录（None = 纯聊天模式，未绑定项目）
    pub project_root: Option<std::path::PathBuf>,
    /// 运行时批准桥：宿主层（Tauri）实现——发事件给前端 + oneshot 等决定
    pub bridge: Arc<dyn PermissionBridge>,
    /// 取消令牌：工具内部的耗时操作应监听此信号（如子进程终止）
    pub cancelled: CancellationToken,
}

impl ToolContext {
    /// 工具执行中途申请权限（Confirm 类的补充授权）。
    /// 取消优先：Run 被取消时立即返回 Deny，且宿主层收到的请求会被
    /// 取消（bridge 实现方应同时监听 cancelled，或在决定前检查）。
    pub async fn request_permission(&self, tool_name: &str, summary: String) -> Decision {
        let req = PermissionRequest {
            tool_call_id: String::new(),
            tool_name: tool_name.to_string(),
            summary,
        };
        tokio::select! {
            _ = self.cancelled.cancelled() => Decision::Deny,
            decision = self.bridge.request(req) => decision,
        }
    }
}

/// Agent 工具 trait。实现放 Box<dyn AgentTool> 进注册表。
#[async_trait]
pub trait AgentTool: Send + Sync {
    fn name(&self) -> &str;
    fn description(&self) -> &str;
    /// JSON Schema（声明式注册，参数契约一目了然，可单测）
    fn parameters_schema(&self) -> Value;
    /// 执行。错误信息会回传模型；实现方应保证路径/参数已校验。
    async fn call(&self, args: Value, ctx: &ToolContext) -> Result<ToolOutput>;
}

/// 类型擦除注册表
#[derive(Default, Clone)]
pub struct ToolRegistry {
    tools: Vec<Arc<dyn AgentTool>>,
}

impl ToolRegistry {
    pub fn new() -> Self {
        Self::default()
    }

    pub fn register(mut self, tool: impl AgentTool + 'static) -> Self {
        self.tools.push(Arc::new(tool));
        self
    }

    pub fn get(&self, name: &str) -> Option<Arc<dyn AgentTool>> {
        self.tools.iter().find(|t| t.name() == name).cloned()
    }

    pub fn definitions(&self) -> Vec<ToolDefinition> {
        self.tools
            .iter()
            .map(|t| ToolDefinition {
                name: t.name().to_string(),
                description: t.description().to_string(),
                parameters: t.parameters_schema(),
            })
            .collect()
    }

    pub fn names(&self) -> Vec<String> {
        self.tools.iter().map(|t| t.name().to_string()).collect()
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::permissions::{AutoBridge, Decision, PermissionRequest};
    use std::time::Duration;
    use tokio_util::sync::CancellationToken;

    struct StubTool {
        name: &'static str,
        schema: Value,
    }

    #[async_trait]
    impl AgentTool for StubTool {
        fn name(&self) -> &str {
            self.name
        }
        fn description(&self) -> &str {
            "stub"
        }
        fn parameters_schema(&self) -> Value {
            self.schema.clone()
        }
        async fn call(&self, _args: Value, _ctx: &ToolContext) -> crate::Result<ToolOutput> {
            Ok(ToolOutput::from(String::from("ok")))
        }
    }

    fn ctx(
        bridge: Arc<dyn crate::permissions::PermissionBridge>,
        token: CancellationToken,
    ) -> ToolContext {
        ToolContext {
            project_root: None,
            bridge,
            cancelled: token,
        }
    }

    // ===== ToolRegistry =====

    #[test]
    fn registry_register_get_definitions_names() {
        let registry = ToolRegistry::new()
            .register(StubTool {
                name: "read_file",
                schema: serde_json::json!({"type": "object"}),
            })
            .register(StubTool {
                name: "write_file",
                schema: serde_json::json!({"type": "object", "required": ["path"]}),
            });

        assert!(registry.get("read_file").is_some());
        assert!(registry.get("missing").is_none());
        assert_eq!(registry.names(), vec!["read_file", "write_file"]);

        let defs = registry.definitions();
        assert_eq!(defs.len(), 2);
        assert_eq!(defs[0].name, "read_file");
        assert_eq!(defs[1].parameters["required"], serde_json::json!(["path"]));
    }

    #[test]
    fn registry_empty_has_no_definitions() {
        let registry = ToolRegistry::new();
        assert!(registry.get("read_file").is_none());
        assert!(registry.names().is_empty());
        assert!(registry.definitions().is_empty());
    }

    #[test]
    fn registry_get_returns_shared_arc() {
        let registry = ToolRegistry::new().register(StubTool {
            name: "t",
            schema: Value::Null,
        });
        let a = registry.get("t").unwrap();
        let b = registry.get("t").unwrap();
        assert!(Arc::ptr_eq(&a, &b), "同一工具应返回同一 Arc 实例");
    }

    // ===== ToolOutput serde =====

    #[test]
    fn tool_output_from_string_has_no_artifacts_field() {
        let output = ToolOutput::from("内容".to_string());
        let v = serde_json::to_value(&output).unwrap();
        assert_eq!(v["content"], "内容");
        assert!(v.get("artifacts").is_none(), "空产物应省略");
    }

    #[test]
    fn tool_output_artifacts_roundtrip() {
        let output = ToolOutput::with_artifacts(
            "已写入",
            vec![Artifact {
                path: "out/a.md".into(),
                kind: ArtifactKind::FileCreated,
            }],
        );
        let json = serde_json::to_string(&output).unwrap();
        let back: ToolOutput = serde_json::from_str(&json).unwrap();
        assert_eq!(back.artifacts[0].path, "out/a.md");
        assert_eq!(back.artifacts[0].kind, ArtifactKind::FileCreated);
    }

    #[test]
    fn artifact_kind_serializes_snake_case() {
        let v = serde_json::to_value(ArtifactKind::FileModified).unwrap();
        assert_eq!(v, "file_modified");
    }

    // ===== ToolContext::request_permission =====

    #[tokio::test]
    async fn request_permission_returns_bridge_decision() {
        let token = CancellationToken::new();
        let context = ctx(Arc::new(AutoBridge(Decision::Allow)), token.clone());
        let d = context
            .request_permission("write_file", "写入 a.txt".into())
            .await;
        assert_eq!(d, Decision::Allow);

        let context = ctx(Arc::new(AutoBridge(Decision::Deny)), token);
        let d = context
            .request_permission("write_file", "写入 a.txt".into())
            .await;
        assert_eq!(d, Decision::Deny);
    }

    #[tokio::test]
    async fn request_permission_cancelled_first_is_deny_without_calling_bridge() {
        // 令牌已取消 + 桥永不返回：select! 只能走取消分支 → Deny，且不调桥
        let token = CancellationToken::new();
        token.cancel();
        let bridge = Arc::new(crate::test_support::PendingBridge);
        let context = ctx(bridge, token);
        let d = context.request_permission("write_file", "x".into()).await;
        assert_eq!(d, Decision::Deny);
    }

    #[tokio::test]
    async fn request_permission_cancel_during_pending_beats_slow_bridge() {
        // 桥 300ms 后才返回 Allow；100ms 时取消 → 应立即 Deny（取消优先）
        let token = CancellationToken::new();
        let bridge = Arc::new(crate::test_support::SleepingBridge::new(
            300,
            Decision::Allow,
        ));
        let context = ctx(bridge.clone(), token.clone());

        let task =
            tokio::spawn(async move { context.request_permission("write_file", "x".into()).await });
        tokio::time::sleep(Duration::from_millis(100)).await;
        token.cancel();

        let started = std::time::Instant::now();
        let d = task.await.unwrap();
        assert_eq!(d, Decision::Deny, "取消应压过未完成的授权");
        assert!(
            started.elapsed() < Duration::from_millis(250),
            "取消后应立即返回，不应等桥的 300ms"
        );
    }

    #[tokio::test]
    async fn request_permission_slow_bridge_not_cancelled_returns_decision() {
        // 无取消时，等待桥的最终决定
        let token = CancellationToken::new();
        let bridge = Arc::new(crate::test_support::SleepingBridge::new(30, Decision::Deny));
        let context = ctx(bridge, token);
        let d = context.request_permission("write_file", "x".into()).await;
        assert_eq!(d, Decision::Deny);
    }

    #[tokio::test]
    async fn tool_call_arguments_roundtrip() {
        // ToolCall 会被持久化（阶段 4），serde 形状需稳定
        let call = ToolCall {
            id: "t1".into(),
            call_id: Some("c1".into()),
            name: "write_file".into(),
            arguments: serde_json::json!({"path": "a.txt", "content": "hi"}),
        };
        let json = serde_json::to_string(&call).unwrap();
        let back: ToolCall = serde_json::from_str(&json).unwrap();
        assert_eq!(back.id, "t1");
        assert_eq!(back.call_id.as_deref(), Some("c1"));
        assert_eq!(back.name, "write_file");
    }

    // 抑制未使用告警（PermissionRequest 仅在桥签名中出现）
    #[allow(dead_code)]
    fn _touch(_: PermissionRequest) {}
}
