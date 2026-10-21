//! 系统提示词与发往模型的纯文本统一收口层。
//!
//! 模板文件位于 `src-tauri/prompts/`，编译期经 `include_str!` 内嵌（零运行时 IO）；
//! 模板中的 `{app_name}` 占位符由 [`APP_NAME`] 常量填充。
//!
//! 修改提示词：直接编辑 `prompts/` 下的模板文件后重新编译即可，无需动 Rust 代码。
//! 全量清单（含 agent-core 侧发往模型的文本）见 `docs/PROMPTS.md`。

/// 应用名（提示词中的品牌名占位符，改这一处即全局生效）
pub const APP_NAME: &str = "astra";

/// Agent 默认系统提示词（模板：prompts/agent_system_prompt.md）。
/// 用户在 Agent 页 Settings 手填的系统提示会整体覆盖它。
pub fn agent_system_prompt() -> String {
    render(include_str!("../../prompts/agent_system_prompt.md"))
}

/// 系统提示词中的技能库列表节头（模板：prompts/agent_skills_section.md）。
/// 以 "\n\n" 起始；技能条目由调用方按 `- 名称: 描述` 逐行追加。
pub fn agent_skills_section() -> String {
    format!(
        "\n\n{}\n",
        render(include_str!("../../prompts/agent_skills_section.md"))
    )
}

/// 模板渲染：填充 {app_name} 占位符，并去掉模板文件首尾多余空白
fn render(template: &str) -> String {
    template.trim().replace("{app_name}", APP_NAME)
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn system_prompt_fills_app_name() {
        let p = agent_system_prompt();
        assert!(!p.contains("{app_name}"), "占位符必须被填充");
        assert!(p.contains(APP_NAME));
        assert!(p.starts_with("你是 astra 的 Agent 助手"));
    }

    #[test]
    fn skills_section_shape() {
        let s = agent_skills_section();
        assert!(s.starts_with("\n\n## 可用技能\n"));
        assert!(s.ends_with("：\n"));
        assert!(!s.contains("{app_name}"));
    }
}
