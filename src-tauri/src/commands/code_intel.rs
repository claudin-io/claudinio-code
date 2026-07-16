use crate::code_intel::db::{IndexDb, SearchResult, SymbolRecord};
use crate::code_intel::embeddings::{self, SharedEmbedder};
use crate::code_intel::indexer;
use crate::code_intel::watcher::FileWatcher;
use crate::state::{AppState, WorkspaceState};
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
    pub embeddings_count: i64,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub watcher_warning: Option<String>,
}

fn resolve_model_dir(app_handle: &tauri::AppHandle, workspace_root: &Path) -> Option<std::path::PathBuf> {
    let subdir = format!("models/{}", embeddings::model_cache_dirname());
    let model_file = embeddings::model_filename();
    if let Ok(r) = app_handle.path().resource_dir() {
        let p = r.join(&subdir);
        if p.join(model_file).exists() {
            return Some(p);
        }
    }
    if let Ok(manifest) = std::env::var("CARGO_MANIFEST_DIR") {
        let p = Path::new(&manifest).join(&subdir);
        if p.join(model_file).exists() {
            return Some(p);
        }
    }
    let cache = cache_model_dir(workspace_root);
    if cache.join(model_file).exists() {
        return Some(cache);
    }
    None
}

fn cache_model_dir(workspace_root: &Path) -> std::path::PathBuf {
    workspace_root
        .join(".claudinio/models")
        .join(embeddings::model_cache_dirname())
}

#[tauri::command]
pub async fn open_workspace(
    path: String,
    app_handle: tauri::AppHandle,
    state: State<'_, AppState>,
    progress_channel: Channel<indexer::IndexProgress>,
) -> Result<IndexStatus, String> {
    // Already open: switching back to this workspace must be cheap and must
    // not restart indexing/watcher/LSP under a possibly-running agent.
    if let Ok(ws) = state.workspace(&path).await {
        let (files_count, symbols_count, embeddings_count) = ws.index_db.index_stats().unwrap_or((0, 0, 0));
        let warning = ws.watcher_warning.lock().await.clone();
        return Ok(IndexStatus {
            status: "ok".into(),
            files_count,
            symbols_count,
            embeddings_count,
            watcher_warning: warning,
        });
    }

    let db_path = Path::new(&path).join(".claudinio/index.db");
    let db = Arc::new(IndexDb::open(&db_path)?);

    // ── Build workspace state EARLY so send_message works during indexing ──
    let root = std::path::PathBuf::from(&path);
    let index_progress: Arc<std::sync::Mutex<Option<indexer::IndexProgress>>> =
        Arc::new(std::sync::Mutex::new(Some(indexer::IndexProgress {
            status: "indexing".into(),
            files_indexed: 0,
            symbols_indexed: 0,
            total_files: 0,
            workspace: path.clone(),
        })));
    let lsp_manager = Arc::new(tokio::sync::Mutex::new(crate::lsp::manager::LspManager::new()));
    let early_workspace = Arc::new(WorkspaceState {
        root: root.clone(),
        index_db: db.clone(),
        skills_manager: Arc::new(tokio::sync::Mutex::new(
            crate::agent::skills::SkillManager::new(Some(root.clone())),
        )),
        lsp_manager: lsp_manager.clone(),
        _watcher: tokio::sync::Mutex::new(None),
        watcher_warning: tokio::sync::Mutex::new(None),
        active_session: tokio::sync::Mutex::new(None),
        mcp: tokio::sync::Mutex::new(None),
        mcp_fingerprint: tokio::sync::Mutex::new(None),
        index_progress: index_progress.clone(),
    });
    {
        let mut map = state.workspaces.lock().await;
        map.insert(root.clone(), early_workspace);
    }
    // Get a handle back to update fields later (lsp, watcher, index_progress)
    let ws = state.workspace(&path).await.map_err(|e| {
        // Clean up: if we somehow can't get it back, clear progress
        let _ = index_progress.lock().map(|mut p| *p = None);
        e
    })?;

    let code_intel_enabled = state.config.lock().await.code_intel_enabled;

    // ── When code intel is disabled, skip indexing, embeddings, watcher, LSP ──
    if !code_intel_enabled {
        return Ok(IndexStatus {
            status: "ok".into(),
            files_count: 0,
            symbols_count: 0,
            embeddings_count: 0,
            watcher_warning: None,
        });
    }

    let _ = app_handle.emit("index-progress", indexer::IndexProgress {
        status: "loading_model".into(),
        files_indexed: 0,
        symbols_indexed: 0,
        total_files: 0,
        workspace: path.clone(),
    });

    let ws_root = Path::new(&path);

    // Download model to cache if not bundled
    if resolve_model_dir(&app_handle, ws_root).is_none() {
        let cache = cache_model_dir(ws_root);
        if let Err(e) = embeddings::ensure_model_downloaded(&cache).await {
            eprintln!("[open_workspace] failed to download embedding model: {e}");
            let _ = app_handle.emit("index-progress", indexer::IndexProgress {
                status: "embedding_model_error".into(),
                files_indexed: 0,
                symbols_indexed: 0,
                total_files: 0,
                workspace: path.clone(),
            });
        }
    }

    // Phase 1: Start scanning WITHOUT embeddings immediately
    let scan_handle = spawn_blocking({
        let db = db.clone();
        let path = path.clone();
        let app_handle = app_handle.clone();
        let progress_channel = progress_channel.clone();
        let shared_progress = index_progress.clone();
        move || {
            indexer::scan_workspace(
                db.as_ref(),
                &path,
                Some(&app_handle),
                None, // no embedder yet
                Some(&progress_channel),
                Some(&shared_progress),
            )
        }
    });

    // Phase 2: Reuse existing embedding model if already loaded — it is the
    // same embedding model for every workspace. On the very first
    // call, spawn the load in parallel with the scan (same as before).
    let model_handle = {
        let guard = state.embedding_model.lock().await;
        if guard.is_some() {
            // Already loaded — no disk I/O needed, reuse the same Arc.
            None
        } else {
            // First load: spawn in background (parallel with scan).
            Some(spawn_blocking({
                let app_handle = app_handle.clone();
                let path = path.clone();
                move || match resolve_model_dir(&app_handle, std::path::Path::new(&path)) {
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
            }))
        }
    };

    // Phase 3: Wait for scan to complete (this is what the user sees)
    let (files_count, symbols_count) = scan_handle
        .await
        .map_err(|e| format!("scan task panicked: {e}"))?
        .map_err(|e| e)?;

    // Index scan is done — clear the shared progress so tools know the index is ready
    {
        let mut p = index_progress.lock().map_err(|e| format!("index_progress lock: {e}"))?;
        *p = None;
    }

    // Query persisted embeddings count from a prior session (embedding phase
    // runs async after this and will be reflected on re-open).
    let embeddings_count = db.index_stats().unwrap_or((0, 0, 0)).2;

    // Phase 4: If we spawned a load in Phase 2, await it with timeout.
    // Otherwise reuse the model already in state.
    let embedder: Option<SharedEmbedder> = if let Some(handle) = model_handle {
        tokio::time::timeout(std::time::Duration::from_secs(30), handle)
            .await
            .ok()
            .and_then(|r| r.ok())
            .flatten()
    } else {
        let guard = state.embedding_model.lock().await;
        guard.clone()
    };

    // Publish the model as soon as it's available so agent tools can use it,
    // even while Phase 5 is still generating embeddings.
    {
        let mut em = state.embedding_model.lock().await;
        *em = embedder.clone();
    }

    // Phase 5: If model loaded, generate embeddings for all indexed symbols
    if let Some(ref shared) = embedder {
        let shared = Arc::clone(shared);
        let db2 = db.clone();
        let emit_handle = app_handle.clone();
        let ws_path = path.clone();
        let join = spawn_blocking(move || {
            indexer::generate_all_embeddings(db2.as_ref(), &shared, Some(&emit_handle), &ws_path)
        });
        let emit_handle = app_handle.clone();
        let db3 = db.clone();
        let ws_path = path.clone();
        tokio::spawn(async move {
            let result = join.await;
            let (status, files) = match result {
                Ok(Ok((processed, _total))) => ("embeddings_done", processed),
                Ok(Err(e)) => {
                    eprintln!("[open_workspace] embedding generation failed: {e}");
                    ("embeddings_error", 0)
                }
                Err(e) => {
                    eprintln!("[open_workspace] embedding task panicked: {e}");
                    ("embeddings_error", 0)
                }
            };
            // Query real embeddings count from the DB — generate_all_embeddings
            // only returns newly generated embeddings (0 on re-open since all
            // files are already embedded), but we need the total count for the UI.
            let real_embeddings = db3.index_stats().unwrap_or((0, 0, 0)).2;
            let _ = emit_handle.emit("index-progress", indexer::IndexProgress {
                status: status.into(),
                files_indexed: files,
                symbols_indexed: real_embeddings,
                total_files: files,
                workspace: ws_path,
            });
        });
    }

    let (watcher, watcher_warning): (Option<FileWatcher>, Option<String>) =
        match FileWatcher::start(&path, &db_path, app_handle.clone()) {
            Ok(w) => (Some(w), None),
            Err(e) => {
                eprintln!("[open_workspace] file watcher failed (workspace still usable): {e}");
                let _ = app_handle.emit("index-progress", indexer::IndexProgress {
                    status: "watcher_warning".into(),
                    files_indexed: files_count,
                    symbols_indexed: symbols_count,
                    total_files: files_count,
                    workspace: path.clone(),
                });
                (None, Some(format!("Live file watching unavailable: {e}")))
            }
        };

    // Update watcher and watcher_warning on the already-inserted workspace
    {
        let mut w = ws._watcher.lock().await;
        *w = watcher;
    }
    {
        let mut w = ws.watcher_warning.lock().await;
        *w = watcher_warning;
    }

    // Start LSP for the workspace
    {
        let mut lsp = ws.lsp_manager.lock().await;
        let _ = lsp.start_for_workspace(&path);
    }

    Ok(IndexStatus {
        status: "ok".into(),
        files_count,
        symbols_count,
        embeddings_count,
        watcher_warning: None,
    })
}

/// Close an open workspace: drops its watcher, LSP servers and index handle.
/// Any in-flight agent run keeps its own snapshot of these and finishes.
#[tauri::command]
pub async fn close_workspace(
    path: String,
    state: State<'_, AppState>,
) -> Result<(), String> {
    let removed = {
        let mut map = state.workspaces.lock().await;
        map.remove(Path::new(&path))
    };
    if let Some(ws) = removed {
        if let Some(mcp) = ws.mcp.lock().await.take() {
            mcp.shutdown().await;
        }
    }
    Ok(())
}

#[tauri::command]
pub async fn search_symbols(
    workspace: String,
    query: String,
    limit: Option<i64>,
    state: State<'_, AppState>,
) -> Result<Vec<SearchResult>, String> {
    let ws = state.workspace(&workspace).await?;
    ws.index_db.search_symbols(&query, limit.unwrap_or(30))
}

#[tauri::command]
pub async fn symbol_lookup(
    workspace: String,
    name: String,
    state: State<'_, AppState>,
) -> Result<Vec<SearchResult>, String> {
    let ws = state.workspace(&workspace).await?;
    ws.index_db.search_symbols(&name, 20)
}

#[tauri::command]
pub async fn file_outline(
    workspace: String,
    file_path: String,
    state: State<'_, AppState>,
) -> Result<Vec<SymbolRecord>, String> {
    let ws = state.workspace(&workspace).await?;
    ws.index_db.symbols_in_file(&file_path)
}
