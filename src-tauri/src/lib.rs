mod agent;
mod code_intel;
mod commands;
mod lsp;
mod state;

use state::AppState;

#[cfg_attr(mobile, tauri::mobile_entry_point)]
pub fn run() {
    tauri::Builder::default()
        .plugin(tauri_plugin_opener::init())
        .plugin(tauri_plugin_dialog::init())
        .manage(AppState::new())
        .invoke_handler(tauri::generate_handler![
            commands::fs::list_dir,
            commands::fs::read_file,
            commands::fs::write_file,
            commands::agent::send_message,
            commands::agent::new_session,
            commands::agent::list_sessions,
            commands::agent::load_session,
            commands::agent::approve_tool,
            commands::agent::reject_tool,
            commands::agent::submit_answers,
            commands::agent::set_config,
            commands::agent::get_config,
            commands::code_intel::open_workspace,
            commands::code_intel::search_symbols,
            commands::code_intel::symbol_lookup,
            commands::code_intel::file_outline,
            commands::lsp::lsp_definition,
            commands::lsp::lsp_references,
            commands::lsp::lsp_hover,
            commands::agent::queue_steering,
            commands::agent::interrupt_session,
            commands::agent::compact_session,
            commands::skills::list_skills,
            commands::skills::get_skill_catalog,
            commands::skills::get_skill_content,
            commands::skills::rescan_skills,
            commands::skills::find_remote_skills,
            commands::skills::preview_remote_skill,
            commands::skills::install_remote_skill,
        ])
        .run(tauri::generate_context!())
        .expect("error while running tauri application");
}
