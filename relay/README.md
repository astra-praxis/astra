# astra-relay — Astra 手机遥控公网中继

独立部署的转发服务器：让手机从**任何网络**（不限于桌面端所在局域网）接入桌面端的
「手机遥控桥」。桌面端主动连出（穿透 NAT），relay 只做字节转发，**不解析、不缓存、
不存储任何流量内容**——桥的凭证体系（临时 token / 设备密钥 / 票据 / 限速）原样生效。

> **信任边界**：relay 不做任何应用层防护（认证/限速/防重放全部由桌面端桥自己承担），
> 它的安全完全建立在「桥侧够硬 + 令牌不泄 + 流量加密」上。
> **公网部署必须前置 TLS 终止，否则令牌与全部遥控流量明文裸奔**（见下文）。

## 架构

```
手机 ──HTTP/WS──▶ relay(数据端口) ──空闲管道──▶ relay(控制连接) ◀──桌面端主动连出
                     1:1 对接                                      桥(axum) 跑在本机
```

- 桌面端启动中继后连 relay 控制端口 `REGISTER <token>` → relay 分配一个**数据端口**
  （按 token 哈希选定，断线重连端口不变，入口 URL/二维码稳定）
- 桌面端维持 ~12 条 `PIPE` 空闲管道；手机连数据端口时 relay 弹一条空闲管道与手机
  1:1 互拷字节，管道对端是桌面端本机的桥端口
- 桌面端断开注册连接（关机/停止中继）→ relay 释放通道并断开全部管道

## 安全加固（已内置）

- **令牌**：启动强制 ≥32 字符；恒时比较（含长度填充，无长度侧信道）
- **握手**：控制连接读首行 10s 超时（防慢速握手占 fd）；peek+read_exact 读法，
  无逐字节 syscall；首行后多余字节保留在内核缓冲（管道前缀不丢）
- **资源上限**：管道池每通道 ≤64 / 全局 ≤4096，超限回 `ERR pool full`；
  池内管道 30s 空闲淘汰（防慢速占坑）；控制并发 ≤512、手机并发等管道 ≤512
  （Semaphore，满时在 accept 处背压）
- **数据端口 accept 不阻塞**：每台手机独立任务等管道（最多 15s），不影响后续 accept
- **半开连接**：注册/管道连接启用 TCP keepalive（60s 探测间隔 30s），
  对端掉线无 FIN 也能释放通道
- **通道归属**：通道记录 owner 令牌，释放/供管道时校验，防并发注册误删

## 部署

```bash
# 构建（任意有 Rust 的机器，产物是静态二进制）
cd relay
cargo build --release
# 产物: target/release/astra-relay，拷到服务器
ASTRA_RELAY_TOKEN=$(openssl rand -hex 24) ./astra-relay
```

防火墙放行：控制端口 + 数据端口段（默认 21000/tcp、21001-21099/tcp）。

桌面端「手机遥控」页 → 公网中继卡片：填服务器地址、控制端口、上面的令牌 →
开启中继。二维码与设备密钥入口自动切换为中继地址（手机任意网络可扫）。

## ⚠️ TLS 终止（公网部署必做）

relay 本体是**明文 TCP**：令牌在控制连接首行明文传输，手机流量同样明文。
公网部署必须在 relay 前面加 TLS 终止层，推荐任选其一：

**方案 A：stunnel（轻量纯 TCP TLS，推荐）**

```ini
; /etc/stunnel/astra-relay.conf — 每个端口一段（控制 + 数据段都透传）
[relay-control]
accept = 0.0.0.0:21443
connect = 127.0.0.1:21000
cert = /etc/stunnel/fullchain.pem
key  = /etc/stunnel/privkey.pem

[relay-data]
accept = 0.0.0.0:21444
connect = 127.0.0.1:21001
cert = /etc/stunnel/fullchain.pem
key  = /etc/stunnel/privkey.pem
```

**方案 B：nginx stream 模块**

```nginx
stream {
  upstream relay_ctrl { server 127.0.0.1:21000; }
  server {
    listen 21443 ssl;
    ssl_certificate     /etc/nginx/fullchain.pem;
    ssl_certificate_key /etc/nginx/privkey.pem;
    proxy_pass relay_ctrl;
  }
}
```

**方案 C：Caddy l4 插件（自动证书）**

```caddyfile
# xcaddy build --with github.com/mholt/caddy-l4
l4 :21443 {
  route {
    tls {
      dns cloudflare   # 或换成你的 DNS 提供商
    }
    proxy 127.0.0.1:21000
  }
}
```

要点：
- TLS 层与 relay 之间走回环（配合防火墙只放行 TLS 层来源，relay 的 0.0.0.0 监听
  就不会直接暴露公网）
- 数据端口段也要同样透传（手机连的是数据端口）；TLS 终止后手机侧入口应改为
  `https://`；**fragment（#secret）不会发送到网络**，认证凭证不进 TLS 终止层日志
- 若无法配置 TLS，至少把 relay 放在内网/VPN（WireGuard/Tailscale）后使用

## 端到端凭证（relay 不加分）

手机与桌面端桥之间仍是完整的凭证链（URL fragment secret → `/api/ticket` 一次性
票据 → WS/HTTP Bearer），relay 看不到也不解析这些内容；拿到数据端口也只能连到
桥的认证层，无凭证直接被 401/限速。服务器侧请保护好 token 不外泄（可任意注册
通道占用数据端口段）。

## 已知取舍（v1）

- 多台桌面端共用同一 token：第二台会拿到相邻数据端口（入口地址不同），各自独立
- 经 relay 的手机流量在桥侧都来自 127.0.0.1，共享认证限速桶（认证成功即清零，
  正常使用无感；暴力枚举场景下多设备会互相放大退避，属预期行为）
- relay 无持久化状态，重启即丢通道（桌面端 30s 内自动退避重连重注册）
- 池满（`ERR pool full`）时桌面端管道任务会静默重试，正常使用碰不到
  （桌面端池目标 12 ≪ 每通道上限 64）；日志 stderr 明文、未限速，被扫描时可能刷屏
