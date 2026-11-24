# 手机遥控桥 R1+R2：持久设备 Key + 票据认证 + 固定端口 + 单会话 Scope

> 跟踪文档：需求汇总 + 实现方案 + 进度。随实现推进更新。
> 目标版本：v0.6.27（conventional commit + 附注 tag + push）。

## 一、需求汇总（用户原话归纳）

### 已完成（v0.6.26，勿重做）
- [x] 手机扫码绑定桌面端，查看全部会话 + 正在进行中的对话
- [x] 消息发送：执行中→入队（可排序/删除），空闲→直接发送
- [x] 用户消息按下发送即镜像到所有设备（不等 Run 准入，v0.6.26）
- [x] 模型/思考/自动模式选择从 PC 带到手机
- [x] 手机端 Markdown 渲染、对话样式对齐 PC（左侧满行、用户/agent 背景区分）
- [x] 输入框/发送按钮样式优化、对话区固定底部
- [x] 多人扫同一码：sender 标识 + 共享对话内容
- [x] SVG/动画页面优化、模型与思考弹框优化

### 本轮（R1+R2）
- [ ] **R1 持久设备 Key**：绑定做成类似 agent key 的形式——现在每次重启都失效要重新扫码；要支持失效时间（7天/30天）+ 永久 + 可手动撤销
- [ ] **R2 单会话 Scope**：扫码后现在是全部会话；支持「只分享一个对话」——scoped 设备只能看到/操作绑定的那个会话，其他会话完全不可见（桌面端强制执行）
- [ ] **固定端口**：不再随机端口，持久化端口选择（被占则回退），否则持久 key 救不了 URL
- [ ] **票据认证**：key 不进 URL query（长期凭证进日志会泄漏）；`/api/ticket` 换短时效一次性 ticket，WS/HTTP 用 ticket/Bearer
- [ ] **限速**：按 IP 指数退避（持久 key 给了暴力枚举无限时间）
- [ ] **撤销即踢下线**：撤销 key 时把该 key 的在线连接踢掉
- [ ] **手机 App 前置**：以上完成后具备做手机 App 的基础（key 长期有效 + 不依赖局域网的 relay 是后续 R3/R4）

### 后续路线（本轮不做，已与用户对齐）
- R3：服务器中继（relay）——突破局域网限制
- R4：手机 App（基于 R1 的持久 key + R3 的 relay）

## 二、关键设计决策（已定，不再讨论）

### 1. Key 数据模型
```json
{
  "key_id": "随机 id",
  "name": "设备名（手机自己上报或创建时指定）",
  "key_hash": "SHA-256(secret)，绝不落明文",
  "scope": "all | { session_id }",
  "expires_at": null | ms（null = 永久）,
  "revoked_at": null | ms,
  "last_used_at": null | ms,
  "last_ip": null
}
```
- key 本体（secret）只在创建时出现在 QR 中，丢失即撤销重建（GitHub PAT 逻辑）
- 配对二选一：**临时配对**（现行为，内存 token，重启失效，保留）+ **持久设备 key**（新）

### 2. 认证流程（票据制）
```
QR = url#<secret>（fragment 不进服务器日志/历史）
手机: location.hash 取 secret → POST /api/ticket (Authorization: Bearer <secret>)
    → { ticket }（内存 map，~30s TTL，单次使用）
    → WS /api/ws?ticket=<ticket>
    → 后续 HTTP fetch 用 Bearer <ticket or secret>
```
- 临时配对 token 也统一走 ticket 路径（同形处理，保持简单）
- 成功认证后 secret 存手机 localStorage（`mb_key`）——持久设备的既定取舍

### 3. Scope 强制执行（桌面端桥，不能只靠手机端隐藏 UI）
- `hello.sessions`：scoped 只下发绑定会话；`hello.current` 强制 = 绑定 sid；hello 增加 `scope: 'all'|'one'`
- `GET /api/session/:id`：非绑定 sid → 403
- **事件广播逐连接过滤**：agent-event / run-settled / agent-question 载荷带 session_id 按连接 scope 过滤；queue 快照按 sessionId 过滤 items；config/view/toast 广播不受限
- WS 命令（send / queue_reorder / queue_remove / answer）：scoped 连接强制 session_id = 绑定 sid
- 手机端 scoped 时隐藏「新对话」入口

### 4. 端口持久化
- 持久化端口选择（被占则递减回退并写回成功值），URL 稳定才能让持久 key 有意义

### 5. 限速
- 认证失败按 IP 记录，指数退避（内存 HashMap<ip, {fails, blocked_until}>）

### 6. 存储位置（本轮调研确认）
- `accounts.rs` 的规则：**新增存储必须经 `data_dir(manager)` 取路径，禁止直接 `app_data_dir()`**；没有公开的全局目录访问器
- 决策：key 文件与端口持久化都放 **账户私有 data_dir**（`mobile_bridge/keys.json` + `mobile_bridge/port.txt`）
- 取舍：账户热切换 = 桥随数据目录重绑，key 不跨账户共享（绑定的是账户的会话数据，语义一致）；写进提交说明

### 7. 新增 Tauri 命令
- `mobile_bridge_key_create(name, scope_session_id: Option<String>) -> { secret, url, qr_svg, id }`
- `mobile_bridge_key_list`
- `mobile_bridge_key_revoke(id)`（撤销 = 置 revoked + 踢掉该 key 在线 WS）

## 三、实现步骤

| # | 步骤 | 状态 |
|---|------|------|
| 1 | 读 RemotePage.tsx + accounts.rs（确认全局目录 + QR UI） | ✅ |
| 2 | Rust：key store（mobile_keys.rs：结构/持久化/validate/list/revoke）+ 端口持久化 | ✅ |
| 3 | Rust：auth 重构（/api/ticket、WS ticket、Bearer、per-IP 限速） | ✅ |
| 4 | Rust：scope 过滤（hello/HTTP/事件转发/queue/WS 命令）+ 撤销踢人 | ✅ |
| 5 | Rust：key_create/list/revoke 命令 + lib.rs 注册 | ✅ |
| 6 | 前端：mobile.ts key 管理 + RemotePage.tsx 持久 key UI（名称/scope/有效期 + 设备列表 + 撤销 + 创建后 QR 只显示一次） | ✅ |
| 7 | 手机 HTML：fragment 取凭证（mbk_→localStorage / 临时→sessionStorage）→ 换票 → WS ticket → Bearer fetch；scoped 隐藏新对话；401 清凭证显示失效门页 | ✅ |
| 8 | Mock 升级（/api/ticket、WS ticket、scope 过滤、多凭证：mocktoken=all / mbk_s1=仅 s1 / mbk_bad=401） | ✅ |
| 9 | 测试：Rust 单测 + node 测试三件套 + mock 浏览器实测 | ✅ |
| 10 | 提交 feat(remote) + tag v0.6.27 + push | ⬜ |

## 四、验证清单

- Rust 单测：key 生命周期（创建/过期/撤销/校验）、scope 过滤（hello + 事件 + queue + 命令）、限速、端口持久化
  - ⚠️ 全局静态坑：LIVE_RUNS/bus 是 OnceLock 全局，测试只断言自己 probe 的 key
- node 测试：`node --test src/stores/mobile.test.mjs src/stores/agent.test.mjs`（63 passed 基线）
- typecheck：`pnpm --dir src typecheck`
- cargo test：src-tauri 下（205 passed 基线）
- mock 浏览器实测：正常流 / scoped 流 / 撤销后拒绝 / 错误 key 限速
- ⚠️ 手机 HTML 是 `include_str!` 内嵌——改完必须重新 `cargo build` 才进桌面端

### 本轮验证记录（2026-11-24）

- cargo test：**215 passed / 0 failed**（含新增 mobile_keys 5 项 + mobile_bridge 认证/scope 6 项单测）
- node 测试三件套：**63 passed**；typecheck ✅；`pnpm --dir src build` ✅
- mock 浏览器实测（390×844）：
  - `#mocktoken`（临时 token, all）：hello 到达、2 会话、队列徽标 2、权限卡渲染、发送直接起跑 ✅
  - `#mbk_s1`（持久 key, 仅 s1）：自动落在绑定会话、1 会话、队列徽标 0（s2 条目被过滤）、抽屉无「新对话」、发送强制进 s1 并回放整轮、s2 起跑完全不可见 ✅
  - `#mbk_bad`：401 → 门页「绑定已失效，请重新扫码」，凭证未残留（localStorage/sessionStorage 清空）✅
  - curl 验证：scoped ticket 返回 `scope:one`；scoped /api/sessions 只返回 s1；scoped /api/session/s2 → 403；错误凭证 → 401
  - ⚠️ 调试坑：同 URL 仅换 fragment（#mocktoken→#mbk_s1）浏览器不会重载页面，需先去 about:blank 再导航

## 五、踩坑备忘

- edit_file 中文落地后 grep `兆|锟|斤拷` 查转义损伤；验证无 CR 混入
- grep 工具 include_pattern 不稳，优先终端 `grep -rn`
- 涉及认证字段的改动必须 grep 所有调用点：手机 HTML 三处 fetch（~L787/L881/L920/L1053）+ WS connect + Rust handler
- 手机页 fetch 是相对路径 `base` 变量
- mock 的 state 在 IIFE 闭包内，浏览器验证用 DOM 查询
- zustand subscribe 两参即可（mobile.ts 既有模式）
