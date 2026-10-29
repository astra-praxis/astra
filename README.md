
# Astra

Astra 是一个基于 Tauri 2 的**本地优先 AI Agent 桌面应用**。选择一个工作目录，通过对话让 Agent 阅读和修改文件、执行命令、搜索网页、生成文档与图片。数据与密钥全部留在本机。

## 🚀 快速体验

**无需编译，直接下载安装包体验**（macOS / Windows / Linux 三端）：

👉 **[GitHub Releases](https://github.com/astra-praxis/astra/releases)**

选择对应平台安装包下载安装，进入「设置」配置一个 OpenAI 兼容的模型服务即可开始对话。

## 功能特性

- **Agent 对话**：多步骤任务协作，多会话标签页与历史导航
- **文件与工作区**：在指定工作目录中读取、编辑、管理文件
- **工具与扩展**：内置工具（文件、命令、搜索）+ 扩展协议接入外部工具与 Agent + 技能扩展
- **长期记忆**：跨会话自动沉淀，本地加密存储
- **多模型支持**：OpenAI 兼容 API 与本地模型服务
- **Artifact**：集中展示任务生成的文档与结果
- **桌面集成**：Tauri 2 原生桌面体验，多窗口 / 单实例

## 核心亮点

### 多会话并行体验

多标签页随时切换，运行中的会话可驻留后台持续工作；会话历史全局平铺、时间分桶，一键恢复。

### 安全优先

Agent 的每次工具调用都经过独立授权，支持多级放权策略；越权操作被硬性拦截。工具输出按不可信数据处理，不因内容获得任何额外权限，高风险操作始终重新经过授权链。记忆数据端到端加密，密钥不离开本机。

### 长期记忆

对话自动沉淀为可复用的长期记忆，跨会话召回；错误记忆可被纠正，旧结论自动失效。

### 三端自动发布

GitHub Actions 一键构建并发布 macOS / Windows / Linux 安装包，版本化 Release 持续更新。

## 技术栈

| 模块 | 技术 |
|---|---|
| 桌面应用 | Tauri 2 |
| 后端 | Rust |
| 前端 | React 19、TypeScript |
| 状态管理 | Zustand |
| UI | Radix UI |
| 构建工具 | Vite、pnpm |
| 扩展协议 | MCP、ACP |
| 本地存储 | SQLite |

## 功能完成度

### ✅ 已完成

**Agent 核心（astra-core）**

- **Agent Loop**：多步任务执行循环，流式响应、工具调用、错误重试（上限 4 次）、用户取消
- **权限引擎**：Policy 分级（只读放行 / 确认 / 拒绝）+ 授权链硬拦截；工具输出按不可信数据处理
- **工具运行时**：内置工具（文件 / 命令 / 搜索）+ ToolManifest 元数据（category / risk）
- **预算策略**：单次 Run 的轮次 / 运行时长 / 工具调用预算（内置默认上限，防失控循环）
- **上下文压缩**：长对话自动 compaction，撞上下文窗口前自动收敛

**会话与持久化**

- **多会话**：多标签页并行，历史会话恢复、全局平铺与时间分桶
- **SQLite 持久化**：ThreadStore trait + astra-store-sqlite（Memory / SQLite 同套契约测试）
- **Run 内增量落盘**：对话过程逐条写穿，进程意外退出最多丢最后一段流式文本
- **Checkpoint 恢复边界**：对话历史锚点 + workspace git 快照，可恢复到任意检查点
- **错误恢复 UX**：Run 失败后仅重试，不回档；完整对话记录始终保留

**子代理（Subagent v1.6）**

- `delegate_task` 委托工具：主 Agent 拆解任务给子 Run 执行
- **有界并发**：Semaphore 槽位排队（桌面端上限 8），父 Run 取消 / 超时真实中断排队任务
- **活动可见性**：子任务执行过程实时展示（工具调用 / 阶段），经 `parent_tool_call_id` 精确关联卡片
- **权限继承**：子 Run 有效能力 ⊆ 父 Run（结构性禁递归 + 负向测试）

**生态与扩展**

- **MCP**：本地 stdio / 远程 Server 接入（rmcp），配置驱动的权限规则推导
- **ACP**：外部 Agent 接入（claude-code-acp 等），权限三档 + 会话池生命周期
- **Skills**：技能包指导与工作流，`skill.yaml` 依赖声明
- **Artifact**：任务产出的文件注册、版本链、集中展示

**产品与工程**

- **配置系统**：`~/.astra/config.toml`，Provider / 存储 / 权限 / MCP / ACP 分段
- **错误与可观测**：flat 错误枚举 + tracing 日志 + Execution Record
- **长期记忆**：跨会话沉淀与召回，本地加密存储，支持纠正与失效
- **多模型**：OpenAI 兼容 API 与本地模型服务
- **桌面集成**：Tauri 2 多窗口 / 单实例 / 托盘 / 全局热键
- **三端自动发布**：GitHub Actions 构建 macOS / Windows / Linux 安装包

### ⬜ 未完成 / 触发式规划

**子代理（Subagent）**

- ⬜ 子事件流前端内嵌：子任务活动为快照轮询，非实时流式转发（Phase D/E）
- ⬜ 子 Run 持久化：子 Run 对话 ephemeral，不落库
- ⬜ 子代理模型分级选择：恒为父模型实例，未支持注入不同 backend
- ⬜ astra-serve 子代理接入：仅桌面端接委托

**核心运行时（astra-core）**

- ⬜ ThreadQuery 扩展：标签 / 时间范围过滤（出现真实消费方再冻结）
- ⬜ Memory Provider 实现（core 侧）：trait 已留口，宿主已接 astra-memory，core 零实现

**生态后续**

- ⬜ MCP Streamable HTTP / 非文本结果：当前仅 stdio + 文本
- ⬜ ACP Workspace Bridge / AgentProvider：协议层与工具适配已完成，能力外供未做
- ⬜ Artifact 内容存储 / 回滚：只存 metadata，内容恢复不做
- ⬜ Scheduler / Multi-Agent 框架：明确不做（架构纪律），出现 Agent 并行执行需求再评估

> 未完成项均为「触发式」：有真实消费方或需求再冻结设计，避免过度建设。

## 项目状态

持续开发中，功能和界面随版本调整。欢迎下载体验并反馈问题。

## License

MIT
