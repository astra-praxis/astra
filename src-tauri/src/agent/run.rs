use std::sync::Arc;

use agent_core::model::AnthropicBackend;
use agent_core::model::ImageContent;
use agent_core::skill_tools::truncate_desc;
use agent_core::{
    Envelope, MessageContent, ModelBackend, PermissionBridge, RigBackend, ThreadConfig,
    DEFAULT_AUTO_COMPACT_TOKENS, DEFAULT_MAX_TURNS,
};
use base64::Engine as _;
use serde::Deserialize;
use serde_json::json;
use std::path::PathBuf;
use tauri::{AppHandle, Emitter, Manager, State};

use crate::providers;

use super::*;
// ===== 命令 =====

/// 用户附带图片的传输形状（与 astra-core `ImageContent` 对齐：base64 规范 PNG）
#[derive(Debug, Clone, Deserialize)]
pub struct AgentImageInput {
    pub data: String,
    pub width: u32,
    pub height: u32,
}

#[derive(Debug, Deserialize)]
pub struct AgentStartReq {
    /// API 来源：provider_id 与 model_port 二选一
    pub provider_id: Option<String>,
    pub model_port: Option<u16>,
    /// 模型名覆盖；不传则用 provider 的 active_model
    pub model: Option<String>,
    /// 项目根目录（None = 纯聊天，不注册文件工具）
    pub project_root: Option<String>,
    pub system_prompt: Option<String>,
    /// 最大续轮数（不传用 agent-core 默认）
    pub max_turns: Option<u32>,
    /// 上下文自动压缩阈值（估算 token 数；0 或 None = 关闭）
    pub auto_compact_tokens: Option<u64>,
    /// 用户通过 /技能名 指定的技能（内容注入本次用户消息）
    pub skill: Option<String>,
    /// 前端生成的会话 id（同会话复用 Thread 与历史）
    pub session_id: String,
    pub user_text: String,
    /// 用户附带的图片（规范 PNG 的 base64 与尺寸，经 agent_normalize_image 归一化）。
    /// 需要后端 vision 能力（image_recognition 开启）；与文本保持顺序（文本在前）。
    #[serde(default)]
    pub images: Vec<AgentImageInput>,
    /// Explicit consent for image-capable model/gateway requests. Never inferred from names.
    #[serde(default)]
    pub image_recognition: bool,
    /// Thinking mode for reasoning models; None leaves gateway defaults untouched.
    #[serde(default)]
    pub thinking_enabled: Option<bool>,
    /// Reasoning effort level (e.g. low/medium/high); requires a thinking-capable model.
    #[serde(default)]
    pub thinking_effort: Option<String>,
}

/// 单条消息最多附带图片数（与 core 的 MAX_IMAGES_PER_REQUEST 一致；
/// 工具图与本图共享该预算，超限时由 core 二次校验明确拒绝）
pub(crate) const MAX_ATTACHED_IMAGES: usize = agent_core::model::image::MAX_IMAGES_PER_REQUEST;

/// 组装用户输入内容块（多模态 M1）。
///
/// 顺序：文本块在前（技能包装只影响文本），图片块随后。纯图片消息允许
/// （文本为空时不产生空文本块）；core 的 `try_send_content` 做最终准入校验
/// （vision 能力、预算、PNG 有效性），此处只做宿主侧的快速失败。
pub(crate) fn build_user_content(
    user_text: &str,
    images: &[AgentImageInput],
    skill: Option<&str>,
    skills_dir: &Option<std::path::PathBuf>,
) -> Result<Vec<MessageContent>, String> {
    if images.len() > MAX_ATTACHED_IMAGES {
        return Err(format!("单条消息最多附带 {MAX_ATTACHED_IMAGES} 张图片"));
    }
    let text = match skill {
        None => user_text.to_string(),
        Some(skill_name) => {
            let dir = skills_dir
                .as_ref()
                .ok_or_else(|| format!("技能库不可用，无法加载技能 '{skill_name}'"))?;
            let content = crate::skills::user::read_skill_content(dir, skill_name, None)
                .map_err(|e| e.to_string())?;
            format!(
                "【用户通过 /{skill_name} 指定了技能，请先阅读以下技能说明并严格按其执行】\n\n{content}\n\n---\n\n用户任务：{user_text}"
            )
        }
    };
    let mut content = Vec::new();
    if !text.trim().is_empty() {
        content.push(MessageContent::text(text));
    }
    for img in images {
        let bytes = base64::engine::general_purpose::STANDARD
            .decode(img.data.as_bytes())
            .map_err(|_| "图片数据不是合法的 base64".to_string())?;
        let image = ImageContent::from_png(&bytes, img.width, img.height)
            .map_err(|e| format!("图片未通过规范校验：{e}"))?;
        content.push(MessageContent::Image { image });
    }
    if content.is_empty() {
        return Err("消息内容为空".into());
    }
    Ok(content)
}

/// 启动一次 Run（多轮对话中的一轮）。事件经 `agent-event` 流出。
/// 返回会话 id。
#[tauri::command]
pub async fn agent_start_run(
    app: AppHandle,
    state: State<'_, AgentState>,
    req: AgentStartReq,
) -> Result<String, String> {
    // P3 concurrent runs (SESSION_RUNTIME_SPEC §5): the host lock now only
    // covers admission (store open, config bind, Thread build). Each Thread
    // serializes its own Runs via its execution lock, so different sessions
    // stream in parallel and only two simultaneous admissions queue here.
    let mut threads = Arc::clone(&state.threads).lock_owned().await;
    let directory = app.path().app_data_dir().map_err(|e| e.to_string())?;
    threads
        .open(&directory, Arc::clone(&state.persistence_errors))
        .await?;

    // 1. 模型来源（URL 约定与 chat.rs 一致）。kind 决定后端协议：
    // openai = chat/completions（本地端口与旧数据默认），anthropic = Messages API。
    let (base_url, api_key, model_name, anthropic) = if let Some(port) = req.model_port {
        (
            format!("http://localhost:{port}/v1"),
            String::new(),
            req.model.unwrap_or_else(|| "local".into()),
            false,
        )
    } else if let Some(pid) = &req.provider_id {
        let p = providers::get_provider_inner(&app, pid)?;
        let model = req
            .model
            .or(p.active_model)
            .ok_or_else(|| format!("provider '{}' 没有设置活动模型", pid))?;
        let anthropic = p.kind == crate::providers::ProviderKind::Anthropic;
        (p.base_url, p.api_key, model, anthropic)
    } else {
        return Err("请先选择模型来源".into());
    };
    // bind_image_config 的目的地键：协议类型参与变更判定（同一 endpoint+model
    // 在两种协议间切换也必须重建 Thread）。openai 保持裸 URL，旧会话记录不因
    // 本次升级产生虚假的目的地变更。
    let dest_endpoint = if anthropic {
        format!("anthropic://{base_url}")
    } else {
        base_url.clone()
    };

    // 2. 项目根（提供了但无效 → 明确报错，不静默降级为纯聊天）
    let project_root = match &req.project_root {
        None => None,
        Some(p) => {
            let pb = PathBuf::from(p);
            if !pb.is_dir() {
                return Err(format!("项目目录不存在: {p}"));
            }
            Some(pb)
        }
    };

    let session_id = req.session_id;
    let user_text = req.user_text;

    // 记忆作用域键：取项目根目录名（稳定、跨机器可复现）。ingest 侧
    // set_thread_scope 与 recall 侧 ProjectBoundProvider 必须用同一个键，
    // 项目记忆才能对上。
    let bound_project = project_root.as_ref().map(|p| {
        p.file_name()
            .map(|n| n.to_string_lossy().into_owned())
            .unwrap_or_else(|| p.to_string_lossy().into_owned())
    });

    // 2.5 技能库：注册发现/读取工具 + system prompt 注入摘要（跨会话复用，建 Thread 时生效）
    let skills_dir = crate::skills::user::app_skills_dir(&app).ok();
    let skills_summary = skills_dir
        .as_deref()
        .map(crate::skills::user::summary_in)
        .unwrap_or_default();

    let dest_changed = threads
        .bind_image_config(
            &session_id,
            req.image_recognition,
            &dest_endpoint,
            &model_name,
        )
        .await?;

    // 2.6 多模态 M5（MULTIMODAL_SPEC §10）：读取生成配置并按需构建 provider。
    // enabled 且配置完整 → build_registry 注册 generate_image；
    // disabled / 缺配置 / 构建失败 → None（不注册、不报错、不阻塞会话）。
    // 配置变更只对新会话生效（已开 Thread 的工具集合冻结）。
    let generation = state
        .generation
        .sync(&directory, crate::agent_generation::load_config(&app));

    // 3. 取或建 Thread（绑定 root/backend/policy；历史保留在消息序列里）
    {
        if dest_changed && threads.threads.contains_key(&session_id) {
            // P3: never unload a Thread whose Run is still streaming — a
            // reload would build a second in-memory instance diverging from
            // the live run. Stop it first, then switch model source.
            if state
                .active
                .lock()
                .unwrap_or_else(|e| e.into_inner())
                .contains_key(&session_id)
            {
                return Err("会话正在运行，请先停止再切换模型来源".into());
            }
            // 模型来源切换：卸载缓存 Thread 后按新 backend 重建。历史消息
            // 已持久化在 SQLite，Thread::load 会恢复；存在持久化错误（未
            // 落盘尾部）时卸载会失败，此处明确报错而不是静默丢消息。
            threads
                .unload(&session_id, &state.persistence_errors)
                .map_err(|e| format!("切换模型来源失败：{e}"))?;
        }
        if !threads.threads.contains_key(&session_id) {
            let policy = host_tool_policy();
            let mut system_prompt = req
                .system_prompt
                .unwrap_or_else(crate::agent::prompts::agent_system_prompt);
            if !skills_summary.is_empty() {
                system_prompt.push_str(&crate::agent::prompts::agent_skills_section());
                for s in &skills_summary {
                    system_prompt.push_str(&format!(
                        "- {}: {}\n",
                        s.name,
                        truncate_desc(&s.description)
                    ));
                }
            }
            let mut config = ThreadConfig::new(session_id.clone(), system_prompt);
            config.project_root = project_root.clone();
            config.max_turns = req.max_turns.unwrap_or(DEFAULT_MAX_TURNS);
            config.auto_compact_tokens = req
                .auto_compact_tokens
                .or(Some(DEFAULT_AUTO_COMPACT_TOKENS))
                .filter(|&t| t > 0);
            config.policy = policy;
            // 记忆集成：注入 MemoryProvider，每个 Run 由 agent-core 自动
            // recall 最新用户输入进 system prompt（REMEMBERED CONTEXT）。
            // memory-off（init 失败）时保持 None，prompt 逐字节不变。
            // 项目绑定：core 的 recall 恒传 scope=None（冻结接口），这里用
            // ProjectBoundProvider 把本次 Run 的项目绑定进搜索范围
            // （全局偏好 + 本项目记忆），项目私有记忆不会跨项目串线。
            if let Some(memory) = state.memory.get() {
                config.memory = Some(Arc::new(astra_memory_provider::ProjectBoundProvider::new(
                    memory.provider(),
                    bound_project.clone(),
                )));
            }
            // Artifact records share the threads sqlite file; a failure to
            // open it keeps artifacts event-only (same degradation as before
            // this wiring) instead of blocking the run.
            if let Ok(artifacts) = threads.artifacts() {
                config.artifact_registry = Some(artifacts);
            }
            config.texts = std::sync::Arc::new(agent_core::TextPack::zh());
            config.thinking_enabled = req.thinking_enabled;
            config.thinking_effort = req.thinking_effort.clone();
            // Anthropic 原生 Messages API 后端（vision/tool_call 为协议内建
            // 能力，无需 image_encoding 开关）；其余走 OpenAI 兼容 SSE。
            let backend: Arc<dyn ModelBackend> = if anthropic {
                Arc::new(AnthropicBackend::new(base_url, api_key, model_name))
            } else {
                Arc::new(
                    RigBackend::new(base_url, api_key, model_name)
                        .with_image_encoding(image_encoding(req.image_recognition)),
                )
            };
            let bridge: Arc<dyn PermissionBridge> = Arc::new(HubBridge {
                hub: Arc::clone(&state.hub),
                app: app.clone(),
                session_id: session_id.clone(),
                project_root: project_root.clone(),
            });
            let question_bridge: Arc<dyn agent_core::UserQuestionBridge> =
                Arc::new(QuestionBridge {
                    hub: Arc::clone(&state.questions),
                    app: app.clone(),
                    session_id: session_id.clone(),
                });
            let mut registry = build_registry(
                &session_id,
                project_root.as_ref(),
                skills_dir.as_deref(),
                question_bridge,
                settings_search(&app),
                Arc::clone(&state.processes),
                Arc::clone(&state.tasks),
                generation,
                state.memory.get().map(|m| m.engine().clone()),
            );
            // MCP/ACP 装配（MCP_SPEC §14 / ACP_SPEC §11 宿主清单）：
            // ~/.astra/config.toml 的 [mcp]/[acp] 段加载外部能力并注册进本次
            // registry；required 失败阻断启动，其余失败降级（工具不出现）。
            // manager 句柄登记到 state.extensions：运行状态查询 + ACP 懒重连。
            let mut ext = SessionExtensions::default();
            registry = attach_extensions(
                registry,
                &mut config,
                &mut ext,
                &session_id,
                Arc::clone(&state.mcp_tasks),
                Arc::clone(&state.tasks),
            )
            .await?;
            state
                .extensions
                .lock()
                .await
                .insert(session_id.clone(), ext);
            threads
                .load_or_create(config, backend, registry, bridge)
                .await?;
        }
    }

    let (tx, mut rx) = tokio::sync::mpsc::channel::<Envelope>(1024);
    // 多模态 M1：组装用户输入内容块（文本在前、图片随后，保持原始顺序）。
    // /技能名 指定：技能内容只包装文本块（技能匹配从不接触图片数据）。
    let content = build_user_content(&user_text, &req.images, req.skill.as_deref(), &skills_dir)?;
    let thread = threads
        .threads
        .get(&session_id)
        .ok_or_else(|| "Agent thread is unavailable".to_string())?;
    // Apply explicit request settings after restore, before admitting the Run.
    // Await persistence and preserve the error instead of starting with stale settings.
    if req.thinking_enabled.is_some() || req.thinking_effort.is_some() {
        thread
            .set_thinking(req.thinking_enabled, req.thinking_effort)
            .await
            .map_err(|error| error.to_string())?;
    }
    let previous_error = state
        .persistence_errors
        .lock()
        .unwrap_or_else(|e| e.into_inner())
        .remove(&session_id);
    // 3.5 无感快照（隐形保险层）：每个发送前自动锚定 BeforeTurn checkpoint
    // （对话游标 + git 工作区快照），供「回到出错前 / 编辑重发」回退使用。
    // 用户不感知、无常驻 UI；捕获失败按 CHECKPOINT spec §5.3 降级为仅锚点，
    // 打点本身失败只记日志，绝不阻断 Run。
    if project_root.is_some() {
        match threads.checkpoints() {
            Ok(checkpoints) => {
                let provider = agent_core::astra_workspace::GitWorkspaceCheckpointProvider::new();
                if let Err(error) = thread
                    .create_checkpoint(
                        checkpoints,
                        Some(Arc::new(provider)),
                        agent_core::CheckpointKind::BeforeTurn,
                        None,
                        None,
                    )
                    .await
                {
                    eprintln!(
                        "[checkpoint] auto before_turn failed (session {session_id}): {error}"
                    );
                }
            }
            // 快照存储不可用只损失回退能力，不阻断发送
            Err(error) => {
                eprintln!("[checkpoint] store unavailable (session {session_id}): {error}")
            }
        }
    }
    let handle = match thread.try_send_content(content, tx) {
        Ok(handle) => handle,
        Err(error) => {
            if let Some(previous_error) = previous_error {
                state
                    .persistence_errors
                    .lock()
                    .unwrap_or_else(|e| e.into_inner())
                    .insert(session_id.clone(), previous_error);
            }
            return Err(error.to_string());
        }
    };
    // Register the run with a generation so the forwarder below only
    // releases its own slot (an interrupt-send may already have started a
    // newer run for the same session before this one finished draining).
    let generation = RUN_SEQ.fetch_add(1, std::sync::atomic::Ordering::Relaxed);
    state
        .active
        .lock()
        .unwrap_or_else(|e| e.into_inner())
        .insert(session_id.clone(), ActiveRun { generation, handle });

    // 4. 事件转发任务：Envelope → agent-event + JSONL 日志；Run 结束（tx 全部 drop）后自然退出
    let forward_app = app.clone();
    let active = Arc::clone(&state.active);
    let errors = Arc::clone(&state.persistence_errors);
    let settled_session = session_id.clone();
    // 记忆采集（宿主能力，不暴露给 agent 模型）：Run 结束后把用户输入、
    // 助手最终回答和已完成的工具结果 ingest 进 CME 待处理队列，达到阈值时
    // 压缩会话并提炼长期记忆。提炼是确定性规则，无模型调用；异步 spawn，
    // 不阻塞 Run 结束路径。任何失败只记日志：记忆故障永不阻断 agent（D21）。
    let observe_engine = state.memory.get().map(|m| m.engine().clone());
    let observe_reviewer = state.memory.get().and_then(|m| m.supersede_reviewer());
    let observe_text = user_text.clone();
    let observe_project = bound_project.clone();
    tokio::spawn(async move {
        // 边转发边采集（采集边界见 MemoryRunCollector 文档）。任何失败只记
        // 日志：记忆故障永不阻断 agent（D21）。
        let mut collector = MemoryRunCollector::new();
        while let Some(env) = rx.recv().await {
            collector.observe(&env);
            log_event(&forward_app, &env);
            let _ = forward_app.emit("agent-event", &env);
        }
        // 先结算后记忆：流关闭即释放准入并通知 UI（底部 loading 随渲染
        // 完成立即消失），记忆提交放在 settled 之后后台执行——开 agent
        // 提炼时 process_pending 要调 LLM（可达十几秒），不能让用户看着
        // loading 等记忆。与下一 Run 的记忆提交并发时由 CME 游标 CAS 兜底
        // （冲突只记日志，不丢消息）。
        let persistence_error = errors
            .lock()
            .unwrap_or_else(|e| e.into_inner())
            .get(&settled_session)
            .cloned();
        // Release this run's slot before notifying the UI that another send
        // is safe. Generation fence: a newer run for the same session owns
        // the slot when this forwarder lost the interrupt-send race.
        {
            let mut active = active.lock().unwrap_or_else(|e| e.into_inner());
            if active.get(&settled_session).map(|r| r.generation) == Some(generation) {
                active.remove(&settled_session);
            }
        }
        let _ = forward_app.emit(
            "agent-run-settled",
            json!({
                "session_id": settled_session,
                "persistence_error": persistence_error,
            }),
        );
        if let Some(engine) = observe_engine {
            // 项目绑定：本 Run 提炼的记忆落到对应项目作用域，避免跨项目串线。
            // best-effort：失败只记日志。键与 recall 侧 ProjectBoundProvider
            // 同源（bound_project），否则项目记忆对不上。
            let now_ms = std::time::SystemTime::now()
                .duration_since(std::time::UNIX_EPOCH)
                .map(|d| d.as_millis() as u64)
                .unwrap_or(0);
            commit_collected_memory(
                &engine,
                collector,
                &settled_session,
                observe_project.as_deref(),
                &observe_text,
                now_ms,
                observe_reviewer.as_deref(),
            )
            .await;
        }
    });

    Ok(session_id)
}
