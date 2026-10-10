//! 统一错误类型。所有 fallible 操作错误向上传播，不静默丢弃。

use thiserror::Error;

#[derive(Debug, Error)]
pub enum Error {
    #[error("模型请求失败: {0}")]
    Model(String),

    #[error("流被取消")]
    Cancelled,

    #[error("工具 '{tool}' 执行失败: {message}")]
    Tool { tool: String, message: String },

    #[error("权限拒绝: {message}")]
    PermissionDenied { message: String },

    #[error("路径越界: {0}")]
    PathEscape(String),

    #[error("运行超时（预算 {budget_secs}s）")]
    BudgetExceeded { budget_secs: u64 },

    #[error("已达最大续轮数（{max_turns} 轮），任务未完成；请拆小任务重试")]
    MaxTurnsExceeded { max_turns: u32 },

    #[error("上下文压缩失败: {0}")]
    CompactionFailed(String),

    #[error("{0}")]
    Other(String),
}

pub type Result<T> = std::result::Result<T, Error>;
