use crate::code_intel::db::IndexDb;
use crate::code_intel::embeddings::{
    build_embedding_chunks, build_embedding_text, CodeEmbedder, EmbedChunk, SharedEmbedder,
};
use crate::code_intel::parser::{self, ParseResult};
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
    /// Root path of the workspace this progress belongs to, so the frontend
    /// can route global "index-progress" events to the right workspace entry.
    pub workspace: String,
}

pub fn compute_hash(content: &str) -> String {
    format!("{:x}", xxh3_64(content.as_bytes()))
}

/// Symbol kinds excluded from the embedding index: they carry no retrieval
/// signal and crowd out real results. `import` is our own synthesized kind
/// (see parser::IMPORT_KINDS); the others are TS/JS, C/C++/C#, and Kotlin/
/// Scala property node kinds from parser::DECLARATION_KINDS.
const NON_RETRIEVABLE_KINDS: &[&str] = &[
    "import",
    "property_signature",
    "field_declaration",
    "property_declaration",
];

/// Generic names that carry no retrieval signal on their own — only worth
/// embedding if accompanied by substantial doc/body text.
const GENERIC_NAMES: &[&str] = &[
    "props", "children", "id", "key", "value", "classname", "class_name",
    "name", "type", "data", "item", "items", "index", "i", "x", "y", "_",
];

/// Symbols excluded from the embedding index: they carry no retrieval signal
/// and crowd out real results (imports, tiny property signatures, generic names).
fn should_embed_symbol(kind: &str, name: &str, embedding_text: &str) -> bool {
    if NON_RETRIEVABLE_KINDS.contains(&kind) {
        return false;
    }
    if embedding_text.len() < 40 {
        return false;
    }
    if GENERIC_NAMES.contains(&name.to_lowercase().as_str()) && embedding_text.len() < 120 {
        return false;
    }
    true
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
        let chunks = collect_embedding_chunks(&parse_result.symbols, &symbol_ids);
        encode_and_store_batched(db, emb, &chunks);
    }

    Ok(parse_result)
}

/// Gate symbols through `should_embed_symbol`, then split survivors into
/// line-based chunks paired with their DB symbol id. Language-agnostic: it
/// only looks at the extracted body text, never at syntax.
fn collect_embedding_chunks(
    symbols: &[parser::ParsedSymbol],
    symbol_ids: &[(String, i64)],
) -> Vec<(i64, EmbedChunk)> {
    let mut out: Vec<(i64, EmbedChunk)> = Vec::new();
    for (sym, (_, id)) in symbols.iter().zip(symbol_ids.iter()) {
        let gate_text = build_embedding_text(
            &sym.kind,
            &sym.name,
            sym.parent_context.as_deref(),
            sym.doc_comment.as_deref(),
            sym.body_text.as_deref(),
        );
        if !should_embed_symbol(&sym.kind, &sym.name, &gate_text) {
            continue;
        }
        for chunk in build_embedding_chunks(
            &sym.kind,
            &sym.name,
            sym.parent_context.as_deref(),
            sym.doc_comment.as_deref(),
            sym.body_text.as_deref(),
            sym.start_line,
            sym.end_line,
        ) {
            out.push((*id, chunk));
        }
    }
    out
}

pub fn index_doc_file(
    db: &IndexDb,
    path: &str,
    content: &str,
    mut embedder: Option<&mut CodeEmbedder>,
) -> Result<usize, String> {
    let lang = parser::detect_doc_language(path).unwrap_or("markdown");
    let hash = compute_hash(content);
    let modified = SystemTime::now()
        .duration_since(SystemTime::UNIX_EPOCH)
        .map(|d| d.as_secs() as i64)
        .unwrap_or(0);
    let size = content.len() as i64;

    let file_id = db.upsert_file(path, lang, &hash, modified, size)?;
    let doc_symbols = parser::parse_doc_file(path, content);

    db.delete_symbols_for_file(file_id)?;

    let mut symbol_ids: Vec<(String, i64)> = Vec::new();
    for sym in &doc_symbols {
        let id = db.insert_symbol(
            file_id,
            &sym.name,
            &sym.kind,
            None,
            sym.start_line,
            sym.start_col,
            sym.end_line,
            sym.end_col,
            sym.doc_comment.as_deref(),
        )?;
        symbol_ids.push((sym.name.clone(), id));
    }

    if let Some(ref mut emb) = embedder {
        // Doc symbols carry no doc_comment (the body is the content), so the
        // shared chunk collector works for them unchanged.
        let chunks = collect_embedding_chunks(&doc_symbols, &symbol_ids);
        encode_and_store_batched(db, emb, &chunks);
    }

    Ok(doc_symbols.len())
}

/// Encode+store embeddings in bounded-size chunks so memory stays flat
/// regardless of how many symbols a single file produces (e.g. a minified
/// bundle can yield thousands of "symbols" in one shot).
const EMBED_BATCH_SIZE: usize = 16;

fn encode_and_store_batched(db: &IndexDb, emb: &mut CodeEmbedder, chunks: &[(i64, EmbedChunk)]) {
    for batch in chunks.chunks(EMBED_BATCH_SIZE) {
        let str_refs: Vec<&str> = batch.iter().map(|(_, c)| c.text.as_str()).collect();
        if let Ok(vectors) = emb.encode(&str_refs) {
            for ((sid, chunk), vec) in batch.iter().zip(vectors.iter()) {
                let _ = db.upsert_embedding(*sid, chunk.chunk_index, chunk.start_line, chunk.end_line, vec);
            }
        }
    }
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
        workspace: root.to_string(),
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
                workspace: root.to_string(),
            };
            if let Some(handle) = app_handle {
                let _ = handle.emit("index-progress", prog.clone());
            }
            if let Some(ch) = progress_channel {
                let _ = ch.send(prog);
            }
        }
    }

    // ── Second pass: documentation files (.md, .mdx, .txt) ──────────────
    let doc_walker = ignore::WalkBuilder::new(root)
        .git_ignore(true)
        .git_global(true)
        .hidden(true)
        .build();

    let doc_paths: Vec<String> = doc_walker
        .filter_map(|e| e.ok())
        .filter(|e| e.file_type().map(|t| t.is_file()).unwrap_or(false))
        .filter(|e| {
            let p = e.path().to_string_lossy();
            p.ends_with(".md") || p.ends_with(".mdx") || p.ends_with(".txt")
        })
        .map(|e| e.path().to_string_lossy().to_string())
        .collect();

    let doc_total = doc_paths.len() as i64;

    // Update total for progress display: code files + doc files
    let grand_total = total + doc_total;
    let grand_progress = IndexProgress {
        status: "indexing".into(),
        files_indexed: total_files,
        symbols_indexed: total_symbols,
        total_files: grand_total,
        workspace: root.to_string(),
    };
    if let Some(handle) = app_handle.as_ref() {
        let _ = handle.emit("index-progress", grand_progress);
    }

    for path_str in &doc_paths {
        let content = match std::fs::read_to_string(path_str) {
            Ok(c) => c,
            Err(_) => continue,
        };

        match index_doc_file(db, path_str, &content, embedder.as_deref_mut()) {
            Ok(num_symbols) => {
                total_files += 1;
                total_symbols += num_symbols as i64;
            }
            Err(_) => {
                total_files += 1;
            }
        }
    }

    // Drop rows for files no longer in the scan set (deleted files, or junk
    // like node_modules/dist indexed before ignore rules existed).
    let mut keep: std::collections::HashSet<String> = all_paths.iter().cloned().collect();
    keep.extend(doc_paths);
    match db.prune_files_not_in(&keep) {
        Ok(pruned) if pruned > 0 => eprintln!("[indexer] pruned {pruned} stale files from index"),
        Ok(_) => {}
        Err(e) => eprintln!("[indexer] prune failed: {e}"),
    }

    let done_progress = IndexProgress {
        status: "done".into(),
        files_indexed: total_files,
        symbols_indexed: total_symbols,
        total_files: grand_total,
        workspace: root.to_string(),
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
    workspace: &str,
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

        // Delete old embeddings for this file so stale symbols don't linger
        let _ = db.delete_embeddings_for_file(file.id);

        // For doc files, use parse_doc_file; for code files, use tree-sitter parser
        let symbols: Vec<parser::ParsedSymbol> = if file.language.as_deref() == Some("markdown")
            || file.language.as_deref() == Some("text")
        {
            parser::parse_doc_file(&file.path, &content)
        } else {
            let parse_result = parser::parse_file(&file.path, &content);
            if parse_result.error.is_some() {
                continue;
            }
            parse_result.symbols
        };

        // Skip low-signal symbols (imports, tiny property signatures, generic
        // names), then split survivors into line-based chunks. Chunks stay
        // paired with their parsed symbol so the DB-row lookup below matches
        // by identity, not position.
        let filtered: Vec<(&parser::ParsedSymbol, EmbedChunk)> = symbols
            .iter()
            .filter(|sym| {
                let gate_text = build_embedding_text(
                    &sym.kind,
                    &sym.name,
                    sym.parent_context.as_deref(),
                    sym.doc_comment.as_deref(),
                    sym.body_text.as_deref(),
                );
                should_embed_symbol(&sym.kind, &sym.name, &gate_text)
            })
            .flat_map(|sym| {
                build_embedding_chunks(
                    &sym.kind,
                    &sym.name,
                    sym.parent_context.as_deref(),
                    sym.doc_comment.as_deref(),
                    sym.body_text.as_deref(),
                    sym.start_line,
                    sym.end_line,
                )
                .into_iter()
                .map(move |c| (sym, c))
            })
            .collect();

        if !filtered.is_empty() {
            let db_symbols = db.symbols_in_file(&file.path)?;
            // Encode in small batches, locking per batch, so the watcher and
            // semantic_search never wait long and memory stays bounded.
            for chunk in filtered.chunks(EMBED_BATCH_SIZE) {
                let str_refs: Vec<&str> = chunk.iter().map(|(_, c)| c.text.as_str()).collect();
                let vectors = {
                    let mut emb = match embedder.lock() {
                        Ok(g) => g,
                        Err(e) => return Err(format!("embedder lock poisoned: {e}")),
                    };
                    emb.encode(&str_refs)
                };
                match vectors {
                    Ok(vectors) => {
                        for ((sym, c), vec) in chunk.iter().zip(vectors.iter()) {
                            // Match parsed symbols to DB rows by identity, not position.
                            let row = db_symbols.iter().find(|r| {
                                r.name == sym.name
                                    && r.kind == sym.kind
                                    && r.start_line == sym.start_line
                            });
                            if let Some(row) = row {
                                if db
                                    .upsert_embedding(row.id, c.chunk_index, c.start_line, c.end_line, vec)
                                    .is_ok()
                                {
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
                    workspace: workspace.to_string(),
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

    // Doc files use the doc-specific indexer instead of tree-sitter parsing
    if path.ends_with(".md") || path.ends_with(".mdx") || path.ends_with(".txt") {
        let _ = index_doc_file(db, path, &content, embedder);
        return Ok(Some(ParseResult {
            language: "markdown".into(),
            symbols: vec![],
            calls: vec![],
            error: None,
        }));
    }

    let result = index_file(db, path, &content, embedder).ok();
    Ok(result)
}

#[cfg(test)]
mod should_embed_symbol_tests {
    use super::should_embed_symbol;

    #[test]
    fn excludes_non_retrievable_kinds() {
        let long_text = "a".repeat(200);
        assert!(!should_embed_symbol("import", "SomeModule", &long_text));
        assert!(!should_embed_symbol("property_signature", "onClick", &long_text));
        assert!(!should_embed_symbol("field_declaration", "counter", &long_text));
        assert!(!should_embed_symbol("property_declaration", "counter", &long_text));
    }

    #[test]
    fn excludes_short_embedding_text() {
        assert!(!should_embed_symbol("function_item", "compute_hash", "short text"));
        assert!(should_embed_symbol(
            "function_item",
            "compute_hash",
            "function_item compute_hash: computes a hash of the given content for caching"
        ));
    }

    #[test]
    fn excludes_generic_names_without_substantial_text() {
        let short_text = "property_signature props: react component props";
        assert!(short_text.len() >= 40 && short_text.len() < 120);
        assert!(!should_embed_symbol("variable_declaration", "props", short_text));
        assert!(!should_embed_symbol("variable_declaration", "ID", short_text)); // case-insensitive

        let long_text = "a".repeat(150);
        assert!(should_embed_symbol("variable_declaration", "props", &long_text));
    }

    #[test]
    fn keeps_meaningful_symbols() {
        let text = "function_item authenticate_user: verifies credentials and issues a session token";
        assert!(should_embed_symbol("function_item", "authenticate_user", text));
    }
}
