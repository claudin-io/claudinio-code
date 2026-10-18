//! The living document's IPC surface (lean-harness Phase 4b): the panel's
//! initial load and a person's save. Thin adapters; the logic is
//! `agent::doc_session`'s.

use crate::agent::doc_session::{self, DocView};
use crate::state::AppState;
use std::path::PathBuf;
use tauri::State;

/// The session file `session_id` names, or the workspace's active session.
async fn store_for(
    state: &State<'_, AppState>,
    workspace: &str,
    session_id: Option<String>,
) -> Option<(PathBuf, PathBuf)> {
    let ws = state.workspace(workspace).await.ok()?;
    let store = match session_id {
        Some(id) => doc_session::session_file(&ws.root, &id)?,
        None => ws.active_session.lock().await.as_ref()?.store_path.clone(),
    };
    Some((ws.root.clone(), store))
}

/// The session's document as the panel shows it, or `None` when it has none.
#[tauri::command]
pub async fn get_document(
    workspace: String,
    session_id: Option<String>,
    state: State<'_, AppState>,
) -> Result<Option<DocView>, String> {
    Ok(store_for(&state, &workspace, session_id)
        .await
        .and_then(|(root, store)| doc_session::view(&root, &store)))
}

/// Save a person's edit of the document from the app: `base` is the text the
/// editor loaded, `content` what they saved. Sections they changed become
/// `human`; what agents wrote meanwhile is kept. Returns the new view; the
/// file's new text is read back by the editor.
#[tauri::command]
pub async fn save_document(
    workspace: String,
    session_id: Option<String>,
    base: String,
    content: String,
    state: State<'_, AppState>,
) -> Result<DocView, String> {
    let (root, store) = store_for(&state, &workspace, session_id)
        .await
        .ok_or("no session with a document")?;
    doc_session::save_human(&root, &store, &base, &content)
}
