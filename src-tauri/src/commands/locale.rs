use tauri;

/// Returns the OS locale string (e.g. "en-US", "pt-BR"), or "en-US" on failure.
#[tauri::command]
pub fn get_os_locale() -> String {
    sys_locale::get_locale().unwrap_or_else(|| "en-US".to_string())
}
