# 提示词与模型对话纯文本清单（PROMPTS）

记录 astra 中所有发往模型的系统提示词与纯文本限制，便于统一审查和修改。
改完任何一处后需重新编译对应 crate 才会生效。

## 宿主侧（astra/src-tauri）

所有提示词模板集中在 `src-tauri/prompts/`，编译期内嵌（`include_str!`）；
应用名占位符 `{app_name}` 由 `src-tauri/src/prompts.rs` 的 `APP_NAME` 常量填充（当前值 `astra`）。

| 文本 | 模板 / 位置 | 用途 |
|---|---|---|
| Agent 默认系统提示词 | `src-tauri/prompts/agent_system_prompt.md` | 每轮 Run 的 system 消息。工具使用规则、回答语言（中文）、简洁性等纯文本限制都在这里 |
| 技能库列表节 | `src-tauri/prompts/agent_skills_section.md` | 有可用技能时拼接到系统提示词末尾；技能条目 `- 名称: 描述` 在 `agent.rs`（agent_start_run）生成 |

补充说明：

- 用户在 Agent 页 Settings 手动填写的「系统提示」会**整体覆盖**默认提示词（`AgentStartReq.system_prompt`）
- 运行时变量：`max_turns` / `auto_compact_tokens` / 温度等是结构化配置，不是提示词文本，见 `agent.rs`

## agent-core 侧（astra-core 仓库，实际构建引用 `path = "../../astra-core"`）

core 自有面向模型/用户的文案全部收口在 `TextPack`（`crates/astra-local/src/`，默认中文 `zh::pack()`，宿主可经 `RunConfig`/`ThreadConfig.texts` 注入覆盖；另有英文 `en::pack()`）：

| 文本 | 位置 | 流向 |
|---|---|---|
| 上下文压缩提示词（结构化交接摘要要求） | `crates/astra-local/src/zh.rs` `compaction_prompt` | 作为 User 消息发给模型生成摘要（`run.rs` `compact_context`） |
| 工具错误 / 参数校验 / 路径逃逸 / 超限等文案 | `crates/astra-local/src/{zh,en}.rs` 其余字段 | 作为工具结果回传模型，同时用于前端展示 |
| 压缩摘要回放前缀 | `crates/astra-model/src/model.rs` `Role::compaction_prefixed` | 历史摘要发回模型时的前缀（当前为英文硬编码） |
| 工具 description（schema 内） | 各 `*_tools.rs` | 随工具定义发给模型，影响模型选工具 |

注意：仓库内 `astra/agent-core/` 是旧副本，构建不使用；改文案请去 `../astra-core`。

## 如何修改

- **改 Agent 系统提示词 / 技能节**：编辑 `src-tauri/prompts/*.md` 模板，重新编译
- **改应用名**：改 `src-tauri/src/prompts.rs` 的 `APP_NAME` 一处即可
- **改 core 文案**：宿主注入自定义 `TextPack`（推荐，零侵入），或直接改 `astra-core` 的 `zh.rs`/`en.rs`
