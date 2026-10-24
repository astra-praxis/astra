//! Windows 控制台闪窗抑制。
//!
//! GUI 子系统的宿主进程（Tauri 桌面）spawn 控制台子系统程序（`cmd`、
//! `git`、`node`、`npx`、`.cmd` 批处理入口…）时，Windows 默认为子进程
//! 新建控制台窗口，表现为对话框闪一下黑框。加 `CREATE_NO_WINDOW` 创建
//! 标志即可抑制；pipe 重定向与输出采集不受影响。
//!
//! 例外：需要展示 GUI 的启动（编辑器、浏览器）其子进程本身是 GUI 子系统
//! 不弹窗，但 `.cmd` 包装层（code.cmd）会先弹 cmd 黑框，统一抑制。

/// 给 `std::process::Command` 加 Windows `CREATE_NO_WINDOW`；其他平台无操作。
pub(crate) fn hide_window(cmd: &mut std::process::Command) {
    #[cfg(windows)]
    {
        const CREATE_NO_WINDOW: u32 = 0x0800_0000;
        use std::os::windows::process::CommandExt;
        cmd.creation_flags(CREATE_NO_WINDOW);
    }
    #[cfg(not(windows))]
    {
        let _ = cmd;
    }
}
