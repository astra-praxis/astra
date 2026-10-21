//! 开发模式 - 管理 casp-portal 项目下的卡片前端服务
//!
//! 扫描项目下的子目录（卡片），检测 web/ / mobile/ / component/ 目录。
//! 支持并行启动/停止 pnpm dev 服务。

pub(crate) mod commands;
pub(crate) mod config;
pub(crate) mod history;
pub(crate) mod parse;
pub(crate) mod process;
pub(crate) mod types;

pub(crate) use process::{kill_processes_on_port, terminate_process_tree};
pub(crate) use types::*;
