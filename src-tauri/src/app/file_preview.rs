//! 文件预览：为 Agent 页文件预览面板读取本地文件内容。
//!
//! 按扩展名分类：图片 / 视频 / PDF 返回 data URL（前端 blob 化后内嵌渲染）；
//! Markdown / 代码 / 纯文本返回文本（前端分别用 md 渲染、代码块高亮、pre 直出）；
//! 其余（docx / xlsx / zip 等无法内嵌渲染的）返回 unsupported，由前端引导
//! 「使用默认应用打开」。未知扩展名按文本尝试解码，含 NUL 字节或非 UTF-8
//! 则归为 unsupported。
//!
//! 读取不做路径白名单——与 open_in_editor 同一语境：桌面应用允许打开
//! 用户（或其 Agent 会话）指定路径的本地文件。

use base64::Engine as _;
use serde::Serialize;
use std::path::Path;

/// 文本类读取上限（2 MiB）：超限直接报错，避免面板渲染卡死
const TEXT_MAX_BYTES: u64 = 2 * 1024 * 1024;
/// 二进制（图片 / 视频 / PDF）读取上限（64 MiB）
const MEDIA_MAX_BYTES: u64 = 64 * 1024 * 1024;

#[derive(Serialize)]
pub struct PreviewFile {
    /// text | markdown | code | image | video | pdf | unsupported
    kind: String,
    /// 文本内容（text / markdown / code）
    text: Option<String>,
    /// data:image/png;base64,... （image / video / pdf）
    data_url: Option<String>,
    /// 代码高亮语言提示（code 时给前端拼 ``` 围栏）
    lang: Option<String>,
    mime: String,
    size: u64,
}

fn unsupported(size: u64) -> PreviewFile {
    PreviewFile {
        kind: "unsupported".into(),
        text: None,
        data_url: None,
        lang: None,
        mime: "application/octet-stream".into(),
        size,
    }
}

/// 二进制类扩展名 → (kind, mime)
fn media_of(ext: &str) -> Option<(&'static str, &'static str)> {
    Some(match ext {
        "png" => ("image", "image/png"),
        "jpg" | "jpeg" => ("image", "image/jpeg"),
        "gif" => ("image", "image/gif"),
        "webp" => ("image", "image/webp"),
        "bmp" => ("image", "image/bmp"),
        "svg" => ("image", "image/svg+xml"),
        "mp4" => ("video", "video/mp4"),
        "webm" => ("video", "video/webm"),
        "mov" => ("video", "video/quicktime"),
        "m4v" => ("video", "video/x-m4v"),
        "mkv" => ("video", "video/x-matroska"),
        "pdf" => ("pdf", "application/pdf"),
        _ => return None,
    })
}

/// 代码类扩展名 → highlight.js 语言提示（rehype-highlight 按此高亮）
fn code_lang(ext: &str) -> Option<&'static str> {
    Some(match ext {
        "js" | "jsx" | "mjs" | "cjs" => "javascript",
        "ts" | "tsx" | "mts" => "typescript",
        "py" => "python",
        "rs" => "rust",
        "go" => "go",
        "java" => "java",
        "kt" => "kotlin",
        "swift" => "swift",
        "dart" => "dart",
        "c" | "h" => "c",
        "cpp" | "hpp" | "cc" => "cpp",
        "cs" => "csharp",
        "rb" => "ruby",
        "php" => "php",
        "lua" => "lua",
        "pl" => "perl",
        "r" => "r",
        "sh" | "bash" | "zsh" => "bash",
        "ps1" => "powershell",
        "bat" => "dos",
        "sql" => "sql",
        "css" => "css",
        "scss" => "scss",
        "json" => "json",
        "yaml" | "yml" => "yaml",
        "xml" => "xml",
        "toml" => "toml",
        _ => return None,
    })
}

fn fmt_size(size: u64) -> String {
    if size >= 1024 * 1024 {
        format!("{:.1} MB", size as f64 / (1024.0 * 1024.0))
    } else if size >= 1024 {
        format!("{:.1} KB", size as f64 / 1024.0)
    } else {
        format!("{size} B")
    }
}

/// 检查路径在磁盘上是否存在（文件或目录均可），供前端把文本中的路径
/// 升级为可点击链接前做存在性验证，避免纯正则猜测的误报。
/// 绝对路径直接校验；相对路径以 base_dir 解析后校验；
/// 解析失败（无基准相对路径 / 含协议前缀）或不存在一律返回 false。
#[tauri::command]
pub async fn path_exists(path: String, base_dir: Option<String>) -> bool {
    let resolved = match crate::app::settings::resolve_open_path(&path, base_dir.as_deref()) {
        Ok(p) => p,
        Err(_) => return false,
    };
    tokio::fs::metadata(&resolved).await.is_ok()
}

/// 读取本地文件并按类型打包为预览载荷。
/// Err 时前端回退到「使用默认应用打开」卡片。
#[tauri::command]
pub async fn preview_read_file(path: String) -> Result<PreviewFile, String> {
    let p = Path::new(&path);
    let meta = tokio::fs::metadata(p)
        .await
        .map_err(|e| format!("读取文件元信息失败: {e}"))?;
    if !meta.is_file() {
        return Err("路径不是文件".into());
    }
    let size = meta.len();
    let ext = p
        .extension()
        .and_then(|e| e.to_str())
        .unwrap_or("")
        .to_ascii_lowercase();

    // 图片 / 视频 / PDF：base64 data URL
    if let Some((kind, mime)) = media_of(&ext) {
        if size > MEDIA_MAX_BYTES {
            return Err(format!(
                "文件过大（{}），超过内嵌预览上限 64MB，请使用默认应用打开",
                fmt_size(size)
            ));
        }
        let bytes = tokio::fs::read(p)
            .await
            .map_err(|e| format!("读取文件失败: {e}"))?;
        let data_url = format!(
            "data:{mime};base64,{}",
            base64::engine::general_purpose::STANDARD.encode(bytes)
        );
        return Ok(PreviewFile {
            kind: kind.into(),
            text: None,
            data_url: Some(data_url),
            lang: None,
            mime: mime.into(),
            size,
        });
    }

    // 文本类（含未知扩展名尝试解码）
    if size > TEXT_MAX_BYTES {
        return Err(format!(
            "文件过大（{}），超过内嵌预览上限 2MB，请使用默认应用打开",
            fmt_size(size)
        ));
    }
    let bytes = tokio::fs::read(p)
        .await
        .map_err(|e| format!("读取文件失败: {e}"))?;
    if bytes.contains(&0) {
        // 二进制且无预览能力的类型（docx / xlsx / zip / …）
        return Ok(unsupported(size));
    }
    let text = String::from_utf8(bytes).map_err(|_| "文件不是有效的 UTF-8 文本".to_string())?;

    let (kind, lang) = if ext == "md" || ext == "markdown" {
        ("markdown", None)
    } else if let Some(lang) = code_lang(&ext) {
        ("code", Some(lang))
    } else {
        ("text", None)
    };
    Ok(PreviewFile {
        kind: kind.into(),
        text: Some(text),
        data_url: None,
        lang: lang.map(str::to_string),
        mime: "text/plain".into(),
        size,
    })
}

#[cfg(test)]
mod tests {
    use super::*;

    #[tokio::test]
    async fn path_exists_reports_existing_file_and_directory() {
        let dir = tempfile::tempdir().unwrap();
        let file = dir.path().join("a.txt");
        std::fs::write(&file, "x").unwrap();
        assert!(path_exists(file.to_string_lossy().into_owned(), None).await);
        // 目录也算存在（点击后由预览面板决定如何展示）
        assert!(path_exists(dir.path().to_string_lossy().into_owned(), None).await);
    }

    #[tokio::test]
    async fn path_exists_missing_path_is_false() {
        let dir = tempfile::tempdir().unwrap();
        let missing = dir.path().join("nope.txt");
        assert!(!path_exists(missing.to_string_lossy().into_owned(), None).await);
    }

    #[tokio::test]
    async fn path_exists_relative_resolves_against_base_dir() {
        let dir = tempfile::tempdir().unwrap();
        std::fs::create_dir_all(dir.path().join("src")).unwrap();
        std::fs::write(dir.path().join("src/main.rs"), "x").unwrap();
        let base = dir.path().to_string_lossy().into_owned();
        assert!(path_exists("src/main.rs".into(), Some(base.clone())).await);
        assert!(path_exists("./src/main.rs".into(), Some(base)).await);
        // 无基准的相对路径解析失败 → false，而不是静默落在进程 cwd
        assert!(!path_exists("src/main.rs".into(), None).await);
    }

    #[tokio::test]
    async fn path_exists_rejects_url_and_missing_base() {
        assert!(!path_exists("file:///tmp/a.txt".into(), None).await);
        assert!(!path_exists("https://example.com/a.txt".into(), Some("/tmp".into())).await);
    }
}
