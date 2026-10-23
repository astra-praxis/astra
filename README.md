# Astra

**本地优先的 AI Agent 桌面应用** — 项目工作空间管理 + 全能 Agent 对话助手

Astra 是一个基于 Tauri 2 的桌面应用：把你的项目目录交给一个具备 **23+ 内置工具、MCP 扩展、技能系统、长期记忆** 的 AI Agent，它能读写文件、执行命令、检索网页、生成图片与文档——所有操作都在你本机完成，权限边界清晰可控。

> Agent Runtime 基于 [astra-core](https://github.com/astra-praxis/astra-core)（Rust 实现的可嵌入 Agent Runtime SDK）与 [astra-memory](https://github.com/astra-praxis/astra-memory)（认知记忆引擎）构建。

---

## ✨ 功能亮点

### 🤖 Agent 对话

- **流式对话**：实时展示回答、思考过程（Reasoning）与工具调用进度
- **23+ 内置工具**：文件读写（`read_file` / `edit_file` / `find_path`…）、Shell 执行（前台 + 后台任务）、网页抓取（`fetch` / `search_web`）、图片生成（`generate_image`）、文档转换（`extract_document` / `build_docx`）等
- **Artifact 系统**：Agent 产出的文件/代码/图片以卡片形式沉淀，支持预览（Markdown / 代码高亮 / 图片 / 视频渲染）
- **多会话并行**：多标签页管理多个 Agent 会话，互不干扰
- **多模态输入**：粘贴图片直接进对话
- **@ 项目提及**：Agent 感知当前项目上下文
- **斜杠技能**：`/技能名` 调用已安装的 Agent 技能
- **Checkpoint 回退**：会话支持历史回退与编辑重发

### 🔐 权限与安全

- 每个工具调用都有**权限等级**：只读操作自动放行，写入/执行类操作弹出确认卡
- 工具级 `allow` / `deny` 名单，MCP 工具按需放行
- 敏感操作（覆盖文件、执行命令）完整展示 diff 或命令内容后才执行

### 🧩 MCP / ACP 扩展

- 兼容 **Model Context Protocol**：在 Agent 对话中说「帮我安装 xxx MCP」，AI 自动下载依赖并写好配置
- 支持 ACP Agent 扩展
- 启停 / 删除 / 运行状态监控，工具数量实时显示

### 🧠 认知记忆（CME）

- Run 结束自动提炼长期记忆（确定性规则 + 词法检索，无模型依赖）
- 加密 SQLite 存储，密钥落在本机
- 跨会话召回：新对话自动注入「REMEMBERED CONTEXT」，Agent 记得你的偏好与项目背景
- 记忆按项目作用域隔离，不串线

### 🗂️ 工作空间

- 多工作空间管理，自动扫描项目
- 项目详情：git remote、子项目、README 预览
- 一键在编辑器中打开（VS Code / Zed / Cursor / 自定义命令）

### 🎨 其他

- **亮 / 暗主题**，完整的设计变量体系
- **系统托盘**：关闭窗口最小化到托盘，`Alt+Space` 全局唤起
- **本地模型**：接入 llama.cpp 服务器运行 `.gguf` 模型；也支持任意 OpenAI 兼容 API

---

## 技术栈

| 层 | 技术 |
|----|------|
| 桌面框架 | **Tauri 2**（Rust） |
| 前端 | **React 19** + TypeScript + Zustand + Radix UI |
| Agent Runtime | [astra-core](https://github.com/astra-praxis/astra-core) — 自研 Rust Agent Runtime（工具系统 / 权限策略 / 事件协议 / Artifact） |
| 记忆引擎 | [astra-memory](https://github.com/astra-praxis/astra-memory) — CME 认知记忆（加密 SQLite） |
| 协议 | MCP（Model Context Protocol）/ ACP / OpenAI 兼容 API |
| 构建 | pnpm + Vite + GitHub Actions（macOS / Windows / Linux 自动打包发布） |

---

## 目录结构

```
astra/
├── src/                          # 前端（React 19 + Vite）
│   └── src/
│       ├── pages/
│       │   ├── agent/            # Agent 对话页（多会话 / 权限卡 / 预览面板）
│       │   ├── artifacts/        # Artifact 产出管理
│       │   ├── market/           # MCP / ACP / 技能管理
│       │   ├── workspace/        # 工作空间与项目
│       │   ├── chat/             # 基础对话组件（Mention 输入等）
│       │   ├── providers/        # API Provider 配置
│       │   ├── models/           # 本地模型（llama.cpp）
│       │   ├── tools/            # 内置工具页
│       │   └── settings/         # 设置（编辑器 / 记忆 / 图像生成 / 搜索…）
│       ├── stores/               # Zustand 全局状态（agent 事件流等）
│       └── components/           # 通用组件
├── src-tauri/                    # Rust 后端（Tauri 2）
│   └── src/
│       ├── agent/                # Agent 桥接层（Run 启动 / 事件转发 / 记忆采集）
│       ├── memory/               # CME 宿主集成（加密库 + Provider 注入）
│       ├── skills/               # 技能库管理（内置技能安装）
│       └── app/                  # 工作空间 / 设置 / 下载器 / 编辑器启动
└── .github/workflows/            # CI：tag 推送自动出 macOS dmg + Windows exe + Linux AppImage/deb
```

---

## 开发

```bash
# 前置：Rust + pnpm；astra-core / astra-memory 按兄弟目录克隆到同级
git clone https://github.com/astra-praxis/astra.git
git clone https://github.com/astra-praxis/astra-core.git    # ../astra-core
git clone https://github.com/astra-praxis/astra-memory.git  # ../astra-memory

pnpm install            # 根目录（tauri CLI）
pnpm --dir src install  # 前端依赖

pnpm tauri dev          # 完整应用（前端 + Rust 热重载）
pnpm tauri build        # 打包
```

前端测试与检查：

```bash
pnpm --dir src typecheck              # TypeScript
pnpm --dir src test                   # node --test 单测（store 纯逻辑）
pnpm --dir src build                  # 前端构建
cd src-tauri && cargo check           # Rust
```

---

## 发布

推送 `v*` tag 即触发 [GitHub Actions](.github/workflows/build.yml) 构建并发布：

```bash
git tag v0.x.y && git push origin v0.x.y
```

产物：

| 平台 | 产物 | 说明 |
|---|---|---|
| Windows | `astra_<version>_x64-setup.exe` | NSIS 安装器，WebView2 在线下载 |
| Windows | `astra_<version>_x64-setup-offline.exe` | WebView2 已内置（离线机器用，体积 +~130MB） |
| macOS | `astra_<version>_aarch64.dmg` | Apple Silicon |
| Linux | `astra_<version>_amd64.AppImage` | 通用格式，Arch 等滚动发行版 `chmod +x` 直接运行 |
| Linux | `astra_<version>_amd64.deb` | Ubuntu / Debian |

> Linux 产物（AppImage / deb）自 v0.3.4 起提供，尚未在真机上充分测试，遇到问题欢迎提 issue。运行依赖 WebKitGTK 4.1（多数桌面发行版默认具备）；AppImage 内已自带大部分依赖。

> Windows 首次安装如遇 SmartScreen 提示，点「更多信息 → 仍要运行」。

---

## License

MIT
