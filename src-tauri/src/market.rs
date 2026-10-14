//! 扩展市场（MCP / Skill）：
//!
//! - 内置精选目录（curated catalog），前端按关键词过滤搜索；
//!   不做网络目录抓取（目录随版本内置，来源标注清楚）
//! - MCP 安装 = 下载预热（npm install -g）+ 把 [mcp.servers.<id>] /
//!   [acp.agents.<id>] 段合并写入 ~/.astra/config.toml
//!   （MCP_SPEC §11 / ACP_SPEC §11 冻结 schema；文本级合并，段已存在则跳过）
//! - Skill 安装 = git clone --depth 1 到临时目录，拷贝子目录进应用技能库
//!   （~/.config/astra/skills/<name>，与 user_skills / Agent read_skill 打通）
//!
//! 推荐：静态推荐位（recommended + reason），排序置顶；不做评分系统。

use serde::Serialize;
use std::path::{Path, PathBuf};

// ===== 数据模型 =====

#[derive(Debug, Clone, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct McpMarketEntry {
    pub id: String,
    /// "mcp" | "acp"
    pub kind: String,
    pub name: String,
    pub description: String,
    pub tags: Vec<String>,
    pub official: bool,
    pub recommended: bool,
    pub recommend_reason: String,
    pub command: String,
    pub args: Vec<String>,
    /// 需要本机具备的运行时（提示用）
    pub requires: Vec<String>,
    /// 安装时预热的 npm 包（None = 无需下载）
    pub npm_package: Option<String>,
    /// 写入 permissions.allow 的 MCP 侧工具名（只读类，安装即放行）
    pub suggested_allow: Vec<String>,
    /// 需要用户预先导出的环境变量名（写入 env 名单，值不在配置文件）
    pub requires_env: Vec<String>,
}

#[derive(Debug, Clone, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct SkillMarketEntry {
    pub id: String,
    pub name: String,
    pub description: String,
    pub tags: Vec<String>,
    pub official: bool,
    pub recommended: bool,
    pub recommend_reason: String,
    pub repo: String,
    /// 仓库内技能目录相对路径
    pub subpath: String,
    /// 安装后的技能目录名（= /调用名）
    pub install_name: String,
}

#[derive(Debug, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct InstallResult {
    pub ok: bool,
    pub message: String,
}

#[derive(Debug, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct MarketStatus {
    /// 已写入 config.toml 的 mcp/acp 标识
    pub configured_ids: Vec<String>,
    pub node_available: bool,
    pub git_available: bool,
}

// ===== 内置目录 =====

fn mcp_catalog() -> Vec<McpMarketEntry> {
    vec![
        McpMarketEntry {
            id: "browser".into(),
            kind: "mcp".into(),
            name: "Playwright 浏览器".into(),
            description: "让 Agent 打开网页、点击、填表、截图、读取页面内容；使用本机 Chrome，适合网页自动化 / 前端联调 / 抓取".into(),
            tags: vec!["浏览器".into(), "自动化".into(), "网页".into()],
            official: true,
            recommended: true,
            recommend_reason: "最常用的 MCP 之一：微软官方维护、支持无头模式、无需账号密钥，装完即可用".into(),
            command: "npx".into(),
            args: vec!["-y".into(), "@playwright/mcp@latest".into()],
            requires: vec!["node".into()],
            npm_package: Some("@playwright/mcp".into()),
            suggested_allow: vec![
                "browser_navigate".into(),
                "browser_snapshot".into(),
                "browser_click".into(),
                "browser_type".into(),
                "browser_take_screenshot".into(),
            ],
            requires_env: vec![],
        },
        McpMarketEntry {
            id: "filesystem".into(),
            kind: "mcp".into(),
            name: "Filesystem 文件访问".into(),
            description: "暴露指定目录的读写/搜索/移动能力给 MCP 协议；适合给外部工具共享文件，或替代内置文件工具的对照测试".into(),
            tags: vec!["文件".into(), "本地".into()],
            official: true,
            recommended: false,
            recommend_reason: String::new(),
            command: "npx".into(),
            args: vec!["-y".into(), "@modelcontextprotocol/server-filesystem".into()],
            requires: vec!["node".into()],
            npm_package: Some("@modelcontextprotocol/server-filesystem".into()),
            suggested_allow: vec!["list_directory".into(), "read_file".into()],
            requires_env: vec![],
        },
        McpMarketEntry {
            id: "memory".into(),
            kind: "mcp".into(),
            name: "Memory 知识图谱记忆".into(),
            description: "基于本地知识图谱的长期记忆（实体/关系/事实），Agent 跨会话记住关键信息".into(),
            tags: vec!["记忆".into(), "知识图谱".into()],
            official: true,
            recommended: false,
            recommend_reason: String::new(),
            command: "npx".into(),
            args: vec!["-y".into(), "@modelcontextprotocol/server-memory".into()],
            requires: vec!["node".into()],
            npm_package: Some("@modelcontextprotocol/server-memory".into()),
            suggested_allow: vec!["read_graph".into()],
            requires_env: vec![],
        },
        McpMarketEntry {
            id: "sequential-thinking".into(),
            kind: "mcp".into(),
            name: "Sequential Thinking 推理".into(),
            description: "给模型提供结构化分步思考工具，复杂任务规划时逐步修订假设".into(),
            tags: vec!["推理".into(), "规划".into()],
            official: true,
            recommended: false,
            recommend_reason: String::new(),
            command: "npx".into(),
            args: vec!["-y".into(), "@modelcontextprotocol/server-sequential-thinking".into()],
            requires: vec!["node".into()],
            npm_package: Some("@modelcontextprotocol/server-sequential-thinking".into()),
            suggested_allow: vec![],
            requires_env: vec![],
        },
        McpMarketEntry {
            id: "github".into(),
            kind: "mcp".into(),
            name: "GitHub".into(),
            description: "管理 Issue / PR / 仓库 / 文件读写；需要先在本机导出 GITHUB_TOKEN 环境变量".into(),
            tags: vec!["github".into(), "协作".into()],
            official: true,
            recommended: false,
            recommend_reason: String::new(),
            command: "npx".into(),
            args: vec!["-y".into(), "@modelcontextprotocol/server-github".into()],
            requires: vec!["node".into()],
            npm_package: Some("@modelcontextprotocol/server-github".into()),
            suggested_allow: vec!["search_repositories".into(), "get_file_contents".into()],
            requires_env: vec!["GITHUB_TOKEN".into()],
        },
        McpMarketEntry {
            id: "everything".into(),
            kind: "mcp".into(),
            name: "Everything（演示/测试）".into(),
            description: "官方演示 server：回显、计算、资源订阅等小工具，用来验证 MCP 链路是否打通".into(),
            tags: vec!["测试".into(), "演示".into()],
            official: true,
            recommended: false,
            recommend_reason: "首次配置 MCP 想先跑通链路时选它：零依赖零风险，工具立即出现".into(),
            command: "npx".into(),
            args: vec!["-y".into(), "@modelcontextprotocol/server-everything".into()],
            requires: vec!["node".into()],
            npm_package: Some("@modelcontextprotocol/server-everything".into()),
            suggested_allow: vec![],
            requires_env: vec![],
        },
        McpMarketEntry {
            id: "claude".into(),
            kind: "acp".into(),
            name: "Claude Code（ACP）".into(),
            description: "把 Claude Code 作为外部编码 Agent 接入：独立会话长任务、自主调用它自己的工具；需要 Claude 订阅登录".into(),
            tags: vec!["编码".into(), "外部 agent".into()],
            official: true,
            recommended: true,
            recommend_reason: "ACP 首选：Zed 官方适配器、生态最成熟；默认 permission=confirm，外部请求逐条过宿主确认".into(),
            command: "npx".into(),
            args: vec!["-y".into(), "@zed-industries/claude-code-acp".into()],
            requires: vec!["node".into(), "claude 订阅".into()],
            npm_package: Some("@zed-industries/claude-code-acp".into()),
            suggested_allow: vec![],
            requires_env: vec![],
        },
        McpMarketEntry {
            id: "gemini".into(),
            kind: "acp".into(),
            name: "Gemini CLI（ACP）".into(),
            description: "把 Google Gemini CLI 作为外部 Agent 接入（实验性 ACP 模式）；需要 Google 账号登录".into(),
            tags: vec!["编码".into(), "外部 agent".into()],
            official: true,
            recommended: false,
            recommend_reason: String::new(),
            command: "npx".into(),
            args: vec!["-y".into(), "@google/gemini-cli".into(), "--experimental-acp".into()],
            requires: vec!["node".into(), "google 账号".into()],
            npm_package: Some("@google/gemini-cli".into()),
            suggested_allow: vec![],
            requires_env: vec![],
        },
    ]
}

fn skill_catalog() -> Vec<SkillMarketEntry> {
    let repo = "https://github.com/anthropics/skills".to_string();
    vec![
        SkillMarketEntry {
            id: "docx".into(),
            name: "docx · Word 文档".into(),
            description:
                "创建、编辑、分析 Word 文档（跟踪修订、保留格式），配合 build_docx 交付正式文稿"
                    .into(),
            tags: vec!["文档".into(), "office".into()],
            official: true,
            recommended: true,
            recommend_reason: "Anthropic 官方技能，与内置 build_docx 形成完整交付链路".into(),
            repo: repo.clone(),
            subpath: "skills/docx".into(),
            install_name: "docx".into(),
        },
        SkillMarketEntry {
            id: "pptx".into(),
            name: "pptx · PPT 演示文稿".into(),
            description: "创建、编辑、分析 PowerPoint；从大纲到成品幻灯片".into(),
            tags: vec!["文档".into(), "office".into()],
            official: true,
            recommended: false,
            recommend_reason: String::new(),
            repo: repo.clone(),
            subpath: "skills/pptx".into(),
            install_name: "pptx".into(),
        },
        SkillMarketEntry {
            id: "xlsx".into(),
            name: "xlsx · Excel 表格".into(),
            description: "创建、编辑、分析带公式/图表的电子表格".into(),
            tags: vec!["文档".into(), "office".into()],
            official: true,
            recommended: false,
            recommend_reason: String::new(),
            repo: repo.clone(),
            subpath: "skills/xlsx".into(),
            install_name: "xlsx".into(),
        },
        SkillMarketEntry {
            id: "pdf".into(),
            name: "pdf · PDF 处理".into(),
            description: "PDF 表单填写、文本/图片提取、拆分合并".into(),
            tags: vec!["文档".into(), "pdf".into()],
            official: true,
            recommended: false,
            recommend_reason: String::new(),
            repo: repo.clone(),
            subpath: "skills/pdf".into(),
            install_name: "pdf".into(),
        },
        SkillMarketEntry {
            id: "frontend-design".into(),
            name: "frontend-design · 前端设计".into(),
            description: "提升前端产出的设计审美：排版、配色、组件质感，避免模板化审美".into(),
            tags: vec!["前端".into(), "设计".into()],
            official: true,
            recommended: true,
            recommend_reason: "让 Agent 写页面质量明显提升的通用技能，推荐前端任务常备".into(),
            repo: repo.clone(),
            subpath: "skills/frontend-design".into(),
            install_name: "frontend-design".into(),
        },
        SkillMarketEntry {
            id: "webapp-testing".into(),
            name: "webapp-testing · Web 应用测试".into(),
            description: "用 Playwright 驱动真实浏览器做端到端验证与调试".into(),
            tags: vec!["测试".into(), "浏览器".into()],
            official: true,
            recommended: false,
            recommend_reason: String::new(),
            repo: repo.clone(),
            subpath: "skills/webapp-testing".into(),
            install_name: "webapp-testing".into(),
        },
        SkillMarketEntry {
            id: "mcp-builder".into(),
            name: "mcp-builder · MCP 开发指南".into(),
            description: "指导创建高质量 MCP server：schema 设计、评测、最佳实践".into(),
            tags: vec!["开发".into(), "mcp".into()],
            official: true,
            recommended: false,
            recommend_reason: String::new(),
            repo: repo.clone(),
            subpath: "skills/mcp-builder".into(),
            install_name: "mcp-builder".into(),
        },
        SkillMarketEntry {
            id: "skill-creator".into(),
            name: "skill-creator · 技能创作".into(),
            description: "指导创建自定义技能：结构、描述写法、打包规范（配合「我的 Skills」）"
                .into(),
            tags: vec!["技能".into(), "开发".into()],
            official: true,
            recommended: false,
            recommend_reason: String::new(),
            repo: repo.clone(),
            subpath: "skills/skill-creator".into(),
            install_name: "skill-creator".into(),
        },
        SkillMarketEntry {
            id: "canvas-design".into(),
            name: "canvas-design · 视觉设计".into(),
            description: "海报/视觉设计的专业设计原则：构图、色彩、字体层级".into(),
            tags: vec!["设计".into(), "视觉".into()],
            official: true,
            recommended: false,
            recommend_reason: String::new(),
            repo: repo.clone(),
            subpath: "skills/canvas-design".into(),
            install_name: "canvas-design".into(),
        },
    ]
}

// ===== 搜索 =====

fn matches(query: &str, fields: &[String]) -> bool {
    let q = query.trim().to_lowercase();
    if q.is_empty() {
        return true;
    }
    // 空格分词：全部命中才算匹配（AND）
    q.split_whitespace()
        .all(|token| fields.iter().any(|f| f.to_lowercase().contains(token)))
}

fn mcp_sort(a: &McpMarketEntry, b: &McpMarketEntry) -> std::cmp::Ordering {
    b.recommended.cmp(&a.recommended).then(a.name.cmp(&b.name))
}

fn skill_sort(a: &SkillMarketEntry, b: &SkillMarketEntry) -> std::cmp::Ordering {
    b.recommended.cmp(&a.recommended).then(a.name.cmp(&b.name))
}

#[tauri::command]
pub fn market_mcp_search(query: Option<String>) -> Vec<McpMarketEntry> {
    let q = query.unwrap_or_default();
    let mut list: Vec<McpMarketEntry> = mcp_catalog()
        .into_iter()
        .filter(|e| {
            matches(
                &q,
                &[e.name.clone(), e.description.clone(), e.tags.join(" ")],
            )
        })
        .collect();
    list.sort_by(mcp_sort);
    list
}

#[tauri::command]
pub fn market_skill_search(query: Option<String>) -> Vec<SkillMarketEntry> {
    let q = query.unwrap_or_default();
    let mut list: Vec<SkillMarketEntry> = skill_catalog()
        .into_iter()
        .filter(|e| {
            matches(
                &q,
                &[e.name.clone(), e.description.clone(), e.tags.join(" ")],
            )
        })
        .collect();
    list.sort_by(skill_sort);
    list
}

#[tauri::command]
pub fn market_status() -> MarketStatus {
    MarketStatus {
        configured_ids: configured_extension_ids(),
        node_available: which::which("npm").is_ok(),
        git_available: which::which("git").is_ok(),
    }
}

// ===== 配置文件定位与解析 =====

fn astra_config_path() -> Result<PathBuf, String> {
    let home = dirs::home_dir().ok_or("无法获取用户主目录")?;
    let dir = home.join(".astra");
    std::fs::create_dir_all(&dir).map_err(|e| format!("创建 ~/.astra 失败: {e}"))?;
    Ok(dir.join("config.toml"))
}

/// 扫描 config.toml 中已配置的扩展标识（文本级，不引入 toml 解析写回：
/// 避免重排用户文件）。返回 "mcp:<id>" / "acp:<id>"。
fn configured_extension_ids() -> Vec<String> {
    let Ok(path) = astra_config_path() else {
        return vec![];
    };
    let Ok(text) = std::fs::read_to_string(&path) else {
        return vec![];
    };
    let mut ids = vec![];
    for line in text.lines() {
        let line = line.trim();
        if let Some(rest) = line.strip_prefix("[mcp.servers.") {
            if let Some(id) = rest.strip_suffix(']') {
                if !id.is_empty() {
                    ids.push(format!("mcp:{id}"));
                }
            }
        } else if let Some(rest) = line.strip_prefix("[acp.agents.") {
            if let Some(id) = rest.strip_suffix(']') {
                if !id.is_empty() {
                    ids.push(format!("acp:{id}"));
                }
            }
        }
    }
    ids
}

/// 把一段 TOML 追加到 config.toml（EOF 追加：目标表不存在时合法；
/// 已存在的表由调用方先行检测跳过）
fn append_config_section(section: &str) -> Result<(), String> {
    let path = astra_config_path()?;
    use std::io::Write;
    let mut file = std::fs::OpenOptions::new()
        .create(true)
        .append(true)
        .open(&path)
        .map_err(|e| format!("打开 config.toml 失败: {e}"))?;
    // 段落之间空一行，保持可读
    writeln!(file, "\n{section}").map_err(|e| format!("写入 config.toml 失败: {e}"))
}

fn toml_array(values: &[String]) -> String {
    values
        .iter()
        .map(|v| format!("\"{}\"", v.replace('\\', "\\\\").replace('"', "\\\"")))
        .collect::<Vec<_>>()
        .join(", ")
}

// ===== MCP / ACP 安装 =====

#[tauri::command]
pub async fn market_mcp_install(id: String) -> Result<InstallResult, String> {
    let entry = mcp_catalog()
        .into_iter()
        .find(|e| e.id == id)
        .ok_or_else(|| format!("未知扩展: {id}"))?;

    // 1. 运行时检查（npm 缺失时 ACP/MCP 都无法拉起）
    if which::which("npm").is_err() {
        return Ok(InstallResult {
            ok: false,
            message: "未检测到 Node.js/npm，请先安装 Node（扩展运行依赖 npx）".into(),
        });
    }

    // 2. 下载预热：npm install -g（npx 运行时也会自动下载，这里提前完成
    //    避免 Agent 首次调用时卡在网络下载上；失败不阻断——运行时仍可拉取）
    let mut download_note = String::new();
    if let Some(pkg) = &entry.npm_package {
        let output = tokio::process::Command::new("npm")
            .args(["install", "-g", pkg])
            .output()
            .await
            .map_err(|e| format!("启动 npm 失败: {e}"))?;
        if !output.status.success() {
            download_note = format!(
                "（预下载失败：{}，首次调用时会自动重试下载）",
                String::from_utf8_lossy(&output.stderr)
                    .lines()
                    .next()
                    .unwrap_or("")
            );
        }
    }

    // 3. 写入 config.toml（已配置 → 直接返回）
    let configured = configured_extension_ids();
    let key = format!("{}:{}", entry.kind, entry.id);
    if configured.contains(&key) {
        return Ok(InstallResult {
            ok: true,
            message: format!("{} 已配置过，无需重复安装", entry.name),
        });
    }
    let mut section = match entry.kind.as_str() {
        "mcp" => {
            let mut s = format!(
                "[mcp.servers.{}]\nenabled = true\ntransport = \"stdio\"\ncommand = \"{}\"\nargs = [{}]\n",
                entry.id,
                entry.command,
                toml_array(&entry.args)
            );
            if !entry.requires_env.is_empty() {
                s.push_str(&format!("env = [{}]\n", toml_array(&entry.requires_env)));
            }
            if !entry.suggested_allow.is_empty() {
                s.push_str(&format!(
                    "\n[mcp.servers.{}.permissions]\nallow = [{}]\n",
                    entry.id,
                    toml_array(&entry.suggested_allow)
                ));
            }
            s
        }
        "acp" => format!(
            "[acp.agents.{}]\nenabled = true\ncommand = \"{}\"\nargs = [{}]\npermission = \"confirm\"\nsession_policy = \"workspace\"\n",
            entry.id,
            entry.command,
            toml_array(&entry.args)
        ),
        other => return Err(format!("未知扩展类型: {other}")),
    };
    if !entry.requires_env.is_empty() {
        section.push_str(&format!(
            "# 注意：需要先在终端导出 {}（配置只存变量名，不存值）\n",
            entry.requires_env.join(" / ")
        ));
    }
    append_config_section(&section)?;

    let kind_label = if entry.kind == "acp" {
        "ACP agent"
    } else {
        "MCP server"
    };
    Ok(InstallResult {
        ok: true,
        message: format!(
            "{} 安装完成（{kind_label} '{}' 已写入 ~/.astra/config.toml）{}。新建会话后生效。",
            entry.name, entry.id, download_note
        ),
    })
}

// ===== Skill 安装 =====

/// 递归拷贝目录（skill 安装用，条目数有限不做上限防御）
fn copy_dir_recursive(src: &Path, dst: &Path) -> Result<(), String> {
    std::fs::create_dir_all(dst).map_err(|e| format!("创建目录失败: {e}"))?;
    for entry in std::fs::read_dir(src).map_err(|e| format!("读取目录失败: {e}"))? {
        let entry = entry.map_err(|e| format!("读取目录项失败: {e}"))?;
        let target = dst.join(entry.file_name());
        if entry.path().is_dir() {
            copy_dir_recursive(&entry.path(), &target)?;
        } else {
            std::fs::copy(entry.path(), &target).map_err(|e| format!("复制文件失败: {e}"))?;
        }
    }
    Ok(())
}

fn skills_lib_dir() -> Result<PathBuf, String> {
    let home = dirs::home_dir().ok_or("无法获取用户主目录")?;
    let dir = home.join(".config").join("astra").join("skills");
    std::fs::create_dir_all(&dir).map_err(|e| format!("创建技能库目录失败: {e}"))?;
    Ok(dir)
}

#[tauri::command]
pub async fn market_skill_install(id: String) -> Result<InstallResult, String> {
    let entry = skill_catalog()
        .into_iter()
        .find(|e| e.id == id)
        .ok_or_else(|| format!("未知技能: {id}"))?;

    if which::which("git").is_err() {
        return Ok(InstallResult {
            ok: false,
            message: "未检测到 git，请先安装 git（技能安装通过 git clone 完成）".into(),
        });
    }

    let lib = skills_lib_dir()?;
    let target = lib.join(&entry.install_name);
    if target.exists() {
        return Ok(InstallResult {
            ok: true,
            message: format!("技能 '{}' 已存在于技能库，无需重复安装", entry.install_name),
        });
    }

    // clone --depth 1 到临时目录，再拷贝子目录（skill 可能是仓库的子目录）
    let tmp = std::env::temp_dir().join(format!("astra-skill-{}", std::process::id()));
    let _ = std::fs::remove_dir_all(&tmp);
    let output = tokio::process::Command::new("git")
        .args(["clone", "--depth", "1", &entry.repo])
        .arg(&tmp)
        .output()
        .await
        .map_err(|e| format!("启动 git 失败: {e}"))?;
    if !output.status.success() {
        let _ = std::fs::remove_dir_all(&tmp);
        return Ok(InstallResult {
            ok: false,
            message: format!(
                "克隆仓库失败: {}",
                String::from_utf8_lossy(&output.stderr)
                    .lines()
                    .next()
                    .unwrap_or("未知错误")
            ),
        });
    }

    let source = tmp.join(&entry.subpath);
    if !source.is_dir() || !source.join("SKILL.md").is_file() {
        let _ = std::fs::remove_dir_all(&tmp);
        return Ok(InstallResult {
            ok: false,
            message: format!(
                "仓库中未找到技能目录 {}（上游目录结构可能已调整）",
                entry.subpath
            ),
        });
    }

    copy_dir_recursive(&source, &target)?;
    let _ = std::fs::remove_dir_all(&tmp);

    Ok(InstallResult {
        ok: true,
        message: format!(
            "技能 '{}' 已安装到技能库，Agent 中用 /{} 直接调用",
            entry.install_name, entry.install_name
        ),
    })
}

// ===== 单测 =====

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn test_matches_and_sort() {
        assert!(matches(
            "浏览",
            &["Playwright 浏览器".into(), "浏览器".into()]
        ));
        assert!(!matches("xyz", &["Playwright 浏览器".into()]));
        // 多词 AND
        assert!(matches(
            "浏览器 自动",
            &["自动化".into(), "Playwright 浏览器".into()]
        ));
    }

    #[test]
    fn test_toml_array() {
        assert_eq!(
            toml_array(&["-y".into(), "@x/y@latest".into()]),
            "\"-y\", \"@x/y@latest\""
        );
    }

    #[test]
    fn test_catalog_nonempty() {
        assert!(mcp_catalog().len() >= 6);
        assert!(skill_catalog().len() >= 6);
    }
}
