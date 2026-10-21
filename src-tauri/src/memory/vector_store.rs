use rusqlite::{params, Connection};
use serde::{Deserialize, Serialize};
use std::fs;
use std::path::PathBuf;
use tauri::{AppHandle, Manager};

// ===== 类型 =====

/// 检索结果
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct SearchResult {
    pub project_path: String,
    pub file_path: String,
    pub chunk_index: usize,
    pub content: String,
    /// 相似度分数（0.0 ~ 1.0，越大越相似）
    pub score: f32,
}

/// 向量索引统计
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct IndexStats {
    pub total_chunks: usize,
    pub total_projects: usize,
    pub total_files: usize,
    pub projects: Vec<ProjectStats>,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct ProjectStats {
    pub project_path: String,
    pub chunks: usize,
    pub files: usize,
}

// ===== 数据库管理 =====

/// 获取向量数据库路径（app_data_dir/vector_index.db）
fn get_db_path(app: &AppHandle) -> Result<PathBuf, String> {
    let app_data_dir = app
        .path()
        .app_data_dir()
        .map_err(|e| format!("获取应用数据目录失败: {}", e))?;
    fs::create_dir_all(&app_data_dir).map_err(|e| format!("创建配置目录失败: {}", e))?;
    Ok(app_data_dir.join("vector_index.db"))
}

/// 在指定路径打开向量数据库连接（自动建表）
pub fn open_db_at(db_path: &std::path::Path) -> Result<Connection, String> {
    if let Some(parent) = db_path.parent() {
        fs::create_dir_all(parent).map_err(|e| format!("创建目录失败: {}", e))?;
    }
    let conn = Connection::open(db_path).map_err(|e| format!("打开向量数据库失败: {}", e))?;

    // 初始化表结构
    conn.execute_batch(
        "
        CREATE TABLE IF NOT EXISTS project_chunks (
            id INTEGER PRIMARY KEY AUTOINCREMENT,
            project_path TEXT NOT NULL,
            file_path TEXT NOT NULL,
            chunk_index INTEGER NOT NULL,
            content TEXT NOT NULL,
            embedding TEXT NOT NULL,
            updated_at INTEGER NOT NULL
        );
        CREATE INDEX IF NOT EXISTS idx_chunks_project ON project_chunks(project_path);
        CREATE INDEX IF NOT EXISTS idx_chunks_file ON project_chunks(project_path, file_path);
        CREATE UNIQUE INDEX IF NOT EXISTS idx_chunks_unique
            ON project_chunks(project_path, file_path, chunk_index);

        -- 已索引文件哈希表（增量索引：文件内容未变则跳过）
        CREATE TABLE IF NOT EXISTS indexed_files (
            project_path TEXT NOT NULL,
            file_path TEXT NOT NULL,
            file_hash TEXT NOT NULL,
            updated_at INTEGER NOT NULL,
            PRIMARY KEY (project_path, file_path)
        );
        ",
    )
    .map_err(|e| format!("初始化向量数据库失败: {}", e))?;

    Ok(conn)
}

/// 打开向量数据库连接（应用数据目录）
pub fn open_db(app: &AppHandle) -> Result<Connection, String> {
    let db_path = get_db_path(app)?;
    open_db_at(&db_path)
}

// ===== 写入 =====

/// 存储一批代码分块及其向量（核心逻辑，接受连接）
///
/// 同一项目同一文件下的旧分块会被替换（增量更新）。
pub fn store_chunks_conn(
    conn: &Connection,
    project_path: &str,
    file_path: &str,
    chunks: &[(usize, String, Vec<f32>)],
) -> Result<usize, String> {
    let now = std::time::SystemTime::now()
        .duration_since(std::time::UNIX_EPOCH)
        .unwrap_or_default()
        .as_secs();

    let tx = conn
        .unchecked_transaction()
        .map_err(|e| format!("开启事务失败: {}", e))?;

    // 删除该文件的旧分块
    tx.execute(
        "DELETE FROM project_chunks WHERE project_path = ?1 AND file_path = ?2",
        params![project_path, file_path],
    )
    .map_err(|e| format!("删除旧分块失败: {}", e))?;

    // 插入新分块
    for (chunk_index, content, embedding) in chunks {
        let embedding_json =
            serde_json::to_string(embedding).map_err(|e| format!("序列化向量失败: {}", e))?;

        tx.execute(
            "INSERT INTO project_chunks
                (project_path, file_path, chunk_index, content, embedding, updated_at)
             VALUES (?1, ?2, ?3, ?4, ?5, ?6)",
            params![
                project_path,
                file_path,
                *chunk_index,
                content,
                embedding_json,
                now
            ],
        )
        .map_err(|e| format!("插入分块失败: {}", e))?;
    }

    tx.commit().map_err(|e| format!("提交事务失败: {}", e))?;

    Ok(chunks.len())
}

/// 存储一批代码分块及其向量（Tauri 命令包装）
///
/// 参数：
/// - project_path: 项目根路径
/// - file_path: 相对文件路径
/// - chunks: (分块序号, 分块内容, 向量) 列表
/// - model_name: 使用的嵌入模型名称（保留字段，当前未使用）
#[tauri::command]
pub fn store_chunks(
    app: AppHandle,
    project_path: String,
    file_path: String,
    chunks: Vec<(usize, String, Vec<f32>)>,
    model_name: String,
) -> Result<usize, String> {
    let _ = model_name; // 模型名目前仅用于日志/未来扩展
    let conn = open_db(&app)?;
    store_chunks_conn(&conn, &project_path, &file_path, &chunks)
}

/// 删除整个项目的索引（重新索引时使用）
#[tauri::command]
pub fn delete_project_index(app: AppHandle, project_path: String) -> Result<usize, String> {
    let conn = open_db(&app)?;
    let count = conn
        .execute(
            "DELETE FROM project_chunks WHERE project_path = ?1",
            params![project_path],
        )
        .map_err(|e| format!("删除项目索引失败: {}", e))?;
    Ok(count)
}

// ===== 检索 =====

/// 相似度检索（核心逻辑，接受连接）
///
/// 在指定项目的向量索引中，查找与查询向量最相似的 Top-K 分块。
pub fn search_similar_conn(
    conn: &Connection,
    project_path: &str,
    query_embedding: &[f32],
    top_k: usize,
) -> Result<Vec<SearchResult>, String> {
    let top_k = top_k.max(1).min(50);

    let mut stmt = conn
        .prepare(
            "SELECT project_path, file_path, chunk_index, content, embedding
             FROM project_chunks
             WHERE project_path = ?1",
        )
        .map_err(|e| format!("查询失败: {}", e))?;

    let rows = stmt
        .query_map(params![project_path], |row| {
            Ok((
                row.get::<_, String>(0)?,
                row.get::<_, String>(1)?,
                row.get::<_, i64>(2)?,
                row.get::<_, String>(3)?,
                row.get::<_, String>(4)?,
            ))
        })
        .map_err(|e| format!("读取检索结果失败: {}", e))?;

    // 计算每个分块的相似度
    let mut results: Vec<(f32, String, String, i64, String)> = Vec::new();
    for row in rows {
        if let Ok((project, file, idx, content, emb_json)) = row {
            if let Ok(embedding) = serde_json::from_str::<Vec<f32>>(&emb_json) {
                let score = cosine_similarity(query_embedding, &embedding);
                results.push((score, project, file, idx, content));
            }
        }
    }

    // 按相似度降序
    results.sort_by(|a, b| b.0.partial_cmp(&a.0).unwrap_or(std::cmp::Ordering::Equal));

    // 取 Top-K
    let results: Vec<SearchResult> = results
        .into_iter()
        .take(top_k)
        .map(|(score, project, file, idx, content)| SearchResult {
            project_path: project,
            file_path: file,
            chunk_index: idx as usize,
            content,
            score,
        })
        .collect();

    Ok(results)
}

/// 相似度检索（Tauri 命令包装）
///
/// 参数：
/// - project_path: 项目根路径
/// - query_embedding: 查询文本的嵌入向量
/// - top_k: 返回数量
///
/// 返回按相似度降序排列的分块列表。
#[tauri::command]
pub fn search_similar(
    app: AppHandle,
    project_path: String,
    query_embedding: Vec<f32>,
    top_k: usize,
) -> Result<Vec<SearchResult>, String> {
    let conn = open_db(&app)?;
    search_similar_conn(&conn, &project_path, &query_embedding, top_k)
}

// ===== 统计 =====

/// 获取向量索引统计信息
#[tauri::command]
pub fn get_index_stats(app: AppHandle) -> Result<IndexStats, String> {
    let conn = open_db(&app)?;

    let total_chunks: i64 = conn
        .query_row("SELECT COUNT(*) FROM project_chunks", [], |row| row.get(0))
        .unwrap_or(0);

    let total_files: i64 = conn
        .query_row(
            "SELECT COUNT(DISTINCT project_path || '|' || file_path) FROM project_chunks",
            [],
            |row| row.get(0),
        )
        .unwrap_or(0);

    let mut stmt = conn
        .prepare(
            "SELECT project_path, COUNT(*) as chunks, COUNT(DISTINCT file_path) as files
             FROM project_chunks
             GROUP BY project_path",
        )
        .map_err(|e| format!("查询统计失败: {}", e))?;

    let mut projects: Vec<ProjectStats> = Vec::new();
    let rows = stmt
        .query_map([], |row| {
            Ok((
                row.get::<_, String>(0)?,
                row.get::<_, i64>(1)?,
                row.get::<_, i64>(2)?,
            ))
        })
        .map_err(|e| format!("读取统计失败: {}", e))?;

    for row in rows {
        if let Ok((project_path, chunks, files)) = row {
            projects.push(ProjectStats {
                project_path,
                chunks: chunks as usize,
                files: files as usize,
            });
        }
    }

    Ok(IndexStats {
        total_chunks: total_chunks as usize,
        total_projects: projects.len(),
        total_files: total_files as usize,
        projects,
    })
}

// ===== 数学工具 =====

/// 计算两个向量的余弦相似度（0.0 ~ 1.0）
pub fn cosine_similarity(a: &[f32], b: &[f32]) -> f32 {
    if a.len() != b.len() || a.is_empty() {
        return 0.0;
    }

    let mut dot = 0.0f64;
    let mut norm_a = 0.0f64;
    let mut norm_b = 0.0f64;

    for i in 0..a.len() {
        let va = a[i] as f64;
        let vb = b[i] as f64;
        dot += va * vb;
        norm_a += va * va;
        norm_b += vb * vb;
    }

    if norm_a == 0.0 || norm_b == 0.0 {
        return 0.0;
    }

    (dot / (norm_a.sqrt() * norm_b.sqrt())) as f32
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn test_cosine_similarity_identical() {
        let v = vec![1.0, 2.0, 3.0];
        assert!((cosine_similarity(&v, &v) - 1.0).abs() < 1e-6);
    }

    #[test]
    fn test_cosine_similarity_orthogonal() {
        let a = vec![1.0, 0.0];
        let b = vec![0.0, 1.0];
        assert!((cosine_similarity(&a, &b) - 0.0).abs() < 1e-6);
    }

    #[test]
    fn test_cosine_similarity_opposite() {
        let a = vec![1.0, 0.0];
        let b = vec![-1.0, 0.0];
        assert!((cosine_similarity(&a, &b) + 1.0).abs() < 1e-6);
    }

    #[test]
    fn test_cosine_similarity_dimension_mismatch() {
        assert_eq!(cosine_similarity(&[1.0], &[1.0, 2.0]), 0.0);
        assert_eq!(cosine_similarity(&[], &[]), 0.0);
    }

    #[test]
    fn test_cosine_similarity_partial() {
        let a = vec![1.0, 0.0, 0.0];
        let b = vec![1.0, 1.0, 0.0];
        let expected = 1.0 / 2.0_f32.sqrt();
        assert!((cosine_similarity(&a, &b) - expected).abs() < 1e-6);
    }

    // ===== 端到端：存储 → 检索 =====

    /// 创建唯一的临时数据库连接（避免并行测试互相干扰）
    fn temp_db() -> Connection {
        use std::sync::atomic::{AtomicU64, Ordering};
        static COUNTER: AtomicU64 = AtomicU64::new(0);
        let id = COUNTER.fetch_add(1, Ordering::SeqCst);
        let dir = std::env::temp_dir().join(format!("vec_test_{}_{}", std::process::id(), id));
        let _ = std::fs::create_dir_all(&dir);
        let path = dir.join("test.db");
        let _ = std::fs::remove_file(&path);
        open_db_at(&path).unwrap()
    }

    #[test]
    fn test_store_and_search_roundtrip() {
        let conn = temp_db();

        // 存入两个文件的多个分块
        let chunks_a = vec![
            (0, "fn add(a, b) { a + b }".to_string(), vec![1.0, 0.0, 0.0]),
            (
                1,
                "fn multiply(a, b) { a * b }".to_string(),
                vec![0.0, 1.0, 0.0],
            ),
        ];
        let chunks_b = vec![(
            0,
            "fn greet(name) { name }".to_string(),
            vec![0.0, 0.0, 1.0],
        )];

        store_chunks_conn(&conn, "/proj", "math.rs", &chunks_a).unwrap();
        store_chunks_conn(&conn, "/proj", "greet.rs", &chunks_b).unwrap();

        // 查询向量与 math.rs 的 add 分块最接近
        let results = search_similar_conn(&conn, "/proj", &[0.9, 0.1, 0.0], 5).unwrap();
        assert_eq!(results.len(), 3);
        assert_eq!(results[0].file_path, "math.rs");
        assert_eq!(results[0].chunk_index, 0);
        assert!(results[0].score > 0.9);

        // 查询向量与 greet 分块最接近
        let results = search_similar_conn(&conn, "/proj", &[0.1, 0.1, 0.95], 1).unwrap();
        assert_eq!(results.len(), 1);
        assert_eq!(results[0].file_path, "greet.rs");
    }

    #[test]
    fn test_store_replaces_old_chunks() {
        let conn = temp_db();

        let old = vec![(0, "v1 content".to_string(), vec![1.0, 0.0])];
        let new = vec![
            (0, "v2 content".to_string(), vec![1.0, 0.0]),
            (1, "v2 second chunk".to_string(), vec![0.0, 1.0]),
        ];

        store_chunks_conn(&conn, "/proj", "app.rs", &old).unwrap();
        store_chunks_conn(&conn, "/proj", "app.rs", &new).unwrap();

        let results = search_similar_conn(&conn, "/proj", &[1.0, 0.0], 10).unwrap();
        assert_eq!(results.len(), 2);
        assert!(results.iter().all(|r| r.content.starts_with("v2")));
    }

    #[test]
    fn test_search_isolated_by_project() {
        let conn = temp_db();

        store_chunks_conn(
            &conn,
            "/proj_a",
            "a.rs",
            &[(0, "alpha".to_string(), vec![1.0, 0.0])],
        )
        .unwrap();
        store_chunks_conn(
            &conn,
            "/proj_b",
            "b.rs",
            &[(0, "beta".to_string(), vec![1.0, 0.0])],
        )
        .unwrap();

        // 只检索 proj_a
        let results = search_similar_conn(&conn, "/proj_a", &[1.0, 0.0], 10).unwrap();
        assert_eq!(results.len(), 1);
        assert_eq!(results[0].project_path, "/proj_a");

        // 不存在的项目返回空
        let results = search_similar_conn(&conn, "/nonexistent", &[1.0, 0.0], 10).unwrap();
        assert!(results.is_empty());
    }

    #[test]
    fn test_search_top_k_limits() {
        let conn = temp_db();

        let chunks: Vec<(usize, String, Vec<f32>)> = (0..10)
            .map(|i| (i, format!("chunk {}", i), vec![i as f32, 0.0]))
            .collect();
        store_chunks_conn(&conn, "/proj", "big.rs", &chunks).unwrap();

        let results = search_similar_conn(&conn, "/proj", &[5.0, 0.0], 3).unwrap();
        assert_eq!(results.len(), 3);
        // 相似度最高的应该是 chunk 5、6、7 附近（降序）
        assert!(results[0].score >= results[1].score);
        assert!(results[1].score >= results[2].score);
    }
}
