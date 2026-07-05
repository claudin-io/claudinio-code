use crate::code_intel::db::{IndexDb, SearchResult, SymbolRecord};
use crate::code_intel::indexer;
use crate::code_intel::watcher::FileWatcher;
use crate::state::AppState;
use serde::Serialize;
use std::path::Path;
use std::sync::Arc;
use tauri::State;
use tokio::task::spawn_blocking;

#[derive(Serialize)]
#[serde(rename_all = "camelCase")]
pub struct IndexStatus {
    pub status: String,
    pub files_count: i64,
    pub symbols_count: i64,
}

#[tauri::command]
pub async fn open_workspace(
    path: String,
    app_handle: tauri::AppHandle,
    state: State<'_, AppState>,
) -> Result<IndexStatus, String> {
    let db_path = Path::new(&path).join(".claudinio_index.db");
    let db = Arc::new(IndexDb::open(&db_path)?);

    let (files_count, symbols_count) = spawn_blocking({
        let db = Arc::clone(&db);
        let path = path.clone();
        let app_handle = app_handle.clone();
        move || indexer::scan_workspace(db.as_ref(), &path, Some(&app_handle))
    })
    .await
    .map_err(|e| format!("scan task panicked: {e}"))??;

    let watcher = FileWatcher::start(&path, &db_path, app_handle.clone())?;

    {
        let mut state_db = state.index_db.lock().await;
        *state_db = Some(db);
    }
    {
        let mut ws = state.workspace_root.lock().await;
        *ws = Some(std::path::PathBuf::from(&path));
    }
    {
        let mut w = state._watcher.lock().await;
        *w = Some(watcher);
    }

    {
        let mut lsp = state.lsp_manager.lock().await;
        let _ = lsp.start_for_workspace(&path);
    }

    Ok(IndexStatus {
        status: "ok".into(),
        files_count,
        symbols_count,
    })
}

#[tauri::command]
pub async fn search_symbols(
    query: String,
    limit: Option<i64>,
    state: State<'_, AppState>,
) -> Result<Vec<SearchResult>, String> {
    let db_guard = state.index_db.lock().await;
    let db = db_guard.as_ref().ok_or("no workspace open")?;
    db.search_symbols(&query, limit.unwrap_or(30))
}

#[tauri::command]
pub async fn symbol_lookup(
    name: String,
    state: State<'_, AppState>,
) -> Result<Vec<SearchResult>, String> {
    let db_guard = state.index_db.lock().await;
    let db = db_guard.as_ref().ok_or("no workspace open")?;
    db.search_symbols(&name, 20)
}

#[tauri::command]
pub async fn file_outline(
    file_path: String,
    state: State<'_, AppState>,
) -> Result<Vec<SymbolRecord>, String> {
    let db_guard = state.index_db.lock().await;
    let db = db_guard.as_ref().ok_or("no workspace open")?;
    db.symbols_in_file(&file_path)
}
