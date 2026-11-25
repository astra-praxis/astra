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

### 1. 构建与上传

```bash
# 构建（任意有 Rust 的机器，产物是静态二进制）
cd relay
cargo build --release
# 产物: target/release/astra-relay，拷到服务器
scp target/release/astra-relay README.md deploy/astra-relay.service user@server:/tmp/astra-relay-upload/
```

> 服务器上直接装 Rust 构建也行：`rustup` 装好后 `cargo build --release`，免传二进制。
> 产物为纯静态 TCP 转发器，无运行时依赖；注意服务器架构要与构建机一致（一般 x86_64）。

### 2. 安装为 systemd 服务（推荐）

服务器上执行：

```bash
sudo mkdir -p /opt/astra-relay
sudo cp /tmp/astra-relay-upload/astra-relay /tmp/astra-relay-upload/README.md /opt/astra-relay/
sudo chmod 755 /opt/astra-relay/astra-relay

# 生成强令牌（只打印一次，妥善保存，桌面端中继卡片要用）
openssl rand -hex 24

# 写入令牌到单元文件（把 __REPLACE_...__ 换成上面生成的值）
sudo cp /tmp/astra-relay-upload/astra-relay.service /etc/systemd/system/
sudo nano /etc/systemd/system/astra-relay.service

sudo systemctl daemon-reload
sudo systemctl enable --now astra-relay
sudo systemctl status astra-relay
```

不想用 systemd 也可以手工跑：

```bash
ASTRA_RELAY_TOKEN=<令牌> ASTRA_RELAY_LOG_DIR=/var/log/astra-relay ./astra-relay
```

### 3. 防火墙放行

控制端口 + 数据端口段（默认 21000/tcp、21001-21099/tcp）：

```bash
sudo ufw allow 21000/tcp && sudo ufw allow 21001:21099/tcp
```

### 4. 桌面端接入

桌面端「手机遥控」页 → 公网中继卡片：填服务器地址、控制端口、上面的令牌 →
开启中继。二维码与设备密钥入口自动切换为中继地址（手机任意网络可扫）。

### 常用运维命令

```bash
sudo systemctl restart astra-relay   # 重启（桌面端自动重连）
journalctl -u astra-relay -f         # 实时看 stderr 日志
ls /var/log/astra-relay/             # 按天滚动的日志文件
```

## 日志

基于 `tracing` 实现，格式：`时间戳 级别 消息`。

- **输出位置**：默认 stderr（systemd 下自动进 journald）；设 `ASTRA_RELAY_LOG_DIR`
  后按天滚动落盘（`astra-relay.log.<日期>`），同时保留 stderr
- **级别**：`ASTRA_RELAY_LOG_LEVEL`（默认 `info`；`debug` 额外记录每次手机
  对接/管道入池/转发字节数）

info 级别记录的关键事件：

| 事件 | 级别 | 内容 |
|------|------|------|
| 启动就绪 | info | 控制端口 / 数据端口段 / 管道上限 |
| 通道注册成功 | info | 数据端口 + 桌面端来源地址 |
| 通道断开释放 | info | 数据端口 + 剩余通道数 |
| 无空闲数据端口 | warn | 注册被拒 |
| 非法握手（错令牌/乱发字节） | warn | 来源地址（公网扫描可在此发现） |
| PIPE 拒绝（未知端口/池满） | warn | 端口 + 来源地址 |

安全约束：**令牌内容绝不落日志**；relay 不解析流量，管道内转发内容只有
`debug` 级别的字节计数，无内容泄漏。

## ⚠️ TLS 终止（公网部署必做）

relay 本体是**明文 TCP**：令牌在控制连接首行明文传输，手机流量同样明文。
公网部署必须在 relay 前面加 TLS 终止层。推荐方案：**stunnel 同端口透传**——
stunnel 绑公网同一批端口（21000 + 数据段），relay 改绑 `127.0.0.1` 只收回环，
入口 URL/端口与明文部署完全一致，桌面端勾上「TLS」即可。

### 第 0 步：域名与证书

TLS 公网证书需要一个域名（IP 签不了 Let's Encrypt 证书）：

```bash
# DNS 把 relay.example.com 指到服务器 IP，然后：
sudo apt install certbot
sudo certbot certonly --standalone -d relay.example.com
# 证书：/etc/letsencrypt/live/relay.example.com/{fullchain.pem,privkey.pem}
# 续期钩子（证书 90 天一换，renew 后重启 stunnel；用 hook 目录比改 renewal 配置可靠）：
sudo tee /etc/letsencrypt/renewal-hooks/deploy/restart-stunnel.sh <<'EOF'
#!/bin/sh
systemctl restart stunnel-astra-relay
EOF
sudo chmod +x /etc/letsencrypt/renewal-hooks/deploy/restart-stunnel.sh
```

> 服务器 80 被封（家宽未备案等）时改用 DNS 验证签证书，完全不碰 80；
> 免费域名的便捷选择：DuckDNS（在公共后缀列表里，每个子域独立限额，
> 装 `certbot-dns-duckdns` 插件即可 DNS 验证，家宽动态 IP 加 cron 定时更新）。

### 第 1 步：relay 改绑回环

systemd 单元（deploy/astra-relay.service）里加一行：

```ini
Environment=ASTRA_RELAY_BIND=127.0.0.1
```

重启 relay。启动日志会变成「监听 127.0.0.1」；若单桌面端使用，可顺手
收窄数据段（如 `ASTRA_RELAY_DATA_END=21010`），stunnel 配置段跟着少写 89 个。

### 第 2 步：stunnel 同端口透传

```bash
sudo apt install stunnel4
```

`/etc/stunnel/astra-relay.conf`（数据段用循环生成，别手写 99 个）：

```bash
sudo sh -c 'cat > /etc/stunnel/astra-relay.conf <<EOF
; 全局
foreground = yes
syslog = no
cert = /etc/letsencrypt/live/relay.example.com/fullchain.pem
key = /etc/letsencrypt/live/relay.example.com/privkey.pem
sslVersionMin = TLSv1.2

[relay-control]
accept = 0.0.0.0:21000
connect = 127.0.0.1:21000
EOF
for p in $(seq 21001 21010); do
  printf "\n[relay-data-%s]\naccept = 0.0.0.0:%s\nconnect = 127.0.0.1:%s\n" \
    "$p" "$p" "$p" >> /etc/stunnel/astra-relay.conf
done'
```

（`seq 21001 21010` 范围要与 relay 的数据段一致。）

systemd 单元 `/etc/systemd/system/stunnel-astra-relay.service`：

```ini
[Unit]
Description=stunnel TLS termination for astra-relay
After=network-online.target
Wants=network-online.target

[Service]
ExecStart=/usr/bin/stunnel /etc/stunnel/astra-relay.conf
ExecReload=/bin/kill -HUP $MAINPID
Restart=always
RestartSec=3

[Install]
WantedBy=multi-user.target
```

```bash
sudo systemctl daemon-reload
sudo systemctl enable --now stunnel-astra-relay
```

### 第 3 步：防火墙与桌面端

端口不用变（21000 + 数据段，但现在都是 TLS）；明文端口已不暴露——relay 只
收 127.0.0.1。桌面端「手机遥控」→ 公网中继卡片：地址填 `relay.example.com`、
控制端口 21000、令牌，**勾选「TLS」**→ 开启中继。二维码自动变成
`https://relay.example.com:<数据端口>/m#...`，手机任意网络扫码即用。

> `#secret` 片段不发送到网络，凭证不会进 stunnel 日志；relay 侧日志也只记
> 端口与来源，无内容。

### 备选方案

- **nginx stream 模块**：每个端口一个 `server { listen 21000 ssl; proxy_pass
  127.0.0.1:21000; }` 块（同样需逐端口写，无段写法）
- **Caddy l4 插件**（自动证书，免 certbot）：`xcaddy build --with
  github.com/mholt/caddy-l4` 后逐端口 `l4 :21000 { proxy 127.0.0.1:21000 }`
- **无域名/不想配 TLS**：把 relay 放在 VPN（WireGuard/Tailscale）后使用，
  桌面端不勾 TLS，地址填 tailnet IP——传输由 WireGuard 加密，relay 端口
  不对公网开放

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
  （桌面端池目标 12 ≪ 每通道上限 64）；info 级别日志不逐连接记录管道，
  被扫描时只有 warn 级拒绝记录，不会刷屏
