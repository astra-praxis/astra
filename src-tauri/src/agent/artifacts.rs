use std::sync::Arc;

use serde::Serialize;
use serde_json::Value;
use tauri::{AppHandle, State};

use super::*;
// ===== Artifacts（Phase 4，ARTIFACT_SPEC：location-based 元数据登记）=====

/// 前端可见的 artifact 记录。registry 只证明「写入时产物存在于 location」，
/// 不保证文件现在仍在（spec §1：无内容快照/校验/回滚）。
#[derive(Debug, Serialize)]
pub struct AgentArtifactInfo {
    pub id: String,
    pub thread_id: String,
    /// 逻辑身份，版本链锚点（v1 = location）
    pub artifact_key: String,
    pub kind: String,
    /// 写入时刻的路径（相对项目根），不是存活声明
    pub location: String,
    pub preview: Option<String>,
    pub metadata: Value,
    pub version: u32,
    pub previous_id: Option<String>,
    pub created_at: u64,
    pub status: String,
}

pub(crate) fn artifact_info(
    record: agent_core::artifact_registry::ArtifactRecord,
) -> AgentArtifactInfo {
    AgentArtifactInfo {
        id: record.id,
        thread_id: record.thread_id,
        artifact_key: record.artifact_key,
        kind: serde_json::to_value(record.kind)
            .ok()
            .and_then(|v| v.as_str().map(str::to_owned))
            .unwrap_or_else(|| "generic".into()),
        location: record.location,
        preview: record.preview,
        metadata: record.metadata,
        version: record.version,
        previous_id: record.previous_id,
        created_at: record.created_at,
        status: serde_json::to_value(record.status)
            .ok()
            .and_then(|v| v.as_str().map(str::to_owned))
            .unwrap_or_else(|| "ready".into()),
    }
}

/// 粘贴落盘结果（spilled 恒为 true，保留字段位以防未来引入禁写场景）。
#[derive(serde::Serialize)]
pub struct SpillPasteResult {
    pub spilled: bool,
    pub path: Option<String>,
    pub size: u64,
}

/// 粘贴目录：与 agent_spill_paste 同一约定（有项目时 `<project>/.astra/pastes/`，
/// 纯聊天时 `~/.astra/pastes/`），并确保目录存在。
fn pastes_dir(project_root: &Option<String>) -> Result<std::path::PathBuf, String> {
    let dir = match project_root
        .as_deref()
        .map(std::path::Path::new)
        .filter(|p| p.is_dir())
    {
        Some(root) => root.join(".astra").join("pastes"),
        // 纯聊天：账户私有 astra home（legacy 模式回退 ~/.astra），
        // agent 的 read_file 以主目录为 root，需给绝对路径
        None => crate::app::accounts::astra_home().join("pastes"),
    };
    std::fs::create_dir_all(&dir).map_err(|e| format!("创建粘贴目录失败：{e}"))?;
    Ok(dir)
}

/// 把用户粘贴的超长内容写入 `.astra/pastes/paste-<时间戳>.txt`，
/// 发送时消息正文只带「先 read_file 读取该文件再处理指令」，避免超长文本撑爆
/// 输入框与上下文。应用侧功能：只负责写文件，不参与 Agent 循环。
#[tauri::command]
pub fn agent_spill_paste(
    project_root: Option<String>,
    content: String,
) -> Result<SpillPasteResult, String> {
    if content.is_empty() {
        return Err("粘贴内容为空".into());
    }
    let dir = pastes_dir(&project_root)?;
    let stamp = chrono::Local::now().format("%Y%m%d-%H%M%S%3f");
    let path = dir.join(format!("paste-{stamp}.txt"));
    std::fs::write(&path, &content).map_err(|e| format!("写入粘贴文件失败：{e}"))?;
    Ok(SpillPasteResult {
        spilled: true,
        path: Some(path.display().to_string()),
        size: content.len() as u64,
    })
}

/// 已导入的粘贴文件（前端渲染 chip，发送时拼 read_file 指令）。
#[derive(serde::Serialize)]
pub struct PastedFileInfo {
    /// 落盘后的绝对路径，agent 的 read_file 可直接读取
    pub path: String,
    /// 展示用文件名（含扩展名）
    pub name: String,
    pub size: u64,
}

/// 递归复制文件/目录，返回复制的字节数（目录只计文件内容）。
fn copy_path_recursive(src: &std::path::Path, dst: &std::path::Path) -> std::io::Result<u64> {
    if src.is_dir() {
        std::fs::create_dir_all(dst)?;
        let mut total = 0u64;
        for entry in std::fs::read_dir(src)? {
            let entry = entry?;
            total += copy_path_recursive(&entry.path(), &dst.join(entry.file_name()))?;
        }
        Ok(total)
    } else {
        std::fs::copy(src, dst)
    }
}

/// 清洗粘贴文件名：去除路径分隔符、首尾空白，超长时截断 stem 保留扩展名。
fn sanitize_paste_name(raw: &str) -> String {
    let mut safe: String = raw
        .chars()
        .map(|c| if c == '/' || c == '\\' { '_' } else { c })
        .collect();
    safe = safe.trim().to_string();
    if safe.is_empty() {
        safe = "file".into();
    }
    let (stem, ext) = match safe.rsplit_once('.') {
        Some((s, e)) if !s.is_empty() => (s.to_string(), format!(".{e}")),
        _ => (safe.clone(), String::new()),
    };
    let stem: String = stem.chars().take(60).collect();
    format!("{stem}{ext}")
}

/// 把系统剪贴板里的文件（Finder / 资源管理器复制的文件 URL）复制到粘贴目录，
/// 返回实际导入的文件列表。与 agent_spill_paste 同目录约定。
/// 剪贴板里没有文件 URL（纯文本 / 截图等图片数据）时返回空列表，
/// 前端降级为普通文本粘贴。
#[tauri::command]
pub fn agent_import_pasted_files(
    project_root: Option<String>,
) -> Result<Vec<PastedFileInfo>, String> {
    let sources = match clipboard_files::read() {
        Ok(list) => list,
        // 无文件 URL 属正常场景（文本/图片粘贴），不报错
        Err(_) => return Ok(vec![]),
    };
    if sources.is_empty() {
        return Ok(vec![]);
    }
    let dir = pastes_dir(&project_root)?;
    let stamp = chrono::Local::now().format("%Y%m%d-%H%M%S%3f");
    let multiple = sources.len() > 1;
    let mut imported = Vec::with_capacity(sources.len());
    for (idx, src) in sources.iter().enumerate() {
        let raw = src
            .file_name()
            .map(|n| n.to_string_lossy().to_string())
            .unwrap_or_else(|| "file".into());
        let safe = sanitize_paste_name(&raw);

        let prefix = if multiple {
            format!("paste-{stamp}-{idx}")
        } else {
            format!("paste-{stamp}")
        };
        let mut target = dir.join(format!("{prefix}-{safe}"));
        let mut n = 1;
        while target.exists() {
            target = dir.join(format!("{prefix}-{n}-{safe}"));
            n += 1;
        }

        let size = copy_path_recursive(src, &target)
            .map_err(|e| format!("复制粘贴文件失败（{}）：{e}", safe))?;
        imported.push(PastedFileInfo {
            path: target.display().to_string(),
            name: safe,
            size,
        });
    }
    Ok(imported)
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn sanitize_paste_name_strips_separators_and_trims() {
        assert_eq!(sanitize_paste_name("report.pdf"), "report.pdf");
        assert_eq!(sanitize_paste_name("a/b\\c.txt"), "a_b_c.txt");
        assert_eq!(sanitize_paste_name("  data.json "), "data.json");
        assert_eq!(sanitize_paste_name(""), "file");
        assert_eq!(sanitize_paste_name(".gitignore"), ".gitignore");
    }

    #[test]
    fn sanitize_paste_name_truncates_long_stem_keeps_ext() {
        let long = format!("{}.txt", "长".repeat(80));
        let safe = sanitize_paste_name(&long);
        assert!(safe.ends_with(".txt"));
        assert!(safe.chars().count() <= 64);
    }

    #[test]
    fn copy_path_recursive_handles_files_and_dirs() {
        let tmp = tempfile::tempdir().unwrap();
        let src_dir = tmp.path().join("src-dir");
        std::fs::create_dir_all(src_dir.join("nested")).unwrap();
        std::fs::write(src_dir.join("a.txt"), "hello").unwrap();
        std::fs::write(src_dir.join("nested/b.txt"), "world!").unwrap();

        let dst_dir = tmp.path().join("dst-dir");
        let size = copy_path_recursive(&src_dir, &dst_dir).unwrap();
        assert_eq!(size, 11);
        assert_eq!(
            std::fs::read_to_string(dst_dir.join("a.txt")).unwrap(),
            "hello"
        );
        assert_eq!(
            std::fs::read_to_string(dst_dir.join("nested/b.txt")).unwrap(),
            "world!"
        );

        let dst_file = tmp.path().join("dst-file.txt");
        let size = copy_path_recursive(&src_dir.join("a.txt"), &dst_file).unwrap();
        assert_eq!(size, 5);
        assert_eq!(std::fs::read_to_string(&dst_file).unwrap(), "hello");
    }
}

/// 列出一个会话的全部 artifact 记录（created_at 升序）。
/// 版本链由前端按 artifact_key 自行分组（registry 无分页，spec §4）。
#[tauri::command]
pub async fn agent_artifact_list(
    app: AppHandle,
    state: State<'_, AgentState>,
    session_id: String,
) -> Result<Vec<AgentArtifactInfo>, String> {
    let mut threads = state.threads.lock().await;
    let directory = crate::app::accounts::data_dir(&app)?;
    threads
        .open(&directory, Arc::clone(&state.persistence_errors))
        .await?;
    let artifacts = threads.artifacts()?;
    drop(threads);
    let records = artifacts
        .list(&session_id)
        .await
        .map_err(|e| e.to_string())?;
    Ok(records.into_iter().map(artifact_info).collect())
}
