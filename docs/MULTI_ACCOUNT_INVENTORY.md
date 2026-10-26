# 多账户 P0（冷切换）只读盘点清单

> 基于真实代码盘点（2026-10）。目标架构见《Astra 本地多账户架构设计》，
> 本文档是其 P0 阶段的三份前置清单：全量存储清单、账户归属矩阵、迁移与验收矩阵。
> 所有路径与入口均已核实到具体文件与函数，未做任何猜测。
>
> **实施状态（P0-1 / P0-2 / P0-3 / P0-4 已落地）**：
> - P0-1 / P0-2：账户模块 `src-tauri/src/app/accounts.rs`：accounts.json 元数据、
>   `--account=<id>` 启动参数绑定、账户数据目录解析器 `data_dir()`（全部账户
>   私有落盘的唯一入口）、`astra_home()`、ASTRA_HOME 注入、全局单实例锁
>   （pid + 存活检测）、account_status / account_list / account_create /
>   account_launch 四命令；全部落盘点已改走
>   `crate::app::accounts::data_dir()`（本清单 #1–16、21），
>   #17–19 走 `astra_home()` 或 ASTRA_HOME 注入；#20 系统技能保持共享；
>   前端账户门 `src/src/components/account/AccountGate.tsx`。
> - P0-3：`src-tauri/src/app/secret.rs`（每账户登录密码：Argon2id 派生密钥
>   包装随机 KEK，凭据存 `<data_dir>/.astra-auth.json`；XChaCha20-Poly1305
>   信封格式）。**keychain 已弃用**（会反复弹授权框）；legacy `.astra-kek`
>   0600 文件保留为 dev 兜底，auth_init 时承接并用密码重新包装。
>   `providers.json` 信封加密落盘（legacy 明文仍可读，首次写盘自动升级；
>   KEK 不可用时拒绝写明文）；`memory.key` KEK 包装格式
>   （ASTRA-WRAP1 + 信封，legacy 裸密钥读入后 best-effort 升级重写）。
> - P0-5（praxis 式登录）：账户门改为登录界面——选用户 → 输密码 →
>   登录（`account_launch` 验证密码 → KEK 写一次性交接 token（0600、即读即删）
>   → 冷切换）；新进程 setup 消费 token 免二次输密。绑定账户启动后处于
>   锁定态需 `account_unlock`。老账户（无凭据）首次登录设置初始密码。
>   设置中心：创建账户即设登录密码；更换密码 `account_password_change`
>   （KEK 不变仅重新包装）。密码仅本地验证，不做云同步。
> - P0-4：`src-tauri/src/app/migration.rs`（首个账户创建时一次性迁移 legacy
>   数据：APPDATA 根数据文件 → 账户 data/，config.toml copy 保留原件，
>   pastes/skills 搬移；marker 幂等，rename 失败回退 copy+保留）；
>   `account_launch` 冷切换收尾（agent_cancel + dev server 进程树终止）；
>   设置中心新增「账户」分区（AccountSettings，legacy 用户经此进入多账户）。

## 0. 已核实的架构事实（影响 P0 方案）

1. **未启用 single-instance 插件**：`lib.rs` 只注册 opener / dialog / fs /
   global-shortcut 四个插件，Cargo.toml 无 `tauri-plugin-single-instance`。
   双进程可同时打开并写同一套数据目录，P0-1 必须补。
2. **`~/.astra` 归属 astra-core**：`resolve_astra_home()` 实现在
   `astra-core/crates/astra-config/src/lib.rs:139`，支持 `ASTRA_HOME` 环境变量覆盖。
   astra 宿主（attach_extensions / mcp_config_tool）与 astra-core 运行时共用。
   ⇒ **冷切换的关键杠杆：按账户启动进程时注入不同的 `ASTRA_HOME`，
   MCP/ACP 配置的账户化在 astra-core 侧零改动。**
3. **AgentState 是 Tauri 全局 State**（`agent/state.rs`）：threads 单例锁、
   active 单槽、session_files、memory host 全部内嵌。P0 不做账户化重构，
   冷切换下每个进程只服务一个账户，AgentState 天然单账户。
4. **`memory.key` 是现成的 DEK 雏形**：`memory/host.rs` 已用 XChaCha20-Poly1305
   加密记忆库，仅主密钥明文落盘。P0-3 只需把该密钥改为「KEK 包装后落盘」，
   加密数据本体不动。
5. **`providers.json` 明文存 API Key**（`providers.rs`），且被 `memory/host.rs`
   （AgentSummaryClient）与模型 backend 多处读取，加密时需同步所有读取入口。

## 1. 全量存储清单

路径约定：`APPDATA` = Tauri `app_data_dir()`（macOS
`~/Library/Application Support/com.astra.app`）；`ASTRA_HOME` = `~/.astra`
（`ASTRA_HOME` 环境变量可覆盖）；`TMP` = `std::env::temp_dir()`。

| # | 数据 | 路径 | 创建 / 读取 / 写入 / 删除入口 | 模块 |
|---|------|------|------------------------------|------|
| 1 | 应用设置 | `APPDATA/settings.json` | get_settings_path → get_settings / save_settings 命令 | app/settings.rs |
| 2 | Provider 配置（**明文 API Key**） | `APPDATA/providers.json` | providers_path → providers CRUD 命令；memory/host.rs 与模型 backend 读取 | providers.rs |
| 3 | Thread/Run/Artifact/Checkpoint 库 | `APPDATA/agent_threads.sqlite3` | ThreadsStore::open(directory)——几乎所有 agent 命令入口都调用 | agent/threads.rs |
| 4 | Agent 会话条目流 | `APPDATA/agent_sessions.json` | session_store::ensure_json_file；list/save/delete_agent_session | agent/session_store.rs, agent/sessions.rs |
| 5 | 简单对话会话 | `APPDATA`（session_store 同族，`items.json`） | chat_sessions 命令 | chat.rs, chat_sessions.rs |
| 6 | **全量事件日志（敏感）** | `APPDATA/agent_events.log` | log_event 追加（每条 Envelope）；agent_recent_events 读取 | agent/events.rs |
| 7 | 应用日志 | `APPDATA/astra.log`（+.old 轮转） | logging::init（setup 阶段）；app_log! 宏 | logging.rs |
| 8 | 崩溃日志 | `TMP/astra-panic.log` | lib.rs panic hook | lib.rs |
| 9 | 记忆库（已加密） | `APPDATA/memory-cme.sqlite`（legacy: memory.sqlite） | memory host setup 初始化；memory_search/remember/forget | memory/host.rs |
| 10 | **记忆主密钥（明文）** | `APPDATA/memory.key` | memory host setup（缺失时生成） | memory/host.rs |
| 11 | RAG 向量索引 | `APPDATA/vector_index.db` | embed_text / store_chunks / search_similar / delete_project_index | memory/vector_store.rs |
| 12 | 用户技能库 | `~/.config/astra/skills/`（非 APPDATA；`skills/user.rs app_skills_dir`） | list/create/import/delete_user_skill；skill 文件读写命令 | skills/user.rs |
| 13 | 生成媒体资产 | `APPDATA/media/`（占位: `TMP/astra-media`） | MediaStore::new + GenerationStack sync（setup 指向 app_data_dir/media） | agent_generation/stack.rs |
| 14 | 图片生成/识别配置 | `APPDATA/generation.json` | agent_generation config 读写 | agent_generation/config.rs |
| 15 | 下载历史 | `APPDATA/download_history.json` | downloader 命令 | app/downloader.rs |
| 16 | 开发模式历史 | `APPDATA/dev_history.json` | dev_mode history 读写 | app/dev_mode/history.rs |
| 17 | MCP/ACP 扩展配置 | `~/.astra/config.toml` | market.rs astra_config_path（读/写）；mcp_config_read/write 宿主工具（读/写，toml_edit 保注释）；attach_extensions 每次 Run 前经 agent_core::astra_config::load() 读取 | extensions/*，agent/run.rs |
| 18 | 纯聊天粘贴溢出文件 | `ASTRA_HOME/pastes/paste-<ts>.txt` | agent_spill_paste（无项目 root 时） | agent/artifacts.rs |
| 19 | 项目内粘贴溢出文件 | `<project>/.astra/pastes/` | agent_spill_paste（有项目 root 时）——项目真实目录，OS 权限边界 | agent/artifacts.rs |
| 20 | 系统技能（只读共享） | `~/.agents/skills/` | scan_skills / read_skill_doc / update_skill 等只读+显式更新 | skills 系统命令 |
| 21 | 生成资产占位目录 | `TMP/astra-media` | GenerationStack 懒构建占位，setup 后弃用 | agent_generation/stack.rs |

清单外的隐含规则：任何新增存储若直接 `app.path().app_data_dir()` 或
`resolve_astra_home()`，即落入本清单管辖（见 §2 白名单）。

## 2. 账户归属矩阵

分类定义（写入规则白名单，新存储必须先归类）：

- **账户私有**：默认放 `APPDATA/accounts/<account_id>/`，任何命令经账户目录解析器取路径。
- **应用共享**：放 `APPDATA/app/`，须在代码注释与本矩阵登记共享理由。
- **临时**：`TMP`，必须声明清理时机。
- **密钥凭据**：明文禁止落盘；进 KEK 包装流程（P0-3）。
- **OS 边界**：项目真实目录 / 系统目录，Astra 只记录引用，不做隔离声明。

| # | 数据（同上表编号） | 归属 | 说明 |
|---|------|------|------|
| 1 | settings.json | **拆分**：窗口偏好→应用共享；工作空间/编辑器/主题/搜索引擎→账户私有 | 迁移时按字段拆 |
| 2 | providers.json | 账户私有 + **密钥凭据** | API Key 明文是 P0-3 第一目标 |
| 3 | agent_threads.sqlite3 | 账户私有 | 含会话正文与 Artifact 索引 |
| 4 | agent_sessions.json | 账户私有 | |
| 5 | chat 会话 | 账户私有 | |
| 6 | agent_events.log | 账户私有 + 敏感 | 按账户分文件后仍需最小化：考虑剔除正文只留元数据，或纳入加密范围 |
| 7 | astra.log | 账户私有（或应用共享但禁止记录账户内容） | 现状可能含对话片段（app_log），默认按账户私有处理 |
| 8 | astra-panic.log | 临时 | 崩溃信息可能含数据，列入 P0-3 检查 |
| 9 | memory-cme.sqlite | 账户私有 | 已加密，随目录迁移即可 |
| 10 | memory.key | **密钥凭据** | P0-3 改为 KEK 包装落盘（包装后文件仍是账户私有） |
| 11 | vector_index.db | 账户私有 | 行级已按项目分域，目录级账户化即可 |
| 12 | skills/（用户技能） | 账户私有（账户模式：`accounts/<id>/astra-home/skills/`；legacy：`~/.config/astra/skills` 不变） | 跨账户经显式导出/导入 |
| 13 | media/ | 账户私有 | |
| 14 | generation.json | 账户私有 | |
| 15 | download_history.json | 账户私有 | |
| 16 | dev_history.json | 账户私有 | |
| 17 | ~/.astra/config.toml | 账户私有（账户模式：启动时注入 `ASTRA_HOME=accounts/<id>/astra-home`，astra-core 的 resolve_astra_home() 自动跟随，runtime 零改动） | mcp_config_write 宿主工具无需改路径逻辑 |
| 18 | ~/.astra/pastes | 随 ASTRA_HOME 账户化 | 同上，agent_spill_paste 的 home 分支改用账户 home |
| 19 | <project>/.astra/pastes | OS 边界 | 项目真实目录，不声明账户隔离 |
| 20 | ~/.agents/skills | 应用共享（只读） | 系统技能，符合设计「内置资源可共享」 |
| 21 | TMP/astra-media | 临时 | 改为账户目录占位或直接移除占位逻辑 |

## 3. 迁移与验收矩阵

旧版单用户数据现状：全部在 `APPDATA/` 与 `~/.astra/` 根部（即上表路径去掉
`accounts/<account_id>/` 中段）。迁移策略：**首次创建账户时整体归入该账户，
状态机 `未迁移 → 迁移中 → 校验通过 → 完成`，可重复执行，原始数据在校验通过后
才进入清理阶段。**

| 数据 | 旧 → 新 | 迁移动作 | 失败恢复 | 隔离/验收测试 |
|------|--------|---------|---------|--------------|
| providers.json | `APPDATA/` → `accounts/<id>/` + 字段加密 | 拷贝→读旧值→KEK 包装 API Key→写新→校验可解→删旧 | 校验失败保留旧文件原样；绝不先删 | 静态扫描新目录无明文 key；旧文件清理后账户 B 无法读取 |
| agent_threads.sqlite3 | `APPDATA/` → `accounts/<id>/` | 停止所有 Run → 文件级 move（同卷原子）→ open 校验表结构版本 | move 失败原文件未动；保留迁移中标记可重试 | 账户 A 迁移后 agent_recent_sessions 仅见原会话；账户 B 看不到 |
| agent_sessions.json / chat 会话 | 同上 | 同上 | 同上 | 两账户列表互不可见 |
| agent_events.log | `APPDATA/` → `accounts/<id>/` | 文件级 move | 同上 | 账户 B 的 agent_recent_events 不含账户 A 事件 |
| memory-cme.sqlite + memory.key | `APPDATA/` → `accounts/<id>/` | move + 主密钥改 KEK 包装（解旧 key→包装→落盘→验证可解密→清理旧 key 文件） | 任何一步失败保留旧 key 与旧库 | memory_search 在账户 B 返回空；锁定态下无法解密记忆 |
| config.toml（MCP/ACP） | `~/.astra/` → `accounts/<id>/astra-home/` | 拷贝（保留注释）→ 注入 ASTRA_HOME 启动 → attach_extensions 验证工具数一致 | 拷贝失败保留 ~/.astra 原样（可能与 astra-core CLI 共用） | 账户 A 安装的 MCP 不出现在账户 B 的 extensions_list |
| 用户技能 skills/ | `APPDATA/` → `accounts/<id>/` | 目录拷贝 → 计数校验 | 同上 | 账户 B list_user_skills 不含账户 A 自建技能；`~/.agents/skills` 两账户均可见 |
| vector_index.db | `APPDATA/` → `accounts/<id>/` | move → get_index_stats 校验 | 同上 | 账户 B search_similar 不返回账户 A 索引内容 |
| media/ + generation.json | 同上 | 目录 move + 配置 move | 同上 | 账户 B 的 Artifact 列表为空 |
| settings.json | 字段拆分 | 解析旧 JSON → 按归属矩阵拆两份 → 校验 | 解析失败保留旧文件 | 账户 B 不继承账户 A 的 active_workspace |
| 下载/开发历史 | `APPDATA/` → `accounts/<id>/` | move | 同上 | 列表互不可见 |

### P0 进程级验收（每项都要有自动化或手动脚本）

1. **单实例**：启动第二个进程 → 聚焦/唤起已有实例而非双开（新增
   tauri-plugin-single-instance）；账户切换请求由主实例协调「旧进程安全退出 →
   以目标账户启动」。
2. **账户启动参数**：进程启动必须携带目标 `account_id` 参数（由主实例传入），
   无参数且未初始化 → 创建账户流程；无参数且已初始化 → 账户选择页。
   拒绝无效 `account_id`（不存在 / 迁移中 / 锁定未解锁）。
3. **账户活动锁**：`accounts/<id>/` 下 lockfile（含 pid）防双进程写同一账户；
   单实例插件为主，lockfile 兜底竞态。
4. **冷切换子进程收尾**：退出前 shell_background 任务、MCP server 进程
   全部终止（对齐现有 agent_cancel 的清理语义），超时强制 kill；
   验收：切换后 `ps` 无残留 MCP/shell 子进程。
5. **迁移原子性**：中断迁移（kill 进程）→ 重启后迁移状态机可续跑，
   原始数据完整。

### 全局数据白名单规则（对齐设计的「三个工程约束」之 3）

新增任何存储必须先在本文件 §2 登记归属，规则：

- 账户私有 = 默认。路径一律经账户目录解析器（Rust 侧唯一入口，参照
  `resolve_open_path` 的集中解析模式），禁止直接调用 `app_data_dir()`。
- 应用共享需在 PR 中说明共享理由并更新本表。
- 临时数据必须声明清理时机。
- 密钥凭据必须走 KEK 包装，PR 中不得出现明文落盘路径。

## 附：P0 不做清单（防 scope 蔓延）

- 不做进程内热切换 / AccountContext 重构 AgentState
- 不做账户间实时共享记忆或共享工作空间
- 不做 OS 账户级强隔离（独立 OS 用户 / 文件访问代理）
- 不做账户级 UI 多开（单实例单账户）
- 不做远程账户、云备份
