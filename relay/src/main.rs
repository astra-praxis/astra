//! astra-relay — Astra 手机遥控桥的中继服务器（独立部署在公网/可达服务器上）。
//!
//! 设计（TCP 池式管道，对流量零解析）：
//! - 桌面端主动连出（穿透 NAT），第一条连接 REGISTER 注册通道 → 分配一个
//!   数据端口；之后维持若干条 PIPE 空闲管道连接
//! - 手机连数据端口（与桌面端桥之间的真实协议照旧：页面 / ticket 认证 / WS
//!   都由桌面端桥自己处理），relay 弹一条空闲管道与手机 1:1 互拷字节
//! - relay 不解析、不缓存任何流量内容；桌面端桥的凭证体系（临时 token /
//!   设备 key / 票据 / 限速）原样生效
//!
//! 控制协议（首行文本，\n 结尾，之后注册/管道连接均为纯字节流）：
//!   REGISTER <token>\n → PORT <数据端口>\n（连接保持作为通道持有者，断开即
//!                        释放通道并断开其所有空闲管道）
//!   PIPE <token> <端口>\n → OK\n（连接进入该端口的空闲池，等手机接入对接）
//!
//! 环境变量：
//!   ASTRA_RELAY_TOKEN        桌面端接入令牌（必填，防任意注册）
//!   ASTRA_RELAY_CONTROL_PORT 控制端口（默认 21000）
//!   ASTRA_RELAY_DATA_BASE    数据端口起始（默认 21001）
//!   ASTRA_RELAY_DATA_END     数据端口结束（默认 21099，含）
//!
//! 用法：cargo build --release 后把 target/release/astra-relay 放到服务器运行
//!（放行控制端口与数据端口段），桌面端「手机遥控」页填服务器地址 + 控制端口 +
//! 此令牌即可开启中继。

use std::collections::HashMap;
use std::sync::{Arc, Mutex};

use tokio::io::{copy_bidirectional, AsyncReadExt, AsyncWriteExt};
use tokio::net::{TcpListener, TcpStream};

/// 通道：注册连接（持有者）+ 空闲管道池 + accept 任务句柄
struct Channel {
    pipes: Vec<TcpStream>,
    accept_task: tokio::task::JoinHandle<()>,
}

#[derive(Default)]
struct Registry {
    /// 数据端口 → 通道
    channels: HashMap<u16, Channel>,
}

type SharedRegistry = Arc<Mutex<Registry>>;

fn main() {
    let token = std::env::var("ASTRA_RELAY_TOKEN").unwrap_or_default();
    if token.is_empty() {
        eprintln!("环境变量 ASTRA_RELAY_TOKEN 未设置：桌面端接入令牌必填（防任意注册）");
        std::process::exit(1);
    }
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
        eprintln!("ASTRA_RELAY_DATA_END ({data_end}) 不能小于 ASTRA_RELAY_DATA_BASE ({data_base})");
        std::process::exit(1);
    }

    let registry: SharedRegistry = Arc::new(Mutex::new(Registry::default()));

    let rt = tokio::runtime::Builder::new_multi_thread()
        .enable_all()
        .build()
        .expect("构建 tokio runtime 失败");
    rt.block_on(async move {
        let listener = TcpListener::bind(("0.0.0.0", control_port))
            .await
            .unwrap_or_else(|e| panic!("控制端口 {control_port} 绑定失败: {e}"));
        eprintln!("astra-relay 就绪：控制端口 {control_port}，数据端口 {data_base}-{data_end}");
        loop {
            let (stream, peer) = match listener.accept().await {
                Ok(v) => v,
                Err(e) => {
                    eprintln!("accept 失败: {e}");
                    continue;
                }
            };
            let registry = Arc::clone(&registry);
            let token = token.clone();
            tokio::spawn(async move {
                handle_control(stream, peer, registry, token, data_base, data_end).await;
            });
        }
    });
}

/// 手动读一行（不用 BufReader：读完首行后连接要原样保留为纯管道，不能有缓冲残留）
async fn read_first_line(stream: &mut TcpStream) -> Option<String> {
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
                    return None; // 首行超长：非法握手
                }
            }
        }
    }
    Some(String::from_utf8_lossy(&buf).trim().to_string())
}

/// 恒时比较令牌，避免计时侧信道
fn constant_time_eq(a: &str, b: &str) -> bool {
    let (a, b) = (a.as_bytes(), b.as_bytes());
    if a.len() != b.len() {
        return false;
    }
    let mut diff = 0u8;
    for (x, y) in a.iter().zip(b.iter()) {
        diff |= x ^ y;
    }
    diff == 0
}

async fn handle_control(
    mut stream: TcpStream,
    peer: std::net::SocketAddr,
    registry: SharedRegistry,
    token: String,
    data_base: u16,
    data_end: u16,
) {
    let Some(first) = read_first_line(&mut stream).await else {
        return;
    };
    let parts: Vec<&str> = first.split_whitespace().collect();
    match parts.as_slice() {
        ["REGISTER", t] if constant_time_eq(t, &token) => {
            register_channel(stream, registry, &token, data_base, data_end).await;
        }
        ["PIPE", t, port] if constant_time_eq(t, &token) => match port.parse::<u16>() {
            Ok(port) => supply_pipe(stream, registry, port).await,
            Err(_) => {}
        },
        _ => {
            eprintln!("拒绝非法握手来自 {peer}");
            let _ = stream.write_all(b"ERR bad handshake\n").await;
        }
    }
}

/// 注册通道：按 token 哈希选首选端口（同一桌面端重连端口不变，入口 URL/QR 稳定）
/// → 回 PORT → 起 accept 循环 → 盯注册连接断连
fn preferred_port(token: &str, data_base: u16, data_end: u16) -> u16 {
    // FNV-1a 64
    let mut h: u64 = 0xcbf2_9ce4_8422_2325;
    for b in token.as_bytes() {
        h ^= u64::from(*b);
        h = h.wrapping_mul(0x0000_0100_0000_01b3);
    }
    let range = u64::from(data_end - data_base + 1);
    data_base + (h % range) as u16
}

async fn register_channel(
    mut stream: TcpStream,
    registry: SharedRegistry,
    token: &str,
    data_base: u16,
    data_end: u16,
) {
    // 端口探测：锁内只取已占集合（std Mutex 不能跨 await），绑定在锁外尝试
    let range = u32::from(data_end - data_base + 1);
    let start = u32::from(preferred_port(token, data_base, data_end));
    let occupied: std::collections::HashSet<u16> = {
        let reg = registry.lock().unwrap();
        reg.channels.keys().copied().collect()
    };
    let mut bound = None;
    for i in 0..range {
        let p = data_base + ((start - u32::from(data_base) + i) % range) as u16;
        if occupied.contains(&p) {
            continue;
        }
        if let Ok(l) = TcpListener::bind(("0.0.0.0", p)).await {
            bound = Some((p, l));
            break;
        }
    }
    let Some((port, listener)) = bound else {
        let _ = stream.write_all(b"ERR no free data port\n").await;
        return;
    };
    if stream
        .write_all(format!("PORT {port}\n").as_bytes())
        .await
        .is_err()
    {
        return;
    }

    // 数据端口 accept 循环：每个手机连接弹一条空闲管道对接
    let accept_task = {
        let registry = Arc::clone(&registry);
        tokio::spawn(async move {
            loop {
                let (phone, _peer) = match listener.accept().await {
                    Ok(v) => v,
                    Err(_) => break,
                };
                // 等空闲管道（最多 ~15s；桌面端池会持续补充）
                let mut pipe = None;
                for _ in 0..150 {
                    let popped = registry
                        .lock()
                        .unwrap()
                        .channels
                        .get_mut(&port)
                        .and_then(|c| c.pipes.pop());
                    if popped.is_some() {
                        pipe = popped;
                        break;
                    }
                    tokio::time::sleep(std::time::Duration::from_millis(100)).await;
                }
                let Some(pipe) = pipe else {
                    drop(phone); // 没管道可用：放弃该手机连接（桌面端桥侧无感知）
                    continue;
                };
                tokio::spawn(async move {
                    // 纯字节互拷：手机 ⇄ 管道（管道对端是桌面端桥的本地端口）
                    let (mut p, mut t) = (pipe, phone);
                    let _ = copy_bidirectional(&mut p, &mut t).await;
                });
            }
        })
    };

    {
        let mut reg = registry.lock().unwrap();
        reg.channels.insert(
            port,
            Channel {
                pipes: Vec::new(),
                accept_task,
            },
        );
    }

    // 注册连接断连探测：整个连接移进探测任务（之后无人再用它），
    // 持续读，EOF/错误 → 释放通道
    let registry = Arc::clone(&registry);
    tokio::spawn(async move {
        let mut buf = [0u8; 512];
        loop {
            match stream.read(&mut buf).await {
                Ok(0) | Err(_) => break,
                Ok(_) => {} // 注册连接不应再有数据，忽略
            }
        }
        release_channel(&registry, port);
    });
}

/// 释放通道：中止 accept 任务并丢弃所有空闲管道（dropping TcpStream 即关闭）
fn release_channel(registry: &SharedRegistry, port: u16) {
    let mut reg = registry.lock().unwrap();
    if let Some(ch) = reg.channels.remove(&port) {
        ch.accept_task.abort();
        // pipes 随 Channel drop 关闭，桌面端管道任务结束 → 池自动补充
    }
}

/// 管道供给：校验端口已注册 → 回 OK → 进空闲池
async fn supply_pipe(mut stream: TcpStream, registry: SharedRegistry, port: u16) {
    let registered = {
        let reg = registry.lock().unwrap();
        reg.channels.contains_key(&port)
    };
    if !registered {
        let _ = stream.write_all(b"ERR unknown port\n").await;
        return;
    }
    if stream.write_all(b"OK\n").await.is_err() {
        return;
    }
    let mut reg = registry.lock().unwrap();
    if let Some(ch) = reg.channels.get_mut(&port) {
        ch.pipes.push(stream);
    }
    // 理论上不可达（刚校验过）；stream drop 即关闭
}
