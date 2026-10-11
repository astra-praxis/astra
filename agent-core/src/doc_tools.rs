//! pandoc 文档工具：extract_document / build_docx。
//!
//! 不给通用 exec：外部程序只通过固定参数的专用工具暴露（build-agent-runtime
//! skill 的安全原则）。超时 60s，错误信息含安装提示。

use crate::error::{Error, Result};
use crate::permissions::ensure_within_root;
use crate::tool::{AgentTool, Artifact, ArtifactKind, ToolContext, ToolOutput};
use async_trait::async_trait;
use serde_json::{json, Value};
use std::path::{Path, PathBuf};

/// pandoc 超时；大文档转换实测在秒级，60s 已足够宽裕
const PANDOC_TIMEOUT: std::time::Duration = std::time::Duration::from_secs(60);

async fn run_pandoc(root: &Path, args: &[&str], tool: &str) -> Result<String> {
    let output = tokio::time::timeout(
        PANDOC_TIMEOUT,
        tokio::process::Command::new("pandoc")
            .args(args)
            .current_dir(root)
            .output(),
    )
    .await
    .map_err(|_| Error::tool(tool, "pandoc 执行超时（60s）"))?
    .map_err(|e| {
        if e.kind() == std::io::ErrorKind::NotFound {
            Error::tool(
                tool,
                "未找到 pandoc，请先安装：brew install pandoc（或访问 pandoc.org）",
            )
        } else {
            Error::tool(tool, e.to_string())
        }
    })?;
    if !output.status.success() {
        let stderr = String::from_utf8_lossy(&output.stderr);
        return Err(Error::tool(tool, format!("pandoc 失败: {stderr}")));
    }
    Ok(String::from_utf8_lossy(&output.stdout).to_string())
}

/// extract_document 固定输出路径：out/converted/<stem>.md（纯函数，供单测）
fn converted_md_path(full: &Path) -> String {
    let stem = full
        .file_stem()
        .map(|s| s.to_string_lossy().to_string())
        .unwrap_or_else(|| "document".into());
    format!("out/converted/{stem}.md")
}

/// build_docx 默认输出路径：源文件同目录同名 .docx（纯函数，供单测）
fn default_docx_output(full: &Path) -> PathBuf {
    let stem = full
        .file_stem()
        .map(|s| s.to_string_lossy().to_string())
        .unwrap_or_else(|| "document".into());
    match full.parent() {
        Some(dir) if !dir.as_os_str().is_empty() => dir.join(format!("{stem}.docx")),
        _ => PathBuf::from(format!("{stem}.docx")),
    }
}

struct ExtractDocument {
    root: PathBuf,
}

#[async_trait]
impl AgentTool for ExtractDocument {
    fn name(&self) -> &str {
        "extract_document"
    }
    fn description(&self) -> &str {
        "用 pandoc 把 .docx 等文档转换为 Markdown（输出固定写到 out/converted/ 目录）。\
         转换结果文件可能很大，请用 read_file 的 offset/limit 分段阅读"
    }
    fn parameters_schema(&self) -> Value {
        json!({
            "type": "object",
            "properties": {
                "path": { "type": "string", "description": "相对项目根的文档路径，如 招标文件.docx" }
            },
            "required": ["path"]
        })
    }
    async fn call(&self, args: Value, _ctx: &ToolContext) -> Result<ToolOutput> {
        let path = args
            .get("path")
            .and_then(|v| v.as_str())
            .ok_or_else(|| Error::tool("extract_document", "缺少 path"))?;
        let full = ensure_within_root(&self.root, &PathBuf::from(path))?;
        if self.root.join(&full).is_dir() {
            return Err(Error::tool("extract_document", "path 是目录而非文件"));
        }

        let out_md = converted_md_path(&full);
        let stem = Path::new(&full)
            .file_stem()
            .map(|s| s.to_string_lossy().to_string())
            .unwrap_or_else(|| "document".into());
        let media_dir = format!("out/converted/media_{stem}");
        let full_s = full.to_string_lossy().into_owned();

        run_pandoc(
            &self.root,
            &[
                &full_s,
                "--wrap=none",
                &format!("--extract-media={media_dir}"),
                "-o",
                &out_md,
            ],
            "extract_document",
        )
        .await?;

        let md_meta = tokio::fs::metadata(self.root.join(&out_md))
            .await
            .map_err(|e| Error::tool("extract_document", e.to_string()))?;
        Ok(ToolOutput::with_artifacts(
            format!(
                "已转换为 {out_md}（{} KB），请用 read_file 分段阅读",
                md_meta.len() / 1024
            ),
            vec![Artifact {
                path: out_md,
                kind: ArtifactKind::FileCreated,
            }],
        ))
    }
}

struct BuildDocx {
    root: PathBuf,
}

#[async_trait]
impl AgentTool for BuildDocx {
    fn name(&self) -> &str {
        "build_docx"
    }
    fn description(&self) -> &str {
        "用 pandoc 把 Markdown 转换为 docx 文件（写操作，需用户确认）。\
         输出默认写到源文件同目录的同名 .docx"
    }
    fn parameters_schema(&self) -> Value {
        json!({
            "type": "object",
            "properties": {
                "path": { "type": "string", "description": "相对项目根的 Markdown 路径" },
                "output": { "type": "string", "description": "输出 docx 的相对路径，可选，默认同名 .docx" }
            },
            "required": ["path"]
        })
    }
    async fn call(&self, args: Value, _ctx: &ToolContext) -> Result<ToolOutput> {
        let path = args
            .get("path")
            .and_then(|v| v.as_str())
            .ok_or_else(|| Error::tool("build_docx", "缺少 path"))?;
        let full = ensure_within_root(&self.root, &PathBuf::from(path))?;
        let output = match args.get("output").and_then(|v| v.as_str()) {
            Some(o) => ensure_within_root(&self.root, &PathBuf::from(o))?,
            None => default_docx_output(&full),
        };

        let full_s = full.to_string_lossy().into_owned();
        let output_s = output.to_string_lossy().into_owned();
        run_pandoc(&self.root, &[&full_s, "-o", &output_s], "build_docx").await?;

        Ok(ToolOutput::with_artifacts(
            format!("已生成 {}", output.display()),
            vec![Artifact {
                path: output.to_string_lossy().into_owned(),
                kind: ArtifactKind::FileCreated,
            }],
        ))
    }
}

/// 注册 pandoc 文档工具（需要项目根）
pub fn register(reg: crate::tool::ToolRegistry, root: &PathBuf) -> crate::tool::ToolRegistry {
    reg.register(ExtractDocument { root: root.clone() })
        .register(BuildDocx { root: root.clone() })
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn converted_md_path_uses_stem() {
        assert_eq!(
            converted_md_path(Path::new("docs/招标文件.docx")),
            "out/converted/招标文件.md"
        );
        assert_eq!(
            converted_md_path(Path::new("a.b.pdf")),
            "out/converted/a.b.md"
        );
    }

    #[test]
    fn default_docx_output_sits_beside_source() {
        assert_eq!(
            default_docx_output(Path::new("方案.md")),
            PathBuf::from("方案.docx")
        );
        assert_eq!(
            default_docx_output(Path::new("out/初稿.md")),
            PathBuf::from("out/初稿.docx")
        );
        // 根级绝对路径：parent 为 "/"（非空），产出 /xxx.docx
        assert_eq!(
            default_docx_output(Path::new("/tmp/x.md")),
            PathBuf::from("/tmp/x.docx")
        );
    }
}
