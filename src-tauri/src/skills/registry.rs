use serde::Serialize;
use std::fs;
use std::path::Path;
use std::process::Command;

// ===== Skills 管理 =====

#[derive(Debug, Serialize)]
pub(crate) struct SkillCard {
    name: String,
    description: String,
    path: String,
    version: String,
    is_git: bool,
}

/// 解析 SKILL.md 的 YAML frontmatter（name / description / version）。
/// 字段解析（含 YAML 块标量折叠）统一走 agent-core 的规范实现，
/// 避免与 user.rs / astra-skill 各自维护一份解析逻辑漂移。
pub(crate) fn parse_skill_frontmatter(content: &str) -> (String, String, String) {
    let field = |name: &str| agent_core::skill_tools::parse_frontmatter_field(content, name);
    (field("name"), field("description"), field("version"))
}

#[tauri::command]
pub(crate) fn list_skills() -> Result<Vec<SkillCard>, String> {
    let home = dirs::home_dir().ok_or("无法获取用户主目录")?;
    let skills_dir = home.join(".agents").join("skills");

    if !skills_dir.exists() {
        return Ok(Vec::new());
    }

    let entries = fs::read_dir(&skills_dir).map_err(|e| format!("读取 skills 目录失败: {}", e))?;

    let mut skills = Vec::new();

    for entry in entries {
        let entry = entry.map_err(|e| format!("读取条目失败: {}", e))?;
        let entry_path = entry.path();

        if !entry_path.is_dir() {
            continue;
        }

        let folder_name = entry_path
            .file_name()
            .unwrap_or_default()
            .to_string_lossy()
            .to_string();

        if folder_name.starts_with('.') {
            continue;
        }

        let skill_md_path = entry_path.join("SKILL.md");
        let (name, description, version) = if skill_md_path.exists() {
            match fs::read_to_string(&skill_md_path) {
                Ok(content) => parse_skill_frontmatter(&content),
                Err(_) => (folder_name.clone(), String::new(), String::new()),
            }
        } else {
            (folder_name.clone(), String::new(), String::new())
        };

        // 检查是否是 git 仓库
        let is_git = entry_path.join(".git").exists();

        skills.push(SkillCard {
            name: if name.is_empty() { folder_name } else { name },
            description,
            path: entry_path.to_string_lossy().to_string(),
            version,
            is_git,
        });
    }

    skills.sort_by(|a, b| a.name.to_lowercase().cmp(&b.name.to_lowercase()));
    Ok(skills)
}

#[tauri::command]
pub(crate) fn delete_skill(path: String) -> Result<(), String> {
    let dir = Path::new(&path);
    if !dir.exists() {
        return Err("技能目录不存在".to_string());
    }
    fs::remove_dir_all(dir).map_err(|e| format!("删除失败: {}", e))?;
    Ok(())
}

/// 更新单个 Skill（通过 npx skills add）
#[tauri::command]
pub(crate) fn update_skill(path: String) -> Result<String, String> {
    let dir = Path::new(&path);
    if !dir.exists() || !dir.is_dir() {
        return Err("技能目录不存在".to_string());
    }

    // 从路径中提取 skill 名称
    let skill_name = dir
        .file_name()
        .unwrap_or_default()
        .to_string_lossy()
        .to_string();

    if skill_name.is_empty() {
        return Err("无法提取技能名称".to_string());
    }

    // 执行 npx skills add <skill-name> -g -y 来更新
    let mut cmd = Command::new("npx");
    cmd.args(["skills", "add", &skill_name, "-g", "-y"]);
    crate::proc_util::hide_window(&mut cmd);
    let output = cmd
        .output()
        .map_err(|e| format!("执行 npx skills add 失败: {}", e))?;

    let stdout = String::from_utf8_lossy(&output.stdout).to_string();
    let stderr = String::from_utf8_lossy(&output.stderr).to_string();

    if output.status.success() {
        Ok(format!("{}\n{}", stdout, stderr))
    } else {
        Err(format!("更新失败: {}\n{}", stdout, stderr))
    }
}

/// 更新所有 Skills（通过 npx skills update -g）
#[tauri::command]
pub(crate) fn update_all_skills() -> Result<Vec<(String, Result<String, String>)>, String> {
    let home = dirs::home_dir().ok_or("无法获取用户主目录")?;
    let skills_dir = home.join(".agents").join("skills");

    if !skills_dir.exists() {
        return Ok(Vec::new());
    }

    // 执行 npx skills update -g 命令
    let mut cmd = Command::new("npx");
    cmd.args(["skills", "update", "-g"]);
    crate::proc_util::hide_window(&mut cmd);
    let output = cmd
        .output()
        .map_err(|e| format!("执行 npx skills update 失败: {}", e))?;

    let stdout = String::from_utf8_lossy(&output.stdout).to_string();
    let stderr = String::from_utf8_lossy(&output.stderr).to_string();

    if !output.status.success() {
        return Err(format!("更新失败: {}\n{}", stdout, stderr));
    }

    // 解析输出，提取更新的 skills
    let mut results = Vec::new();
    let mut current_skill = String::new();
    let mut current_status = String::new();

    for line in stdout.lines() {
        let line = line.trim();

        // 匹配 "Updating <skill-name>…" 格式
        if line.starts_with("Updating ") && line.ends_with("…") {
            // 保存上一个 skill 的结果
            if !current_skill.is_empty() {
                let result = if current_status.contains("✓") {
                    Ok(current_status.clone())
                } else {
                    Err(current_status.clone())
                };
                results.push((current_skill.clone(), result));
            }

            // 提取新的 skill 名称
            current_skill = line
                .strip_prefix("Updating ")
                .unwrap_or("")
                .strip_suffix("…")
                .unwrap_or("")
                .to_string();
            current_status = String::new();
        } else if line.contains("✓ Updated") || line.contains("✗ Failed") {
            current_status = line.to_string();
        }
    }

    // 添加最后一个 skill 的结果
    if !current_skill.is_empty() {
        let result = if current_status.contains("✓") {
            Ok(current_status)
        } else {
            Err(current_status)
        };
        results.push((current_skill, result));
    }

    // 如果没有解析到任何结果，返回整体输出
    if results.is_empty() {
        results.push(("all".to_string(), Ok(format!("{}\n{}", stdout, stderr))));
    }

    Ok(results)
}

#[tauri::command]
pub(crate) fn read_skill_doc(path: String) -> Result<String, String> {
    let dir = Path::new(&path);
    if !dir.exists() || !dir.is_dir() {
        return Err("技能目录不存在".to_string());
    }

    // 按优先级尝试读取文档文件
    let candidates = ["SKILL.md", "README.md", "readme.md", "README", "Readme.md"];
    for candidate in &candidates {
        let file_path = dir.join(candidate);
        if file_path.exists() && file_path.is_file() {
            return fs::read_to_string(&file_path).map_err(|e| format!("读取文件失败: {}", e));
        }
    }

    // 尝试任意 .md 文件
    if let Ok(entries) = fs::read_dir(dir) {
        for entry in entries.flatten() {
            let entry_path = entry.path();
            if entry_path.is_file() {
                if let Some(ext) = entry_path.extension() {
                    if ext == "md" {
                        return fs::read_to_string(&entry_path)
                            .map_err(|e| format!("读取文件失败: {}", e));
                    }
                }
            }
        }
    }

    Err("未找到文档文件（SKILL.md / README.md）".to_string())
}
