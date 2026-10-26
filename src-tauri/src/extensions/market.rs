//! 扩展页后端（MCP / Skill）：
//!
//! - MCP/ACP：不再提供远程目录与一键安装；前端展示已安装（config.toml
//!   已配置）的扩展，启停 / 删除由 extensions.rs 的命令承担，运行状态由
//!   agent.rs 的 extensions_runtime_status 提供。新扩展由用户在 Agent 对话
//!   中让 AI 下载并写好配置（配置本体在 ~/.astra/config.toml 的
//!   [mcp.servers.<id>] / [acp.agents.<id>] 段，冻结 schema 见
//!   MCP_SPEC §11 / ACP_SPEC §11；安装态读取与 extensions_list 同源）
//! - Skill：不再提供 skills.sh 在线搜索与 git 安装；已安装技能由
//!   user_skills.rs 的 list_user_skills 展示，新技能同样回 Agent 对话让
//!   AI 下载装进技能库

use serde::Serialize;
use std::path::PathBuf;

#[derive(Debug, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct MarketStatus {
    pub node_available: bool,
}

/// 检查裸命令是否可解析：GUI 启动的生产包里宿主 PATH 极小（不含
/// /opt/homebrew/bin、nvm/fnm 等），which::which 会误报；走 astra-process
/// 的解析器（宿主 PATH + 常见安装位 + 版本管理器 shim 目录）。
fn tool_available(name: &str) -> bool {
    astra_process::resolve_sync(name).resolved
}

#[tauri::command]
pub fn market_status() -> MarketStatus {
    MarketStatus {
        node_available: tool_available("npm"),
    }
}

// ===== 配置文件定位与解析（extensions.rs 共用） =====

pub(crate) fn astra_config_path() -> Result<PathBuf, String> {
    // 账户模式下为 accounts/<id>/astra-home/config.toml；legacy 回退 ~/.astra
    let dir = crate::app::accounts::astra_home();
    std::fs::create_dir_all(&dir).map_err(|e| format!("创建 astra home 失败: {e}"))?;
    Ok(dir.join("config.toml"))
}

/// 把字符串数组序列化为 TOML 数组字面量（extensions.rs 写段共用）
pub(crate) fn toml_array(values: &[String]) -> String {
    values
        .iter()
        .map(|v| format!("\"{}\"", v.replace('\\', "\\\\").replace('"', "\\\"")))
        .collect::<Vec<_>>()
        .join(", ")
}
