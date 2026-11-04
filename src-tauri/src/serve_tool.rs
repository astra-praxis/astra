//! Serve bot 工具（Agent 对话内管理 astra-serve 上的 Entity）。
//!
//! 用户口中的「bot」即 astra-serve 上的 Entity（长期 Agent 主体）。本组
//! 工具把 Serve 的 HTTP API 包装成 AgentTool，让桌面端 Agent 能在对话里：
//! - `serve_bot_list`   列出 bot 及运行状态（只读 Allow）
//! - `serve_bot_manage` 创建 / 启动 / 停止 / 暂停 / 恢复（Confirm）
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
    let client = reqwest::Client::builder()
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
            let id = e.get("id").and_then(|v| v.as_str()).unwrap_or("?");
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
         参数：action、id（create 时传 name，其余传 bot 的 id）、workspace（可选，bot 的工作目录；\
         create 缺省时自动用当前会话的项目根目录，让 bot 直接在用户选的项目里干活）"
    }
    fn parameters_schema(&self) -> Value {
        json!({
            "type": "object",
            "required": ["action"],
            "properties": {
                "action": { "type": "string", "enum": ["create", "start", "stop", "pause", "resume"] },
                "id": { "type": "string", "description": "bot id（start/stop/pause/resume 必填）" },
                "name": { "type": "string", "description": "bot 名称（create 必填）" },
                "workspace": { "type": "string", "description": "bot 工作目录（create 可选；缺省用当前项目根目录）" }
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
                let id = created.get("id").and_then(|v| v.as_str()).unwrap_or("?");
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
         用户说「让 bot 去做XX」「给 bot 安排个任务」时用，bot Runtime 必须已启动）；\
         action=report 查最新任务报告；action=cancel 取消当前任务。\
         注意：这是长期目标式任务（bot 自主执行），只是一句话问答请用 serve_bot_chat"
    }
    fn parameters_schema(&self) -> Value {
        json!({
            "type": "object",
            "required": ["action", "id"],
            "properties": {
                "action": { "type": "string", "enum": ["list", "assign", "report", "cancel"] },
                "id": { "type": "string", "description": "bot id" },
                "goal": { "type": "string", "description": "任务目标（assign 必填）" }
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
                let text = serde_json::to_string_pretty(&raw).unwrap_or_default();
                Ok(ToolOutput::from(format!(
                    "bot {id} 的任务：\n{}",
                    text.chars().take(2000).collect::<String>()
                )))
            }
            "assign" => {
                let goal = require(&args, self.name(), "goal")?;
                call_serve(
                    self.name(),
                    reqwest::Method::POST,
                    &base,
                    Some(json!({ "goal": goal, "idempotency_key": uuid::Uuid::new_v4().to_string() })),
                )
                .await?;
                Ok(ToolOutput::from(format!(
                    "任务已下发给 bot {id}：「{goal}」。它会自主执行；进度用 serve_bot_task action=list 查，\
                     报告用 action=report 查。"
                )))
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
    /// 订阅 SSE 事件流等本轮终态：run_completed / 提问 / 审批请求。
    ///
    /// 订阅用 `?after=0`：replay 帧带 `id:` 行，live 帧不带——以此区分
    /// 历史与本次新事件，避免“订阅前 run 已立刻失败（如未配模型）导致
    /// 永远等不到 run_completed”的竞态；历史帧一律忽略，只处理 live。
    async fn wait_turn(&self, id: &str, session: &str) -> std::result::Result<TurnEvent, String> {
        let token = read_token().map_err(|e| e.to_string())?;
        let url = format!(
            "{}/api/entities/{id}/api/events?token={}&after=0",
            base_url(),
            token
        );
        let client = reqwest::Client::new();
        let resp = client
            .get(&url)
            .timeout(Duration::from_secs(600))
            .send()
            .await
            .map_err(|e| format!("订阅事件流失败: {e}"))?;
        if !resp.status().is_success() {
            return Err(format!("事件流返回 {}", resp.status()));
        }
        let mut stream = resp.bytes_stream();
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
        call_serve(self.name(), reqwest::Method::POST, path, Some(body)).await?;
        match self.wait_turn(id, &format!("{id}-chat")).await {
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
                match self.wait_turn(&id, &session).await {
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
