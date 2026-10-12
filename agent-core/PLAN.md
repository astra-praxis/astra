# agent-core 项目规划

> 从「星野应用内 Agent 模块」升级为「可嵌入的 Agent Runtime SDK」。
>
> 输入来源：
> 1. Zed agent 架构对比（2026-10，见 `build-agent-runtime` skill / architecture.md）
> 2. 外部评审第一轮（Task/Artifact/Policy/Memory/Router 六抽象 + 优先级调整）
> 3. 外部评审第二轮（ModelCapability / MemoryProvider 先定义 / Rewrite 命名 / Task.context）
> 4. 外部评审第三轮（五个预留接口：Session / Context / Capability / Artifact 生命周期 / 企业 PolicyContext）
> 5. 现有代码盘点（本文件「现状」一节）
>
> 长期约束沉淀：`rust-agent-runtime` skill（全局，跨项目复用）
>
> 状态标记：✅ 已完成 / 🔨 进行中 / 📋 规划中 / ⏸ 明确推迟 / ❌ 明确不做

---

## 一、定位

**现在是什么**：astra 的 Tauri 宿主内嵌 Agent Runtime，四层结构（model / run / tool / thread），
参考 Zed agent 架构瘦身重写，约 8000 行，无 Tauri / GPUI 依赖。

**目标是什么**：任何 Rust 应用 `cargo add` 后，实现 2~3 个 trait + 自建 UI，即拥有完整 Agent 能力
的独立 crate 体系。对标对象不是「聊天机器人框架」，而是 Zed ACP / Agent SDK 这类基础设施。

**北极星验收标准**：

```
□ 独立 crate（可脱离 astra workspace 构建）
□ feature flags: fs / net / shell / skills / store-sqlite
□ AgentConfig 聚合配置，const 全部可覆盖
□ 持久化 / 权限确认 / 提问 / 模型接入 全部走 trait，宿主注入
□ prompt 与模型可见文案可注入（默认中文）✅
□ Envelope schema 快照测试 + 版本字段 ✅
□ 宿主无关 e2e 测试套件
```

---

## 二、现状盘点（2026-10）

### 已完成 ✅

| 层 | 内容 | 位置 |
|---|---|---|
| model | `ModelBackend` trait、`RigBackend`（OpenAI 兼容）、自研 `openai_sse`、工具调用聚合状态机、可重试错误分类 | `model.rs` / `openai_sse.rs` |
| run | Agent Loop：流式 → 工具拦截 → 权限三态 → 执行 → 续轮；取消优先（三处 select 覆盖）；空闲超时；max_turns 预算；自动压缩（CJK 感知估算、失败降级）；`ToolCallArgsDelta` 流式工具参数 | `run.rs` |
| tool | `AgentTool` trait、声明式 schema、`ToolOutput` + Artifact（v2：`{id, kind, location, metadata, preview}`，7 类语义 kind）、类型擦除注册表、工具错误回传模型不中断 Run | `tool.rs` + 各工具模块 |
| thread | `Thread`：会话消息序列、旧 Run 取消优先、失败保留部分消息 | `thread.rs` |
| permission | `decide()` 纯函数（工具名级 Allow/Confirm/Deny）、`PermissionBridge`、`ensure_within_root`（symlink 逃逸 / mkdir -p / 相对路径全覆盖）、`validate_args`（required 级） | `permissions.rs` |
| 工具集 | fs_basic、fs_tools（Zed 移植：搜索/编辑/移动）、shell_tool、net_tools（SSRF 门）、skill_tools、doc_tools、ask_tool（UserQuestionBridge） | `*_tools.rs` / `ask_tool.rs` |
| 事件 | `Envelope{schema_version=2, event_id, timestamp, session_id, run_id, sequence, event}` 全 serde，snake_case tag 有单测 + 快照契约锁定 | `events.rs` |
| 文案包 | `TextPack` 文案模板包：core 自有面向模型/用户文案全部收口，默认中文 `zh()`，宿主经 RunConfig/ThreadConfig 注入（166 测试全绿，宿主侧 cargo check 通过） | `texts.rs` |
| 宿主集成 | `src-tauri/src/agent.rs`：build_registry 按项目根组装、HubBridge 确认桥、QuestionBridge | src-tauri |
| 测试 | 权限判定表、取消时序、compaction、Envelope 形状、e2e 探针（examples/e2e.rs）、live_net 冒烟 | 各模块 + tests/ |

### 半成品 🔨

- **Plan**：`Plan`/`PlanStep`/`upsert` 已实现并有单测（events.rs），但 run_loop 未 emit `PlanUpdated`，无驱动机制。

### 对照 Zed 的六大差距

1. 双层线程模型缺失（无 UI entry 状态机 / diff 审查 / checkpoint）
2. 持久化未实现（Thread 纯内存）
3. 无 `AgentConnection` 外部 Agent 统一抽象
4. 权限模型简化（无路径级规则 / 硬编码安全规则 / profile / sandbox）
5. 工具执行串行、生态小、Plan 未接线
6. 模型层单薄（单后端、启发式 token 估算、无 prompt cache 意识）

### 现有优势（保持住，升级时不许破坏）

- 零宿主耦合：宿主能力全走 trait（ModelBackend / PermissionBridge / UserQuestionBridge）
- 事件通道由调用方持有（`send(text, event_tx)`），天然适配 Tauri / WebSocket / CLI
- 取消优先是全局竞态规则且有测试锁定
- `ensure_within_root` 路径边界实现质量高于多数同类
- 单位代码测试密度高

---

## 三、六个新抽象（升级核心）

> 定位排序即重要性。每个抽象给出：目标形态、与现有代码的衔接、**明确不做的事**。

### 3.1 Artifact（产物系统）—— 从「diff 附件」升为一级概念

**定位**：Agent 的价值不是调用工具，而是产出可被用户查看 / 审查 / 继续加工的产物。
Diff 只是 Artifact 的一种。

现有 `Artifact{path, kind: FileCreated | FileModified}` 是雏形，升级为：

```rust
pub struct Artifact {
    pub id: String,
    pub kind: ArtifactKind,
    /// 位置：相对项目根路径，或 URI
    pub location: String,
    /// 结构化元数据（页数 / 增删行数 / 格式 / 大小…），按 kind 约定 key
    pub metadata: serde_json::Value,
    /// 可选预览文本（代码 diff / 文本摘录），供 UI 直接渲染
    pub preview: Option<String>,
    /// 🔒 预留字段位（第三轮评审，不实现审批流）：
    /// pub version: u32            — 重新生成/用户修改后递增
    /// pub status: ArtifactStatus  — Draft→Generated→Reviewed→Approved→Published→Archived
}
```

pub enum ArtifactKind {
    FileChange,     // 含 diff，metadata: {insertions, deletions}
    Document,       // docx / xlsx / ppt（标书场景）
    Pdf,            // metadata: {pages, size}
    Image,
    CommandOutput,  // metadata: {command, exit_code}
    Data,           // json / csv / dataset
    Generic,
}
```

**衔接**：`ToolOutput.artifacts` 已存在 → 扩展结构 + 让 fs_tools / shell_tool 填充新 kind；
edit_file 落地时产出 `FileChange{preview: diff}`。

**❌ 不做**：
- 不在 core 做 PDF 视觉 diff / docx 段落 diff 的**计算**——core 只定义 Artifact 数据形状与
  preview 约定，各类 diff 的生成是工具实现方的事（后续可出 `agent-artifact-diff` 工具 crate）。
- 不做产物存储仓库（Artifact 只登记引用，文件在磁盘/对象存储由宿主管）。

### 3.2 PolicyEngine（策略引擎）—— 从「工具名三态」升为规则引擎

**定位**：权限不只是「用户允许吗」，而是「策略决定 Allow / Deny / Ask / Modify」。

```rust
pub enum Decision { Allow, Confirm, Deny }           // 保留，序列化形状不变
// 语义升级：decide 的输入从 ToolPolicy(工具名列表) 变为规则表

pub trait RuleProvider: Send + Sync {
    /// 返回按优先级排序的规则；路径规则取 most_restrictive（Zed 语义）
    async fn rules(&self, ctx: &PolicyContext) -> Vec<PermissionRule>;
}

pub struct PermissionRule {
    pub tool: Option<String>,        // None = 匹配任意工具
    pub path_pattern: Option<glob>,  // 路径级规则
    pub decision: Decision,
}
```

**衔接**：
- `decide()` 签名保持纯函数；内部从「查两个 Vec」变为「遍历 RuleProvider 规则取 most_restrictive」。
- `PermissionBridge` 原样保留（Confirm 的运行时批准通道不变）。
- `ensure_within_root` 保留为路径校验地基，规则层复用。
- 新增硬编码安全规则层（不可被配置覆盖）：拦 `rm -rf /`、越界写、危险 shell 模式——对齐 Zed
  `HardcodedSecurityRules`。
- 宿主侧（astra）的规则来源：settings JSON（`ToolPolicy` 序列化向后兼容，读入即转规则表）。

**❌ 不做**：sandbox 执行（ThreadSandbox / seccomp）——单独立项，不塞进 PolicyEngine 首版。

**命名与预留（第二轮/第三轮评审）**：
- 参数改写能力命名 **Rewrite**，不叫 Modify——它已接近 Intent Transformation，
  不属于权限判定语义；推迟实现（2.3 不排期）。
- 首版 Decision 只有三态 Allow/Confirm/Deny。
- 🔒 `PolicyContext` 预留企业字段位：`user_id / role / organization`（校园场景：
  老师可查成绩、学生不可查他人成绩）。字段位写入 trait 签名注释，不实现鉴权逻辑，
  否则未来企业权限系统会重写。

### 3.3 Task（任务层）—— 从「聊天 Thread」升为「目标驱动」

**定位**：Agent 的入口单元不是 message，是 goal。但**不引入调度器/执行引擎**，
Task 是 Thread 之上的轻量状态包装：

```rust
pub struct AgentTask {
    pub id: String,
    pub goal: String,
    pub status: TaskStatus,          // Planning / Running / AwaitingInput / Done / Failed / Cancelled
    pub plan: Plan,                  // 复用现有 Plan 结构
    pub artifacts: Vec<Artifact>,    // 任务级产物汇总
    pub thread_id: String,           // 底层会话
    /// 业务上下文（第二轮评审）：企业资质/招标文件/模板引用等，与 Thread 聊天上下文分离，
    /// 防止业务信息全部塞进 Thread（50k token 问题）。Run 组装请求时注入 system 上下文块。
    pub context: TaskContext,
}
```

**层级争议记录（第三轮评审，保留意见）**：保持「Task 是 Thread 之上的轻量包装」。
若未来主战场明确从编辑器类转向企业自动化，可翻转为
`AgentSession → Task → Conversation(Thread) → Run` 层级（Session 持有 user/project/
permissions/memory）。翻转条件写入决策记录，代码不预建。

**衔接**：
- 第一步就是接线半成品：run_loop 在工具调用前后驱动 `Plan::upsert` 并 emit `PlanUpdated`
  （驱动机制：工具执行钩子，不新增模型专用工具）。
- `AgentTask` 提供状态推进纯函数（事件 + task → 新 task），Thread 保持面向 LLM 不动。

**❌ 不做**：
- ❌ Scheduler / Executor 层（外部评审建议的 runtime 三件套）——当前单 Thread 串行足够，
  并发多任务由宿主开多个 Thread 实现。等出现真实的「多任务并行调度」需求再立项（YAGNI）。
- ❌ 多 Agent 通信协议（Planner→Coder 编排）——那是 AgentConnection 成熟后的应用层课题。

### 3.4 MemoryStore（长期记忆）—— 与 ThreadStore 分离

**定位**：Thread 短期上下文 ≠ 项目知识（企业资质 / 历史案例 / 模板）。

```rust
pub trait MemoryStore: Send + Sync {
    async fn put(&self, scope: &str, key: &str, content: MemoryEntry) -> Result<()>;
    async fn search(&self, scope: &str, query: &str, top_k: usize) -> Result<Vec<MemoryEntry>>;
}
```

**衔接**：astra 已有 `embedding.rs` + `vector_store.rs`（SQLite 余弦检索）——这就是现成的
MemoryStore 底座，实现 `agent-memory` crate 时直接平移，core 只定 trait。

**⏸ 推迟**：排在 ThreadStore 之后。理由：标书 / 校园场景启动前，vector_store 已可独立使用，
Memory trait 化不阻塞其他阶段。

### 3.5 ModelRouter（模型路由）—— ModelBackend 之上的薄层

**定位**：按任务特征选后端（简单问题 → 小模型，规划 → 大模型，代码 → coder 模型）。

```rust
pub trait ModelRouter: Send + Sync {
    fn select(&self, req: &ModelRequest, hint: RouteHint) -> Arc<dyn ModelBackend>;
}
// RouteHint: { phase: Chat|Planning|Coding, cost_bias, ... } 由 Task/Run 提供
```

**衔接**：`ModelBackend` trait 就是预留的缝，Router 只是插入在 run_loop 组装 `ModelRequest`
前的一个函数层。

**前置（第二轮评审）**：`ModelBackend` 增加 `capabilities()`（`ModelCapability{tool_call,
vision, context_tokens, reasoning}`，默认保守实现）——Router 未来按能力路由而非
硬编码 `if task == "code"`。定义排在阶段一（trait 形状也是契约）。

**⏸ 推迟**：排在 AgentConfig 之后、可与 Task 并行。首版允许「Router = 静态映射表」，
不做按负载 / 成本动态调度。

### 3.6 AgentConnection（连接抽象）—— 战略价值最高，最后做

**定位**：不止外部 CLI agent。是「Agent 通道」统一 trait：内置 Runtime、ACP 外部 agent
（claude / gemini CLI）、未来远程 agent 都是其实现，UI 无感知切换。

**衔接**：现有 `Thread + execute + Envelope` 就是内置实现的行为基线；trait 的 prompt / cancel /
事件流签名直接从这三者反推，对齐 Zed `acp_thread::AgentConnection`。

**⏸ 排最后**：不阻塞任何其他阶段；做之前先跑通 ACP 协议调研（acp_proto v1/v2）。

### 3.7 五个预留接口（第三轮评审：只留口，不实现）

> 平衡结论：现在全实现 = 过度设计；完全不考虑 = 未来重构。
> **保持现有阶段路线，为以下概念预留接口位置：**

| 概念 | 预留方式 | 不做 |
|---|---|---|
| Session（一级对象） | `AgentSession{user, project, permissions, memory} → Task → Conversation(Thread) → Run` 层级只记录在本文与注释，代码不建 | Session 生命周期管理 |
| TaskContext | AgentTask 增加 `context` 字段（见 3.3） | context 自动注入策略 |
| Capability/Provider | 工具未来视为 Capability 的实现（FileCapability → LocalFS/S3/Git Provider）。新工具照常写 AgentTool | 重写现有工具系统 |
| Artifact 生命周期 | 结构预留 version/status 字段位（见 3.1） | 审批流实现 |
| 企业 PolicyContext | 预留 user_id/role/organization 字段位（见 3.2） | 5W 全量权限模型 |

---

## 四、阶段规划（按外部评审调整后排序）

### 阶段一：SDK 地基（最高优先级，不做后面全返工）

| # | 项 | 内容 | 验收 |
|---|---|---|---|
| 1.1 | **AgentConfig** | 聚合配置：ToolPolicy→规则表、max_turns / compact 阈值 / idle timeout 从 const 变配置、项目根策略。✅ 文案模板包部分已落地（`texts.rs` TextPack，RunConfig/ThreadConfig 可注入，zh 默认逐字一致） | 余项：e2e 用纯 AgentConfig 跑通；astra settings 页可编辑 |
| 1.2 | **Event Schema 稳定化** | ✅ 已落地：Envelope 加 `schema_version`（`EVENT_SCHEMA_VERSION=1`，反序列化缺省兼容旧载荷）；全部 14 个变体 JSON 形状快照锁定（`src/events_testdata/`，含编译期穷尽守卫 + 覆盖守卫 + `write_event_snapshot` 再生成命令）；事件文档 `docs/EVENTS.md` | 改事件形状必须改快照 → 有意识地破坏 |
| 1.3 | **Artifact 规范化** | ✅ 已落地：Artifact 升级为 `{id, kind, location, metadata, preview}`（version/status 字段位注释预留）；ArtifactKind 扩为 7 类语义 kind（file_change/document/pdf/image/command_output/data/generic）；`EVENT_SCHEMA_VERSION=2`（快照重生成 `events_schema_v2.snapshot.json`）；edit_file 产出行级 diff（新增 `diff.rs` LCS 实现，insertions/deletions × 命中次数 + preview）；write/copy/move/extract/build_docx 填充 metadata 约定；run.rs 直接透传产物（ArtifactRef 删除）；177 测试全绿，src-tauri cargo check 通过 | e2e 断言产物结构；前端可渲染 kind 分支（前端当前零消费，无迁移成本） |
| 1.4 | **edit_file + diff** | 大部分已随 1.3 落地：EditFile 已产出 FileChange Artifact（metadata insertions/deletions × 命中次数，preview=行级 diff）；余项：e2e 场景中显式验证 diff 预览打印 | diff 预览事件可在 CLI e2e 中打印 |
| 1.5 | **Plan 接线** | run_loop 驱动 Plan upsert + emit PlanUpdated（3.3 第一步） | e2e 场景出现 plan_updated 事件 |
| 1.6 | **ModelCapability** | `ModelBackend` 增加 `capabilities()`（默认保守实现）：tool_call / vision / context_tokens / reasoning（3.5 前置） | RigBackend 与 MockBackend 均能声明能力 |
| 1.7 | **MemoryProvider 定义** | 只定义 trait（put/search），零实现（3.4 前置，防 TaskContext 设计时失锚） | trait 编译通过 + 文档注释 |

### 阶段二：安全（Policy Engine）

| # | 项 | 内容 | 验收 |
|---|---|---|---|
| 2.1 | **RuleProvider + 规则表** | 路径级规则、most_restrictive 合并、ToolPolicy 向后兼容读入 | 权限判定表测试扩展 × 路径维度 |
| 2.2 | **硬编码安全规则** | 拦 rm -rf、危险模式，配置不可覆盖 | 安全规则测试独立成表 |
| 2.3 | **Rewrite（参数改写，推迟）** | 规则改写参数后放行（如限定 shell 工作目录）。命名 Rewrite 不叫 Modify（第二轮评审，见 3.2）；接近 Intent Transformation，不排期 | 未排期，理由见决策记录 |

### 阶段三：状态（ThreadStore）

| # | 项 | 内容 | 验收 |
|---|---|---|---|
| 3.1 | **ThreadStore trait** | save / load / append_entry / list_metadata | trait 定义 + memory 实现供测试 |
| 3.2 | **agent-store-sqlite** | 独立实现 crate：流式期间增量保存、消息体 JSON（可选 zstd）、元数据表 | 重启恢复会话 e2e |
| 3.3 | **Run 中断恢复** | 失败/取消后从持久化点续跑（thread.rs 现有 TODO 兑现） | kill 进程后恢复测试 |

### 阶段四：生态（Connection / Memory / Router）

| # | 项 | 内容 |
|---|---|---|
| 4.1 | **AgentConnection trait** | 从现有内置实现反推签名；内置 Runtime 为首个实现 |
| 4.2 | **ACP 外部 agent** | 子进程 JSON-RPC / acp_proto，Claude/Gemini CLI 作为第二实现验证抽象 |
| 4.3 | **agent-memory** | MemoryStore trait + 基于 vector_store 的 SQLite 实现 |
| 4.4 | **ModelRouter** | 首版静态映射；RouteHint 由 Task 提供 |

### crate 拆分路线

```
阶段一~二：agent-core 单体（模块内聚，避免过早拆分）
阶段三：  + agent-store-sqlite（实现 crate 分离，core 只留 trait）
阶段四：  + agent-memory / agent-connection-acp
```

> 对外部评审「sqlite 不做 feature、直接拆 crate」的采纳说明：**采纳**，但拆分时机放在
> ThreadStore 落地时而非现在——当前单体没有第二个存储实现之前，拆 crate 只增加维护成本。

---

## 五、明确不做 / 暂缓清单

| 项 | 状态 | 理由 |
|---|---|---|
| Scheduler / Execution Engine | ❌ 暂缓 | 单 Thread + 宿主多开已够；出现真实并行调度需求再立项 |
| 多 Agent 编排协议 | ❌ 暂缓 | 依赖 4.1/4.2，是应用层课题 |
| Sandbox 执行 | ❌ 暂缓 | 独立立项，不进 PolicyEngine 首版 |
| Rewrite（参数改写） | ⏸ 推迟 | 接近 Intent Transformation，不属权限判定语义；命名已定，不排期 |
| PDF 视觉 diff / docx 结构 diff 计算 | ❌ 暂缓 | core 只定 Artifact 形状；diff 生成放工具 crate |
| 动态成本感知模型调度 | ⏸ | ModelRouter 首版静态映射 |
| 完整 JSON Schema 校验 | ⏸ | validate_args 升级为低优先级项，排阶段二之后 |
| prompt cache / 结构化 ReasoningDetails | ⏸ | 模型层深化，随实际 provider 需求做 |

---

## 六、风险与对策

1. **事件形状是事实契约** → ✅ 快照已生效（1.2）：任何形状变更必须 schema_version +1 →
   重写快照 → 同步 docs/EVENTS.md → 通知前端；新增变体会被编译期穷尽守卫拦截。
2. **中文硬编码扩散** → ✅ TextPack 已落地（1.1 文案部分），core 自有文案已全部收口。
   此后**新增面向模型/用户文案必须走 TextPack**，禁止硬编码字符串；
   工具模块内部文案随工具构造函数注入 `Arc<TextPack>` 渐进迁移。
3. **Plan 驱动机制过度设计** → 1.5 只做工具调用钩子驱动；若模型自管计划（task 工具类）
   有需求，作为工具实现叠加，不改 run_loop 核心。
4. **astra 宿主迁移** → 每阶段结束跑 astra 集成回归（agent_start_run 全链路 + e2e 探针），
   禁止长期分支。

---

## 七、决策记录

| 日期 | 决策 | 依据 |
|---|---|---|
| 2026-10 | 采纳外部评审的阶段重排：Config/Schema/Artifact 先于 Store/Connection | 没有配置与契约稳定，后面全部返工 |
| 2026-10 | Artifact 升为一级概念，diff 是其子集 | 目标场景（标书/校园）产物不止代码 |
| 2026-10 | 拒绝 Scheduler/Executor 层 | YAGNI；单会话场景无调度需求 |
| 2026-10 | Task 采用轻量状态包装而非新执行引擎 | 现有 run_loop 已覆盖执行语义 |
| 2026-10 | MemoryStore 基于 astra 现有 vector_store | 零新增基建 |
| 2026-10 | PermissionBridge 保留、内部升级 RuleProvider | 序列化兼容 + 运行时批准语义已验证 |
| 2026-10 | TextPack 落地：core 自有文案全部收口（texts.rs），zh 默认逐字一致 | 通用化前提（1.1 文案部分），166 测试全绿 |
| 2026-10 | Task 增加 context 字段：业务上下文与聊天上下文分离 | 第二轮评审；防 Thread 50k token |
| 2026-10 | ModelBackend 增加 capabilities()，Router 按能力路由 | 第二轮评审；避免 if task==code 硬编码 |
| 2026-10 | MemoryProvider 只定义不实现（阶段一 1.7） | 第二轮评审；TaskContext 设计时需要锚点 |
| 2026-10 | 参数改写命名 Rewrite 不叫 Modify，且推迟 | 第二轮评审；改写是 Intent Transformation 非权限语义 |
| 2026-10 | 第三轮评审五概念（Session/Context/Capability/Artifact 生命周期/企业 PolicyContext）：保持阶段路线不变，只预留接口位不实现 | 现在全实现 = 过度设计，完全不考虑 = 未来重构；详见 3.7 |
| 2026-10 | Task 层级保持「Thread 之上轻量包装」，记录翻转条件：主战场明确转向企业自动化时改为 Session→Task→Thread 层级 | 第三轮评审争议点；编辑器类场景下当前选择正确 |
| 2026-10 | 1.2 落地：schema_version=1 + 快照契约（穷尽守卫 + 覆盖守卫 + 再生成命令）+ docs/EVENTS.md | 事件是宿主间唯一通信契约，形状变更必须有意识破坏 |
| 2026-10 | 新代码注释/rustdoc/日志一律英文；存量不回改；模型/用户文案仍走 TextPack 默认中文 | 面向国际复用的 SDK 代码可读性；文案是产品内容不受影响 |
| 2026-10 | 1.3 落地：Artifact `{id,kind,location,metadata,preview}` + 7 类语义 kind，ArtifactRef 删除（schema v2）；文本行级 diff 是 core 唯一 diff 计算（LCS，`diff.rs`），PDF/docx diff 仍拒入 core | Diff 是 preview 内容形式非独立 kind；core 只定义形状的红线不变 |
| 2026-10 | 第四轮评审（事件协议）：采纳 event_id（派生 `evt_{run_id}_{seq}`，重放稳定）+ timestamp（additive 不 bump）；拒绝 payload 包装替代 flatten；拒绝 args delta_index（sequence 已保证序）；拒绝 location/preview 结构化（YAGNI） | event_id/timestamp 重连去重与 Agent Trace 即时需要；其余等生产方出现再加，additive 零返工成本 |
| 2026-10 | 第四轮评审推迟项打包到 v3：tool_call_progress（首个进度生产方落地时）、Permission risk_level/preview（阶段二 PolicyEngine）、agent_id（Phase 5 Scheduler）、plan children/progress（1.5 接线时评估）、status queued/paused（调度/暂停机制出现时） | 破坏性变更集中一次 bump，避免反复破坏快照契约 |
| 2026-10 | 测试规格采纳为验收基准（docs/TESTS.md）：能力闭环而非逐函数覆盖；补 envelope 兼容性（旧载荷/未知字段）与 sequence 单调性三个 P0 测试；活跃缺口登记 G-1~G-7 | 测试视角从实现正确性升级为 Runtime 能力完备性；新能力先登记场景再写测试 |
