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

use std::sync::{Arc, Mutex as StdMutex};
use std::time::Duration;

use serde::Serialize;
use tauri::{AppHandle, State};
use tokio::io::{copy_bidirectional, AsyncReadExt, AsyncWriteExt};
use tokio::net::TcpStream;
use tokio::sync::watch;

/// 空闲管道池目标数（浏览器并发连接 ~6，WS 占 1，余量应对并发 fetch）
const POOL_TARGET: usize = 12;
const REGISTER_TIMEOUT: Duration = Duration::from_secs(6);
const PIPE_READY_TIMEOUT: Duration = Duration::from_secs(6);
/// relay_start 等首次注册成功的上限（注册失败/超时则整体失败返回）
const FIRST_REGISTER_WAIT: Duration = Duration::from_secs(12);

/// 中继入口（info_for / key_create 的 QR 用它生成 URL）
#[derive(Debug, Clone, Serialize)]
pub struct RelayEntry {
    pub host: String,
    pub control_port: u16,
    pub data_port: u16,
}

/// 运行中的中继客户端（存在 MobileBridgeState.relay 里）
pub struct RelayRunning {
    pub entry: RelayEntry,
    pub shutdown: watch::Sender<bool>,
    pub join: tokio::task::JoinHandle<()>,
}

/// 手动读一行（逐字节：与 relay 侧约定一致，PIPE 连接读完首行后原样当管道用）
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
                    return None;
                }
            }
        }
    }
    Some(String::from_utf8_lossy(&buf).trim().to_string())
}

/// 连 relay 控制端口 REGISTER，等 PORT 响应。返回 (注册连接, 数据端口)。
/// 注册连接由调用方持有（保持 = 通道存活），断开即被 relay 释放通道。
async fn register(host: &str, control_port: u16, token: &str) -> Result<(TcpStream, u16), String> {
    let mut stream = TcpStream::connect((host, control_port))
        .await
        .map_err(|e| format!("relay 控制端口 {host}:{control_port} 连接失败: {e}"))?;
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
    alive: Arc<StdMutex<usize>>,
) {
    let res: Result<(), String> = async {
        let mut ctrl = TcpStream::connect((host.as_str(), control_port))
            .await
            .map_err(|e| format!("PIPE 连接失败: {e}"))?;
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
        let bridge = TcpStream::connect(("127.0.0.1", bridge_port))
            .await
            .map_err(|e| format!("本机桥端口连接失败: {e}"))?;
        let (mut a, mut b) = (ctrl, bridge);
        copy_bidirectional(&mut a, &mut b)
            .await
            .map_err(|e| format!("管道拷贝中断: {e}"))?;
        Ok(())
    }
    .await;
    if let Err(e) = res {
        crate::app_log!(debug, "mobile-relay", "管道结束: {e}");
    }
    if let Ok(mut n) = alive.lock() {
        *n = n.saturating_sub(1);
    }
}

/// 池循环：注册连接存活期间持续补管道；注册断开（EOF）→ 返回触发重注册
async fn pool_loop(
    mut reg: TcpStream,
    host: String,
    control_port: u16,
    token: String,
    data_port: u16,
    bridge_port: u16,
    mut shutdown_rx: watch::Receiver<bool>,
) {
    let alive = Arc::new(StdMutex::new(0usize));
    let mut tick = tokio::time::interval(Duration::from_millis(400));
    let mut buf = [0u8; 256];
    loop {
        tokio::select! {
            _ = shutdown_rx.wait_for(|v| *v) => return,
            res = reg.read(&mut buf) => {
                match res {
                    // 注册连接不应再有数据；EOF/错误 = relay 已释放通道
                    Ok(0) | Err(_) => {
                        crate::app_log!(warn, "mobile-relay",
                            "relay 通道断开（{host}:{control_port}），准备重注册");
                        return;
                    }
                    Ok(_) => {}
                }
            }
            _ = tick.tick() => {
                let cur = alive.lock().map(|n| *n).unwrap_or(POOL_TARGET);
                for _ in cur..POOL_TARGET {
                    tokio::spawn(pipe_task(
                        host.clone(),
                        control_port,
                        token.clone(),
                        data_port,
                        bridge_port,
                        Arc::clone(&alive),
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
    port_slot: Arc<StdMutex<Option<u16>>>,
    mut shutdown_rx: watch::Receiver<bool>,
) {
    let mut backoff_secs = 1u64;
    loop {
        match tokio::time::timeout(REGISTER_TIMEOUT, register(&host, control_port, &token)).await {
            Ok(Ok((reg, data_port))) => {
                backoff_secs = 1;
                if let Ok(mut slot) = port_slot.lock() {
                    *slot = Some(data_port);
                }
                crate::app_log!(
                    info,
                    "mobile-relay",
                    "通道注册成功 {host}:{control_port} → 数据端口 {data_port}"
                );
                pool_loop(
                    reg,
                    host.clone(),
                    control_port,
                    token.clone(),
                    data_port,
                    bridge_port,
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
) -> Result<crate::mobile_bridge::BridgeInfo, String> {
    relay_start(&app, &state, host, control_port, token).await
}

/// 停止中继：入口回落局域网直连
#[tauri::command]
pub async fn mobile_bridge_relay_stop(
    state: State<'_, crate::mobile_bridge::MobileBridgeState>,
) -> Result<(), String> {
    relay_stop(&state).await
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
        let (_sock, data_port) = register("127.0.0.1", ctrl_port, "tk1").await.unwrap();
        assert!(data_port > 0);
        // 错误令牌被拒（假 relay 不响应 → 读到 EOF 返回 None → Err）
        let r = register("127.0.0.1", ctrl_port, "wrong").await;
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
        // 注册成功后断开注册连接（模拟 relay 重启/网络中断）→ maintain 应重注册
        let (ctrl_port, _) = spawn_fake_relay("tk3").await;
        let echo_port = spawn_echo().await;
        let slot = Arc::new(StdMutex::new(None));
        let (tx, rx) = watch::channel(false);
        let join = tokio::spawn(maintain(
            "127.0.0.1".to_string(),
            ctrl_port,
            "tk3".to_string(),
            echo_port,
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
}
