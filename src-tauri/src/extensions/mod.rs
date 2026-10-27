//! 扩展管理（MCP / ACP 配置面）：
//!
//! - 读侧走 astra-config 冻结 schema（`load()`，deny_unknown_fields，
//!   解析失败原样暴露给 UI，不让坏配置静默消失）
//! - 写侧保持文本级修改（与 market.rs 同策略）：只动目标段内的目标键，
//!   不整文件重排，保留用户注释与未托管键
//! - 修改后各会话的下一次对话自动生效：agent_start_run 检测 config.toml
//!   指纹变化并重建缓存 Thread（历史在 SQLite，无损恢复），无需新建会话
//!
//! 段定位规则：`[mcp.servers.<id>]` 起始，遇到下一个非子表 `[` 行结束
//! （子表如 `[mcp.servers.<id>.permissions]` 归属本段）。

pub(crate) mod market;
pub(crate) mod mcp_config_tool;

use agent_core::astra_config;
use serde::Serialize;

// ===== DTO =====

#[derive(Debug, Clone, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct McpExtensionInfo {
    pub id: String,
    pub enabled: bool,
    pub transport: String,
    pub command: String,
    pub args: Vec<String>,
    /// 环境变量名（值在宿主环境，不在配置文件）
    pub env: Vec<String>,
    pub required: bool,
    /// 权限摘要（allow / deny 工具名，其余默认 Confirm）
    pub allow: Vec<String>,
    pub deny: Vec<String>,
    pub call_timeout_secs: u64,
}

#[derive(Debug, Clone, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct AcpExtensionInfo {
    pub id: String,
    pub enabled: bool,
    pub command: String,
    pub args: Vec<String>,
    pub env: Vec<String>,
    pub required: bool,
    /// deny | confirm | allow
    pub permission: String,
    /// none | workspace | persistent
    pub session_policy: String,
}

#[derive(Debug, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct ExtensionsList {
    pub mcp: Vec<McpExtensionInfo>,
    pub acp: Vec<AcpExtensionInfo>,
    pub config_path: String,
    /// 配置解析失败时的错误（坏配置也能在 UI 看到，而不是空白）
    pub parse_error: Option<String>,
}

// ===== 读取 =====

#[tauri::command]
pub fn extensions_list() -> Result<ExtensionsList, String> {
    let path = crate::extensions::market::astra_config_path()?;
    let config_path = path.display().to_string();
    match astra_config::load() {
        Ok(cfg) => Ok(ExtensionsList {
            mcp: cfg
                .mcp
                .servers
                .iter()
                .map(|(id, e)| McpExtensionInfo {
                    id: id.clone(),
                    enabled: e.enabled,
                    transport: e.transport.clone(),
                    command: e.command.clone(),
                    args: e.args.clone(),
                    env: e.env.clone(),
                    required: e.required,
                    allow: e.permissions.allow.clone(),
                    deny: e.permissions.deny.clone(),
                    call_timeout_secs: e.limits.call_timeout_secs,
                })
                .collect(),
            acp: cfg
                .acp
                .agents
                .iter()
                .map(|(id, a)| AcpExtensionInfo {
                    id: id.clone(),
                    enabled: a.enabled,
                    command: a.command.clone(),
                    args: a.args.clone(),
                    env: a.env.clone(),
                    required: a.required,
                    permission: a.permission.clone(),
                    session_policy: a.session_policy.clone(),
                })
                .collect(),
            config_path,
            parse_error: None,
        }),
        Err(e) => Ok(ExtensionsList {
            mcp: vec![],
            acp: vec![],
            config_path,
            parse_error: Some(e.to_string()),
        }),
    }
}

// ===== 文本级段操作 =====

/// 段边界：返回 (起始行索引, 结束行索引)（不含结束行）。
/// 段 = 主 header 行 + 全部子表（`[header.xxx]`）直到下一个无关 `[` 行。
fn section_bounds(lines: &[&str], header: &str) -> Option<(usize, usize)> {
    let start = lines.iter().position(|l| l.trim() == header)?;
    let child_prefix = format!("[{}.", header.trim_matches(|c| c == '[' || c == ']'));
    let mut end = lines.len();
    for (i, line) in lines.iter().enumerate().skip(start + 1) {
        let t = line.trim();
        if t.starts_with('[') && !t.starts_with(&child_prefix) {
            end = i;
            break;
        }
    }
    Some((start, end))
}

/// 在段内设置一批键值（键不存在则插在段首，存在则原位替换；
/// 数组值跨行时整块吞掉直到方括号配平）。
fn set_keys_in_section(
    text: &str,
    header: &str,
    keys: &[(&str, String)],
) -> Result<String, String> {
    let lines: Vec<&str> = text.lines().collect();
    let (start, end) =
        section_bounds(&lines, header).ok_or_else(|| format!("配置中未找到段 {header}"))?;

    // 逐键处理：新值插段首，段内所有同名键行全部吞掉（含历史重复行，可自愈）
    let mut out: Vec<String> = lines[..start].iter().map(|s| s.to_string()).collect();
    out.push(header.to_string());
    let managed: std::collections::HashSet<&str> = keys.iter().map(|(k, _)| *k).collect();
    // 新键插在段首（紧跟 header）；同名旧键后面全部吞掉（含历史重复行，可自愈）
    for (k, v) in keys {
        out.push(format!("{k} = {v}"));
    }

    let mut i = start + 1;
    while i < end {
        let line = lines[i];
        let trimmed = line.trim();
        let key_of = trimmed.split('=').next().unwrap_or("").trim().to_string();
        if managed.contains(key_of.as_str()) {
            // 同名旧键：新值已插在段首，这里只吞掉原行（含跨行数组，含历史重复行）
            let mut depth =
                trimmed.matches('[').count() as i32 - trimmed.matches(']').count() as i32;
            let mut consumed = 1;
            while depth > 0 && i + consumed < end {
                let l = lines[i + consumed];
                depth += l.matches('[').count() as i32 - l.matches(']').count() as i32;
                consumed += 1;
            }
            i += consumed;
        } else {
            out.push(line.to_string());
            i += 1;
        }
    }
    out.extend(lines[end..].iter().map(|s| s.to_string()));
    Ok(out.join("\n") + if text.ends_with('\n') { "\n" } else { "" })
}

/// 删除整段（含子表）
fn remove_section(text: &str, header: &str) -> Result<String, String> {
    let lines: Vec<&str> = text.lines().collect();
    let (start, end) =
        section_bounds(&lines, header).ok_or_else(|| format!("配置中未找到段 {header}"))?;
    let mut out: Vec<&str> = Vec::new();
    out.extend_from_slice(&lines[..start]);
    out.extend_from_slice(&lines[end..]);
    let mut joined = out.join("\n");
    if text.ends_with('\n') && !joined.ends_with('\n') {
        joined.push('\n');
    }
    Ok(joined)
}

fn quote_toml_str(v: &str) -> String {
    format!("\"{}\"", v.replace('\\', "\\\\").replace('"', "\\\""))
}

fn section_header(kind: &str, id: &str) -> Result<String, String> {
    match kind {
        "mcp" => Ok(format!("[mcp.servers.{id}]")),
        "acp" => Ok(format!("[acp.agents.{id}]")),
        other => Err(format!("未知扩展类型: {other}")),
    }
}

// ===== 写入命令 =====

fn load_config_text() -> Result<String, String> {
    let path = crate::extensions::market::astra_config_path()?;
    std::fs::read_to_string(&path).or_else(|e| {
        if e.kind() == std::io::ErrorKind::NotFound {
            Ok(String::new())
        } else {
            Err(format!("读取配置失败: {e}"))
        }
    })
}

fn save_config_text(text: &str) -> Result<(), String> {
    let path = crate::extensions::market::astra_config_path()?;
    // 原子写：临时文件 + rename，避免半写状态
    let tmp = path.with_extension("toml.tmp");
    std::fs::write(&tmp, text).map_err(|e| format!("写入临时文件失败: {e}"))?;
    std::fs::rename(&tmp, &path).map_err(|e| format!("替换配置失败: {e}"))
}

/// 启用 / 停用（enabled = false 后装配直接跳过；保存后各会话下次对话自动重建生效）
#[tauri::command]
pub fn extensions_set_enabled(kind: String, id: String, enabled: bool) -> Result<(), String> {
    let header = section_header(&kind, &id)?;
    let text = load_config_text()?;
    let new_text = set_keys_in_section(&text, &header, &[("enabled", enabled.to_string())])?;
    save_config_text(&new_text)
}

/// 删除扩展配置段（含 permissions / limits 子表）
#[tauri::command]
pub fn extensions_remove(kind: String, id: String) -> Result<(), String> {
    let header = section_header(&kind, &id)?;
    let text = load_config_text()?;
    let new_text = remove_section(&text, &header)?;
    save_config_text(&new_text)
}

/// 编辑核心运行参数（command / args / env 名单 / required）。
/// 不动的键（permissions / limits / enabled）原样保留。
#[tauri::command]
pub fn extensions_update(
    kind: String,
    id: String,
    command: String,
    args: Vec<String>,
    env: Vec<String>,
    required: bool,
) -> Result<(), String> {
    let header = section_header(&kind, &id)?;
    if command.trim().is_empty() {
        return Err("command 不能为空".into());
    }
    for name in &env {
        if name.trim().is_empty() || name.contains('=') {
            return Err(format!(
                "环境变量名无效: '{name}'（只填变量名，值在宿主环境导出）"
            ));
        }
    }
    let toml = crate::extensions::market::toml_array;
    let keys = [
        ("command", quote_toml_str(command.trim())),
        (
            "args",
            format!(
                "[{}]",
                toml(
                    &args
                        .iter()
                        .map(|s| s.trim().to_string())
                        .collect::<Vec<_>>()
                )
            ),
        ),
        (
            "env",
            format!(
                "[{}]",
                toml(&env.iter().map(|s| s.trim().to_string()).collect::<Vec<_>>())
            ),
        ),
        ("required", required.to_string()),
    ];
    let text = load_config_text()?;
    let new_text = set_keys_in_section(&text, &header, &keys)?;
    save_config_text(&new_text)
}

// ===== 单测 =====

#[cfg(test)]
mod tests {
    use super::*;

    const SAMPLE: &str = "# top comment\n\
[mcp.servers.browser]\nenabled = true\ncommand = \"npx\"\nargs = [\"-y\", \"@playwright/mcp\"]\n\n\
[mcp.servers.browser.permissions]\nallow = [\"browser_navigate\"]\n\n\
[mcp.servers.fs]\ncommand = \"npx\"\nargs = [\n  \"-y\",\n  \"@modelcontextprotocol/server-filesystem\",\n]\n\n\
[acp.agents.claude]\nenabled = false\ncommand = \"claude\"\n";

    #[test]
    fn test_set_enabled_existing_key() {
        let out = set_keys_in_section(
            SAMPLE,
            "[mcp.servers.browser]",
            &[("enabled", "false".into())],
        )
        .unwrap();
        assert!(out.contains("[mcp.servers.browser]\nenabled = false"));
        // 子表不动
        assert!(out.contains("allow = [\"browser_navigate\"]"));
        // 其他段不动
        assert!(out.contains("[acp.agents.claude]\nenabled = false"));
    }

    #[test]
    fn test_set_enabled_missing_key_inserted() {
        // fs 段没有 enabled 键 → 插到段首
        let out =
            set_keys_in_section(SAMPLE, "[mcp.servers.fs]", &[("enabled", "true".into())]).unwrap();
        let seg = out.split("[mcp.servers.fs]").nth(1).unwrap();
        assert!(seg.starts_with("\nenabled = true\ncommand"));
    }

    #[test]
    fn test_replace_multiline_array() {
        let out = set_keys_in_section(
            SAMPLE,
            "[mcp.servers.fs]",
            &[("args", "[\"-y\", \"other\"]".into())],
        )
        .unwrap();
        assert!(out.contains("args = [\"-y\", \"other\"]"));
        assert!(!out.contains("@modelcontextprotocol/server-filesystem"));
        // 后续段保留
        assert!(out.contains("[acp.agents.claude]"));
    }

    #[test]
    fn test_remove_section_with_children() {
        let out = remove_section(SAMPLE, "[mcp.servers.browser]").unwrap();
        assert!(!out.contains("@playwright/mcp"));
        assert!(!out.contains("browser_navigate"));
        // 后续段原样保留
        assert!(out.contains("[mcp.servers.fs]"));
        assert!(out.contains("[acp.agents.claude]"));
        assert!(out.contains("# top comment"));
    }

    #[test]
    fn test_remove_missing_section_errors() {
        assert!(remove_section(SAMPLE, "[mcp.servers.nope]").is_err());
    }

    #[test]
    fn test_duplicate_key_self_heal() {
        // 模拟历史 bug 造成的段内重复键：set 后所有旧键行都应被吞掉，只留一个
        let sample = "[mcp.servers.browser]\nenabled = false\ncommand = \"npx\"\nenabled = false\n";
        let out = set_keys_in_section(
            sample,
            "[mcp.servers.browser]",
            &[("enabled", "true".into())],
        )
        .unwrap();
        assert_eq!(out.matches("enabled").count(), 1);
        assert!(out.contains("enabled = true"));
        assert!(out.contains("command = \"npx\""));
    }
}
