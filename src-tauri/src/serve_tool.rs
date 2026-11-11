//! Serve bot 工具（Agent 对话内管理 astra-serve 上的 Entity）。
//!
//! 用户口中的「bot」即 astra-serve 上的 Entity（长期 Agent 主体）。本组
//! 工具把 Serve 的 HTTP API 包装成 AgentTool，让桌面端 Agent 能在对话里：
//! - `serve_bot_list`   列出 bot 及运行状态（只读 Allow）
//! - `serve_bot_manage` 创建 / 启动 / 停止 / 暂停 / 恢复 / 触发器管理（Confirm）
//! - `serve_bot_task`   下发任务 / 取消 / 查列表 / 查报告产物（Confirm）
//! - `serve_bot_chat`   给 bot 发消息并等待本轮回答（Confirm）
//!
//! 边界：桌面端只是 Client，执行全部发生在 Serve 进程；这里只发 HTTP。
//! 连接信息与 serve_console.rs 共用：`<app_data_dir>/serve/{token,port}`，
//! Serve 未运行时工具返回引导文案（提示先在「Agent 服务」页启动）。

use std::time::Duration;

use agent_core::tool::{ToolContext, ToolOutput};
use agent_core::{AgentTool, Error, Result, ToolCategory, ToolRisk};
use serde_json::{json, Value};

use crate::serve_console::DEFAULT_PORT;

// ===== 连接信息 =====

/// serve 数据目录（与 serve_console.rs 写入的一致，机器级不随账户）
fn serve_dir() -> std::path::PathBuf {
    dirs::data_dir()
        .unwrap_or_else(|| std::path::PathBuf::from("."))
        .join("com.astra.app/serve")
}

fn read_token() -> Result<String> {
    let path = serve_dir().join("token");
    let token = std::fs::read_to_string(&path)
        .ok()
        .map(|s| s.trim().to_string())
        .filter(|s| !s.is_empty());
    token.ok_or_else(|| {
        Error::tool(
            "serve_bot",
            "还没配置过 Serve token。请先在「Agent 服务」页启动一次服务（会自动生成 token），再重试。",
        )
    })
}

fn read_port() -> u16 {
    std::fs::read_to_string(serve_dir().join("port"))
        .ok()
        .and_then(|s| s.trim().parse().ok())
        .unwrap_or(DEFAULT_PORT)
}

fn base_url() -> String {
    format!("http://127.0.0.1:{}", read_port())
}

/// 统一 HTTP 调用：带 Bearer token，非 2xx 时把 Serve 的错误信息透出。
async fn call_serve(
    tool: &str,
    method: reqwest::Method,
    path: &str,
    body: Option<Value>,
) -> Result<Value> {
    let token = read_token()?;
    let url = format!("{}{}", base_url(), path);
    let client = crate::net_util::client_builder_for(&url)
        .timeout(Duration::from_secs(20))
        .build()
        .map_err(|e| Error::tool(tool, format!("HTTP 客户端创建失败: {e}")))?;
    let mut req = client.request(method, &url).bearer_auth(token);
    if let Some(b) = body {
        req = req.json(&b);
    }
    let resp = req.send().await.map_err(|e| {
        Error::tool(
            tool,
            format!("连不上 Serve（{url}）: {e}。请先在「Agent 服务」页启动服务。"),
        )
    })?;
    let status = resp.status();
    let text = resp.text().await.unwrap_or_default();
    if !status.is_success() && status != reqwest::StatusCode::ACCEPTED {
        let hint = match status.as_u16() {
            409 => "（Entity Runtime 未运行：先 serve_bot_manage action=start）",
            401 => "（token 不对：在「Agent 服务」页重新启动服务会刷新配置）",
            _ => "",
        };
        return Err(Error::tool(
            tool,
            format!(
                "Serve 返回 {status}{hint}: {}",
                text.chars().take(300).collect::<String>()
            ),
        ));
    }
    if text.trim().is_empty() {
        return Ok(json!({}));
    }
    serde_json::from_str(&text).map_err(|e| Error::tool(tool, format!("Serve 响应解析失败: {e}")))
}

fn arg_str(args: &Value, key: &str) -> Option<String> {
    args.get(key)
        .and_then(|v| v.as_str())
        .map(|s| s.trim().to_string())
        .filter(|s| !s.is_empty())
}

fn arg_num(args: &Value, key: &str) -> Option<u32> {
    args.get(key).and_then(|v| v.as_u64()).map(|v| v as u32)
}

fn require(args: &Value, tool: &str, key: &str) -> Result<String> {
    arg_str(args, key).ok_or_else(|| Error::tool(tool, format!("缺少必填参数 {key}")))
}

// ===== serve_bot_list =====

pub struct ServeBotList;

#[async_trait::async_trait]
impl AgentTool for ServeBotList {
    fn name(&self) -> &str {
        "serve_bot_list"
    }
    fn description(&self) -> &str {
        "列出本机 Astra Serve 上所有 bot（长期运行的 Agent 主体，即 Entity）：名称、运行状态、当前任务。\
         用户问「有哪些 bot / bot 什么状态」时先用它"
    }
    fn parameters_schema(&self) -> Value {
        json!({ "type": "object", "properties": {} })
    }
    fn category(&self) -> ToolCategory {
        ToolCategory::External
    }
    fn risk(&self) -> ToolRisk {
        ToolRisk::Low
    }
    async fn call(&self, _args: Value, _ctx: &ToolContext) -> Result<ToolOutput> {
        let raw = call_serve(self.name(), reqwest::Method::GET, "/api/entities", None).await?;
        let mut out = String::new();
        // 兼容 {entities:[...]} 与 [...] 两种包装
        let items = raw
            .get("entities")
            .and_then(|v| v.as_array())
            .cloned()
            .or_else(|| raw.as_array().cloned())
            .unwrap_or_default();
        if items.is_empty() {
            return Ok(ToolOutput::from(
                "Serve 上还没有 bot。可用 serve_bot_manage action=create 创建，\
                 例如用户说「新建一个 bot 帮我整理文档」。"
                    .to_string(),
            ));
        }
        out.push_str(&format!("Serve 上共 {} 个 bot：\n", items.len()));
        for e in &items {
            let id = e
                .get("entity_id")
                .or_else(|| e.get("id"))
                .and_then(|v| v.as_str())
                .unwrap_or("?");
            let name = e.get("name").and_then(|v| v.as_str()).unwrap_or(id);
            let state = e
                .pointer("/runtime/state")
                .and_then(|v| v.as_str())
                .or_else(|| e.get("status").and_then(|v| v.as_str()))
                .unwrap_or("unknown");
            let task = e
                .pointer("/runtime/current_task")
                .and_then(|v| v.as_str())
                .unwrap_or("");
            out.push_str(&format!("- {name} (id={id}) 状态={state}"));
            if !task.is_empty() {
                out.push_str(&format!(" 当前任务={task}"));
            }
            out.push('\n');
        }
        Ok(ToolOutput::from(out))
    }
}

// ===== serve_bot_manage =====

pub struct ServeBotManage;

#[async_trait::async_trait]
impl AgentTool for ServeBotManage {
    fn name(&self) -> &str {
        "serve_bot_manage"
    }
    fn description(&self) -> &str {
        "管理本机 Serve 上的 bot。action=create 新建（参数 name；用户说「新建一个 bot 来做XX」时用，\
         XX 可并入 name）；action=start 启动它的 Runtime（必须先启动才能对话/接任务）；\
         action=stop 停止 Runtime（历史保留）；action=pause 暂停（拒绝新任务）；action=resume 恢复。\
         触发器管理（bot 无人值守自动触发，触发后走任务流）：action=trigger_list 列出；\
         action=trigger_add 添加/覆盖（kind=schedule 用 every_seconds 或 daily_at；\
         kind=file_watch 用 dir+patterns，mission 是触发后下发的任务模板，支持 {{字段}} 占位符）；\
         action=trigger_remove 删除；action=trigger_fire 手动触发（可选 payload，\
         body 即渲染数据本身，不要包 {\"payload\":...}）。\
         参数：action、id（create 时传 name，其余传 bot 的 id）、workspace（可选，bot 的工作目录；\
         create 缺省时自动用当前会话的项目根目录，让 bot 直接在用户选的项目里干活）"
    }
    fn parameters_schema(&self) -> Value {
        json!({
            "type": "object",
            "required": ["action"],
            "properties": {
                "action": { "type": "string", "enum": ["create", "start", "stop", "pause", "resume", "trigger_list", "trigger_add", "trigger_remove", "trigger_fire"] },
                "id": { "type": "string", "description": "bot id（create 时传 name，其余必填）" },
                "name": { "type": "string", "description": "bot 名称（create 必填）" },
                "workspace": { "type": "string", "description": "bot 工作目录（create 可选；缺省用当前项目根目录）" },
                "trigger_id": { "type": "string", "description": "触发器 id（trigger_add/remove/fire 必填）" },
                "kind": { "type": "string", "enum": ["schedule", "file_watch"], "description": "触发器类型（trigger_add 必填）" },
                "mission": { "type": "string", "description": "触发后下发给 bot 的任务模板（trigger_add 必填），支持 {{payload字段}}/{{fire_time}} 占位符" },
                "every_seconds": { "type": "integer", "description": "schedule：间隔秒数（与 daily_at 二选一）" },
                "daily_at": { "type": "string", "description": "schedule：每天固定时间 HH:MM（24 小时制，与 every_seconds 二选一）" },
                "dir": { "type": "string", "description": "file_watch：监听目录（必填）" },
                "patterns": { "type": "array", "items": { "type": "string" }, "description": "file_watch：文件后缀 glob，如 [\"*.txt\"]（缺省 [\"*\"]）" },
                "payload": { "type": "object", "description": "trigger_fire 可选：模板渲染数据，body 即它本身" }
            }
        })
    }
    fn category(&self) -> ToolCategory {
        ToolCategory::External
    }
    fn risk(&self) -> ToolRisk {
        ToolRisk::Medium
    }
    async fn call(&self, args: Value, ctx: &ToolContext) -> Result<ToolOutput> {
        let action = require(&args, self.name(), "action")?;
        match action.as_str() {
            "create" => {
                let name = require(&args, self.name(), "name")?;
                // 工作空间优先级：显式 workspace 参数 > Agent 页当前选中的项目根 >
                // 默认 <serve 数据目录>/workspaces/<name>。bot 应该在用户的项目里
                // 干活，而不是在以它名字命名的隔离目录里（会导致工具全部越权）。
                let explicit = args
                    .get("workspace")
                    .and_then(|v| v.as_str())
                    .map(std::path::PathBuf::from)
                    .filter(|p| p.is_dir());
                let ws = match explicit
                    .or_else(|| ctx.project_root.clone())
                    .filter(|p| p.is_dir())
                {
                    Some(p) => p,
                    None => {
                        let slug: String = name
                            .chars()
                            .map(|c| {
                                if c.is_alphanumeric() || matches!(c, '-' | '_' | '.') {
                                    c
                                } else {
                                    '-'
                                }
                            })
                            .collect();
                        let slug = slug.trim_matches('-');
                        let slug = if slug.is_empty() { "bot" } else { slug };
                        let ws = serve_dir().join("workspaces").join(slug);
                        std::fs::create_dir_all(&ws).map_err(|e| {
                            Error::tool(self.name(), format!("创建工作空间失败: {e}"))
                        })?;
                        ws
                    }
                };
                let created = call_serve(
                    self.name(),
                    reqwest::Method::POST,
                    "/api/entities",
                    Some(json!({
                        "name": name,
                        "runtime": { "workspace": ws.display().to_string() }
                    })),
                )
                .await?;
                let id = created
                    .get("entity_id")
                    .or_else(|| created.get("id"))
                    .and_then(|v| v.as_str())
                    .unwrap_or("?");
                Ok(ToolOutput::from(format!(
                    "已创建 bot「{name}」（id={id}），工作空间 {}，模型来源沿用桌面端默认。\
                     它还不能对话：先用 serve_bot_manage action=start id={id} 启动 Runtime。",
                    ws.display()
                )))
            }
            "start" => {
                let id = require(&args, self.name(), "id")?;
                call_serve(
                    self.name(),
                    reqwest::Method::POST,
                    &format!("/api/entities/{id}/runtime/start"),
                    Some(json!({})),
                )
                .await?;
                Ok(ToolOutput::from(format!(
                    "bot {id} 的 Runtime 已启动，可以对话（serve_bot_chat）或下发任务（serve_bot_task action=assign）。"
                )))
            }
            "stop" => {
                let id = require(&args, self.name(), "id")?;
                call_serve(
                    self.name(),
                    reqwest::Method::POST,
                    &format!("/api/entities/{id}/runtime/stop"),
                    Some(json!({})),
                )
                .await?;
                Ok(ToolOutput::from(format!(
                    "bot {id} 的 Runtime 已停止。bot 和历史都保留，随时可重新 start。"
                )))
            }
            "pause" | "resume" => {
                let id = require(&args, self.name(), "id")?;
                call_serve(
                    self.name(),
                    reqwest::Method::POST,
                    &format!(
                        "/api/entities/{id}/{}",
                        if action == "pause" { "pause" } else { "resume" }
                    ),
                    Some(json!({})),
                )
                .await?;
                Ok(ToolOutput::from(format!(
                    "bot {id} 已{}。",
                    if action == "pause" {
                        "暂停（不再接受新任务和新对话，直到 resume）"
                    } else {
                        "恢复"
                    }
                )))
            }
            "trigger_list" => {
                let id = require(&args, self.name(), "id")?;
                let raw = call_serve(
                    self.name(),
                    reqwest::Method::GET,
                    &format!("/api/entities/{id}/triggers"),
                    None,
                )
                .await?;
                let mut out = format!("bot {id} 的触发器：\n");
                let triggers = raw
                    .get("triggers")
                    .and_then(|v| v.as_array())
                    .cloned()
                    .unwrap_or_default();
                if triggers.is_empty() {
                    out.push_str("（无）\n");
                }
                for t in &triggers {
                    let tid = t.get("id").and_then(|v| v.as_str()).unwrap_or("?");
                    let kind = t.get("kind").and_then(|v| v.as_str()).unwrap_or("?");
                    let enabled = t.get("enabled").and_then(|v| v.as_bool()).unwrap_or(false);
                    let mission = t
                        .get("mission_template")
                        .and_then(|v| v.as_str())
                        .unwrap_or("");
                    let config = t.get("config").cloned().unwrap_or(serde_json::json!({}));
                    out.push_str(&format!(
                        "- {tid}（{kind}，{}）：{mission}；config={config}\n",
                        if enabled { "已启用" } else { "已禁用" }
                    ));
                }
                // 忙时入队、待补跑的触发器事件（问题 10：入队要有可见性）
                let queue = raw
                    .get("queue")
                    .and_then(|v| v.as_array())
                    .cloned()
                    .unwrap_or_default();
                if !queue.is_empty() {
                    out.push_str(&format!(
                        "\n排队中的触发器事件（{} 条，忙时入队待补跑）：\n",
                        queue.len()
                    ));
                    for q in queue.iter().take(10) {
                        let tid = q.get("trigger_id").and_then(|v| v.as_str()).unwrap_or("?");
                        let title = q.get("title").and_then(|v| v.as_str()).unwrap_or("");
                        out.push_str(&format!("- {tid}: {title}\n"));
                    }
                }
                Ok(ToolOutput::from(out))
            }
            "trigger_add" => {
                let id = require(&args, self.name(), "id")?;
                let trigger_id = require(&args, self.name(), "trigger_id")?;
                let kind = require(&args, self.name(), "kind")?;
                let mission = require(&args, self.name(), "mission")?;
                let config = match kind.as_str() {
                    "schedule" => {
                        let every = args.get("every_seconds").and_then(|v| v.as_u64());
                        let daily = arg_str(&args, "daily_at");
                        match (every, daily) {
                            (Some(s), _) => json!({ "every_seconds": s }),
                            (None, Some(t)) => json!({ "daily_at": t }),
                            (None, None) => {
                                return Err(Error::tool(
                                    self.name(),
                                    "schedule 触发器需要 every_seconds 或 daily_at 之一",
                                ))
                            }
                        }
                    }
                    "file_watch" => {
                        let dir = require(&args, self.name(), "dir")?;
                        let patterns: Vec<String> = args
                            .get("patterns")
                            .and_then(|v| v.as_array())
                            .map(|a| {
                                a.iter()
                                    .filter_map(|p| p.as_str().map(String::from))
                                    .collect()
                            })
                            .unwrap_or_else(|| vec!["*".to_string()]);
                        json!({ "dir": dir, "patterns": patterns })
                    }
                    other => {
                        return Err(Error::tool(
                            self.name(),
                            format!("未知触发器类型: {other}（支持 schedule / file_watch）"),
                        ))
                    }
                };
                // PUT 是整组替换：先读现有集合，同 id 覆盖、否则追加。
                let existing = call_serve(
                    self.name(),
                    reqwest::Method::GET,
                    &format!("/api/entities/{id}/triggers"),
                    None,
                )
                .await?;
                let mut triggers = existing
                    .get("triggers")
                    .and_then(|v| v.as_array())
                    .cloned()
                    .unwrap_or_default();
                let spec = json!({
                    "id": trigger_id,
                    "kind": kind,
                    "enabled": true,
                    "mission_template": mission,
                    "config": config,
                });
                if let Some(pos) = triggers
                    .iter()
                    .position(|t| t.get("id").and_then(|v| v.as_str()) == Some(trigger_id.as_str()))
                {
                    triggers[pos] = spec;
                } else {
                    triggers.push(spec);
                }
                let resp = call_serve(
                    self.name(),
                    reqwest::Method::PUT,
                    &format!("/api/entities/{id}/triggers"),
                    Some(json!({ "triggers": triggers })),
                )
                .await?;
                let mut msg = format!(
                    "bot {id} 的触发器 {trigger_id}（{kind}）已保存并生效，触发后走任务流：\
                     用 serve_bot_task action=list 查进度，action=report 查产物。"
                );
                if let Some(warnings) = resp.get("warnings").and_then(|v| v.as_array()) {
                    for w in warnings {
                        if let Some(s) = w.as_str() {
                            msg.push_str(&format!("\n注意：{s}"));
                        }
                    }
                }
                Ok(ToolOutput::from(msg))
            }
            "trigger_remove" => {
                let id = require(&args, self.name(), "id")?;
                let trigger_id = require(&args, self.name(), "trigger_id")?;
                let existing = call_serve(
                    self.name(),
                    reqwest::Method::GET,
                    &format!("/api/entities/{id}/triggers"),
                    None,
                )
                .await?;
                let triggers: Vec<Value> = existing
                    .get("triggers")
                    .and_then(|v| v.as_array())
                    .cloned()
                    .unwrap_or_default()
                    .into_iter()
                    .filter(|t| t.get("id").and_then(|v| v.as_str()) != Some(trigger_id.as_str()))
                    .collect();
                call_serve(
                    self.name(),
                    reqwest::Method::PUT,
                    &format!("/api/entities/{id}/triggers"),
                    Some(json!({ "triggers": triggers })),
                )
                .await?;
                Ok(ToolOutput::from(format!(
                    "bot {id} 的触发器 {trigger_id} 已删除。"
                )))
            }
            "trigger_fire" => {
                let id = require(&args, self.name(), "id")?;
                let trigger_id = require(&args, self.name(), "trigger_id")?;
                let payload = args.get("payload").cloned().unwrap_or_else(|| json!({}));
                call_serve(
                    self.name(),
                    reqwest::Method::POST,
                    &format!("/api/entities/{id}/triggers/{trigger_id}/fire"),
                    Some(payload),
                )
                .await?;
                Ok(ToolOutput::from(format!(
                    "触发器 {trigger_id} 已受理（202）。fire 是异步的：任务创建后 bot 忙会先入队，\
                     结果用 serve_bot_task action=list / report 查。"
                )))
            }
            other => Err(Error::tool(self.name(), format!("未知 action: {other}"))),
        }
    }
}

// ===== serve_bot_task =====

pub struct ServeBotTask;

#[async_trait::async_trait]
impl AgentTool for ServeBotTask {
    fn name(&self) -> &str {
        "serve_bot_task"
    }
    fn description(&self) -> &str {
        "管理 bot 的任务（Task）。action=list 列任务；action=assign 给 bot 下发新目标（参数 id + goal，\
         用户说「让 bot 去做XX」「给 bot 安排个任务」时用，bot Runtime 必须已启动，\
         停止态会提示先 start）；action=report 查最新任务报告；action=cancel 取消当前任务。\
         可选：max_turns_per_action / max_actions 收紧执行预算（mission 模糊时防长时间空转）。\
         注意：这是长期目标式任务（bot 自主执行），只是一句话问答请用 serve_bot_chat"
    }
    fn parameters_schema(&self) -> Value {
        json!({
            "type": "object",
            "required": ["action", "id"],
            "properties": {
                "action": { "type": "string", "enum": ["list", "assign", "report", "cancel"] },
                "id": { "type": "string", "description": "bot id" },
                "goal": { "type": "string", "description": "任务目标/完整要求（assign 必填）" },
                "title": { "type": "string", "description": "任务标题（assign 可选；缺省取 goal 首行前 40 字）" },
                "max_turns_per_action": { "type": "integer", "description": "assign 可选：单 Action 轮数上限（默认 50，模糊任务可降到 10-20 防空转）" },
                "max_actions": { "type": "integer", "description": "assign 可选：整任务 Action 数上限（默认 12）" }
            }
        })
    }
    fn category(&self) -> ToolCategory {
        ToolCategory::External
    }
    fn risk(&self) -> ToolRisk {
        ToolRisk::Medium
    }
    async fn call(&self, args: Value, _ctx: &ToolContext) -> Result<ToolOutput> {
        let action = require(&args, self.name(), "action")?;
        let id = require(&args, self.name(), "id")?;
        let base = format!("/api/entities/{id}/tasks");
        match action.as_str() {
            "list" => {
                let raw = call_serve(self.name(), reqwest::Method::GET, &base, None).await?;
                let tasks = raw
                    .get("tasks")
                    .and_then(|v| v.as_array())
                    .cloned()
                    .unwrap_or_default();
                if tasks.is_empty() {
                    return Ok(ToolOutput::from(format!("bot {id} 目前没有任务记录。")));
                }
                // 结构化摘要而非原始 JSON：单条任务就足以撑爆截断窗口，
                // 半截 JSON 还会误导模型。
                let mut out = format!("bot {id} 的任务（共 {} 条，新→旧）：\n", tasks.len());
                for t in tasks.iter().take(10) {
                    let goal_id = t.get("goal_id").and_then(|v| v.as_str()).unwrap_or("?");
                    let status = t.get("status").and_then(|v| v.as_str()).unwrap_or("?");
                    let title = t
                        .pointer("/goal/title")
                        .and_then(|v| v.as_str())
                        .unwrap_or("");
                    let reason = t
                        .get("status_reason")
                        .and_then(|v| v.as_str())
                        .unwrap_or("");
                    out.push_str(&format!("- {goal_id} [{status}] {title}"));
                    if !reason.is_empty() {
                        out.push_str(&format!("（{reason}）"));
                    }
                    out.push('\n');
                }
                if tasks.len() > 10 {
                    out.push_str(&format!("（其余 {} 条略）\n", tasks.len() - 10));
                }
                out.push_str("列表第一条是当前任务；报告用 action=report。");
                Ok(ToolOutput::from(out))
            }
            "assign" => {
                let goal = require(&args, self.name(), "goal")?;
                // 接口必填 title + mission：title 缺省从 goal 首行派生。
                let title = arg_str(&args, "title").unwrap_or_else(|| {
                    let first_line = goal.lines().next().unwrap_or(&goal).trim();
                    let mut t: String = first_line.chars().take(40).collect();
                    if first_line.chars().count() > 40 {
                        t.push('…');
                    }
                    t
                });
                let resp = call_serve(
                    self.name(),
                    reqwest::Method::POST,
                    &base,
                    Some(json!({
                        "title": title,
                        "mission": goal,
                        "idempotency_key": uuid::Uuid::new_v4().to_string(),
                        "budget": {
                            "per_action_max_turns": arg_num(&args, "max_turns_per_action"),
                            "max_actions": arg_num(&args, "max_actions"),
                        }
                    })),
                )
                .await?;
                // Serve 对 stopped 的 bot 会落盘返回 200（runtime_running=false），
                // 任务要等启动后才执行——必须如实告知，否则像「假成功卡死」。
                let running = resp
                    .get("runtime_running")
                    .and_then(|v| v.as_bool())
                    .unwrap_or(true);
                if running {
                    Ok(ToolOutput::from(format!(
                        "任务已下发给 bot {id}：「{goal}」。它会自主执行；进度用 serve_bot_task action=list 查，\
                         报告用 action=report 查。"
                    )))
                } else {
                    Ok(ToolOutput::from(format!(
                        "任务已保存但尚未执行：bot {id} 的 Runtime 当前是停止状态。\
                         先用 serve_bot_manage action=start id={id} 启动，启动后任务会自动继续。"
                    )))
                }
            }
            "report" => {
                let raw = call_serve(
                    self.name(),
                    reqwest::Method::GET,
                    &format!("{base}/report"),
                    None,
                )
                .await?;
                let text = serde_json::to_string_pretty(&raw).unwrap_or_default();
                Ok(ToolOutput::from(format!(
                    "bot {id} 的任务报告：\n{}",
                    text.chars().take(3000).collect::<String>()
                )))
            }
            "cancel" => {
                call_serve(
                    self.name(),
                    reqwest::Method::POST,
                    &format!("{base}/cancel"),
                    Some(json!({})),
                )
                .await?;
                Ok(ToolOutput::from(format!(
                    "已请求取消 bot {id} 的当前任务。"
                )))
            }
            other => Err(Error::tool(self.name(), format!("未知 action: {other}"))),
        }
    }
}

// ===== serve_bot_chat =====

pub struct ServeBotChat;

/// 等待本轮回答/提问/审批的最长时间（对话式问答一般几十秒；超时引导去控制台看）
const CHAT_WAIT: Duration = Duration::from_secs(180);

/// wait_turn 的中途事件：本轮尚未结束，但 bot 在等用户输入。
#[derive(Debug)]
enum TurnEvent {
    /// 本轮结束，拿到最终回答（或失败/取消原因）
    Done(std::result::Result<String, String>),
    /// bot 提了问题（ask_user），需要用户回答
    Asked {
        question_id: String,
        question: String,
        options: Vec<String>,
        allow_free_text: bool,
    },
    /// bot 在等权限审批
    NeedsApproval {
        tool_call_id: String,
        tool_name: String,
        summary: String,
    },
}

impl ServeBotChat {
    /// 先于业务请求建立 SSE 订阅。必须赶在 POST 之前打开流：否则本轮
    /// 秒级失败（如单运行锁冲突、未配模型）的终态事件会先落进事件日志，
    /// 订阅后回放时带 `id:` 行被当历史丢掉，工具白等满超时。
    async fn subscribe_events(&self, id: &str) -> std::result::Result<reqwest::Response, String> {
        let token = read_token().map_err(|e| e.to_string())?;
        let url = format!(
            "{}/api/entities/{id}/api/events?token={}&after=0",
            base_url(),
            token
        );
        let client = crate::net_util::client_builder_for(&url)
            .build()
            .unwrap_or_default();
        let resp = client
            .get(&url)
            .timeout(Duration::from_secs(600))
            .send()
            .await
            .map_err(|e| format!("订阅事件流失败: {e}"))?;
        if !resp.status().is_success() {
            return Err(format!("事件流返回 {}", resp.status()));
        }
        Ok(resp)
    }

    /// 在已建立的流上等本轮终态：run_completed / 提问 / 审批请求。
    ///
    /// replay 帧带 `id:` 行（订阅前已发生的历史），一律跳过；live 帧
    /// 不带 `id:`，必然是订阅之后发生的事件。调用方必须先 `subscribe_events`
    /// 再发业务 POST，这样本轮产生的任何事件（含秒级失败）都是 live 帧。
    async fn read_turn(
        stream: reqwest::Response,
        session: &str,
    ) -> std::result::Result<TurnEvent, String> {
        let mut stream = stream.bytes_stream();
        let mut buf = Vec::<u8>::new();
        let deadline = tokio::time::Instant::now() + CHAT_WAIT;
        use futures_util::StreamExt;
        loop {
            let Some(remaining) = deadline.checked_duration_since(tokio::time::Instant::now())
            else {
                return Err(
                    "等待 bot 回应超时（3 分钟）。可能任务较长或卡在审批，请到 Serve 控制台查看。"
                        .into(),
                );
            };
            let chunk = match tokio::time::timeout(remaining, stream.next()).await {
                Ok(Some(Ok(bytes))) => bytes,
                Ok(Some(Err(e))) => return Err(format!("事件流中断: {e}")),
                Ok(None) => return Err("事件流已关闭但未等到回应，请到 Serve 控制台查看。".into()),
                Err(_) => return Err(
                    "等待 bot 回应超时（3 分钟）。可能任务较长或卡在审批，请到 Serve 控制台查看。"
                        .into(),
                ),
            };
            buf.extend_from_slice(&chunk);
            // SSE 帧以空行分隔；逐帧解析
            while let Some(pos) = find_frame(&buf) {
                let frame: Vec<u8> = buf.drain(..pos).collect();
                let text = String::from_utf8_lossy(&frame);
                // replay 帧带 id 行（历史事件），live 帧没有——只处理 live
                let is_replay = text.lines().any(|l| l.starts_with("id: "));
                if is_replay {
                    continue;
                }
                if let Some(event) = parse_sse_data(&frame) {
                    let Ok(ev) = serde_json::from_str::<Value>(&event) else {
                        continue;
                    };
                    if let Some(te) = classify_event(&ev, session) {
                        return Ok(te);
                    }
                }
            }
            if buf.len() > 4 * 1024 * 1024 {
                buf.clear(); // 防御：重放事件过大时丢弃已解析前的内容
            }
        }
    }

    /// 决策（回答问题 / 权限审批）后继续等本轮终态，拿到最终回答。
    async fn decide_and_wait(
        &self,
        id: &str,
        path: &str,
        body: Value,
        what: &str,
    ) -> Result<ToolOutput> {
        // 先订阅再提交决策：决策后的终态事件（含秒级失败）不会错过。
        let stream = self
            .subscribe_events(id)
            .await
            .map_err(|e| Error::tool(self.name(), e))?;
        call_serve(self.name(), reqwest::Method::POST, path, Some(body)).await?;
        match Self::read_turn(stream, &format!("{id}-chat")).await {
            Ok(TurnEvent::Done(Ok(answer))) => Ok(ToolOutput::from(format!(
                "{what}已提交。bot 继续执行后的回答：\n{answer}"
            ))),
            Ok(TurnEvent::Done(Err(reason))) => {
                Err(Error::tool(self.name(), format!("{what}已提交，但 bot 本轮失败：{reason}")))
            }
            Ok(TurnEvent::Asked { question, .. }) => Ok(ToolOutput::from(format!(
                "{what}已提交。bot 又提了新问题：{question}\n请向用户转述后用 serve_bot_chat action=answer 回答。"
            ))),
            Ok(TurnEvent::NeedsApproval { tool_name, summary, .. }) => Ok(ToolOutput::from(format!(
                "{what}已提交。bot 又在等权限审批：{tool_name} {summary}\n可用 serve_bot_chat action=approve 处理。"
            ))),
            Err(e) => Err(Error::tool(self.name(), e)),
        }
    }
}

/// 从一帧 SSE 事件里识别本轮终态/中途事件；其余返回 None。
fn classify_event(ev: &Value, session: &str) -> Option<TurnEvent> {
    let kind = ev.get("kind").and_then(|v| v.as_str()).unwrap_or("");
    let ev_session = ev.get("session_id").and_then(|v| v.as_str()).unwrap_or("");
    let session_matches = ev_session.is_empty() || ev_session == session;
    match kind {
        "run_completed" if session_matches => {
            let failed = ev.get("failed").and_then(|v| v.as_bool()).unwrap_or(false);
            let cancelled = ev
                .get("cancelled")
                .and_then(|v| v.as_bool())
                .unwrap_or(false);
            if failed {
                let reason = ev
                    .get("failure_reason")
                    .and_then(|v| v.as_str())
                    .unwrap_or("执行失败");
                Some(TurnEvent::Done(Err(format!("bot 执行失败：{reason}"))))
            } else if cancelled {
                Some(TurnEvent::Done(Err("bot 本轮执行被取消。".into())))
            } else {
                let final_text = ev.get("final_text").and_then(|v| v.as_str()).unwrap_or("");
                Some(TurnEvent::Done(Ok(if final_text.is_empty() {
                    "bot 已完成本轮，但没有返回文字（可能有工具产物，见 Serve 控制台）。"
                        .to_string()
                } else {
                    final_text.to_string()
                })))
            }
        }
        "question" if session_matches => Some(TurnEvent::Asked {
            question_id: ev
                .get("question_id")
                .and_then(|v| v.as_str())
                .unwrap_or("")
                .to_string(),
            question: ev
                .get("question")
                .and_then(|v| v.as_str())
                .unwrap_or("")
                .to_string(),
            options: ev
                .get("options")
                .and_then(|v| v.as_array())
                .map(|a| {
                    a.iter()
                        .filter_map(|o| o.as_str().map(str::to_string))
                        .collect()
                })
                .unwrap_or_default(),
            allow_free_text: ev
                .get("allow_free_text")
                .and_then(|v| v.as_bool())
                .unwrap_or(false),
        }),
        "permission_requested" if session_matches => Some(TurnEvent::NeedsApproval {
            tool_call_id: ev
                .get("tool_call_id")
                .and_then(|v| v.as_str())
                .unwrap_or("")
                .to_string(),
            tool_name: ev
                .get("tool_name")
                .and_then(|v| v.as_str())
                .unwrap_or("")
                .to_string(),
            summary: ev
                .get("summary")
                .and_then(|v| v.as_str())
                .unwrap_or("")
                .to_string(),
        }),
        _ => None,
    }
}

fn find_frame(buf: &[u8]) -> Option<usize> {
    buf.windows(2).position(|w| w == b"\n\n").map(|p| p + 2)
}

/// 取帧内 data: 行拼接的内容
fn parse_sse_data(frame: &[u8]) -> Option<String> {
    let text = String::from_utf8_lossy(frame);
    let data: Vec<&str> = text
        .lines()
        .filter_map(|l| l.strip_prefix("data: "))
        .collect();
    if data.is_empty() {
        None
    } else {
        Some(data.join("\n"))
    }
}

#[async_trait::async_trait]
impl AgentTool for ServeBotChat {
    fn name(&self) -> &str {
        "serve_bot_chat"
    }
    fn description(&self) -> &str {
        "与 bot 对话。默认 action=send：发消息并等本轮回答（最多 3 分钟）。\n\
         若 bot 中途提问（ask_user），会返回问题卡（含 question_id），请向用户转述，\n\
         用户回答后用 action=answer question_id=... text=... 转达并继续等回答；\n\
         若 bot 在等权限审批，可用 action=approve（同意）或 action=deny（拒绝）处理，\n\
         之后会继续等本轮结果。长期目标式任务请用 serve_bot_task action=assign。"
    }
    fn parameters_schema(&self) -> Value {
        json!({
            "type": "object",
            "required": ["id"],
            "properties": {
                "action": {
                    "type": "string",
                    "enum": ["send", "answer", "approve", "deny"],
                    "description": "send=发消息（默认）；answer=回答 bot 的提问（需 question_id+text）；approve=同意权限审批（需 tool_call_id）；deny=拒绝权限审批（需 tool_call_id）"
                },
                "id": { "type": "string", "description": "bot id" },
                "text": { "type": "string", "description": "send 的消息内容 / answer 的回答内容" },
                "question_id": { "type": "string", "description": "answer 时必填：要回答的问题 id" },
                "tool_call_id": { "type": "string", "description": "approve/deny 时必填：待审批的工具调用 id" }
            }
        })
    }
    fn category(&self) -> ToolCategory {
        ToolCategory::External
    }
    fn risk(&self) -> ToolRisk {
        ToolRisk::Medium
    }
    async fn call(&self, args: Value, _ctx: &ToolContext) -> Result<ToolOutput> {
        let id = require(&args, self.name(), "id")?;
        let action = arg_str(&args, "action").unwrap_or_else(|| "send".into());
        match action.as_str() {
            "send" => {
                let text = require(&args, self.name(), "text")?;
                // 先订阅再发消息：保证本轮产生的任何事件（含秒级失败，
                // 如单运行锁冲突）都是 live 帧，不会被回放过滤掉。
                let stream = self
                    .subscribe_events(&id)
                    .await
                    .map_err(|e| Error::tool(self.name(), e))?;
                let accepted = call_serve(
                    self.name(),
                    reqwest::Method::POST,
                    &format!("/api/entities/{id}/api/chat"),
                    Some(json!({ "text": text })),
                )
                .await?;
                let session = accepted
                    .get("session_id")
                    .and_then(|v| v.as_str())
                    .unwrap_or("")
                    .to_string();
                if session.is_empty() {
                    return Ok(ToolOutput::from(
                        "消息已受理，但未返回会话 id，无法等待回答；请到 Serve 控制台查看。"
                            .to_string(),
                    ));
                }
                match Self::read_turn(stream, &session).await {
                    Ok(TurnEvent::Done(Ok(answer))) => {
                        Ok(ToolOutput::from(format!("bot 的回答：\n{answer}")))
                    }
                    Ok(TurnEvent::Done(Err(reason))) => Err(Error::tool(self.name(), reason)),
                    Ok(TurnEvent::Asked { question_id, question, options, allow_free_text }) => {
                        let opts = if options.is_empty() {
                            String::new()
                        } else {
                            format!("\n可选选项：{}", options.join(" / "))
                        };
                        Ok(ToolOutput::from(format!(
                            "bot 在执行中向你（用户）提出了问题，本轮还未结束：\n{question}{opts}{}\n\
                             请把问题转述给用户；拿到回答后调用 serve_bot_chat action=answer id={id} \
                             question_id={question_id} text=<用户回答> 转达并继续等 bot 的后续回答。",
                            if allow_free_text { "" } else { "\n（不接受自由文本，请让用户从选项中选）" }
                        )))
                    }
                    Ok(TurnEvent::NeedsApproval { tool_call_id, tool_name, summary }) => {
                        Ok(ToolOutput::from(format!(
                            "bot 在执行中需要权限审批，本轮暂停等待：\n工具：{tool_name}\n内容：{summary}\n\
                             请询问用户是否同意，然后用 serve_bot_chat action=approve id={id} \
                             tool_call_id={tool_call_id} 同意，或 action=deny 拒绝。"
                        )))
                    }
                    Err(e) => Err(Error::tool(self.name(), e)),
                }
            }
            "answer" => {
                let question_id = require(&args, self.name(), "question_id")?;
                let text = require(&args, self.name(), "text")?;
                self.decide_and_wait(
                    &id,
                    &format!("/api/entities/{id}/api/questions/{question_id}"),
                    json!({ "answer": text }),
                    "回答",
                )
                .await
            }
            "approve" | "deny" => {
                let tool_call_id = require(&args, self.name(), "tool_call_id")?;
                let decision = if action == "approve" { "allow" } else { "deny" };
                self.decide_and_wait(
                    &id,
                    &format!("/api/entities/{id}/api/permissions/{tool_call_id}"),
                    json!({ "decision": decision }),
                    if action == "approve" {
                        "同意审批"
                    } else {
                        "拒绝审批"
                    },
                )
                .await
            }
            other => Err(Error::tool(self.name(), format!("未知 action: {other}"))),
        }
    }
}
