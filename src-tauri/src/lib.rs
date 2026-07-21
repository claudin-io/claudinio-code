// O backend foi extraído para o crate `claudinio-core` (Fase 1). Reexportamos
// os módulos aqui para que os caminhos `crate::agent::...`, `crate::state::...`
// etc. usados no restante do app (commands/) continuem resolvendo sem reescrita.
pub use claudinio_core::{agent, askpass, code_intel, http, lsp, net_activity, state};

mod commands;
mod tauri_sinks;

use state::AppState;
use tauri::Emitter;

#[cfg_attr(mobile, tauri::mobile_entry_point)]
pub fn run() {
    tauri::Builder::default()
        .plugin(tauri_plugin_opener::init())
        .plugin(tauri_plugin_dialog::init())
        .plugin(tauri_plugin_updater::Builder::new().build())
        .plugin(tauri_plugin_process::init())
        .manage(AppState::new())
        .manage(commands::power::KeepAwakeState::default())
        .setup(|app| {
            // Liga os sinks/observers do core (Tauri-free) aos eventos do app.
            let net_app = tauri::AppHandle::clone(app.handle());
            net_activity::set_observer(Box::new(move |views| {
                let _ = net_app.emit(net_activity::EVENT_NAME, &views);
            }));
            let ask_app = tauri::AppHandle::clone(app.handle());
            askpass::set_observer(Box::new(move |id, prompt| {
                let _ = ask_app.emit(
                    askpass::EVENT_NAME,
                    serde_json::json!({ "id": id, "prompt": prompt }),
                );
            }));
            // O core spawna o listener na runtime do Tauri.
            tauri::async_runtime::spawn(async {
                if let Err(e) = askpass::serve().await {
                    eprintln!("[askpass] bridge unavailable: {e}");
                }
            });
            commands::system_stats::start_poller(app.handle().clone());
            Ok(())
        })
        .invoke_handler(tauri::generate_handler![
            commands::fs::list_dir,
            commands::fs::read_file,
            commands::fs::write_file,
            commands::fs::write_file_bytes,
            commands::fs::read_attachment,
            commands::fs::walk_dir,
            commands::locale::get_os_locale,
            commands::agent::send_message,
            commands::agent::new_session,
            commands::agent::list_sessions,
            commands::agent::load_session,
            commands::agent::approve_tool,
            commands::agent::reject_tool,
            commands::agent::submit_answers,
            commands::agent::set_config,
            commands::agent::get_config,
            commands::agent::set_workspace_config,
            commands::agent::list_models,
            commands::auth::login_with_claudinio,
            commands::auth::logout_claudinio,
            commands::auth::validate_api_key,
            commands::providers::openrouter_login,
            commands::providers::openrouter_login_cancel,
            commands::providers::fetch_provider_catalog,
            commands::providers::connect_provider,
            commands::providers::disconnect_provider,
            commands::providers::list_provider_models,
            commands::providers::list_all_models,
            commands::clipboard::write_clipboard_blob,
            commands::git::git_status,
            commands::git::git_file_diff,
            commands::git::git_branch,
            commands::git::check_git_available,
            commands::code_intel::open_workspace,
            commands::code_intel::close_workspace,
            commands::code_intel::search_symbols,
            commands::code_intel::symbol_lookup,
            commands::code_intel::file_outline,
            commands::lsp::lsp_definition,
            commands::lsp::lsp_references,
            commands::lsp::lsp_hover,
            commands::agent::queue_steering,
            commands::agent::interrupt_session,
            commands::agent::compact_session,
            commands::agent::commit_and_push,
            commands::agent::set_session_mode,
            commands::agent::continue_with_builder,
            commands::agent::get_session_mode,
            commands::agent::check_plan_exists,
            commands::agent::list_plans,
            commands::agent::set_workspace_config,
            commands::context::get_context_warning,
            commands::enhance::enhance_prompt,
            commands::skills::list_skills,
            commands::skills::get_skill_catalog,
            commands::skills::get_skill_content,
            commands::skills::rescan_skills,
            commands::skills::find_remote_skills,
            commands::skills::preview_remote_skill,
            commands::skills::install_remote_skill,
            commands::tasks::get_tasks,
            commands::tasks::set_tasks,
            commands::tasks::dismiss_golden_tasks,
            commands::network_log::get_network_log,
            commands::mcp::mcp_list_servers,
            commands::mcp::mcp_test_server,
            commands::mcp::mcp_reconnect,
            commands::shell::open_in_terminal,
            commands::power::set_keep_awake,
            commands::ide::detect_ides,
            commands::ide::open_in_ide,
            commands::askpass::answer_askpass,
        ])
        .run(tauri::generate_context!())
        .expect("error while running tauri application");
}
