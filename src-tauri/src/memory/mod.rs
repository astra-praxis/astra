//! 记忆与检索：astra-memory 宿主集成、RAG 索引管线
//!
//! - `host`：CME 记忆宿主（加密 SQLite + recall 注入 + 提炼/确认通道）
//! - `tool`：memory_remember / memory_forget 等 agent 工具注册
//! - `embedding` / `indexer` / `vector_store`：项目索引 RAG 管线

pub(crate) mod embedding;
pub(crate) mod host;
pub(crate) mod indexer;
pub(crate) mod tool;
pub(crate) mod vector_store;
