use std::collections::HashMap;

use serde::Serialize;
use tauri::State;


use super::*;
// ===== 运行状态查询 =====

#[derive(Debug, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct McpRuntimeStatus {
    pub id: String,
    /// stopped | starting | ready | failed
    pub status: String,
    pub tool_count: usize,
    pub tools: Vec<String>,
    pub error: Option<String>,
    pub attempts: u32,
}

#[derive(Debug, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct AcpRuntimeStatus {
    pub id: String,
    /// stopped | starting | ready | failed
    pub status: String,
    pub tool: Option<String>,
    pub error: Option<String>,
    pub attempts: u32,
}

#[derive(Debug, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct SessionRuntimeStatus {
    pub session_id: String,
    pub mcp: Vec<McpRuntimeStatus>,
    pub acp: Vec<AcpRuntimeStatus>,
}

/// MCP/ACP 运行状态（agent_start_run 装配时登记的 manager 快照）。
/// `session_id = None` 返回所有活跃会话；会话存在但未配置扩展 → 空 vec。
#[tauri::command]
pub async fn extensions_runtime_status(
    state: State<'_, AgentState>,
    session_id: Option<String>,
) -> Result<Vec<SessionRuntimeStatus>, String> {
    let extensions = state.extensions.lock().await;
    let mut out = Vec::new();
    for (sid, ext) in extensions.iter() {
        if session_id.as_ref().is_some_and(|s| s != sid) {
            continue;
        }
        let mut mcp = Vec::new();
        for manager in &ext.mcp {
            for s in manager.snapshot().await {
                mcp.push(McpRuntimeStatus {
                    id: s.id,
                    status: format!("{:?}", s.status).to_lowercase(),
                    tool_count: s.tools.len(),
                    tools: s.tools.into_iter().map(|t| t.remote).collect(),
                    error: s.error,
                    attempts: s.attempts,
                });
            }
        }
        let mut acp = Vec::new();
        for manager in &ext.acp {
            for s in manager.snapshot().await {
                acp.push(AcpRuntimeStatus {
                    id: s.id,
                    status: format!("{:?}", s.status).to_lowercase(),
                    tool: s.tool,
                    error: s.error,
                    attempts: s.attempts,
                });
            }
        }
        out.push(SessionRuntimeStatus {
            session_id: sid.clone(),
            mcp,
            acp,
        });
    }
    Ok(out)
}
/// 当前 Agent 是否有进行中的 Run（页面刷新后恢复状态用）
#[derive(Debug, Serialize)]
pub struct AgentStatus {
    /// P3 兼容字段：任一会话在跑即 true（完整列表见 running_sessions）
    pub running: bool,
    /// 恰好一个会话在跑时返回它；多个/零个时为 None（完整列表见
    /// running_sessions，旧调用方不应再依赖本字段）
    pub session_id: Option<String>,
    /// 所有活跃 Run 的会话 id（P3 并发运行）
    pub running_sessions: Vec<String>,
    pub persistence_errors: HashMap<String, String>,
}

#[tauri::command]
pub fn agent_status(state: State<'_, AgentState>) -> AgentStatus {
    let active = state.active.lock().unwrap_or_else(|e| e.into_inner());
    let mut running_sessions: Vec<String> = active.keys().cloned().collect();
    running_sessions.sort();
    AgentStatus {
        running: !active.is_empty(),
        session_id: if active.len() == 1 {
            active.keys().next().cloned()
        } else {
            None
        },
        running_sessions,
        persistence_errors: state
            .persistence_errors
            .lock()
            .unwrap_or_else(|e| e.into_inner())
            .clone(),
    }
}
pub(crate) fn image_encoding(enabled: bool) -> agent_core::model::ImageEncoding {
    if enabled {
        agent_core::model::ImageEncoding::UserImageUrl
    } else {
        agent_core::model::ImageEncoding::Disabled
    }
}
