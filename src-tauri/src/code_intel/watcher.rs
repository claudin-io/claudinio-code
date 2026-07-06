use crate::code_intel::db::IndexDb;
use crate::code_intel::embeddings::SharedEmbedder;
use crate::code_intel::indexer;
use crate::state::AppState;
use notify::Config;
use notify::RecommendedWatcher;
use notify::RecursiveMode;
use notify::Watcher;
use std::path::Path;
use std::time::Duration;
use tauri::Emitter;
use tauri::Manager;

fn resolve_embedder(handle: &tauri::AppHandle) -> Option<SharedEmbedder> {
    let state = handle.state::<AppState>();
    let guard = state.embedding_model.blocking_lock();
    guard.clone()
}

pub struct FileWatcher {
    _watcher: RecommendedWatcher,
}

impl FileWatcher {
    pub fn start(
        root: &str,
        db_path: &Path,
        app_handle: tauri::AppHandle,
    ) -> Result<Self, String> {
        let db_path = db_path.to_path_buf();
        let handle = app_handle.clone();

        let mut watcher = notify::recommended_watcher(
            move |event: Result<notify::Event, notify::Error>| {
                let event = match event {
                    Ok(e) => e,
                    Err(_) => return,
                };

                let paths: Vec<String> = event
                    .paths
                    .iter()
                    .filter(|p| p.is_file())
                    .filter(|p| {
                        p.extension()
                            .and_then(|e| e.to_str())
                            .is_some_and(|e| matches!(e, "ts" | "tsx" | "js" | "jsx" | "rs" | "py" | "swift"))
                    })
                    .map(|p| p.to_string_lossy().to_string())
                    .collect();

                if paths.is_empty() {
                    return;
                }

                let db_p = db_path.clone();
                let h = handle.clone();
                std::thread::spawn(move || {
                    std::thread::sleep(Duration::from_millis(1500));

                    let db = match IndexDb::open(&db_p) {
                        Ok(d) => d,
                        Err(_) => return,
                    };

                    let embedder = resolve_embedder(&h);

                    for path_str in &paths {
                        let p = Path::new(path_str);
                        if !p.exists() {
                            if let Ok(Some(_)) = db.file_by_path(path_str) {
                                let conn = match db.conn.lock() {
                                    Ok(c) => c,
                                    Err(_) => continue,
                                };
                                let _ = conn.execute("DELETE FROM files WHERE path = ?1", rusqlite::params![path_str]);
                            }
                            continue;
                        }

                        let _ = h.emit("index-progress", serde_json::json!({
                            "status": "reindexing",
                            "file": path_str,
                        }));

                        let mut emb = embedder.as_ref().and_then(|e| e.lock().ok());
                        match indexer::reindex_file(&db, path_str, emb.as_deref_mut()) {
                            Ok(Some(result)) => {
                                let _ = h.emit("index-progress", serde_json::json!({
                                    "status": "reindexed",
                                    "file": path_str,
                                    "symbols": result.symbols.len(),
                                }));
                            }
                            Ok(None) => {}
                            Err(e) => {
                                let _ = h.emit("index-progress", serde_json::json!({
                                    "status": "reindex_error",
                                    "file": path_str,
                                    "error": e,
                                }));
                            }
                        }
                    }
                });
            },
        )
        .map_err(|e| format!("watcher create: {e}"))?;

        watcher
            .configure(Config::default().with_poll_interval(Duration::from_secs(2)))
            .map_err(|e| format!("watcher config: {e}"))?;

        watcher
            .watch(Path::new(root), RecursiveMode::Recursive)
            .map_err(|e| format!("watcher watch: {e}"))?;

        Ok(FileWatcher {
            _watcher: watcher,
        })
    }
}
