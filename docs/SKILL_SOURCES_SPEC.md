# 技能来源与按需加载 · 设计稿（Skill Sources v1）

> 状态：**设计稿（未实现）**。对应 astra-core `docs/SKILL_SPEC.md`（Skill v1，已冻结）的宿主侧扩展，
> 不改动 Skill manifest 协议本身。

## 结论先行（这套东西怎么用）

1. **一个来源 = 一个目录**。把 `~/.agents/skills`、`~/.claude/skills`、应用技能库、项目内 `.claude/skills`
   各登记为一个来源，每个来源有 **id**（`agents` / `claude` / `app` / `project`）和 **开关**。
2. **「一次只用某目录下某些技能」= 目录级开关 + 技能级白/黑名单**。关掉的来源根本不扫描、不进 prompt、
   也不进 `list_skills` 结果——这是你说的「不用一次全量加载」的落点。
3. **prompt 只放索引，不放正文**。system prompt 里只有「名字 + 一句话描述」，且有硬预算（24 条 / 1200 字符）；
   正文永远靠 `read_skill` 按需读，所以技能涨到几百个，prompt 体积也不涨。
4. **显式永远能覆盖**。`/app:pdf-extract` 这种带来源的写法无视开关直接可用，避免「关掉来源后连临时用一个都做不到」。
5. **默认零变化**。未配置来源时等价于现在的单源 `app`，prompt 逐字节一致——升级不会改变任何现有行为。

---

## 一、现状与问题（v0.6.20）

技能链路（全部绑定**单一根目录**）：

| 环节 | 位置 | 行为 |
|---|---|---|
| 目录 | `skills::user::app_skills_dir()` | 账户 `astra-home/skills`，legacy `~/.config/astra/skills` |
| 摘要 | `agent/run.rs` → `summary_in(dir)` | 遍历**全部**技能，逐个读 `SKILL.md` 取 frontmatter |
| 注入 | `run.rs` + `prompts/agent_skills_section.md` | **全量**拼进 system prompt（`- name: desc`，无上限） |
| 工具 | `agent/registry.rs` → `skill_tools::register(reg, dir)` | `list_skills` / `read_skill` 绑定单根 |
| 显式调用 | `run.rs::build_user_content` | `/技能名` 直接读 SKILL.md 包进用户消息 |
| 前端 `/` 菜单 | `pages/agent/AgentPage.tsx` → `list_user_skills()` | 同样单根 |
| 全局 Git 技能 | `skills/registry.rs::list_skills()` | 扫 `~/.agents/skills`（合并页面后仅详情页 fallback 使用） |

问题：

1. **单根，不能并存多来源**：`.claude/skills`、`.agents/skills`、应用技能库各有技能，目前只能进一个。
2. **全量注入**：技能数与 prompt 体积线性相关（每条 ≈ `name` + ≤120 字符描述），几十上百个就开始挤占上下文。
3. **没有选择能力**：无法表达「这次只用 A 目录下的这几个技能」。
4. **无缓存**：每次 `agent_start_run` 都把每个 `SKILL.md` 重读一遍。

---

## 二、目标 / 非目标

**目标**

- G1 多来源：任意数量的技能根目录（用户级 / 项目级），可启停。
- G2 选择性：目录级开关 + 技能级白/黑名单 + 会话级作用域。
- G3 按需加载：system prompt 只注入**索引**（有预算上限），内容永远按需 `read_skill`。
- G4 确定性：同名冲突、匹配顺序在任何输入下结果一致。
- G5 兼容：未配置时行为与现状逐字节一致。
- G6 安全：只读、只在白名单根内、不新增权限面。

**非目标**

- 不给技能权限、不执行技能内代码（沿用 SKILL_SPEC 职责边界）。
- 不改 Skill v1 manifest（`SKILL.md` / `skill.yaml`）字段与解析规则。
- 不做远程市场拉取（安装仍走 Agent 对话）。

---

## 三、数据模型

### 3.1 来源配置（持久，`settings.json`）

```rust
pub(crate) struct SkillSourceConfig {
    pub id: String,            // 唯一标识，同时用作限定名前缀（如 "agents"、"claude"、"app"、"project"）
    pub label: String,         // 展示名
    pub path: String,          // 根目录，支持 ~ 展开
    pub scope: SkillSourceScope, // user | project
    pub enabled: bool,         // 目录级总开关（"只用这个目录"就是这个）
    #[serde(default)] pub include: Vec<String>, // 白名单：技能名或 glob；空 = 全部
    #[serde(default)] pub exclude: Vec<String>, // 黑名单，优先级高于 include
    pub priority: u8,          // 同名冲突胜负：大者胜
}
```

### 3.2 运行时索引（进程内，按根缓存）

```rust
/// 一次装配（= 建 Thread 时快照）内的技能目录索引
pub struct SkillCatalog {
    entries: Vec<SkillEntry>,           // 已按启用/范围过滤、已解决同名遮蔽
    shadowed: Vec<(String, String, String)>, // (name, 胜者 source, 败者 source)
}

pub struct SkillEntry {
    pub id: String,          // 目录名（沿用 SKILL_SPEC：目录名 = 技能 id）
    pub name: String,        // 展示名（frontmatter name → 目录名）
    pub description: String, // ≤120 字符（SKILL_DESC_MAX_CHARS）
    pub source: String,      // 来源 id
    pub root: PathBuf,
    pub dir: PathBuf,        // 技能目录（只读）
}
```

- 解析顺序回退：`id` 相同取 `priority` 高者；同 priority 时 `project` > `user`；仍相同按 `id` 升序（BTreeMap 天然确定性，对齐 SKILL_SPEC 第五节「最低 id」规则）。
- 败者不消失：留在 `shadowed` 里给 UI 展示（"被 x 覆盖"），agent 侧不注入。

---

## 四、作用域与优先级（三层覆盖）

```
会话作用域（显式选择，内存）        ← 最高
   ↓ 未指定则
项目配置 <project>/.astra/skills.toml   ← 中（随仓库走，项目会话生效）
   ↓ 未指定则
全局设置 settings.json.skill_sources    ← 默认
```

**会话作用域**（`agent_start_run` 新增字段）：

```rust
pub struct SkillScope {
    pub sources: Vec<String>,  // 只用这些来源（空 = 默认集合）
    pub skills: Vec<String>,   // 只用这些技能（空 = 不限）
}
```

- 前端 chip 显示当前范围；切换会话时重置为默认（P2 可选：写入 `agent_sessions.json` 随会话恢复）。
- 与既有「建 Thread 时冻结」语义一致：**作用域变更需新会话生效**（和 MCP 工具集合同一条心智，避免运行中索引漂移）。

**项目配置** `<project>/.astra/skills.toml`：

```toml
# 本项目只用项目内技能 + 应用技能库
sources = ["project", "app"]

# 按来源细化（可选）
[[include]]
source = "app"
names = ["pdf-*", "docx"]

[[exclude]]
source = "agents"
names = ["*-experimental"]
```

- 缺失文件 = 不约束（继承全局）。
- 写这个文件不交给 agent（用户手写或 UI 生成）；如需 agent 帮写，走 Confirm 级工具（P3）。

---

## 五、分层加载与预算（核心：不全量）

| 层 | 数据 | 时机 | 成本 |
|---|---|---|---|
| **L0 索引** | `SkillCatalog`（name + 120 字符描述） | 建 Thread 时一次扫描（启用来源） | 每技能只读 `SKILL.md` 头部 8KB 供 frontmatter |
| **L1 提示词索引** | 目录的子集，**硬预算** | 建 Thread 时渲染一次（`system_with_skill` 之前） | 有上限，与技能总数无关 |
| **L2 内容** | `SKILL.md` 正文（≤30k 字符） | `read_skill` / `/技能名` 触发 | 按需 |

**L1 预算**（常量放宿主侧，可后续进设置）：

```rust
const MAX_PROMPT_SKILLS: usize = 24;   // 最多注入条目数
const MAX_PROMPT_CHARS: usize = 1200;  // 索引总字符上限
```

超出时的做法（不丢能力）：

```text
## 可用技能
（列出的技能够用了就先 list_skills 检索——技能库里还有 N 个未列出）：
- pdf-extract: 从 PDF 抽取结构化文本 […]
...
另有 37 个技能未列出（来源：app 20 / agents 17）。任务相关时先调用 list_skills(query) 查找。
```

- 排序：作用域内按 `priority` 降序 → 最近使用频次（P3）→ `id` 升序。
- 未激活任何技能时，system prompt 与现状**增量对比**一致（沿用 SKILL_SPEC 第六节的字节一致要求）。

**L0 缓存**：

```rust
// key = (canonical root, 根目录 mtime 秒)
static CATALOG_CACHE: Mutex<HashMap<(PathBuf, u64), Arc<SkillCatalog>>>
```

- 目录 mtime 变化才重扫；单技能内容变化不影响索引（描述变化到下次重扫为止，可接受）。
- 单根技能数上限 500（超出记 warning 并在 UI 提示），沿用 `WALK_MAX_ENTRIES` 的文件级预算。

---

## 六、工具契约（工具名不变）

`list_skills` / `read_skill` 已在 `ToolPolicy::default()` 的 `allowed_readonly` 白名单里
（astra-core `crates/astra-permission/src/permissions.rs`，`strict()` 预设里也在 confirm 名单），
**名称不变 = 宿主的 `host_tool_policy()` 零改动**。

```jsonc
// list_skills（新增可选参数，向后兼容）
{
  "type": "object",
  "properties": {
    "query":  { "type": "string", "description": "按名称/描述检索（子串，大小写不敏感）" },
    "source": { "type": "string", "description": "只看某个来源" },
    "limit":  { "type": "integer", "description": "默认 20" }
  }
}
// 返回
"Available skills (source shown in parentheses):\n- pdf-extract (app): …\n"

// read_skill
{ "name": "pdf-extract", "source": "app", "file": "SKILL.md" }
// source 缺省：作用域内唯一 → 直接用；有同名 → 取 priority 高者（并在输出里注明来源）
```

实现方式二选一（推荐 a）：

- **a. core additive**：`astra-skill`（`crates/astra-skill`）增加 `MultiRootCatalog`（id → root 解析 + 合并 + 遮蔽），
  `astra-tools::skill_tools`（`crates/astra-tools`）增加 `register_catalog(reg, Arc<SkillCatalog>)`；
  旧 `register(reg, dir)` 保留。
  优点：解析/校验单一事实源。需同步 `SKILL_SPEC.md`（additive，不改 v1 冻结语义）。
- **b. 宿主侧适配**：在 `src-tauri/src/skills/tools.rs` 实现两个同名工具，内部复用
  `astra_skill` 的 `parse_frontmatter_field` / `validate_skill_name` / `validate_rel_path`。
  优点：不动核心仓库、不卡发布；缺点：工具实现出现第二份（可接受，逻辑很薄）。

> 决策建议：P1 走 b（先解决诉求），P2 归一化到 a（若 core 有版本窗口）。

---

## 七、配置与存储

- **全局**：`settings.json`（已账号隔离，`accounts::data_dir`）新增 `skill_sources`。
  **不放 `~/.astra/config.toml`** —— 那是 MCP/ACP 扩展配置且已暴露给 agent 写入（`mcp_config_write`）；
  技能来源不该被 agent 改写。
- **项目**：`<project>/.astra/skills.toml`（随仓库分发，用户/团队可提交）。
- **自动发现候选**（首次启动，只登记不启用，保持现状行为）：
  - `~/.agents/skills`（全局 Git 技能，已在用）
  - `~/.claude/skills`（Claude Code 技能，格式同为 `SKILL.md` + name/description frontmatter，天然兼容）
  - `<project>/.agents/skills`、`<project>/.claude/skills`、`<project>/.astra/skills`（scope = project）
  - 应用技能库（`app`，默认 `enabled = true`，优先级最高，保持现状）

新增 Tauri 命令（只读）：

```rs
list_skill_sources()            // 来源列表 + 自动发现候选 + 每个来源的技能数
skill_catalog(scope: Option<SkillScope>)   // 已过滤/已解决遮蔽的索引（前端分组渲染 + / 菜单）
```

写操作（增删改来源、勾选 include/exclude）走已有 settings 保存路径；
旧的 `list_user_skills` / `list_skills` / `read_skill_doc` 保留（详情页 fallback 与 market 页在用）。

---

## 八、安全约束

- **根白名单**：只扫配置里存在的根；`~` 展开后 `canonicalize`，技能目录必须落在根内（防 `..` / 符号链接逃逸）。
- **project 来源**：根必须 canonical 后位于 `project_root` 内，否则拒绝（并在 UI 标记非法）。
- 名称/相对路径校验沿用 `astra_skill::validate_skill_name` / `validate_rel_path`（单一事实源）。
- 只读：不新增任何写入路径；`/技能名` 注入沿用 `SKILL_READ_MAX_CHARS` 截断。
- 预算上限：单根 ≤500 技能、文件级遍历预算（沿用宿主 `src-tauri/src/skills/user.rs::WALK_MAX_ENTRIES` = 1000）、
  L1 硬字符上限——三层都在宿主侧硬编码防护。

---

## 九、前端 UI

| 位置 | 变更 |
|---|---|
| 市场「Skill 技能」页（`pages/market/`） | 按**来源分组**：源标题 + 开关 + 计数 + 展开技能；卡片加来源徽标；冲突技能标「被 x 覆盖」 |
| 技能详情页（`pages/skills/SkillDetailPage.tsx`） | 头部展示来源徽标；`read_skill_doc` 仍按路径读，无需改协议 |
| Agent 输入区 `/` 菜单（`pages/agent/`） | 按来源分组 + 搜索框（技能多时必配）；选中后 chip 显示 `来源:技能` |
| Agent 输入区顶部 | 作用域 chip：`技能范围: app（3 个技能）`，点击展开选择来源/技能 |
| 设置页新增「技能来源」分区（`pages/settings/`） | 增删来源、路径、优先级、启停、include/exclude 编辑；自动发现候选一键加入 |

限定名语法（后端 `parseSendInput` 扩展）：

```
/pdf-extract            # 作用域内唯一 → 直接用；同名冲突报错并给候选
/app:pdf-extract        # 指定来源，永远可用
```

---

## 十、迁移与兼容

1. `settings.json` 无 `skill_sources` → 等价单源 `app`（`enabled=true`），prompt 与现状**逐字节一致**。
2. 自动发现只**登记候选**（disabled），首次进入技能页提示「检测到 `~/.agents/skills`、`~/.claude/skills`，启用后这些技能才参与」。
3. 旧命令保留；前端 `/` 菜单改调 `skill_catalog`，失败时回退 `list_user_skills`（本地调试友好）。
4. `/技能名` 语义不变；仅当存在同名冲突时要求 `来源:技能`。

---

## 十一、分阶段实施

### P1 · 多来源 + 按需加载（解决本次诉求）

- [ ] `SkillSourceConfig` + `AppSettings.skill_sources`（含默认值与自动发现候选）
- [ ] `skills/sources.rs`：根展开/canonical/过滤；`skills/catalog.rs`：多根扫描 + 遮蔽 + mtime 缓存
- [ ] `agent/run.rs`：装配改用 catalog；L1 预算渲染（`MAX_PROMPT_SKILLS` / `MAX_PROMPT_CHARS`）
- [ ] `agent/registry.rs` / `skills/tools.rs`：多根 `list_skills`（query/source/limit）+ `read_skill`（source）
- [ ] 命令：`list_skill_sources` / `skill_catalog`
- [ ] 前端：market 页按来源分组 + 源开关；`/` 菜单分组 + 搜索；设置页「技能来源」分区
- **验收**：两个目录各放技能 → 只启用 A → prompt 只出现 A；技能数 100 时 prompt 索引 ≤ 预算；`/b:only-b` 能跨来源调用被过滤的技能（显式优先）。

### P2 · 项目级与会话级选择

- [ ] `.astra/skills.toml` 解析与合并（sources + 按来源 include/exclude）
- [ ] `agent_start_run` 增加 `skill_scope`；store 会话作用域 + chip UI
- [ ] `来源:技能` 限定名（前端解析 + 后端 source 解析）
- **验收**：同一项目下切换会话作用域立即生效（新会话）；`/技能名` 同名冲突时有确定行为。

### P3 · 规模化增强（技能真的很多时）

- [ ] 使用频次加权排序（记录在会话/项目维度，仅影响 L1 排序）
- [ ] 语义检索：复用 RAG 子系统（`embedding.rs` / `vector_store.rs`）对技能描述建索引，`list_skills(query)` 升级为语义 + 关键词混排
- [ ] 来源变更热生效（免新建会话），需处理运行中索引一致性
- [ ] agent 帮写项目配置的工具（Confirm）

---

## 十二、测试清单

**Rust 单测**

| 层 | 用例 |
|---|---|
| sources | `~` 展开、canonicalize、`..` 逃逸拒绝、project 根越界拒绝、非法 id 拒绝 |
| catalog | 多根合并、同名遮蔽（priority / scope / id 三级裁决）、include/exclude glob、隐藏目录跳过、单根 500 上限 |
| 预算 | L1 条目数/字符上限、超出提示行、按 priority 截断顺序、空技能库时 prompt 逐字节不变 |
| 缓存 | 目录 mtime 变化触发重扫；未变化命中缓存 |
| tools | `list_skills({query})` 子串匹配、`source` 过滤、`read_skill` 缺省 source 解析、同名二选一稳定 |

**前端**：来源分组渲染、源开关即时反映、`/` 菜单搜索、作用域 chip、冲突徽标。

**端到端**：`~/.agents/skills` 放 2 个 + `~/.claude/skills` 放 2 个 → 只启用 agents → 运行一轮，确认 prompt 索引只含 agents、`read_skill` 能读被过滤来源的技能（显式 `source`）。

---

## 十三、影响面清单（P1）

```
src-tauri/src/app/settings.rs          + skill_sources 字段与默认值
src-tauri/src/skills/sources.rs        (新) 来源展开/校验/过滤
src-tauri/src/skills/catalog.rs        (新) 多根扫描 + 遮蔽 + 缓存
src-tauri/src/skills/tools.rs          (新) list_skills / read_skill 多根实现（备选 core additive）
src-tauri/src/agent/run.rs             装配改用 catalog + L1 预算 + source 感知的 /技能名
src-tauri/src/agent/registry.rs        工具注册入参改为 catalog
src-tauri/src/skills/registry.rs       list_skills(全局) 保留，标注"仅 UI fallback"
src-tauri/src/lib.rs                   新命令注册
src/src/pages/market/MarketPage.tsx    按来源分组 + 源开关 + 冲突徽标
src/src/pages/agent/AgentPage.tsx      / 菜单分组 + 搜索 + 作用域 chip
src/src/pages/settings/**              新增「技能来源」分区与图标
```
