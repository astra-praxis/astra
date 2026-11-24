# R4 手机 App：技术选型（设计冻结待确认）

> 前置：R1 设备密钥（v0.6.27）+ R3 公网中继（v0.6.28/29）已完成。
> 手机端现状：桥自带单页 HTML（md 渲染 / 队列 / scope / sender 徽标 / 票据认证 /
> 限速），浏览器扫码即用，凭证存 localStorage。
> R4 目标：把「浏览器标签页」原生化——图标、独立窗口、常驻凭证、系统级扫码入口。

## 一、核心事实（选型的地基）

1. **手机端是纯客户端**：所有对话/Agent 能力都在桌面端桥后面，手机只需要
   HTTP/WS + 凭证存储 + 页面渲染。不跑 agent、不跑 MCP、不碰 SQLite。
2. **全部业务逻辑已在桥的 HTML 里**：桌面端 `mobile_bridge_mobile.html`
   （include_str! 内嵌）承载了 R0-R3 的全部成果。App 不需要重写任何对话 UI。
3. **iOS 平台规则**：第三方 App 禁止 fork/exec 子进程 → `astra-process`
   （MCP server 拉起）在 iOS 上无法工作。
4. **桌面 crate 体量**：src-tauri 拖着 agent-core、astra-memory(sqlite)、
   rusqlite bundled、image、RAG 全家桶——对手机客户端全是死重。

## 二、候选方案对比

| | A. 原生薄壳 + WebView | B. Tauri v2 mobile（复用桌面 crate） | C. Tauri v2 mobile（新建超薄 crate） | D. PWA（不装 App） | E. RN/Flutter 重写 |
|---|---|---|---|---|---|
| 原生代码量 | iOS ~300 行 Swift + Android ~300 行 Kotlin | 大量 cfg(mobile) 砍功能 | 中（Tauri 工程 + 插件） | 0 | 全套 UI 重写 |
| Rust 交叉编译 | 不需要 | 需要（agent-core 全家桶上机） | 需要（但 crate 很小） | 不需要 | 不需要 |
| 复用既有对话 UI | ✅ 100%（WebView 加载桥页面） | ⚠️ 桌面前端跑不动，仍要套桥 HTML | ✅ | ✅ | ❌ 全重写 |
| iOS 子进程禁令 | 无冲突 | ❌ 硬伤（astra-process 无法工作） | 无冲突（不带 process） | 无冲突 | 无冲突 |
| 凭证安全 | Keychain / Keystore（加密硬件级） | localStorage | 插件级（keychain-vault） | localStorage（现状） | AsyncStorage |
| 独立图标/窗口 | ✅ | ✅ | ✅ | ⚠️ 添加到主屏幕（伪 App） | ✅ |
| 分发 | TestFlight / APK | TestFlight / APK | TestFlight / APK | 即刻可用 | TestFlight / APK |
| 维护面 | 双平台各一薄层（逻辑零重复） | 最重：双端 profile + 砍功能同步 | 中：多一套 Tauri 工程 | 零 | 最重 |

### 逐案结论

- **B（复用桌面 crate）：淘汰。** 手机不需要本地 agent；iOS 子进程禁令是平台级
  硬伤；把 agent-core/sqlite/RAG 编进手机 = 纯死重，还要长期维护 cfg(mobile) 门。
  Tauri mobile 的适用前提是「同一套前后端跑两端」，本场景不成立。
- **C（超薄 Tauri crate）：淘汰。** 砍到只剩 WebView + keychain 后，Tauri 相对
  原生壳没有增量价值，反而多一套移动端 Rust 工具链（targets、模拟器、签名）。
- **E（RN/Flutter）：淘汰。** 重写全部对话 UI，违背「逻辑在桥、壳只管装载」的
  架构事实；md 渲染/队列/scope 全部二次实现、二次维护。
- **A（原生薄壳）：✅ 推荐。** 与架构事实完全对齐——原生层只做三件事：
  扫码、存凭证、装 WebView。逻辑零重复，双端各 ~300 行。
- **D（PWA）：✅ 作为 R4.0 先行。** 半天工作量，给手机页加 manifest + meta，
  「添加到主屏幕」即刻获得伪 App（全屏、独立图标）。原生壳做完后 D 仍保留，
  作为不装 App 用户的兜底。

## 三、决策（待确认后冻结）

**R4.0 PWA 化**（先做，零工具链依赖）+ **R4.1 原生薄壳**（A 方案）。

### R4.0 — PWA 化手机页

- 手机 HTML 增加 `manifest`（动态生成或内嵌 data: URL）：名称「Astra 遥控」、
  图标（内嵌 SVG→PNG data URL）、`display: standalone`、主题色随暗色模式
- iOS meta：`apple-mobile-web-app-capable` / `apple-mobile-web-app-status-bar-style`
- 凭证已是 localStorage，PWA 安装后天然持久
- 改动面：只动 `mobile_bridge_mobile.html`，`cargo build` 后生效

### R4.1 — 原生薄壳（新目录 `apps/mobile/`）

```
apps/mobile/
├── ios/AstraRemote/          # SwiftUI + WKWebView
│   ├── App.swift             # 入口 + 深链 astra://bind?url=...
│   ├── ScannerView.swift     # AVFoundation 扫码（识别桥 URL）
│   ├── Keychain.swift        # key 存 Keychain（设备唯一绑定）
│   └── WebScreen.swift       # WKWebView 加载桥 URL（断连/失效态提示）
├── android/                  # Kotlin + WebView（单 Activity）
│   ├── ScannerFragment       # CameraX + ML Kit 条码
│   ├── SecureStore           # EncryptedSharedPreferences（Keystore）
│   └── MainActivity          # WebView + 深链 intent-filter
└── README.md
```

关键行为（双端一致）：
1. **首次启动**：内置扫码 → 得桥 URL（含 #secret fragment）→ WebView 加载
2. **凭证**：从 URL fragment 提取 secret 后，localStorage 照旧 + **同步存
   Keychain/Keystore**；下次启动优先用本地凭证直连（不需要重扫）
3. **失效处理**：桥返回 401/门页时清凭证（既有逻辑），App 回扫码页
4. **多设备/多桌面**：凭证按「桌面端 host:port」维度存多条，可切换/删除
5. **中继兼容**：URL 可能是局域网或 relay 入口，壳不关心，照 URL 加载即可

### 分发与环境依赖（实现前需确认）

| 项 | iOS | Android |
|---|---|---|
| 工具链 | Xcode 16+（macOS） | Android Studio |
| 真机调试 | 免费 Apple ID（7 天重签） | 直接装 APK |
| 正式分发 | Apple Developer $99/年（TestFlight） | APK / 应用市场 |
| Rust | 不需要 | 不需要 |

## 四、里程碑

| # | 里程碑 | 依赖 | 状态 |
|---|---|---|---|
| R4.0 | PWA 化（manifest + iOS meta + 图标） | 无，立即可做 | 待做 |
| R4.1-scaffold | 独立开源客户端仓库 `~/workplace/astra-mobile`（iOS 壳脚手架 + 绑定协议文档） | 无 | ✅ 已建（dac0169） |
| R4.1-embed | 手机页嵌入模式：fragment `embed=1`（隐藏页面自带顶栏/输入区/抽屉/队列，只留消息流）+ `open=<id\|new>`（首连指定会话） | 无 | ✅ v0.6.30 |
| R4.1-chrome | 壳原生 chrome：顶栏/左抽屉（搜索+历史+用户入口）/输入条/队列面板/配置 sheet，壳自持 WS | R4.1-embed | ✅ 原生 chrome 完成（23def77），CI 流水线（b38c118） |
| R4.1a | Android 壳（扫码/Keystore/WebView/深链） | Android Studio | 占位 |
| R4.1b | iOS 壳完善（本机无 Xcode，编译验证待环境） | Xcode；分发需开发者账号 | 待验证 |
| R4.2 | 打磨：深链 astra://、多桌面凭证管理、启动直连 | R4.1 | 待做 |

补充决策（2026-11-24 用户确认）：
- 壳形态升级：不再是纯全屏 WebView，改为「原生 chrome + WebView 只渲染对话记录」；
  队列也原生化；抽屉底部保留「用户」入口（内容暂为设置 + 绑定信息，账号体系后议）
- iOS 先行，Android 占位
- 客户端仓库与桌面端仓库分离（开源边界），契约由仓库内 `docs/BINDING_PROTOCOL.md` 维护
