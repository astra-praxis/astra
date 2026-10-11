//! # agent-core
//!
//! 星野 Agent Runtime 核心库。参考 Zed agent 架构分四层：
//!
//! 1. **model**（模型适配层）：`ModelBackend` trait —— 任何能流式产生
//!    `Delta` 的后端都可接入（rig / 自研 SSE / 未来 ACP 外部 agent）。
//!    当前内置 `RigBackend`（rig-core 的 OpenAI 兼容 chat completions）。
//! 2. **run**（运行层）：自定义 Agent Loop —— 模型流式请求 → 工具调用拦截 →
//!    权限判定 → 工具执行 → 结果回传 → 续轮。可取消（CancellationToken），
//!    带空闲超时与轮数预算，产出带 sequence 的 `Envelope` 事件流。
//! 3. **tool**（工具层）：`AgentTool` trait —— 声明式 schema 注册，
//!    工具错误以字符串回传模型（模型可自恢复）。
//! 4. **thread**（会话层）：`Thread` —— 长期会话：消息序列、多轮 Run、
//!    项目上下文绑定。UI 只消费 `Envelope` 与 Thread 快照，不依赖具体后端。
//!
//! 设计依据：`build-agent-runtime` skill（Zed 源码研究）。
//! 与 rig 高层 `stream_chat` 的关键差异：工具调用在事件流中**可见**
//! （rig 0.22 高层循环不 yield ToolCall，实测确认），权限在执行前挂起。

pub mod error;
pub mod events;
pub mod model;
pub mod openai_sse;
pub mod permissions;
pub mod run;
pub mod thread;
pub mod tool;

pub mod ask_tool;
pub mod doc_tools;
pub mod fs_basic;
pub mod fs_tools;
pub mod net_tools;
pub mod shell_tool;
pub mod skill_tools;
#[cfg(test)]
pub(crate) mod test_support;

pub use ask_tool::{AskUserTool, AutoAnswerBridge, UserQuestionBridge};
pub use error::{Error, Result};
pub use events::{Envelope, Plan, PlanStep, PlanStepState, RunEvent, RunStatus, Sequence};
pub use model::{ChatMessage, Delta, MessageContent, ModelBackend, ModelRequest, RigBackend, Role};
pub use net_tools::{FetchTool, SearchResult, WebSearchTool};
pub use permissions::{
    decide, ensure_within_root, validate_args, AutoBridge, Decision, PermissionBridge,
    PermissionRequest, ToolPolicy,
};
pub use run::{execute, RunConfig, RunHandle, DEFAULT_AUTO_COMPACT_TOKENS, DEFAULT_MAX_TURNS};
pub use thread::{Thread, ThreadConfig};
pub use tool::{
    AgentTool, Artifact, ArtifactKind, ToolCall, ToolContext, ToolDefinition, ToolOutput,
    ToolRegistry,
};
