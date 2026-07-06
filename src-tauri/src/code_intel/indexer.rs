use crate::code_intel::db::IndexDb;
use crate::code_intel::embeddings::{build_embedding_text, CodeEmbedder, SharedEmbedder};
use crate::code_intel::parser::{self, ParseResult};
use std::path::Path;
use std::time::SystemTime;
use xxhash_rust::xxh3::xxh3_64;
use tauri::Emitter;
use tauri::ipc::Channel;
use serde::Serialize;

#[derive(Debug, Clone, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct IndexProgress {
    pub status: String,
    pub files_indexed: i64,
    pub symbols_indexed: i64,
    pub total_files: i64,
}

pub fn compute_hash(content: &str) -> String {
    format!("{:x}", xxh3_64(content.as_bytes()))
}

pub fn index_file(
    db: &IndexDb,
    path: &str,
    content: &str,
    mut embedder: Option<&mut CodeEmbedder>,
) -> Result<ParseResult, String> {
    let lang = parser::detect_language(path).unwrap_or("unknown");
    let hash = compute_hash(content);
    let modified = SystemTime::now()
        .duration_since(SystemTime::UNIX_EPOCH)
        .map(|d| d.as_secs() as i64)
        .unwrap_or(0);
    let size = content.len() as i64;

    let file_id = db.upsert_file(path, lang, &hash, modified, size)?;

    let parse_result = parser::parse_file(path, content);

    if let Some(ref err) = parse_result.error {
        return Err(err.clone());
    }

    db.delete_symbols_for_file(file_id)?;

    let mut symbol_ids: Vec<(String, i64)> = Vec::new();

    for sym in &parse_result.symbols {
        let sig = sym.signature.as_deref();
        let doc = sym.doc_comment.as_deref();
        let id = db.insert_symbol(
            file_id,
            &sym.name,
            &sym.kind,
            sig,
            sym.start_line,
            sym.start_col,
            sym.end_line,
            sym.end_col,
            doc,
        )?;
        symbol_ids.push((sym.name.clone(), id));
    }

    for call in &parse_result.calls {
        let from_id = symbol_ids.iter().find(|(n, _)| n == &call.from_name).map(|(_, id)| *id);
        if let Some(fid) = from_id {
            let to_id = symbol_ids.iter().find(|(n, _)| n == &call.to_name).map(|(_, id)| *id);
            if let Some(tid) = to_id {
                db.insert_relation(fid, tid, "calls")?;
            }
        }
    }

    db.update_fts_for_file(file_id)?;

    if let Some(ref mut emb) = embedder {
        let texts: Vec<String> = parse_result
            .symbols
            .iter()
            .map(|sym| {
                build_embedding_text(
                    &sym.kind,
                    &sym.name,
                    sym.parent_context.as_deref(),
                    sym.doc_comment.as_deref(),
                    sym.body_text.as_deref(),
                )
            })
            .collect();

        if !texts.is_empty() {
            let str_refs: Vec<&str> = texts.iter().map(|s| s.as_str()).collect();
            if let Ok(vectors) = emb.encode(&str_refs) {
                for ((_, sid), vec) in symbol_ids.iter().zip(vectors.iter()) {
                    let _ = db.upsert_embedding(*sid, vec);
                }
            }
        }
    }

    Ok(parse_result)
}

pub fn scan_workspace(
    db: &IndexDb,
    root: &str,
    app_handle: Option<&tauri::AppHandle>,
    mut embedder: Option<&mut CodeEmbedder>,
    progress_channel: Option<&Channel<IndexProgress>>,
) -> Result<(i64, i64), String> {
    let mut total_files = 0i64;
    let mut total_symbols = 0i64;
    let mut counted = 0i64;

    let walker = ignore::WalkBuilder::new(root)
        .git_ignore(true)
        .git_global(true)
        .hidden(true)
        .build();

    let all_paths: Vec<String> = walker
        .filter_map(|e| e.ok())
        .filter(|e| e.file_type().map(|t| t.is_file()).unwrap_or(false))
        .filter(|e| {
            parser::detect_language(
                e.path().to_string_lossy().as_ref(),
            )
            .is_some()
        })
        .map(|e| e.path().to_string_lossy().to_string())
        .collect();

    let total = all_paths.len() as i64;

    let initial_progress = IndexProgress {
        status: "indexing".into(),
        files_indexed: 0,
        symbols_indexed: 0,
        total_files: total,
    };
    if let Some(handle) = app_handle.as_ref() {
        let _ = handle.emit("index-progress", initial_progress.clone());
    }
    if let Some(ch) = progress_channel {
        let _ = ch.send(initial_progress);
    }

    for path_str in &all_paths {
        let content = match std::fs::read_to_string(path_str) {
            Ok(c) => c,
            Err(_) => continue,
        };

        match index_file(db, path_str, &content, embedder.as_deref_mut()) {
            Ok(parse_result) => {
                total_files += 1;
                total_symbols += parse_result.symbols.len() as i64;
            }
            Err(_) => {
                let lang = parser::detect_language(path_str).unwrap_or("unknown");
                let hash = compute_hash(&content);
                let modified = std::fs::metadata(path_str)
                    .and_then(|m| m.modified().map(|t| t.duration_since(SystemTime::UNIX_EPOCH).map(|d| d.as_secs() as i64).unwrap_or(0)))
                    .unwrap_or(0);
                let size = content.len() as i64;
                let _ = db.upsert_file(path_str, lang, &hash, modified, size);
                total_files += 1;
            }
        }

        counted += 1;
        if counted % 10 == 0 {
            let prog = IndexProgress {
                status: "indexing".into(),
                files_indexed: counted,
                symbols_indexed: total_symbols,
                total_files: total,
            };
            if let Some(handle) = app_handle {
                let _ = handle.emit("index-progress", prog.clone());
            }
            if let Some(ch) = progress_channel {
                let _ = ch.send(prog);
            }
        }
    }

    // Drop rows for files no longer in the scan set (deleted files, or junk
    // like node_modules/dist indexed before ignore rules existed).
    let keep: std::collections::HashSet<String> = all_paths.iter().cloned().collect();
    match db.prune_files_not_in(&keep) {
        Ok(pruned) if pruned > 0 => eprintln!("[indexer] pruned {pruned} stale files from index"),
        Ok(_) => {}
        Err(e) => eprintln!("[indexer] prune failed: {e}"),
    }

    let done_progress = IndexProgress {
        status: "done".into(),
        files_indexed: total_files,
        symbols_indexed: total_symbols,
        total_files: total,
    };
    if let Some(handle) = app_handle {
        let _ = handle.emit("index-progress", done_progress.clone());
    }
    if let Some(ch) = progress_channel {
        let _ = ch.send(done_progress);
    }

    Ok((total_files, total_symbols))
}

pub fn generate_all_embeddings(
    db: &IndexDb,
    embedder: &SharedEmbedder,
    app_handle: Option<&tauri::AppHandle>,
) -> Result<(i64, i64), String> {
    let files = db.all_files()?;
    let total = files.len() as i64;
    let mut processed = 0i64;
    let mut total_embeddings = 0i64;
    let mut failed = 0i64;

    for file in &files {
        processed += 1;

        let content = match std::fs::read_to_string(&file.path) {
            Ok(c) => c,
            Err(_) => continue,
        };
        let parse_result = parser::parse_file(&file.path, &content);
        if parse_result.error.is_some() {
            continue;
        }

        // Delete old embeddings for this file so stale symbols don't linger
        let _ = db.delete_embeddings_for_file(file.id);

        let texts: Vec<String> = parse_result
            .symbols
            .iter()
            .map(|sym| {
                build_embedding_text(
                    &sym.kind,
                    &sym.name,
                    sym.parent_context.as_deref(),
                    sym.doc_comment.as_deref(),
                    sym.body_text.as_deref(),
                )
            })
            .collect();

        if !texts.is_empty() {
            let db_symbols = db.symbols_in_file(&file.path)?;
            // Encode in small batches, locking per batch, so the watcher and
            // semantic_search never wait long and memory stays bounded.
            for (chunk_syms, chunk_texts) in parse_result
                .symbols
                .chunks(16)
                .zip(texts.chunks(16))
            {
                let str_refs: Vec<&str> = chunk_texts.iter().map(|s| s.as_str()).collect();
                let vectors = {
                    let mut emb = match embedder.lock() {
                        Ok(g) => g,
                        Err(e) => return Err(format!("embedder lock poisoned: {e}")),
                    };
                    emb.encode(&str_refs)
                };
                match vectors {
                    Ok(vectors) => {
                        for (sym, vec) in chunk_syms.iter().zip(vectors.iter()) {
                            // Match parsed symbols to DB rows by identity, not position.
                            let row = db_symbols.iter().find(|r| {
                                r.name == sym.name
                                    && r.kind == sym.kind
                                    && r.start_line == sym.start_line
                            });
                            if let Some(row) = row {
                                if db.upsert_embedding(row.id, vec).is_ok() {
                                    total_embeddings += 1;
                                }
                            }
                        }
                    }
                    Err(e) => {
                        failed += 1;
                        eprintln!("[embeddings] encode failed for {}: {e}", file.path);
                        break;
                    }
                }
            }
        }

        if processed % 10 == 0 {
            if let Some(handle) = app_handle {
                let _ = handle.emit("index-progress", IndexProgress {
                    status: "embedding".into(),
                    files_indexed: processed,
                    symbols_indexed: total_embeddings,
                    total_files: total,
                });
            }
        }
    }

    if failed > 0 {
        eprintln!("[embeddings] {failed}/{total} files failed to encode");
    }

    Ok((processed, total_embeddings))
}

pub fn reindex_file(db: &IndexDb, path: &str, embedder: Option<&mut CodeEmbedder>) -> Result<Option<ParseResult>, String> {
    let existing = db.file_by_path(path)?;
    let content = match std::fs::read_to_string(path) {
        Ok(c) => c,
        Err(_) => {
            if existing.is_some() {
                let conn = db.conn.lock().map_err(|e| e.to_string())?;
                let _ = conn.execute("DELETE FROM files WHERE path = ?1", rusqlite::params![path]);
            }
            return Ok(None);
        }
    };

    let new_hash = compute_hash(&content);
    if let Some(ref file) = existing {
        if file.hash.as_deref() == Some(&new_hash) {
            return Ok(None);
        }
    }

    let result = index_file(db, path, &content, embedder).ok();
    Ok(result)
}
