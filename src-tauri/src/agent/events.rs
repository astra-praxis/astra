use agent_core::Envelope;
use serde_json::Value;
use tauri::AppHandle;

// ===== 事件日志（JSONL，一行一个 Envelope，供排查对话问题） =====

/// 截断记忆采集用的长文本（按字符，不劈开 UTF-8 序列），附带长度标记。
pub(crate) fn truncate_for_memory(s: &str, max_chars: usize) -> String {
    if s.chars().count() <= max_chars {
        return s.to_owned();
    }
    let head: String = s.chars().take(max_chars).collect();
    format!("{head}…[truncated]")
}

/// 记忆采集器：从 Run 事件流中提取值得进入记忆管线的内容，纯逻辑、
/// 无 AppHandle 依赖——宿主 observe spawn 与集成测试共用同一段代码。
///
/// 采集边界：助手最终回答（unverified，只进摘要不成记忆）、已验证成功的
/// 工具结果、工具失败原因（进摘要，解决方案需验证后才进记忆）。内容截断，
/// 记忆模块不需要完整工具输出。MessageDelta / Started / Permission 等忽略。
pub(crate) struct MemoryRunCollector {
    collected: Vec<(astra_memory::cme::compactor::MessageRole, String, String)>,
}

impl MemoryRunCollector {
    pub(crate) fn new() -> Self {
        Self {
            collected: Vec::new(),
        }
    }

    pub(crate) fn observe(&mut self, env: &Envelope) {
        match &env.event {
            agent_core::RunEvent::RunCompleted { final_text } => {
                let text = final_text.trim();
                if text.chars().count() >= 6 {
                    self.collected.push((
                        astra_memory::cme::compactor::MessageRole::Assistant,
                        truncate_for_memory(text, 2000),
                        env.event_id.clone(),
                    ));
                }
            }
            agent_core::RunEvent::ToolCallCompleted {
                tool_name,
                result_summary,
                raw_output,
                ..
            } => {
                let body = match raw_output {
                    Some(raw) if !raw.trim().is_empty() => truncate_for_memory(raw.trim(), 2000),
                    _ => truncate_for_memory(result_summary.trim(), 2000),
                };
                if !body.is_empty() {
                    self.collected.push((
                        astra_memory::cme::compactor::MessageRole::Tool,
                        format!("{tool_name}: {body}"),
                        env.event_id.clone(),
                    ));
                }
            }
            agent_core::RunEvent::ToolCallFailed {
                tool_name, error, ..
            } => {
                let body = error.trim();
                if !body.is_empty() {
                    self.collected.push((
                        astra_memory::cme::compactor::MessageRole::Tool,
                        format!(
                            "[tool failed] {tool_name}: {}",
                            truncate_for_memory(body, 2000)
                        ),
                        env.event_id.clone(),
                    ));
                }
            }
            _ => {}
        }
    }

    /// 组装 ingest 消息：用户输入在前，采集到的运行事件随后。id 用事件
    /// event_id（重放稳定）；空则回退到时间序合成 id。
    pub(crate) fn into_messages(
        self,
        user_text: &str,
        now_ms: u64,
    ) -> Vec<astra_memory::cme::compactor::ConversationMessage> {
        let mut messages: Vec<astra_memory::cme::compactor::ConversationMessage> = Vec::new();
        let text = user_text.trim();
        if !text.is_empty() {
            messages.push(astra_memory::cme::compactor::ConversationMessage {
                id: format!("u_{now_ms}"),
                thread_id: String::new(), // ingest 按 session 盖章
                role: astra_memory::cme::compactor::MessageRole::User,
                content: text.to_owned(),
                created_at: now_ms as i64,
            });
        }
        for (role, content, event_id) in self.collected {
            let id = if event_id.is_empty() {
                format!("{now_ms}_{}", messages.len())
            } else {
                event_id
            };
            messages.push(astra_memory::cme::compactor::ConversationMessage {
                id,
                thread_id: String::new(),
                role,
                content,
                created_at: now_ms as i64,
            });
        }
        messages
    }
}

/// Run 结束后的记忆提交（observe spawn 与真实 Run 测试共用）：登记采集
/// 消息进 CME 待处理队列，含优先级线索时立即压缩提炼；有待确认的
/// supersedes 意图且语义确认通道可用时，逐条裁决建链（docs/13 W2）。
/// 任何失败只记日志：记忆故障永不阻断 agent（D21）。返回是否走了立即
/// 处理路径（priority gate 触发），测试用于断言。
pub(crate) async fn commit_collected_memory(
    engine: &std::sync::Arc<
        astra_memory::cme::engine::MemoryEngine<astra_memory::cme::store_sqlite::CmeSqliteStore>,
    >,
    collector: MemoryRunCollector,
    session_id: &str,
    project: Option<&str>,
    user_text: &str,
    now_ms: u64,
    reviewer: Option<&dyn astra_memory::cme::consolidate::SupersedeReviewer>,
) -> bool {
    if let Err(error) = engine.set_thread_scope(session_id, project).await {
        eprintln!("[memory] cme set_thread_scope failed (session {session_id}): {error}");
    }
    let messages = collector.into_messages(user_text, now_ms);
    if messages.is_empty() {
        return false;
    }
    engine.ingest(session_id, messages);
    // 含重要线索（决策/纠正/偏好/已验证工具结果）立即处理，不让关键信息
    // 滞留在待处理队列；普通对话走阈值批处理。
    let force = engine.has_priority_pending();
    if let Err(error) = engine.process_pending(force).await {
        eprintln!("[memory] cme process_pending failed (session {session_id}): {error}");
    }
    // 语义确认（docs/13 W2）：规则发现的非 Decision supersedes 意图只有
    // 宿主 agent 裁决后才建链；拒绝/失败都保守并存，意图不重复入队。
    if engine.pending_supersede_count() > 0 {
        match reviewer {
            Some(reviewer) => match engine.resolve_pending_supersedes(reviewer).await {
                Ok(r) => eprintln!(
                    "[memory] supersede resolved (session {session_id}): chained={} rejected={} unresolved={} vanished={}",
                    r.chained, r.rejected, r.unresolved, r.vanished
                ),
                Err(error) => {
                    eprintln!(
                        "[memory] cme resolve_supersedes failed (session {session_id}): {error}"
                    )
                }
            },
            None => eprintln!(
                "[memory] {} supersede intent(s) pending (semantic reviewer disabled)",
                engine.pending_supersede_count()
            ),
        }
    }
    force
}

/// 把事件追加写入 app_data_dir/agent_events.log（JSONL）。
/// 失败只忽略——日志不能影响对话本身。
pub(crate) fn log_event(app: &AppHandle, env: &Envelope) {
    let Some(dir) = crate::app::accounts::data_dir(app).ok() else {
        return;
    };
    let path = dir.join("agent_events.log");
    let Ok(mut f) = std::fs::OpenOptions::new()
        .create(true)
        .append(true)
        .open(&path)
    else {
        return;
    };
    if let Ok(mut line) = serde_json::to_value(env) {
        // 时间戳便于离线量化每步耗时（事件流本身无时序信息）
        line["logged_at"] = Value::String(chrono::Local::now().to_rfc3339());
        if let Ok(s) = serde_json::to_string(&line) {
            use std::io::Write;
            let _ = writeln!(f, "{s}");
        }
    }
}

/// 读取最近 N 条事件日志（最新在后），供 /logs 页或前端排查
#[tauri::command]
pub fn agent_recent_events(app: AppHandle, limit: Option<u32>) -> Result<Vec<Value>, String> {
    let dir =
        crate::app::accounts::data_dir(&app).map_err(|e| format!("无法定位应用数据目录: {e}"))?;
    let path = dir.join("agent_events.log");
    let content = std::fs::read_to_string(&path).unwrap_or_default();
    let lines: Vec<&str> = content.lines().collect();
    let start = lines.len().saturating_sub(limit.unwrap_or(200) as usize);
    Ok(lines[start..]
        .iter()
        .filter_map(|l| serde_json::from_str(l).ok())
        .collect())
}
