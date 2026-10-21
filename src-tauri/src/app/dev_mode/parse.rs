use std::path::Path;

use super::types::*;
// ===== vue.config.js 解析辅助 =====

/// 从 vue.config.js 内容中提取 Cookie 值（）
/// 匹配: setHeader("Cookie", "VALUE")
pub(crate) fn extract_cookie(content: &str) -> String {
    let key = "\"Cookie\"";
    let pos = match content.find(key) {
        Some(p) => p,
        None => return String::new(),
    };
    let rest = &content[pos + key.len()..];
    // 找第一个 "（Cookie 值的起始引号）
    let q1 = match rest.find('"') {
        Some(p) => p,
        None => return String::new(),
    };
    let rest2 = &rest[q1 + 1..];
    // 找第二个 "（Cookie 值的结束引号）
    let q2 = match rest2.find('"') {
        Some(p) => p,
        None => return String::new(),
    };
    rest2[..q2].to_string()
}

/// 从 vue.config.js 中提取后端代理地址（第一个非 localhost 的 target）
pub(crate) fn extract_backend_target(content: &str) -> String {
    for line in content.lines() {
        let trimmed = line.trim_start();
        // 跳过注释行
        if trimmed.starts_with("//") {
            continue;
        }
        if trimmed.contains("target:") && trimmed.contains('"') {
            if let Some(start) = trimmed.find('"') {
                let after = &trimmed[start + 1..];
                if let Some(end) = after.find('"') {
                    let url = &after[..end];
                    // 跳过 localhost / 127.0.0.1（卡片代理）
                    if !url.contains("localhost") && !url.contains("127.0.0.1") {
                        return url.to_string();
                    }
                }
            }
        }
    }
    String::new()
}

/// 从 vue.config.js 中提取 devServer.port
pub(crate) fn extract_dev_server_port(content: &str) -> Option<u16> {
    for line in content.lines() {
        let trimmed = line.trim();
        if trimmed.starts_with("//") {
            continue;
        }
        if trimmed.contains("port:") && !trimmed.contains("false") {
            // 提取 port: 后面的数字
            if let Some(idx) = trimmed.find("port:") {
                let after = &trimmed[idx + 5..];
                let num_str: String = after
                    .trim()
                    .chars()
                    .take_while(|c| c.is_ascii_digit())
                    .collect();
                if let Ok(port) = num_str.parse::<u16>() {
                    return Some(port);
                }
            }
        }
    }
    None
}

/// 从 dev server 输出行中解析监听端口
/// 匹配 localhost:PORT / 127.0.0.1:PORT / 0.0.0.0:PORT 等常见格式
pub(crate) fn extract_port_from_output(line: &str) -> Option<u16> {
    for pattern in &[
        "localhost:",
        "127.0.0.1:",
        "0.0.0.0:",
        "http://[::]:",
        "http://[::1]:",
        "[::1]:",
    ] {
        if let Some(idx) = line.find(pattern) {
            let after = &line[idx + pattern.len()..];
            let num_str: String = after.chars().take_while(|c| c.is_ascii_digit()).collect();
            if let Ok(port) = num_str.parse::<u16>() {
                if port > 0 {
                    return Some(port);
                }
            }
        }
    }
    None
}

/// 替换 vue.config.js 中的 Cookie 值
pub(crate) fn replace_cookie_value(content: &str, new_cookie: &str) -> Option<String> {
    let key = "\"Cookie\"";
    let pos = content.find(key)?;
    let rest = &content[pos + key.len()..];
    let q1 = rest.find('"')?;
    let val_start = pos + key.len() + q1 + 1;
    let rest2 = &content[val_start..];
    let q2 = rest2.find('"')?;

    let mut result = String::with_capacity(content.len() + new_cookie.len());
    result.push_str(&content[..val_start]);
    result.push_str(new_cookie);
    result.push_str(&content[val_start + q2..]);
    Some(result)
}

/// 构建 PATH 环境变量，补充打包后可能缺失的路径（nvm/fnm/volta/homebrew 等）
pub(crate) fn build_full_path() -> String {
    let existing = std::env::var("PATH").unwrap_or_default();
    let home = std::env::var("HOME").unwrap_or_default();

    let mut extra: Vec<String> = vec![
        "/usr/local/bin".into(),
        "/opt/homebrew/bin".into(),
        "/opt/homebrew/sbin".into(),
        "/usr/bin".into(),
        "/bin".into(),
    ];

    if !home.is_empty() {
        // npm global
        extra.push(format!("{}/.npm-global/bin", home));

        // ===== fnm (Fast Node Manager) =====
        // 现代版 fnm 默认目录: ~/.local/share/fnm/
        // default alias: ~/.local/share/fnm/aliases/default → node-versions/vXX.X.X/installation
        // 传统路径: ~/Library/fnm/
        extra.push(format!("{}/.local/share/fnm/aliases/default/bin", home));
        extra.push(format!("{}/Library/fnm/aliases/default/bin", home));
        // glob 所有版本目录（最新开发的 fallback）
        for base in &[
            format!("{}/.local/share/fnm/node-versions", home),
            format!("{}/Library/fnm/node-versions", home),
        ] {
            if let Ok(entries) = std::fs::read_dir(base) {
                for entry in entries.flatten() {
                    let bin = entry.path().join("installation/bin");
                    if bin.is_dir() {
                        extra.push(bin.to_string_lossy().to_string());
                    }
                }
            }
        }

        // ===== nvm =====
        // nvm 没有 current 软链，需 glob 版本目录
        let nvm_dir_base = format!("{}/.nvm/versions/node", home);
        if let Ok(entries) = std::fs::read_dir(&nvm_dir_base) {
            for entry in entries.flatten() {
                let bin = entry.path().join("bin");
                if bin.is_dir() {
                    extra.push(bin.to_string_lossy().to_string());
                }
            }
        }
        // nvm 也可能用 NVM_HOME 环境变量（$HOME/.nvm）
        // 允许用户自定义 NVM_DIR
        if let Ok(nvm_dir) = std::env::var("NVM_DIR") {
            let nvm_versions = format!("{}/versions/node", nvm_dir);
            if let Ok(entries) = std::fs::read_dir(&nvm_versions) {
                for entry in entries.flatten() {
                    let bin = entry.path().join("bin");
                    if bin.is_dir() {
                        extra.push(bin.to_string_lossy().to_string());
                    }
                }
            }
        }

        // ===== volta =====
        extra.push(format!("{}/.volta/bin", home));

        // ===== pnpm global =====
        extra.push(format!("{}/Library/pnpm", home));
    }

    // 现有 PATH 放最后（优先使用上面稳定的路径）
    extra.push(existing);
    extra.join(":")
}

/// 检测是否为日期目录（如 20260717）
pub(crate) fn is_date_folder(name: &str) -> bool {
    name.len() == 8 && name.chars().all(|c| c.is_ascii_digit())
}

/// 读取目录下 README.md 的首行标题作为显示名
/// 如果没有 README 或首行为空，返回 fallback
pub(crate) fn read_readme_title(dir: &Path, fallback: &str) -> String {
    let readme = dir.join("README.md");
    if let Ok(content) = std::fs::read_to_string(&readme) {
        if let Some(first_line) = content.lines().next() {
            let title = first_line.trim().trim_start_matches('#').trim();
            if !title.is_empty() {
                return title.to_string();
            }
        }
    }
    fallback.to_string()
}

/// 扫描卡片目录下的前端子目录
/// 检测 src/main/ 下的 web / mobile / component-web
/// 对于 webapps 下的模板目录，检测根级 web / mobile
pub(crate) fn scan_card_subdirs(card_path: &Path, _folder_name: &str) -> Vec<DevSubDir> {
    let mut sub_dirs: Vec<DevSubDir> = Vec::new();

    // 标准卡片: src/main/ 下的 web / mobile / component-web
    let src_main = card_path.join("src/main");
    if src_main.is_dir() {
        check_subdir(&mut sub_dirs, &src_main.join("web"), "Web", "web");
        check_subdir(&mut sub_dirs, &src_main.join("mobile"), "Mobile", "mobile");
        check_subdir(
            &mut sub_dirs,
            &src_main.join("component-web"),
            "Component",
            "component-web",
        );
    }

    // 如果 src/main 下没找到，检测根级 web / mobile（模板项目可能如此）
    if sub_dirs.is_empty() {
        check_subdir(&mut sub_dirs, &card_path.join("web"), "Web", "web");
        check_subdir(&mut sub_dirs, &card_path.join("mobile"), "Mobile", "mobile");
    }

    sub_dirs
}

/// 辅助: 检查单个子目录是否存在，存在则加入列表
pub(crate) fn check_subdir(sub_dirs: &mut Vec<DevSubDir>, dir: &Path, label: &str, key: &str) {
    if dir.is_dir() {
        let has_pkg = dir.join("package.json").exists();
        if has_pkg {
            sub_dirs.push(DevSubDir {
                label: label.to_string(),
                key: key.to_string(),
                work_dir: dir.to_string_lossy().to_string(),
                has_package_json: true,
            });
        }
    }
}

