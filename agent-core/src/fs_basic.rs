//! 基础文件工具：list_files / read_file / write_file。
//!
//! 自 src-tauri/src/agent.rs 迁入（工具属 runtime 层，与 Zed 的 tools
//! 在 agent crate 内一致）。路径校验走 `ensure_within_root`
//! （canonicalize + 符号链接逃逸拦截）。

use crate::error::{Error, Result};
use crate::permissions::ensure_within_root;
use crate::tool::{AgentTool, Artifact, ArtifactKind, ToolContext, ToolOutput};
use async_trait::async_trait;
use serde_json::{json, Value};
use std::path::PathBuf;

// ===== list_files =====

struct ListFiles {
    root: PathBuf,
}

#[async_trait]
impl AgentTool for ListFiles {
    fn name(&self) -> &str {
        "list_files"
    }
    fn description(&self) -> &str {
        "列出项目内目录的文件与子目录"
    }
    fn parameters_schema(&self) -> Value {
        json!({
            "type": "object",
            "properties": {
                "path": { "type": "string", "description": "相对项目根的目录路径，\".\" 表示根目录" }
            },
            "required": ["path"]
        })
    }
    async fn call(&self, args: Value, _ctx: &ToolContext) -> Result<ToolOutput> {
        let path = args.get("path").and_then(|v| v.as_str()).unwrap_or(".");
        let full = ensure_within_root(&self.root, &PathBuf::from(path))?;
        let mut entries: Vec<String> = Vec::new();
        let mut rd = tokio::fs::read_dir(self.root.join(&full))
            .await
            .map_err(|e| Error::tool("list_files", e.to_string()))?;
        while let Some(e) = rd
            .next_entry()
            .await
            .map_err(|e| Error::tool("list_files", e.to_string()))?
        {
            let kind = if e.path().is_dir() { "dir" } else { "file" };
            entries.push(format!("{} {kind}", e.file_name().to_string_lossy()));
        }
        entries.sort();
        Ok(ToolOutput::from(if entries.is_empty() {
            "(空目录)".to_string()
        } else {
            entries.join("\n")
        }))
    }
}

// ===== read_file =====

struct ReadFile {
    root: PathBuf,
}

/// read_file 默认每次返回的最大行数；大文件（如 pandoc 转出的 md）必须分段读。
const READ_FILE_DEFAULT_LIMIT: usize = 2000;

/// cat -n 风格行号：右对齐 6 字符 + 制表符（对齐 Zed read_file 的输出格式，
/// 模型对该格式熟悉，后续引用某一行时可直接用行号定位）。
fn format_with_line_numbers(lines: &[&str], start_line: usize) -> String {
    use std::fmt::Write as _;
    let mut out = String::new();
    for (i, line) in lines.iter().enumerate() {
        let _ = write!(out, "{:>6}\t{}", start_line + i, line);
    }
    out
}

#[async_trait]
impl AgentTool for ReadFile {
    fn name(&self) -> &str {
        "read_file"
    }
    fn description(&self) -> &str {
        "读取项目内的文本文件，带行号。大文件用 offset/limit 分段读取：\
         先全量读，若返回截断提示则用返回的 offset 继续读下一段"
    }
    fn parameters_schema(&self) -> Value {
        json!({
            "type": "object",
            "properties": {
                "path": { "type": "string", "description": "相对项目根的路径" },
                "offset": { "type": "integer", "description": "起始行号（从 1 开始），默认 1" },
                "limit": { "type": "integer", "description": "本次最多读取的行数，默认 2000" }
            },
            "required": ["path"]
        })
    }
    async fn call(&self, args: Value, _ctx: &ToolContext) -> Result<ToolOutput> {
        let path = args
            .get("path")
            .and_then(|v| v.as_str())
            .ok_or_else(|| Error::tool("read_file", "缺少 path"))?;
        let full = ensure_within_root(&self.root, &PathBuf::from(path))?;
        let content = tokio::fs::read_to_string(self.root.join(&full))
            .await
            .map_err(|e| Error::tool("read_file", e.to_string()))?;

        // 1-indexed 闭区间语义对齐 Zed：offset 钳到 >=1，limit 至少 1 行
        let offset = args
            .get("offset")
            .and_then(|v| v.as_u64())
            .map(|v| v as usize)
            .unwrap_or(1)
            .max(1);
        let limit = args
            .get("limit")
            .and_then(|v| v.as_u64())
            .map(|v| v as usize)
            .unwrap_or(READ_FILE_DEFAULT_LIMIT)
            .max(1);

        // split_inclusive 保留每行末尾换行（含最后一行），拼接后格式不丢
        let lines: Vec<&str> = content.split_inclusive('\n').collect();
        let total = lines.len();
        let start_idx = (offset - 1).min(total);
        let end_idx = (start_idx + limit).min(total);

        let mut out = format_with_line_numbers(&lines[start_idx..end_idx], start_idx + 1);
        if end_idx < total {
            out.push_str(&format!(
                "\n（共 {total} 行，当前显示第 {}–{} 行，已截断；继续读取请用 offset={}）",
                start_idx + 1,
                end_idx,
                end_idx + 1
            ));
        }
        Ok(ToolOutput::from(out))
    }
}

// ===== write_file =====

struct WriteFile {
    root: PathBuf,
}

#[async_trait]
impl AgentTool for WriteFile {
    fn name(&self) -> &str {
        "write_file"
    }
    fn description(&self) -> &str {
        "写入文本文件（相对项目根，父目录不存在会自动创建）"
    }
    fn parameters_schema(&self) -> Value {
        json!({
            "type": "object",
            "properties": {
                "path": { "type": "string", "description": "相对项目根的路径" },
                "content": { "type": "string", "description": "完整文件内容（整体覆盖）" }
            },
            "required": ["path", "content"]
        })
    }
    async fn call(&self, args: Value, _ctx: &ToolContext) -> Result<ToolOutput> {
        let path = args
            .get("path")
            .and_then(|v| v.as_str())
            .ok_or_else(|| Error::tool("write_file", "缺少 path"))?;
        let content = args
            .get("content")
            .and_then(|v| v.as_str())
            .ok_or_else(|| Error::tool("write_file", "缺少 content"))?;
        let full = ensure_within_root(&self.root, &PathBuf::from(path))?;
        let target = self.root.join(&full);
        if let Some(parent) = target.parent() {
            tokio::fs::create_dir_all(parent)
                .await
                .map_err(|e| Error::tool("write_file", e.to_string()))?;
        }
        tokio::fs::write(&target, content)
            .await
            .map_err(|e| Error::tool("write_file", e.to_string()))?;
        Ok(ToolOutput::with_artifacts(
            format!("已写入 {path}（{} 字节）", content.len()),
            vec![Artifact {
                path: path.to_string(),
                kind: ArtifactKind::FileCreated,
            }],
        ))
    }
}

/// 注册基础文件工具（需要项目根）
pub fn register(reg: crate::tool::ToolRegistry, root: &PathBuf) -> crate::tool::ToolRegistry {
    reg.register(ListFiles { root: root.clone() })
        .register(ReadFile { root: root.clone() })
        .register(WriteFile { root: root.clone() })
}

#[cfg(test)]
mod tests {
    use super::*;

    fn tmp_root(tag: &str) -> PathBuf {
        let dir =
            std::env::temp_dir().join(format!("agent-core-fsbasic-{tag}-{}", std::process::id()));
        let _ = std::fs::remove_dir_all(&dir);
        std::fs::create_dir_all(&dir).unwrap();
        dir
    }

    fn ctx() -> ToolContext {
        ToolContext {
            project_root: None,
            bridge: std::sync::Arc::new(crate::permissions::AutoBridge(
                crate::permissions::Decision::Allow,
            )),
            cancelled: tokio_util::sync::CancellationToken::new(),
        }
    }

    #[test]
    fn line_numbers_are_aligned() {
        let out = format_with_line_numbers(&["a\n", "b\n"], 9);
        assert_eq!(out, "     9\ta\n    10\tb\n");
    }

    #[tokio::test]
    async fn read_file_offset_limit_and_truncation_hint() {
        let root = tmp_root("read");
        let content: String = (1..=30).map(|i| format!("line{i}\n")).collect();
        std::fs::write(root.join("big.txt"), &content).unwrap();

        let tool = ReadFile { root: root.clone() };
        let out = tool
            .call(json!({"path": "big.txt", "offset": 25, "limit": 3}), &ctx())
            .await
            .unwrap();
        assert!(out.content.contains("line25"));
        assert!(out.content.contains("line27"));
        assert!(!out.content.contains("line28"));
        assert!(out.content.contains("已截断"));
        assert!(out.content.contains("offset=28"));

        // 默认 limit：超过 2000 行才截断
        let small: String = (1..=10).map(|i| format!("l{i}\n")).collect();
        std::fs::write(root.join("small.txt"), &small).unwrap();
        let out = tool
            .call(json!({"path": "small.txt"}), &ctx())
            .await
            .unwrap();
        assert!(!out.content.contains("已截断"));
        std::fs::remove_dir_all(&root).unwrap();
    }

    #[tokio::test]
    async fn read_file_rejects_escape() {
        let root = tmp_root("escape");
        let tool = ReadFile { root: root.clone() };
        let err = tool
            .call(json!({"path": "/etc/passwd"}), &ctx())
            .await
            .unwrap_err();
        assert!(matches!(err, Error::PathEscape(_)));
        std::fs::remove_dir_all(&root).unwrap();
    }

    #[tokio::test]
    async fn write_file_creates_parents_and_artifact() {
        let root = tmp_root("write");
        let tool = WriteFile { root: root.clone() };
        let out = tool
            .call(
                json!({"path": "docs/2026/report.md", "content": "hi"}),
                &ctx(),
            )
            .await
            .unwrap();
        assert_eq!(out.artifacts[0].path, "docs/2026/report.md");
        assert_eq!(out.artifacts[0].kind, ArtifactKind::FileCreated);
        assert_eq!(
            std::fs::read_to_string(root.join("docs/2026/report.md")).unwrap(),
            "hi"
        );
        std::fs::remove_dir_all(&root).unwrap();
    }

    #[tokio::test]
    async fn list_files_marks_kinds_and_empty() {
        let root = tmp_root("list");
        std::fs::create_dir_all(root.join("src")).unwrap();
        std::fs::write(root.join("a.txt"), "x").unwrap();

        let tool = ListFiles { root: root.clone() };
        let out = tool.call(json!({"path": "."}), &ctx()).await.unwrap();
        assert!(out.content.contains("src dir"));
        assert!(out.content.contains("a.txt file"));

        let empty = ListFiles { root: root.clone() };
        let out = empty.call(json!({"path": "src"}), &ctx()).await.unwrap();
        assert_eq!(out.content, "(空目录)");
        std::fs::remove_dir_all(&root).unwrap();
    }
}
