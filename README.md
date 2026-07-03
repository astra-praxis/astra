```
astra/
├── src/                    # SvelteKit 前端源码
│   ├── lib/               # 共享组件和工具
│   │   └── theme.svelte.ts # 主题状态管理
│   ├── routes/            # 页面路由
│   │   ├── workspace/     # 工作空间页面
│   │   ├── settings/      # 设置页面
│   │   └── skills/        # 技能页面
│   └── app.html           # HTML 模板（含防闪烁脚本）
├── src-tauri/             # Tauri/Rust 后端
│   ├── src/               # Rust 源码
│   │   └── lib.rs         # 主要后端逻辑
│   └── tauri.conf.json    # Tauri 配置
└── package.json           # 前端依赖
```
