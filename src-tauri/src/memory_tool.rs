//! Agent 记忆工具（对话内显式记忆 / 主动检索）。
//!
//! 边界（astra-memory docs/12）：
//! - `memory_remember`：显式写入，立即持久化（confidence 1.0，绕过阈值批
//!   处理），走引擎同一套去重——重复内容只强化不重复。项目绑定取
//!   `ctx.project_root`（与 ingest/recall 两侧同一作用域键：根目录名），
//!   无项目 = 全局（User 级，跨项目可见）。
//! - `memory_search`：主动检索，作用域 = 全局 + 当前项目（与 Run 的自动
//!   召回 REMEMBERED CONTEXT 同一可见范围）。只读，故障降级为空结果说明。
//!
//! 自动采集（用户输入 / 工具结果 / Run 结束压缩）不经过这两个工具——它们
//! 只是让 agent 在对话里能主动「记住这个」和「查一下之前」。

use agent_core::tool::{ToolContext, ToolOutput};
use agent_core::{AgentTool, Error, Result, ToolCategory, ToolRisk};
use astra_memory::cme::engine::{MemoryEngine, RememberRequest};
use astra_memory::cme::model::MemoryKind;
use astra_memory::cme::store_sqlite::CmeSqliteStore;
use serde_json::{json, Value};
use std::sync::Arc;

/// 与 agent.rs 侧 bound_project（ingest 的 set_thread_scope / recall 的
/// ProjectBoundProvider 都用它）完全同源的作用域键：项目根目录名（稳定、
/// 跨机器可复现），无 file_name 时回退全路径。
fn scope_key(ctx: &ToolContext) -> Option<String> {
    ctx.project_root.as_ref().map(|p| {
        p.file_name()
            .map(|n| n.to_string_lossy().into_owned())
            .unwrap_or_else(|| p.to_string_lossy().into_owned())
    })
}

fn limit_from(args: &Value) -> usize {
    args.get("limit")
        .and_then(Value::as_u64)
        .map(|n| (n as usize).clamp(1, 20))
        .unwrap_or(5)
}

fn kind_label(kind: MemoryKind) -> &'static str {
    match kind {
        MemoryKind::Fact => "fact",
        MemoryKind::Preference => "preference",
        MemoryKind::Decision => "decision",
        MemoryKind::Experience => "experience",
        MemoryKind::Task => "task",
    }
}

// ===== memory_remember =====

pub struct MemoryRememberTool {
    engine: Arc<MemoryEngine<CmeSqliteStore>>,
}

impl MemoryRememberTool {
    pub fn new(engine: Arc<MemoryEngine<CmeSqliteStore>>) -> Self {
        Self { engine }
    }
}

#[async_trait::async_trait]
impl AgentTool for MemoryRememberTool {
    fn name(&self) -> &str {
        "memory_remember"
    }
    fn description(&self) -> &str {
        "把一条值得跨会话复用的信息写入长期记忆（立即生效）。只记有复用价值的：\
         用户明确要求记住的偏好、已确认的决策、经工具验证的环境事实（路径/版本/配置）、\
         排查后确认的问题原因与修复方案。不要记一次性任务、普通问答、闲聊内容。\
         用户说「记住：…」「以后都…」时主动使用"
    }
    fn parameters_schema(&self) -> Value {
        json!({
            "type": "object",
            "required": ["content"],
            "properties": {
                "content": {
                    "type": "string",
                    "description": "一条完整、自包含的记忆。写清主体与范围（哪个项目/什么环境），\
                                    不要只写代词指代。例：「星野项目的 Pandoc 路径是 /opt/homebrew/bin/pandoc（2026-01 验证）」"
                },
                "kind": {
                    "type": "string",
                    "enum": ["preference", "decision", "fact", "experience", "task"],
                    "description": "记忆类型：preference=用户偏好（默认）、decision=已确认决策、\
                                    fact=环境/项目事实、experience=经验/解决方案、task=待续任务"
                }
            }
        })
    }
    fn category(&self) -> ToolCategory {
        ToolCategory::External
    }
    fn risk(&self) -> ToolRisk {
        ToolRisk::Low
    }
    async fn call(&self, args: Value, ctx: &ToolContext) -> Result<ToolOutput> {
        let content = args
            .get("content")
            .and_then(Value::as_str)
            .map(str::trim)
            .filter(|s| !s.is_empty())
            .ok_or_else(|| Error::tool("memory_remember", "content 不能为空"))?;
        let kind = match args.get("kind").and_then(Value::as_str) {
            Some("decision") => MemoryKind::Decision,
            Some("fact") => MemoryKind::Fact,
            Some("experience") => MemoryKind::Experience,
            Some("task") => MemoryKind::Task,
            _ => MemoryKind::Preference,
        };
        let req = RememberRequest {
            content: content.to_owned(),
            kind,
            project_id: scope_key(ctx),
            thread_id: None,
        };
        let memory = self
            .engine
            .remember(req)
            .await
            .map_err(|e| Error::tool("memory_remember", e.to_string()))?;
        Ok(ToolOutput::from(format!(
            "已记住（kind={}, id={}）。重复提交相同内容只会强化，不会产生重复记忆。",
            kind_label(memory.kind),
            memory.id
        )))
    }
}

// ===== memory_search =====

pub struct MemorySearchTool {
    engine: Arc<MemoryEngine<CmeSqliteStore>>,
}

impl MemorySearchTool {
    pub fn new(engine: Arc<MemoryEngine<CmeSqliteStore>>) -> Self {
        Self { engine }
    }
}

#[async_trait::async_trait]
impl AgentTool for MemorySearchTool {
    fn name(&self) -> &str {
        "memory_search"
    }
    fn description(&self) -> &str {
        "检索跨会话长期记忆（用户偏好、历史决策、工具经验、项目事实），\
         范围 = 全局 + 当前项目。当用户提到「上次/之前/我记得/我们之前决定的」\
         且对话上下文里没有答案时，先用它查，不要凭空猜测"
    }
    fn parameters_schema(&self) -> Value {
        json!({
            "type": "object",
            "required": ["query"],
            "properties": {
                "query": {
                    "type": "string",
                    "description": "检索词。必须包含具体实体词（项目名、路径、工具名等），\
                                    纯中文问句可能检索不到；多给几个关键词"
                },
                "limit": {
                    "type": "integer",
                    "description": "返回条数上限（1-20，默认 5）"
                }
            }
        })
    }
    fn category(&self) -> ToolCategory {
        ToolCategory::External
    }
    fn risk(&self) -> ToolRisk {
        ToolRisk::Low
    }
    async fn call(&self, args: Value, ctx: &ToolContext) -> Result<ToolOutput> {
        let query = args
            .get("query")
            .and_then(Value::as_str)
            .map(str::trim)
            .filter(|s| !s.is_empty())
            .ok_or_else(|| Error::tool("memory_search", "query 不能为空"))?;
        let hits = self
            .engine
            .recall_in_scope(scope_key(ctx).as_deref(), query, limit_from(&args))
            .await
            .map_err(|e| Error::tool("memory_search", e.to_string()))?;
        if hits.is_empty() {
            return Ok(ToolOutput::from(
                "记忆中没有相关内容（当前可见范围 = 全局 + 当前项目）。\
                 可以换个包含具体实体词的检索词重试，或如实告知用户没有相关记忆。"
                    .to_string(),
            ));
        }
        let mut lines = Vec::with_capacity(hits.len());
        for hit in hits {
            let kind = hit.memory.kind;
            let content = hit.memory.content.replace('\n', " ");
            lines.push(format!(
                "- [{}] (confidence {:.2}) {content}",
                kind_label(kind),
                hit.memory.confidence
            ));
        }
        Ok(ToolOutput::from(format!(
            "找到 {} 条相关记忆：\n{}",
            lines.len(),
            lines.join("\n")
        )))
    }
}

// ===== 注册 =====

/// 记忆可用（MemoryHost 初始化成功）时注册两个工具；memory-off 时一个都
/// 不注册——agent 不会看到无法调用的工具。
pub fn register(
    reg: agent_core::ToolRegistry,
    engine: Arc<MemoryEngine<CmeSqliteStore>>,
) -> agent_core::ToolRegistry {
    reg.register(MemoryRememberTool::new(Arc::clone(&engine)))
        .register(MemorySearchTool::new(engine))
}

#[cfg(test)]
mod tests {
    use super::*;
    use agent_core::permissions::{AutoBridge, Decision};
    use tokio_util::sync::CancellationToken;

    fn test_ctx(project_root: Option<std::path::PathBuf>) -> ToolContext {
        ToolContext {
            project_root,
            bridge: Arc::new(AutoBridge(Decision::Allow)),
            cancelled: CancellationToken::new(),
            deadline: None,
        }
    }

    fn test_engine() -> Arc<MemoryEngine<CmeSqliteStore>> {
        // 每次调用独立目录：并行测试共用同一 SQLite 文件会撞写锁
        static SEQ: std::sync::atomic::AtomicU64 = std::sync::atomic::AtomicU64::new(0);
        let n = SEQ.fetch_add(1, std::sync::atomic::Ordering::Relaxed);
        let dir = std::env::temp_dir().join(format!("astra-mem-tool-{}-{n}", std::process::id()));
        let _ = std::fs::remove_dir_all(&dir);
        std::fs::create_dir_all(&dir).unwrap();
        let db = dir.join("cme.db");
        let key = [11u8; 32];
        Arc::new(MemoryEngine::new(CmeSqliteStore::open(&db, &key).unwrap()))
    }

    #[tokio::test]
    async fn remember_then_search_roundtrip_with_project_scope() {
        let engine = test_engine();
        let remember = MemoryRememberTool::new(Arc::clone(&engine));
        let root = std::env::temp_dir().join("astra-mem-tool-proj");
        let ctx = test_ctx(Some(root));

        let out = remember
            .call(
                json!({"content": "星野项目的 pandoc 路径是 /opt/homebrew/bin/pandoc", "kind": "fact"}),
                &ctx,
            )
            .await
            .unwrap();
        assert!(out.content.contains("已记住"));

        // 同项目会话可召回
        let search = MemorySearchTool::new(Arc::clone(&engine));
        let hit = search
            .call(json!({"query": "pandoc 路径"}), &ctx)
            .await
            .unwrap();
        assert!(hit.content.contains("pandoc"), "got: {}", hit.content);

        // 无项目会话只见全局：项目记忆搜不到是作用域隔离，不是丢失
        let global_ctx = test_ctx(None);
        let miss = search
            .call(json!({"query": "pandoc 路径"}), &global_ctx)
            .await
            .unwrap();
        assert!(miss.content.contains("没有相关内容"));
    }

    #[tokio::test]
    async fn remember_rejects_empty_content() {
        let engine = test_engine();
        let remember = MemoryRememberTool::new(engine);
        let err = remember
            .call(json!({"content": "   "}), &test_ctx(None))
            .await
            .unwrap_err();
        assert!(err.to_string().contains("content 不能为空"));
    }

    #[tokio::test]
    async fn register_adds_both_tools() {
        let engine = test_engine();
        let reg = register(agent_core::ToolRegistry::default(), engine);
        assert!(reg.get("memory_remember").is_some());
        assert!(reg.get("memory_search").is_some());
    }
}
