//! 文件系统类内置工具（对标 Zed crates/agent/src/tools/ 中适用的子集：
//! grep_tool → search_files、find_path_tool → find_path、edit_file_tool → edit_file、
//! copy_path_tool → copy_path、move_path_tool → move_path、create_directory_tool → create_directory）。
//!
//! 选型原则（与 run 循环/权限层约定一致）：
//! - 只操作项目根内路径（ensure_within_root 兜底，含符号链接逃逸检测）
//! - 搜索类只读（默认 Allow）；一切产生变更的工具走 Confirm（宿主在策略里声明）
//! - 目录遍历跳过隐藏目录与常见构建产物（与宿主 @ 引用弹层的 skip 约定对齐）
//! - 输出有上限，防止超大项目把上下文撑爆

use crate::error::{Error, Result};
use crate::permissions::ensure_within_root;
use crate::tool::{AgentTool, Artifact, ArtifactKind, ToolContext, ToolOutput};
use serde_json::{json, Value};
use std::path::{Path, PathBuf};

fn tool_error(tool: &str, msg: impl Into<String>) -> Error {
    Error::Tool {
        tool: tool.into(),
        message: msg.into(),
    }
}

/// 遍历项目时跳过的目录（宿主 @ 引用弹层用同一份约定；隐藏目录一律跳过）
const SKIP_DIRS: &[&str] = &[
    "node_modules",
    ".git",
    "dist",
    "build",
    "target",
    ".next",
    ".nuxt",
    "__pycache__",
    ".venv",
    "venv",
    "vendor",
    "coverage",
    ".idea",
    ".vscode",
    ".cache",
    "out",
    ".output",
    "bin",
    "obj",
];

const MAX_DEPTH: u32 = 12;
const MAX_FILES: usize = 2000;

/// 收集 base 下的相对路径（文件），应用 skip 规则，按相对路径排序保证输出稳定。
/// 满足 `pred` 的路径写入 out，达到上限即停止。
fn collect_files(base: &Path, pred: &mut impl FnMut(&str) -> bool, out: &mut Vec<String>) {
    fn walk(
        dir: &Path,
        rel: &str,
        depth: u32,
        pred: &mut impl FnMut(&str) -> bool,
        out: &mut Vec<String>,
    ) {
        if depth >= MAX_DEPTH || out.len() >= MAX_FILES {
            return;
        }
        let Ok(entries) = std::fs::read_dir(dir) else {
            return;
        };
        let mut items: Vec<_> = entries.flatten().collect();
        items.sort_by_key(|e| e.file_name());
        for entry in items {
            if out.len() >= MAX_FILES {
                return;
            }
            let Ok(file_type) = entry.file_type() else {
                continue;
            };
            let name = entry.file_name().to_string_lossy().to_string();
            let child_rel = if rel.is_empty() {
                name.clone()
            } else {
                format!("{rel}/{name}")
            };
            if file_type.is_dir() {
                if name.starts_with('.') || SKIP_DIRS.contains(&name.as_str()) {
                    continue;
                }
                walk(&entry.path(), &child_rel, depth + 1, pred, out);
            } else if file_type.is_file() && !name.starts_with('.') && pred(&child_rel) {
                out.push(child_rel);
            }
        }
    }
    walk(base, "", 0, pred, out);
}

// ===== search_files（对标 Zed GrepTool） =====

/// 返回给模型的最大匹配行数；超出会提示缩小范围继续搜
const SEARCH_MAX_MATCHES: usize = 200;

struct SearchFiles {
    root: PathBuf,
}

/// 内容搜索：正则匹配文件内容，输出 `相对路径:行号: 内容`。
/// 可选 include glob 过滤文件名（如 `*.md`）。
#[async_trait::async_trait]
impl AgentTool for SearchFiles {
    fn name(&self) -> &str {
        "search_files"
    }
    fn description(&self) -> &str {
        "在项目内按正则搜索文件内容，返回「路径:行号: 内容」。path 可传目录或单个文件；\
         大项目建议用 include 按文件名缩小范围（如 \"*.md\"）；超上限时缩小 pattern 或路径重搜"
    }
    fn parameters_schema(&self) -> Value {
        json!({
            "type": "object",
            "properties": {
                "pattern": { "type": "string", "description": "正则表达式" },
                "path": { "type": "string", "description": "限定搜索的子目录，默认 \".\"（项目根）" },
                "include": { "type": "string", "description": "可选：文件名 glob 过滤，如 \"*.md\"" }
            },
            "required": ["pattern"]
        })
    }
    async fn call(&self, args: Value, _ctx: &ToolContext) -> Result<ToolOutput> {
        let pattern = args
            .get("pattern")
            .and_then(|v| v.as_str())
            .ok_or_else(|| tool_error("search_files", "缺少 pattern"))?;
        let sub = args.get("path").and_then(|v| v.as_str()).unwrap_or(".");
        let include = args.get("include").and_then(|v| v.as_str());

        let re = regex::Regex::new(pattern)
            .map_err(|e| tool_error("search_files", format!("正则无效: {e}")))?;
        let matcher = match include {
            Some(g) => Some(
                glob::Pattern::new(g)
                    .map_err(|e| tool_error("search_files", format!("include glob 无效: {e}")))?,
            ),
            None => None,
        };
        let base = ensure_within_root(&self.root, &PathBuf::from(sub))?;
        let base = self.root.join(&base);
        // path 允许指向单个文件（模型常见用法）：只搜该文件
        let single_file = base.is_file();
        // 输出给模型的路径必须相对项目根（可直接拿去 read_file）；base == root 时不加前缀。
        // 单文件搜索时 rel 已相对项目根，前缀必须为空。
        let prefix = if single_file || base == self.root {
            String::new()
        } else {
            format!(
                "{}/",
                base.strip_prefix(&self.root)
                    .unwrap_or(Path::new(""))
                    .display()
            )
        };

        // 阻塞任务里同步遍历 + 读文件，避免占用异步线程
        let root = self.root.clone();
        // 单文件时 rel 相对项目根（prefix 已带子目录），目录时 rel 相对 base
        let file_base = if single_file {
            root.clone()
        } else {
            base.clone()
        };
        let matches = tokio::task::spawn_blocking(move || {
            let mut files: Vec<String> = Vec::new();
            if single_file {
                files.push(
                    base.strip_prefix(&root)
                        .unwrap_or(&base)
                        .to_string_lossy()
                        .to_string(),
                );
            } else {
                collect_files(
                    &base,
                    &mut |rel| {
                        matcher
                            .as_ref()
                            .map(|m| m.matches_path(Path::new(rel)))
                            .unwrap_or(true)
                    },
                    &mut files,
                );
            }
            let mut out = Vec::new();
            'files: for rel in files {
                let Ok(bytes) = std::fs::read(file_base.join(&rel)) else {
                    continue;
                };
                // 二进制检测：首个 NUL 即视为二进制（对齐 grep 惯例）
                if bytes.iter().take(1024).any(|&b| b == 0) {
                    continue;
                }
                let Ok(text) = String::from_utf8(bytes) else {
                    continue;
                };
                for (i, line) in text.lines().enumerate() {
                    if re.is_match(line) {
                        out.push(format!("{prefix}{}:{}: {}", rel, i + 1, line.trim_end()));
                        if out.len() >= SEARCH_MAX_MATCHES {
                            break 'files;
                        }
                    }
                }
            }
            out
        })
        .await
        .map_err(|e| tool_error("search_files", e.to_string()))?;

        if matches.is_empty() {
            return Ok(ToolOutput::from("未找到匹配".to_string()));
        }
        let mut text = matches.join("\n");
        if matches.len() >= SEARCH_MAX_MATCHES {
            text.push_str(&format!(
                "\n（已达 {SEARCH_MAX_MATCHES} 条上限，结果可能不完整；建议缩小 pattern 或指定 path/include 续搜）"
            ));
        }
        Ok(ToolOutput::from(text))
    }
}

// ===== find_path（对标 Zed FindPathTool） =====

/// 返回给模型的最大结果数
const FIND_MAX_RESULTS: usize = 200;

struct FindPath {
    root: PathBuf,
}

#[async_trait::async_trait]
impl AgentTool for FindPath {
    fn name(&self) -> &str {
        "find_path"
    }
    fn description(&self) -> &str {
        "按 glob 模式查找项目内文件路径（如 \"**/*.md\"、\"src/**/*.rs\"、\"README*\"）"
    }
    fn parameters_schema(&self) -> Value {
        json!({
            "type": "object",
            "properties": {
                "pattern": { "type": "string", "description": "glob 模式，匹配相对路径；** 跨目录，* 不跨 /" },
                "path": { "type": "string", "description": "限定查找的子目录，默认 \".\"（项目根）" }
            },
            "required": ["pattern"]
        })
    }
    async fn call(&self, args: Value, _ctx: &ToolContext) -> Result<ToolOutput> {
        let pattern = args
            .get("pattern")
            .and_then(|v| v.as_str())
            .ok_or_else(|| tool_error("find_path", "缺少 pattern"))?;
        let sub = args.get("path").and_then(|v| v.as_str()).unwrap_or(".");
        let glob = glob::Pattern::new(pattern)
            .map_err(|e| tool_error("find_path", format!("glob 无效: {e}")))?;
        let base = ensure_within_root(&self.root, &PathBuf::from(sub))?;
        let base = self.root.join(&base);
        // 输出给模型的路径必须相对项目根（可直接拿去 read_file）；base == root 时不加前缀。
        // glob 匹配仍按 base 相对路径，避免「* 不跨 /」语义被子路径破坏
        let prefix = if base == self.root {
            String::new()
        } else {
            format!(
                "{}/",
                base.strip_prefix(&self.root)
                    .unwrap_or(Path::new(""))
                    .display()
            )
        };

        let results: Vec<String> = tokio::task::spawn_blocking(move || {
            let mut out: Vec<String> = Vec::new();
            collect_files(
                &base,
                &mut |rel| glob.matches_path(Path::new(rel)),
                &mut out,
            );
            out.into_iter()
                .map(|rel| format!("{prefix}{rel}"))
                .collect()
        })
        .await
        .map_err(|e| tool_error("find_path", e.to_string()))?;

        if results.is_empty() {
            return Ok(ToolOutput::from("未找到匹配文件".to_string()));
        }
        let mut text = results.join("\n");
        if results.len() >= FIND_MAX_RESULTS {
            text.push_str(&format!(
                "\n（已达 {FIND_MAX_RESULTS} 条上限，建议细化 pattern）"
            ));
        }
        Ok(ToolOutput::from(text))
    }
}

// ===== edit_file（对标 Zed EditFileTool：精确替换，避免整文件重写） =====

struct EditFile {
    root: PathBuf,
}

#[async_trait::async_trait]
impl AgentTool for EditFile {
    fn name(&self) -> &str {
        "edit_file"
    }
    fn description(&self) -> &str {
        "编辑项目内文本文件：把 old_text 精确替换为 new_text。\
         old_text 必须与文件内容完全一致（含缩进与换行），唯一匹配时直接替换；\
         出现多处匹配时需设置 replace_all=true 或在 old_text 中包含更多上下文使其唯一"
    }
    fn parameters_schema(&self) -> Value {
        json!({
            "type": "object",
            "properties": {
                "path": { "type": "string", "description": "相对项目根的路径" },
                "old_text": { "type": "string", "description": "要被替换的原文（须与文件内容逐字符一致）" },
                "new_text": { "type": "string", "description": "替换后的新文本" },
                "replace_all": { "type": "boolean", "description": "替换全部匹配，默认 false（多处匹配且未开启时报错）" }
            },
            "required": ["path", "old_text", "new_text"]
        })
    }
    async fn call(&self, args: Value, _ctx: &ToolContext) -> Result<ToolOutput> {
        let path = args
            .get("path")
            .and_then(|v| v.as_str())
            .ok_or_else(|| tool_error("edit_file", "缺少 path"))?;
        let old_text = args
            .get("old_text")
            .and_then(|v| v.as_str())
            .ok_or_else(|| tool_error("edit_file", "缺少 old_text"))?;
        let new_text = args
            .get("new_text")
            .and_then(|v| v.as_str())
            .ok_or_else(|| tool_error("edit_file", "缺少 new_text"))?;
        let replace_all = args
            .get("replace_all")
            .and_then(|v| v.as_bool())
            .unwrap_or(false);

        if old_text.is_empty() {
            return Err(tool_error("edit_file", "old_text 不能为空"));
        }

        let full = ensure_within_root(&self.root, &PathBuf::from(path))?;
        let target = self.root.join(&full);
        let content = tokio::fs::read_to_string(&target)
            .await
            .map_err(|e| tool_error("edit_file", e.to_string()))?;

        let count = content.matches(old_text).count();
        if count == 0 {
            return Err(tool_error(
                "edit_file",
                "未找到 old_text：请先 read_file 确认内容，old_text 必须与文件逐字符一致（含缩进）",
            ));
        }
        if count > 1 && !replace_all {
            return Err(tool_error(
                "edit_file",
                format!("old_text 匹配了 {count} 处：请扩大 old_text 上下文使其唯一，或设置 replace_all=true"),
            ));
        }

        let first_line = content[..content.find(old_text).unwrap()]
            .matches('\n')
            .count()
            + 1;
        let updated = content.replace(old_text, new_text);
        tokio::fs::write(&target, &updated)
            .await
            .map_err(|e| tool_error("edit_file", e.to_string()))?;

        Ok(ToolOutput::with_artifacts(
            format!("已替换 {count} 处（第 {first_line} 行起）"),
            vec![Artifact {
                path: path.to_string(),
                kind: ArtifactKind::FileModified,
            }],
        ))
    }
}

// ===== copy_path（对标 Zed CopyPathTool） =====

struct CopyPath {
    root: PathBuf,
}

#[async_trait::async_trait]
impl AgentTool for CopyPath {
    fn name(&self) -> &str {
        "copy_path"
    }
    fn description(&self) -> &str {
        "复制项目内的文件或目录（目录递归复制）。目标路径已存在时报错，不做覆盖"
    }
    fn parameters_schema(&self) -> Value {
        json!({
            "type": "object",
            "properties": {
                "from": { "type": "string", "description": "源路径（相对项目根）" },
                "to": { "type": "string", "description": "目标路径（相对项目根，不能已存在）" }
            },
            "required": ["from", "to"]
        })
    }
    async fn call(&self, args: Value, _ctx: &ToolContext) -> Result<ToolOutput> {
        let from = arg_path(&args, "copy_path", "from")?;
        let to = arg_path(&args, "copy_path", "to")?;
        let src = self.root.join(ensure_within_root(&self.root, &from)?);
        let dst_rel = ensure_within_root(&self.root, &to)?;
        let dst = self.root.join(&dst_rel);

        if !src.exists() {
            return Err(tool_error(
                "copy_path",
                format!("源不存在: {}", from.display()),
            ));
        }
        if dst.exists() {
            return Err(tool_error(
                "copy_path",
                format!("目标已存在，不做覆盖: {}", to.display()),
            ));
        }

        copy_recursive(&src, &dst).await?;
        Ok(ToolOutput::with_artifacts(
            format!("已复制 {} → {}", from.display(), to.display()),
            vec![Artifact {
                path: dst_rel.to_string_lossy().to_string(),
                kind: ArtifactKind::FileCreated,
            }],
        ))
    }
}

fn arg_path(args: &Value, tool: &str, key: &str) -> Result<PathBuf> {
    args.get(key)
        .and_then(|v| v.as_str())
        .map(PathBuf::from)
        .ok_or_else(|| tool_error(tool, format!("缺少 {key}")))
}

async fn copy_recursive(src: &Path, dst: &Path) -> Result<()> {
    if src.is_dir() {
        tokio::fs::create_dir_all(dst)
            .await
            .map_err(|e| tool_error("copy_path", e.to_string()))?;
        let mut rd = tokio::fs::read_dir(src)
            .await
            .map_err(|e| tool_error("copy_path", e.to_string()))?;
        while let Some(e) = rd
            .next_entry()
            .await
            .map_err(|e| tool_error("copy_path", e.to_string()))?
        {
            let name = e.file_name();
            // 跳过符号链接，防止把项目外内容带进来
            if e.file_type().await.map(|t| t.is_symlink()).unwrap_or(true) {
                continue;
            }
            Box::pin(copy_recursive(&e.path(), &dst.join(name))).await?;
        }
        Ok(())
    } else {
        if let Some(parent) = dst.parent() {
            tokio::fs::create_dir_all(parent)
                .await
                .map_err(|e| tool_error("copy_path", e.to_string()))?;
        }
        tokio::fs::copy(src, dst)
            .await
            .map_err(|e| tool_error("copy_path", e.to_string()))?;
        Ok(())
    }
}

// ===== move_path（对标 Zed MovePathTool） =====

struct MovePath {
    root: PathBuf,
}

#[async_trait::async_trait]
impl AgentTool for MovePath {
    fn name(&self) -> &str {
        "move_path"
    }
    fn description(&self) -> &str {
        "移动或重命名项目内的文件/目录。目标路径已存在时报错，不做覆盖"
    }
    fn parameters_schema(&self) -> Value {
        json!({
            "type": "object",
            "properties": {
                "from": { "type": "string", "description": "源路径（相对项目根）" },
                "to": { "type": "string", "description": "目标路径（相对项目根，不能已存在）" }
            },
            "required": ["from", "to"]
        })
    }
    async fn call(&self, args: Value, _ctx: &ToolContext) -> Result<ToolOutput> {
        let from = arg_path(&args, "move_path", "from")?;
        let to = arg_path(&args, "move_path", "to")?;
        let src = self.root.join(ensure_within_root(&self.root, &from)?);
        let dst_rel = ensure_within_root(&self.root, &to)?;
        let dst = self.root.join(&dst_rel);

        if !src.exists() {
            return Err(tool_error(
                "move_path",
                format!("源不存在: {}", from.display()),
            ));
        }
        if dst.exists() {
            return Err(tool_error(
                "move_path",
                format!("目标已存在，不做覆盖: {}", to.display()),
            ));
        }
        if let Some(parent) = dst.parent() {
            tokio::fs::create_dir_all(parent)
                .await
                .map_err(|e| tool_error("move_path", e.to_string()))?;
        }
        tokio::fs::rename(&src, &dst)
            .await
            .map_err(|e| tool_error("move_path", e.to_string()))?;
        Ok(ToolOutput::with_artifacts(
            format!("已移动 {} → {}", from.display(), to.display()),
            vec![Artifact {
                path: dst_rel.to_string_lossy().to_string(),
                kind: ArtifactKind::FileCreated,
            }],
        ))
    }
}

// ===== create_directory（对标 Zed CreateDirectoryTool） =====

struct CreateDirectory {
    root: PathBuf,
}

#[async_trait::async_trait]
impl AgentTool for CreateDirectory {
    fn name(&self) -> &str {
        "create_directory"
    }
    fn description(&self) -> &str {
        "创建项目内目录（含父目录，mkdir -p 语义）"
    }
    fn parameters_schema(&self) -> Value {
        json!({
            "type": "object",
            "properties": {
                "path": { "type": "string", "description": "相对项目根的目录路径" }
            },
            "required": ["path"]
        })
    }
    async fn call(&self, args: Value, _ctx: &ToolContext) -> Result<ToolOutput> {
        let path = arg_path(&args, "create_directory", "path")?;
        let full = ensure_within_root(&self.root, &path)?;
        tokio::fs::create_dir_all(self.root.join(&full))
            .await
            .map_err(|e| tool_error("create_directory", e.to_string()))?;
        Ok(ToolOutput::from(format!("已创建目录 {}", path.display())))
    }
}

/// 把全部文件系统工具注册进注册表（有项目根时调用）。
/// 权限：search_files / find_path 默认只读 Allow（ToolPolicy::default 已含 search_files）；
/// edit_file / copy_path / move_path / create_directory 由宿主放入 confirm_tools。
pub fn register(reg: crate::tool::ToolRegistry, root: &PathBuf) -> crate::tool::ToolRegistry {
    reg.register(SearchFiles { root: root.clone() })
        .register(FindPath { root: root.clone() })
        .register(EditFile { root: root.clone() })
        .register(CopyPath { root: root.clone() })
        .register(MovePath { root: root.clone() })
        .register(CreateDirectory { root: root.clone() })
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::permissions::{AutoBridge, Decision};
    use crate::tool::ToolRegistry as R;
    use std::sync::Arc;
    use tokio_util::sync::CancellationToken;

    fn tmp_root(tag: &str) -> PathBuf {
        let root =
            std::env::temp_dir().join(format!("astra-fs-tools-{tag}-{}", std::process::id()));
        let _ = std::fs::remove_dir_all(&root);
        std::fs::create_dir_all(&root).unwrap();
        root
    }

    fn ctx() -> ToolContext {
        ToolContext {
            project_root: None,
            bridge: Arc::new(AutoBridge(Decision::Deny)),
            cancelled: CancellationToken::new(),
        }
    }

    fn text_of(out: ToolOutput) -> String {
        out.content
    }

    // ----- collect_files / skip 规则 -----

    #[test]
    fn collect_files_skips_hidden_and_build_dirs() {
        let root = tmp_root("collect");
        std::fs::create_dir_all(root.join("src/components")).unwrap();
        std::fs::create_dir_all(root.join("node_modules/pkg")).unwrap();
        std::fs::create_dir_all(root.join(".git")).unwrap();
        std::fs::write(root.join("README.md"), "x").unwrap();
        std::fs::write(root.join("src/main.rs"), "x").unwrap();
        std::fs::write(root.join("src/components/Button.tsx"), "x").unwrap();
        std::fs::write(root.join("node_modules/pkg/index.js"), "x").unwrap();
        std::fs::write(root.join(".git/config"), "x").unwrap();

        let mut out = Vec::new();
        collect_files(&root, &mut |_| true, &mut out);
        assert_eq!(
            out,
            vec![
                "README.md".to_string(),
                "src/components/Button.tsx".to_string(),
                "src/main.rs".to_string(),
            ]
        );
    }

    // ----- search_files -----

    #[tokio::test]
    async fn search_files_finds_matches_with_line_numbers() {
        let root = tmp_root("search");
        std::fs::write(root.join("a.md"), "第一行\n招标要求：质量优先\n第三行").unwrap();
        std::fs::write(root.join("b.md"), "无关键词内容").unwrap();

        let tool = SearchFiles { root: root.clone() };
        let text = text_of(tool.call(json!({"pattern": "招标"}), &ctx()).await.unwrap());
        assert_eq!(text, "a.md:2: 招标要求：质量优先");
    }

    #[tokio::test]
    async fn search_files_respects_include_and_skips_binary() {
        let root = tmp_root("search2");
        std::fs::write(root.join("doc.md"), "关键词在这里").unwrap();
        std::fs::write(root.join("doc.txt"), "关键词在txt里").unwrap();
        std::fs::write(root.join("bin.md"), b"\x00\x01binary".to_vec()).unwrap();

        let tool = SearchFiles { root: root.clone() };
        let text = text_of(
            tool.call(json!({"pattern": "关键词", "include": "*.md"}), &ctx())
                .await
                .unwrap(),
        );
        assert_eq!(text, "doc.md:1: 关键词在这里");
    }

    #[tokio::test]
    async fn search_files_reports_no_match() {
        let root = tmp_root("search3");
        std::fs::write(root.join("a.md"), "普通内容").unwrap();
        let tool = SearchFiles { root };
        let text = text_of(
            tool.call(json!({"pattern": "不存在"}), &ctx())
                .await
                .unwrap(),
        );
        assert_eq!(text, "未找到匹配");
    }

    /// path 传单个文件时只搜该文件（此前传文件路径会静默返回「未找到匹配」，
    /// 迫使模型退化成分块 read_file，轮数和上下文双暴涨）
    #[tokio::test]
    async fn search_files_accepts_single_file_path() {
        let root = tmp_root("search_file");
        std::fs::write(root.join("doc.md"), "# 第一章\n正文\n# 第二章").unwrap();
        std::fs::write(root.join("other.md"), "# 第一章").unwrap();
        let tool = SearchFiles { root: root.clone() };
        let text = text_of(
            tool.call(json!({"pattern": "^# 第", "path": "doc.md"}), &ctx())
                .await
                .unwrap(),
        );
        // 只命中 doc.md 的两行，other.md 不参与
        assert_eq!(text, "doc.md:1: # 第一章\ndoc.md:3: # 第二章");
    }

    /// 子目录内文件的 path 也要返回可直接 read_file 的相对路径
    #[tokio::test]
    async fn search_files_single_file_in_subdir() {
        let root = tmp_root("search_file_sub");
        std::fs::create_dir_all(root.join("out")).unwrap();
        std::fs::write(root.join("out/converted.md"), "招标要求：质量优先").unwrap();
        let tool = SearchFiles { root: root.clone() };
        let text = text_of(
            tool.call(
                json!({"pattern": "招标", "path": "out/converted.md"}),
                &ctx(),
            )
            .await
            .unwrap(),
        );
        assert_eq!(text, "out/converted.md:1: 招标要求：质量优先");
    }

    #[tokio::test]
    async fn search_files_rejects_invalid_regex() {
        let root = tmp_root("search4");
        let tool = SearchFiles { root };
        let err = tool
            .call(json!({"pattern": "(["}), &ctx())
            .await
            .unwrap_err();
        match err {
            Error::Tool { tool, .. } => assert_eq!(tool, "search_files"),
            other => panic!("应为工具错误，实际 {other:?}"),
        }
    }

    #[tokio::test]
    async fn search_files_escapes_subdirectory_scope() {
        let root = tmp_root("search5");
        std::fs::create_dir_all(root.join("sub")).unwrap();
        std::fs::write(root.join("sub/a.md"), "目标词").unwrap();
        std::fs::write(root.join("b.md"), "目标词在外面").unwrap();

        let tool = SearchFiles { root };
        let text = text_of(
            tool.call(json!({"pattern": "目标词", "path": "sub"}), &ctx())
                .await
                .unwrap(),
        );
        assert_eq!(text, "sub/a.md:1: 目标词");
    }

    // ----- find_path -----

    #[tokio::test]
    async fn find_path_matches_glob_patterns() {
        let root = tmp_root("find");
        std::fs::create_dir_all(root.join("docs/nested")).unwrap();
        std::fs::write(root.join("docs/guide.md"), "x").unwrap();
        std::fs::write(root.join("docs/nested/spec.md"), "x").unwrap();
        std::fs::write(root.join("main.rs"), "x").unwrap();

        let tool = FindPath { root: root.clone() };
        let text = text_of(
            tool.call(json!({"pattern": "**/*.md"}), &ctx())
                .await
                .unwrap(),
        );
        assert_eq!(text, "docs/guide.md\ndocs/nested/spec.md");

        // 精确子目录限定：glob 按子目录相对路径匹配，输出仍是项目根相对路径
        let text = text_of(
            tool.call(json!({"pattern": "*.md", "path": "docs/nested"}), &ctx())
                .await
                .unwrap(),
        );
        assert_eq!(text, "docs/nested/spec.md");
    }

    #[tokio::test]
    async fn find_path_no_match() {
        let root = tmp_root("find2");
        std::fs::write(root.join("a.rs"), "x").unwrap();
        let tool = FindPath { root };
        let text = text_of(
            tool.call(json!({"pattern": "**/*.xyz"}), &ctx())
                .await
                .unwrap(),
        );
        assert_eq!(text, "未找到匹配文件");
    }

    // ----- edit_file -----

    #[tokio::test]
    async fn edit_file_replaces_unique_match() {
        let root = tmp_root("edit");
        std::fs::write(root.join("a.md"), "第一章\n旧内容\n结尾").unwrap();

        let tool = EditFile { root: root.clone() };
        let out = tool
            .call(
                json!({"path": "a.md", "old_text": "旧内容", "new_text": "新内容"}),
                &ctx(),
            )
            .await
            .unwrap();
        assert_eq!(
            std::fs::read_to_string(root.join("a.md")).unwrap(),
            "第一章\n新内容\n结尾"
        );
        assert_eq!(out.artifacts[0].kind, ArtifactKind::FileModified);
        assert!(text_of(out).contains("第 2 行"), "应含行号");
    }

    #[tokio::test]
    async fn edit_file_multiple_matches_error_without_replace_all() {
        let root = tmp_root("edit2");
        std::fs::write(root.join("a.md"), "重复\n重复").unwrap();
        let tool = EditFile { root };
        let err = tool
            .call(
                json!({"path": "a.md", "old_text": "重复", "new_text": "x"}),
                &ctx(),
            )
            .await
            .unwrap_err();
        match err {
            Error::Tool { message, .. } => assert!(message.contains("2 处")),
            other => panic!("应为工具错误，实际 {other:?}"),
        }
    }

    #[tokio::test]
    async fn edit_file_replace_all_updates_every_match() {
        let root = tmp_root("edit3");
        std::fs::write(root.join("a.md"), "重复\n重复").unwrap();
        let tool = EditFile { root: root.clone() };
        tool.call(
            json!({"path": "a.md", "old_text": "重复", "new_text": "x", "replace_all": true}),
            &ctx(),
        )
        .await
        .unwrap();
        assert_eq!(std::fs::read_to_string(root.join("a.md")).unwrap(), "x\nx");
    }

    #[tokio::test]
    async fn edit_file_missing_text_is_error() {
        let root = tmp_root("edit4");
        std::fs::write(root.join("a.md"), "hello").unwrap();
        let tool = EditFile { root };
        let err = tool
            .call(
                json!({"path": "a.md", "old_text": "不存在", "new_text": "x"}),
                &ctx(),
            )
            .await
            .unwrap_err();
        match err {
            Error::Tool { message, .. } => assert!(message.contains("未找到")),
            other => panic!("应为工具错误，实际 {other:?}"),
        }
    }

    #[tokio::test]
    async fn edit_file_empty_old_text_is_error() {
        let root = tmp_root("edit5");
        std::fs::write(root.join("a.md"), "hello").unwrap();
        let tool = EditFile { root };
        let err = tool
            .call(
                json!({"path": "a.md", "old_text": "", "new_text": "x"}),
                &ctx(),
            )
            .await
            .unwrap_err();
        match err {
            Error::Tool { message, .. } => assert!(message.contains("不能为空")),
            other => panic!("应为工具错误，实际 {other:?}"),
        }
    }

    // ----- copy_path / move_path / create_directory -----

    #[tokio::test]
    async fn copy_path_copies_file_and_refuses_existing_target() {
        let root = tmp_root("copy");
        std::fs::write(root.join("src.txt"), "内容").unwrap();

        let tool = CopyPath { root: root.clone() };
        tool.call(json!({"from": "src.txt", "to": "out/dst.txt"}), &ctx())
            .await
            .unwrap();
        assert_eq!(
            std::fs::read_to_string(root.join("out/dst.txt")).unwrap(),
            "内容"
        );

        let err = tool
            .call(json!({"from": "src.txt", "to": "out/dst.txt"}), &ctx())
            .await
            .unwrap_err();
        match err {
            Error::Tool { message, .. } => assert!(message.contains("已存在")),
            other => panic!("应为工具错误，实际 {other:?}"),
        }
    }

    #[tokio::test]
    async fn copy_path_recursively_copies_directory() {
        let root = tmp_root("copy2");
        std::fs::create_dir_all(root.join("dir/sub")).unwrap();
        std::fs::write(root.join("dir/a.txt"), "A").unwrap();
        std::fs::write(root.join("dir/sub/b.txt"), "B").unwrap();

        let tool = CopyPath { root: root.clone() };
        tool.call(json!({"from": "dir", "to": "backup"}), &ctx())
            .await
            .unwrap();
        assert_eq!(
            std::fs::read_to_string(root.join("backup/sub/b.txt")).unwrap(),
            "B"
        );
    }

    #[tokio::test]
    async fn copy_path_missing_source_is_error() {
        let root = tmp_root("copy3");
        let tool = CopyPath { root };
        let err = tool
            .call(json!({"from": "nope.txt", "to": "x.txt"}), &ctx())
            .await
            .unwrap_err();
        match err {
            Error::Tool { message, .. } => assert!(message.contains("不存在")),
            other => panic!("应为工具错误，实际 {other:?}"),
        }
    }

    #[tokio::test]
    async fn move_path_renames_and_refuses_existing_target() {
        let root = tmp_root("move");
        std::fs::write(root.join("old.md"), "内容").unwrap();

        let tool = MovePath { root: root.clone() };
        tool.call(json!({"from": "old.md", "to": "new.md"}), &ctx())
            .await
            .unwrap();
        assert!(!root.join("old.md").exists());
        assert_eq!(
            std::fs::read_to_string(root.join("new.md")).unwrap(),
            "内容"
        );

        std::fs::write(root.join("other.md"), "x").unwrap();
        let err = tool
            .call(json!({"from": "new.md", "to": "other.md"}), &ctx())
            .await
            .unwrap_err();
        match err {
            Error::Tool { message, .. } => assert!(message.contains("已存在")),
            other => panic!("应为工具错误，实际 {other:?}"),
        }
    }

    #[tokio::test]
    async fn create_directory_makes_nested_dirs() {
        let root = tmp_root("mkdir");
        let tool = CreateDirectory { root: root.clone() };
        tool.call(json!({"path": "docs/api/v2"}), &ctx())
            .await
            .unwrap();
        assert!(root.join("docs/api/v2").is_dir());
    }

    // ----- 路径安全 -----

    #[tokio::test]
    async fn tools_reject_paths_outside_root() {
        let root = tmp_root("escape");
        let edit = EditFile { root: root.clone() };
        let err = edit
            .call(
                json!({"path": "../outside.md", "old_text": "a", "new_text": "b"}),
                &ctx(),
            )
            .await
            .unwrap_err();
        assert!(matches!(err, Error::PathEscape(_)));

        let mkdir = CreateDirectory { root };
        let err = mkdir
            .call(json!({"path": "../evil"}), &ctx())
            .await
            .unwrap_err();
        assert!(matches!(err, Error::PathEscape(_)));
    }

    // ----- 注册表 -----

    #[test]
    fn register_adds_all_six_tools() {
        let root = tmp_root("reg");
        let reg = register(R::new(), &root);
        let names = reg.names();
        for expected in [
            "search_files",
            "find_path",
            "edit_file",
            "copy_path",
            "move_path",
            "create_directory",
        ] {
            assert!(names.contains(&expected.to_string()), "缺工具 {expected}");
        }
    }
}
