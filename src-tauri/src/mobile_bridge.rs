//! 手机遥控桥：桌面端在局域网起一个小型 HTTP + WebSocket 服务，手机扫码
//! （或手输地址）即绑定本桌面端，可查看历史/进行中的对话并把消息发进
//! PC 的既有队列（Zed 式排队）。
//!
//! 职责边界（只做「传输与配对」，对话真相在 agent.rs 与前端 store）：
//! - 服务生命周期：随机端口 + 随机 token，启动返回 QR（SVG），停止即关闭
//! - 事件镜像：run.rs / permissions.rs / questions.rs 在 `app.emit` 之后把
//!   同一载荷投进 broadcast 总线，已连接的手机经 WS 收到；无人连接时为 no-op
//! - 手机发送：WS `send` → Tauri 事件 `mobile-bridge-event`(enqueue) →
//!   前端 store（stores/mobile.ts）解析目标会话并进入既有队列，由前端
//!   既有的续发 effect 自动发出；排序/删除同样经事件转发给前端执行
//! - 队列镜像：前端在队列变化时推快照（mobile_bridge_push_queue），本模块
//!   只缓存供新连接的 hello 快照使用，不拥有队列真相
//!
//! 安全模型：每次启动生成随机 token，QR 内容 `http://<lan-ip>:<port>/m#<token>`；
//! WS 与 HTTP API 都要求 token（query 参数）；token 只存在于 QR 与内存，不落盘。
//! 手机页面是浏览器打开的静态 HTML（本模块内嵌），无需安装任何 App。

use std::collections::HashMap;
use std::sync::atomic::{AtomicUsize, Ordering};
use std::sync::{Arc, Mutex, OnceLock};

use agent_core::Decision;
use serde::Serialize;
use serde_json::{json, Value};
use tauri::{AppHandle, Emitter, Manager, State};
use tokio::sync::broadcast;

static MOBILE_HTML: &str = include_str!("mobile_bridge_mobile.html");

// ===== 广播总线（宿主事件 → 已连接的手机） =====

fn bus() -> &'static broadcast::Sender<String> {
    static BUS: OnceLock<broadcast::Sender<String>> = OnceLock::new();
    BUS.get_or_init(|| broadcast::channel(1024).0)
}

// ===== 正在进行的 Run 事件缓冲（mid-run 加入的手机补看当前轮） =====
//
// 磁盘 agent_sessions.json 只在 Run 结算后由前端保存：中途扫码的手机拉历史
// 会缺整个当前轮。这里按会话缓存每个 Run 的实时事件，hello 时下发给新连接
//（S0 单活动 Run → 全局最多一个非 settled 的缓冲）。手机端把 live 事件直接
// 写进 pending 缓冲，复用既有的收养/回放路径，零新增回放代码。
// 缓冲在下一次该会话 user_message（新 Run 开始）时重置，不主动清理。

#[derive(Default)]
struct LiveRun {
    events: Vec<Value>,
    settled: bool,
}

/// 单个 Run 缓冲的事件上限（超长 Run 截断保头：手机端至少看得到开头）
const LIVE_EVENTS_CAP: usize = 800;

fn live_runs() -> &'static Mutex<HashMap<String, LiveRun>> {
    static LIVE: OnceLock<Mutex<HashMap<String, LiveRun>>> = OnceLock::new();
    LIVE.get_or_init(|| Mutex::new(HashMap::new()))
}

/// 把宿主事件记进对应会话的 live 缓冲（直接 append，不挂在广播接收端上，
/// bus lagged 丢事件不影响这里）
fn record_live(kind: &str, payload: &Value) {
    let Some(sid) = payload.get("session_id").and_then(|v| v.as_str()) else {
        return;
    };
    if sid.is_empty() {
        return;
    }
    let Ok(mut map) = live_runs().lock() else {
        return;
    };
    if kind == "agent-event" {
        let is_user = payload.get("type").and_then(|v| v.as_str()) == Some("user_message");
        let entry = map.entry(sid.to_string()).or_default();
        if is_user {
            // 新一轮 Run 从用户消息开始：重置缓冲，丢弃上一轮的尾部事件
            entry.events.clear();
            entry.settled = false;
        }
        if entry.events.len() < LIVE_EVENTS_CAP {
            entry.events.push(payload.clone());
        }
    } else if kind == "run-settled" {
        if let Some(entry) = map.get_mut(sid) {
            entry.settled = true;
        }
    }
}

/// hello 用：收集所有非 settled Run 的事件（S0 下通常至多一个会话）
fn live_events_for_hello() -> Vec<Value> {
    let Ok(map) = live_runs().lock() else {
        return Vec::new();
    };
    map.values()
        .filter(|run| !run.settled)
        .flat_map(|run| run.events.iter().cloned())
        .collect()
}

/// 镜像一条宿主事件给所有手机（kind 复用 Tauri 事件名，如 "agent-event"）
pub fn broadcast_event(kind: &str, payload: &Value) {
    record_live(kind, payload);
    broadcast_msg(&json!({ "type": kind, "payload": payload }));
}

/// 原样广播一条协议消息（queue 快照 / 配对数 / toast）
pub fn broadcast_msg(msg: &Value) {
    if let Ok(s) = serde_json::to_string(msg) {
        // 无接收端时 send 返回 Err：无人连接是常态，静默即可
        let _ = bus().send(s);
    }
}

// ===== 托管状态与命令 =====

pub struct MobileBridgeState {
    inner: tokio::sync::Mutex<Option<BridgeRunning>>,
}

impl Default for MobileBridgeState {
    fn default() -> Self {
        Self {
            inner: tokio::sync::Mutex::new(None),
        }
    }
}

struct BridgeRunning {
    port: u16,
    token: Arc<String>,
    shutdown: tokio::sync::watch::Sender<bool>,
    join: tokio::task::JoinHandle<()>,
    paired: Arc<AtomicUsize>,
    /// 前端推来的队列快照（camelCase QueuedMessage 去图片），hello 时下发
    queue: Arc<Mutex<Vec<Value>>>,
    /// 前端推来的当前查看会话 id（activeTabId），hello 时作为 current 下发
    view: Arc<Mutex<Option<String>>>,
}

#[derive(Clone)]
struct BridgeShared {
    app: AppHandle,
    token: Arc<String>,
    paired: Arc<AtomicUsize>,
    queue: Arc<Mutex<Vec<Value>>>,
    view: Arc<Mutex<Option<String>>>,
}

#[derive(Debug, Serialize)]
pub struct BridgeInfo {
    pub running: bool,
    pub port: u16,
    /// 手机浏览器打开的入口地址（token 在 fragment 里，不进服务器日志）
    pub url: String,
    pub token: String,
    /// 入口地址的二维码（SVG 字符串）
    pub qr_svg: String,
    pub lan_ip: String,
}

#[derive(Debug, Serialize)]
pub struct BridgeStatus {
    pub running: bool,
    pub port: Option<u16>,
    pub paired: usize,
    pub lan_ip: String,
}

/// 启动桥服务（已在运行则原样返回现有信息：QR 不变，避免每次打开页面都换 token）
#[tauri::command]
pub async fn mobile_bridge_start(
    app: AppHandle,
    state: State<'_, MobileBridgeState>,
) -> Result<BridgeInfo, String> {
    let mut guard = state.inner.lock().await;
    if let Some(running) = guard.as_ref() {
        return Ok(info_for(&app, running));
    }
    let token = Arc::new(uuid::Uuid::new_v4().simple().to_string());
    let paired = Arc::new(AtomicUsize::new(0));
    let queue = Arc::new(Mutex::new(Vec::new()));
    let view = Arc::new(Mutex::new(None));
    let shared = BridgeShared {
        app: app.clone(),
        token: Arc::clone(&token),
        paired: Arc::clone(&paired),
        queue: Arc::clone(&queue),
        view: Arc::clone(&view),
    };
    let (shutdown_tx, shutdown_rx) = tokio::sync::watch::channel(false);
    let listener = tokio::net::TcpListener::bind(("0.0.0.0", 0))
        .await
        .map_err(|e| format!("桥服务端口绑定失败: {e}"))?;
    let port = listener
        .local_addr()
        .map_err(|e| format!("桥服务地址读取失败: {e}"))?
        .port();
    let router = build_router(shared);
    let join = tokio::spawn(async move {
        if let Err(e) = axum::serve(listener, router)
            .with_graceful_shutdown(async move {
                let mut shutdown_rx = shutdown_rx;
                let _ = shutdown_rx.wait_for(|v| *v).await;
            })
            .await
        {
            app_log!(error, "mobile-bridge", "serve exited: {e}");
        }
    });
    let running = BridgeRunning {
        port,
        token,
        shutdown: shutdown_tx,
        join,
        paired,
        queue,
        view,
    };
    let info = info_for(&app, &running);
    *guard = Some(running);
    app_log!(info, "mobile-bridge", "started on port {}", info.port);
    Ok(info)
}

/// 停止桥服务（断开所有手机连接；队列真相在前端，不受影响）
#[tauri::command]
pub async fn mobile_bridge_stop(state: State<'_, MobileBridgeState>) -> Result<(), String> {
    let mut guard = state.inner.lock().await;
    if let Some(running) = guard.take() {
        running.paired.store(0, Ordering::Relaxed);
        let _ = running.shutdown.send(true);
        // 给 graceful shutdown 一点时间；超时不阻塞 UI
        let _ = tokio::time::timeout(std::time::Duration::from_secs(2), running.join).await;
        app_log!(info, "mobile-bridge", "stopped");
    }
    Ok(())
}

#[tauri::command]
pub async fn mobile_bridge_status(
    state: State<'_, MobileBridgeState>,
) -> Result<BridgeStatus, String> {
    let guard = state.inner.lock().await;
    Ok(BridgeStatus {
        running: guard.is_some(),
        port: guard.as_ref().map(|r| r.port),
        paired: guard
            .as_ref()
            .map(|r| r.paired.load(Ordering::Relaxed))
            .unwrap_or(0),
        lan_ip: lan_ip(),
    })
}

/// 前端在队列变化时推快照：更新镜像并广播给手机
#[tauri::command]
pub fn mobile_bridge_push_queue(state: State<'_, MobileBridgeState>, items: Vec<Value>) {
    let inner = state.inner.blocking_lock();
    if let Some(running) = inner.as_ref() {
        *running.queue.lock().unwrap_or_else(|e| e.into_inner()) = items.clone();
        broadcast_msg(&json!({ "type": "queue", "items": items }));
    }
}

/// 前端在当前查看会话（activeTabId）变化时推送：镜像并广播 view 给手机。
/// 手机端只有停在「新对话」时才跟进收养，不劫持正在浏览其他会话的设备
#[tauri::command]
pub fn mobile_bridge_push_view(state: State<'_, MobileBridgeState>, session_id: String) {
    let inner = state.inner.blocking_lock();
    if let Some(running) = inner.as_ref() {
        *running.view.lock().unwrap_or_else(|e| e.into_inner()) = Some(session_id.clone());
        broadcast_msg(&json!({ "type": "view", "session_id": session_id }));
    }
}

/// 前端在 pushUser（用户按下发送）时镜像首条 user_message：时机早于 Run 准入
///（checkpoint / MCP 装配等启动开销可达数秒），任一终端发送后所有设备立即
/// 同步。kind 复用 Tauri 事件名（如 "agent-event"），走 broadcast_event 同一条
/// 「镜像 + live 缓冲」链路（user_message 重置该会话的 live 缓冲，mid-run
/// 加入的手机从新用户消息开始补看）
#[tauri::command]
pub fn mobile_bridge_push_event(kind: String, payload: Value) {
    broadcast_event(&kind, &payload);
}

/// 前端向手机广播任意协议消息（toast 提示、会话列表刷新通知等）
#[tauri::command]
pub fn mobile_bridge_server_event(value: Value) {
    broadcast_msg(&value);
}

fn info_for(app: &AppHandle, running: &BridgeRunning) -> BridgeInfo {
    let _ = app;
    let lan_ip = lan_ip();
    let url = format!("http://{lan_ip}:{}/m#{}", running.port, running.token);
    let qr_svg = qr_svg(&url).unwrap_or_else(|e| {
        app_log!(error, "mobile-bridge", "qr render failed: {e}");
        String::new()
    });
    BridgeInfo {
        running: true,
        port: running.port,
        token: running.token.as_str().to_string(),
        url,
        qr_svg,
        lan_ip,
    }
}

// ===== 局域网 IP 与二维码 =====

/// 探测局域网 IP：优先 UDP connect 的路由候选（不发包），VPN/TUN 抢走默认路由时
/// （候选不是私网地址，如 Clash 的 198.18.x）回退到接口枚举里的私网 IPv4。
fn lan_ip() -> String {
    if let Ok(s) = std::net::UdpSocket::bind("0.0.0.0:0") {
        if s.connect("8.8.8.8:80").is_ok() {
            if let Ok(addr) = s.local_addr() {
                if let std::net::IpAddr::V4(v4) = addr.ip() {
                    if is_usable_lan_v4(v4) {
                        return v4.to_string();
                    }
                }
            }
        }
    }
    if let Ok(list) = if_addrs::get_if_addrs() {
        let mut scored: Vec<(u8, String)> = list
            .into_iter()
            .filter_map(|i| match i.addr {
                if_addrs::IfAddr::V4(v4) => Some(v4.ip),
                _ => None,
            })
            .filter(|ip| is_usable_lan_v4(*ip))
            .map(|ip| {
                let s = ip.to_string();
                let score = match ip.octets()[0] {
                    192 => 0,
                    10 => 1,
                    _ => 2,
                };
                (score, s)
            })
            .collect();
        scored.sort_by_key(|(score, _)| *score);
        if let Some((_, ip)) = scored.into_iter().next() {
            return ip;
        }
    }
    "127.0.0.1".to_string()
}

/// 可用的局域网私网 IPv4：排除回环 / 链路本地 / CGNAT 100.64/10 / 基准测试段 198.18/15
/// （is_private 只覆盖 RFC1918：10/8、172.16/12、192.168/16，后两段额外排除）
fn is_usable_lan_v4(ip: std::net::Ipv4Addr) -> bool {
    let o = ip.octets();
    !ip.is_loopback()
        && !ip.is_link_local()
        && ip.is_private()
        && !(o[0] == 100 && (64..=127).contains(&o[1]))
        && !(o[0] == 198 && (18..=19).contains(&o[1]))
}

fn qr_svg(text: &str) -> Result<String, String> {
    let code = qrcode::QrCode::new(text.as_bytes()).map_err(|e| e.to_string())?;
    Ok(code
        .render::<qrcode::render::svg::Color>()
        .min_dimensions(232, 232)
        .build())
}

// ===== HTTP 路由 =====

fn build_router(shared: BridgeShared) -> axum::Router {
    use axum::routing::get;
    axum::Router::new()
        .route("/m", get(|| async { axum::response::Html(MOBILE_HTML) }))
        .route("/api/sessions", get(api_sessions))
        .route("/api/session/:id", get(api_session_detail))
        .route("/api/ws", get(api_ws))
        .with_state(shared)
}

#[derive(serde::Deserialize)]
struct TokenQuery {
    token: Option<String>,
}

fn check_token(shared: &BridgeShared, q: &TokenQuery) -> bool {
    q.token.as_deref() == Some(shared.token.as_str())
}

/// 会话元数据列表（不带 entries；按更新时间倒序，与桌面 list_agent_sessions 一致）
fn session_metas(app: &AppHandle) -> Result<Vec<Value>, String> {
    let path = crate::agent::session_store::ensure_json_file(
        app,
        "agent_sessions.json",
        "创建会话记录文件",
    )?;
    let mut sessions: Vec<crate::agent::sessions::AgentSession> =
        crate::agent::session_store::read_json_vec(&path);
    sessions.sort_by(|a, b| b.updated_at.cmp(&a.updated_at));
    Ok(sessions
        .iter()
        .map(|s| {
            json!({
                "id": s.id,
                "title": s.title,
                "project_name": s.project_name,
                "project_path": s.project_path,
                "created_at": s.created_at,
                "updated_at": s.updated_at,
                "archived": s.archived,
            })
        })
        .collect())
}

async fn api_sessions(
    axum::extract::State(shared): axum::extract::State<BridgeShared>,
    axum::extract::Query(q): axum::extract::Query<TokenQuery>,
) -> axum::response::Response {
    use axum::response::IntoResponse;
    if !check_token(&shared, &q) {
        return axum::http::StatusCode::UNAUTHORIZED.into_response();
    }
    match session_metas(&shared.app) {
        Ok(v) => axum::Json(v).into_response(),
        Err(e) => (axum::http::StatusCode::INTERNAL_SERVER_ERROR, e).into_response(),
    }
}

async fn api_session_detail(
    axum::extract::Path(id): axum::extract::Path<String>,
    axum::extract::State(shared): axum::extract::State<BridgeShared>,
    axum::extract::Query(q): axum::extract::Query<TokenQuery>,
) -> axum::response::Response {
    use axum::response::IntoResponse;
    if !check_token(&shared, &q) {
        return axum::http::StatusCode::UNAUTHORIZED.into_response();
    }
    let path = match crate::agent::session_store::ensure_json_file(
        &shared.app,
        "agent_sessions.json",
        "创建会话记录文件",
    ) {
        Ok(p) => p,
        Err(e) => return (axum::http::StatusCode::INTERNAL_SERVER_ERROR, e).into_response(),
    };
    let sessions: Vec<crate::agent::sessions::AgentSession> =
        crate::agent::session_store::read_json_vec(&path);
    match sessions.into_iter().find(|s| s.id == id) {
        Some(s) => axum::Json(json!({
            "id": s.id,
            "title": s.title,
            "project_name": s.project_name,
            "project_path": s.project_path,
            "updated_at": s.updated_at,
            "archived": s.archived,
            // 前端条目流原样回放（camelCase Entry 形状），手机页按同款 kind 渲染
            "entries": s.entries,
        }))
        .into_response(),
        None => (axum::http::StatusCode::NOT_FOUND, "会话不存在".to_string()).into_response(),
    }
}

// ===== WebSocket =====

async fn api_ws(
    ws: axum::extract::ws::WebSocketUpgrade,
    axum::extract::Query(q): axum::extract::Query<TokenQuery>,
    axum::extract::State(shared): axum::extract::State<BridgeShared>,
) -> axum::response::Response {
    use axum::response::IntoResponse;
    if !check_token(&shared, &q) {
        return axum::http::StatusCode::UNAUTHORIZED.into_response();
    }
    ws.on_upgrade(move |socket| ws_connection(socket, shared))
}

async fn ws_connection(socket: axum::extract::ws::WebSocket, shared: BridgeShared) {
    use axum::extract::ws::Message;
    use futures_util::stream::StreamExt;
    use futures_util::SinkExt;

    let (mut sink, mut stream) = socket.split();
    // 出站统一走 mpsc：hello / pong（读循环）与总线转发两个生产者共用
    let (out_tx, mut out_rx) = tokio::sync::mpsc::channel::<String>(128);
    let mut bus_rx = bus().subscribe();

    shared.paired.fetch_add(1, Ordering::Relaxed);
    let paired_now = shared.paired.load(Ordering::Relaxed);
    // 桌面 UI 也要看到配对数变化
    let _ = shared.app.emit(
        "mobile-bridge-event",
        json!({ "type": "pair", "count": paired_now }),
    );
    broadcast_msg(&json!({ "type": "pair", "count": paired_now }));

    // hello 快照：先订阅总线再发快照，连接期间事件不丢。
    // current：PC 当前查看的会话（手机端停在「新对话」时跟进收养）；
    // live：进行中 Run 的实时事件（mid-run 加入的手机补看当前轮，
    // 配合磁盘历史缺失的窗口）；已结算的 Run 不下发（磁盘已有，回放会重复）
    let queue = shared
        .queue
        .lock()
        .unwrap_or_else(|e| e.into_inner())
        .clone();
    let current = shared
        .view
        .lock()
        .unwrap_or_else(|e| e.into_inner())
        .clone();
    let live = live_events_for_hello();
    let hello = json!({
        "type": "hello",
        "sessions": session_metas(&shared.app).unwrap_or_default(),
        "queue": queue,
        "paired": paired_now,
        "current": current,
        "live": live,
    });
    let _ = out_tx.send(hello.to_string()).await;

    // 总线 → 出站
    let bus_tx = out_tx.clone();
    let forwarder = tokio::spawn(async move {
        loop {
            match bus_rx.recv().await {
                Ok(msg) => {
                    if bus_tx.send(msg).await.is_err() {
                        break;
                    }
                }
                Err(broadcast::error::RecvError::Closed) => break,
                Err(broadcast::error::RecvError::Lagged(_)) => continue,
            }
        }
    });
    // 出站 → socket
    let writer = tokio::spawn(async move {
        while let Some(msg) = out_rx.recv().await {
            if sink.send(Message::Text(msg)).await.is_err() {
                break;
            }
        }
    });

    while let Some(msg) = stream.next().await {
        match msg {
            Ok(Message::Text(text)) => handle_client_msg(&shared, &out_tx, &text).await,
            Ok(_) => {}
            Err(_) => break,
        }
    }
    forwarder.abort();
    drop(out_tx);
    let _ = writer.await;
    let remaining = shared
        .paired
        .fetch_sub(1, Ordering::Relaxed)
        .saturating_sub(1);
    let _ = shared.app.emit(
        "mobile-bridge-event",
        json!({ "type": "pair", "count": remaining }),
    );
    broadcast_msg(&json!({ "type": "pair", "count": remaining }));
}

async fn handle_client_msg(
    shared: &BridgeShared,
    out_tx: &tokio::sync::mpsc::Sender<String>,
    text: &str,
) {
    let Ok(v) = serde_json::from_str::<Value>(text) else {
        return;
    };
    match v["type"].as_str() {
        Some("ping") => {
            let _ = out_tx.send(json!({ "type": "pong" }).to_string()).await;
        }
        Some("send") => {
            let body = v["text"].as_str().unwrap_or("").trim().to_string();
            if body.is_empty() {
                return;
            }
            let session_id = v["session_id"].as_str().map(|s| s.to_string());
            let skill = v["skill"]
                .as_str()
                .map(|s| s.trim().to_string())
                .filter(|s| !s.is_empty());
            // 发送者设备名（手机端本地生成，桥不注册身份，纯透传；重名无害）
            let sender = v["sender"]
                .as_str()
                .map(|s| s.trim().to_string())
                .filter(|s| !s.is_empty());
            let _ = shared.app.emit(
                "mobile-bridge-event",
                json!({
                    "type": "enqueue",
                    "msg_id": v["msg_id"],
                    "session_id": session_id,
                    "text": body,
                    "skill": skill,
                    "sender": sender,
                }),
            );
        }
        Some("queue_reorder") => {
            let _ = shared.app.emit(
                "mobile-bridge-event",
                json!({
                    "type": "queue_reorder",
                    "id": v["id"],
                    "target_id": v["target_id"],
                    "after": v["after"].as_bool().unwrap_or(false),
                }),
            );
        }
        Some("queue_remove") => {
            let _ = shared.app.emit(
                "mobile-bridge-event",
                json!({ "type": "queue_remove", "id": v["id"] }),
            );
        }
        Some("permission_decide") => {
            let tool_call_id = v["tool_call_id"].as_str().unwrap_or_default().to_string();
            if tool_call_id.is_empty() {
                return;
            }
            let decision = match v["decision"].as_str() {
                Some("allow") => Decision::Allow,
                _ => Decision::Deny,
            };
            let state = shared.app.state::<crate::agent::AgentState>();
            if let Err(e) = state.hub.decide(&tool_call_id, decision) {
                broadcast_msg(&json!({ "type": "toast", "text": e }));
            }
        }
        Some("question_answer") => {
            let question_id = v["question_id"].as_str().unwrap_or_default().to_string();
            if question_id.is_empty() {
                return;
            }
            let answer = v["answer"].as_str().map(|s| s.to_string());
            let state = shared.app.state::<crate::agent::AgentState>();
            if let Err(e) = state.questions.answer(&question_id, answer) {
                broadcast_msg(&json!({ "type": "toast", "text": e }));
            }
        }
        Some("set_model") => {
            let _ = shared.app.emit(
                "mobile-bridge-event",
                json!({
                    "type": "set_model",
                    "source": v["source"],
                    "port": v["port"],
                    "provider_id": v["provider_id"],
                    "model": v["model"],
                }),
            );
        }
        Some("set_thinking") => {
            let _ = shared.app.emit(
                "mobile-bridge-event",
                json!({
                    "type": "set_thinking",
                    "enabled": v["enabled"].as_bool().unwrap_or(false),
                    "effort": v["effort"],
                }),
            );
        }
        Some("set_auto_mode") => {
            let _ = shared.app.emit(
                "mobile-bridge-event",
                json!({
                    "type": "set_auto_mode",
                    "mode": v["mode"],
                }),
            );
        }
        _ => {}
    }
}

// ===== 测试 =====

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn live_run_buffer_resets_on_user_message_and_marks_settled() {
        let sid = "sess-live";
        record_live(
            "agent-event",
            &json!({ "type": "message_delta", "session_id": sid, "text": "a" }),
        );
        record_live(
            "agent-event",
            &json!({ "type": "tool_call_started", "session_id": sid, "tool_call_id": "t1" }),
        );
        {
            let map = live_runs().lock().unwrap();
            let run = map.get(sid).unwrap();
            assert_eq!(run.events.len(), 2);
            assert!(!run.settled);
        }
        // 新 Run 的用户消息重置缓冲
        record_live(
            "agent-event",
            &json!({ "type": "user_message", "session_id": sid, "text": "再来一轮" }),
        );
        {
            let map = live_runs().lock().unwrap();
            let run = map.get(sid).unwrap();
            assert_eq!(run.events.len(), 1);
            assert_eq!(run.events[0]["type"], "user_message");
            assert!(!run.settled);
        }
        // 结算置位后 hello 不再下发
        record_live("run-settled", &json!({ "session_id": sid }));
        assert!(live_events_for_hello()
            .iter()
            .all(|e| e["session_id"] != sid));
    }
    #[test]
    fn live_run_buffer_ignores_events_without_session() {
        // 与其他测试共享全局 LIVE_RUNS，不能断言总长度（并行执行会互相干扰），
        // 只断言无 session_id 的事件不会创建缓冲
        record_live(
            "agent-event",
            &json!({ "type": "toast", "text": "no session" }),
        );
        record_live("queue", &json!({ "items": [] }));
        let map = live_runs().lock().unwrap();
        assert!(
            map.get("sess-no-such").is_none(),
            "无 session_id 的事件不应创建 live 缓冲"
        );
    }

    #[test]
    fn lan_ip_filter_excludes_tunnel_and_link_local_ranges() {
        let ok = ["192.168.1.5", "10.0.0.2", "172.20.1.9"];
        let bad = [
            "127.0.0.1",
            "169.254.10.2",
            "100.64.0.1",
            "100.100.1.1",
            "198.18.0.1",
            "8.8.8.8",
        ];
        for ip in ok {
            assert!(
                is_usable_lan_v4(ip.parse().unwrap()),
                "{ip} 应被视为可用私网地址"
            );
        }
        for ip in bad {
            assert!(
                !is_usable_lan_v4(ip.parse().unwrap()),
                "{ip} 不应被视为可用私网地址"
            );
        }
    }

    #[test]
    fn broadcast_bus_delivers_to_subscribers_and_ignores_silence() {
        let mut rx = bus().subscribe();
        broadcast_event(
            "agent-event",
            &json!({ "type": "message_delta", "text": "hi" }),
        );
        let got = rx.try_recv().expect("订阅者应收到消息");
        let v: Value = serde_json::from_str(&got).unwrap();
        assert_eq!(v["type"], "agent-event");
        assert_eq!(v["payload"]["text"], "hi");
        // 无订阅者也不 panic
        broadcast_msg(&json!({ "type": "queue", "items": [] }));
    }

    #[tokio::test]
    async fn qr_svg_embeds_payload_and_state_defaults_to_offline() {
        let svg = qr_svg("http://192.168.1.5:40000/m#abc").unwrap();
        assert!(svg.contains("<svg"), "应输出 SVG：{svg}");
        // 状态读取不依赖运行中的服务：默认未启动
        let state = MobileBridgeState::default();
        assert!(state.inner.lock().await.is_none());
    }
}
