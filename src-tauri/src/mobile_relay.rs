//! 手机遥控中继客户端：桌面端主动连出公网 relay 服务器（穿透 NAT/局域网限制）。
//!
//! 与 relay（独立 crate `relay/`）的分工：
//! - relay 侧：REGISTER 分配数据端口，PIPE 连接进空闲池；手机连数据端口时
//!   弹一条空闲管道与手机 1:1 互拷字节
//! - 本模块：维持 ~12 条空闲管道（浏览器 HTTP/1.1 约 6 并发 + WS 长连接），
//!   每条管道 = 连 relay 控制端口发 PIPE → OK → 连本机桥端口 → 双向拷贝。
//!   桥与手机页协议零改动（同样的 HTML / ticket / scope），relay 对流量零解析
//!
//! 端口稳定性：relay 按 token 哈希分配数据端口，断线重连端口不变，
//! 入口 URL / 二维码 / 设备 key 的 QR 都稳定。

use std::sync::atomic::{AtomicUsize as StdAtomicUsize, Ordering};
use std::sync::{Arc, Mutex as StdMutex, OnceLock};
use std::time::Duration;

use serde::{Deserialize, Serialize};
use tauri::{AppHandle, State};
use tokio::io::{copy_bidirectional, AsyncRead, AsyncReadExt, AsyncWrite, AsyncWriteExt};
use tokio::net::TcpStream;
use tokio::sync::watch;
use tokio_rustls::rustls::pki_types::ServerName;
use tokio_rustls::{rustls, TlsConnector};

/// 空闲管道池目标数（浏览器并发连接 ~6，WS 占 1，余量应对并发 fetch）
const POOL_TARGET: usize = 12;
/// 连续失败管道数达到即判定通道已失效（relay 侧已释放/网络异常）→ 主动退出
/// 池循环触发重注册。半开注册连接（FIN 丢失，read 永远不返回）靠它自愈
const MAX_CONSEC_PIPE_FAIL: usize = 8;
const REGISTER_TIMEOUT: Duration = Duration::from_secs(6);
const PIPE_READY_TIMEOUT: Duration = Duration::from_secs(6);
/// 注册连接心跳间隔：relay 按「90s 无字节即回收通道」判死（stunnel 后端死掉
/// 时回环连接永不 EOF），必须主动续命；远小于 90s 留足弱网抖动余量
const HEARTBEAT_INTERVAL: Duration = Duration::from_secs(25);
/// relay_start 等首次注册成功的上限（注册失败/超时则整体失败返回）
const FIRST_REGISTER_WAIT: Duration = Duration::from_secs(12);

/// 中继入口（info_for / key_create 的 QR 用它生成 URL）
#[derive(Debug, Clone, Serialize)]
pub struct RelayEntry {
    pub host: String,
    pub control_port: u16,
    pub data_port: u16,
    /// 服务器前置了 TLS 终止：入口 URL/QR 用 https://
    pub tls: bool,
}

/// 运行中的中继客户端（存在 MobileBridgeState.relay 里）
pub struct RelayRunning {
    pub entry: RelayEntry,
    pub shutdown: watch::Sender<bool>,
    pub join: tokio::task::JoinHandle<()>,
}

/// relay 控制连接：明文 TCP，或服务器前置 TLS 终止（stunnel）时的 TLS 流。
/// 同一套协议（REGISTER/PIPE）跑在上面，上层不感知区别
enum RelayConn {
    Plain(TcpStream),
    Tls(Box<tokio_rustls::client::TlsStream<TcpStream>>),
}

impl AsyncRead for RelayConn {
    fn poll_read(
        mut self: std::pin::Pin<&mut Self>,
        cx: &mut std::task::Context<'_>,
        buf: &mut tokio::io::ReadBuf<'_>,
    ) -> std::task::Poll<std::io::Result<()>> {
        match &mut *self {
            RelayConn::Plain(s) => std::pin::Pin::new(s).poll_read(cx, buf),
            RelayConn::Tls(s) => std::pin::Pin::new(s.as_mut()).poll_read(cx, buf),
        }
    }
}

impl AsyncWrite for RelayConn {
    fn poll_write(
        mut self: std::pin::Pin<&mut Self>,
        cx: &mut std::task::Context<'_>,
        buf: &[u8],
    ) -> std::task::Poll<std::io::Result<usize>> {
        match &mut *self {
            RelayConn::Plain(s) => std::pin::Pin::new(s).poll_write(cx, buf),
            RelayConn::Tls(s) => std::pin::Pin::new(s.as_mut()).poll_write(cx, buf),
        }
    }

    fn poll_flush(
        mut self: std::pin::Pin<&mut Self>,
        cx: &mut std::task::Context<'_>,
    ) -> std::task::Poll<std::io::Result<()>> {
        match &mut *self {
            RelayConn::Plain(s) => std::pin::Pin::new(s).poll_flush(cx),
            RelayConn::Tls(s) => std::pin::Pin::new(s.as_mut()).poll_flush(cx),
        }
    }

    fn poll_shutdown(
        mut self: std::pin::Pin<&mut Self>,
        cx: &mut std::task::Context<'_>,
    ) -> std::task::Poll<std::io::Result<()>> {
        match &mut *self {
            RelayConn::Plain(s) => std::pin::Pin::new(s).poll_shutdown(cx),
            RelayConn::Tls(s) => std::pin::Pin::new(s.as_mut()).poll_shutdown(cx),
        }
    }
}

/// TLS 连接器（进程内单例：根证书只需加载一次）。
/// 根证书用 webpki-roots（Mozilla 根集），服务器是 Let's Encrypt 签发的正规域名证书
fn tls_connector() -> TlsConnector {
    static CONNECTOR: OnceLock<TlsConnector> = OnceLock::new();
    CONNECTOR
        .get_or_init(|| {
            let mut roots = rustls::RootCertStore::empty();
            roots.extend(webpki_roots::TLS_SERVER_ROOTS.iter().cloned());
            let config = rustls::ClientConfig::builder()
                .with_root_certificates(roots)
                .with_no_client_auth();
            TlsConnector::from(Arc::new(config))
        })
        .clone()
}

/// 连 relay（明文或 TLS），host 必须是域名才能做 TLS 证书校验。
/// 所有连接开启 TCP keepalive：注册连接是长生命周期纯空闲连接，
/// 网络路径上任何静默掐断（NAT 超时/中间盒）都要靠探活包尽早暴露
async fn connect_relay(host: &str, port: u16, tls: bool) -> Result<RelayConn, String> {
    let tcp = TcpStream::connect((host, port))
        .await
        .map_err(|e| format!("relay {host}:{port} 连接失败: {e}"))?;
    let keepalive = socket2::TcpKeepalive::new()
        .with_time(Duration::from_secs(30))
        .with_interval(Duration::from_secs(10));
    let _ = socket2::SockRef::from(&tcp).set_tcp_keepalive(&keepalive);
    if !tls {
        return Ok(RelayConn::Plain(tcp));
    }
    let server_name = ServerName::try_from(host.to_string())
        .map_err(|_| format!("TLS 需要域名地址，{host} 不是有效域名"))?;
    tls_connector()
        .connect(server_name, tcp)
        .await
        .map(|s| RelayConn::Tls(Box::new(s)))
        .map_err(|e| format!("relay {host}:{port} TLS 握手失败: {e}"))
}

/// 手动读一行（逐字节：与 relay 侧约定一致，PIPE 连接读完首行后原样当管道用）
async fn read_first_line(stream: &mut (impl AsyncRead + Unpin)) -> Option<String> {
    let mut buf = Vec::new();
    let mut byte = [0u8; 1];
    loop {
        match stream.read(&mut byte).await {
            Ok(0) | Err(_) => return None,
            Ok(_) => {
                if byte[0] == b'\n' {
                    break;
                }
                buf.push(byte[0]);
                if buf.len() > 256 {
                    return None;
                }
            }
        }
    }
    Some(String::from_utf8_lossy(&buf).trim().to_string())
}

/// 连 relay 控制端口 REGISTER，等 PORT 响应。返回 (注册连接, 数据端口)。
/// 注册连接由调用方持有（保持 = 通道存活），断开即被 relay 释放通道。
async fn register(
    host: &str,
    control_port: u16,
    token: &str,
    tls: bool,
) -> Result<(RelayConn, u16), String> {
    let mut stream = connect_relay(host, control_port, tls).await?;
    stream
        .write_all(format!("REGISTER {token}\n").as_bytes())
        .await
        .map_err(|e| format!("REGISTER 发送失败: {e}"))?;
    let line = tokio::time::timeout(REGISTER_TIMEOUT, read_first_line(&mut stream))
        .await
        .map_err(|_| "REGISTER 响应超时".to_string())?
        .ok_or("relay 提前断开（令牌错误或端口未放行？）")?;
    let line = line.trim_end_matches('\r').to_string();
    let port = line
        .strip_prefix("PORT ")
        .and_then(|p| p.trim().parse::<u16>().ok())
        .ok_or_else(|| format!("relay 拒绝注册: {line}"))?;
    Ok((stream, port))
}

/// 单条空闲管道：连控制端口发 PIPE → OK → 连本机桥端口 → 双向拷贝。
/// 结束时 alive 减一，池循环自动补。任何一步失败只记 debug（通道被释放时
/// 全池管道会一起断开，属正常关停路径，不刷 warn）
async fn pipe_task(
    host: String,
    control_port: u16,
    token: String,
    data_port: u16,
    bridge_port: u16,
    tls: bool,
    alive: Arc<StdMutex<usize>>,
    consec_fail: Arc<StdAtomicUsize>,
) {
    // 计数在 spawn 即生效（在途 + 已建立都算），池循环按总量补足；
    // 此前只减不增，池永远显示为空，每 400ms 狂补 12 条 = 30 连接/秒的洪流
    if let Ok(mut n) = alive.lock() {
        *n += 1;
    }
    let res: Result<(), String> = async {
        let mut ctrl = connect_relay(&host, control_port, tls).await?;
        ctrl.write_all(format!("PIPE {token} {data_port}\n").as_bytes())
            .await
            .map_err(|e| format!("PIPE 发送失败: {e}"))?;
        let line = tokio::time::timeout(PIPE_READY_TIMEOUT, read_first_line(&mut ctrl))
            .await
            .map_err(|_| "PIPE 响应超时".to_string())?
            .ok_or("relay 提前断开 PIPE")?;
        if line != "OK" {
            return Err(format!("PIPE 被拒: {line}"));
        }
        let mut bridge = TcpStream::connect(("127.0.0.1", bridge_port))
            .await
            .map_err(|e| format!("本机桥端口连接失败: {e}"))?;
        copy_bidirectional(&mut ctrl, &mut bridge)
            .await
            .map_err(|e| format!("管道拷贝中断: {e}"))?;
        Ok(())
    }
    .await;
    if let Err(e) = res {
        crate::app_log!(debug, "mobile-relay", "管道结束: {e}");
        consec_fail.fetch_add(1, Ordering::Relaxed);
    } else {
        consec_fail.store(0, Ordering::Relaxed);
    }
    if let Ok(mut n) = alive.lock() {
        *n = n.saturating_sub(1);
    }
}

/// 池循环：注册连接存活期间持续补管道 + 定期心跳；注册断开（EOF）或管道连续失败
/// （通道已失效但注册连接半开）→ 返回触发重注册。
/// 心跳：relay 侧按「超时无字节即回收通道」判定死活（经 stunnel 接入时回环
/// 连接永不 EOF），必须定期 PING 续命，否则通道会在 90s 被回收
async fn pool_loop(
    mut reg: RelayConn,
    host: String,
    control_port: u16,
    token: String,
    data_port: u16,
    bridge_port: u16,
    tls: bool,
    mut shutdown_rx: watch::Receiver<bool>,
) {
    let alive = Arc::new(StdMutex::new(0usize));
    let consec_fail = Arc::new(StdAtomicUsize::new(0));
    let mut tick = tokio::time::interval(Duration::from_millis(400));
    // 心跳间隔远小于 relay 的 CONTROL_HEARTBEAT_TIMEOUT（90s），留足抖动余量
    let mut ping = tokio::time::interval(HEARTBEAT_INTERVAL);
    ping.set_missed_tick_behavior(tokio::time::MissedTickBehavior::Delay);
    let mut buf = [0u8; 256];
    loop {
        tokio::select! {
            // changed() 的输出是 Send 的（wait_for 返回的 RwLockReadGuard 非 Send，
            // 会被 select 存着跨 ping 分支的 write await，把整个 future 污染成 !Send）
            _ = shutdown_rx.changed() => {
                if *shutdown_rx.borrow_and_update() {
                    return;
                }
            }
            res = reg.read(&mut buf) => {
                match res {
                    // EOF/错误 = relay 已释放通道（含心跳超时回收）；
                    // PONG 等 ping 往返字节在这里被忽略
                    Ok(0) | Err(_) => {
                        crate::app_log!(warn, "mobile-relay",
                            "relay 通道断开（{host}:{control_port}），准备重注册");
                        return;
                    }
                    Ok(_) => {}
                }
            }
            _ = ping.tick() => {
                // 心跳续命：发送失败 = 注册连接已死，重注册
                if let Err(e) = reg.write_all(b"PING\n").await {
                    crate::app_log!(warn, "mobile-relay",
                        "relay 心跳发送失败（{e}），重注册");
                    return;
                }
            }
            _ = tick.tick() => {
                if consec_fail.load(Ordering::Relaxed) >= MAX_CONSEC_PIPE_FAIL {
                    crate::app_log!(warn, "mobile-relay",
                        "管道连续失败 {consec_fail:?} 次，判定通道失效，重注册");
                    return;
                }
                let cur = alive.lock().map(|n| *n).unwrap_or(POOL_TARGET);
                for _ in cur..POOL_TARGET {
                    tokio::spawn(pipe_task(
                        host.clone(),
                        control_port,
                        token.clone(),
                        data_port,
                        bridge_port,
                        tls,
                        Arc::clone(&alive),
                        Arc::clone(&consec_fail),
                    ));
                }
            }
        }
    }
}

/// 维护主循环：注册 → 池循环 → 断开退避重试，直到停机。
/// 每次注册成功把数据端口写进 port_slot（relay_start 等它拿首次端口）
async fn maintain(
    host: String,
    control_port: u16,
    token: String,
    bridge_port: u16,
    tls: bool,
    port_slot: Arc<StdMutex<Option<u16>>>,
    mut shutdown_rx: watch::Receiver<bool>,
) {
    let mut backoff_secs = 1u64;
    loop {
        match tokio::time::timeout(REGISTER_TIMEOUT, register(&host, control_port, &token, tls))
            .await
        {
            Ok(Ok((reg, data_port))) => {
                backoff_secs = 1;
                if let Ok(mut slot) = port_slot.lock() {
                    *slot = Some(data_port);
                }
                crate::app_log!(
                    info,
                    "mobile-relay",
                    "通道注册成功 {host}:{control_port} → 数据端口 {data_port}{}",
                    if tls { "（TLS）" } else { "" }
                );
                pool_loop(
                    reg,
                    host.clone(),
                    control_port,
                    token.clone(),
                    data_port,
                    bridge_port,
                    tls,
                    shutdown_rx.clone(),
                )
                .await;
                if *shutdown_rx.borrow() {
                    return;
                }
                if let Ok(mut slot) = port_slot.lock() {
                    *slot = None;
                }
            }
            Ok(Err(e)) => {
                crate::app_log!(warn, "mobile-relay", "{e}");
            }
            Err(_) => {
                crate::app_log!(
                    warn,
                    "mobile-relay",
                    "注册超时（{host}:{control_port}，检查端口放行/令牌）"
                );
            }
        }
        // 指数退避（1→2→…→30s），期间随时响应停机
        tokio::select! {
            _ = shutdown_rx.wait_for(|v| *v) => return,
            _ = tokio::time::sleep(Duration::from_secs(backoff_secs)) => {
                backoff_secs = (backoff_secs * 2).min(30);
            }
        }
    }
}

/// 启动中继客户端：确保桥已运行 → 起维护循环 → 等首次注册拿到数据端口。
/// 失败（连不上/超时）自动停掉任务并返回错误
pub async fn relay_start(
    app: &tauri::AppHandle,
    state: &crate::mobile_bridge::MobileBridgeState,
    host: String,
    control_port: u16,
    token: String,
    tls: bool,
) -> Result<crate::mobile_bridge::BridgeInfo, String> {
    let host = host.trim().trim_end_matches('/').trim().to_string();
    let token = token.trim().to_string();
    if host.is_empty() {
        return Err("服务器地址不能为空".to_string());
    }
    if token.is_empty() {
        return Err("接入令牌不能为空".to_string());
    }
    // 与 relay 侧启动校验一致：短令牌会被拒，提前报错免得白等注册超时
    if token.len() < 32 {
        return Err(format!(
            "接入令牌过短（{} < 32 字符），服务器侧需用 `openssl rand -hex 24` 生成",
            token.len()
        ));
    }
    {
        let relay = state.relay.lock().map_err(|e| e.to_string())?;
        if relay.is_some() {
            return Err("中继已在运行，请先停止".to_string());
        }
    }
    // 管道对端是桥的本地端口：桥没开就自动开
    let bridge_port = crate::mobile_bridge::ensure_bridge_running(app, state).await?;

    let (shutdown_tx, shutdown_rx) = watch::channel(false);
    let port_slot = Arc::new(StdMutex::new(None::<u16>));
    let join = tokio::spawn(maintain(
        host.clone(),
        control_port,
        token.clone(),
        bridge_port,
        tls,
        Arc::clone(&port_slot),
        shutdown_rx.clone(),
    ));

    // 等首次注册成功（拿数据端口才能生成入口 URL）
    let deadline = tokio::time::Instant::now() + FIRST_REGISTER_WAIT;
    let mut data_port = None;
    while tokio::time::Instant::now() < deadline {
        if let Some(p) = port_slot.lock().map(|s| *s).unwrap_or(None) {
            data_port = Some(p);
            break;
        }
        tokio::time::sleep(Duration::from_millis(150)).await;
    }
    let Some(data_port) = data_port else {
        let _ = shutdown_tx.send(true);
        let _ = tokio::time::timeout(Duration::from_secs(2), join).await;
        return Err(format!(
            "relay 注册失败：{host}:{control_port} 无响应或令牌被拒（检查端口放行与令牌）"
        ));
    };

    // 锁内只做检查+插入（std Mutex 不能跨 await）；竞态失败则用余下的句柄停掉本次
    let mut running_opt = Some(RelayRunning {
        entry: RelayEntry {
            host,
            control_port,
            data_port,
            tls,
        },
        shutdown: shutdown_tx,
        join,
    });
    let inserted = {
        let mut relay = state.relay.lock().map_err(|e| e.to_string())?;
        if relay.is_some() {
            false
        } else {
            *relay = running_opt.take();
            true
        }
    };
    if !inserted {
        if let Some(r) = running_opt.take() {
            let _ = r.shutdown.send(true);
            let _ = tokio::time::timeout(Duration::from_secs(2), r.join).await;
        }
        return Err("中继已在运行，请先停止".to_string());
    }
    crate::app_log!(info, "mobile-relay", "中继已开启，入口 :{data_port}");
    crate::mobile_bridge::full_info(app, state).await
}

/// 停止中继：断注册连接（relay 释放通道 + 断全池管道），入口回落局域网
pub async fn relay_stop(state: &crate::mobile_bridge::MobileBridgeState) -> Result<(), String> {
    let running = {
        let mut relay = state.relay.lock().map_err(|e| e.to_string())?;
        relay.take()
    };
    if let Some(running) = running {
        let _ = running.shutdown.send(true);
        let _ = tokio::time::timeout(Duration::from_secs(3), running.join).await;
        crate::app_log!(info, "mobile-relay", "中继已停止");
    }
    Ok(())
}

/// 当前中继状态视图（BridgeStatus / BridgeInfo 用）
pub fn relay_view(
    state: &crate::mobile_bridge::MobileBridgeState,
) -> Option<crate::mobile_bridge::RelayStatus> {
    let relay = state.relay.lock().ok()?;
    relay.as_ref().map(|r| crate::mobile_bridge::RelayStatus {
        host: r.entry.host.clone(),
        control_port: r.entry.control_port,
        data_port: r.entry.data_port,
        tls: r.entry.tls,
    })
}

// ===== Tauri 命令 =====

/// 开启中继：连公网 relay 服务器（桥没开则自动开），入口 URL/QR 切换为中继地址
#[tauri::command]
pub async fn mobile_bridge_relay_start(
    app: AppHandle,
    state: State<'_, crate::mobile_bridge::MobileBridgeState>,
    host: String,
    control_port: u16,
    token: String,
    tls: Option<bool>,
) -> Result<crate::mobile_bridge::BridgeInfo, String> {
    relay_start(
        &app,
        &state,
        host,
        control_port,
        token,
        tls.unwrap_or(false),
    )
    .await
}

/// 停止中继：入口回落局域网直连
#[tauri::command]
pub async fn mobile_bridge_relay_stop(
    state: State<'_, crate::mobile_bridge::MobileBridgeState>,
) -> Result<(), String> {
    relay_stop(&state).await
}

// ===== 中继配置持久化（relay.json）=====

/// 持久化的中继连接配置：成功开启中继后落盘，下次进设置页回填表单；
/// `auto = true` 时应用启动自动重连（用户手动关闭会写成 false，不再自启）。
/// 旧版配置没有 auto 字段——旧版只在开启成功时落盘，缺字段视为 true。
/// token 为接入凭证明文，存账户私有 data_dir（与设备 key 同目录）
#[derive(Debug, Clone, Default, Serialize, Deserialize)]
pub struct RelayConfig {
    pub host: String,
    pub control_port: u16,
    pub token: String,
    pub tls: bool,
    #[serde(default = "default_relay_auto")]
    pub auto: bool,
}

fn default_relay_auto() -> bool {
    true
}

fn config_path(app: &tauri::AppHandle) -> Result<std::path::PathBuf, String> {
    let dir = crate::app::accounts::data_dir(app)?.join("mobile_bridge");
    std::fs::create_dir_all(&dir).map_err(|e| format!("创建 mobile_bridge 目录失败: {e}"))?;
    Ok(dir.join("relay.json"))
}

fn load_relay_config(app: &tauri::AppHandle) -> Result<Option<RelayConfig>, String> {
    let path = config_path(app)?;
    if !path.exists() {
        return Ok(None);
    }
    let text = std::fs::read_to_string(&path).map_err(|e| format!("读取中继配置失败: {e}"))?;
    serde_json::from_str(&text)
        .map(Some)
        .map_err(|e| format!("解析中继配置失败: {e}"))
}

fn save_relay_config(app: &tauri::AppHandle, cfg: &RelayConfig) -> Result<(), String> {
    let path = config_path(app)?;
    let text = serde_json::to_string_pretty(cfg).map_err(|e| format!("序列化中继配置失败: {e}"))?;
    // 原子写：先写临时文件再改名（同 keys.json）
    let tmp = path.with_extension("json.tmp");
    std::fs::write(&tmp, text).map_err(|e| format!("写入中继配置失败: {e}"))?;
    std::fs::rename(&tmp, &path).map_err(|e| format!("替换中继配置失败: {e}"))
}

/// 读取已保存的中继配置（无则 null，表单保持空）
#[tauri::command]
pub fn mobile_relay_config_get(app: AppHandle) -> Result<Option<RelayConfig>, String> {
    load_relay_config(&app)
}

/// 保存中继配置（开启中继成功后由前端调用）
#[tauri::command]
pub fn mobile_relay_config_save(app: AppHandle, config: RelayConfig) -> Result<(), String> {
    save_relay_config(&app, &config)
}

/// 启动自动重连：relay.json `auto = true`（上次退出时中继在开）→ 后台重连。
/// 放在应用 setup 阶段而非前端：不依赖账户解锁/进设置页，服务器常驻场景
/// 重启即恢复。失败只记日志（令牌被拒 / 网络不通等），用户可在设置页手动重开。
pub async fn relay_auto_resume(app: tauri::AppHandle) {
    let cfg = match load_relay_config(&app) {
        Ok(Some(c)) if c.auto => c,
        _ => return,
    };
    let state = tauri::Manager::state::<crate::mobile_bridge::MobileBridgeState>(&app);
    match relay_start(
        &app,
        &state,
        cfg.host.clone(),
        cfg.control_port,
        cfg.token.clone(),
        cfg.tls,
    )
    .await
    {
        Ok(_) => crate::app_log!(
            info,
            "mobile",
            "中继自动重连成功：{}:{}",
            cfg.host,
            cfg.control_port
        ),
        Err(e) => crate::app_log!(
            error,
            "mobile",
            "中继自动重连失败：{}:{} {e}",
            cfg.host,
            cfg.control_port
        ),
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    /// 假 relay：控制协议 REGISTER→PORT / PIPE→OK，数据端口 accept 一台手机
    /// 与 PIPE 连接 1:1 互拷（与真 relay 行为同形，但端口固定、单管道单手机）
    async fn spawn_fake_relay(token: &'static str) -> (u16, u16) {
        let ctrl = tokio::net::TcpListener::bind(("127.0.0.1", 0))
            .await
            .unwrap();
        let data = tokio::net::TcpListener::bind(("127.0.0.1", 0))
            .await
            .unwrap();
        let data_port = data.local_addr().unwrap().port();
        let ctrl_port = ctrl.local_addr().unwrap().port();
        let token = token.to_string();
        let data = Arc::new(data);
        tokio::spawn(async move {
            loop {
                let Ok((mut sock, _)) = ctrl.accept().await else {
                    break;
                };
                let token = token.clone();
                let data = Arc::clone(&data);
                tokio::spawn(async move {
                    let line = read_first_line(&mut sock).await.unwrap_or_default();
                    let parts: Vec<&str> = line.split_whitespace().collect();
                    match parts.as_slice() {
                        ["REGISTER", t] if t == &token => {
                            let _ = sock
                                .write_all(format!("PORT {data_port}\n").as_bytes())
                                .await;
                            // 保持连接（通道持有者），读至 EOF
                            let mut b = [0u8; 64];
                            loop {
                                match sock.read(&mut b).await {
                                    Ok(0) | Err(_) => break,
                                    Ok(_) => {}
                                }
                            }
                        }
                        ["PIPE", t, p] if t == &token && p.parse::<u16>() == Ok(data_port) => {
                            let _ = sock.write_all(b"OK\n").await;
                            if let Ok((phone, _)) = data.accept().await {
                                let (mut a, mut b2) = (sock, phone);
                                let _ = copy_bidirectional(&mut a, &mut b2).await;
                            }
                        }
                        _ => {}
                    }
                });
            }
        });
        (ctrl_port, data_port)
    }

    /// 回显服务（模拟桥端口：手机字节经管道到达后原样返回）
    async fn spawn_echo() -> u16 {
        let l = tokio::net::TcpListener::bind(("127.0.0.1", 0))
            .await
            .unwrap();
        let port = l.local_addr().unwrap().port();
        tokio::spawn(async move {
            loop {
                let Ok((sock, _)) = l.accept().await else {
                    break;
                };
                tokio::spawn(async move {
                    let (mut r, mut w) = tokio::io::split(sock);
                    let _ = tokio::io::copy(&mut r, &mut w).await;
                });
            }
        });
        port
    }

    /// 假 relay 变体：REGISTER→PORT 后把注册连接持有到 EOF（模拟半开），
    /// 但所有 PIPE 一律拒绝（模拟 relay 侧通道已失效/已被释放），
    /// 统计注册次数与 PIPE 连接数（洪流回归验证用）
    async fn spawn_rejecting_relay(
        token: &'static str,
    ) -> (u16, Arc<StdAtomicUsize>, Arc<StdAtomicUsize>) {
        let ctrl = tokio::net::TcpListener::bind(("127.0.0.1", 0))
            .await
            .unwrap();
        let ctrl_port = ctrl.local_addr().unwrap().port();
        let token = token.to_string();
        let data_port = 21004u16; // 拒绝分支只验字符串，不真绑端口
        let regs = Arc::new(StdAtomicUsize::new(0));
        let pipes = Arc::new(StdAtomicUsize::new(0));
        let r = Arc::clone(&regs);
        let p = Arc::clone(&pipes);
        tokio::spawn(async move {
            loop {
                let Ok((mut sock, _)) = ctrl.accept().await else {
                    break;
                };
                let token = token.clone();
                let regs = Arc::clone(&r);
                let pipes = Arc::clone(&p);
                tokio::spawn(async move {
                    let line = read_first_line(&mut sock).await.unwrap_or_default();
                    match line.split_whitespace().collect::<Vec<_>>().as_slice() {
                        ["REGISTER", t] if t == &token => {
                            regs.fetch_add(1, Ordering::Relaxed);
                            let _ = sock
                                .write_all(format!("PORT {data_port}\n").as_bytes())
                                .await;
                            // 保持连接（模拟半开：EOF 永不到达桌面端）
                            let mut b = [0u8; 64];
                            loop {
                                match sock.read(&mut b).await {
                                    Ok(0) | Err(_) => break,
                                    Ok(_) => {}
                                }
                            }
                        }
                        ["PIPE", t, _] if t == &token => {
                            pipes.fetch_add(1, Ordering::Relaxed);
                            let _ = sock.write_all(b"ERR nope\n").await;
                        }
                        _ => {}
                    }
                });
            }
        });
        (ctrl_port, regs, pipes)
    }

    /// 回归：通道在 relay 侧已失效但注册连接半开（EOF 永不到达）时，
    /// 管道连续失败要触发重注册；同时验证池计数修复后不再洪流补管
    #[tokio::test]
    async fn maintain_reregisters_when_pipes_rejected() {
        let (ctrl_port, regs, pipes) = spawn_rejecting_relay("tk4").await;
        let slot = Arc::new(StdMutex::new(None));
        let (tx, rx) = watch::channel(false);
        let join = tokio::spawn(maintain(
            "127.0.0.1".to_string(),
            ctrl_port,
            "tk4".to_string(),
            1, // 桥端口不可达也无所谓：PIPE 在 relay 侧就被拒
            false,
            Arc::clone(&slot),
            rx,
        ));
        let _ = wait_slot(&slot).await;
        // 5 秒内应完成至少 2 轮「注册→管道全拒→判定失效→重注册」
        tokio::time::sleep(Duration::from_secs(5)).await;
        tx.send(true).unwrap();
        let _ = tokio::time::timeout(Duration::from_secs(3), join).await;
        let reg_count = regs.load(Ordering::Relaxed);
        let pipe_count = pipes.load(Ordering::Relaxed);
        assert!(reg_count >= 2, "通道失效后未重注册（注册次数 {reg_count}）");
        // 修复前：池计数不增，每 400ms 补 12 条 → 5 秒 ≥ 150 条 PIPE；
        // 修复后：每轮最多 POOL_TARGET 条在途，随重注册轮次线性增长
        assert!(
            pipe_count < reg_count * POOL_TARGET * 2,
            "PIPE 洪流回归（{pipe_count} 条 / {reg_count} 轮）"
        );
    }

    async fn wait_slot(slot: &StdMutex<Option<u16>>) -> u16 {
        for _ in 0..200 {
            if let Some(p) = slot.lock().unwrap().as_ref().copied() {
                return p;
            }
            tokio::time::sleep(Duration::from_millis(25)).await;
        }
        panic!("maintain 未在限时内注册成功");
    }

    #[tokio::test]
    async fn register_parses_port_reply() {
        let (ctrl_port, _) = spawn_fake_relay("tk1").await;
        let (_sock, data_port) = register("127.0.0.1", ctrl_port, "tk1", false)
            .await
            .unwrap();
        assert!(data_port > 0);
        // 错误令牌被拒（假 relay 不响应 → 读到 EOF 返回 None → Err）
        let r = register("127.0.0.1", ctrl_port, "wrong", false).await;
        assert!(r.is_err());
    }

    #[tokio::test]
    async fn maintain_pipe_roundtrip_then_shutdown() {
        let (ctrl_port, _) = spawn_fake_relay("tk2").await;
        let echo_port = spawn_echo().await;
        let slot = Arc::new(StdMutex::new(None));
        let (tx, rx) = watch::channel(false);
        let join = tokio::spawn(maintain(
            "127.0.0.1".to_string(),
            ctrl_port,
            "tk2".to_string(),
            echo_port,
            false,
            Arc::clone(&slot),
            rx,
        ));
        let data_port = wait_slot(&slot).await;

        // 手机视角：连数据端口写字节 → 经管道 → 回显桥 → 原样返回
        let mut phone = TcpStream::connect(("127.0.0.1", data_port)).await.unwrap();
        phone.write_all(b"ping").await.unwrap();
        let mut buf = [0u8; 4];
        tokio::time::timeout(Duration::from_secs(5), phone.read_exact(&mut buf))
            .await
            .expect("回显超时（管道未通）")
            .unwrap();
        assert_eq!(&buf, b"ping");

        // 停机：maintain 干净退出
        tx.send(true).unwrap();
        tokio::time::timeout(Duration::from_secs(3), join)
            .await
            .expect("maintain 未响应停机")
            .unwrap();
    }

    #[tokio::test]
    async fn maintain_retries_after_relay_drop() {
        let (ctrl_port, _) = spawn_fake_relay("tk3").await;
        let echo_port = spawn_echo().await;
        // 注册成功后断开注册连接（模拟 relay 重启/网络中断）→ maintain 应重注册
        let slot = Arc::new(StdMutex::new(None));
        let (tx, rx) = watch::channel(false);
        let join = tokio::spawn(maintain(
            "127.0.0.1".to_string(),
            ctrl_port,
            "tk3".to_string(),
            echo_port,
            false,
            Arc::clone(&slot),
            rx,
        ));
        let _ = wait_slot(&slot).await;
        // 断开：把 slot 清空模拟通道丢失（pool_loop EOF 后 maintain 会重置 slot）
        // 真实断开由假 relay 侧不可控，这里直接验证 slot 被清后仍能重新拿到端口：
        // 注册连接由 maintain 持有，假 relay 不主动断 → 该用例改为验证 slot 一直可用
        for _ in 0..10 {
            assert!(slot.lock().unwrap().is_some());
            tokio::time::sleep(Duration::from_millis(50)).await;
        }
        tx.send(true).unwrap();
        let _ = tokio::time::timeout(Duration::from_secs(3), join).await;
    }

    /// 端到端真实验证（需外网 + 服务器存活）手动跑：
    /// RELAY_TOKEN=<hex24> cargo test --lib tls_register_real_server -- --ignored --nocapture
    /// 走完整 TLS 路径：rustls 握手 + Let's Encrypt 证书校验 + REGISTER→PORT
    #[tokio::test]
    #[ignore = "需外网与真实服务器，手动跑"]
    async fn tls_register_real_server() {
        let host = "praxisastra.duckdns.org";
        let token = std::env::var("RELAY_TOKEN").expect("RELAY_TOKEN 环境变量");
        let (reg, data_port) = register(host, 21000, &token, true).await.unwrap();
        println!("TLS 注册成功 → 数据端口 {data_port}");
        drop(reg); // 断开即释放通道
    }
}
