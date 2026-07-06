use crate::code_intel::db::{IndexDb, SearchResult, SymbolRecord};
use crate::code_intel::embeddings;
use crate::code_intel::indexer;
use crate::code_intel::watcher::FileWatcher;
use crate::state::AppState;
use serde::Serialize;
use std::path::Path;
use std::sync::Arc;
use tauri::ipc::Channel;
use tauri::Emitter;
use tauri::Manager;
use tauri::State;
use tokio::task::spawn_blocking;

#[derive(Serialize)]
#[serde(rename_all = "camelCase")]
pub struct IndexStatus {
    pub status: String,
    pub files_count: i64,
    pub symbols_count: i64,
}

fn resolve_model_dir(app_handle: &tauri::AppHandle) -> Option<std::path::PathBuf> {
    if let Ok(r) = app_handle.path().resource_dir() {
        let p = r.join("models/LateOn-Code-edge");
        if p.join("model_int8.onnx").exists() {
            return Some(p);
        }
    }
    if let Ok(manifest) = std::env::var("CARGO_MANIFEST_DIR") {
        let p = Path::new(&manifest).join("models/LateOn-Code-edge");
        if p.join("model_int8.onnx").exists() {
            return Some(p);
        }
    }
    let cache = dirs::config_dir()
        .unwrap_or_else(|| Path::new(".").to_path_buf())
        .join("claudinio-code/models/LateOn-Code-edge");
    if cache.join("model_int8.onnx").exists() {
        return Some(cache);
    }
    None
}

fn cache_model_dir() -> std::path::PathBuf {
    dirs::config_dir()
        .unwrap_or_else(|| Path::new(".").to_path_buf())
        .join("claudinio-code/models/LateOn-Code-edge")
}

#[tauri::command]
pub async fn open_workspace(
    path: String,
    app_handle: tauri::AppHandle,
    state: State<'_, AppState>,
    progress_channel: Channel<indexer::IndexProgress>,
) -> Result<IndexStatus, String> {
    let db_path = Path::new(&path).join(".claudinio_index.db");
    let db = Arc::new(IndexDb::open(&db_path)?);

    let _ = app_handle.emit("index-progress", indexer::IndexProgress {
        status: "loading_model".into(),
        files_indexed: 0,
        symbols_indexed: 0,
        total_files: 0,
    });

    // Download model to cache if not bundled
    if resolve_model_dir(&app_handle).is_none() {
        let cache = cache_model_dir();
        if let Err(e) = embeddings::ensure_model_downloaded(&cache).await {
            eprintln!("[open_workspace] failed to download embedding model: {e}");
            let _ = app_handle.emit("index-progress", indexer::IndexProgress {
                status: "embedding_model_error".into(),
                files_indexed: 0,
                symbols_indexed: 0,
                total_files: 0,
            });
        }
    }

    // Phase 1: Start scanning WITHOUT embeddings immediately
    let scan_handle = spawn_blocking({
        let db = Arc::clone(&db);
        let path = path.clone();
        let app_handle = app_handle.clone();
        let progress_channel = progress_channel.clone();
        move || {
            indexer::scan_workspace(
                db.as_ref(),
                &path,
                Some(&app_handle),
                None, // no embedder yet
                Some(&progress_channel),
            )
        }
    });

    // Phase 2: In parallel, try to load the embedding model
    let model_handle = spawn_blocking({
        let app_handle = app_handle.clone();
        move || match resolve_model_dir(&app_handle) {
            Some(d) => match embeddings::load_shared(&d) {
                Ok(shared) => Some(shared),
                Err(e) => {
                    eprintln!("[open_workspace] embedding model load failed: {e}");
                    None
                }
            },
            None => {
                eprintln!("[open_workspace] embedding model not found (bundle, dev dir and cache all missing)");
                None
            }
        }
    });

    // Phase 3: Wait for scan to complete (this is what the user sees)
    let (files_count, symbols_count) = scan_handle
        .await
        .map_err(|e| format!("scan task panicked: {e}"))?
        .map_err(|e| e)?;

    // Phase 4: Try to get embedder with a generous timeout
    let embedder = tokio::time::timeout(std::time::Duration::from_secs(30), model_handle)
        .await
        .ok()
        .and_then(|r| r.ok())
        .flatten();

    // Publish the model as soon as it's available so agent tools can use it,
    // even while Phase 5 is still generating embeddings.
    {
        let mut em = state.embedding_model.lock().await;
        *em = embedder.clone();
    }

    // Phase 5: If model loaded, generate embeddings for all indexed symbols
    if let Some(ref shared) = embedder {
        let shared = std::sync::Arc::clone(shared);
        let db2 = Arc::clone(&db);
        let emit_handle = app_handle.clone();
        let join = spawn_blocking(move || {
            indexer::generate_all_embeddings(db2.as_ref(), &shared, Some(&emit_handle))
        });
        let emit_handle = app_handle.clone();
        tokio::spawn(async move {
            let result = join.await;
            let (status, files, symbols) = match result {
                Ok(Ok((processed, total))) => ("embeddings_done", processed, total),
                Ok(Err(e)) => {
                    eprintln!("[open_workspace] embedding generation failed: {e}");
                    ("embeddings_error", 0, 0)
                }
                Err(e) => {
                    eprintln!("[open_workspace] embedding task panicked: {e}");
                    ("embeddings_error", 0, 0)
                }
            };
            let _ = emit_handle.emit("index-progress", indexer::IndexProgress {
                status: status.into(),
                files_indexed: files,
                symbols_indexed: symbols,
                total_files: files,
            });
        });
    }

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
        // Update the SkillManager with the new workspace root
        let mut skills = state.skills_manager.lock().await;
        *skills = crate::agent::skills::SkillManager::new(Some(std::path::PathBuf::from(&path)));
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
