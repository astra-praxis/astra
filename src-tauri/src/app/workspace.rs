use serde::Serialize;
use std::fs;
use std::path::Path;
use std::process::Command;

use crate::app::settings::get_settings;

// ===== 类型定义（保持原有）=====

#[derive(Debug, Serialize, Clone)]
pub(crate) struct SubProject {
    name: String,
    path: String,
}

#[derive(Debug, Serialize)]
pub(crate) struct ProjectCard {
    name: String,
    path: String,
    has_readme: bool,
    sub_projects: Vec<SubProject>,
}

// ===== 命令：扫描工作空间 =====

#[tauri::command]
pub(crate) async fn scan_workspace(path: String) -> Result<Vec<ProjectCard>, String> {
    tokio::task::spawn_blocking(move || {
        let dir = Path::new(&path);

        if !dir.is_dir() {
            app_log!(
                error,
                "workspace",
                "scan_workspace: not a directory: {}",
                path
            );
            return Err(format!("路径不是有效的目录: {}", path));
        }

        let entries = match fs::read_dir(dir) {
            Ok(e) => e,
            Err(e) => {
                app_log!(
                    error,
                    "workspace",
                    "scan_workspace: read_dir failed {}: {}",
                    path,
                    e
                );
                return Err(format!("读取目录失败: {}", e));
            }
        };

        let mut projects = Vec::new();

        for entry in entries {
            let entry = entry.map_err(|e| format!("读取条目失败: {}", e))?;
            let entry_path = entry.path();

            if !entry_path.is_dir() {
                continue;
            }

            // 跳过隐藏目录（以 . 开头）
            let folder_name = entry_path
                .file_name()
                .unwrap_or_default()
                .to_string_lossy()
                .to_string();

            if folder_name.starts_with('.') {
                continue;
            }

            // 检查 README.md
            let readme_path = entry_path.join("README.md");
            let has_readme = readme_path.exists();

            // 无 README 的项目会自动补建 README.md（首次扫描会写文件）
            let project_name = if has_readme {
                match fs::read_to_string(&readme_path) {
                    Ok(content) => content
                        .lines()
                        .next()
                        .unwrap_or(&folder_name)
                        .trim()
                        .trim_start_matches("# ")
                        .trim()
                        .to_string(),
                    Err(_) => folder_name.clone(),
                }
            } else {
                let readme_content =
                    format!("# {}\n\nThis is the {} project.", folder_name, folder_name);
                if let Err(e) = fs::write(&readme_path, &readme_content) {
                    eprintln!("创建 README.md 失败: {}", e);
                }
                folder_name.clone()
            };

            projects.push(ProjectCard {
                name: project_name,
                path: entry_path.to_string_lossy().to_string(),
                has_readme,
                sub_projects: scan_sub_projects(&entry_path),
            });
        }

        projects.sort_by(|a, b| a.name.to_lowercase().cmp(&b.name.to_lowercase()));
        app_log!(
            info,
            "workspace",
            "scan_workspace ok: {} projects ({})",
            projects.len(),
            path
        );
        Ok(projects)
    })
    .await
    .map_err(|e| {
        app_log!(
            error,
            "workspace",
            "scan_workspace: task join failed: {}",
            e
        );
        format!("扫描工作空间失败: {}", e)
    })?
}

// ===== 命令：列出项目内文件（@ 文件引用候选） =====

/// 递归列出项目内文件的相对路径（`/` 分隔），供输入框 @ 引用弹层搜索。
#[tauri::command]
pub(crate) async fn list_project_files(path: String) -> Result<Vec<String>, String> {
    tokio::task::spawn_blocking(move || {
        let root = Path::new(&path);
        if !root.is_dir() {
            app_log!(
                error,
                "workspace",
                "list_project_files: not a directory: {}",
                path
            );
            return Err(format!("路径不是有效的目录: {}", path));
        }
        Ok(collect_project_files(root))
    })
    .await
    .map_err(|e| format!("扫描项目文件失败: {}", e))?
}

/// 递归收集项目内文件的相对路径，独立成函数便于单测。
/// 跳过隐藏目录与常见构建产物目录，限制深度与数量，避免超大项目拖慢弹层。
pub(crate) fn collect_project_files(root: &Path) -> Vec<String> {
    const MAX_DEPTH: u32 = 8;
    const MAX_FILES: usize = 2000;
    // 这些目录几乎不会是 @ 引用的目标，跳过既省时间也避免候选被噪音塞满
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

    fn walk(dir: &Path, rel: &str, depth: u32, out: &mut Vec<String>) {
        if depth >= MAX_DEPTH || out.len() >= MAX_FILES {
            return;
        }
        let Ok(entries) = fs::read_dir(dir) else {
            return;
        };
        let mut items: Vec<_> = entries.flatten().collect();
        items.sort_by_key(|e| e.file_name());
        for entry in items {
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
                walk(&entry.path(), &child_rel, depth + 1, out);
            } else if file_type.is_file() && !name.starts_with('.') {
                if out.len() >= MAX_FILES {
                    return;
                }
                out.push(child_rel);
            }
        }
    }

    let mut files = Vec::new();
    walk(root, "", 0, &mut files);
    files
}

// ===== 扫描项目子目录 =====

pub(crate) fn scan_sub_projects(project_dir: &Path) -> Vec<SubProject> {
    let mut subs = Vec::new();

    let entries = match fs::read_dir(project_dir) {
        Ok(e) => e,
        Err(_) => return subs,
    };

    for entry in entries.flatten() {
        let entry_path = entry.path();

        if !entry_path.is_dir() {
            continue;
        }

        let folder_name = match entry_path.file_name() {
            Some(n) => n.to_string_lossy().to_string(),
            None => continue,
        };

        if folder_name.starts_with('.') || folder_name == "node_modules" {
            continue;
        }

        subs.push(SubProject {
            name: folder_name,
            path: entry_path.to_string_lossy().to_string(),
        });
    }

    subs.sort_by(|a, b| a.name.to_lowercase().cmp(&b.name.to_lowercase()));
    subs
}

// ===== 获取 Git 远程地址 =====

pub(crate) fn get_git_remote_url(repo_path: &Path) -> Option<String> {
    let mut cmd = Command::new("git");
    cmd.args([
        "-C",
        &repo_path.to_string_lossy(),
        "remote",
        "get-url",
        "origin",
    ]);
    crate::proc_util::hide_window(&mut cmd);
    let output = cmd.output().ok()?;
    if output.status.success() {
        let url = String::from_utf8_lossy(&output.stdout).trim().to_string();
        if !url.is_empty() {
            Some(url)
        } else {
            None
        }
    } else {
        None
    }
}

// ===== 项目详情 =====

#[derive(Debug, Serialize)]
pub(crate) struct GitRepo {
    name: String,
    path: String,
    remote_url: Option<String>,
}

#[derive(Debug, Serialize)]
pub(crate) struct SubDetail {
    name: String,
    path: String,
    git_repo: Option<GitRepo>,
    children: Vec<SubDetail>,
    readme_preview: String,
    depth: u32,
}

#[derive(Debug, Serialize)]
pub(crate) struct ProjectDetail {
    name: String,
    path: String,
    has_readme: bool,
    readme_preview: String,
    sub_items: Vec<SubDetail>,
}

pub(crate) fn read_readme_preview(dir: &Path) -> String {
    let readme_path = dir.join("README.md");
    if !readme_path.exists() {
        return String::new();
    }
    match fs::read_to_string(&readme_path) {
        Ok(content) => content.chars().take(1000).collect::<String>(),
        Err(_) => String::new(),
    }
}

// ===== 递归扫描子目录（用于详情视图）=====

pub(crate) fn scan_subdirectory(dir: &Path, current_depth: u32, max_depth: u32) -> Vec<SubDetail> {
    if current_depth >= max_depth {
        return Vec::new();
    }

    let mut items = Vec::new();
    let entries = match fs::read_dir(dir) {
        Ok(e) => e,
        Err(_) => return items,
    };

    for entry in entries.flatten() {
        let entry_path = entry.path();
        if !entry_path.is_dir() {
            continue;
        }

        let name = match entry_path.file_name() {
            Some(n) => n.to_string_lossy().to_string(),
            None => continue,
        };

        if name.starts_with('.') || name == "node_modules" {
            continue;
        }

        let git_repo = get_git_remote_url(&entry_path).map(|url| GitRepo {
            name: name.clone(),
            path: entry_path.to_string_lossy().to_string(),
            remote_url: Some(url),
        });

        // 如果目录本身有 .git 子目录，说明这是一个 git 仓库根目录，不再遍历其子目录
        let has_git_dir = entry_path.join(".git").exists();
        let children = if has_git_dir {
            Vec::new()
        } else {
            scan_subdirectory(&entry_path, current_depth + 1, max_depth)
        };

        items.push(SubDetail {
            name,
            path: entry_path.to_string_lossy().to_string(),
            git_repo,
            children,
            readme_preview: read_readme_preview(&entry_path),
            depth: current_depth + 1,
        });
    }

    items.sort_by(|a, b| a.name.to_lowercase().cmp(&b.name.to_lowercase()));
    items
}

#[tauri::command]
pub(crate) fn get_project_detail(
    app: tauri::AppHandle,
    path: String,
) -> Result<ProjectDetail, String> {
    let settings = get_settings(app);
    let max_depth = settings.scan_depth;

    let dir = Path::new(&path);
    if !dir.is_dir() {
        app_log!(
            error,
            "workspace",
            "get_project_detail: dir not found: {}",
            path
        );
        return Err(format!("项目目录不存在: {}", path));
    }

    let folder_name = dir
        .file_name()
        .unwrap_or_default()
        .to_string_lossy()
        .to_string();

    let readme_path = dir.join("README.md");
    let (project_name, readme_preview, has_readme) = if readme_path.exists() {
        match fs::read_to_string(&readme_path) {
            Ok(content) => {
                let first_line = content
                    .lines()
                    .next()
                    .unwrap_or(&folder_name)
                    .trim()
                    .trim_start_matches("# ")
                    .trim()
                    .to_string();
                let preview = content.chars().take(300).collect::<String>();
                (
                    if first_line.is_empty() {
                        folder_name.clone()
                    } else {
                        first_line
                    },
                    preview,
                    true,
                )
            }
            Err(_) => (folder_name.clone(), String::new(), false),
        }
    } else {
        (folder_name.clone(), String::new(), false)
    };

    // 如果项目根目录本身就有 .git，说明整个项目就是一个 git 仓库，不遍历子目录
    let sub_items = if dir.join(".git").exists() {
        Vec::new()
    } else {
        scan_subdirectory(&dir, 0, max_depth)
    };

    app_log!(
        info,
        "workspace",
        "get_project_detail ok: {} (sub_items: {}, depth: {})",
        path,
        sub_items.len(),
        max_depth
    );

    Ok(ProjectDetail {
        name: project_name,
        path: dir.to_string_lossy().to_string(),
        has_readme,
        readme_preview,
        sub_items,
    })
}

// ===== 命令：查找 casp-portal 目录（用于「开发模式」按钮判断） =====

#[tauri::command]
pub(crate) fn find_casp_portal_dir(path: String) -> Result<Option<String>, String> {
    let dir = Path::new(&path);
    if !dir.is_dir() {
        return Err("目录不存在".to_string());
    }
    let entries = match std::fs::read_dir(dir) {
        Ok(e) => e,
        Err(e) => return Err(format!("读取目录失败: {}", e)),
    };
    for entry in entries.flatten() {
        let entry_path = entry.path();
        if !entry_path.is_dir() {
            continue;
        }
        if let Some(name) = entry_path.file_name().and_then(|n| n.to_str()) {
            if name.starts_with("casp-portal") {
                return Ok(Some(entry_path.to_string_lossy().to_string()));
            }
        }
    }
    Ok(None)
}

// ===== 命令：创建新项目 =====

#[tauri::command]
pub(crate) fn create_project(
    workspace_path: String,
    folder_name: String,
    project_name: String,
) -> Result<ProjectCard, String> {
    // 验证文件夹名称
    if folder_name.trim().is_empty() {
        return Err("文件夹名称不能为空".to_string());
    }

    let folder_name = folder_name.trim();

    // 不允许以 . 开头（隐藏目录）
    if folder_name.starts_with('.') {
        return Err("文件夹名称不能以 . 开头".to_string());
    }

    // 不允许包含路径分隔符
    if folder_name.contains('/') || folder_name.contains('\\') {
        return Err("文件夹名称不能包含路径分隔符".to_string());
    }

    let project_dir = Path::new(&workspace_path).join(folder_name);

    // 检查目录是否已存在
    if project_dir.exists() {
        app_log!(
            warn,
            "workspace",
            "create_project: dir exists: {}",
            project_dir.display()
        );
        return Err(format!("文件夹 '{}' 已存在", folder_name));
    }

    // 创建目录
    if let Err(e) = fs::create_dir_all(&project_dir) {
        app_log!(
            error,
            "workspace",
            "create_project: mkdir failed {}: {}",
            project_dir.display(),
            e
        );
        return Err(format!("创建目录失败: {}", e));
    }

    let project_name = if project_name.trim().is_empty() {
        folder_name.to_string()
    } else {
        project_name.trim().to_string()
    };

    // 写入 README.md
    let readme_content = format!("# {}\n\nThis is the {} project.", project_name, folder_name);
    let readme_path = project_dir.join("README.md");
    if let Err(e) = fs::write(&readme_path, &readme_content) {
        app_log!(
            error,
            "workspace",
            "create_project: write README failed: {}",
            e
        );
        return Err(format!("创建 README.md 失败: {}", e));
    }
    app_log!(
        info,
        "workspace",
        "create_project ok: {}",
        project_dir.display()
    );

    Ok(ProjectCard {
        name: project_name,
        path: project_dir.to_string_lossy().to_string(),
        has_readme: true,
        sub_projects: scan_sub_projects(&project_dir),
    })
}
// ===== 测试 =====

#[cfg(test)]
mod lib_tests {
    use super::*;

    /// 在临时目录里搭一棵小项目树，验证 collect_project_files 的收集/过滤规则
    #[test]
    fn collect_project_files_skips_hidden_and_junk_dirs() {
        let root = std::env::temp_dir().join("astra-lib-collect-files");
        let _ = fs::remove_dir_all(&root);
        fs::create_dir_all(root.join("src/components")).unwrap();
        fs::create_dir_all(root.join("node_modules/pkg")).unwrap();
        fs::create_dir_all(root.join(".git")).unwrap();
        fs::create_dir_all(root.join("dist")).unwrap();

        fs::write(root.join("README.md"), "# demo").unwrap();
        fs::write(root.join(".env"), "K=V").unwrap();
        fs::write(root.join("src/main.rs"), "fn main() {}").unwrap();
        fs::write(root.join("src/components/Button.tsx"), "x").unwrap();
        fs::write(root.join("node_modules/pkg/index.js"), "x").unwrap();
        fs::write(root.join(".git/config"), "x").unwrap();
        fs::write(root.join("dist/index.html"), "x").unwrap();

        let files = collect_project_files(&root);

        // 只收普通文件，跳过隐藏文件与 node_modules/.git/dist
        assert_eq!(
            files,
            vec![
                "README.md".to_string(),
                "src/components/Button.tsx".to_string(),
                "src/main.rs".to_string(),
            ]
        );

        let _ = fs::remove_dir_all(&root);
    }

    /// 非目录路径报错（命令层校验）
    #[test]
    fn collect_project_files_nested_dirs_use_slash() {
        let root = std::env::temp_dir().join("astra-lib-collect-nested");
        let _ = fs::remove_dir_all(&root);
        fs::create_dir_all(root.join("a/b/c")).unwrap();
        fs::write(root.join("a/b/c/deep.txt"), "x").unwrap();

        let files = collect_project_files(&root);
        assert_eq!(files, vec!["a/b/c/deep.txt".to_string()]);

        let _ = fs::remove_dir_all(&root);
    }
}
