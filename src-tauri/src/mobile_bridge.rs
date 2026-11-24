//! 手机遥控桥：桌面端在局域网起一个小型 HTTP + WebSocket 服务，手机扫码
//! （或手输地址）即绑定本桌面端，可查看历史/进行中的对话并把消息发进
//! PC 的既有队列（Zed 式排队）。
//!
//! 职责边界（只做「传输与配对」，对话真相在 agent.rs 与前端 store）：
//! - 服务生命周期：持久化端口 + 临时 token，启动返回 QR（SVG），停止即关闭
//! - 事件镜像：run.rs / permissions.rs / questions.rs 在 `app.emit` 之后把
//!   同一载荷投进 broadcast 总线，已连接的手机经 WS 收到；无人连接时为 no-op
//! - 手机发送：WS `send` → Tauri 事件 `mobile-bridge-event`(enqueue) →
//!   前端 store（stores/mobile.ts）解析目标会话并进入既有队列，由前端
//!   既有的续发 effect 自动发出；排序/删除同样经事件转发给前端执行
//! - 队列镜像：前端在队列变化时推快照（mobile_bridge_push_queue），本模块
//!   只缓存供新连接的 hello 快照使用，不拥有队列真相
//!
//! 安全模型（两层凭证 + 票据 + 限速 + scope）：
//! - 临时配对：每次启动生成随机 token，QR 内容 `http://<lan-ip>:<port>/m#<token>`；
//!   token 只存在于 QR 与内存，不落盘，重启失效
//! - 持久设备 key：`mbk_` 前缀 secret，QR `.../m#mbk_<64hex>`，服务端只存 SHA-256
//!   hash（mobile_keys.rs），支持失效时间与撤销；scope 可限定单会话
//! - 票据：长期凭证不进 URL query（会进日志）——页面拿凭证 POST /api/ticket
//!   （Authorization: Bearer）换短时效一次性 ticket，WS 用 `?ticket=` 连接；
//!   HTTP fetch 直接用 Bearer 凭证
//! - 限速：认证失败按 IP 指数退避封禁（持久 key 给了暴力枚举无限时间）
//! - scope 强制执行：scoped 连接的 hello / HTTP / 事件转发 / WS 命令都在本层
//!   按绑定会话过滤，不依赖手机端 UI 隐藏
//! 手机页面是浏览器打开的静态 HTML（本模块内嵌），无需安装任何 App。

use std::collections::HashMap;
use std::net::IpAddr;
use std::sync::atomic::{AtomicUsize, Ordering};
use std::sync::{Arc, Mutex, OnceLock};
use std::time::Instant;

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
    // 顺带记录待处置上下文（问题/权限卡片 → 会话），供 scoped 连接作答时
    // 校验归属（question_answer / permission_decide 只带 id 不带 session_id）
    if kind == "agent-question" {
        if let Some(qid) = payload.get("question_id").and_then(|v| v.as_str()) {
            record_pending_ctx(qid, sid);
        }
    } else if kind == "agent-event" {
        if let Some(tid) = payload.get("tool_call_id").and_then(|v| v.as_str()) {
            if !tid.is_empty() {
                record_pending_ctx(tid, sid);
            }
        }
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

// --- 待处置上下文：问题/权限卡片 id → 会话 id（scoped 作答校验归属用） ---

/// 上限防膨胀（超限整体清空：最坏情况是 scoped 设备短暂答不了老卡片）
const PENDING_CTX_CAP: usize = 512;

fn pending_ctx() -> &'static Mutex<HashMap<String, String>> {
    static PENDING: OnceLock<Mutex<HashMap<String, String>>> = OnceLock::new();
    PENDING.get_or_init(|| Mutex::new(HashMap::new()))
}

fn record_pending_ctx(id: &str, session_id: &str) {
    if let Ok(mut map) = pending_ctx().lock() {
        if map.len() >= PENDING_CTX_CAP {
            map.clear();
        }
        map.insert(id.to_string(), session_id.to_string());
    }
}

/// scoped 连接的队列操作是否被允许：目标条目必须属于绑定会话。
/// all 连接永远允许；查不到条目时 fail-closed（scoped 拒绝）
fn scoped_queue_op_allowed(shared: &BridgeShared, ctx: &AuthCtx, item_id: &Value) -> bool {
    if matches!(ctx.scope, crate::mobile_keys::KeyScope::All) {
        return true;
    }
    let Some(bound) = ctx.forced_session() else {
        return true;
    };
    let Some(item_id) = item_id.as_str() else {
        return false;
    };
    let queue = shared.queue.lock().unwrap_or_else(|e| e.into_inner());
    queue.iter().any(|item| {
        item.get("id").and_then(|v| v.as_str()) == Some(item_id)
            && queue_item_in_scope(item, Some(&bound))
    })
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

// ===== 认证：凭证 → AuthCtx（scope） + 票据 + 按IP 限速 =====

/// 连接的授权上下文：scope 决定可见/可操作范围，key_id 供撤销时踢人
#[derive(Clone, Debug, PartialEq)]
struct AuthCtx {
    scope: crate::mobile_keys::KeyScope,
    /// None = 临时配对 token（不可单独踢，桥停止时一起断）
    key_id: Option<String>,
}

impl AuthCtx {
    fn all() -> Self {
        Self {
            scope: crate::mobile_keys::KeyScope::All,
            key_id: None,
        }
    }

    /// 该会话是否在授权范围内
    fn allows_session(&self, session_id: &str) -> bool {
        match &self.scope {
            crate::mobile_keys::KeyScope::All => true,
            crate::mobile_keys::KeyScope::Session { session_id: bound } => bound == session_id,
        }
    }

    /// scoped 连接上强制使用的会话 id
    fn forced_session(&self) -> Option<String> {
        self.scope.session_id().map(|s| s.to_string())
    }
}

// --- 票据：短时效一次性，替代把长期凭证放进 WS URL query ---

struct Ticket {
    ctx: AuthCtx,
    expires: Instant,
}

const TICKET_TTL_SECS: u64 = 30;

fn tickets() -> &'static Mutex<HashMap<String, Ticket>> {
    static TICKETS: OnceLock<Mutex<HashMap<String, Ticket>>> = OnceLock::new();
    TICKETS.get_or_init(|| Mutex::new(HashMap::new()))
}

fn issue_ticket(ctx: AuthCtx) -> String {
    let id = uuid::Uuid::new_v4().simple().to_string();
    let ticket = Ticket {
        ctx,
        expires: Instant::now() + std::time::Duration::from_secs(TICKET_TTL_SECS),
    };
    if let Ok(mut map) = tickets().lock() {
        // 顺手清过期票据，防长开页面累积
        map.retain(|_, t| t.expires > Instant::now());
        map.insert(id.clone(), ticket);
    }
    id
}

/// 消费票据：单次使用（取后即删），过期无效。有效不在这里计失败
///（票据是换来的短期凭证，拿不到票据本身已经过认证环节）
fn consume_ticket(ticket: &str) -> Option<AuthCtx> {
    tickets()
        .lock()
        .ok()
        .and_then(|mut map| map.remove(ticket))
        .filter(|t| t.expires > Instant::now())
        .map(|t| t.ctx)
}

// --- 按IP 限速：认证失败指数退避（持久 key 给了暴力枚举无限时间） ---

#[derive(Default)]
struct IpGuard {
    fails: u32,
    blocked_until: Option<Instant>,
}

fn ip_guards() -> &'static Mutex<HashMap<IpAddr, IpGuard>> {
    static GUARDS: OnceLock<Mutex<HashMap<IpAddr, IpGuard>>> = OnceLock::new();
    GUARDS.get_or_init(|| Mutex::new(HashMap::new()))
}

/// 认证前检查：被封禁中返回剩余秒数
fn auth_blocked_remaining(ip: IpAddr) -> Option<u64> {
    let mut map = ip_guards().lock().ok()?;
    let guard = map.get_mut(&ip)?;
    if let Some(until) = guard.blocked_until {
        let now = Instant::now();
        if until > now {
            return Some((until - now).as_secs().max(1));
        }
        // 封禁到期：解除（保留计数作参考，下次失败重新升级）
        guard.blocked_until = None;
    }
    None
}

fn auth_failure(ip: IpAddr) {
    if let Ok(mut map) = ip_guards().lock() {
        let fails = map.entry(ip).or_default();
        fails.fails = fails.fails.saturating_add(1);
        let secs = crate::mobile_keys::backoff_secs(fails.fails);
        fails.blocked_until = Some(Instant::now() + std::time::Duration::from_secs(secs));
    }
}

fn auth_success(ip: IpAddr) {
    if let Ok(mut map) = ip_guards().lock() {
        map.remove(&ip);
    }
}

// --- 凭证解析：Bearer header（secret/ticket）或 query（ticket/token） ---

#[derive(Debug)]
struct AuthReject {
    status: axum::http::StatusCode,
    message: String,
}

impl AuthReject {
    fn unauthorized(msg: &str) -> Self {
        Self {
            status: axum::http::StatusCode::UNAUTHORIZED,
            message: msg.to_string(),
        }
    }

    fn too_many(secs: u64) -> Self {
        Self {
            status: axum::http::StatusCode::TOO_MANY_REQUESTS,
            message: format!("认证失败次数过多，请 {secs} 秒后重试"),
        }
    }
}

fn bearer_of(headers: &axum::http::HeaderMap) -> Option<String> {
    let v = headers
        .get(axum::http::header::AUTHORIZATION)?
        .to_str()
        .ok()?;
    v.strip_prefix("Bearer ")
        .map(|s| s.trim().to_string())
        .filter(|s| !s.is_empty())
}

/// 解析凭证（ticket / Bearer 凭证 / query token → AuthCtx）。
/// 凭证无效会累计 IP 失败计数并可能触发封禁
fn resolve_auth_creds(
    app: &AppHandle,
    temp_token: &str,
    bearer: Option<&str>,
    query_ticket: Option<&str>,
    query_token: Option<&str>,
    ip: IpAddr,
) -> Result<AuthCtx, AuthReject> {
    if let Some(secs) = auth_blocked_remaining(ip) {
        return Err(AuthReject::too_many(secs));
    }
    // 1. 票据（换票时已完成真正的凭证校验）
    if let Some(t) = query_ticket {
        if let Some(ctx) = consume_ticket(t) {
            auth_success(ip);
            return Ok(ctx);
        }
    }
    // 2. Bearer / query：短期凭证（临时 token）或持久 key secret
    if let Some(cred) = bearer.or(query_token) {
        if cred == temp_token {
            auth_success(ip);
            return Ok(AuthCtx::all());
        }
        if let Ok(keys) = crate::mobile_keys::load_keys(app) {
            if let Some(key) =
                crate::mobile_keys::validate_in_list(&keys, cred, crate::mobile_keys::now_ms())
            {
                auth_success(ip);
                // 更新 last_used / last_ip（低频路径，读改写文件可接受）
                let mut keys = keys;
                if let Some(k) = keys.iter_mut().find(|k| k.key_id == key.key_id) {
                    k.last_used_at = Some(crate::mobile_keys::now_ms());
                    k.last_ip = Some(ip.to_string());
                }
                let _ = crate::mobile_keys::save_keys(app, &keys);
                return Ok(AuthCtx {
                    scope: key.scope,
                    key_id: Some(key.key_id),
                });
            }
        }
    }
    auth_failure(ip);
    Err(AuthReject::unauthorized("无效的配对凭证，请重新扫码"))
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
    /// 在线连接注册表：key_id → 出站通道列表（撤销时丢弃 sender = 踢下线；
    /// 临时配对无 key_id，不注册）
    conns: Arc<Mutex<HashMap<String, Vec<tokio::sync::mpsc::Sender<String>>>>>,
}

#[derive(Clone)]
struct BridgeShared {
    app: AppHandle,
    token: Arc<String>,
    paired: Arc<AtomicUsize>,
    queue: Arc<Mutex<Vec<Value>>>,
    view: Arc<Mutex<Option<String>>>,
    conns: Arc<Mutex<HashMap<String, Vec<tokio::sync::mpsc::Sender<String>>>>>,
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
    let conns = Arc::new(Mutex::new(HashMap::new()));
    let shared = BridgeShared {
        app: app.clone(),
        token: Arc::clone(&token),
        paired: Arc::clone(&paired),
        queue: Arc::clone(&queue),
        view: Arc::clone(&view),
        conns: Arc::clone(&conns),
    };
    let (shutdown_tx, shutdown_rx) = tokio::sync::watch::channel(false);
    // 端口持久化：优先复用上次成功端口（URL 稳定，持久 key 才有意义），
    // 被占则向下试探 20 个，仍不行回退随机端口（port 0 = 内核分配）
    let preferred = crate::mobile_keys::load_saved_port(&app);
    let listener = pick_listener(preferred).await?;
    let port = listener
        .local_addr()
        .map_err(|e| format!("桥服务地址读取失败: {e}"))?
        .port();
    crate::mobile_keys::save_port(&app, port);
    let router = build_router(shared);
    let join = tokio::spawn(async move {
        if let Err(e) = axum::serve(
            listener,
            router.into_make_service_with_connect_info::<std::net::SocketAddr>(),
        )
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
        conns,
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

// ===== 持久设备 key 命令 =====

#[derive(Debug, Serialize)]
pub struct KeyCreated {
    pub id: String,
    /// 完整 secret（mbk_ 前缀）：只在创建时返回一次，后续无法再取
    pub secret: String,
    /// 手机扫码入口：url#secret（QR 同内容）
    pub url: String,
    pub qr_svg: String,
    pub name: String,
    pub scope: crate::mobile_keys::KeyScope,
}

/// 创建持久设备 key。name = 设备名；scope_session_id 传 Some 则只授权单会话；
/// expires_days 传 Some(7/30) 则按时过期，None = 永久。
/// 要求桥已在运行（QR 要带端口；未运行提示先开启）
#[tauri::command]
pub fn mobile_bridge_key_create(
    app: AppHandle,
    state: State<'_, MobileBridgeState>,
    name: String,
    scope_session_id: Option<String>,
    expires_days: Option<u64>,
) -> Result<KeyCreated, String> {
    let guard = state.inner.blocking_lock();
    let running = guard
        .as_ref()
        .ok_or_else(|| "手机遥控未开启，请先开启后再创建设备密钥".to_string())?;
    let scope = match scope_session_id {
        Some(sid) if !sid.trim().is_empty() => {
            crate::mobile_keys::KeyScope::Session { session_id: sid }
        }
        _ => crate::mobile_keys::KeyScope::All,
    };
    let expires_at =
        expires_days.map(|d| crate::mobile_keys::now_ms() + d.saturating_mul(86_400_000));
    let mut keys = crate::mobile_keys::load_keys(&app)?;
    let (key, secret) = crate::mobile_keys::create_in_list(
        &mut keys,
        if name.trim().is_empty() {
            "未命名设备"
        } else {
            &name
        },
        scope,
        expires_at,
    );
    crate::mobile_keys::save_keys(&app, &keys)?;
    let url = format!("http://{}:{}/m#{}", lan_ip(), running.port, secret);
    let qr_svg = qr_svg(&url)?;
    Ok(KeyCreated {
        id: key.key_id,
        secret,
        url,
        qr_svg,
        name: key.name,
        scope: key.scope,
    })
}

/// 设备 key 列表（不含 hash/secret）
#[tauri::command]
pub fn mobile_bridge_key_list(app: AppHandle) -> Result<Vec<Value>, String> {
    Ok(crate::mobile_keys::load_keys(&app)?
        .iter()
        .map(|k| k.to_view())
        .collect())
}

/// 撤销设备 key：立即失效并踢掉该设备的在线连接，返回踢掉数量
#[tauri::command]
pub async fn mobile_bridge_key_revoke(
    app: AppHandle,
    state: State<'_, MobileBridgeState>,
    key_id: String,
) -> Result<usize, String> {
    let mut keys = crate::mobile_keys::load_keys(&app)?;
    if !crate::mobile_keys::revoke_in_list(&mut keys, &key_id) {
        return Err("设备密钥不存在或已撤销".to_string());
    }
    crate::mobile_keys::save_keys(&app, &keys)?;
    // 踢下线：丢弃该 key 的所有出站通道（writer 结束 → socket 关闭）
    let kicked = {
        let guard = state.inner.lock().await;
        match guard.as_ref() {
            Some(running) => {
                let mut kicked = 0usize;
                if let Ok(mut map) = running.conns.lock() {
                    if let Some(list) = map.remove(&key_id) {
                        kicked = list.len();
                    }
                }
                kicked
            }
            None => 0,
        }
    };
    if kicked > 0 {
        app_log!(
            info,
            "mobile-bridge",
            "revoked key {key_id}, kicked {kicked} conn(s)"
        );
    }
    Ok(kicked)
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

/// 端口选择：优先 preferred，被占则向下试探 20 个，全不行回退随机端口
async fn pick_listener(preferred: Option<u16>) -> Result<tokio::net::TcpListener, String> {
    if let Some(port) = preferred {
        if port >= 1024 {
            for candidate in (port.saturating_sub(20)..=port).rev() {
                if let Ok(l) = tokio::net::TcpListener::bind(("0.0.0.0", candidate)).await {
                    return Ok(l);
                }
            }
        }
    }
    tokio::net::TcpListener::bind(("0.0.0.0", 0))
        .await
        .map_err(|e| format!("桥服务端口绑定失败: {e}"))
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
    use axum::routing::{get, post};
    axum::Router::new()
        .route("/m", get(|| async { axum::response::Html(MOBILE_HTML) }))
        .route("/api/ticket", post(api_ticket))
        .route("/api/sessions", get(api_sessions))
        .route("/api/session/:id", get(api_session_detail))
        .route("/api/ws", get(api_ws))
        .with_state(shared)
}

#[derive(serde::Deserialize)]
struct TokenQuery {
    /// 兼容旧页面的临时 token（query）；新页面走 Bearer/ticket
    token: Option<String>,
    /// WS 连接用：一次性短时效票据
    ticket: Option<String>,
}

/// 统一认证入口：query + header + 客户端 IP → AuthCtx。拒绝时已计限速
fn resolve_auth(
    shared: &BridgeShared,
    headers: &axum::http::HeaderMap,
    q: &TokenQuery,
    ip: IpAddr,
) -> Result<AuthCtx, axum::response::Response> {
    use axum::response::IntoResponse;
    match resolve_auth_creds(
        &shared.app,
        &shared.token,
        bearer_of(headers).as_deref(),
        q.ticket.as_deref(),
        q.token.as_deref(),
        ip,
    ) {
        Ok(ctx) => Ok(ctx),
        Err(rej) => Err((rej.status, rej.message).into_response()),
    }
}

/// 换票：长期凭证（临时 token / 持久 key secret）经 Authorization header 校验，
/// 换短时效一次性 ticket。手机页面用它在 WS URL 里避免长期凭证进 query
async fn api_ticket(
    axum::extract::State(shared): axum::extract::State<BridgeShared>,
    headers: axum::http::HeaderMap,
    axum::extract::ConnectInfo(addr): axum::extract::ConnectInfo<std::net::SocketAddr>,
) -> axum::response::Response {
    use axum::response::IntoResponse;
    let ctx = match resolve_auth(
        &shared,
        &headers,
        &TokenQuery {
            token: None,
            ticket: None,
        },
        addr.ip(),
    ) {
        Ok(ctx) => ctx,
        Err(resp) => return resp,
    };
    let ticket = issue_ticket(ctx.clone());
    axum::Json(json!({
        "ticket": ticket,
        "scope": ctx.scope.tag(),
        "session_id": ctx.scope.session_id(),
    }))
    .into_response()
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
    headers: axum::http::HeaderMap,
    axum::extract::Query(q): axum::extract::Query<TokenQuery>,
    axum::extract::ConnectInfo(addr): axum::extract::ConnectInfo<std::net::SocketAddr>,
) -> axum::response::Response {
    use axum::response::IntoResponse;
    let ctx = match resolve_auth(&shared, &headers, &q, addr.ip()) {
        Ok(ctx) => ctx,
        Err(resp) => return resp,
    };
    // scoped 只能看到绑定的会话（本层强制，不依赖手机端 UI）
    let metas = match session_metas(&shared.app) {
        Ok(v) => v,
        Err(e) => return (axum::http::StatusCode::INTERNAL_SERVER_ERROR, e).into_response(),
    };
    let filtered: Vec<Value> = metas
        .into_iter()
        .filter(|m| {
            m.get("id")
                .and_then(|v| v.as_str())
                .map(|id| ctx.allows_session(id))
                .unwrap_or(false)
        })
        .collect();
    axum::Json(filtered).into_response()
}

async fn api_session_detail(
    axum::extract::Path(id): axum::extract::Path<String>,
    axum::extract::State(shared): axum::extract::State<BridgeShared>,
    headers: axum::http::HeaderMap,
    axum::extract::Query(q): axum::extract::Query<TokenQuery>,
    axum::extract::ConnectInfo(addr): axum::extract::ConnectInfo<std::net::SocketAddr>,
) -> axum::response::Response {
    use axum::response::IntoResponse;
    let ctx = match resolve_auth(&shared, &headers, &q, addr.ip()) {
        Ok(ctx) => ctx,
        Err(resp) => return resp,
    };
    if !ctx.allows_session(&id) {
        // 不存在与越权同回应（403 语义留给手机端判断「无权」）
        return (
            axum::http::StatusCode::FORBIDDEN,
            "该设备未授权查看此会话".to_string(),
        )
            .into_response();
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
    headers: axum::http::HeaderMap,
    axum::extract::ConnectInfo(addr): axum::extract::ConnectInfo<std::net::SocketAddr>,
) -> axum::response::Response {
    let ctx = match resolve_auth(&shared, &headers, &q, addr.ip()) {
        Ok(ctx) => ctx,
        Err(resp) => return resp,
    };
    ws.on_upgrade(move |socket| ws_connection(socket, shared, ctx))
}

/// 队列快照按 scope 过滤条目（camelCase QueuedMessage：sessionId 字段）
fn filter_queue_for_scope(items: Vec<Value>, ctx: &AuthCtx) -> Vec<Value> {
    let bound = ctx.forced_session();
    match bound {
        None => items,
        Some(_) => items
            .into_iter()
            .filter(|item| queue_item_in_scope(item, bound.as_deref()))
            .collect(),
    }
}

/// 单个队列条目是否在 scope 内：bound 为 None（all 连接）永真；
/// 条目缺 sessionId 视为不在任何会话内（scoped 拒绝）
fn queue_item_in_scope(item: &Value, bound: Option<&str>) -> bool {
    match bound {
        None => true,
        Some(b) => item.get("sessionId").and_then(|v| v.as_str()) == Some(b),
    }
}

/// 总线消息按连接 scope 过滤：返回 None 表示不下发。
/// all 连接原样返回；scoped 连接：会话相关事件按 session_id 过滤，
/// queue 按条目过滤，view 丢弃（PC 切走别的会话不应把手机拽走）
fn scoped_view_of_msg(msg: &Value, ctx: &AuthCtx) -> Option<Value> {
    if matches!(ctx.scope, crate::mobile_keys::KeyScope::All) {
        return Some(msg.clone());
    }
    let kind = msg.get("type").and_then(|v| v.as_str()).unwrap_or("");
    let bound = match ctx.forced_session() {
        Some(s) => s,
        None => return Some(msg.clone()),
    };
    match kind {
        "agent-event" | "run-settled" | "agent-question" => {
            let sid = msg
                .get("payload")
                .and_then(|p| p.get("session_id"))
                .and_then(|v| v.as_str())
                .unwrap_or("");
            if sid == bound {
                Some(msg.clone())
            } else {
                None
            }
        }
        "queue" => {
            let mut copy = msg.clone();
            let items = msg
                .get("items")
                .and_then(|v| v.as_array())
                .cloned()
                .unwrap_or_default();
            if let Some(obj) = copy.as_object_mut() {
                obj.insert(
                    "items".to_string(),
                    Value::Array(filter_queue_for_scope(items, ctx)),
                );
            }
            Some(copy)
        }
        "view" => None,
        _ => Some(msg.clone()),
    }
}

async fn ws_connection(socket: axum::extract::ws::WebSocket, shared: BridgeShared, ctx: AuthCtx) {
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
    // 配合磁盘历史缺失的窗口）；已结算的 Run 不下发（磁盘已有，回放会重复）。
    // scoped 连接：sessions/queue/live 按绑定会话过滤，current 强制 = 绑定 sid
    let queue: Vec<Value> = shared
        .queue
        .lock()
        .unwrap_or_else(|e| e.into_inner())
        .clone();
    let queue = filter_queue_for_scope(queue, &ctx);
    let current = match ctx.forced_session() {
        Some(bound) => Some(bound.clone()),
        None => shared
            .view
            .lock()
            .unwrap_or_else(|e| e.into_inner())
            .clone(),
    };
    let live = live_events_for_hello()
        .into_iter()
        .filter(|e| {
            e.get("session_id")
                .and_then(|v| v.as_str())
                .map(|sid| ctx.allows_session(sid))
                .unwrap_or(true)
        })
        .collect::<Vec<_>>();
    let sessions = session_metas(&shared.app)
        .unwrap_or_default()
        .into_iter()
        .filter(|m| {
            m.get("id")
                .and_then(|v| v.as_str())
                .map(|id| ctx.allows_session(id))
                .unwrap_or(false)
        })
        .collect::<Vec<_>>();
    let hello = json!({
        "type": "hello",
        "sessions": sessions,
        "queue": queue,
        "paired": paired_now,
        "current": current,
        "live": live,
        "scope": ctx.scope.tag(),
    });
    let _ = out_tx.send(hello.to_string()).await;

    // 持久 key 连接注册：撤销该 key 时丢 sender = 踢下线
    if let Some(key_id) = &ctx.key_id {
        if let Ok(mut map) = shared.conns.lock() {
            map.entry(key_id.clone()).or_default().push(out_tx.clone());
        }
    }

    // 总线 → 出站（逐连接按 scope 过滤：agent-event / run-settled /
    // agent-question 看 payload.session_id；queue 按条目 sessionId 过滤；
    // view 对 scoped 丢弃——PC 切走别的会话不应影响手机视线）
    let bus_tx = out_tx.clone();
    let fwd_ctx = ctx.clone();
    let forwarder = tokio::spawn(async move {
        loop {
            match bus_rx.recv().await {
                Ok(msg) => {
                    let forward = serde_json::from_str::<Value>(&msg)
                        .map(|v| scoped_view_of_msg(&v, &fwd_ctx))
                        .ok()
                        .flatten();
                    match forward {
                        Some(v) => {
                            if bus_tx.send(v.to_string()).await.is_err() {
                                break;
                            }
                        }
                        None => continue,
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
            Ok(Message::Text(text)) => handle_client_msg(&shared, &out_tx, &ctx, &text).await,
            Ok(_) => {}
            Err(_) => break,
        }
    }
    forwarder.abort();
    // 注销连接注册
    if let Some(key_id) = &ctx.key_id {
        if let Ok(mut map) = shared.conns.lock() {
            if let Some(list) = map.get_mut(key_id) {
                list.retain(|tx| !tx.same_channel(&out_tx));
                if list.is_empty() {
                    map.remove(key_id);
                }
            }
        }
    }
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
    ctx: &AuthCtx,
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
            // scoped 连接强制发进绑定会话（本层覆盖，不信客户端）
            let session_id = match ctx.forced_session() {
                Some(bound) => Some(bound),
                None => v["session_id"].as_str().map(|s| s.to_string()),
            };
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
            if !scoped_queue_op_allowed(shared, ctx, &v["id"]) {
                let _ = out_tx
                    .send(
                        json!({ "type": "toast", "text": "该设备只能操作绑定会话的队列" })
                            .to_string(),
                    )
                    .await;
                return;
            }
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
            if !scoped_queue_op_allowed(shared, ctx, &v["id"]) {
                let _ = out_tx
                    .send(
                        json!({ "type": "toast", "text": "该设备只能操作绑定会话的队列" })
                            .to_string(),
                    )
                    .await;
                return;
            }
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
            // scoped 连接只能处置本会话的待决策卡
            if !ctx.allows_session(
                pending_ctx()
                    .lock()
                    .ok()
                    .and_then(|m| m.get(&tool_call_id).cloned())
                    .as_deref()
                    .unwrap_or(""),
            ) {
                let _ = out_tx
                    .send(
                        json!({ "type": "toast", "text": "该设备未授权处置此会话的审批" })
                            .to_string(),
                    )
                    .await;
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
            if !ctx.allows_session(
                pending_ctx()
                    .lock()
                    .ok()
                    .and_then(|m| m.get(&question_id).cloned())
                    .as_deref()
                    .unwrap_or(""),
            ) {
                let _ = out_tx
                    .send(
                        json!({ "type": "toast", "text": "该设备未授权回答此会话的提问" })
                            .to_string(),
                    )
                    .await;
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

    // ===== 认证 / scope 过滤 =====

    /// 用唯一 IP 隔离限速全局状态（并行测试互不干扰）
    fn unique_ip(tag: &str) -> IpAddr {
        use std::net::Ipv4Addr;
        let n = tag.bytes().map(|b| b as u32).sum::<u32>() + std::process::id() as u32;
        IpAddr::V4(Ipv4Addr::from(0xA0000000u32 + (n % 0xFFFFFF)))
    }

    #[test]
    fn ip_guard_rate_limit_escalates_and_releases() {
        let ip = unique_ip("escalate");
        assert!(auth_blocked_remaining(ip).is_none());
        // 失败 1 次：立即封禁 backoff_secs(1) = 2s
        auth_failure(ip);
        let secs = auth_blocked_remaining(ip).expect("失败后应进入封禁");
        assert!(secs <= 2, "首次封禁应 ≈2s，实际 {secs}");
        // 成功后解除
        auth_success(ip);
        assert!(auth_blocked_remaining(ip).is_none(), "成功后应解除封禁");
        // 多次失败：封禁时长升级（用新 IP 避免与并行测试干扰）
        let ip2 = unique_ip("escalate2");
        for _ in 0..10 {
            auth_failure(ip2);
        }
        let secs2 = auth_blocked_remaining(ip2).expect("多次失败后应进入封禁");
        assert!(secs2 >= 60, "10 次失败后封禁应达分钟级，实际 {secs2}s");
    }

    #[test]
    fn ticket_roundtrip_single_use_and_ttl() {
        let ctx = AuthCtx {
            scope: crate::mobile_keys::KeyScope::Session {
                session_id: "s-t".into(),
            },
            key_id: Some("k1".into()),
        };
        let t = issue_ticket(ctx.clone());
        // 第一次消费命中
        let got = consume_ticket(&t).expect("票据应可消费");
        assert_eq!(got, ctx);
        // 单次使用：第二次拿不到
        assert!(consume_ticket(&t).is_none(), "票据应单次使用");
        // 随机票据拿不到
        assert!(consume_ticket("no-such-ticket").is_none());
    }

    #[test]
    fn scoped_msg_view_filters_events_and_queue() {
        let all = AuthCtx::all();
        let scoped = AuthCtx {
            scope: crate::mobile_keys::KeyScope::Session {
                session_id: "s-a".into(),
            },
            key_id: None,
        };
        let ev_a =
            json!({ "type": "agent-event", "payload": { "session_id": "s-a", "text": "x" } });
        let ev_b =
            json!({ "type": "agent-event", "payload": { "session_id": "s-b", "text": "y" } });
        // all 连接原样
        assert_eq!(scoped_view_of_msg(&ev_a, &all), Some(ev_a.clone()));
        // scoped：本会话放行、他全会话拦截
        assert_eq!(scoped_view_of_msg(&ev_a, &scoped), Some(ev_a.clone()));
        assert_eq!(scoped_view_of_msg(&ev_b, &scoped), None);
        // queue：按条目 sessionId 过滤
        let queue = json!({ "type": "queue", "items": [
            { "id": "m1", "sessionId": "s-a" },
            { "id": "m2", "sessionId": "s-b" }
        ] });
        let got = scoped_view_of_msg(&queue, &scoped).expect("queue 应放行但过滤条目");
        let items = got["items"].as_array().unwrap();
        assert_eq!(items.len(), 1);
        assert_eq!(items[0]["id"], "m1");
        // view：scoped 丢弃（PC 切走别的会话不拉手机）
        assert_eq!(
            scoped_view_of_msg(&json!({ "type": "view", "session_id": "s-b" }), &scoped),
            None
        );
        // toast / pair 等无关消息放行
        assert!(scoped_view_of_msg(&json!({ "type": "toast", "text": "hi" }), &scoped).is_some());
        assert!(scoped_view_of_msg(&json!({ "type": "pair", "count": 1 }), &scoped).is_some());
    }

    #[test]
    fn queue_item_scope_and_op_enforcement() {
        let m1 = json!({ "id": "m1", "sessionId": "s-a" });
        let m2 = json!({ "id": "m2", "sessionId": "s-b" });
        let no_sid = json!({ "id": "m3" });
        // all 连接（bound None）永真
        assert!(queue_item_in_scope(&m1, None));
        assert!(queue_item_in_scope(&no_sid, None));
        // scoped：绑定会话内放行，他全拒绝，缺 sessionId 拒绝
        assert!(queue_item_in_scope(&m1, Some("s-a")));
        assert!(!queue_item_in_scope(&m2, Some("s-a")));
        assert!(!queue_item_in_scope(&no_sid, Some("s-a")));
    }

    #[test]
    fn pending_ctx_records_and_caps() {
        record_pending_ctx("q1", "s-1");
        record_pending_ctx("q2", "s-2");
        {
            let map = pending_ctx().lock().unwrap();
            assert_eq!(map.get("q1").map(|s| s.as_str()), Some("s-1"));
            assert_eq!(map.get("q2").map(|s| s.as_str()), Some("s-2"));
        }
        // 超限整体清空后重建
        for i in 0..(PENDING_CTX_CAP + 10) {
            record_pending_ctx(&std::format!("p{i}"), "s-x");
        }
        let map = pending_ctx().lock().unwrap();
        assert!(map.len() <= PENDING_CTX_CAP, "超限后不应继续膨胀");
    }
}
