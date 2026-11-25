#!/bin/sh
# astra-relay 一键部署 — praxisastra.duckdns.org (65.49.219.76)
#
# 用法（服务器上，需 root）：
#   sudo DUCKDNS_TOKEN=你的duckdns令牌 ./setup-praxisastra.sh
#
# 前置：本脚本同目录下有 relay/ 源码目录（从 astra 仓库拷出），或已把
#       编译好的 astra-relay 二进制放到 /opt/astra-relay/
# 效果：relay 绑 127.0.0.1，stunnel 前置 TLS 绑公网同一批端口（21000 + 21001-21010），
#       证书 Let's Encrypt（DuckDNS TXT 验证，自动续期），日志落 /var/log/astra-relay
set -eu
export PATH="$HOME/.cargo/bin:$PATH"   # rustup 装过的 cargo 在新 shell 里不在 PATH

DOMAIN=praxisastra.duckdns.org
CONTROL_PORT=21000
DATA_BASE=21001
DATA_END=21010          # 单桌面端够用；要扩容改这里并重跑（stunnel 段自动重生成）
OPT=/opt/astra-relay
LOGDIR=/var/log/astra-relay
HOOKDIR="$(cd "$(dirname "$0")" && pwd)"

[ "$(id -u)" = 0 ] || { echo "请用 sudo 运行"; exit 1; }
[ -n "${DUCKDNS_TOKEN:-}" ] || { echo "用法: sudo DUCKDNS_TOKEN=你的duckdns令牌 $0"; exit 1; }

echo "== 1/6 接入令牌"
mkdir -p "$OPT"
if [ -f "$OPT/env" ] && grep -q ASTRA_RELAY_TOKEN "$OPT/env"; then
    TOKEN=$(grep ASTRA_RELAY_TOKEN "$OPT/env" | cut -d= -f2)
    echo "   复用已有令牌"
else
    TOKEN=$(openssl rand -hex 24)
fi
cat > "$OPT/env" <<EOF
ASTRA_RELAY_TOKEN=$TOKEN
ASTRA_RELAY_BIND=127.0.0.1
ASTRA_RELAY_LOG_DIR=$LOGDIR
ASTRA_RELAY_LOG_LEVEL=info
EOF
chmod 600 "$OPT/env"
echo "   令牌: $TOKEN"
echo "   （桌面端「手机遥控 → 公网中继」要填这个，妥善保存）"

echo "== 2/6 编译 relay（有二进制则跳过）"
if [ ! -x "$OPT/astra-relay" ]; then
    command -v cc >/dev/null 2>&1 || { echo "   装 C 工具链（build-essential）..."; apt-get update -qq; apt-get install -y -qq build-essential >/dev/null; }
    if ! command -v cargo >/dev/null 2>&1; then
        echo "   未安装 Rust，装 rustup（约 1 分钟）..."
        curl --proto '=https' --tlsv1.2 -sSf https://sh.rustup.rs | sh -s -- -y --profile minimal
        . "$HOME/.cargo/env"
    fi
    if [ ! -d "$HOOKDIR/relay/src" ]; then
        echo "   ✗ 找不到 $HOOKDIR/relay 源码：请先 scp -r astra 仓库的 relay/ 目录到服务器本目录"; exit 1
    fi
    echo "   cargo build --release ..."
    (cd "$HOOKDIR/relay" && cargo build --release --locked 2>/dev/null || cargo build --release)
    cp "$HOOKDIR/relay/target/release/astra-relay" "$OPT/astra-relay"
fi
chmod 755 "$OPT/astra-relay"
echo "   二进制就绪：$OPT/astra-relay"

echo "== 3/6 证书（Let's Encrypt，DuckDNS TXT 验证，不占用 80/443）"
CERT_DIR="/etc/letsencrypt/live/$DOMAIN"
if [ -f "$CERT_DIR/fullchain.pem" ]; then
    echo "   已有证书，跳过"
else
    command -v certbot >/dev/null 2>&1 || apt-get update -qq && apt-get install -y -qq certbot >/dev/null

    # --manual 验证钩子：用 DuckDNS API 写/清 TXT（无需 pip 插件）
    cat > "$OPT/duckdns-auth.sh" <<EOF
#!/bin/sh
D=\$(echo "\$CERTBOT_DOMAIN" | sed 's/\\.duckdns\\.org\$//')
curl -s "https://www.duckdns.org/update?domains=\$D&token=$DUCKDNS_TOKEN&txt=\$CERTBOT_VALIDATION" >/dev/null
sleep 45   # 等 TXT 传播（DuckDNS TTL ~60s）
EOF
    cat > "$OPT/duckdns-cleanup.sh" <<EOF
#!/bin/sh
D=\$(echo "\$CERTBOT_DOMAIN" | sed 's/\\.duckdns\\.org\$//')
curl -s "https://www.duckdns.org/update?domains=\$D&token=$DUCKDNS_TOKEN&txt=" >/dev/null
EOF
    chmod 700 "$OPT/duckdns-auth.sh" "$OPT/duckdns-cleanup.sh"

    certbot certonly --non-interactive --agree-tos --register-unsafely-without-email \
        --manual --preferred-challenges dns \
        --manual-auth-hook "$OPT/duckdns-auth.sh" \
        --manual-cleanup-hook "$OPT/duckdns-cleanup.sh" \
        -d "$DOMAIN"
fi
echo "   证书：$CERT_DIR/"

echo "== 4/6 stunnel 同端口 TLS 透传"
command -v stunnel >/dev/null 2>&1 || apt-get install -y -qq stunnel4 >/dev/null
STUNNEL_BIN=$(command -v stunnel || command -v stunnel4)

{
    echo "foreground = yes"
    echo "syslog = no"
    echo "cert = $CERT_DIR/fullchain.pem"
    echo "key = $CERT_DIR/privkey.pem"
    echo "sslVersionMin = TLSv1.2"
    echo ""
    echo "[relay-control]"
    echo "accept = 0.0.0.0:$CONTROL_PORT"
    echo "connect = 127.0.0.1:$CONTROL_PORT"
    p=$(($DATA_BASE))
    while [ "$p" -le "$DATA_END" ]; do
        echo ""
        echo "[relay-data-$p]"
        echo "accept = 0.0.0.0:$p"
        echo "connect = 127.0.0.1:$p"
        p=$((p + 1))
    done
} > /etc/stunnel/astra-relay.conf

echo "== 5/6 systemd 服务（relay + stunnel）"
cat > /etc/systemd/system/astra-relay.service <<EOF
[Unit]
Description=Astra 手机遥控公网中继
After=network-online.target
Wants=network-online.target

[Service]
EnvironmentFile=$OPT/env
ExecStart=$OPT/astra-relay
Restart=always
RestartSec=3
LogsDirectory=astra-relay
DynamicUser=yes
NoNewPrivileges=yes
ProtectSystem=strict
ProtectHome=yes
PrivateTmp=yes
RestrictAddressFamilies=AF_INET AF_INET6

[Install]
WantedBy=multi-user.target
EOF

cat > /etc/systemd/system/stunnel-astra-relay.service <<EOF
[Unit]
Description=stunnel TLS termination for astra-relay
After=network-online.target astra-relay.service
Wants=network-online.target

[Service]
ExecStart=$STUNNEL_BIN /etc/stunnel/astra-relay.conf
ExecReload=/bin/kill -HUP \$MAINPID
Restart=always
RestartSec=3

[Install]
WantedBy=multi-user.target
EOF

# 续期后重启 stunnel（换证书生效）
cat > /etc/letsencrypt/renewal-hooks/deploy/restart-stunnel.sh <<EOF
#!/bin/sh
systemctl restart stunnel-astra-relay
EOF
chmod +x /etc/letsencrypt/renewal-hooks/deploy/restart-stunnel.sh

systemctl daemon-reload
systemctl enable --now astra-relay stunnel-astra-relay

echo "== 6/6 防火墙"
if command -v ufw >/dev/null 2>&1; then
    ufw allow 22/tcp  >/dev/null 2>&1 || true
    ufw allow $CONTROL_PORT/tcp >/dev/null 2>&1 || true
    ufw allow $DATA_BASE:$DATA_END/tcp >/dev/null 2>&1 || true
    ufw --force enable
    echo "   ufw 已放行 22 / $CONTROL_PORT / $DATA_BASE-$DATA_END"
else
    echo "   无 ufw，请自行放行 $CONTROL_PORT 与 $DATA_BASE:$DATA_END（TCP）"
fi

echo
echo "========== 部署完成 =========="
echo "桌面端「手机遥控 → 公网中继」填写："
echo "  服务器地址: $DOMAIN"
echo "  控制端口:   $CONTROL_PORT"
echo "  接入令牌:   $TOKEN"
echo "  TLS:        ✅ 勾选"
echo "验证：journalctl -u astra-relay -f   /   ls $LOGDIR/"
