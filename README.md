
# Astra

Astra 是一个基于 Tauri 2 的本地优先 AI Agent 桌面应用。选择一个工作目录，就可以通过对话让 Agent 阅读和修改文件、执行命令、搜索网页，以及生成文档和图片。

## 功能

- **Agent 对话**：通过模型与工具协作完成多步骤任务。
- **文件与工作区**：在指定工作目录中读取、编辑和管理文件。
- **内置工具**：提供文件操作、命令执行、搜索等工具。
- **MCP / ACP**：支持通过扩展协议接入外部工具和 Agent。
- **Skills**：通过技能定义扩展 Agent 的任务处理能力。
- **长期记忆**：使用本地加密存储保留跨会话的相关信息。
- **多模型支持**：支持 OpenAI 兼容 API 和本地模型服务。
- **Artifact**：集中展示任务生成的文档及其他结果。
- **桌面集成**：基于 Tauri 2，提供桌面应用体验。

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

## 获取应用

前往 [GitHub Releases](https://github.com/astra-praxis/astra/releases) 查看已发布的版本，并下载对应平台的安装包。

请根据 Release 页面提供的文件选择适合自己系统的版本。暂未提供对应安装包的平台可能无法直接使用。

## 项目状态

Astra 仍在持续开发中，功能和界面可能随版本调整。

## License

MIT
