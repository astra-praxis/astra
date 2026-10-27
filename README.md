
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

## 架构总览

Astra · 知行 —— 本地优先的 AI Agent 桌面应用，核心运行时由 Rust 承载，支持远程模型 API 与本地模型服务。

```mermaid
graph TB
    subgraph FE["产品交互层 · Frontend"]
        FE1["Agent 对话"]
        FE2["工作空间"]
        FE3["模型与配置"]
        FE4["能力与扩展"]
    end

    IPC["Tauri IPC · Commands / Events / DTO"]

    subgraph HOST["应用宿主层 · Tauri + Rust"]
        H1["应用生命周期 · 启动 / 退出 / 窗口"]
        H2["应用命令桥接 · 输入 / 事件 / 结果"]
        H3["配置与凭证 · Provider / OS 密钥库"]
        H4["运行时装配"]
    end

    API["Rust API · 配置注入 · 运行事件"]

    subgraph CORE["Agent Runtime · astra-core · 独立于 UI 的 Agent 执行核心"]
        C1["Run 生命周期 · 执行 / 取消 / 超时"]
        C2["模型交互 · 请求 / 流式响应"]
        C3["权限控制 · 授权 / 审批 / 拦截"]
        C4["工具执行循环 · 调用 / 结果 / 继续"]
    end

    DEP["依赖：配置 / Provider / 工具注册表 / Skill Registry / 权限与事件协议"]

    subgraph MS["模型服务"]
        M1["远程 API"]
        M2["OpenAI 兼容接口"]
        M3["Anthropic 等协议适配"]
        M4["本地模型端点"]
    end

    subgraph TC["工具与能力"]
        T1["内置工具"]
        T2["本地进程 / stdio MCP"]
        T3["远程 MCP"]
        T4["Skills 指导与工作流"]
    end

    subgraph LD["本地数据"]
        D1["对话与运行记录"]
        D2["配置与状态"]
        D3["加密长期记忆"]
        D4["工作空间文件"]
    end

    subgraph OS["操作系统能力"]
        O1["文件系统"]
        O2["进程与命令执行"]
        O3["系统密钥库"]
        O4["权限与资源边界"]
    end

    FE --> IPC
    IPC --> HOST
    HOST --> API
    API --> CORE
    CORE --> DEP
    DEP --> MS
    DEP --> TC
    DEP --> LD
    DEP --> OS
```

> 注：此图为逻辑架构总览，不代表每个模块均已完整接线；数据存储与记忆的具体调用关系以当前实现为准。

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

## 项目状态

持续开发中，功能和界面随版本调整。欢迎下载体验并反馈问题。

## License

MIT
