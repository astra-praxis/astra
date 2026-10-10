//! 用户技能库：app_data_dir/skills/<技能名>/ 目录管理。
//!
//! 与 ~/.agents/skills（全局 Git 技能）不同，这里是应用自有的技能库，
//! 供「我的 Skills」页面导入/新建/多文件编辑，并供 Agent 通过
//! list_skills / read_skill 工具发现与读取、通过 /技能名 直接调用。
//!
//! 分层：核心逻辑全部基于目录路径（可单测），#[tauri::command] 只做薄包装。
//! 路径安全：技能名与文件相对路径均校验，读取走 agent_core::ensure_within_root
//! （canonicalize 含符号链接逃逸拦截）。

use serde::Serialize;
use std::fs;
use std::path::{Path, PathBuf};
use tauri::AppHandle;

/// 单个技能文件读取上限（字符数），超过截断并提示
const SKILL_READ_MAX_CHARS: usize = 30_000;
/// 目录递归统计/列举的条目上限（防异常大目录拖垮 UI）
const WALK_MAX_ENTRIES: usize = 1000;

#[derive(Debug, Serialize)]
pub struct UserSkillMeta {
    /// 规范名 = 目录名（/调用 与 read_skill 都用这个名字）
    pub name: String,
    pub description: String,
    pub path: String,
    pub file_count: usize,
}

#[derive(Debug, Serialize)]
pub struct UserSkillFile {
    pub rel_path: String,
    pub size: u64,
}

/// Agent 侧的轻量摘要（注入 system prompt / 前端 / 弹窗）
#[derive(Debug, Clone)]
pub struct SkillSummary {
    pub name: String,
    pub description: String,
}

// ===== 校验 =====

/// 技能名（目录名）校验：非空、不以 . 开头、不含路径分隔符与 ..
pub fn validate_skill_name(name: &str) -> Result<(), String> {
    let name = name.trim();
    if name.is_empty() {
        return Err("技能名不能为空".into());
    }
    if name.starts_with('.') {
        return Err("技能名不能以 . 开头".into());
    }
    if name == "." || name == ".." {
        return Err("技能名非法".into());
    }
    if name.contains('/') || name.contains('\\') {
        return Err("技能名不能包含路径分隔符".into());
    }
    Ok(())
}

/// 技能内文件相对路径校验：拒绝绝对路径、.. 分量、\ 分隔符
pub fn validate_rel_path(rel: &str) -> Result<(), String> {
    if rel.trim().is_empty() {
        return Err("文件路径不能为空".into());
    }
    if rel.starts_with('/') {
        return Err("文件路径必须是相对路径".into());
    }
    if rel.contains('\\') {
        return Err("文件路径请使用 / 分隔".into());
    }
    if Path::new(rel)
        .components()
        .any(|c| matches!(c, std::path::Component::ParentDir))
    {
        return Err("文件路径不能包含 ..".into());
    }
    Ok(())
}

// ===== 目录级核心逻辑（可单测） =====

/// 从 SKILL.md 内容解析 frontmatter description（name 一律用目录名，不信任 frontmatter）
fn parse_description(content: &str) -> String {
    let content = content.trim();
    if !content.starts_with("---") {
        return String::new();
    }
    let Some(rest) = content.strip_prefix("---") else {
        return String::new();
    };
    let Some(end) = rest.find("---") else {
        return String::new();
    };
    for line in rest[..end].lines() {
        if let Some(v) = line.trim().strip_prefix("description:") {
            return v.trim().trim_matches('"').trim_matches('\'').to_string();
        }
    }
    String::new()
}

fn skill_description(skill_dir: &Path) -> String {
    fs::read_to_string(skill_dir.join("SKILL.md"))
        .map(|c| parse_description(&c))
        .unwrap_or_default()
}

/// 递归统计技能内文件数（跳过隐藏目录与 .git）
fn count_files(dir: &Path, budget: &mut usize) -> usize {
    let mut count = 0;
    let Ok(entries) = fs::read_dir(dir) else {
        return 0;
    };
    for entry in entries.flatten() {
        if *budget == 0 {
            break;
        }
        *budget -= 1;
        let p = entry.path();
        let Some(name) = p.file_name().and_then(|n| n.to_str()) else {
            continue;
        };
        if name.starts_with('.') {
            continue;
        }
        if p.is_dir() {
            count += count_files(&p, budget);
        } else {
            count += 1;
        }
    }
    count
}

/// 递归列举技能内文件（相对路径，跳过隐藏与 .git，按路径排序，封顶 WALK_MAX_ENTRIES）
fn walk_files(dir: &Path, rel_prefix: &str, out: &mut Vec<UserSkillFile>) {
    if out.len() >= WALK_MAX_ENTRIES {
        return;
    }
    let Ok(entries) = fs::read_dir(dir) else {
        return;
    };
    let mut names: Vec<(String, PathBuf)> = entries
        .flatten()
        .filter_map(|e| {
            let n = e.file_name().to_string_lossy().to_string();
            if n.starts_with('.') {
                return None;
            }
            Some((n, e.path()))
        })
        .collect();
    names.sort_by(|a, b| a.0.cmp(&b.0));
    for (name, path) in names {
        if out.len() >= WALK_MAX_ENTRIES {
            return;
        }
        let rel = if rel_prefix.is_empty() {
            name.clone()
        } else {
            format!("{rel_prefix}/{name}")
        };
        if path.is_dir() {
            walk_files(&path, &rel, out);
        } else if let Ok(meta) = fs::metadata(&path) {
            out.push(UserSkillFile {
                rel_path: rel,
                size: meta.len(),
            });
        }
    }
}

/// 列出技能库目录下的全部技能
pub fn list_in(dir: &Path) -> Vec<UserSkillMeta> {
    let Ok(entries) = fs::read_dir(dir) else {
        return Vec::new();
    };
    let mut skills: Vec<UserSkillMeta> = entries
        .flatten()
        .filter_map(|e| {
            let path = e.path();
            if !path.is_dir() {
                return None;
            }
            let name = e.file_name().to_string_lossy().to_string();
            if name.starts_with('.') {
                return None;
            }
            let mut budget = WALK_MAX_ENTRIES;
            Some(UserSkillMeta {
                description: skill_description(&path),
                path: path.to_string_lossy().to_string(),
                file_count: count_files(&path, &mut budget),
                name,
            })
        })
        .collect();
    skills.sort_by(|a, b| a.name.cmp(&b.name));
    skills
}

/// Agent 用摘要列表（name + description）
pub fn summary_in(dir: &Path) -> Vec<SkillSummary> {
    list_in(dir)
        .into_iter()
        .map(|m| SkillSummary {
            name: m.name,
            description: m.description,
        })
        .collect()
}

/// 新建技能：创建目录 + SKILL.md 脚手架
pub fn create_skill_in(dir: &Path, name: &str, description: &str) -> Result<(), String> {
    validate_skill_name(name)?;
    let target = dir.join(name.trim());
    if target.exists() {
        return Err(format!("技能 '{name}' 已存在"));
    }
    fs::create_dir_all(&target).map_err(|e| format!("创建技能目录失败: {e}"))?;
    let desc = description.trim();
    let md = format!(
        "---\nname: {name}\ndescription: {desc}\n---\n\n# {name}\n\n（在这里描述这个技能的用途、输入与执行步骤）\n"
    );
    fs::write(target.join("SKILL.md"), md).map_err(|e| format!("写入 SKILL.md 失败: {e}"))?;
    Ok(())
}

/// 递归复制目录（跳过 .git）
fn copy_dir_recursive(src: &Path, dst: &Path) -> Result<(), String> {
    fs::create_dir_all(dst).map_err(|e| format!("创建目录失败: {e}"))?;
    let entries = fs::read_dir(src).map_err(|e| format!("读取源目录失败: {e}"))?;
    for entry in entries.flatten() {
        let sp = entry.path();
        let name = entry.file_name();
        if name.to_string_lossy() == ".git" {
            continue;
        }
        let dp = dst.join(&name);
        if sp.is_dir() {
            copy_dir_recursive(&sp, &dp)?;
        } else {
            fs::copy(&sp, &dp).map_err(|e| format!("复制 {} 失败: {e}", name.to_string_lossy()))?;
        }
    }
    Ok(())
}

/// 导入技能：把外部目录整体复制进技能库（重名报错；缺 SKILL.md 时补脚手架）
pub fn import_skill_into(dir: &Path, source: &Path) -> Result<String, String> {
    if !source.is_dir() {
        return Err("源目录不存在或不是目录".into());
    }
    let raw = source
        .file_name()
        .map(|n| n.to_string_lossy().to_string())
        .unwrap_or_default();
    validate_skill_name(&raw)?;
    let name = raw.trim().to_string();
    let target = dir.join(&name);
    if target.exists() {
        return Err(format!("技能 '{name}' 已存在，请先重命名源文件夹"));
    }
    copy_dir_recursive(source, &target)?;
    // 无 SKILL.md 的目录导入后补一个脚手架，保证 / 调用与 AI 读取都有说明入口
    if !target.join("SKILL.md").exists() {
        let md = format!(
            "---\nname: {name}\ndescription: （导入的技能，请补充描述）\n---\n\n# {name}\n"
        );
        let _ = fs::write(target.join("SKILL.md"), md);
    }
    Ok(name)
}

/// 读取技能内容：默认 SKILL.md，可指定技能内相对路径。超长截断。
pub fn read_skill_content(dir: &Path, name: &str, file: Option<&str>) -> Result<String, String> {
    validate_skill_name(name)?;
    let skill_dir = dir.join(name.trim());
    if !skill_dir.is_dir() {
        return Err(format!("技能 '{name}' 不存在"));
    }
    let rel = file.unwrap_or("SKILL.md");
    validate_rel_path(rel)?;
    let full = agent_core::ensure_within_root(&skill_dir, Path::new(rel))
        .map_err(|e| format!("路径校验失败: {e}"))?;
    let content =
        fs::read_to_string(skill_dir.join(full)).map_err(|e| format!("读取技能文件失败: {e}"))?;
    if content.chars().count() > SKILL_READ_MAX_CHARS {
        let truncated: String = content.chars().take(SKILL_READ_MAX_CHARS).collect();
        return Ok(format!(
            "{truncated}\n\n（内容超过 {SKILL_READ_MAX_CHARS} 字符已截断；如需更多内容请用 file 参数按文件分段读取）"
        ));
    }
    Ok(content)
}

/// 写入技能内文件（父目录自动创建）
pub fn write_skill_file(dir: &Path, name: &str, rel: &str, content: &str) -> Result<(), String> {
    validate_skill_name(name)?;
    validate_rel_path(rel)?;
    let skill_dir = dir.join(name.trim());
    if !skill_dir.is_dir() {
        return Err(format!("技能 '{name}' 不存在"));
    }
    let full = agent_core::ensure_within_root(&skill_dir, Path::new(rel))
        .map_err(|e| format!("路径校验失败: {e}"))?;
    let target = skill_dir.join(full);
    if let Some(parent) = target.parent() {
        fs::create_dir_all(parent).map_err(|e| format!("创建目录失败: {e}"))?;
    }
    fs::write(target, content).map_err(|e| format!("写入文件失败: {e}"))
}

/// 删除技能内单个文件
pub fn delete_skill_file(dir: &Path, name: &str, rel: &str) -> Result<(), String> {
    validate_skill_name(name)?;
    validate_rel_path(rel)?;
    let skill_dir = dir.join(name.trim());
    if !skill_dir.is_dir() {
        return Err(format!("技能 '{name}' 不存在"));
    }
    let full = agent_core::ensure_within_root(&skill_dir, Path::new(rel))
        .map_err(|e| format!("路径校验失败: {e}"))?;
    let target = skill_dir.join(full);
    if !target.is_file() {
        return Err(format!("文件不存在: {rel}"));
    }
    fs::remove_file(target).map_err(|e| format!("删除文件失败: {e}"))
}

/// 删除整个技能目录
pub fn delete_skill_dir(dir: &Path, name: &str) -> Result<(), String> {
    validate_skill_name(name)?;
    let target = dir.join(name.trim());
    if !target.is_dir() {
        return Err(format!("技能 '{name}' 不存在"));
    }
    fs::remove_dir_all(target).map_err(|e| format!("删除技能失败: {e}"))
}

// ===== Tauri 命令薄包装 =====

/// 应用技能库目录（app_data_dir/skills，不存在则创建）。
/// agent.rs 也用它来注册技能发现/读取工具。
/// 应用技能库目录：~/.config/astra/skills（不存在则创建）。
/// 用户技能是用户自持内容，放 ~/.config 便于用户在访达中直接查看与编辑；
/// 与 ~/.agents/skills（全局 Git 技能）心智模型一致。agent.rs 也用它注册技能工具。
pub fn app_skills_dir(_app: &AppHandle) -> Result<PathBuf, String> {
    let home = dirs::home_dir().ok_or("无法获取用户主目录")?;
    let dir = home.join(".config").join("astra").join("skills");
    fs::create_dir_all(&dir).map_err(|e| format!("创建技能库目录失败: {e}"))?;
    Ok(dir)
}

#[tauri::command]
pub fn list_user_skills(app: AppHandle) -> Result<Vec<UserSkillMeta>, String> {
    Ok(list_in(&app_skills_dir(&app)?))
}

#[tauri::command]
pub fn create_user_skill(
    app: AppHandle,
    name: String,
    description: String,
) -> Result<UserSkillMeta, String> {
    let dir = app_skills_dir(&app)?;
    create_skill_in(&dir, &name, &description)?;
    let path = dir.join(name.trim());
    let mut budget = WALK_MAX_ENTRIES;
    Ok(UserSkillMeta {
        name: name.trim().to_string(),
        description: description.trim().to_string(),
        path: path.to_string_lossy().to_string(),
        file_count: count_files(&path, &mut budget),
    })
}

#[tauri::command]
pub fn import_user_skill(app: AppHandle, source: String) -> Result<UserSkillMeta, String> {
    let dir = app_skills_dir(&app)?;
    let name = import_skill_into(&dir, Path::new(&source))?;
    let path = dir.join(&name);
    let mut budget = WALK_MAX_ENTRIES;
    Ok(UserSkillMeta {
        description: skill_description(&path),
        path: path.to_string_lossy().to_string(),
        file_count: count_files(&path, &mut budget),
        name,
    })
}

#[tauri::command]
pub fn list_user_skill_files(app: AppHandle, name: String) -> Result<Vec<UserSkillFile>, String> {
    let dir = app_skills_dir(&app)?;
    validate_skill_name(&name)?;
    let skill_dir = dir.join(name.trim());
    if !skill_dir.is_dir() {
        return Err(format!("技能 '{name}' 不存在"));
    }
    let mut files = Vec::new();
    walk_files(&skill_dir, "", &mut files);
    Ok(files)
}

#[tauri::command]
pub fn read_user_skill_file(
    app: AppHandle,
    name: String,
    rel_path: String,
) -> Result<String, String> {
    read_skill_content(&app_skills_dir(&app)?, &name, Some(&rel_path))
}

#[tauri::command]
pub fn write_user_skill_file(
    app: AppHandle,
    name: String,
    rel_path: String,
    content: String,
) -> Result<(), String> {
    write_skill_file(&app_skills_dir(&app)?, &name, &rel_path, &content)
}

#[tauri::command]
pub fn delete_user_skill_file(
    app: AppHandle,
    name: String,
    rel_path: String,
) -> Result<(), String> {
    delete_skill_file(&app_skills_dir(&app)?, &name, &rel_path)
}

#[tauri::command]
pub fn delete_user_skill(app: AppHandle, name: String) -> Result<(), String> {
    delete_skill_dir(&app_skills_dir(&app)?, &name)
}

// ===== 测试 =====

#[cfg(test)]
mod tests {
    use super::*;

    fn temp_root(label: &str) -> PathBuf {
        let dir = std::env::temp_dir().join(format!("astra-user-skills-{label}"));
        let _ = fs::remove_dir_all(&dir);
        fs::create_dir_all(&dir).unwrap();
        dir
    }

    #[test]
    fn skill_name_validation() {
        assert!(validate_skill_name("my-skill").is_ok());
        assert!(validate_skill_name("中文技能").is_ok());
        assert!(validate_skill_name("").is_err());
        assert!(validate_skill_name("  ").is_err());
        assert!(validate_skill_name(".hidden").is_err());
        assert!(validate_skill_name("..").is_err());
        assert!(validate_skill_name("a/b").is_err());
        assert!(validate_skill_name("a\\b").is_err());
    }

    #[test]
    fn rel_path_validation() {
        assert!(validate_rel_path("SKILL.md").is_ok());
        assert!(validate_rel_path("docs/guide.md").is_ok());
        assert!(validate_rel_path("").is_err());
        assert!(validate_rel_path("  ").is_err());
        assert!(validate_rel_path("/abs.md").is_err());
        assert!(validate_rel_path("a/../b.md").is_err());
        assert!(validate_rel_path("..\\x").is_err());
    }

    #[test]
    fn create_and_list_roundtrip() {
        let root = temp_root("create-list");
        create_skill_in(&root, "demo", "演示技能").unwrap();
        // 重名拒绝
        assert!(create_skill_in(&root, "demo", "x").is_err());

        let list = list_in(&root);
        assert_eq!(list.len(), 1);
        assert_eq!(list[0].name, "demo");
        assert_eq!(list[0].description, "演示技能");
        assert_eq!(list[0].file_count, 1);

        // SKILL.md 脚手架可被 read_skill_content 读回
        let content = read_skill_content(&root, "demo", None).unwrap();
        assert!(content.contains("description: 演示技能"));
        fs::remove_dir_all(&root).unwrap();
    }

    #[test]
    fn import_copies_and_scaffolds() {
        let root = temp_root("import");
        // 源目录必须在技能库之外（导入源在库内会与目标同名冲突）
        let src_root = temp_root("import-src");
        let src = src_root.join("src-skill");
        fs::create_dir_all(src.join("assets")).unwrap();
        fs::write(src.join("assets/a.txt"), "A").unwrap();
        fs::write(src.join("guide.md"), "# guide").unwrap();
        // .git 应被跳过
        fs::create_dir_all(src.join(".git")).unwrap();
        fs::write(src.join(".git/config"), "x").unwrap();

        let name = import_skill_into(&root, &src).unwrap();
        assert_eq!(name, "src-skill");
        let list = list_in(&root);
        assert_eq!(list.len(), 1);
        assert_eq!(list[0].file_count, 3); // assets/a.txt + guide.md + 补的 SKILL.md，.git 被跳过
                                           // 缺 SKILL.md → 补脚手架
        assert!(root.join("src-skill/SKILL.md").exists());

        // 重名导入拒绝
        assert!(import_skill_into(&root, &src).is_err());
        // 源不存在拒绝
        assert!(import_skill_into(&root, &src_root.join("nope")).is_err());
        fs::remove_dir_all(&root).unwrap();
        fs::remove_dir_all(&src_root).unwrap();
    }

    #[test]
    fn read_write_delete_skill_file() {
        let root = temp_root("rw");
        create_skill_in(&root, "demo", "d").unwrap();

        write_skill_file(&root, "demo", "docs/note.md", "hello").unwrap();
        assert_eq!(
            read_skill_content(&root, "demo", Some("docs/note.md")).unwrap(),
            "hello"
        );
        let mut files = Vec::new();
        walk_files(&root.join("demo"), "", &mut files);
        assert_eq!(files.len(), 2);

        // 路径逃逸全部拒绝
        assert!(read_skill_content(&root, "demo", Some("../other/SKILL.md")).is_err());
        assert!(write_skill_file(&root, "demo", "/abs.md", "x").is_err());
        assert!(delete_skill_file(&root, "demo", "../x").is_err());
        // 未知技能拒绝
        assert!(read_skill_content(&root, "ghost", None).is_err());

        delete_skill_file(&root, "demo", "docs/note.md").unwrap();
        assert!(delete_skill_file(&root, "demo", "docs/note.md").is_err());

        delete_skill_dir(&root, "demo").unwrap();
        assert!(list_in(&root).is_empty());
        fs::remove_dir_all(&root).unwrap();
    }

    #[test]
    fn description_parser() {
        let md = "---\nname: x\ndescription: \"带引号的描述\"\n---\n\nbody";
        assert_eq!(parse_description(md), "带引号的描述");
        assert_eq!(parse_description("no frontmatter"), "");
        assert_eq!(parse_description("---\nbroken"), "");
    }
}
