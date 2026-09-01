use rusqlite::{params, Connection};
use serde::Serialize;
use std::collections::{HashMap, HashSet};
use std::fs;
use std::path::{Path, PathBuf};
use std::sync::atomic::{AtomicBool, Ordering};
use std::time::Instant;
use tauri::{AppHandle, Emitter, State};

use crate::embedding::{embed_text, EmbeddingSource};
use crate::vector_store::{open_db, store_chunks_conn};

// ===== 常量 =====

/// 分块大小（行数）
const CHUNK_SIZE: usize = 80;
/// 相邻分块重叠行数（保持上下文连续）
const CHUNK_OVERLAP: usize = 10;
/// 每次嵌入请求的文本数量（多数嵌入 API 的批量上限为 16 或 32）
const EMBED_BATCH_SIZE: usize = 16;
/// 跳过超过此大小的文件（字节），避免大文件/压缩产物干扰
const MAX_FILE_SIZE: u64 = 512 * 1024;

/// 忽略的目录（命中即跳过整个子树）
const IGNORED_DIRS: &[&str] = &[
    "node_modules",
    ".git",
    "dist",
    "build",
    "target",
    ".next",
    "out",
    "coverage",
    ".venv",
    "venv",
    "__pycache__",
    ".idea",
    ".vscode",
    ".svelte-kit",
    ".turbo",
    ".cache",
    "tmp",
    "temp",
    ".DS_Store",
    ".gradle",
    "Pods",
    ".husky",
];

/// 可索引的代码文件扩展名白名单
const CODE_EXTENSIONS: &[&str] = &[
    "rs", "ts", "tsx", "js", "jsx", "svelte", "vue", "py", "go", "java", "c", "h", "cpp", "hpp",
    "cs", "rb", "php", "swift", "kt", "sql", "json", "yaml", "yml", "toml", "xml", "html", "css",
    "scss", "md", "sh", "bash", "zig", "lua", "r", "scala", "dart",
];

// ===== 类型 =====

/// 全局取消标志（同时只能进行一个索引任务）
pub struct IndexCancelFlag(pub AtomicBool);

/// 索引进度事件（每个文件处理完后推送一次）
#[derive(Debug, Clone, Serialize)]
pub struct IndexProgress {
    pub file_path: String,
    /// 已处理的文件数
    pub done: usize,
    /// 总文件数
    pub total: usize,
    /// 本次索引新增的分块数
    pub indexed_chunks: usize,
    /// 跳过的文件数（无变化 / 不可读）
    pub skipped: usize,
}

/// 索引结果摘要
#[derive(Debug, Clone, Serialize)]
pub struct IndexSummary {
    /// 扫描到的候选文件数
    pub files_scanned: usize,
    /// 本次实际索引（内容有变化）的文件数
    pub files_indexed: usize,
    /// 跳过的文件数（内容未变化 / 读取失败 / 空文件）
    pub files_skipped: usize,
    /// 本次索引的新增分块数
    pub chunks_indexed: usize,
    /// 清理的失效文件数（磁盘上已删除的索引条目）
    pub stale_removed: usize,
    /// 耗时（毫秒）
    pub duration_ms: u64,
    /// 是否被取消
    pub cancelled: bool,
}

// ===== 文件扫描 =====

/// 递归扫描项目目录，返回可索引的代码文件列表（排序保证确定性）
fn scan_files(root: &Path) -> Result<Vec<PathBuf>, String> {
    let mut files = Vec::new();
    let mut stack = vec![root.to_path_buf()];

    while let Some(dir) = stack.pop() {
        let entries =
            fs::read_dir(&dir).map_err(|e| format!("读取目录失败 {}: {}", dir.display(), e))?;
        for entry in entries.flatten() {
            let path = entry.path();
            let name = entry.file_name().to_string_lossy().to_string();
            if path.is_dir() {
                // 跳过忽略目录和隐藏目录（如 .github、.idea 等）
                if !IGNORED_DIRS.contains(&name.as_str()) && !name.starts_with('.') {
                    stack.push(path);
                }
            } else if is_indexable_file(&path) {
                files.push(path);
            }
        }
    }

    files.sort();
    Ok(files)
}

/// 判断文件是否值得索引（扩展名白名单 + 大小限制）
fn is_indexable_file(path: &Path) -> bool {
    let ext = path
        .extension()
        .and_then(|e| e.to_str())
        .unwrap_or("")
        .to_lowercase();
    if !CODE_EXTENSIONS.contains(&ext.as_str()) {
        return false;
    }
    let size_ok = fs::metadata(path)
        .map(|m| m.len() <= MAX_FILE_SIZE)
        .unwrap_or(false);
    size_ok
}

// ===== 分块 =====

/// 按行分块，带重叠以保持上下文连续。
/// 空文件返回空列表。
fn chunk_content(content: &str, chunk_size: usize, overlap: usize) -> Vec<String> {
    let lines: Vec<&str> = content.lines().collect();
    if lines.is_empty() {
        return Vec::new();
    }

    let step = chunk_size.saturating_sub(overlap).max(1);
    let mut chunks = Vec::new();
    let mut start = 0;

    while start < lines.len() {
        let end = (start + chunk_size).min(lines.len());
        chunks.push(lines[start..end].join("\n"));
        if end == lines.len() {
            break;
        }
        start += step;
    }

    // 丢弃纯空白分块（如只有换行/空格的伪内容）
    chunks.retain(|c| !c.trim().is_empty());
    chunks
}

/// 计算文件内容哈希（用于变更检测，无需密码学强度）
fn hash_content(content: &str) -> String {
    use std::collections::hash_map::DefaultHasher;
    use std::hash::{Hash, Hasher};
    let mut hasher = DefaultHasher::new();
    content.hash(&mut hasher);
    format!("{:016x}", hasher.finish())
}

// ===== Tauri 命令 =====

/// 索引整个项目：扫描 → 分块 → 嵌入 → 存储。
/// 增量更新：内容未变化的文件自动跳过。
///
/// 事件：
/// - `index-progress`: payload = IndexProgress（每文件推送）
/// - `index-done`: payload = IndexSummary（结束）
/// - `index-error`: payload = 错误描述（String）
#[tauri::command]
pub async fn index_project(
    app: AppHandle,
    cancel_flag: State<'_, IndexCancelFlag>,
    project_path: String,
    source: EmbeddingSource,
) -> Result<IndexSummary, String> {
    cancel_flag.0.store(false, Ordering::SeqCst);
    let conn = open_db(&app)?;

    let app_clone = app.clone();
    index_project_inner(
        conn,
        &cancel_flag.0,
        &project_path,
        |texts| {
            let app = app_clone.clone();
            let source = source.clone();
            async move {
                let result = embed_text(app, source, texts).await?;
                Ok(result.embeddings)
            }
        },
        |progress| {
            let _ = app_clone.emit("index-progress", progress.clone());
        },
    )
    .await
    .map(|summary| {
        let _ = app.emit("index-done", summary.clone());
        summary
    })
}

/// 核心索引逻辑（嵌入器与进度回调可注入，便于单元测试）
///
/// - `embedder`: 文本列表 → 嵌入向量列表
/// - `on_progress`: 每个文件处理完后的回调（接收进度快照）
async fn index_project_inner<F, Fut, P>(
    conn: Connection,
    cancel_flag: &AtomicBool,
    project_path: &str,
    embedder: F,
    mut on_progress: P,
) -> Result<IndexSummary, String>
where
    F: Fn(Vec<String>) -> Fut,
    Fut: std::future::Future<Output = Result<Vec<Vec<f32>>, String>>,
    P: FnMut(&IndexProgress),
{
    let start = Instant::now();
    let project = Path::new(project_path);
    if !project.is_dir() {
        return Err(format!("项目路径不存在: {}", project_path));
    }

    // 1). 扫描文件
    let files = scan_files(project)?;
    let total = files.len();
    if total == 0 {
        return Err("没有找到可索引的代码文件（或项目为空）".to_string());
    }

    // 2). 加载已索引文件的哈希表（增量索引）
    let mut known_hashes: HashMap<String, String> = HashMap::new();
    {
        let mut stmt = conn
            .prepare("SELECT file_path, file_hash FROM indexed_files WHERE project_path = ?1")
            .map_err(|e| format!("读取索引状态失败: {}", e))?;
        let rows = stmt
            .query_map(params![project_path], |row| {
                Ok((row.get::<_, String>(0)?, row.get::<_, String>(1)?))
            })
            .map_err(|e| format!("读取索引状态失败: {}", e))?;
        for row in rows.flatten() {
            known_hashes.insert(row.0, row.1);
        }
    }

    let now = std::time::SystemTime::now()
        .duration_since(std::time::UNIX_EPOCH)
        .unwrap_or_default()
        .as_secs();

    // 3). 逐文件索引
    let mut files_indexed = 0;
    let mut files_skipped = 0;
    let mut chunks_indexed = 0;
    let mut done = 0;
    let mut cancelled = false;

    for file in &files {
        if cancel_flag.load(Ordering::SeqCst) {
            cancelled = true;
            break;
        }
        done += 1;

        let rel_path = file
            .strip_prefix(project)
            .map(|p| p.to_string_lossy().to_string())
            .unwrap_or_default();

        // 读取 + 计算哈希
        let Ok(content) = fs::read_to_string(file) else {
            files_skipped += 1;
            on_progress(&IndexProgress {
                file_path: rel_path.clone(),
                done,
                total,
                indexed_chunks: 0,
                skipped: files_skipped,
            });
            continue;
        };
        let hash = hash_content(&content);

        // 内容未变化 → 跳过
        if known_hashes.get(&rel_path) == Some(&hash) {
            files_skipped += 1;
            on_progress(&IndexProgress {
                file_path: rel_path.clone(),
                done,
                total,
                indexed_chunks: 0,
                skipped: files_skipped,
            });
            continue;
        }

        // 分块
        let raw_chunks = chunk_content(&content, CHUNK_SIZE, CHUNK_OVERLAP);
        if raw_chunks.is_empty() {
            files_skipped += 1;
            on_progress(&IndexProgress {
                file_path: rel_path.clone(),
                done,
                total,
                indexed_chunks: 0,
                skipped: files_skipped,
            });
            continue;
        }

        // 批量嵌入
        let mut embeddings: Vec<Vec<f32>> = Vec::with_capacity(raw_chunks.len());
        for batch in raw_chunks.chunks(EMBED_BATCH_SIZE) {
            if cancel_flag.load(Ordering::SeqCst) {
                cancelled = true;
                break;
            }
            let texts: Vec<String> = batch.to_vec();
            embeddings.extend(embedder(texts).await?);
        }
        if cancelled {
            break;
        }

        // 组装 (chunk_index, content, embedding)
        let chunk_records: Vec<(usize, String, Vec<f32>)> = raw_chunks
            .into_iter()
            .zip(embeddings)
            .enumerate()
            .map(|(i, (content, emb))| (i, content, emb))
            .collect();

        // 存储分块
        store_chunks_conn(&conn, project_path, &rel_path, &chunk_records)?;

        // 记录文件哈希（INSERT OR REPLACE 支持原子更新）
        conn.execute(
            "INSERT OR REPLACE INTO indexed_files
                (project_path, file_path, file_hash, updated_at)
             VALUES (?1, ?2, ?3, ?4)",
            params![project_path, rel_path, hash, now],
        )
        .map_err(|e| format!("记录索引状态失败: {}", e))?;

        files_indexed += 1;
        chunks_indexed += chunk_records.len();
        on_progress(&IndexProgress {
            file_path: rel_path,
            done,
            total,
            indexed_chunks: chunk_records.len(),
            skipped: files_skipped,
        });
    }

    // 4). 清理失效索引（磁盘上已删除的文件）
    let mut stale_removed = 0;
    if !cancelled {
        let current_files: HashSet<String> = files
            .iter()
            .filter_map(|f| {
                f.strip_prefix(project)
                    .ok()
                    .map(|p| p.to_string_lossy().to_string())
            })
            .collect();

        let indexed_files: Vec<String> = {
            let mut stmt = conn
                .prepare("SELECT file_path FROM indexed_files WHERE project_path = ?1")
                .map_err(|e| format!("读取索引状态失败: {}", e))?;
            let rows = stmt
                .query_map(params![project_path], |row| row.get::<_, String>(0))
                .map_err(|e| format!("读取索引状态失败: {}", e))?;
            let mut out = Vec::new();
            for row in rows.flatten() {
                out.push(row);
            }
            out
        };

        for file_path in indexed_files {
            if !current_files.contains(&file_path) {
                conn.execute(
                    "DELETE FROM project_chunks WHERE project_path = ?1 AND file_path = ?2",
                    params![project_path, file_path],
                )
                .map_err(|e| format!("清理失效分块失败: {}", e))?;
                conn.execute(
                    "DELETE FROM indexed_files WHERE project_path = ?1 AND file_path = ?2",
                    params![project_path, file_path],
                )
                .map_err(|e| format!("清理失效索引失败: {}", e))?;
                stale_removed += 1;
            }
        }
    }

    Ok(IndexSummary {
        files_scanned: total,
        files_indexed,
        files_skipped,
        chunks_indexed,
        stale_removed,
        duration_ms: start.elapsed().as_millis() as u64,
        cancelled,
    })
}

/// 取消当前索引任务
#[tauri::command]
pub fn cancel_index(cancel_flag: State<'_, IndexCancelFlag>) {
    cancel_flag.0.store(true, Ordering::SeqCst);
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn test_chunk_content_basic() {
        let content = (0..200)
            .map(|i| format!("line {}", i))
            .collect::<Vec<_>>()
            .join("\n");
        let chunks = chunk_content(&content, 80, 10);
        assert!(chunks.len() >= 2);
        // 每个分块约 80 行
        assert_eq!(chunks[0].lines().count(), 80);
        assert_eq!(chunks[1].lines().count(), 80);
        // 重叠：chunk2 的开头与 chunk1 的结尾有交集
        assert_eq!(chunks[1].lines().next().unwrap(), "line 70");
    }

    #[test]
    fn test_chunk_content_small_file() {
        let content = "a\nb\nc";
        let chunks = chunk_content(content, 80, 10);
        assert_eq!(chunks.len(), 1);
        assert_eq!(chunks[0], "a\nb\nc");
    }

    #[test]
    fn test_chunk_content_empty() {
        assert!(chunk_content("", 80, 10).is_empty());
        assert!(chunk_content("\n\n", 80, 10).is_empty());
    }

    #[test]
    fn test_chunk_content_single_line() {
        let chunks = chunk_content("just one line", 80, 10);
        assert_eq!(chunks.len(), 1);
        assert_eq!(chunks[0], "just one line");
    }

    #[test]
    fn test_hash_content_stable() {
        assert_eq!(hash_content("abc"), hash_content("abc"));
        assert_ne!(hash_content("abc"), hash_content("abd"));
    }

    #[test]
    fn test_is_indexable_file() {
        // 构造真实存在的测试文件（is_indexable_file 会检查文件大小）
        let dir = std::env::temp_dir().join(format!("indexable_test_{}", std::process::id()));
        let _ = fs::remove_dir_all(&dir);
        fs::create_dir_all(&dir).unwrap();
        for name in [
            "a.rs", "a.ts", "a.svelte", "A.RS", "a.exe", "a.png", "Makefile",
        ] {
            fs::write(dir.join(name), "content").unwrap();
        }

        // 扩展名白名单
        assert!(is_indexable_file(&dir.join("a.rs")));
        assert!(is_indexable_file(&dir.join("a.ts")));
        assert!(is_indexable_file(&dir.join("a.svelte")));
        assert!(is_indexable_file(&dir.join("A.RS"))); // 大小写不敏感
                                                       // 非白名单
        assert!(!is_indexable_file(&dir.join("a.exe")));
        assert!(!is_indexable_file(&dir.join("a.png")));
        assert!(!is_indexable_file(&dir.join("Makefile"))); // 无扩展名
                                                            // 不存在的文件
        assert!(!is_indexable_file(&dir.join("missing.rs")));

        let _ = fs::remove_dir_all(&dir);
    }

    #[test]
    fn test_scan_files_ignores_dirs() {
        // 在临时目录构造一个模拟项目
        let dir = std::env::temp_dir().join(format!("scan_test_{}", std::process::id()));
        let _ = fs::remove_dir_all(&dir);
        fs::create_dir_all(dir.join("src")).unwrap();
        fs::create_dir_all(dir.join("node_modules/pkg")).unwrap();
        fs::create_dir_all(dir.join(".git")).unwrap();
        fs::create_dir_all(dir.join(".hidden")).unwrap();
        fs::write(dir.join("src/main.rs"), "fn main() {}").unwrap();
        fs::write(dir.join("src/lib.ts"), "export const x = 1;").unwrap();
        fs::write(dir.join("node_modules/pkg/index.js"), "ignored").unwrap();
        fs::write(dir.join(".git/config"), "ignored").unwrap();
        fs::write(dir.join(".hidden/s.txt"), "ignored").unwrap();
        fs::write(dir.join("README.md"), "hello").unwrap();
        fs::write(dir.join("image.png"), "binary").unwrap();

        let files = scan_files(&dir).unwrap();
        let names: Vec<String> = files
            .iter()
            .map(|f| f.strip_prefix(&dir).unwrap().to_string_lossy().to_string())
            .collect();

        assert!(names.contains(&"src/main.rs".to_string()));
        assert!(names.contains(&"src/lib.ts".to_string()));
        assert!(names.contains(&"README.md".to_string()));
        assert!(!names.iter().any(|n| n.contains("node_modules")));
        assert!(!names.iter().any(|n| n.contains(".git")));
        assert!(!names.iter().any(|n| n.contains(".hidden")));
        assert!(!names.contains(&"image.png".to_string()));

        let _ = fs::remove_dir_all(&dir);
    }

    // ===== 端到端：扫描 → 分块 → 嵌入 → 存储 → 增量 =====

    /// 创建唯一的临时数据库路径（已初始化表结构）
    fn temp_db_path() -> PathBuf {
        use std::sync::atomic::{AtomicU64, Ordering};
        static COUNTER: AtomicU64 = AtomicU64::new(0);
        let id = COUNTER.fetch_add(1, Ordering::SeqCst);
        let dir = std::env::temp_dir().join(format!("idx_test_{}_{}", std::process::id(), id));
        let _ = fs::create_dir_all(&dir);
        let path = dir.join("test.db");
        let _ = fs::remove_file(&path);
        crate::vector_store::open_db_at(&path).unwrap();
        path
    }

    /// 创建唯一的临时项目目录
    fn temp_project() -> PathBuf {
        use std::sync::atomic::{AtomicU64, Ordering};
        static COUNTER: AtomicU64 = AtomicU64::new(0);
        let id = COUNTER.fetch_add(1, Ordering::SeqCst);
        let dir = std::env::temp_dir().join(format!("proj_test_{}_{}", std::process::id(), id));
        let _ = fs::remove_dir_all(&dir);
        fs::create_dir_all(dir.join("src")).unwrap();
        dir
    }

    /// 假嵌入器：文本 → 确定性向量（长度 + 维度 3）
    fn fake_embedder(
        texts: Vec<String>,
    ) -> impl std::future::Future<Output = Result<Vec<Vec<f32>>, String>> {
        async move {
            Ok(texts
                .into_iter()
                .map(|t| vec![t.len() as f32, 1.0, 0.0])
                .collect())
        }
    }

    #[test]
    fn test_index_project_e2e_full_flow() {
        let db_path = temp_db_path();
        let project = temp_project();
        let flag = AtomicBool::new(false);

        fs::write(project.join("src/main.rs"), "fn main() {}\n").unwrap();
        fs::write(project.join("src/lib.ts"), "export const x = 1;\n").unwrap();
        fs::write(project.join("README.md"), "# My Project\n").unwrap();
        fs::create_dir_all(project.join("node_modules/x")).unwrap();
        fs::write(project.join("node_modules/x/index.js"), "ignored\n").unwrap();

        let proj_path = project.to_string_lossy().to_string();
        let rt = tokio::runtime::Runtime::new().unwrap();

        let mut progress_events = Vec::new();
        let summary = rt
            .block_on(index_project_inner(
                crate::vector_store::open_db_at(&db_path).unwrap(),
                &flag,
                &proj_path,
                fake_embedder,
                |p| progress_events.push(p.clone()),
            ))
            .unwrap();

        assert_eq!(summary.files_scanned, 3); // 排除 node_modules
        assert_eq!(summary.files_indexed, 3);
        assert_eq!(summary.files_skipped, 0);
        assert!(summary.chunks_indexed >= 3);
        assert!(!summary.cancelled);
        // 进度事件数量 = 文件数
        assert_eq!(progress_events.len(), 3);

        // 存储后可检索（重新打开连接）
        let conn = crate::vector_store::open_db_at(&db_path).unwrap();
        let results =
            crate::vector_store::search_similar_conn(&conn, &proj_path, &[10.0, 1.0, 0.0], 10)
                .unwrap();
        assert!(!results.is_empty());
        assert_eq!(results[0].project_path, proj_path);

        // 清理
        let _ = fs::remove_dir_all(&project);
    }

    #[test]
    fn test_index_project_incremental_skips_unchanged() {
        let db_path = temp_db_path();
        let project = temp_project();
        let flag = AtomicBool::new(false);

        fs::write(project.join("app.rs"), "let a = 1;\n").unwrap();
        let proj_path = project.to_string_lossy().to_string();
        let rt = tokio::runtime::Runtime::new().unwrap();
        let noop = |_: &IndexProgress| {};

        // 第一次：全部索引
        let s1 = rt
            .block_on(index_project_inner(
                crate::vector_store::open_db_at(&db_path).unwrap(),
                &flag,
                &proj_path,
                fake_embedder,
                noop,
            ))
            .unwrap();
        assert_eq!(s1.files_indexed, 1);

        // 第二次：内容未变 → 跳过
        let s2 = rt
            .block_on(index_project_inner(
                crate::vector_store::open_db_at(&db_path).unwrap(),
                &flag,
                &proj_path,
                fake_embedder,
                noop,
            ))
            .unwrap();
        assert_eq!(s2.files_indexed, 0);
        assert_eq!(s2.files_skipped, 1);

        // 修改文件后：重新索引
        fs::write(project.join("app.rs"), "let a = 2;\n").unwrap();
        let s3 = rt
            .block_on(index_project_inner(
                crate::vector_store::open_db_at(&db_path).unwrap(),
                &flag,
                &proj_path,
                fake_embedder,
                noop,
            ))
            .unwrap();
        assert_eq!(s3.files_indexed, 1);

        let _ = fs::remove_dir_all(&project);
    }

    #[test]
    fn test_index_project_cleans_stale_files() {
        let db_path = temp_db_path();
        let project = temp_project();
        let flag = AtomicBool::new(false);

        fs::write(project.join("a.rs"), "fn a() {}\n").unwrap();
        fs::write(project.join("b.ts"), "const b = 1;\n").unwrap();
        let proj_path = project.to_string_lossy().to_string();
        let rt = tokio::runtime::Runtime::new().unwrap();
        let noop = |_: &IndexProgress| {};

        rt.block_on(index_project_inner(
            crate::vector_store::open_db_at(&db_path).unwrap(),
            &flag,
            &proj_path,
            fake_embedder,
            noop,
        ))
        .unwrap();

        // 删除 b.ts 后重新索引 → 应清理失效条目
        fs::remove_file(project.join("b.ts")).unwrap();
        let s2 = rt
            .block_on(index_project_inner(
                crate::vector_store::open_db_at(&db_path).unwrap(),
                &flag,
                &proj_path,
                fake_embedder,
                noop,
            ))
            .unwrap();
        assert_eq!(s2.stale_removed, 1);

        // 数据库中只剩 a.rs 的分块
        let conn = crate::vector_store::open_db_at(&db_path).unwrap();
        let results =
            crate::vector_store::search_similar_conn(&conn, &proj_path, &[1.0, 1.0, 0.0], 50)
                .unwrap();
        assert!(results.iter().all(|r| r.file_path == "a.rs"));

        let _ = fs::remove_dir_all(&project);
    }

    #[test]
    fn test_index_project_cancel() {
        let db_path = temp_db_path();
        let project = temp_project();
        let flag = AtomicBool::new(true); // 预先置为取消状态

        fs::write(project.join("a.rs"), "fn a() {}\n").unwrap();
        let proj_path = project.to_string_lossy().to_string();
        let rt = tokio::runtime::Runtime::new().unwrap();
        let noop = |_: &IndexProgress| {};

        let summary = rt
            .block_on(index_project_inner(
                crate::vector_store::open_db_at(&db_path).unwrap(),
                &flag,
                &proj_path,
                fake_embedder,
                noop,
            ))
            .unwrap();
        assert!(summary.cancelled);
        assert_eq!(summary.files_indexed, 0);

        let _ = fs::remove_dir_all(&project);
    }
}
