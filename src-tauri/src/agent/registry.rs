use std::sync::Arc;

use agent_core::{ThreadConfig, ToolPolicy, ToolRegistry};
use std::path::{Path, PathBuf};

use super::*;
// ===== 工具注册（全部实现在 agent-core，本层只做组装） =====

/// 宿主工具权限白名单（严格模型：不在名单内一律 Deny）。每次新增内置
/// 工具时必须同步这里，否则注册成功但执行被拒（memory 工具踩过一次：
/// agent 看得到、调不了）。
pub(crate) fn host_tool_policy() -> ToolPolicy {
    // 在默认策略上追加 pandoc 工具：转换是只读产物生成（Allow），
    // 构建 docx 会覆盖/创建交付文件（Confirm）
    let mut policy = ToolPolicy::default();
    // fs_tools（Zed 移植）：find_path 只读；edit/copy/move/mkdir 产生变更走 Confirm
    policy.allowed_readonly.push("find_path".into());
    // pandoc 转换是只读产物生成（Allow）；ask_user / search_web 无副作用（Allow）
    policy.allowed_readonly.push("extract_document".into());
    policy.allowed_readonly.push("ask_user".into());
    policy.allowed_readonly.push("search_web".into());
    // MCP 配置自助管理：读无副作用 Allow；写入配置 Confirm（放权级别
    // 为 all 时随全放行放行，装扩展是用户主动要求的行为）
    policy.allowed_readonly.push("mcp_config_read".into());
    policy.confirm_tools.push("mcp_config_write".into());
    policy.confirm_tools.push("edit_file".into());
    // undo_edit 是文件变更工具（恢复 edit_file 前快照），与 edit_file 同级走 Confirm
    policy.confirm_tools.push("undo_edit".into());
    policy.confirm_tools.push("copy_path".into());
    policy.confirm_tools.push("move_path".into());
    policy.confirm_tools.push("create_directory".into());
    policy.confirm_tools.push("build_docx".into());
    policy.confirm_tools.push("shell_execute".into());
    // 后台任务：启动/停止有副作用走 Confirm；shell_status 只读（Allow）
    policy.confirm_tools.push("shell_background".into());
    policy.confirm_tools.push("shell_kill".into());
    policy.allowed_readonly.push("shell_status".into());
    // 统一任务查询（TASK_SPEC D3）：status/result 只读 Allow，
    // cancel 有副作用走 Confirm
    policy.allowed_readonly.push("task_status".into());
    policy.allowed_readonly.push("task_result".into());
    policy.confirm_tools.push("task_cancel".into());
    // 子 Agent 委托（SUBAGENT_SPEC）：发起委托有副作用（创建子 Run、
    // 消耗预算），走 Confirm；工具参数里没有权限入口，父身份只来自
    // ToolContext.run_id
    policy.confirm_tools.push("delegate_task".into());
    // 付费生成（MULTIMODAL_SPEC §10）：循环层直接放行，付费确认由
    // 工具内部 request_permission 完成（批准仅针对固定请求，换参数
    // 会重新弹窗）——避免循环层 Confirm + 工具内确认双重弹窗。
    policy.allowed_readonly.push("generate_image".into());
    // fetch 涉及外部网络访问，走确认弹窗（SSRF 门在工具内部始终生效）
    policy.confirm_tools.push("fetch".into());
    // Agent 记忆工具：search 只读 Allow；remember 写的是应用自己的
    // 记忆库（非文件系统/系统变更，可经 engine.forget 回退），且设计
    // 要求「显式记住立即生效」——走 Confirm 弹窗会破坏这个语义，
    // 故 Allow（memory-off 时工具根本不注册，不存在未声明即放行）。
    // forget 仅逻辑遗忘（valid_until=now，可逆：重新 remember 同内容
    // 即恢复），物理删除不暴露给 agent，故同为 Allow（docs/13 D2）。
    policy.allowed_readonly.push("memory_search".into());
    policy.allowed_readonly.push("memory_remember".into());
    policy.allowed_readonly.push("memory_forget".into());
    policy
}

pub(crate) fn build_registry(
    session_id: &str,
    project_root: Option<&PathBuf>,
    skills_dir: Option<&Path>,
    question_bridge: Arc<dyn agent_core::UserQuestionBridge>,
    search: agent_core::net_tools::SearchEngine,
    processes: Arc<agent_core::shell_background::ProcessManager>,
    tasks: Arc<agent_core::TaskManager>,
    generation: Option<(
        Arc<agent_core::task::GenerationTaskBackend>,
        Arc<dyn agent_core::astra_generation::GenerationProvider>,
    )>,
    memory: Option<
        std::sync::Arc<
            astra_memory::cme::engine::MemoryEngine<
                astra_memory::cme::store_sqlite::CmeSqliteStore,
            >,
        >,
    >,
    // Shared file-state cache + run-level change tracker (FileChanges card).
    // None = 降级：工具内部各建缓存，无变更记录。
    fs_tracking: Option<(
        Arc<agent_core::FileStateCache>,
        Arc<agent_core::ChangeTracker>,
    )>,
) -> ToolRegistry {
    // 网络工具 + ask_user 不依赖项目根，纯聊天也可用；任务查询工具同全局
    let reg = agent_core::net_tools::register_with_search(ToolRegistry::new(), search)
        .register(agent_core::AskUserTool::new(question_bridge))
        // MCP 配置自助管理：用户在对话里说「装个 xx MCP」时，agent 用它写
        // 账户感知的 config.toml（保存后各会话下次对话自动重建生效）
        .register(crate::extensions::mcp_config_tool::McpConfigRead)
        .register(crate::extensions::mcp_config_tool::McpConfigWrite);
    let reg = agent_core::task_tools::register(reg, tasks.clone());
    // 多模态 M5：enabled 且配置完整时注册 generate_image（付费生成，
    // MULTIMODAL_SPEC §10）；disabled / 缺配置时不注册、不报错。
    let reg = match generation {
        Some((backend, provider)) => agent_core::generation_tools::register(
            reg,
            tasks.clone(),
            backend,
            provider,
            session_id,
        ),
        None => reg,
    };
    let reg = match skills_dir {
        Some(dir) => agent_core::skill_tools::register(reg, dir),
        None => reg,
    };
    // Agent 记忆工具（memory_remember / memory_search）：memory-off 时不注册，
    // agent 不会看到无法调用的工具。
    let reg = match memory {
        Some(engine) => crate::memory::tool::register(reg, engine),
        None => reg,
    };
    match project_root {
        Some(root) => {
            // fs 工具共享同一 FileStateCache（read 基线对 edit/write 生效，
            // 官方 register_with_cache 的设计意图）并挂接变更追踪；降级时
            // 保持旧路径：各自 register（内部各建缓存）。
            let reg = if let Some((cache, _tracker)) = &fs_tracking {
                let texts: agent_core::SharedTextPack = Arc::new(agent_core::TextPack::default());
                let reg = agent_core::fs_basic::register_with_cache(
                    reg,
                    root,
                    cache.clone(),
                    texts.clone(),
                );
                agent_core::fs_tools::register_with_cache(reg, root, cache.clone(), texts)
            } else {
                let reg = agent_core::fs_basic::register(reg, root);
                agent_core::fs_tools::register(reg, root)
            };
            let reg = agent_core::doc_tools::register(reg, root);
            let reg = agent_core::shell_tool::register(reg, root);
            // Background task management (dev servers / watchers): manager is
            // host-owned and shared across sessions, so tasks outlive a chat.
            // Spawned processes are adopted into the unified task runtime
            // (TASK_SPEC §六: same TaskManager as task_status/task_result).
            let reg = agent_core::shell_background::register(reg, root, processes, Some(tasks));
            // fs_tools（Zed 移植）：搜索/编辑/移动类（已在上方按共享缓存注册）
            reg
        }
        None => {
            // Pure chat (no project): still grant file/shell tools, rooted at
            // the user's home directory, so the assistant can read/write/run
            // things outside any project. Permission dispositions are identical
            // to project mode (reads Allow, writes/shell Confirm); paths are
            // validated against home instead of a project root.
            let home = dirs::home_dir().unwrap_or_else(|| PathBuf::from("."));
            let reg = if let Some((cache, _tracker)) = &fs_tracking {
                let texts: agent_core::SharedTextPack = Arc::new(agent_core::TextPack::default());
                let reg = agent_core::fs_basic::register_with_cache(
                    reg,
                    &home,
                    cache.clone(),
                    texts.clone(),
                );
                agent_core::fs_tools::register_with_cache(reg, &home, cache.clone(), texts)
            } else {
                let reg = agent_core::fs_basic::register(reg, &home);
                agent_core::fs_tools::register(reg, &home)
            };
            let reg = agent_core::doc_tools::register(reg, &home);
            let reg = agent_core::shell_tool::register(reg, &home);
            let reg = agent_core::shell_background::register(reg, &home, processes, Some(tasks));
            reg
        }
    }
}

/// MCP/ACP 扩展装配（MCP_SPEC §14 / ACP_SPEC §11 宿主清单）：读账户感知的
/// config.toml（账户模式 accounts/<id>/astra-home/，legacy 回退 ~/.astra）
/// 的 [mcp]/[acp] 段，加载外部能力并注册进本次 registry。规则：
/// - required 失败阻断本次启动；其余失败降级（工具不出现，日志报错）
/// - 配置文件缺失或无效 = 无扩展，不阻断普通对话（配置错误经日志暴露）
/// - 配置变更由 agent_start_run 的指纹检测自动重建（当前对话下一句生效），
///   与「切项目/模型后重建 Thread」同一机制
/// - 每个 Thread 一份独立 manager，避免共享 manager 重载时拆掉其他会话
///   仍在用的连接（v1 简化，代价是同 server 随会话数多开进程）
pub(crate) async fn attach_extensions(
    mut registry: ToolRegistry,
    config: &mut ThreadConfig,
    handles: &mut SessionExtensions,
    session_id: &str,
    mcp_tasks: Arc<astra_mcp::McpTaskBackend>,
    tasks: Arc<agent_core::TaskManager>,
) -> Result<ToolRegistry, String> {
    let cfg = match agent_core::astra_config::load() {
        Ok(c) => c,
        Err(e) => {
            app_log!(error, "extensions", "配置无效，本次运行不加载 MCP/ACP: {e}");
            return Ok(registry);
        }
    };

    // ===== MCP =====
    if cfg.mcp.servers.values().any(|s| s.enabled) {
        // D6 修订版：接线统一任务收编——server 物化的任务（resultType:
        // "task"）经 adapter adopt 进全局 TaskManager，模型拿统一 task_id
        // 用 task_status/task_result 轮询；后端按 session 路由回本 manager。
        let manager = astra_mcp::McpManager::new(cfg.mcp.to_manager_configs())
            .map_err(|e| format!("MCP 配置无效: {e}"))?
            .with_task_tracking(session_id.to_string(), tasks);
        let manager = std::sync::Arc::new(manager);
        mcp_tasks.register_session(session_id, &manager).await;
        let (reg, report) = manager.load_and_register(registry).await;
        registry = reg;
        for failed in &report.failed {
            app_log!(
                error,
                "extensions",
                "MCP server '{}' 加载失败: {}",
                failed.id,
                failed.error
            );
            if failed.required {
                return Err(format!(
                    "MCP server '{}' 为 required，加载失败: {}",
                    failed.id, failed.error
                ));
            }
        }
        for dropped in &report.dropped_tools {
            app_log!(
                warn,
                "extensions",
                "MCP 工具被丢弃: {}/{} ({})",
                dropped.server,
                dropped.tool,
                dropped.reason
            );
        }
        // manager 由 rule_provider + state.extensions 保活；权限规则按发现
        // 结果逐工具生成，未列出的默认 Confirm（MCP_SPEC §9.2），经
        // ThreadConfig 进入权限管线。
        handles.mcp.push(std::sync::Arc::clone(&manager));
        let permissions: std::collections::HashMap<
            String,
            agent_core::astra_config::McpPermissions,
        > = cfg
            .mcp
            .servers
            .iter()
            .filter(|(_, e)| e.enabled)
            .map(|(id, e)| (id.clone(), e.permissions.clone()))
            .collect();
        config.rule_provider = Some(std::sync::Arc::new(
            agent_core::astra_config::McpRuleProvider::new(manager, permissions),
        ));
        app_log!(
            info,
            "extensions",
            "MCP: {} server(s) 已注册 [{}]（{} 个工具被丢弃）",
            report.loaded.len(),
            report.loaded.join(", "),
            report.dropped_tools.len()
        );
    }

    // ===== ACP =====
    if cfg.acp.agents.values().any(|a| a.enabled) {
        let manager = astra_acp::AcpManager::new(cfg.acp.to_specs())
            .map_err(|e| format!("ACP 配置无效: {e}"))?;
        let (reg, report) = manager.load_and_register(registry).await;
        registry = reg;
        for failed in &report.failed {
            app_log!(
                error,
                "extensions",
                "ACP agent '{}' 加载失败: {}",
                failed.id,
                failed.error
            );
            if failed.required {
                return Err(format!(
                    "ACP agent '{}' 为 required，加载失败: {}",
                    failed.id, failed.error
                ));
            }
        }
        app_log!(
            info,
            "extensions",
            "ACP: {} agent(s) 已注册 [{}]",
            report.loaded.len(),
            report.loaded.join(", ")
        );
        // D7.1：调用外部 Agent 本身复用既有 decide 管线——装配期对每个注册
        // 成功的 agent 生成 Confirm 规则（risk=High 不降级，未配置即 Confirm）。
        // [acp.agents.*].permission 只管第二层 request_permission 应答，
        // 不驱动本层。名字必须与 adapter 注册名一致（normalize_tool_name）。
        for id in &report.loaded {
            config
                .policy
                .confirm_tools
                .push(astra_acp::normalize_tool_name(id));
        }
        // manager 保活在 state.extensions：adapter 持有连接不影响调用，
        // 保活额外让崩溃降级 / 懒重连 / 运行状态查询可用。
        handles.acp.push(std::sync::Arc::new(manager));
    }

    Ok(registry)
}
