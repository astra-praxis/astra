//! astra-relay — Astra 手机遥控桥的中继服务器（独立部署在公网/可达服务器上）。
//!
//! 设计（TCP 池式管道，对流量零解析）：
//! - 桌面端主动连出（穿透 NAT），第一条连接 REGISTER 注册通道 → 分配一个
//!   数据端口；之后维持若干条 PIPE 空闲管道连接
//! - 手机连数据端口（与桌面端桥之间的真实协议照旧：页面 / ticket 认证 / WS
//!   都由桌面端桥自己处理），relay 弹一条空闲管道与手机 1:1 互拷字节
//! - relay 不解析、不缓存任何流量内容；桌面端桥的凭证体系（临时 token /
//!   设备 key / 票据 / 限速）原样生效。**信任边界**：relay 不做任何应用层
//!   防护，认证/限速/防重放全部由桌面端桥自己承担
//!
//! 安全与健壮性约束：
//! - 令牌 ≥32 字符（启动强制），恒时比较（含长度填充）
//! - 控制连接握手 10s 超时；控制/手机并发连接各自有 Semaphore 上限
//! - 管道池双重上限（每通道 64 / 全局 4096），池内管道 30s 空闲淘汰
//! - 数据端口 accept 不阻塞（每台手机独立 spawn 等管道，最多 15s）
//! - 注册连接 TCP keepalive，防半开悬挂导致通道永不释放
//! - 通道记录 owner token，释放/供管道时校验，防误删他通道
//! - 明文 TCP：公网部署必须前置 TLS 终止（nginx stream / stunnel / Caddy），
//!   否则令牌与全部遥控流量可被嗅探（见 README）
//!
//! 控制协议（首行文本，\n 结尾，之后注册/管道连接均为纯字节流）：
//!   REGISTER <token>\n → PORT <数据端口>\n
//!   PIPE <token> <端口>\n → OK\n | ERR pool full\n | ERR unknown port\n
//!
//! 环境变量：
//!   ASTRA_RELAY_TOKEN        桌面端接入令牌（必填，≥32 字符）
//!   ASTRA_RELAY_BIND         监听地址（默认 0.0.0.0；TLS 终止部署时设 127.0.0.1，
//!                            让 TLS 层绑公网同一批端口，relay 只收回环）
//!   ASTRA_RELAY_CONTROL_PORT 控制端口（默认 21000）
//!   ASTRA_RELAY_DATA_BASE    数据端口起始（默认 21001）
//!   ASTRA_RELAY_DATA_END     数据端口结束（默认 21099，含）
//!   ASTRA_RELAY_LOG_DIR      日志目录（不设则只输出 stderr；设了则按天滚动落盘）
//!   ASTRA_RELAY_LOG_LEVEL    日志级别（默认 info，支持 trace/debug/info/warn/error）
//!
//! 用法：cargo build --release 后把 target/release/astra-relay 放到服务器运行
//!（放行控制端口与数据端口段），桌面端「手机遥控」页填服务器地址 + 控制端口 +
//! 此令牌即可开启中继。

use std::collections::{HashMap, HashSet};
use std::sync::{Arc, Mutex};
use std::time::{Duration, Instant};

use tokio::io::{copy_bidirectional, AsyncReadExt, AsyncWriteExt};
use tokio::net::{TcpListener, TcpStream};
use tokio::sync::Semaphore;
use tracing::{debug, error, info, warn};
use tracing_subscriber::EnvFilter;

/// 每通道空闲管道上限（桌面端池目标 12，留 5x 余量）
const MAX_PIPES_PER_CHANNEL: usize = 64;
/// 全局空闲管道总数上限（多通道叠加防 fd 耗尽）
const MAX_PIPES_TOTAL: usize = 4096;
/// 池内管道空闲淘汰时限：超龄管道被取用时直接丢弃（桌面端会自动补）
const PIPE_IDLE_TIMEOUT: Duration = Duration::from_secs(30);
/// 控制连接握手（读首行）超时：防慢速握手占住任务与 fd
const HANDSHAKE_TIMEOUT: Duration = Duration::from_secs(10);
/// 手机连接等空闲管道的最长时间（桌面端池每 400ms 补充）
const PHONE_PIPE_WAIT: Duration = Duration::from_secs(15);
/// 接入令牌最小长度
const TOKEN_MIN_LEN: usize = 32;
/// 并发控制连接上限（REGISTER + PIPE 都算）
const MAX_CONTROL_CONNS: usize = 512;
/// 并发「手机等管道」上限（公网数据端口任何人可连）
const MAX_PHONE_WAITS: usize = 512;

/// 池内空闲管道：进池时间戳（取用时超龄即弃）
struct IdlePipe {
    stream: TcpStream,
    since: Instant,
}

impl IdlePipe {
    fn is_stale(&self) -> bool {
        self.since.elapsed() > PIPE_IDLE_TIMEOUT
    }
}

/// 通道：注册连接（持有者）+ 空闲管道池 + accept 任务句柄 + owner 令牌
struct Channel {
    owner: String,
    pipes: Vec<IdlePipe>,
    accept_task: tokio::task::JoinHandle<()>,
}

struct Registry {
    /// 数据端口 → 通道
    channels: HashMap<u16, Channel>,
    /// 全局空闲管道总数（与各通道 pipes.len() 之和保持一致）
    pipes_total: usize,
}

type SharedRegistry = Arc<Mutex<Registry>>;

/// TCP keepalive：注册连接对端掉线无 FIN（半开）时靠内核探测触发 EOF，
/// 否则通道永不释放、数据端口永久占用
fn set_keepalive(stream: &TcpStream) {
    let ka = socket2::TcpKeepalive::new()
        .with_time(Duration::from_secs(60))
        .with_interval(Duration::from_secs(30));
    let _ = socket2::SockRef::from(stream).set_tcp_keepalive(&ka);
}

/// 初始化日志：默认输出 stderr；设了 ASTRA_RELAY_LOG_DIR 则同时按天滚动落盘
/// （astra-relay.log.<日期>）。返回的 guard 必须在 main 里持有到进程结束，
/// 否则后台写线程被提前回收会丢日志。
fn init_logging() -> tracing_appender::non_blocking::WorkerGuard {
    let level = std::env::var("ASTRA_RELAY_LOG_LEVEL").unwrap_or_else(|_| "info".to_string());
    let filter = EnvFilter::new(level);
    match std::env::var("ASTRA_RELAY_LOG_DIR") {
        Ok(dir) if !dir.trim().is_empty() => {
            let appender = tracing_appender::rolling::daily(dir.trim(), "astra-relay.log");
            let (writer, guard) = tracing_appender::non_blocking(appender);
            // 落盘的同时保留 stderr 输出（systemd/journald 也能收到一份）
            tracing_subscriber::fmt()
                .with_env_filter(filter)
                .with_ansi(false)
                .with_writer(writer)
                .init();
            guard
        }
        _ => {
            tracing_subscriber::fmt()
                .with_env_filter(filter)
                .with_ansi(false)
                .init();
            let (_, guard) = tracing_appender::non_blocking(std::io::stderr());
            guard
        }
    }
}

fn main() {
    let _log_guard = init_logging();
    let token = std::env::var("ASTRA_RELAY_TOKEN").unwrap_or_default();
    if token.is_empty() {
        error!("环境变量 ASTRA_RELAY_TOKEN 未设置：桌面端接入令牌必填（防任意注册）");
        std::process::exit(1);
    }
    if token.trim().len() < TOKEN_MIN_LEN {
        error!(
            "ASTRA_RELAY_TOKEN 过短（{} < {TOKEN_MIN_LEN} 字符）：弱令牌会被公网爆破，\
             请用 `openssl rand -hex 24` 生成",
            token.trim().len()
        );
        std::process::exit(1);
    }
    let token = token.trim().to_string();
    let bind: String = std::env::var("ASTRA_RELAY_BIND")
        .ok()
        .map(|v| v.trim().to_string())
        .filter(|v| !v.is_empty())
        .unwrap_or_else(|| "0.0.0.0".to_string());
    let control_port: u16 = std::env::var("ASTRA_RELAY_CONTROL_PORT")
        .ok()
        .and_then(|v| v.parse().ok())
        .unwrap_or(21000);
    let data_base: u16 = std::env::var("ASTRA_RELAY_DATA_BASE")
        .ok()
        .and_then(|v| v.parse().ok())
        .unwrap_or(21001);
    let data_end: u16 = std::env::var("ASTRA_RELAY_DATA_END")
        .ok()
        .and_then(|v| v.parse().ok())
        .unwrap_or(21099);
    if data_end < data_base {
        error!("ASTRA_RELAY_DATA_END ({data_end}) 不能小于 ASTRA_RELAY_DATA_BASE ({data_base})");
        std::process::exit(1);
    }
    if data_base == 0 {
        error!("ASTRA_RELAY_DATA_BASE 不能为 0（端口 0 = 随机分配，数据端口必须确定）");
        std::process::exit(1);
    }
    // 端口段长度用 u64 算（u16 差值 +1 在 base=0/end=65535 时会溢出成 0 → 取模 panic）
    let data_range = u64::from(data_end) - u64::from(data_base) + 1;

    let registry: SharedRegistry = Arc::new(Mutex::new(Registry {
        channels: HashMap::new(),
        pipes_total: 0,
    }));
    let control_sema = Arc::new(Semaphore::new(MAX_CONTROL_CONNS));
    let phone_sema = Arc::new(Semaphore::new(MAX_PHONE_WAITS));

    let rt = tokio::runtime::Builder::new_multi_thread()
        .enable_all()
        .build()
        .expect("构建 tokio runtime 失败");
    rt.block_on(async move {
        let listener = TcpListener::bind((bind.as_str(), control_port))
            .await
            .unwrap_or_else(|e| {
                error!("控制端口 {bind}:{control_port} 绑定失败: {e}");
                std::process::exit(1);
            });
        info!(
            "astra-relay 就绪：监听 {bind}，控制端口 {control_port}，数据端口 {data_base}-{data_end}\
             （管道上限 {MAX_PIPES_TOTAL}，公网部署请前置 TLS）"
        );
        loop {
            let (stream, peer) = match listener.accept().await {
                Ok(v) => v,
                Err(e) => {
                    error!("accept 失败: {e}");
                    continue;
                }
            };
            // 并发控制连接上限：满时在 accept 循环处等待（内核 backlog 兜底）
            let Ok(permit) = control_sema.clone().acquire_owned().await else {
                continue;
            };
            let registry = Arc::clone(&registry);
            let token = token.clone();
            let bind = bind.clone();
            let phone_sema = Arc::clone(&phone_sema);
            tokio::spawn(async move {
                handle_control(
                    stream, peer, registry, token, bind, data_base, data_range, phone_sema, permit,
                )
                .await;
            });
        }
    });
}

/// 读首行：peek 找 \n（不消耗内核缓冲），再 read_exact 精确消费到 \n。
/// \n 之后的多余字节留在内核缓冲，连接原样转纯管道时不丢数据。
/// 相比逐字节 read：一次连接最多 256 次 peek + 1 次 read_exact，无逐字节 syscall。
async fn read_first_line(stream: &mut TcpStream) -> Option<String> {
    let mut nl = None;
    for _ in 0..128 {
        let mut buf = [0u8; 256];
        let n = stream.peek(&mut buf).await.ok()?;
        if let Some(pos) = buf[..n].iter().position(|&b| b == b'\n') {
            nl = Some(pos);
            break;
        }
        if n >= 256 {
            return None; // 首行超长：非法握手
        }
        // 数据未到齐：让出执行权等下一片（外层另有握手总超时）
        tokio::time::sleep(Duration::from_millis(10)).await;
    }
    let nl = nl?;
    let mut line = vec![0u8; nl + 1];
    stream.read_exact(&mut line).await.ok()?;
    line.pop(); // 去掉 \n
    Some(String::from_utf8_lossy(&line).trim().to_string())
}

/// 恒时比较令牌：内容与长度都按位累积比较，不因长度不同提前返回（防长度侧信道）
fn constant_time_eq(a: &str, b: &str) -> bool {
    let (a, b) = (a.as_bytes(), b.as_bytes());
    let n = a.len().max(b.len());
    let mut diff: u32 = (a.len() ^ b.len()) as u32;
    for i in 0..n {
        let x = u32::from(*a.get(i).unwrap_or(&0));
        let y = u32::from(*b.get(i).unwrap_or(&0));
        diff |= x ^ y;
    }
    diff == 0
}

#[allow(clippy::too_many_arguments)]
async fn handle_control(
    mut stream: TcpStream,
    peer: std::net::SocketAddr,
    registry: SharedRegistry,
    token: String,
    bind: String,
    data_base: u16,
    data_range: u64,
    phone_sema: Arc<Semaphore>,
    _permit: tokio::sync::OwnedSemaphorePermit,
) {
    // 握手总超时：慢速握手（每 10ms 发一字节）最多拖 10s
    let first = match tokio::time::timeout(HANDSHAKE_TIMEOUT, read_first_line(&mut stream)).await {
        Ok(Some(first)) => first,
        Ok(None) => return,
        Err(_) => return, // 超时：连接随 stream drop 关闭
    };
    let parts: Vec<&str> = first.split_whitespace().collect();
    match parts.as_slice() {
        ["REGISTER", t] if constant_time_eq(t, &token) => {
            register_channel(
                stream, peer, registry, &token, &bind, data_base, data_range, phone_sema,
            )
            .await;
        }
        ["PIPE", t, port] if constant_time_eq(t, &token) => match port.parse::<u16>() {
            Ok(port) => supply_pipe(stream, peer, registry, &token, port).await,
            Err(_) => warn!("PIPE 端口非法来自 {peer}: {port}"),
        },
        _ => {
            warn!("拒绝非法握手来自 {peer}");
            let _ = stream.write_all(b"ERR bad handshake\n").await;
        }
    }
}

/// 注册通道：按 token 哈希选首选端口（同一桌面端重连端口不变，入口 URL/QR 稳定）
/// → 回 PORT → 起 accept 循环 → 盯注册连接断连
fn preferred_port(token: &str, data_base: u16, data_range: u64) -> u16 {
    // FNV-1a 64
    let mut h: u64 = 0xcbf2_9ce4_8422_2325;
    for b in token.as_bytes() {
        h ^= u64::from(*b);
        h = h.wrapping_mul(0x0000_0100_0000_01b3);
    }
    data_base + (h % data_range) as u16
}

async fn register_channel(
    mut stream: TcpStream,
    peer: std::net::SocketAddr,
    registry: SharedRegistry,
    token: &str,
    bind: &str,
    data_base: u16,
    data_range: u64,
    phone_sema: Arc<Semaphore>,
) {
    // 端口探测：锁内只取已占集合（std Mutex 不能跨 await），绑定在锁外尝试。
    // bind 本身原子，双占不可能；锁外 bind 成功到 insert 之间的崩溃窗口只影响
    // 本进程生命周期（fd 随进程回收），可接受
    let start = u64::from(preferred_port(token, data_base, data_range));
    let base = u64::from(data_base);
    let occupied: HashSet<u16> = {
        let reg = registry.lock().unwrap();
        reg.channels.keys().copied().collect()
    };
    let mut bound = None;
    for i in 0..data_range {
        let p = base + ((start - base + i) % data_range);
        let p = p as u16;
        if occupied.contains(&p) {
            continue;
        }
        if let Ok(l) = TcpListener::bind((bind, p)).await {
            bound = Some((p, l));
            break;
        }
    }
    let Some((port, listener)) = bound else {
        warn!("通道注册失败：无空闲数据端口（peer={peer}）");
        let _ = stream.write_all(b"ERR no free data port\n").await;
        return;
    };
    info!("通道注册成功：port={port} peer={peer}");
    set_keepalive(&stream);
    if stream
        .write_all(format!("PORT {port}\n").as_bytes())
        .await
        .is_err()
    {
        return;
    }

    // 数据端口 accept 循环：每台手机独立 spawn「等管道 + 对接」，
    // accept 循环本身永不阻塞（池空时其他手机照常进并发等待，公网可连）
    // accept_task 块里不再重复克隆 phone_sema（参数直接 move 进 spawn）
    let accept_task = {
        let registry = Arc::clone(&registry);
        tokio::spawn(async move {
            loop {
                let (phone, _peer) = match listener.accept().await {
                    Ok(v) => v,
                    Err(e) => {
                        debug!("数据端口 {port} accept 失败：{e}");
                        break;
                    }
                };
                // 并发「等管道」上限：满时此处等待（accept 暂停，backlog 兜底）
                let Ok(permit) = phone_sema.clone().acquire_owned().await else {
                    continue;
                };
                let registry = Arc::clone(&registry);
                tokio::spawn(async move {
                    let _permit = permit; // 等管道期间占名额，防公网无限挂起
                    wait_and_pair(registry, port, phone).await;
                });
            }
        })
    };

    {
        let mut reg = registry.lock().unwrap();
        reg.channels.insert(
            port,
            Channel {
                owner: token.to_string(),
                pipes: Vec::new(),
                accept_task,
            },
        );
    }

    // 注册连接断连探测：整个连接移进探测任务（之后无人再用它），
    // 持续读，EOF/错误（含 keepalive 探测失败）→ 释放通道
    let registry = Arc::clone(&registry);
    let owner = token.to_string();
    tokio::spawn(async move {
        let mut buf = [0u8; 512];
        loop {
            match stream.read(&mut buf).await {
                Ok(0) | Err(_) => break,
                Ok(_) => {} // 注册连接不应再有数据，忽略
            }
        }
        release_channel(&registry, port, &owner);
    });
}

/// 等空闲管道并对接（独立任务，不阻塞 accept）：最多 PHONE_PIPE_WAIT，
/// 取用时丢弃超龄管道（慢速占坑防不住池上限，但坑不会被永久占满）
async fn wait_and_pair(registry: SharedRegistry, port: u16, mut phone: TcpStream) {
    let deadline = Instant::now() + PHONE_PIPE_WAIT;
    loop {
        let taken = {
            let mut reg = registry.lock().unwrap();
            let mut taken = None;
            let mut popped = 0usize;
            if let Some(ch) = reg.channels.get_mut(&port) {
                while let Some(p) = ch.pipes.pop() {
                    popped += 1;
                    if !p.is_stale() {
                        taken = Some(p.stream);
                        break;
                    }
                }
            }
            reg.pipes_total -= popped; // 含被丢弃的超龄管道
            taken
        };
        if let Some(mut pipe) = taken {
            // 纯字节互拷：手机 ⇄ 管道（管道对端是桌面端桥的本地端口）
            let n = copy_bidirectional(&mut pipe, &mut phone)
                .await
                .map(|(a, b)| a + b)
                .unwrap_or(0);
            debug!("手机会话结束：port={port} 转发字节 {n}");
            return;
        }
        if Instant::now() >= deadline {
            debug!("手机等待管道超时：port={port}");
            return; // 没等到管道：放弃该手机连接（手机侧重试）
        }
        tokio::time::sleep(Duration::from_millis(100)).await;
    }
}

/// 释放通道：校验 owner（防并发重注册时误删他通道）→ 中止 accept 任务并
/// 丢弃所有空闲管道（dropping TcpStream 即关闭）
fn release_channel(registry: &SharedRegistry, port: u16, owner: &str) {
    let mut reg = registry.lock().unwrap();
    if reg.channels.get(&port).map_or(true, |ch| ch.owner != owner) {
        return;
    }
    if let Some(ch) = reg.channels.remove(&port) {
        info!(
            "通道断开释放：port={port}（剩余通道 {}）",
            reg.channels.len()
        );
        ch.accept_task.abort();
        reg.pipes_total -= ch.pipes.len();
    }
}

/// 管道供给：校验端口已注册且属于该令牌 → 回 OK → 进空闲池（双重上限）
async fn supply_pipe(
    mut stream: TcpStream,
    peer: std::net::SocketAddr,
    registry: SharedRegistry,
    token: &str,
    port: u16,
) {
    enum Admit {
        Ok,
        UnknownPort,
        PoolFull,
    }
    let verdict = {
        let reg = registry.lock().unwrap();
        match reg.channels.get(&port) {
            None => Admit::UnknownPort,
            Some(ch) if ch.owner != token => Admit::UnknownPort,
            Some(ch) if ch.pipes.len() >= MAX_PIPES_PER_CHANNEL => Admit::PoolFull,
            _ if reg.pipes_total >= MAX_PIPES_TOTAL => Admit::PoolFull,
            Some(_) => Admit::Ok,
        }
    };
    match verdict {
        Admit::Ok => {}
        Admit::UnknownPort => {
            warn!("PIPE 拒绝（未知端口/不属该令牌）：port={port} peer={peer}");
            let _ = stream.write_all(b"ERR unknown port\n").await;
            return;
        }
        Admit::PoolFull => {
            warn!("PIPE 拒绝（池满）：port={port} peer={peer}");
            let _ = stream.write_all(b"ERR pool full\n").await;
            return;
        }
    }
    if stream.write_all(b"OK\n").await.is_err() {
        return;
    }
    set_keepalive(&stream);
    // push 前复检（check/push 之间有并发窗口，超限即弃，宁可少不可爆）
    let mut reg = registry.lock().unwrap();
    let admissible = match reg.channels.get_mut(&port) {
        Some(ch) if ch.owner == token && ch.pipes.len() < MAX_PIPES_PER_CHANNEL => true,
        _ => false,
    };
    if admissible && reg.pipes_total < MAX_PIPES_TOTAL {
        if let Some(ch) = reg.channels.get_mut(&port) {
            ch.pipes.push(IdlePipe {
                stream,
                since: Instant::now(),
            });
            reg.pipes_total += 1;
            debug!(
                "管道入池：port={port} peer={peer}（池总数 {}）",
                reg.pipes_total
            );
        }
    }
    // 不可达路径（刚校验过）：stream drop 即关闭
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn constant_time_eq_matches_and_rejects() {
        assert!(constant_time_eq("abc123", "abc123"));
        assert!(!constant_time_eq("abc123", "abc124"));
        assert!(!constant_time_eq("abc123", "abc1234"));
        assert!(!constant_time_eq("", "a"));
        assert!(constant_time_eq("", ""));
    }

    #[test]
    fn preferred_port_stays_in_range() {
        // 常规段
        for tok in ["t1", "hello", "0123456789abcdef0123456789abcdef"] {
            let p = preferred_port(tok, 21001, 99);
            assert!((21001..=21099).contains(&p), "{tok} -> {p}");
        }
        // 极限段：base=1, end=65535（range=65535，u16 差值+1 溢出会 panic，u64 不应）
        let p = preferred_port("x", 1, 65535);
        assert!((1..=65535).contains(&p));
    }

    #[test]
    fn read_first_line_keeps_trailing_bytes_in_socket() {
        let rt = tokio::runtime::Builder::new_current_thread()
            .enable_all()
            .build()
            .unwrap();
        rt.block_on(async {
            let l = TcpListener::bind(("127.0.0.1", 0)).await.unwrap();
            let addr = l.local_addr().unwrap();
            let server = tokio::spawn(async move {
                let (mut s, _) = l.accept().await.unwrap();
                let line = read_first_line(&mut s).await.unwrap();
                // \n 之后的多余字节必须还能从连接里读到（管道前缀不丢）
                let mut rest = Vec::new();
                s.read_to_end(&mut rest).await.unwrap();
                (line, rest)
            });
            let mut cli = TcpStream::connect(addr).await.unwrap();
            cli.write_all(b"REGISTER tk\nTRAILING-BYTES").await.unwrap();
            cli.shutdown().await.unwrap();
            let (line, rest) = server.await.unwrap();
            assert_eq!(line, "REGISTER tk");
            assert_eq!(rest, b"TRAILING-BYTES");
        });
    }

    #[test]
    fn idle_pipe_stale_after_timeout() {
        let rt = tokio::runtime::Builder::new_current_thread()
            .enable_all()
            .build()
            .unwrap();
        rt.block_on(async {
            let l = TcpListener::bind(("127.0.0.1", 0)).await.unwrap();
            let _cli = TcpStream::connect(l.local_addr().unwrap()).await.unwrap();
            let stream = l.accept().await.unwrap().0;
            let fresh = IdlePipe {
                stream,
                since: Instant::now(),
            };
            assert!(!fresh.is_stale());
            // 31s 前进的老管道：把 since 手动回拨构造（字段私有，测试内可见）
            let stale = IdlePipe {
                stream: fresh.stream,
                since: Instant::now() - Duration::from_secs(31),
            };
            assert!(stale.is_stale());
        });
    }
}
