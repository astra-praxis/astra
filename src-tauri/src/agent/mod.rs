//! Agent Runtime 桥接层：把 agent-core 的事件流接到 Tauri 前端。
//!
//! 职责边界（对齐 build-agent-runtime skill）：
//! - agent-core 拥有运行循环、权限判定与工具执行；本层只做「宿主适配」：
//!   模型来源解析（provider / 本地 llama.cpp）、事件转发（Envelope → agent-event）、
//!   权限决定回传（oneshot hub）与运行取消
//! - 前端只消费 Envelope（type 标签 + snake_case 载荷），不依赖 agent-core 具体类型
//! - 权限批准绑定 tool_call_id：批准只放行该次具体调用，不扩大范围
//!
//! 会话模型（MVP）：
//! - session_id 由前端生成；Thread 按 session_id 缓存（含消息历史与项目绑定）
//! - 切换项目 / 模型来源后需新会话（Thread 创建时绑定 root 与 backend）
//! - `agent_reset_session` unloads memory only; SQLite owns restart history.

pub(crate) mod artifacts;
pub(crate) mod images;
pub(crate) mod prompts;
pub(crate) mod session_store;
pub(crate) mod sessions;
pub(crate) mod threads;
pub(crate) mod checkpoints;
pub(crate) mod commands;
pub(crate) mod events;
pub(crate) mod permissions;
pub(crate) mod preview;
pub(crate) mod questions;
pub(crate) mod registry;
pub(crate) mod run;
pub(crate) mod state;
pub(crate) mod status;

#[cfg(test)]
mod tests;

pub(crate) use events::*;
pub(crate) use permissions::*;
pub(crate) use questions::*;
pub(crate) use registry::*;
pub(crate) use state::*;
pub(crate) use status::*;
