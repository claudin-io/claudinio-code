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

/// Flat i18n key -> user-visible copy, merged across locale files.
pub type I18nDict = std::collections::HashMap<String, String>;

/// Whether a file looks like a localization resource, across ecosystems:
/// web (locales/i18n/l10n/translations/lang dirs with ts/js/json), Flutter
/// (.arb), iOS (.strings), Android (res/values*/strings.xml).
fn is_locale_resource(path_lower: &str) -> bool {
    if path_lower.ends_with(".arb") || path_lower.ends_with(".strings") {
        return true;
    }
    if path_lower.ends_with("strings.xml") && path_lower.contains("/values") {
        return true;
    }
    let in_locale_dir = ["/locales/", "/locale/", "/i18n/", "/l10n/", "/translations/", "/lang/"]
        .iter()
        .any(|seg| path_lower.contains(seg));
    in_locale_dir
        && (path_lower.ends_with(".ts")
            || path_lower.ends_with(".tsx")
            || path_lower.ends_with(".js")
            || path_lower.ends_with(".json"))
}

/// Flatten nested JSON (i18next-style) into dotted keys.
fn flatten_json_into(prefix: &str, value: &serde_json::Value, dict: &mut I18nDict) {
    match value {
        serde_json::Value::String(s) => {
            if !prefix.is_empty() && !s.trim().is_empty() {
                dict.insert(prefix.to_string(), s.clone());
            }
        }
        serde_json::Value::Object(map) => {
            for (k, v) in map {
                let key = if prefix.is_empty() { k.clone() } else { format!("{prefix}.{k}") };
                flatten_json_into(&key, v, dict);
            }
        }
        _ => {}
    }
}

/// Collect translation strings from locale resource files. Components
/// reference copy through keys like `t("onboarding.features.agent.title")`,
/// so without this the user-visible vocabulary never reaches their embeddings
/// and NL queries about visible text can't find the component.
///
/// English files are merged last so their values win — the embedding model
/// is English-centric — but keys that only exist in other locales still land.
pub fn load_i18n_dict(root: &str) -> I18nDict {
    let mut locale_files: Vec<String> = Vec::new();
    let walker = ignore::WalkBuilder::new(root).git_ignore(true).build();
    for entry in walker.flatten() {
        let path = entry.path();
        if !path.is_file() {
            continue;
        }
        let Some(path_str) = path.to_str() else { continue };
        if is_locale_resource(&path_str.to_lowercase()) {
            locale_files.push(path_str.to_string());
        }
    }
    // Non-English first, English last (its inserts overwrite). English markers
    // cover "en-US.ts", "en.json", "app_en.arb", iOS "en.lproj" and Android's
    // default "values/strings.xml".
    locale_files.sort_by_key(|p| {
        let lower = p.to_lowercase();
        let base = lower.rsplit('/').next().unwrap_or(&lower).to_string();
        base.starts_with("en")
            || base.contains("_en.")
            || base.contains("-en.")
            || lower.contains("/en.lproj/")
            || lower.ends_with("/values/strings.xml")
    });

    // `"key": "value"` (TS/JS dicts) and `"key" = "value";` (iOS .strings).
    let kv_re = regex::Regex::new(r#""([A-Za-z0-9_.\-]+)"\s*[:=]\s*"((?:[^"\\]|\\.)*)""#)
        .expect("static regex");
    // Android `<string name="key">value</string>`.
    let xml_re = regex::Regex::new(r#"<string\s+name="([A-Za-z0-9_.\-]+)"[^>]*>([^<]*)</string>"#)
        .expect("static regex");

    let mut dict = I18nDict::new();
    for file in locale_files {
        let Ok(content) = std::fs::read_to_string(&file) else { continue };
        let lower = file.to_lowercase();
        if lower.ends_with(".json") || lower.ends_with(".arb") {
            if let Ok(parsed) = serde_json::from_str::<serde_json::Value>(&content) {
                flatten_json_into("", &parsed, &mut dict);
                continue;
            }
        }
        if lower.ends_with(".xml") {
            for cap in xml_re.captures_iter(&content) {
                let value = cap[2].trim();
                if !value.is_empty() {
                    dict.insert(cap[1].to_string(), value.to_string());
                }
            }
            continue;
        }
        for cap in kv_re.captures_iter(&content) {
            let key = cap[1].to_string();
            let value = cap[2].replace("\\\"", "\"").replace("\\n", " ");
            if !value.trim().is_empty() {
                dict.insert(key, value);
            }
        }
    }
    dict
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
    i18n: Option<&I18nDict>,
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
        let chunks = collect_embedding_chunks(&parse_result.symbols, &symbol_ids, i18n);
        encode_and_store_batched(db, emb, &chunks);
        let _ = db.set_embed_hash(file_id, &hash);
    }

    Ok(parse_result)
}

/// Gate symbols through `should_embed_symbol`, then split survivors into
/// line-based chunks paired with their DB symbol id. Language-agnostic: it
/// only looks at the extracted body text, never at syntax.
fn collect_embedding_chunks(
    symbols: &[parser::ParsedSymbol],
    symbol_ids: &[(String, i64)],
    i18n: Option<&I18nDict>,
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
            i18n,
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
    i18n: Option<&I18nDict>,
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
        let chunks = collect_embedding_chunks(&doc_symbols, &symbol_ids, i18n);
        encode_and_store_batched(db, emb, &chunks);
        let _ = db.set_embed_hash(file_id, &hash);
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
    shared_progress: Option<&std::sync::Mutex<Option<IndexProgress>>>,
) -> Result<(i64, i64), String> {
    let mut total_files = 0i64;
    let mut total_symbols = 0i64;
    let mut counted = 0i64;

    // Resolved once per scan; empty when the project has no locale resources,
    // in which case chunk texts are unchanged.
    let i18n_dict = load_i18n_dict(root);
    let i18n = if i18n_dict.is_empty() { None } else { Some(&i18n_dict) };

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
        let _ = ch.send(initial_progress.clone());
    }
    if let Some(sp) = shared_progress {
        let _ = sp.lock().map(|mut guard| *guard = Some(initial_progress));
    }

    for path_str in &all_paths {
        let content = match std::fs::read_to_string(path_str) {
            Ok(c) => c,
            Err(_) => continue,
        };

        // Skip files whose content hasn't changed since the last scan —
        // reparsing/re-embedding them here would be pure waste, and worse,
        // deletes their existing symbols/embeddings via cascade for nothing.
        let new_hash = compute_hash(&content);
        if let Ok(Some(existing)) = db.file_by_path(path_str) {
            if existing.hash.as_deref() == Some(new_hash.as_str()) {
                total_files += 1;
                total_symbols += db.symbols_in_file(path_str).map(|s| s.len() as i64).unwrap_or(0);
                counted += 1;
                continue;
            }
        }

        match index_file(db, path_str, &content, embedder.as_deref_mut(), i18n) {
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
                let _ = ch.send(prog.clone());
            }
            if let Some(sp) = shared_progress {
                let _ = sp.lock().map(|mut guard| *guard = Some(prog));
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
        let _ = handle.emit("index-progress", grand_progress.clone());
    }
    if let Some(sp) = shared_progress {
        let _ = sp.lock().map(|mut guard| *guard = Some(grand_progress));
    }

    for path_str in &doc_paths {
        let content = match std::fs::read_to_string(path_str) {
            Ok(c) => c,
            Err(_) => continue,
        };

        match index_doc_file(db, path_str, &content, embedder.as_deref_mut(), i18n) {
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
        let _ = ch.send(done_progress.clone());
    }
    if let Some(sp) = shared_progress {
        let _ = sp.lock().map(|mut guard| *guard = Some(done_progress));
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
    let i18n_dict = load_i18n_dict(workspace);
    let i18n = if i18n_dict.is_empty() { None } else { Some(&i18n_dict) };

    for file in &files {
        processed += 1;

        // Already embedded from this exact content — nothing to do. This is
        // what keeps re-opening a workspace from re-embedding every symbol
        // in it every time.
        if file.hash.is_some() && file.embed_hash == file.hash {
            continue;
        }

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
                    i18n,
                )
                .into_iter()
                .map(move |c| (sym, c))
            })
            .collect();

        let mut encode_failed = false;
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
                        encode_failed = true;
                        eprintln!("[embeddings] encode failed for {}: {e}", file.path);
                        break;
                    }
                }
                // Breathe between batches so the UI process isn't starved on
                // low-core machines during a long initial index.
                std::thread::sleep(std::time::Duration::from_millis(30));
            }
        }

        // Only mark this file's content as "embedded" if nothing failed —
        // otherwise a transient encode error would permanently skip it.
        if !encode_failed {
            let hash = compute_hash(&content);
            let _ = db.set_embed_hash(file.id, &hash);
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

pub fn reindex_file(
    db: &IndexDb,
    path: &str,
    embedder: Option<&mut CodeEmbedder>,
    workspace_root: Option<&str>,
) -> Result<Option<ParseResult>, String> {
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

    // Cheap per-event reload (locale resources are few and small), so copy
    // edits are reflected without waiting for a full rescan.
    let i18n_dict = workspace_root.map(load_i18n_dict).unwrap_or_default();
    let i18n = if i18n_dict.is_empty() { None } else { Some(&i18n_dict) };

    // Doc files use the doc-specific indexer instead of tree-sitter parsing
    if path.ends_with(".md") || path.ends_with(".mdx") || path.ends_with(".txt") {
        let _ = index_doc_file(db, path, &content, embedder, i18n);
        return Ok(Some(ParseResult {
            language: "markdown".into(),
            symbols: vec![],
            calls: vec![],
            error: None,
        }));
    }

    let result = index_file(db, path, &content, embedder, i18n).ok();
    Ok(result)
}

#[cfg(test)]
mod i18n_dict_tests {
    use super::*;

    #[test]
    fn locale_resource_detection_across_ecosystems() {
        assert!(is_locale_resource("src/lib/locales/en-us.ts"));
        assert!(is_locale_resource("public/i18n/pt-br.json"));
        assert!(is_locale_resource("lib/l10n/app_en.arb"));
        assert!(is_locale_resource("ios/en.lproj/localizable.strings"));
        assert!(is_locale_resource("android/app/src/main/res/values/strings.xml"));
        assert!(is_locale_resource("android/app/src/main/res/values-pt/strings.xml"));
        assert!(!is_locale_resource("src/components/chatpanel.tsx"));
        assert!(!is_locale_resource("res/layout/strings.xml.bak"));
        assert!(!is_locale_resource("src/lib/locales/readme.md"));
    }

    #[test]
    fn flatten_nested_json_to_dotted_keys() {
        let v: serde_json::Value =
            serde_json::from_str(r#"{"chat":{"input":{"placeholder":"Type here"}},"title":"App"}"#)
                .unwrap();
        let mut dict = I18nDict::new();
        flatten_json_into("", &v, &mut dict);
        assert_eq!(dict.get("chat.input.placeholder").map(String::as_str), Some("Type here"));
        assert_eq!(dict.get("title").map(String::as_str), Some("App"));
    }
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
