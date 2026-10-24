/**
 * 前端错误上报：fire-and-forget 调 Rust 端 frontend_log 命令，
 * 落盘到 %APPDATA%\com.astra.app\astra.log（feature=frontend）。
 *
 * 注意：agent.ts 不得 import 本文件（vm mock 只认 4 个模块）。
 */

export function reportError(feature: string, message: string): void {
  try {
    void import('@tauri-apps/api/core')
      .then(({ invoke }) =>
        invoke('frontend_log', {
          level: 'error',
          feature,
          message: message.slice(0, 2000),
        }),
      )
      .catch(() => {
        // Tauri 未就绪 / 命令失败：静默，日志上报绝不能二次报错
      })
  } catch {
    // ignore
  }
}

export function reportInfo(feature: string, message: string): void {
  try {
    void import('@tauri-apps/api/core')
      .then(({ invoke }) =>
        invoke('frontend_log', {
          level: 'info',
          feature,
          message: message.slice(0, 2000),
        }),
      )
      .catch(() => {
        // ignore
      })
  } catch {
    // ignore
  }
}
