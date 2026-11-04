//! Serve bot 反向通道：Serve 上 bot 的待办（提问/审批）、任务终态、Runtime
//! 异常 → 主动推给桌面端前端（Tauri 事件 `serve-bot-event`）。
//!
//! 边界：桌面端只是 Client，这里只轮询 Serve 的 HTTP 快照接口做差量检测，
//! 不在桌面进程内订阅执行。轮询而非 SSE 长连：多 bot 连接管理 + 断线重连
//! 的复杂度在 V1 不值得，6s 轮询对「通知」语义足够。
//!
//! 事件消费方在前端（Agent 页）：收到后把通知追加进当前会话，让用户在
//! 对话里直接看到「XX bot 在等回答 / 任务完成」。

use std::collections::{HashMap, HashSet};
use std::time::Duration;

use serde::Serialize;
use serde_json::Value;
use tauri::{AppHandle, Emitter};

use crate::serve_console::DEFAULT_PORT;

/// 轮询间隔
const POLL_INTERVAL: Duration = Duration::from_secs(6);
/// Serve 不可达时的重试间隔（更快恢复感知）
const RETRY_INTERVAL: Duration = Duration::from_secs(3);

/// 推给前端的事件载荷（前端 ServeBotEvent 类型对应）
#[derive(Debug, Clone, Serialize)]
#[serde(rename_all = "snake_case", tag = "kind")]
pub(crate) enum BotEvent {
    /// bot 提了问题，等用户回答（回答走 serve_bot_chat action=answer）
    Question {
        entity_id: String,
        entity_name: String,
        question_id: String,
        question: String,
        options: Vec<String>,
        allow_free_text: bool,
    },
    /// bot 在等权限审批（批准/拒绝走 serve_bot_chat action=approve/deny）
    Permission {
        entity_id: String,
        entity_name: String,
        tool_call_id: String,
        tool_name: String,
        summary: String,
    },
    /// 任务到达终态
    TaskFinished {
        entity_id: String,
        entity_name: String,
        task_id: String,
        goal: String,
        status: String,
        /// 终态是否成功（Achieved）
        ok: bool,
    },
    /// Runtime 异常退出
    RuntimeFailed {
        entity_id: String,
        entity_name: String,
        detail: Option<String>,
    },
}

#[derive(Default)]
struct WatchState {
    /// 已通知过的 question_id（防重复推送）
    questions: HashSet<String>,
    /// 已通知过的 tool_call_id
    permissions: HashSet<String>,
    /// 上次看到的任务终态（None = 无任务/未终态；Some = 已通知过该终态）
    task_terminal: HashMap<String, String>,
    /// 上次看到的 runtime state（检测 → failed 转变）
    runtime_state: HashMap<String, String>,
}

fn serve_dir() -> std::path::PathBuf {
    dirs::data_dir()
        .unwrap_or_else(|| std::path::PathBuf::from("."))
        .join("com.astra.app/serve")
}

fn read_token() -> Option<String> {
    std::fs::read_to_string(serve_dir().join("token"))
        .ok()
        .map(|s| s.trim().to_string())
        .filter(|s| !s.is_empty())
}

fn read_port() -> u16 {
    std::fs::read_to_string(serve_dir().join("port"))
        .ok()
        .and_then(|s| s.trim().parse().ok())
        .unwrap_or(DEFAULT_PORT)
}

/// 启动常驻轮询任务（setup 时调用一次）。失败只记日志，不阻塞应用。
pub(crate) fn spawn(app: AppHandle) {
    tauri::async_runtime::spawn(async move {
        watch_loop(app).await;
    });
}

async fn watch_loop(app: AppHandle) {
    let client = match reqwest::Client::builder()
        .timeout(Duration::from_secs(8))
        .build()
    {
        Ok(c) => c,
        Err(e) => {
            app_log!(error, "serve-watch", "HTTP client init failed: {e}");
            return;
        }
    };
    let mut state = WatchState::default();
    loop {
        let token = match read_token() {
            Some(t) => t,
            None => {
                // 还没配置过 Serve（用户从未启动过服务）：静默等
                tokio::time::sleep(RETRY_INTERVAL).await;
                continue;
            }
        };
        let base = format!("http://127.0.0.1:{}", read_port());
        let mut wait = POLL_INTERVAL;
        match poll_once(&client, &base, &token, &mut state, &app).await {
            Ok(()) => {}
            Err(e) => {
                // Serve 不在跑是常态（未启动/重启中），降噪为 debug 级
                app_log!(debug, "serve-watch", "poll failed: {e}");
                wait = RETRY_INTERVAL;
            }
        }
        tokio::time::sleep(wait).await;
    }
}

/// 一轮轮询：拉 entity 列表，对每个 running bot 拉待办 + 任务状态，差量推送。
async fn poll_once(
    client: &reqwest::Client,
    base: &str,
    token: &str,
    state: &mut WatchState,
    app: &AppHandle,
) -> Result<(), String> {
    let v = client
        .get(format!("{base}/api/entities"))
        .bearer_auth(token)
        .send()
        .await
        .map_err(|e| e.to_string())?
        .error_for_status()
        .map_err(|e| e.to_string())?
        .json::<Value>()
        .await
        .map_err(|e| e.to_string())?;
    let entities = v
        .get("entities")
        .and_then(Value::as_array)
        .cloned()
        .or_else(|| v.as_array().cloned())
        .unwrap_or_default();

    for e in entities {
        let id = e.get("entity_id").and_then(Value::as_str).unwrap_or("");
        if id.is_empty() {
            continue;
        }
        let name = e
            .get("name")
            .and_then(Value::as_str)
            .unwrap_or(id)
            .to_string();
        let runtime_state = e
            .get("runtime")
            .and_then(|r| r.get("state"))
            .and_then(Value::as_str)
            .unwrap_or("")
            .to_string();

        // Runtime 异常（running/failed 转变只推一次）
        if runtime_state == "failed"
            && state.runtime_state.get(id).map(String::as_str) != Some("failed")
        {
            let detail = e
                .get("runtime")
                .and_then(|r| r.get("detail"))
                .and_then(Value::as_str)
                .map(str::to_string);
            emit(
                app,
                BotEvent::RuntimeFailed {
                    entity_id: id.to_string(),
                    entity_name: name.clone(),
                    detail,
                },
            );
        }
        state
            .runtime_state
            .insert(id.to_string(), runtime_state.clone());

        // 只对 running 的 bot 拉待办与任务
        if runtime_state != "running" {
            continue;
        }

        // pending：questions + permissions
        if let Ok(body) = client
            .get(format!("{base}/api/entities/{id}/api/pending"))
            .bearer_auth(token)
            .send()
            .await
        {
            if let Ok(resp) = body.error_for_status() {
                if let Ok(p) = resp.json::<Value>().await {
                    // 新提问
                    if let Some(qs) = p.get("questions").and_then(Value::as_array) {
                        for q in qs {
                            let qid = q.get("question_id").and_then(Value::as_str).unwrap_or("");
                            if qid.is_empty() || state.questions.contains(qid) {
                                continue;
                            }
                            state.questions.insert(qid.to_string());
                            emit(
                                app,
                                BotEvent::Question {
                                    entity_id: id.to_string(),
                                    entity_name: name.clone(),
                                    question_id: qid.to_string(),
                                    question: q
                                        .get("question")
                                        .and_then(Value::as_str)
                                        .unwrap_or("")
                                        .to_string(),
                                    options: q
                                        .get("options")
                                        .and_then(Value::as_array)
                                        .map(|a| {
                                            a.iter()
                                                .filter_map(|o| o.as_str().map(str::to_string))
                                                .collect()
                                        })
                                        .unwrap_or_default(),
                                    allow_free_text: q
                                        .get("allow_free_text")
                                        .and_then(Value::as_bool)
                                        .unwrap_or(false),
                                },
                            );
                        }
                    }
                    // 新审批
                    if let Some(ps) = p.get("permissions").and_then(Value::as_array) {
                        for pm in ps {
                            let cid = pm.get("tool_call_id").and_then(Value::as_str).unwrap_or("");
                            if cid.is_empty() || state.permissions.contains(cid) {
                                continue;
                            }
                            state.permissions.insert(cid.to_string());
                            emit(
                                app,
                                BotEvent::Permission {
                                    entity_id: id.to_string(),
                                    entity_name: name.clone(),
                                    tool_call_id: cid.to_string(),
                                    tool_name: pm
                                        .get("tool_name")
                                        .and_then(Value::as_str)
                                        .unwrap_or("")
                                        .to_string(),
                                    summary: pm
                                        .get("summary")
                                        .and_then(Value::as_str)
                                        .unwrap_or("")
                                        .to_string(),
                                },
                            );
                        }
                    }
                }
            }
        }

        // 任务终态（host tasks API：{tasks: [GoalLedger...]}，含历史归档）
        if let Ok(body) = client
            .get(format!("{base}/api/entities/{id}/tasks"))
            .bearer_auth(token)
            .send()
            .await
        {
            if let Ok(resp) = body.error_for_status() {
                if let Ok(t) = resp.json::<Value>().await {
                    for task in t
                        .get("tasks")
                        .and_then(Value::as_array)
                        .into_iter()
                        .flatten()
                    {
                        let task_id = task
                            .get("goal_id")
                            .and_then(Value::as_str)
                            .unwrap_or("")
                            .to_string();
                        let status = task
                            .get("status")
                            .and_then(Value::as_str)
                            .unwrap_or("")
                            .to_string();
                        let terminal = matches!(
                            status.as_str(),
                            "achieved"
                                | "not_achievable"
                                | "budget_exhausted"
                                | "cancelled"
                                | "failed"
                        );
                        if !terminal || task_id.is_empty() {
                            continue;
                        }
                        // 同一任务的同一终态只推一次
                        if state.task_terminal.get(&task_id).map(String::as_str)
                            == Some(status.as_str())
                        {
                            continue;
                        }
                        state.task_terminal.insert(task_id.clone(), status.clone());
                        let goal = task
                            .get("goal")
                            .and_then(|g| g.get("title"))
                            .and_then(Value::as_str)
                            .or_else(|| task.get("mission").and_then(Value::as_str))
                            .unwrap_or("")
                            .to_string();
                        emit(
                            app,
                            BotEvent::TaskFinished {
                                entity_id: id.to_string(),
                                entity_name: name.clone(),
                                task_id,
                                goal,
                                ok: status == "achieved",
                                status,
                            },
                        );
                    }
                }
            }
        }
    }
    Ok(())
}

fn emit(app: &AppHandle, event: BotEvent) {
    if let Err(e) = app.emit("serve-bot-event", &event) {
        app_log!(error, "serve-watch", "emit failed: {e}");
    }
    app_log!(
        info,
        "serve-watch",
        "bot event: {}",
        serde_json::to_string(&event).unwrap_or_default()
    );
}
