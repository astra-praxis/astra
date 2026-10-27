//! MCP 配置自助管理工具（Agent 对话内安装/启停/移除 MCP server）。
//!
//! 边界：只写账户感知的 config.toml（账户模式 accounts/<id>/astra-home/，
//! legacy 回退 ~/.astra）的 `[mcp.servers]` 段，用 toml_edit 保留文件其余
//! 部分的注释与格式；写后用 astra-config 的 schema 校验，失败即回滚——
//! 坏配置会静默关闭整个 MCP（attach_extensions 降级路径），所以这里必须
//! 挡在写入前。
//!
//! 生效语义：agent_start_run 检测配置指纹变化自动重建 Thread，各会话的
//! 下一次对话生效（当前对话也是），无需新建会话。

use agent_core::astra_config;
use agent_core::tool::{ToolContext, ToolOutput};
use agent_core::{AgentTool, Error, Result, ToolCategory, ToolRisk};
use serde_json::{json, Value};
use std::path::PathBuf;
use toml_edit::{value, DocumentMut, Item, Table};

// ===== 路径与读写 =====

/// 配置文件路径：与 attach_extensions 读的同一份（ASTRA_HOME 覆盖一致）
fn config_path() -> PathBuf {
    astra_config::resolve_astra_home().join("config.toml")
}

/// 读取现有文件；缺失 = 空文档（首次安装时无需预置文件）
fn load_document(path: &std::path::Path) -> std::result::Result<DocumentMut, String> {
    match std::fs::read_to_string(path) {
        Ok(text) => text
            .parse::<DocumentMut>()
            .map_err(|e| format!("config.toml 解析失败: {e}")),
        Err(e) if e.kind() == std::io::ErrorKind::NotFound => Ok(DocumentMut::new()),
        Err(e) => Err(format!("读取 config.toml 失败: {e}")),
    }
}

/// 取（或创建）`[mcp.servers]` 表；中间层标记 implicit，避免渲染出空的
/// `[mcp]` / `[mcp.servers]` 头。注意 toml_edit 的 Index 对缺失键会 panic，
/// 必须走 get/get_mut。
fn servers_table_mut(doc: &mut DocumentMut) -> &mut Table {
    let root = doc.as_table_mut();
    if root.get("mcp").is_none() {
        root.insert("mcp", Item::Table(Table::new()));
    }
    let mcp = root
        .get_mut("mcp")
        .and_then(Item::as_table_mut)
        .expect("mcp is table");
    if mcp.get("servers").is_none() {
        let mut t = Table::new();
        t.set_implicit(true);
        mcp.insert("servers", Item::Table(t));
    }
    mcp.get_mut("servers")
        .and_then(Item::as_table_mut)
        .expect("servers is table")
}

/// 写前把整个文档按 astra-config schema 校验，失败返回 Err（调用方回滚）
fn validate_config_text(text: &str) -> std::result::Result<(), String> {
    let home = astra_config::resolve_astra_home();
    astra_config::parse_str(text, home)
        .map(|_| ())
        .map_err(|e| e.to_string())
}

/// 服务器 id：TOML 裸键安全字符即可，同时挡住明显误输入
fn validate_server_id(id: &str) -> std::result::Result<(), String> {
    let ok = !id.is_empty()
        && id.len() <= 64
        && id
            .chars()
            .all(|c| c.is_ascii_alphanumeric() || c == '_' || c == '-');
    if ok {
        Ok(())
    } else {
        Err(format!(
            "无效的 server id '{id}'：只允许字母/数字/下划线/连字符，长度 1-64"
        ))
    }
}

// ===== list =====

fn list_servers(path: &std::path::Path) -> Result<ToolOutput> {
    let text = std::fs::read_to_string(path).unwrap_or_default();
    let cfg = astra_config::parse_str(&text, astra_config::resolve_astra_home())
        .map_err(|e| Error::tool("mcp_config_read", e.to_string()))?;

    let path_display = path.display().to_string();
    if cfg.mcp.servers.is_empty() {
        return Ok(ToolOutput::from(format!(
            "尚未配置任何 MCP server（配置文件：{path_display}）。\
             可用 mcp_config_write 安装，例如 {{\"action\":\"upsert\",\"id\":\"blender\",\
             \"command\":\"/opt/homebrew/bin/uvx\",\"args\":[\"mcp-for-blender\"]}}"
        )));
    }

    let mut out = format!("已配置的 MCP server（{path_display}）：\n");
    for (id, s) in &cfg.mcp.servers {
        out.push_str(&format!(
            "- {id}: enabled={} command=\"{}\" args={:?} required={} allow_tools={}\n",
            s.enabled,
            s.command,
            s.args,
            s.required,
            s.permissions.allow.join(","),
        ));
    }
    out.push_str(
        "提示：修改保存后，各会话的下一次对话会自动按新配置重建（当前对话也是），无需新建会话。",
    );
    Ok(ToolOutput::from(out))
}

// ===== write（upsert / set_enabled / remove）=====

fn write_servers(path: &std::path::Path, action: &str, args: Value) -> Result<ToolOutput> {
    let id = args
        .get("id")
        .and_then(Value::as_str)
        .ok_or_else(|| Error::tool("mcp_config_write", "缺少必填参数 id"))?
        .to_string();
    validate_server_id(&id).map_err(|e| Error::tool("mcp_config_write", &e))?;

    let mut doc = load_document(path).map_err(|e| Error::tool("mcp_config_write", &e))?;
    let summary = match action {
        "upsert" => upsert_entry(&mut doc, &id, &args)?,
        "set_enabled" => {
            let enabled = args
                .get("enabled")
                .and_then(Value::as_bool)
                .ok_or_else(|| {
                    Error::tool("mcp_config_write", "set_enabled 需要布尔参数 enabled")
                })?;
            let servers = servers_table_mut(&mut doc);
            let entry = servers
                .get_mut(&id)
                .filter(|e| e.is_table())
                .ok_or_else(|| {
                    Error::tool(
                        "mcp_config_write",
                        format!("server '{id}' 不存在，先用 upsert 安装"),
                    )
                })?;
            entry["enabled"] = value(enabled);
            format!("server '{id}' 已设为 enabled={enabled}")
        }
        "remove" => {
            let servers = servers_table_mut(&mut doc);
            if servers.remove(&id).is_none() {
                return Err(Error::tool(
                    "mcp_config_write",
                    format!("server '{id}' 不存在"),
                ));
            }
            format!("server '{id}' 已移除")
        }
        other => {
            return Err(Error::tool(
                "mcp_config_write",
                format!("未知 action '{other}'，可选：upsert / set_enabled / remove"),
            ))
        }
    };

    // 原文留底：写后 schema 校验失败即回滚，绝不让坏配置落盘
    let original = std::fs::read_to_string(path).unwrap_or_default();
    let new_text = doc.to_string();
    if let Err(e) = validate_config_text(&new_text) {
        let _ = std::fs::write(path, original);
        return Err(Error::tool(
            "mcp_config_write",
            format!(
                "写入被拒绝（生成的配置未通过 schema 校验，已回滚）：{e}。\
                 若报错指向其他已有 server，可先用 action=remove 清理坏条目后再试"
            ),
        ));
    }
    if let Some(parent) = path.parent() {
        let _ = std::fs::create_dir_all(parent);
    }
    std::fs::write(path, new_text).map_err(|e| Error::tool("mcp_config_write", e.to_string()))?;

    Ok(ToolOutput::from(format!(
        "{summary}。配置文件：{}。\
         注意：MCP 工具集合在会话创建时冻结，请让用户新开会话后即可使用新 server。",
        path.display()
    )))
}

/// upsert：command 必填；args/env/required/enabled/allow 可选。
/// env 传变量名列表（值从宿主环境解析，不入文件）。
fn upsert_entry(doc: &mut DocumentMut, id: &str, args: &Value) -> Result<String> {
    let command = args
        .get("command")
        .and_then(Value::as_str)
        .map(str::trim)
        .filter(|c| !c.is_empty())
        .ok_or_else(|| Error::tool("mcp_config_write", "upsert 需要必填参数 command"))?;

    let str_vec = |key: &str| -> Result<Option<Vec<String>>> {
        match args.get(key) {
            None | Some(Value::Null) => Ok(None),
            Some(Value::Array(items)) => {
                let mut out = Vec::with_capacity(items.len());
                for v in items {
                    let s = v.as_str().filter(|s| !s.trim().is_empty()).ok_or_else(|| {
                        Error::tool(
                            "mcp_config_write",
                            format!("{key} 数组元素必须是非空字符串"),
                        )
                    })?;
                    out.push(s.to_string());
                }
                Ok(Some(out))
            }
            Some(_) => Err(Error::tool(
                "mcp_config_write",
                format!("{key} 必须是字符串数组"),
            )),
        }
    };

    let tool_args = str_vec("args")?;
    let env = str_vec("env")?;
    let allow = str_vec("allow")?;
    let required = args.get("required").and_then(Value::as_bool);
    let enabled = args.get("enabled").and_then(Value::as_bool).unwrap_or(true);

    // 净化 PATH 提示：GUI 下 spawn 不走登录 shell，裸命令名（无分隔符）历史上
    // 解析不到 node/uvx（browser MCP 踩过 os error 2）。只提示，不阻断。
    let path_hint = if !command.contains('/') && !command.contains('\\') {
        "\n警告：command 是裸命令名，GUI 环境下 PATH 已净化可能找不到可执行文件，建议用绝对路径。"
    } else {
        ""
    };

    let servers = servers_table_mut(doc);
    let mut entry = Table::new();
    entry["enabled"] = value(enabled);
    entry["transport"] = value("stdio");
    entry["command"] = value(command);
    if let Some(a) = &tool_args {
        let mut arr = toml_edit::Array::new();
        for s in a {
            arr.push(s.as_str());
        }
        entry["args"] = value(arr);
    }
    if let Some(e) = &env {
        let mut arr = toml_edit::Array::new();
        for s in e {
            arr.push(s.as_str());
        }
        entry["env"] = value(arr);
    }
    if let Some(r) = required {
        entry["required"] = value(r);
    }
    if let Some(a) = &allow {
        let mut perms = Table::new();
        let mut arr = toml_edit::Array::new();
        for s in a {
            arr.push(s.as_str());
        }
        perms["allow"] = value(arr);
        entry["permissions"] = Item::Table(perms);
    }
    servers[id] = Item::Table(entry);

    Ok(format!(
        "server '{id}' 已写入配置（command=\"{command}\"，enabled={enabled}）{path_hint}"
    ))
}

// ===== 工具封装 =====

/// 只读：列出已配置的 MCP server（Allow，无副作用）
pub struct McpConfigRead;

#[async_trait::async_trait]
impl AgentTool for McpConfigRead {
    fn name(&self) -> &str {
        "mcp_config_read"
    }
    fn description(&self) -> &str {
        "列出用户已配置的 MCP server（扩展）。用户问「装了哪些 MCP / 有没有 xx 扩展」时先用它确认现状"
    }
    fn parameters_schema(&self) -> Value {
        json!({ "type": "object", "properties": {} })
    }
    fn category(&self) -> ToolCategory {
        ToolCategory::External
    }
    fn risk(&self) -> ToolRisk {
        ToolRisk::Low
    }
    async fn call(&self, _args: Value, _ctx: &ToolContext) -> Result<ToolOutput> {
        let path = config_path();
        tokio::task::spawn_blocking(move || list_servers(&path))
            .await
            .map_err(|e| Error::tool("mcp_config_read", e.to_string()))?
    }
}

/// 写入：安装（upsert）/ 启停（set_enabled）/ 移除（remove）。Confirm 级权限。
pub struct McpConfigWrite;

#[async_trait::async_trait]
impl AgentTool for McpConfigWrite {
    fn name(&self) -> &str {
        "mcp_config_write"
    }
    fn description(&self) -> &str {
        "管理用户的 MCP server 配置（写入 ~/.astra/config.toml，需用户确认）。\
         action=upsert 安装或更新（id + command 必填，args/env/allow/required 可选；\
         env 传环境变量名，值从本机环境读取）；action=set_enabled 启停；action=remove 移除。\
         用户说「帮我装 / 接入某个 MCP」时用这里；command 尽量用绝对路径（GUI 下 PATH 净化）"
    }
    fn parameters_schema(&self) -> Value {
        json!({
            "type": "object",
            "properties": {
                "action": {
                    "type": "string",
                    "enum": ["upsert", "set_enabled", "remove"],
                    "description": "upsert=安装/更新，set_enabled=启停，remove=移除"
                },
                "id": {
                    "type": "string",
                    "description": "server 标识，字母/数字/下划线/连字符，如 blender、fetch"
                },
                "command": {
                    "type": "string",
                    "description": "upsert 必填：可执行文件，推荐绝对路径（如 /opt/homebrew/bin/uvx）"
                },
                "args": {
                    "type": "array",
                    "items": { "type": "string" },
                    "description": "upsert 可选：命令行参数，如 [\"mcp-for-blender\"] 或 [\"-y\", \"@modelcontextprotocol/server-everything\"]"
                },
                "env": {
                    "type": "array",
                    "items": { "type": "string" },
                    "description": "upsert 可选：需要转发的环境变量名列表（值从本机环境解析，不会写入文件）"
                },
                "allow": {
                    "type": "array",
                    "items": { "type": "string" },
                    "description": "upsert 可选：免确认的 MCP 工具名（只读类）；未列出的工具默认逐次确认"
                },
                "required": {
                    "type": "boolean",
                    "description": "upsert 可选：true=加载失败阻断会话启动；默认 false 降级继续"
                },
                "enabled": {
                    "type": "boolean",
                    "description": "upsert/set_enabled 可选：是否启用，默认 true"
                }
            },
            "required": ["action", "id"]
        })
    }
    fn category(&self) -> ToolCategory {
        ToolCategory::External
    }
    fn risk(&self) -> ToolRisk {
        // 写宿主配置文件（逃出工作区），但经 Confirm + 写前 schema 校验 + 失败回滚保护
        ToolRisk::High
    }
    async fn call(&self, args: Value, _ctx: &ToolContext) -> Result<ToolOutput> {
        let action = args
            .get("action")
            .and_then(Value::as_str)
            .unwrap_or_default()
            .to_string();
        let path = config_path();
        tokio::task::spawn_blocking(move || write_servers(&path, &action, args))
            .await
            .map_err(|e| Error::tool("mcp_config_write", e.to_string()))?
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn temp_config() -> (tempfile::TempDir, PathBuf) {
        let dir = tempfile::tempdir().unwrap();
        let path = dir.path().join("config.toml");
        (dir, path)
    }

    fn write_args(action: &str, id: &str, extra: Value) -> Value {
        let mut v = json!({ "action": action, "id": id });
        if let (Some(base), Some(extra)) = (v.as_object_mut(), extra.as_object()) {
            for (k, val) in extra {
                base.insert(k.clone(), val.clone());
            }
        }
        v
    }

    #[test]
    fn upsert_then_list_roundtrip() {
        let (_dir, path) = temp_config();
        // command 必须是本机存在的路径（upsert 有 fail-fast 校验）；
        // Windows 上没有 /bin/echo，用解释器路径代之，POSIX 下路径固定
        let echo = if cfg!(windows) {
            std::env::var("ComSpec").unwrap_or_else(|_| "cmd".into())
        } else {
            "/bin/echo".into()
        };
        let out = write_servers(
            &path,
            "upsert",
            write_args(
                "upsert",
                "blender",
                json!({ "command": echo, "args": ["/c", "echo", "mcp-for-blender"], "allow": ["look"] }),
            ),
        )
        .unwrap();
        assert!(out.content.contains("blender"));

        let text = std::fs::read_to_string(&path).unwrap();
        assert!(text.contains("[mcp.servers.blender]"));
        assert!(text.contains("mcp-for-blender"));

        let listed = list_servers(&path).unwrap();
        assert!(listed.content.contains("enabled=true"));
        assert!(listed.content.contains("look"));
    }

    #[test]
    fn upsert_preserves_comments_and_existing_sections() {
        let (_dir, path) = temp_config();
        let echo = if cfg!(windows) {
            std::env::var("ComSpec").unwrap_or_else(|_| "cmd".into())
        } else {
            "/bin/echo".into()
        };
        std::fs::write(
            &path,
            format!(
                "# my precious comment\n[mcp.servers.browser]\nenabled = true\ncommand = '''{echo}'''\n"
            ),
        )
        .unwrap();
        write_servers(
            &path,
            "upsert",
            write_args("upsert", "new", json!({ "command": echo })),
        )
        .unwrap();
        let text = std::fs::read_to_string(&path).unwrap();
        assert!(text.contains("# my precious comment"));
        assert!(text.contains("[mcp.servers.browser]"));
        assert!(text.contains("[mcp.servers.new]"));
    }

    #[test]
    fn set_enabled_and_remove_roundtrip() {
        let (_dir, path) = temp_config();
        let echo = if cfg!(windows) {
            std::env::var("ComSpec").unwrap_or_else(|_| "cmd".into())
        } else {
            "/bin/echo".into()
        };
        write_servers(
            &path,
            "upsert",
            write_args("upsert", "blender", json!({ "command": echo })),
        )
        .unwrap();

        write_servers(
            &path,
            "set_enabled",
            write_args("set_enabled", "blender", json!({ "enabled": false })),
        )
        .unwrap();
        let listed = list_servers(&path).unwrap();
        assert!(listed.content.contains("enabled=false"));

        write_servers(&path, "remove", write_args("remove", "blender", json!({}))).unwrap();
        let listed = list_servers(&path).unwrap();
        assert!(listed.content.contains("尚未配置任何 MCP server"));
    }

    #[test]
    fn invalid_command_is_rejected_before_write() {
        let (_dir, path) = temp_config();
        let err = write_servers(
            &path,
            "upsert",
            write_args("upsert", "bad", json!({ "command": "  " })),
        )
        .unwrap_err();
        assert!(err.to_string().contains("command"));

        // command 指向不存在的路径：schema 校验拦截（回滚保护）
        let err = write_servers(
            &path,
            "upsert",
            write_args(
                "upsert",
                "bad",
                json!({ "command": "/nonexistent/astra-tool-test-bin" }),
            ),
        )
        .unwrap_err();
        assert!(err.to_string().contains("写入被拒绝"), "{err}");
    }

    #[test]
    fn bad_id_is_rejected() {
        let (_dir, path) = temp_config();
        let err = write_servers(
            &path,
            "upsert",
            write_args("upsert", "bad id!", json!({ "command": "/bin/true" })),
        )
        .unwrap_err();
        assert!(err.to_string().contains("无效的 server id"));
    }

    #[test]
    fn remove_missing_server_errors() {
        let (_dir, path) = temp_config();
        let err =
            write_servers(&path, "remove", write_args("remove", "ghost", json!({}))).unwrap_err();
        assert!(err.to_string().contains("不存在"));
    }

    #[test]
    fn bare_command_name_gets_path_hint() {
        let (_dir, path) = temp_config();
        let out = write_servers(
            &path,
            "upsert",
            write_args("upsert", "x", json!({ "command": "npx" })),
        )
        .unwrap();
        assert!(out.content.contains("警告"));
    }
}
