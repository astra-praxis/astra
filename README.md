# 星野 (Astra)

**项目工作空间管理工具 + AI 对话助手**

星野是一个桌面应用，帮助你管理工作空间中的多个项目，并通过 AI 对话助手提升开发效率。支持调用远程 API（OpenAI 兼容）或本地模型（llama.cpp）进行对话，AI 可以读取项目上下文、在编辑器中打开项目，甚至可以执行 shell 命令。

---

## 技术栈

| 层 | 技术 |
|----|------|
| 前端 | **SvelteKit 5** (SPA, runes) + **TypeScript** |
| 后端 | **Rust** + **Tauri 2** |
| 包管理 | **pnpm** |
| AI 协议 | OpenAI 兼容接口（流式 SSE） |

---

## 目录结构

```
astra/
├── src/                              # 前端源码
│   ├── lib/
│   │   ├── theme.svelte.ts           # 主题管理（亮/暗 + 渐变背景）
│   │   └── workspace.svelte.ts       # 全局工作空间状态
│   ├── routes/
│   │   ├── +layout.svelte            # 根布局：侧边栏导航 + 可拖拽标题栏
│   │   ├── +layout.ts                # SPA 配置（ssr = false）
│   │   ├── +page.svelte              # 工作空间页面（/）
│   │   ├── chat/
│   │   │   ├── +page.svelte          # 对话页面（/chat）
│   │   │   └── MarkdownMessage.svelte# Markdown 渲染 + 代码块执行 + Action 链接
│   │   ├── models/                   # 本地模型管理（/models）
│   │   ├── providers/                # API Provider 配置（/providers）
│   │   ├── settings/                 # 设置页面（/settings）
│   │   └── skills/                   # Skills 管理（/skills）
│   ├── styles/
│   │   └── global.css                # 全局 CSS 变量 + 样式
│   └── app.html
├── src-tauri/                        # Rust 后端
│   ├── src/
│   │   ├── main.rs                   # 入口
│   │   ├── lib.rs                    # 主库：工作空间、项目、编辑器、模型服务、Skills
│   │   ├── chat.rs                   # AI 对话（流式 SSE）
│   │   ├── chat_sessions.rs          # 对话会话持久化
│   │   ├── command_runner.rs         # Shell 命令执行（流式输出）
│   │   └── providers.rs              # API Provider CRUD
│   ├── capabilities/                 # Tauri 权限配置
│   ├── tauri.conf.json
│   └── Cargo.toml
└── package.json
```

---

## 功能

### 🧠 AI 对话 (chat)

- 支持 **API Provider**（OpenAI 兼容接口）和 **本地模型**（llama.cpp）
- 流式对话，实时展示内容与推理过程
- 会话历史管理（保存 / 加载 / 删除）
- **@ 项目提及** — 输入 `@` 触发项目搜索，AI 可感知项目上下文
- **Action 链接** — AI 可输出可点击的操作按钮
- **代码块执行** — shell/bash 代码块可直接执行

### 🔗 Action 链接

AI 在对话中可生成 action 链接，自动触发操作：

| Action | 说明 |
|--------|------|
| `action://open_project?path=xxx` | 在编辑器中打开项目（新消息自动执行） |
| `action://run_command?cmd=xxx&cwd=xxx` | 执行 shell 命令（需用户确认） |

新消息中的 `open_project` 链接会自动执行，无需手动点击。历史消息不会重复执行。

### 💻 Shell 命令执行

AI 输出的 bash/sh/shell 代码块支持一键执行：

1. AI 输出代码块 → 右上角出现 **▶ 执行** 和 **📋 复制** 按钮
2. 点击 **▶ 执行** → 展开确认弹窗，显示完整命令
3. 点击 **允许执行** → 命令运行，输出实时显示在代码块下方
4. 完成后显示退出码和结果

> 仅 `bash`、`sh`、`shell` 语言标签的代码块有执行按钮，普通代码块只有复制功能。

### 🗂️ 工作空间管理 (workspace)

- 添加 / 切换工作空间目录
- 自动扫描项目（检测 README.md）
- 查看项目详情（git remote、子项目、README 预览）
- 绑定默认编辑器，一键打开项目
- 新建项目目录

### ⚙️ 设置 (settings)

- 工作空间管理（添加 / 移除 / 切换）
- 默认编辑器配置（预置 VS Code、Cursor、Windsurf 等）
- 自定义编辑器命令
- 主题切换

### 🔧 模型 & Provider 管理

- **API Provider**：添加 OpenAI 兼容接口，自动拉取模型列表
- **本地模型**：配置 `.gguf` 模型文件，启动 / 停止 llama.cpp 服务器

---

## 开发

```bash
pnpm dev              # Vite 开发服务器
pnpm tauri dev        # 完整 Tauri 应用（含 Rust 热重载）
pnpm build            # 前端构建
pnpm tauri build      # 打包 macOS app
pnpm check            # svelte-check 类型检查
pnpm check:watch      # 监听式类型检查
```

---

## 系统提示注入

AI 的系统提示中会动态注入工作空间上下文（`buildWorkspaceContext()`），包括：

- 当前工作空间名称和路径
- 项目列表（名称 + 路径）
- Action 链接协议说明
- Shell 代码块执行说明

这使得 AI 能感知项目上下文并生成可执行的操作。

---

## 依赖

- **Tauri 2** — 桌面应用框架
- **SvelteKit** — 前端框架（SPA 模式）
- **marked** + **highlight.js** — Markdown 渲染 + 代码高亮
- **reqwest** — Rust HTTP 客户端（AI 接口调用）
- **serde** / **serde_json** — 序列化
- **tokio** — 异步运行时（命令执行）
