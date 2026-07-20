//! Wrapper Tauri para o bridge de askpass do core. A lógica (loopback HTTP,
//! resolução de prompts) vive em `claudinio_core::askpass`; aqui fica apenas o
//! `#[tauri::command]` que a UI chama para responder um prompt.

/// Resolve a pending prompt from the UI. `secret: None` = user cancelled.
#[tauri::command]
pub fn answer_askpass(id: u64, secret: Option<String>) {
    claudinio_core::askpass::answer_askpass(id, secret);
}
