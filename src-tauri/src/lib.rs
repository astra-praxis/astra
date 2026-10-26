// app_log! 宏需要 #[macro_use] 且必须声明在使用它的模块之前
#[macro_use]
mod logging;
mod agent;
mod agent_generation;
mod app;
mod extensions;
mod memory;
mod proc_util;
mod providers;
mod skills;

use std::collections::HashMap;
use std::sync::Mutex;
use tauri::Manager;

// 拆分后的模块在 crate 根的转发绑定（agent 等兄弟模块经 crate:: 访问）
use app::llama_server::RunningServer;
use app::settings::{get_settings, get_settings_path, workspace_roots};

// ===== 应用入口 =====

/// 记忆宿主初始化（CME）：加密 SQLite + 主密钥，失败降级 memory-off。
/// setup 与 account_unlock（解锁后补做）共用；OnceLock 幂等，重复调用安全。
fn init_memory_host(app: &tauri::AppHandle) {
    // 访问密码锁定：跳过初始化（密钥不可用），解锁后再补做
    if let Ok(dir) = app::accounts::data_dir(app) {
        if app::secret::is_locked(&dir) {
            eprintln!("[memory] account locked, init deferred until unlock");
            app_log!(info, "memory", "account locked, init deferred until unlock");
            return;
        }
    }
    let settings = get_settings(app.clone());
    let provider_id = settings.memory_summary_provider_id.clone();
    let agent_summary_client = if settings.memory_agent_summary {
        eprintln!(
            "[memory] agent summary enabled (provider: {})",
            provider_id.as_deref().unwrap_or("auto")
        );
        app_log!(
            info,
            "memory",
            "agent summary enabled (provider: {})",
            provider_id.as_deref().unwrap_or("auto")
        );
        Some(Box::new(crate::memory::host::HostAgentSummaryClient::new(
            app.clone(),
            provider_id.clone(),
        ))
            as Box<dyn astra_memory::cme::compactor::AgentSummaryClient>)
    } else {
        None
    };
    // 同一开关（memory_agent_summary）同时控制提炼路径（W1）与
    // supersedes 语义确认（W2）：都复用宿主 agent LLM 通道，失败
    // 各自降级（提炼回规则、确认不建链）。
    let agent_extraction_client = if settings.memory_agent_summary {
        Some(
            Box::new(crate::memory::host::HostAgentExtractionClient::new(
                app.clone(),
                provider_id.clone(),
            )) as Box<dyn astra_memory::cme::extract::AgentExtractionClient>,
        )
    } else {
        None
    };
    let supersede_reviewer = if settings.memory_agent_summary {
        Some(Box::new(crate::memory::host::HostSupersedeReviewer::new(
            app.clone(),
            provider_id,
        ))
            as Box<dyn astra_memory::cme::consolidate::SupersedeReviewer>)
    } else {
        None
    };
    match app::accounts::data_dir(app) {
        Ok(dir) => {
            match crate::memory::host::MemoryHost::init(
                &dir,
                crate::memory::host::MemoryInitOptions {
                    agent_summary_client,
                    agent_extraction_client,
                    supersede_reviewer,
                },
            ) {
                Ok(host) => {
                    let _ = app.state::<agent::AgentState>().memory.set(host);
                    app_log!(info, "memory", "host init ok");
                }
                Err(error) => {
                    eprintln!("[memory] init failed, memory disabled: {error}");
                    app_log!(error, "memory", "init failed, memory disabled: {error}");
                }
            }
        }
        Err(error) => {
            eprintln!("[memory] app_data_dir unavailable, memory disabled: {error}");
            app_log!(error, "memory", "app_data_dir unavailable: {error}");
        }
    }
}

#[cfg_attr(mobile, tauri::mobile_entry_point)]
pub fn run() {
    // Panic 落盘：Windows GUI 子系统无控制台，panic 信息默认无人可见，
    // 启动即崩时连应用日志都来不及建。写临时目录固定文件（%TEMP%\astra-panic.log），
    // 任何机器崩溃后用户都能直接反馈该文件。stderr 同步保留（CI 重定向可捕获）。
    std::panic::set_hook(Box::new(|info| {
        let msg = format!("panicked: {info}\n");
        eprintln!("{msg}");
        crate::logging::log("error", "panic", info.to_string().trim());
        let log = std::env::temp_dir().join("astra-panic.log");
        if let Ok(mut f) = std::fs::OpenOptions::new()
            .create(true)
            .append(true)
            .open(&log)
        {
            use std::io::Write;
            let _ = f.write_all(msg.as_bytes());
        }
    }));

    // Logging: surface tracing output (agent-core recall failures/skips were
    // invisible before). ASTRA_LOG overrides; default info level.
    let _ = tracing_subscriber::fmt()
        .with_env_filter(
            tracing_subscriber::EnvFilter::try_from_default_env()
                .unwrap_or_else(|_| tracing_subscriber::EnvFilter::new("info")),
        )
        .with_target(false)
        .try_init();

    tauri::Builder::default()
        .manage(Mutex::new(Vec::<RunningServer>::new()))
        .manage(memory::indexer::IndexCancelFlag(
            std::sync::atomic::AtomicBool::new(false),
        ))
        .manage(app::downloader::DownloadManager::new())
        .manage(agent::AgentState::default())
        .manage(Mutex::new(HashMap::<
            String,
            app::dev_mode::RunningDevProcess,
        >::new()))
        .plugin(tauri_plugin_opener::init())
        .plugin(tauri_plugin_dialog::init())
        .plugin(tauri_plugin_fs::init())
        .plugin(
            tauri_plugin_global_shortcut::Builder::new()
                .with_handler(|app, _shortcut, event| {
                    if event.state == tauri_plugin_global_shortcut::ShortcutState::Pressed {
                        if let Some(window) = app.get_webview_window("main") {
                            let _ = window.show();
                            let _ = window.set_focus();
                        }
                    }
                })
                .build(),
        )
        .setup(|app| {
            use tauri::{
                menu::{Menu, MenuItem},
                tray::TrayIconBuilder,
                Manager,
            };
            use tauri_plugin_global_shortcut::GlobalShortcutExt;

            // ===== 本地账户绑定与单实例锁（多账户 P0，冷切换架构） =====
            // 账户身份由启动参数 --account=<id> 决定；无参数 → legacy 模式
            // （落盘仍走原 app_data_dir，旧版本行为不变）。账户数据目录
            // 与实例锁的说明见 docs/MULTI_ACCOUNT_INVENTORY.md。
            let app_root = match app.path().app_data_dir() {
                Ok(dir) => dir,
                Err(e) => {
                    eprintln!("[account] app_data_dir unavailable: {e}");
                    std::process::exit(1);
                }
            };
            match app::accounts::init_from_args(&app_root) {
                Ok(runtime) => {
                    // 账户模式：ASTRA_HOME 指向账户私有 astra-home，
                    // astra-core 的 astra_config::load()（MCP/ACP 装配）
                    // 经 resolve_astra_home() 自动跟随，无需改动 runtime。
                    if let Some(rt) = &runtime {
                        std::env::set_var("ASTRA_HOME", &rt.astra_home);
                    }
                    app::accounts::bind(runtime);
                }
                Err(e) => {
                    eprintln!("[account] {e}");
                    std::process::exit(1);
                }
            }
            match app::accounts::InstanceLock::acquire(&app_root) {
                Ok(lock) => {
                    app.manage(app::accounts::InstanceLockState(std::sync::Mutex::new(
                        lock,
                    )));
                }
                Err(e) => {
                    eprintln!("[account] {e}");
                    std::process::exit(1);
                }
            }

            // 功能点落盘日志：Windows GUI 无控制台，关键事件写
            // %APPDATA%\com.astra.app\astra.log，用户可直接反馈该文件。
            // 账户模式下写账户私有数据目录。
            if let Ok(dir) = app::accounts::data_dir(&*app) {
                logging::init(&dir);
                app_log!(
                    info,
                    "app",
                    "astra start v{} ({} {})",
                    env!("CARGO_PKG_VERSION"),
                    std::env::consts::OS,
                    std::env::consts::ARCH
                );
            }

            // 记忆宿主初始化（astra-memory CME）：加密 SQLite + 主密钥文件。
            // 失败降级为 memory-off：agent 照常运行（无 recall 注入）。
            // 摘要路径开关（memory_agent_summary）重启后生效；开启时用
            // settings 里指定的 provider（或自动选第一个可用的）做总结。
            // 访问密码锁定时跳过，解锁命令会再调 init_memory_host 补做。
            init_memory_host(app.handle());

            // 内置技能（astra-memory 记忆指南 + astra-find-skills 找装技能）：
            // 技能库中已有同名技能时不覆盖，用户修改优先。best-effort。
            if let Err(error) = skills::user::ensure_builtin_skills(app.handle()) {
                eprintln!("[skills] builtin memory skill install failed: {error}");
                app_log!(error, "skills", "builtin skills install failed: {error}");
            }

            let show_item = MenuItem::with_id(app, "show", "显示窗口", true, Some("Alt+Space"))?;
            let separator = tauri::menu::PredefinedMenuItem::separator(app)?;
            let quit_item = MenuItem::with_id(app, "quit", "退出", true, Some("CmdOrCtrl+Q"))?;
            let menu = Menu::with_items(app, &[&show_item, &separator, &quit_item])?;

            let _tray = TrayIconBuilder::new()
                .icon(app.default_window_icon().unwrap().clone())
                .tooltip("Astra")
                .menu(&menu)
                .on_menu_event(|app, event| match event.id.as_ref() {
                    "show" => {
                        if let Some(window) = app.get_webview_window("main") {
                            let _ = window.show();
                            let _ = window.set_focus();
                        }
                    }
                    "quit" => {
                        // 清理所有运行中的开发服务器（终止整个进程树 + 端口兜底，确保端口释放）
                        let state = app.state::<app::dev_mode::DevServerState>();
                        let mut servers = state.lock().unwrap();
                        for (_, mut server) in servers.drain() {
                            let port = server.port;
                            app::dev_mode::terminate_process_tree(&mut server.child);
                            if let Some(p) = port {
                                app::dev_mode::kill_processes_on_port(p);
                            }
                        }
                        app.exit(0);
                    }
                    _ => {}
                })
                .on_tray_icon_event(|tray, event| {
                    use tauri::tray::{MouseButton, MouseButtonState, TrayIconEvent};
                    if let TrayIconEvent::Click {
                        button: MouseButton::Left,
                        button_state: MouseButtonState::Up,
                        ..
                    } = event
                    {
                        let app = tray.app_handle();
                        if let Some(window) = app.get_webview_window("main") {
                            let _ = window.show();
                            let _ = window.set_focus();
                        }
                    }
                })
                .build(app)?;

            // 注册全局快捷键 Alt+Space 唤醒窗口。冲突时只降级不阻断启动——
            // Alt+Space 被 PowerToys Run / uTools / Listary 等启动器默认占用，
            // setup 返回 Err 会导致应用直接退出（Windows 首发秒退的根因）。
            if let Err(error) = app.global_shortcut().register("Alt+Space") {
                eprintln!("[shortcut] Alt+Space register failed (likely in use): {error}");
                app_log!(warn, "shortcut", "Alt+Space register failed: {error}");
            }

            Ok(())
        })
        .on_window_event(|window, event| {
            if let tauri::WindowEvent::CloseRequested { api, .. } = event {
                // 点击关闭按钮时不退出，而是隐藏到托盘
                window.hide().ok();
                api.prevent_close();
            }
        })
        .invoke_handler(tauri::generate_handler![
            greet,
            logging::frontend_log,
            app::workspace::scan_workspace,
            app::workspace::list_project_files,
            app::workspace::create_project,
            app::settings::get_settings,
            app::settings::save_settings,
            app::settings::open_in_editor,
            app::file_preview::preview_read_file,
            app::file_preview::path_exists,
            app::command_runner::run_command,
            app::settings::get_preset_editors,
            skills::registry::list_skills,
            skills::registry::delete_skill,
            skills::registry::read_skill_doc,
            skills::registry::update_skill,
            skills::registry::update_all_skills,
            app::workspace::get_project_detail,
            app::workspace::find_casp_portal_dir,
            app::settings::add_workspace,
            app::settings::remove_workspace,
            app::settings::set_active_workspace,
            app::settings::set_favorite_dev_mode,
            app::window::minimize_to_tray,
            app::window::drag_window,
            app::window::set_window_background,
            app::node_modules::scan_node_modules,
            app::node_modules::clean_node_modules,
            app::llama_server::start_llama_server,
            app::llama_server::stop_llama_server,
            app::llama_server::list_running_servers,
            app::llama_server::check_server_status,
            app::llama_server::check_model_paths,
            app::llama_server::scan_model_directory,
            app::llama_server::list_model_configs,
            app::llama_server::save_model_config,
            app::llama_server::delete_model_config,
            // Providers
            providers::list_providers,
            providers::save_provider,
            providers::delete_provider,
            providers::fetch_provider_models,
            providers::set_active_model,
            providers::get_provider,
            // Agent
            agent::run::agent_start_run,
            agent::images::agent_normalize_image,
            agent::commands::agent_cancel,
            agent::commands::agent_permission_decide,
            agent::commands::agent_question_answer,
            agent::commands::agent_set_auto_mode,
            agent::commands::agent_reset_session,
            agent::status::agent_status,
            agent::events::agent_recent_events,
            agent::artifacts::agent_spill_paste,
            // Agent Generation（多模态 M5：生成供应商配置）
            agent_generation::generation_get_config,
            agent_generation::generation_set_config,
            agent_generation::media_read_asset,
            agent_generation::media_open_asset,
            // Agent Checkpoints
            agent::checkpoints::agent_checkpoint_list,
            agent::checkpoints::agent_checkpoint_create,
            agent::checkpoints::agent_checkpoint_restore,
            agent::checkpoints::agent_rewind_to_user_message,
            // Agent Artifacts（ARTIFACT_SPEC：产物元数据登记）
            agent::artifacts::agent_artifact_list,
            // Memory（astra-memory 集成：recall 注入 + 显式管理）
            memory::host::memory_status,
            memory::host::memory_search,
            memory::host::memory_remember,
            memory::host::memory_forget,
            // Agent Sessions
            agent::sessions::list_agent_sessions,
            agent::sessions::save_agent_session,
            agent::sessions::delete_agent_session,
            agent::sessions::agent_archive_session,
            // Agent Preview（后台任务预览窗口）
            agent::preview::agent_preview_tasks,
            agent::preview::agent_open_preview,
            // User Skills
            skills::user::list_user_skills,
            skills::user::create_user_skill,
            skills::user::import_user_skill,
            skills::user::sync_global_skills,
            skills::user::list_user_skill_files,
            skills::user::read_user_skill_file,
            skills::user::write_user_skill_file,
            skills::user::delete_user_skill_file,
            skills::user::delete_user_skill,
            // 本地账户（多账户 P0）
            app::accounts::account_status,
            app::accounts::account_list,
            app::accounts::account_create,
            app::accounts::account_launch,
            app::accounts::account_password_set,
            app::accounts::account_unlock,
            app::accounts::account_password_remove,
            // Extensions 市场（状态展示；安装走 Agent 对话）
            extensions::market::market_status,
            // Extensions 管理（MCP / ACP 启停、编辑、删除）
            extensions::extensions_list,
            extensions::extensions_set_enabled,
            extensions::extensions_remove,
            extensions::extensions_update,
            agent::commands::agent_unload_all_sessions,
            agent::status::extensions_runtime_status,
            // Dev Mode
            app::dev_mode::commands::scan_dev_dirs,
            app::dev_mode::commands::run_card_command,
            app::dev_mode::commands::start_dev_server,
            app::dev_mode::commands::stop_dev_server,
            app::dev_mode::commands::stop_all_dev_servers,
            app::dev_mode::commands::list_dev_servers,
            app::dev_mode::history::list_dev_history,
            app::dev_mode::history::clear_dev_history,
            app::dev_mode::config::read_dev_config,
            app::dev_mode::config::save_dev_cookie,
            app::dev_mode::config::open_login_url,
            // Downloader
            app::downloader::list_downloads,
            app::downloader::add_download,
            app::downloader::start_download,
            app::downloader::pause_download,
            app::downloader::delete_download,
            app::downloader::retry_download,
            app::downloader::open_download_folder,
            app::downloader::open_download_file,
            app::downloader::clear_download_history,
            // Embedding
            memory::embedding::embed_text,
            memory::embedding::test_embedding_source,
            // Vector Store
            memory::vector_store::store_chunks,
            memory::vector_store::delete_project_index,
            memory::vector_store::search_similar,
            memory::vector_store::get_index_stats,
            // Indexer
            memory::indexer::index_project,
            memory::indexer::cancel_index,
        ])
        .run(tauri::generate_context!())
        .expect("error while running tauri application");
}
// ===== 保留默认 greet =====

#[tauri::command]
fn greet(name: &str) -> String {
    format!("Hello, {}! You've been greeted from Rust!", name)
}
