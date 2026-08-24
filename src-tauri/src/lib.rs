mod agent;
pub(crate) mod askpass;
pub mod browser;
pub mod code_intel;
mod commands;
pub mod download;
pub(crate) mod http;
pub(crate) mod imageutil;
pub mod llama;
mod lsp;
pub(crate) mod net_activity;
pub(crate) mod procutil;
pub mod quality;
pub(crate) mod randutil;
mod state;
pub(crate) mod workspace_path;

use state::AppState;

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
            net_activity::set_app_handle(tauri::AppHandle::clone(app.handle()));
            askpass::set_app_handle(tauri::AppHandle::clone(app.handle()));
            askpass::start();
            commands::system_stats::start_poller(app.handle().clone());
            // An older pin's runtime is dead weight the user cannot see.
            llama::provision::gc_stale_installs();
            Ok(())
        })
        .invoke_handler(tauri::generate_handler![
            commands::fs::list_dir,
            commands::fs::read_file,
            commands::fs::write_file,
            commands::fs::write_file_bytes,
            commands::fs::export_file,
            commands::fs::export_file_bytes,
            commands::fs::read_attachment,
            commands::fs::walk_dir,
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
            commands::context::get_context_warning,
            commands::enhance::enhance_prompt,
            commands::skills::list_skills,
            commands::skills::get_skill_catalog,
            commands::skills::get_skill_content,
            commands::skills::rescan_skills,
            commands::skills::find_remote_skills,
            commands::skills::preview_remote_skill,
            commands::skills::install_remote_skill,
            commands::quality::get_quality_config,
            commands::quality::set_quality_config,
            commands::tasks::get_tasks,
            commands::tasks::set_tasks,
            commands::tasks::dismiss_golden_tasks,
            commands::network_log::get_network_log,
            commands::plugins::plugins_list,
            commands::plugins::plugins_inspect,
            commands::plugins::plugins_set_enabled,
            commands::plugins::plugins_install_from_path,
            commands::plugins::plugins_install_from_url,
            commands::plugins::plugins_uninstall,
            commands::plugins::plugins_scaffold,
            commands::browser::browser_status,
            commands::browser::browser_install,
            commands::browser::browser_uninstall,
            commands::browser::browser_close,
            commands::browser::browser_test,
            commands::local_llm::local_status,
            commands::local_llm::local_hardware,
            commands::local_llm::local_curated_models,
            commands::local_llm::local_install_server,
            commands::local_llm::local_uninstall_server,
            commands::local_llm::local_search_models,
            commands::local_llm::local_repo_quants,
            commands::local_llm::local_install_model,
            commands::local_llm::local_install_drafter,
            commands::local_llm::local_cancel_install,
            commands::local_llm::local_list_models,
            commands::local_llm::local_remove_model,
            commands::local_llm::local_unload_model,
            commands::local_llm::local_server_logs,
            commands::local_llm::local_disk_usage,
            commands::local_llm::local_test_model,
            commands::local_llm::local_runtime_stats,
            commands::local_llm::local_install_mlx,
            commands::local_llm::local_mtplx_models,
            commands::local_llm::local_install_mtplx,
            commands::local_llm::local_uninstall_mtplx,
            commands::local_llm::local_uninstall_mlx,
            commands::mcp::mcp_list_servers,
            commands::mcp::mcp_test_server,
            commands::mcp::mcp_reconnect,
            commands::shell::open_in_terminal,
            commands::power::set_keep_awake,
            commands::ide::detect_ides,
            commands::ide::open_in_ide,
            askpass::answer_askpass,
        ])
        .build(tauri::generate_context!())
        .expect("error while building tauri application")
        .run(|_app, event| {
            // `kill_on_drop` covers a panic, but not process exit: the server
            // registry is a static and is never dropped, so without this a
            // quit leaves llama-server holding the model's worth of RAM.
            if matches!(event, tauri::RunEvent::Exit) {
                tauri::async_runtime::block_on(llama::supervisor::stop_all());
            }
        });
}

#[cfg(test)]
mod architecture_tests {
    use std::path::Path;

    /// `commands/` is the adapter layer: it may reach into the core, never the
    /// other way round. This used to be violated by `commands::procutil` (a
    /// platform helper with no IPC in it), `commands::tasks` (JSONL persistence)
    /// and `commands::code_intel::INDEX_SEMAPHORE` — each pulling the core back
    /// into the adapter. The rule is cheap to state and easy to break silently,
    /// so it is checked instead of only documented.
    #[test]
    fn core_modules_do_not_depend_on_the_command_layer() {
        let mut offenders = Vec::new();
        for dir in [
            "src/agent",
            "src/browser",
            "src/code_intel",
            "src/llama",
            "src/lsp",
            "src/quality",
        ] {
            visit(Path::new(dir), &mut |path, body| {
                if body.contains("crate::commands") {
                    offenders.push(path.display().to_string());
                }
            });
        }
        assert!(
            offenders.is_empty(),
            "these core files import crate::commands, inverting the layering: {offenders:?}"
        );
    }

    fn visit(dir: &Path, f: &mut impl FnMut(&Path, &str)) {
        let Ok(entries) = std::fs::read_dir(dir) else {
            return;
        };
        for entry in entries.flatten() {
            let path = entry.path();
            if path.is_dir() {
                visit(&path, f);
            } else if path.extension().is_some_and(|e| e == "rs")
                && let Ok(body) = std::fs::read_to_string(&path)
            {
                f(&path, &body);
            }
        }
    }
}
