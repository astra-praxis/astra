//! 文案模板包：agent-core 产出的、会进入模型上下文或前端事件的所有文案，
//! 集中在此并通过配置注入（PLAN 1.1）。
//!
//! 原则（对齐 PLAN.md 风险 #2）：
//! - core 自有代码**禁止**新增硬编码的面向模型/用户的文案字符串，一律走 `TextPack`
//! - 默认 `TextPack::zh()` 与历史行为逐字一致，宿主不注入则零行为差异
//! - 模板占位符用 `{name}` 形式，替换是纯字符串操作（不做 i18n 框架引入）
//!
//! 范围说明：各工具模块内部错误文案（`Error::tool("read_file", "缺少 path")` 之类）
//! 属于工具实现方，随工具构造函数注入 `Arc<TextPack>` 渐进迁移（见 PLAN 1.1 遗留项）。

use std::sync::Arc;

/// 文案模板包。字段是模板字符串，方法负责占位符替换。
#[derive(Debug, Clone)]
pub struct TextPack {
    /// 上下文压缩提示词：整段发给模型，要求产出结构化交接摘要
    pub compaction_prompt: String,
    /// 模型调用了注册表中不存在的工具（{name}）
    pub unregistered_tool: String,
    /// 静态安全策略拒绝执行（{tool}）
    pub policy_denied: String,
    /// 用户在确认卡上拒绝了本次授权
    pub user_denied: String,
    /// 参数校验：缺少必填参数（{key}）
    pub missing_required_arg: String,
    /// 项目根目录 canonicalize 失败（{error}）
    pub invalid_project_root: String,
    /// 符号链接目标读取失败（{path} / {error}）
    pub read_link_failed: String,
    /// 达到最大续轮数预算（{max_turns}）
    pub max_turns_exceeded: String,
    /// 流空闲超时（{budget_secs}）
    pub budget_exceeded: String,
    /// 压缩摘要为空（视为失败）
    pub compaction_empty: String,
    /// 参数快照中过长字符串值的截断标注（追加在截断文本后）
    pub arg_value_truncated: String,
}

impl TextPack {
    /// 默认中文文案（与历史硬编码逐字一致）
    pub fn zh() -> Self {
        Self {
            compaction_prompt: String::from(
                "你正在把这段对话压缩成一份交接文档，后续由你基于这份摘要继续任务。\
                 请包含：\n\
                 - 目标：用户最终想达成什么\n\
                 - 进展：已完成的部分、当前阻塞、已确定的决策\n\
                 - 关键上下文：约束、偏好、必需的数据/路径/示例\n\
                 - 下一步：剩余的具体步骤\n\
                 - 踩坑：尝试过但无效的做法\n\
                 要求：直接输出摘要正文，简洁、结构化，不寒暄。",
            ),
            unregistered_tool: String::from("未注册的工具: {name}"),
            policy_denied: String::from("工具 '{tool}' 被安全策略拒绝执行"),
            user_denied: String::from("用户拒绝了本次操作授权"),
            missing_required_arg: String::from("缺少必填参数: {key}"),
            invalid_project_root: String::from("项目根目录无效: {error}"),
            read_link_failed: String::from("{path}: {error}"),
            max_turns_exceeded: String::from(
                "已达最大续轮数（{max_turns} 轮），任务未完成；请拆小任务重试",
            ),
            budget_exceeded: String::from("运行超时（预算 {budget_secs}s）"),
            compaction_empty: String::from("摘要为空"),
            arg_value_truncated: String::from("\n…（内容过长已截断）"),
        }
    }

    fn fill(template: &str, pairs: &[(&str, String)]) -> String {
        let mut out = template.to_string();
        for (key, value) in pairs {
            out = out.replace(&format!("{{{key}}}"), value);
        }
        out
    }

    pub fn unregistered_tool(&self, name: &str) -> String {
        Self::fill(&self.unregistered_tool, &[("name", name.to_string())])
    }

    pub fn policy_denied(&self, tool: &str) -> String {
        Self::fill(&self.policy_denied, &[("tool", tool.to_string())])
    }

    pub fn user_denied(&self) -> String {
        self.user_denied.clone()
    }

    pub fn missing_required_arg(&self, key: &str) -> String {
        Self::fill(&self.missing_required_arg, &[("key", key.to_string())])
    }

    pub fn invalid_project_root(&self, error: &str) -> String {
        Self::fill(&self.invalid_project_root, &[("error", error.to_string())])
    }

    pub fn read_link_failed(&self, path: &str, error: &str) -> String {
        Self::fill(
            &self.read_link_failed,
            &[("path", path.to_string()), ("error", error.to_string())],
        )
    }

    pub fn max_turns_exceeded(&self, max_turns: u32) -> String {
        Self::fill(
            &self.max_turns_exceeded,
            &[("max_turns", max_turns.to_string())],
        )
    }

    pub fn budget_exceeded(&self, budget_secs: u64) -> String {
        Self::fill(
            &self.budget_exceeded,
            &[("budget_secs", budget_secs.to_string())],
        )
    }

    pub fn compaction_empty(&self) -> String {
        self.compaction_empty.clone()
    }

    pub fn arg_value_truncated(&self) -> &str {
        &self.arg_value_truncated
    }
}

impl Default for TextPack {
    fn default() -> Self {
        Self::zh()
    }
}

/// 共享文案包（RunConfig / ThreadConfig 持有）
pub type SharedTextPack = Arc<TextPack>;

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn zh_defaults_match_historical_strings() {
        let pack = TextPack::zh();
        assert_eq!(pack.unregistered_tool("mcp"), "未注册的工具: mcp");
        assert_eq!(pack.policy_denied("exec"), "工具 'exec' 被安全策略拒绝执行");
        assert_eq!(pack.user_denied(), "用户拒绝了本次操作授权");
        assert_eq!(pack.missing_required_arg("path"), "缺少必填参数: path");
        assert_eq!(
            pack.invalid_project_root("enoent"),
            "项目根目录无效: enoent"
        );
        assert_eq!(
            pack.max_turns_exceeded(50),
            "已达最大续轮数（50 轮），任务未完成；请拆小任务重试"
        );
        assert_eq!(pack.budget_exceeded(120), "运行超时（预算 120s）");
        assert_eq!(pack.compaction_empty(), "摘要为空");
        assert!(pack.compaction_prompt.contains("交接文档"));
    }

    #[test]
    fn custom_pack_replaces_text_for_injection() {
        // 宿主注入英文文案：机制验证，不做完整英文包
        let mut pack = TextPack::zh();
        pack.unregistered_tool = "Unregistered tool: {name}".into();
        pack.max_turns_exceeded = "Turn budget exhausted ({max_turns})".into();
        assert_eq!(pack.unregistered_tool("mcp"), "Unregistered tool: mcp");
        assert_eq!(pack.max_turns_exceeded(7), "Turn budget exhausted (7)");
        // 未覆盖的模板仍是中文默认
        assert_eq!(pack.user_denied(), "用户拒绝了本次操作授权");
    }

    #[test]
    fn placeholders_in_values_do_not_recursively_expand() {
        // 值里带 {tool} 不应被再次替换（防注入，替换是单向单层）
        let pack = TextPack::zh();
        let msg = pack.policy_denied("{user_denied}");
        assert_eq!(msg, "工具 '{user_denied}' 被安全策略拒绝执行");
    }

    #[test]
    fn default_trait_is_zh() {
        assert_eq!(
            TextPack::default().user_denied(),
            TextPack::zh().user_denied()
        );
    }
}
