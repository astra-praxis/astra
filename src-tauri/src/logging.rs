//! 功能点落盘日志：JSONL 追加写 `app_data_dir/astra.log`。
//!
//! Windows GUI 子系统无控制台，stderr 输出不可见，故把关键功能点的
//! 成功/失败事件写到应用数据目录，方便用户直接反馈该文件定位问题。
//!
//! - `init(dir)` 在 setup 阶段调用；文件超过 5MB 轮转为 astra.log.old
//! - `log` / `app_log!`：未初始化或写入失败时静默丢弃，绝不影响功能
//! - `frontend_log` 命令：前端错误上报通道（全局 error / 页面 catch）

use std::fs::OpenOptions;
use std::io::Write;
use std::path::PathBuf;
use std::sync::Mutex;

static LOG_PATH: Mutex<Option<PathBuf>> = Mutex::new(None);
const MAX_LOG_SIZE: u64 = 5 * 1024 * 1024;

/// setup 阶段初始化日志文件路径；超限时轮转旧文件。
pub(crate) fn init(dir: &std::path::Path) {
    let path = dir.join("astra.log");
    if let Ok(meta) = std::fs::metadata(&path) {
        if meta.len() > MAX_LOG_SIZE {
            let _ = std::fs::rename(&path, dir.join("astra.log.old"));
        }
    }
    if let Ok(mut guard) = LOG_PATH.lock() {
        *guard = Some(path);
    }
}

/// RFC3339 风格本地时间戳（含毫秒与 ±HH:MM 偏移）。依赖 chrono Local。
fn timestamp() -> String {
    chrono::Local::now()
        .format("%Y-%m-%dT%H:%M:%S%.3f%:z")
        .to_string()
}

/// 追加一条 JSONL 日志。任何失败静默丢弃。
pub(crate) fn log(level: &str, feature: &str, message: &str) {
    let path = match LOG_PATH.lock().ok().and_then(|g| g.clone()) {
        Some(p) => p,
        None => return,
    };
    let ts = timestamp();
    // 简易 JSON 字符串转义（引号、反斜杠、控制字符）
    let esc = json_escape(message);
    let line = format!(
        "{{\"ts\":\"{ts}\",\"level\":\"{level}\",\"feature\":\"{feature}\",\"msg\":\"{esc}\"}}\n"
    );
    if let Ok(mut f) = OpenOptions::new().create(true).append(true).open(&path) {
        let _ = f.write_all(line.as_bytes());
    }
}

fn json_escape(s: &str) -> String {
    let mut out = String::with_capacity(s.len());
    for c in s.chars() {
        match c {
            '"' => out.push_str("\\\""),
            '\\' => out.push_str("\\\\"),
            '\n' => out.push_str("\\n"),
            '\r' => out.push_str("\\r"),
            '\t' => out.push_str("\\t"),
            c if (c as u32) < 0x20 => out.push_str(&format!("\\u{:04x}", c as u32)),
            c => out.push(c),
        }
    }
    out
}

/// `app_log!(info, "workspace", "scan ok: {} projects", n)`
#[macro_export]
macro_rules! app_log {
    ($level:ident, $feature:expr, $($arg:tt)+) => {
        $crate::logging::log(stringify!($level), $feature, &format!($($arg)+))
    };
}

/// 前端错误上报通道：全局 error handler / 页面 catch 调用。
#[tauri::command]
pub(crate) fn frontend_log(level: String, feature: String, message: String) {
    log(&level, &feature, &message);
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn timestamp_format_is_rfc3339_like() {
        let ts = timestamp();
        // 形如 2026-10-24T12:34:56.789+08:00
        assert_eq!(ts.len(), 29, "unexpected ts: {}", ts);
        assert_eq!(&ts[4..5], "-");
        assert_eq!(&ts[10..11], "T");
    }

    #[test]
    fn json_escape_handles_specials() {
        assert_eq!(json_escape("a\"b\\c\nd"), "a\\\"b\\\\c\\nd");
    }

    #[test]
    fn log_before_init_is_silent() {
        // 未 init 时调用不应 panic（路径可能被其他测试占用，静默即可）
        log("info", "test", "should not panic");
    }
}
