//! 技能工具：list_skills / read_skill（只读：发现与读取用户技能库）。
//!
//! 写入（新建/导入/同步/删除）只在宿主的「我的 Skills」页面进行，
//! 这里只搬读路径（frontmatter 解析 + 校验 + 截断），宿主 user_skills.rs
//! 通过 re-export 复用同一实现，避免双份逻辑漂移。

use crate::error::{Error, Result};
use crate::permissions::ensure_within_root;
use crate::tool::{AgentTool, ToolContext, ToolOutput};
use async_trait::async_trait;
use serde_json::{json, Value};
use std::fs;
use std::path::{Path, PathBuf};

/// 单个技能文件读取上限（字符数），超过截断并提示
const SKILL_READ_MAX_CHARS: usize = 30_000;
/// 技能摘要里单条描述的最大长度（防 system prompt 被超长描述撑爆）
const SKILL_DESC_MAX_CHARS: usize = 120;

/// Agent 侧的轻量摘要（注入 system prompt / 前端）
#[derive(Debug, Clone)]
pub struct SkillSummary {
    pub name: String,
    pub description: String,
}

/// 技能名（目录名）校验：非空、不以 . 开头、不含路径分隔符与 ..
pub fn validate_skill_name(name: &str) -> std::result::Result<(), String> {
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
pub fn validate_rel_path(rel: &str) -> std::result::Result<(), String> {
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

/// 从 SKILL.md frontmatter 提取指定字段（name/description 等），单行、去可选引号
fn parse_frontmatter_field(content: &str, field: &str) -> String {
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
    let prefix = format!("{field}:");
    for line in rest[..end].lines() {
        if let Some(v) = line.trim().strip_prefix(&prefix) {
            return v.trim().trim_matches('"').trim_matches('\'').to_string();
        }
    }
    String::new()
}

fn skill_description(skill_dir: &Path) -> String {
    fs::read_to_string(skill_dir.join("SKILL.md"))
        .map(|c| parse_frontmatter_field(&c, "description"))
        .unwrap_or_default()
}

/// Agent 用摘要列表（name = 目录名，description = SKILL.md frontmatter）
pub fn summary_in(dir: &Path) -> Vec<SkillSummary> {
    let Ok(entries) = fs::read_dir(dir) else {
        return Vec::new();
    };
    let mut skills: Vec<SkillSummary> = entries
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
            Some(SkillSummary {
                description: skill_description(&path),
                name,
            })
        })
        .collect();
    skills.sort_by(|a, b| a.name.cmp(&b.name));
    skills
}

/// 读取技能内容：默认 SKILL.md，可指定技能内相对路径。超长截断。
pub fn read_skill_content(
    dir: &Path,
    name: &str,
    file: Option<&str>,
) -> std::result::Result<String, String> {
    validate_skill_name(name)?;
    let skill_dir = dir.join(name.trim());
    if !skill_dir.is_dir() {
        return Err(format!("技能 '{name}' 不存在"));
    }
    let rel = file.unwrap_or("SKILL.md");
    validate_rel_path(rel)?;
    let full =
        ensure_within_root(&skill_dir, Path::new(rel)).map_err(|e| format!("路径校验失败: {e}"))?;
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

/// 截断技能描述到 system prompt 安全长度
pub fn truncate_desc(desc: &str) -> String {
    let desc = desc.trim();
    if desc.chars().count() <= SKILL_DESC_MAX_CHARS {
        return desc.to_string();
    }
    let truncated: String = desc.chars().take(SKILL_DESC_MAX_CHARS).collect();
    format!("{truncated}…")
}

struct ListSkills {
    dir: PathBuf,
}

#[async_trait]
impl AgentTool for ListSkills {
    fn name(&self) -> &str {
        "list_skills"
    }
    fn description(&self) -> &str {
        "列出用户的技能库（名称 + 描述）。任务与某技能相关时，用 read_skill 读取完整说明后遵循执行"
    }
    fn parameters_schema(&self) -> Value {
        json!({ "type": "object", "properties": {} })
    }
    async fn call(&self, _args: Value, _ctx: &ToolContext) -> Result<ToolOutput> {
        let skills = tokio::task::spawn_blocking({
            let dir = self.dir.clone();
            move || summary_in(&dir)
        })
        .await
        .map_err(|e| Error::tool("list_skills", e.to_string()))?;

        if skills.is_empty() {
            return Ok(ToolOutput::from("技能库为空".to_string()));
        }
        let mut out = String::from("可用技能：\n");
        for s in skills {
            out.push_str(&format!(
                "- {}: {}\n",
                s.name,
                truncate_desc(&s.description)
            ));
        }
        Ok(ToolOutput::from(out))
    }
}

struct ReadSkill {
    dir: PathBuf,
}

#[async_trait]
impl AgentTool for ReadSkill {
    fn name(&self) -> &str {
        "read_skill"
    }
    fn description(&self) -> &str {
        "读取一个技能的说明文档（默认 SKILL.md）。先用 list_skills 或系统提示中的技能列表确定技能名"
    }
    fn parameters_schema(&self) -> Value {
        json!({
            "type": "object",
            "properties": {
                "name": { "type": "string", "description": "技能名（/ 调用时用的名字）" },
                "file": { "type": "string", "description": "可选：技能内相对路径，默认 SKILL.md" }
            },
            "required": ["name"]
        })
    }
    async fn call(&self, args: Value, _ctx: &ToolContext) -> Result<ToolOutput> {
        let name = args
            .get("name")
            .and_then(|v| v.as_str())
            .ok_or_else(|| Error::tool("read_skill", "缺少 name"))?;
        let file = args.get("file").and_then(|v| v.as_str());
        let dir = self.dir.clone();
        let name = name.to_string();
        let file = file.map(|s| s.to_string());
        let content =
            tokio::task::spawn_blocking(move || read_skill_content(&dir, &name, file.as_deref()))
                .await
                .map_err(|e| Error::tool("read_skill", e.to_string()))?
                .map_err(|e| Error::tool("read_skill", e))?;
        Ok(ToolOutput::from(content))
    }
}

/// 注册技能工具（不依赖项目根，纯聊天也可用）
pub fn register(reg: crate::tool::ToolRegistry, dir: &Path) -> crate::tool::ToolRegistry {
    reg.register(ListSkills {
        dir: dir.to_path_buf(),
    })
    .register(ReadSkill {
        dir: dir.to_path_buf(),
    })
}

#[cfg(test)]
mod tests {
    use super::*;

    fn tmp_dir(tag: &str) -> PathBuf {
        let dir =
            std::env::temp_dir().join(format!("agent-core-skills-{tag}-{}", std::process::id()));
        let _ = fs::remove_dir_all(&dir);
        fs::create_dir_all(&dir).unwrap();
        dir
    }

    fn ctx() -> ToolContext {
        ToolContext {
            project_root: None,
            bridge: std::sync::Arc::new(crate::permissions::AutoBridge(
                crate::permissions::Decision::Allow,
            )),
            cancelled: tokio_util::sync::CancellationToken::new(),
        }
    }

    #[test]
    fn frontmatter_parsing_extracts_quoted_fields() {
        let md = "---\nname: \"demo\"\ndescription: 测试技能\n---\n\n正文";
        assert_eq!(parse_frontmatter_field(md, "name"), "demo");
        assert_eq!(parse_frontmatter_field(md, "description"), "测试技能");
        assert_eq!(parse_frontmatter_field("无 frontmatter", "name"), "");
        assert_eq!(parse_frontmatter_field("---\n未闭合", "name"), "");
    }

    #[test]
    fn validators_reject_bad_names_and_paths() {
        assert!(validate_skill_name("").is_err());
        assert!(validate_skill_name(".hidden").is_err());
        assert!(validate_skill_name("..").is_err());
        assert!(validate_skill_name("a/b").is_err());
        assert!(validate_skill_name("a\\b").is_err());
        assert!(validate_skill_name(" demo ").is_ok());

        assert!(validate_rel_path("/abs").is_err());
        assert!(validate_rel_path("../up").is_err());
        assert!(validate_rel_path("a\\b").is_err());
        assert!(validate_rel_path("").is_err());
        assert!(validate_rel_path("docs/ref.md").is_ok());
    }

    #[test]
    fn truncate_desc_limits_length() {
        let long = "长".repeat(200);
        let out = truncate_desc(&long);
        assert_eq!(out.chars().count(), SKILL_DESC_MAX_CHARS + 1); // 含省略号
        assert!(out.ends_with('…'));
        assert_eq!(truncate_desc("短描述"), "短描述");
    }

    #[tokio::test]
    async fn summary_and_read_roundtrip() {
        let dir = tmp_dir("roundtrip");
        fs::create_dir_all(dir.join("demo")).unwrap();
        fs::write(
            dir.join("demo/SKILL.md"),
            "---\nname: demo\ndescription: 演示技能\n---\n\n# demo\n\n步骤内容",
        )
        .unwrap();
        fs::create_dir_all(dir.join("demo/docs")).unwrap();
        fs::write(dir.join("demo/docs/ref.md"), "参考内容").unwrap();
        // 隐藏目录应被跳过
        fs::create_dir_all(dir.join(".hidden")).unwrap();

        let skills = summary_in(&dir);
        assert_eq!(skills.len(), 1);
        assert_eq!(skills[0].name, "demo");
        assert_eq!(skills[0].description, "演示技能");

        let content = read_skill_content(&dir, "demo", None).unwrap();
        assert!(content.contains("步骤内容"));
        let ref_doc = read_skill_content(&dir, "demo", Some("docs/ref.md")).unwrap();
        assert_eq!(ref_doc, "参考内容");

        // 工具层
        let tool_out = ReadSkill { dir: dir.clone() }
            .call(json!({"name": "demo"}), &ctx())
            .await
            .unwrap();
        assert!(tool_out.content.contains("步骤内容"));

        let list_out = ListSkills { dir: dir.clone() }
            .call(json!({}), &ctx())
            .await
            .unwrap();
        assert!(list_out.content.contains("- demo: 演示技能"));

        fs::remove_dir_all(&dir).unwrap();
    }

    #[tokio::test]
    async fn read_rejects_traversal_and_missing_skill() {
        let dir = tmp_dir("reject");
        fs::create_dir_all(dir.join("demo")).unwrap();
        fs::write(dir.join("demo/SKILL.md"), "x").unwrap();

        assert!(read_skill_content(&dir, "demo", Some("../../etc/passwd")).is_err());
        assert!(read_skill_content(&dir, "missing", None).is_err());
        // 符号链接逃逸
        #[cfg(unix)]
        {
            std::os::unix::fs::symlink("/etc", dir.join("demo/link")).unwrap();
            assert!(read_skill_content(&dir, "demo", Some("link/passwd")).is_err());
        }
        fs::remove_dir_all(&dir).unwrap();
    }

    #[tokio::test]
    async fn long_content_is_truncated() {
        let dir = tmp_dir("trunc");
        fs::create_dir_all(dir.join("big")).unwrap();
        fs::write(
            dir.join("big/SKILL.md"),
            "字".repeat(SKILL_READ_MAX_CHARS + 500),
        )
        .unwrap();
        let out = ReadSkill { dir: dir.clone() }
            .call(json!({"name": "big"}), &ctx())
            .await
            .unwrap();
        assert!(out.content.contains("已截断"));
        fs::remove_dir_all(&dir).unwrap();
    }

    #[tokio::test]
    async fn empty_library_reports_empty() {
        let dir = tmp_dir("empty");
        let out = ListSkills { dir: dir.clone() }
            .call(json!({}), &ctx())
            .await
            .unwrap();
        assert_eq!(out.content, "技能库为空");
        fs::remove_dir_all(&dir).unwrap();
    }
}
