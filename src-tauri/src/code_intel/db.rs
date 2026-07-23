use rusqlite::{Connection, params};
use serde::Serialize;
use std::path::Path;
use std::sync::Mutex;

pub struct IndexDb {
    pub conn: Mutex<Connection>,
}

#[derive(Debug, Clone, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct FileRecord {
    pub id: i64,
    pub path: String,
    pub language: Option<String>,
    pub hash: Option<String>,
    pub last_modified: i64,
    pub size: i64,
    /// Content hash the symbol_embeddings for this file were last generated
    /// from. Compared against `hash` to skip re-embedding unchanged files.
    pub embed_hash: Option<String>,
}

#[derive(Debug, Clone, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct SymbolRecord {
    pub id: i64,
    pub file_id: i64,
    pub name: String,
    pub kind: String,
    pub signature: Option<String>,
    pub start_line: i64,
    pub start_col: i64,
    pub end_line: i64,
    pub end_col: i64,
    pub file_path: Option<String>,
}

/// One embedded chunk of a symbol as stored: the symbol, the chunk's start and
/// end line, and the vector. Both line numbers are 0 for whole-symbol
/// embeddings (headers, small bodies).
pub type EmbeddingRow = (SymbolRecord, i64, i64, Vec<f32>);

#[derive(Debug, Clone, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct SearchResult {
    pub symbol_id: i64,
    pub name: String,
    pub kind: String,
    pub file_path: String,
    pub start_line: i64,
    pub signature: Option<String>,
}

#[derive(Debug, Clone, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct SemanticSearchResult {
    pub symbol_id: i64,
    pub name: String,
    pub kind: String,
    pub file_path: String,
    pub start_line: i64,
    pub end_line: i64,
    pub signature: Option<String>,
    pub score: f32,
    /// Which retrieval evidence produced this hit: "hybrid" (both legs),
    /// "semantic" (vector only), or "lexical" (BM25 only).
    pub match_type: String,
    /// Source excerpt of the symbol, filled in by the tool layer for top hits.
    pub snippet: Option<String>,
}

/// One persisted retrieval chunk, as fed to the embedding pass.
#[derive(Debug, Clone)]
pub struct StoredChunk {
    pub symbol_id: i64,
    pub chunk_index: i64,
    pub start_line: i64,
    pub end_line: i64,
    pub embed_text: String,
}

/// Bump when the index format changes (schema, embedding layout, ignore
/// rules). A mismatched on-disk index is deleted and rebuilt from scratch.
const SCHEMA_VERSION: i64 = 7;

/// Tuning knobs for `search_hybrid_with`. `Default` holds the production
/// values, calibrated with `examples/semantic_eval.rs --sweep` — re-run the
/// sweep before changing any of them by hand.
#[derive(Debug, Clone)]
pub struct HybridParams {
    /// RRF smoothing constant: higher flattens the rank curve.
    pub rrf_k: f32,
    /// Leg weights in the fused score.
    pub w_vector: f32,
    pub w_bm25: f32,
    /// Candidates kept per leg (best chunk per symbol) before fusion.
    pub k_candidates: usize,
    /// Entry gate for the vector leg: chunks below this raw cosine never
    /// become candidates. Off-topic queries score ~0.36-0.44 against random
    /// code on MiniLM-class models — do not lower this; BM25 rescues the
    /// relevant hits that sit in that band (sweep 2026-07-20: 0.40 kept
    /// top-3/top-15 intact while halving negative-query leaks vs 0.35).
    pub min_cosine_candidate: f32,
    /// Final gate on the fused+boosted score, in the same [0, 1] range the
    /// old cosine threshold used.
    pub min_hybrid_score: f32,
    /// Floor of distinct query tokens a BM25-only hit must contain (see
    /// `required_token_matches`).
    pub min_bm25_term_matches: usize,
}

impl Default for HybridParams {
    // Calibrated with `semantic_eval --sweep` on 2026-07-20 (59 positives /
    // 5 negatives from real sessions): top-1 67%, top-3 91%, top-15 98%,
    // exact-identifier/basename/body-term 100% top-1, negatives 3/5 empty
    // (the two leaks are genuine repo-vocabulary collisions, scored low).
    fn default() -> Self {
        HybridParams {
            rrf_k: 60.0,
            w_vector: 1.0,
            w_bm25: 1.0,
            k_candidates: 50,
            min_cosine_candidate: 0.40,
            min_hybrid_score: 0.35,
            min_bm25_term_matches: 2,
        }
    }
}

/// Doc sections (markdown headings) embed dense natural language, so they
/// consistently out-score code symbols on NL queries; this penalty keeps them
/// in the results without letting them crowd out the code the agent needs.
const DOC_SECTION_PENALTY: f32 = 0.12;

/// Applied to results living in test files (see `is_test_file`).
const TEST_FILE_PENALTY: f32 = 0.10;

/// Max results kept per source file before `limit` is applied, so a single
/// large file (many symbols) can't dominate the whole ranking.
const MAX_RESULTS_PER_FILE: usize = 3;

/// Number of embedding rows loaded per page in the paginated semantic scan.
const EMBEDDING_PAGE_SIZE: i64 = 2000;

const STOPWORDS: &[&str] = &["the", "and", "for", "with"];

/// Lowercase, alphanumeric tokens of at least 3 chars, minus trivial stopwords.
fn tokenize_query(query_text: &str) -> Vec<String> {
    query_text
        .split(|c: char| !c.is_alphanumeric())
        .filter(|t| !t.is_empty())
        .map(|t| t.to_lowercase())
        .filter(|t| t.len() >= 3 && !STOPWORDS.contains(&t.as_str()))
        .collect()
}

/// Max quoted terms in a MATCH expression — queries are short NL phrases;
/// beyond this, OR-recall only adds noise and cost.
const MAX_FTS_TERMS: usize = 12;

/// Standalone words dropped from FTS queries. Identifier runs (containing
/// `_`/`-`/`.`) are never filtered — "for" inside delete_symbols_for_file
/// stays part of the phrase.
const FTS_STOPWORDS: &[&str] = &[
    "the", "and", "for", "with", "this", "that", "does", "how", "what", "where", "when", "which",
    "code", "file",
];

/// BM25 column weights for chunk_fts (fts_name, fts_path, fts_body): a term
/// hit on the symbol name outweighs one on the path, which outweighs one in
/// the body.
const BM25_W_NAME: f64 = 3.0;
const BM25_W_PATH: f64 = 2.0;
const BM25_W_BODY: f64 = 1.0;

/// Build a safe FTS5 MATCH expression from free text. Runs of
/// `[A-Za-z0-9_.-]` are extracted, stopwords and 1-char runs dropped, each
/// run double-quoted (internal quotes doubled) so FTS5 operators (AND, OR,
/// NOT, NEAR, `*`, `^`, `-`, `:`) can never be injected, then joined with OR
/// for recall — BM25 ranking rewards multi-term hits, so OR does not flood
/// the top ranks. Quoted runs containing `_`/`-`/`.` tokenize into phrases,
/// which is what makes "delete_symbols_for_file" an exact adjacency match.
/// Returns None when nothing usable survives.
fn build_fts_match_query(query_text: &str) -> Option<String> {
    let mut terms: Vec<String> = Vec::new();
    for raw in query_text.split(|c: char| !(c.is_alphanumeric() || "_-.".contains(c))) {
        let run = raw.trim_matches(|c: char| "_-.".contains(c));
        if run.chars().count() < 2 {
            continue;
        }
        let is_identifier = run.contains('_') || run.contains('-') || run.contains('.');
        if !is_identifier && FTS_STOPWORDS.contains(&run.to_lowercase().as_str()) {
            continue;
        }
        let quoted = format!("\"{}\"", run.replace('"', "\"\""));
        if !terms.contains(&quoted) {
            terms.push(quoted);
        }
        if terms.len() >= MAX_FTS_TERMS {
            break;
        }
    }
    if terms.is_empty() {
        None
    } else {
        Some(terms.join(" OR "))
    }
}

/// Distinct query tokens a BM25-only hit must contain to survive. Floored at
/// `min_matches` (capped by the token count, so single-token exact-term
/// queries work) and raised to a majority for long NL queries — one
/// incidental common word can't drag junk in via OR-recall.
fn required_token_matches(n_tokens: usize, min_matches: usize) -> usize {
    min_matches.min(n_tokens).max(n_tokens.div_ceil(2)).max(1)
}

/// One candidate from a retrieval leg, reduced to the best chunk per symbol.
struct LegHit {
    symbol_id: i64,
    name: String,
    kind: String,
    signature: Option<String>,
    file_path: String,
    sym_start_line: i64,
    sym_end_line: i64,
    chunk_start: i64,
    chunk_end: i64,
    /// Raw cosine (vector leg only; 0.0 for BM25 hits) — ordering only.
    cosine: f32,
    /// Concatenated fts_* text (BM25 leg only) for the evidence gate.
    fts_text: String,
}

/// True when a hit found only by BM25 has enough lexical evidence to stand
/// without vector support: an exact name/basename/stem token, or at least
/// `required` distinct query tokens present as whole words in its FTS text.
fn bm25_only_hit_has_evidence(tokens: &[String], hit: &LegHit, required: usize) -> bool {
    let (base, stem) = basename_variants(&hit.file_path);
    let name_lower = hit.name.to_lowercase();
    if tokens
        .iter()
        .any(|t| *t == name_lower || *t == base || *t == stem)
    {
        return true;
    }
    let words: std::collections::HashSet<String> = hit
        .fts_text
        .split(|c: char| !c.is_alphanumeric())
        .filter(|w| !w.is_empty())
        .map(|w| w.to_lowercase())
        .collect();
    tokens.iter().filter(|t| words.contains(t.as_str())).count() >= required
}

/// Returns the file's basename, with and without its extension.
fn basename_variants(file_path: &str) -> (String, String) {
    let base = Path::new(file_path)
        .file_name()
        .map(|s| s.to_string_lossy().to_lowercase())
        .unwrap_or_default();
    let stem = Path::new(file_path)
        .file_stem()
        .map(|s| s.to_string_lossy().to_lowercase())
        .unwrap_or_else(|| base.clone());
    (base, stem)
}

/// Highest lexical boost across all query tokens for a given symbol name /
/// file path. Layers don't stack — only the best-matching layer counts.
/// A basename match outranks a symbol-name match: a query naming a file is a
/// strong navigation signal, but short symbol names collide with ordinary
/// query words ("task", "list") and shouldn't dominate the semantic score.
fn lexical_boost(tokens: &[String], name: &str, file_path: &str) -> f32 {
    let name_lower = name.to_lowercase();
    let (base, stem) = basename_variants(file_path);
    let mut boost = 0.0f32;
    for token in tokens {
        if token == &base || token == &stem {
            return 0.25;
        }
        if token == &name_lower {
            boost = boost.max(0.15);
            continue;
        }
        if name_lower.contains(token.as_str())
            || token.contains(name_lower.as_str())
            || base.contains(token.as_str())
            || token.contains(base.as_str())
            || stem.contains(token.as_str())
            || token.contains(stem.as_str())
        {
            boost = boost.max(0.10);
        }
    }
    boost
}

/// Test files answer "how is this used in tests", not "where does this live" —
/// they mirror the vocabulary of the code under test and crowd it out.
fn is_test_file(file_path: &str) -> bool {
    let lower = file_path.to_lowercase();
    let base = lower.rsplit('/').next().unwrap_or(&lower);
    base.contains(".test.")
        || base.contains(".spec.")
        || base.ends_with("_test.rs")
        || base.ends_with("_tests.rs")
        || lower.contains("/tests/")
        || lower.contains("/__tests__/")
}

/// Inline test symbols (Rust `mod tests`, `fn test_*`) live inside production
/// files, so `is_test_file` misses them — catch them by naming convention.
fn is_test_symbol(name: &str) -> bool {
    let lower = name.to_lowercase();
    lower.starts_with("test_")
        || lower.ends_with("_test")
        || lower.ends_with("_tests")
        || lower == "tests"
}

impl IndexDb {
    pub fn open(db_path: &Path) -> Result<Self, String> {
        // Ensure the parent directory exists — SQLite cannot create the file
        // when the directory it belongs to doesn't exist yet.
        if let Some(parent) = db_path.parent() {
            std::fs::create_dir_all(parent)
                .map_err(|e| format!("create db dir {}: {e}", parent.display()))?;
        }
        let mut conn = Connection::open(db_path).map_err(|e| format!("db open: {e}"))?;

        let version: i64 = conn
            .query_row("PRAGMA user_version", [], |row| row.get(0))
            .unwrap_or(0);
        let is_empty: bool = conn
            .query_row(
                "SELECT count(*) FROM sqlite_master WHERE type='table'",
                [],
                |row| row.get::<_, i64>(0),
            )
            .map(|c| c == 0)
            .unwrap_or(true);
        if !is_empty && version != SCHEMA_VERSION {
            eprintln!(
                "[index] stale index (version {version}, expected {SCHEMA_VERSION}) — rebuilding {}",
                db_path.display()
            );
            drop(conn);
            let _ = std::fs::remove_file(db_path);
            let base = db_path.display();
            let _ = std::fs::remove_file(format!("{base}-wal"));
            let _ = std::fs::remove_file(format!("{base}-shm"));
            conn = Connection::open(db_path).map_err(|e| format!("db reopen: {e}"))?;
        }

        // recursive_triggers: without it, the internal DELETE half of an
        // INSERT OR REPLACE does not fire delete triggers, which would leave
        // ghost rows in the external-content FTS tables kept in sync below.
        conn.execute_batch(&format!(
            "PRAGMA journal_mode=WAL; PRAGMA foreign_keys=ON; PRAGMA recursive_triggers=ON; PRAGMA user_version={SCHEMA_VERSION};"
        ))
        .map_err(|e| format!("pragma: {e}"))?;
        let db = IndexDb {
            conn: Mutex::new(conn),
        };
        db.init_schema()?;
        Ok(db)
    }

    fn init_schema(&self) -> Result<(), String> {
        let conn = self.conn.lock().map_err(|e| e.to_string())?;
        conn.execute_batch(
            "
            CREATE TABLE IF NOT EXISTS files (
                id INTEGER PRIMARY KEY,
                path TEXT UNIQUE NOT NULL,
                language TEXT,
                hash TEXT,
                last_modified INTEGER,
                size INTEGER,
                embed_hash TEXT
            );

            CREATE TABLE IF NOT EXISTS symbols (
                id INTEGER PRIMARY KEY,
                file_id INTEGER NOT NULL,
                name TEXT NOT NULL,
                kind TEXT NOT NULL DEFAULT 'unknown',
                signature TEXT,
                start_line INTEGER,
                start_col INTEGER,
                end_line INTEGER,
                end_col INTEGER,
                doc_comment TEXT,
                FOREIGN KEY(file_id) REFERENCES files(id) ON DELETE CASCADE
            );

            CREATE TABLE IF NOT EXISTS relations (
                id INTEGER PRIMARY KEY,
                from_symbol_id INTEGER NOT NULL,
                to_symbol_id INTEGER NOT NULL,
                kind TEXT NOT NULL DEFAULT 'calls',
                FOREIGN KEY(from_symbol_id) REFERENCES symbols(id) ON DELETE CASCADE,
                FOREIGN KEY(to_symbol_id) REFERENCES symbols(id) ON DELETE CASCADE
            );

            CREATE INDEX IF NOT EXISTS idx_symbols_name ON symbols(name);
            CREATE INDEX IF NOT EXISTS idx_symbols_file_id ON symbols(file_id);
            CREATE INDEX IF NOT EXISTS idx_relations_from ON relations(from_symbol_id);
            CREATE INDEX IF NOT EXISTS idx_relations_to ON relations(to_symbol_id);

            CREATE TABLE IF NOT EXISTS symbol_embeddings (
                symbol_id INTEGER NOT NULL,
                chunk_index INTEGER NOT NULL DEFAULT 0,
                start_line INTEGER NOT NULL DEFAULT 0,
                end_line INTEGER NOT NULL DEFAULT 0,
                embedding BLOB NOT NULL,
                PRIMARY KEY(symbol_id, chunk_index),
                FOREIGN KEY(symbol_id) REFERENCES symbols(id) ON DELETE CASCADE
            );

            CREATE TABLE IF NOT EXISTS symbol_chunks (
                id INTEGER PRIMARY KEY,
                symbol_id INTEGER NOT NULL,
                chunk_index INTEGER NOT NULL,
                start_line INTEGER NOT NULL DEFAULT 0,
                end_line INTEGER NOT NULL DEFAULT 0,
                embed_text TEXT NOT NULL,
                fts_name TEXT NOT NULL DEFAULT '',
                fts_path TEXT NOT NULL DEFAULT '',
                fts_body TEXT NOT NULL DEFAULT '',
                UNIQUE(symbol_id, chunk_index),
                FOREIGN KEY(symbol_id) REFERENCES symbols(id) ON DELETE CASCADE
            );
            CREATE INDEX IF NOT EXISTS idx_symbol_chunks_symbol ON symbol_chunks(symbol_id);
            ",
        )
        .map_err(|e| format!("schema: {e}"))?;

        // symbols_fts is the complete name/signature directory (covers
        // non-embeddable symbols like imports for code_search); chunk_fts
        // covers exactly the retrieval units embeddings cover, enriched with
        // path and split-identifier words, and backs the BM25 leg of hybrid
        // search. unicode61 note: `_`/`-`/`.` are separators, so quoted
        // snake/kebab/dotted terms become adjacency-preserving phrases at
        // query time; camelCase stays one token, which is why split words are
        // stored explicitly in fts_name/fts_body.
        conn.execute_batch(
            "CREATE VIRTUAL TABLE IF NOT EXISTS symbols_fts USING fts5(
                name, signature,
                content='symbols',
                content_rowid='id'
            );
            CREATE VIRTUAL TABLE IF NOT EXISTS chunk_fts USING fts5(
                fts_name, fts_path, fts_body,
                content='symbol_chunks',
                content_rowid='id',
                tokenize='unicode61 remove_diacritics 2'
            );",
        )
        .map_err(|e| format!("fts5: {e}"))?;

        // Triggers keep the external-content FTS tables in lockstep with
        // their content tables, so no Rust code path can forget the FTS side
        // (the v6 index leaked stale symbols_fts rows on incremental
        // reindex). Deletes must be explicit statements — cascades are not
        // relied on (see delete_symbols_for_file / delete_file).
        conn.execute_batch(
            "CREATE TRIGGER IF NOT EXISTS symbols_ai AFTER INSERT ON symbols BEGIN
                INSERT INTO symbols_fts(rowid, name, signature)
                VALUES (new.id, new.name, new.signature);
            END;
            CREATE TRIGGER IF NOT EXISTS symbols_ad AFTER DELETE ON symbols BEGIN
                INSERT INTO symbols_fts(symbols_fts, rowid, name, signature)
                VALUES ('delete', old.id, old.name, old.signature);
            END;
            CREATE TRIGGER IF NOT EXISTS symbol_chunks_ai AFTER INSERT ON symbol_chunks BEGIN
                INSERT INTO chunk_fts(rowid, fts_name, fts_path, fts_body)
                VALUES (new.id, new.fts_name, new.fts_path, new.fts_body);
            END;
            CREATE TRIGGER IF NOT EXISTS symbol_chunks_ad AFTER DELETE ON symbol_chunks BEGIN
                INSERT INTO chunk_fts(chunk_fts, rowid, fts_name, fts_path, fts_body)
                VALUES ('delete', old.id, old.fts_name, old.fts_path, old.fts_body);
            END;",
        )
        .map_err(|e| format!("fts triggers: {e}"))?;

        Ok(())
    }

    pub fn upsert_file(
        &self,
        path: &str,
        language: &str,
        hash: &str,
        modified: i64,
        size: i64,
    ) -> Result<i64, String> {
        let conn = self.conn.lock().map_err(|e| e.to_string())?;
        conn.execute(
            "INSERT INTO files (path, language, hash, last_modified, size)
             VALUES (?1, ?2, ?3, ?4, ?5)
             ON CONFLICT(path) DO UPDATE SET
               language=excluded.language,
               hash=excluded.hash,
               last_modified=excluded.last_modified,
               size=excluded.size",
            params![path, language, hash, modified, size],
        )
        .map_err(|e| format!("upsert file: {e}"))?;

        let id: i64 = conn
            .query_row(
                "SELECT id FROM files WHERE path = ?1",
                params![path],
                |row| row.get(0),
            )
            .map_err(|e| format!("get file id: {e}"))?;
        Ok(id)
    }

    pub fn all_files(&self) -> Result<Vec<FileRecord>, String> {
        let conn = self.conn.lock().map_err(|e| e.to_string())?;
        let mut stmt = conn
            .prepare("SELECT id, path, language, hash, last_modified, size, embed_hash FROM files ORDER BY id")
            .map_err(|e| format!("prepare: {e}"))?;
        let results = stmt
            .query_map([], |row| {
                Ok(FileRecord {
                    id: row.get(0)?,
                    path: row.get(1)?,
                    language: row.get(2)?,
                    hash: row.get(3)?,
                    last_modified: row.get(4)?,
                    size: row.get(5)?,
                    embed_hash: row.get(6)?,
                })
            })
            .map_err(|e| format!("query: {e}"))?
            .filter_map(|r| r.ok())
            .collect();
        Ok(results)
    }

    /// Records that `symbol_embeddings` for this file now reflect `hash`, so a
    /// future scan can skip re-embedding it while the content stays the same.
    pub fn set_embed_hash(&self, file_id: i64, hash: &str) -> Result<(), String> {
        let conn = self.conn.lock().map_err(|e| e.to_string())?;
        conn.execute(
            "UPDATE files SET embed_hash = ?1 WHERE id = ?2",
            params![hash, file_id],
        )
        .map_err(|e| format!("set embed_hash: {e}"))?;
        Ok(())
    }

    /// Remove file rows (and, via cascade, their symbols/relations/embeddings)
    /// whose path is not in the current scan set — e.g. node_modules leftovers
    /// indexed before ignore rules existed.
    pub fn prune_files_not_in(
        &self,
        keep: &std::collections::HashSet<String>,
    ) -> Result<i64, String> {
        let conn = self.conn.lock().map_err(|e| e.to_string())?;
        let stale_ids: Vec<i64> = {
            let mut stmt = conn
                .prepare("SELECT id, path FROM files")
                .map_err(|e| format!("prepare: {e}"))?;
            let ids: Vec<i64> = stmt
                .query_map([], |row| {
                    Ok((row.get::<_, i64>(0)?, row.get::<_, String>(1)?))
                })
                .map_err(|e| format!("query: {e}"))?
                .filter_map(|r| r.ok())
                .filter(|(_, path)| !keep.contains(path))
                .map(|(id, _)| id)
                .collect();
            ids
        };
        for id in &stale_ids {
            conn.execute("DELETE FROM files WHERE id = ?1", params![id])
                .map_err(|e| format!("prune file: {e}"))?;
        }
        if !stale_ids.is_empty() {
            // These prune deletes cascade files -> symbols -> chunks, and
            // cascades don't reliably fire the FTS triggers — rebuild both
            // external-content indexes from their content tables (self-heal).
            let _ = conn.execute("INSERT INTO symbols_fts(symbols_fts) VALUES('rebuild')", []);
            let _ = conn.execute("INSERT INTO chunk_fts(chunk_fts) VALUES('rebuild')", []);
        }
        Ok(stale_ids.len() as i64)
    }

    pub fn file_by_path(&self, path: &str) -> Result<Option<FileRecord>, String> {
        let conn = self.conn.lock().map_err(|e| e.to_string())?;
        let mut stmt = conn
            .prepare("SELECT id, path, language, hash, last_modified, size, embed_hash FROM files WHERE path = ?1")
            .map_err(|e| format!("prepare: {e}"))?;
        let result = stmt
            .query_row(params![path], |row| {
                Ok(FileRecord {
                    id: row.get(0)?,
                    path: row.get(1)?,
                    language: row.get(2)?,
                    hash: row.get(3)?,
                    last_modified: row.get(4)?,
                    size: row.get(5)?,
                    embed_hash: row.get(6)?,
                })
            })
            .ok();
        Ok(result)
    }

    pub fn delete_symbols_for_file(&self, file_id: i64) -> Result<(), String> {
        let conn = self.conn.lock().map_err(|e| e.to_string())?;
        conn.execute("DELETE FROM relations WHERE from_symbol_id IN (SELECT id FROM symbols WHERE file_id = ?1) OR to_symbol_id IN (SELECT id FROM symbols WHERE file_id = ?1)", params![file_id])
            .map_err(|e| format!("delete relations: {e}"))?;
        conn.execute("DELETE FROM symbol_embeddings WHERE symbol_id IN (SELECT id FROM symbols WHERE file_id = ?1)", params![file_id])
            .map_err(|e| format!("delete embeddings: {e}"))?;
        // Explicit deletes (not cascades) so the AFTER DELETE triggers fire
        // and purge the external-content FTS rows deterministically.
        conn.execute("DELETE FROM symbol_chunks WHERE symbol_id IN (SELECT id FROM symbols WHERE file_id = ?1)", params![file_id])
            .map_err(|e| format!("delete chunks: {e}"))?;
        conn.execute("DELETE FROM symbols WHERE file_id = ?1", params![file_id])
            .map_err(|e| format!("delete symbols: {e}"))?;
        Ok(())
    }

    /// Remove a file and everything derived from it. Symbols/chunks are
    /// deleted explicitly first so FTS triggers fire — the FK cascade from
    /// `files` is only a safety net, not the mechanism.
    pub fn delete_file(&self, path: &str) -> Result<(), String> {
        if let Some(file) = self.file_by_path(path)? {
            self.delete_symbols_for_file(file.id)?;
        }
        let conn = self.conn.lock().map_err(|e| e.to_string())?;
        conn.execute("DELETE FROM files WHERE path = ?1", params![path])
            .map_err(|e| format!("delete file: {e}"))?;
        Ok(())
    }

    #[allow(clippy::too_many_arguments)]
    pub fn insert_symbol(
        &self,
        file_id: i64,
        name: &str,
        kind: &str,
        signature: Option<&str>,
        start_line: i64,
        start_col: i64,
        end_line: i64,
        end_col: i64,
        doc_comment: Option<&str>,
    ) -> Result<i64, String> {
        let conn = self.conn.lock().map_err(|e| e.to_string())?;
        conn.execute(
            "INSERT INTO symbols (file_id, name, kind, signature, start_line, start_col, end_line, end_col, doc_comment)
             VALUES (?1, ?2, ?3, ?4, ?5, ?6, ?7, ?8, ?9)",
            params![file_id, name, kind, signature, start_line, start_col, end_line, end_col, doc_comment],
        )
        .map_err(|e| format!("insert symbol: {e}"))?;
        let id: i64 = conn.last_insert_rowid();
        Ok(id)
    }

    pub fn insert_relation(&self, from_id: i64, to_id: i64, kind: &str) -> Result<(), String> {
        let conn = self.conn.lock().map_err(|e| e.to_string())?;
        conn.execute(
            "INSERT OR IGNORE INTO relations (from_symbol_id, to_symbol_id, kind) VALUES (?1, ?2, ?3)",
            params![from_id, to_id, kind],
        )
        .map_err(|e| format!("insert relation: {e}"))?;
        Ok(())
    }

    pub fn search_symbols(&self, query: &str, limit: i64) -> Result<Vec<SearchResult>, String> {
        // Raw agent/user text goes through the sanitizing builder — FTS5
        // operator characters in a query used to be a syntax error here.
        let Some(match_query) = build_fts_match_query(query) else {
            return Ok(vec![]);
        };
        let conn = self.conn.lock().map_err(|e| e.to_string())?;
        let mut stmt = conn
            .prepare(
                "SELECT s.id, s.name, s.kind, f.path, s.start_line, s.signature
                 FROM symbols_fts
                 JOIN symbols s ON s.id = symbols_fts.rowid
                 JOIN files f ON f.id = s.file_id
                 WHERE symbols_fts MATCH ?1
                 ORDER BY rank
                 LIMIT ?2",
            )
            .map_err(|e| format!("prepare search: {e}"))?;
        let results = stmt
            .query_map(params![match_query, limit], |row| {
                Ok(SearchResult {
                    symbol_id: row.get(0)?,
                    name: row.get(1)?,
                    kind: row.get(2)?,
                    file_path: row.get(3)?,
                    start_line: row.get(4)?,
                    signature: row.get(5)?,
                })
            })
            .map_err(|e| format!("query search: {e}"))?
            .filter_map(|r| r.ok())
            .collect();
        Ok(results)
    }

    /// Exact (case-insensitive) symbol-name lookup — the contract
    /// symbol_lookup advertises, distinct from tokenized FTS matching.
    pub fn lookup_symbols_exact(
        &self,
        name: &str,
        limit: i64,
    ) -> Result<Vec<SearchResult>, String> {
        let conn = self.conn.lock().map_err(|e| e.to_string())?;
        let mut stmt = conn
            .prepare(
                "SELECT s.id, s.name, s.kind, f.path, s.start_line, s.signature
                 FROM symbols s
                 JOIN files f ON f.id = s.file_id
                 WHERE s.name = ?1 COLLATE NOCASE
                 ORDER BY f.path, s.start_line
                 LIMIT ?2",
            )
            .map_err(|e| format!("prepare lookup: {e}"))?;
        let results = stmt
            .query_map(params![name, limit], |row| {
                Ok(SearchResult {
                    symbol_id: row.get(0)?,
                    name: row.get(1)?,
                    kind: row.get(2)?,
                    file_path: row.get(3)?,
                    start_line: row.get(4)?,
                    signature: row.get(5)?,
                })
            })
            .map_err(|e| format!("query lookup: {e}"))?
            .filter_map(|r| r.ok())
            .collect();
        Ok(results)
    }

    pub fn symbols_in_file(&self, file_path: &str) -> Result<Vec<SymbolRecord>, String> {
        let conn = self.conn.lock().map_err(|e| e.to_string())?;
        let mut stmt = conn
            .prepare(
                "SELECT s.id, s.file_id, s.name, s.kind, s.signature,
                        s.start_line, s.start_col, s.end_line, s.end_col, f.path
                 FROM symbols s
                 JOIN files f ON f.id = s.file_id
                 WHERE f.path = ?1
                 ORDER BY s.start_line",
            )
            .map_err(|e| format!("prepare: {e}"))?;
        let results = stmt
            .query_map(params![file_path], |row| {
                Ok(SymbolRecord {
                    id: row.get(0)?,
                    file_id: row.get(1)?,
                    name: row.get(2)?,
                    kind: row.get(3)?,
                    signature: row.get(4)?,
                    start_line: row.get(5)?,
                    start_col: row.get(6)?,
                    end_line: row.get(7)?,
                    end_col: row.get(8)?,
                    file_path: row.get(9)?,
                })
            })
            .map_err(|e| format!("query: {e}"))?
            .filter_map(|r| r.ok())
            .collect();
        Ok(results)
    }

    #[allow(dead_code)]
    pub fn callers_of(
        &self,
        symbol_name: &str,
        file_path: &str,
    ) -> Result<Vec<SymbolRecord>, String> {
        let conn = self.conn.lock().map_err(|e| e.to_string())?;
        let mut stmt = conn
            .prepare(
                "SELECT s.id, s.file_id, s.name, s.kind, s.signature,
                        s.start_line, s.start_col, s.end_line, s.end_col, f.path
                 FROM relations r
                 JOIN symbols s ON s.id = r.from_symbol_id
                 JOIN files f ON f.id = s.file_id
                 JOIN symbols ts ON ts.id = r.to_symbol_id
                 WHERE ts.name = ?1 AND f.path != ?2
                 ORDER BY f.path, s.start_line
                 LIMIT 50",
            )
            .map_err(|e| format!("prepare: {e}"))?;
        let results = stmt
            .query_map(params![symbol_name, file_path], |row| {
                Ok(SymbolRecord {
                    id: row.get(0)?,
                    file_id: row.get(1)?,
                    name: row.get(2)?,
                    kind: row.get(3)?,
                    signature: row.get(4)?,
                    start_line: row.get(5)?,
                    start_col: row.get(6)?,
                    end_line: row.get(7)?,
                    end_col: row.get(8)?,
                    file_path: row.get(9)?,
                })
            })
            .map_err(|e| format!("query: {e}"))?
            .filter_map(|r| r.ok())
            .collect();
        Ok(results)
    }

    /// Insert one retrieval chunk. Plain INSERT by design: callers always run
    /// `delete_symbols_for_file` first, and REPLACE would route the implicit
    /// delete around the FTS trigger on setups without recursive_triggers.
    #[allow(clippy::too_many_arguments)]
    pub fn insert_chunk(
        &self,
        symbol_id: i64,
        chunk_index: i64,
        start_line: i64,
        end_line: i64,
        embed_text: &str,
        fts_name: &str,
        fts_path: &str,
        fts_body: &str,
    ) -> Result<(), String> {
        let conn = self.conn.lock().map_err(|e| e.to_string())?;
        conn.execute(
            "INSERT INTO symbol_chunks (symbol_id, chunk_index, start_line, end_line, embed_text, fts_name, fts_path, fts_body)
             VALUES (?1, ?2, ?3, ?4, ?5, ?6, ?7, ?8)",
            params![symbol_id, chunk_index, start_line, end_line, embed_text, fts_name, fts_path, fts_body],
        )
        .map_err(|e| format!("insert chunk: {e}"))?;
        Ok(())
    }

    /// Stored retrieval chunks for one file, feeding the background embedding
    /// pass — no re-read or re-parse of the source file is needed.
    pub fn chunks_for_file(&self, file_id: i64) -> Result<Vec<StoredChunk>, String> {
        let conn = self.conn.lock().map_err(|e| e.to_string())?;
        let mut stmt = conn
            .prepare(
                "SELECT c.symbol_id, c.chunk_index, c.start_line, c.end_line, c.embed_text
                 FROM symbol_chunks c
                 JOIN symbols s ON s.id = c.symbol_id
                 WHERE s.file_id = ?1
                 ORDER BY c.symbol_id, c.chunk_index",
            )
            .map_err(|e| format!("prepare chunks: {e}"))?;
        let results = stmt
            .query_map(params![file_id], |row| {
                Ok(StoredChunk {
                    symbol_id: row.get(0)?,
                    chunk_index: row.get(1)?,
                    start_line: row.get(2)?,
                    end_line: row.get(3)?,
                    embed_text: row.get(4)?,
                })
            })
            .map_err(|e| format!("query chunks: {e}"))?
            .filter_map(|r| r.ok())
            .collect();
        Ok(results)
    }

    pub fn upsert_embedding(
        &self,
        symbol_id: i64,
        chunk_index: i64,
        start_line: i64,
        end_line: i64,
        embedding: &[f32],
    ) -> Result<(), String> {
        let bytes: Vec<u8> = embedding.iter().flat_map(|f| f.to_le_bytes()).collect();
        let conn = self.conn.lock().map_err(|e| e.to_string())?;
        conn.execute(
            "INSERT OR REPLACE INTO symbol_embeddings (symbol_id, chunk_index, start_line, end_line, embedding)
             VALUES (?1, ?2, ?3, ?4, ?5)",
            params![symbol_id, chunk_index, start_line, end_line, bytes],
        )
        .map_err(|e| format!("upsert embedding: {e}"))?;
        Ok(())
    }

    pub fn delete_embeddings_for_file(&self, file_id: i64) -> Result<(), String> {
        let conn = self.conn.lock().map_err(|e| e.to_string())?;
        conn.execute(
            "DELETE FROM symbol_embeddings WHERE symbol_id IN (SELECT id FROM symbols WHERE file_id = ?1)",
            params![file_id],
        )
        .map_err(|e| format!("delete embeddings: {e}"))?;
        Ok(())
    }

    /// Every embedded chunk, as stored.
    pub fn load_all_embeddings(&self) -> Result<Vec<EmbeddingRow>, String> {
        let conn = self.conn.lock().map_err(|e| e.to_string())?;
        let mut stmt = conn
            .prepare(
                "SELECT s.id, s.file_id, s.name, s.kind, s.signature,
                        s.start_line, s.start_col, s.end_line, s.end_col, f.path,
                        e.start_line, e.end_line, e.embedding
                 FROM symbols s
                 JOIN files f ON f.id = s.file_id
                 JOIN symbol_embeddings e ON e.symbol_id = s.id",
            )
            .map_err(|e| format!("prepare: {e}"))?;
        let results = stmt
            .query_map([], |row| {
                let blob: Vec<u8> = row.get(12)?;
                // Blob length defines the dimension — self-describing across model swaps.
                let embedding: Vec<f32> = blob
                    .chunks_exact(4)
                    .map(|chunk| f32::from_le_bytes(chunk.try_into().unwrap_or([0; 4])))
                    .collect();
                Ok((
                    SymbolRecord {
                        id: row.get(0)?,
                        file_id: row.get(1)?,
                        name: row.get(2)?,
                        kind: row.get(3)?,
                        signature: row.get(4)?,
                        start_line: row.get(5)?,
                        start_col: row.get(6)?,
                        end_line: row.get(7)?,
                        end_col: row.get(8)?,
                        file_path: row.get(9)?,
                    },
                    row.get::<_, i64>(10)?,
                    row.get::<_, i64>(11)?,
                    embedding,
                ))
            })
            .map_err(|e| format!("query: {e}"))?
            .filter_map(|r| r.ok())
            .collect();
        Ok(results)
    }

    /// Load a single page of embedding rows, ordered deterministically.
    pub fn load_embeddings_page(
        &self,
        page_size: i64,
        offset: i64,
    ) -> Result<Vec<EmbeddingRow>, String> {
        let conn = self.conn.lock().map_err(|e| e.to_string())?;
        let mut stmt = conn
            .prepare(
                "SELECT s.id, s.file_id, s.name, s.kind, s.signature,
                        s.start_line, s.start_col, s.end_line, s.end_col, f.path,
                        e.start_line, e.end_line, e.embedding
                 FROM symbols s
                 JOIN files f ON f.id = s.file_id
                 JOIN symbol_embeddings e ON e.symbol_id = s.id
                 ORDER BY s.id, e.start_line
                 LIMIT ? OFFSET ?",
            )
            .map_err(|e| format!("prepare: {e}"))?;
        let results = stmt
            .query_map(params![page_size, offset], |row| {
                let blob: Vec<u8> = row.get(12)?;
                let embedding: Vec<f32> = blob
                    .chunks_exact(4)
                    .map(|chunk| f32::from_le_bytes(chunk.try_into().unwrap_or([0; 4])))
                    .collect();
                Ok((
                    SymbolRecord {
                        id: row.get(0)?,
                        file_id: row.get(1)?,
                        name: row.get(2)?,
                        kind: row.get(3)?,
                        signature: row.get(4)?,
                        start_line: row.get(5)?,
                        start_col: row.get(6)?,
                        end_line: row.get(7)?,
                        end_col: row.get(8)?,
                        file_path: row.get(9)?,
                    },
                    row.get::<_, i64>(10)?,
                    row.get::<_, i64>(11)?,
                    embedding,
                ))
            })
            .map_err(|e| format!("query: {e}"))?
            .filter_map(|r| r.ok())
            .collect();
        Ok(results)
    }

    /// Vector leg: paginated cosine scan of every embedded chunk, gated at
    /// `min_cosine`, reduced to the best chunk per symbol, ranked by cosine.
    fn vector_leg_candidates(
        &self,
        query_vec: &[f32],
        min_cosine: f32,
        k: usize,
    ) -> Result<Vec<LegHit>, String> {
        let mut best_per_symbol: std::collections::HashMap<i64, LegHit> =
            std::collections::HashMap::new();
        let mut offset: i64 = 0;
        loop {
            let page = self.load_embeddings_page(EMBEDDING_PAGE_SIZE, offset)?;
            let page_len = page.len();
            if page.is_empty() {
                break;
            }
            for (sym, chunk_start, chunk_end, emb) in page {
                if emb.len() != query_vec.len() {
                    continue;
                }
                let dot: f32 = query_vec.iter().zip(emb.iter()).map(|(a, b)| a * b).sum();
                let cosine = dot.clamp(0.0, 1.0);
                if cosine < min_cosine {
                    continue;
                }
                let hit = LegHit {
                    symbol_id: sym.id,
                    name: sym.name,
                    kind: sym.kind,
                    signature: sym.signature,
                    file_path: sym.file_path.unwrap_or_default(),
                    sym_start_line: sym.start_line,
                    sym_end_line: sym.end_line,
                    chunk_start,
                    chunk_end,
                    cosine,
                    fts_text: String::new(),
                };
                match best_per_symbol.entry(hit.symbol_id) {
                    std::collections::hash_map::Entry::Occupied(mut e) => {
                        if hit.cosine > e.get().cosine {
                            e.insert(hit);
                        }
                    }
                    std::collections::hash_map::Entry::Vacant(e) => {
                        e.insert(hit);
                    }
                }
            }
            // Advance the cursor by the actual number of rows read; a short
            // page means there are no more rows.
            offset += page_len as i64;
            if (page_len as i64) < EMBEDDING_PAGE_SIZE {
                break;
            }
        }
        let mut hits: Vec<LegHit> = best_per_symbol.into_values().collect();
        hits.sort_by(|a, b| {
            b.cosine
                .partial_cmp(&a.cosine)
                .unwrap_or(std::cmp::Ordering::Equal)
        });
        hits.truncate(k);
        Ok(hits)
    }

    /// BM25 leg: weighted match over chunk_fts, reduced to the best-ranked
    /// chunk per symbol (rows arrive rank-ordered, so first wins).
    fn bm25_leg_candidates(&self, match_query: &str, k: usize) -> Result<Vec<LegHit>, String> {
        let conn = self.conn.lock().map_err(|e| e.to_string())?;
        let sql = format!(
            "SELECT c.symbol_id, s.name, s.kind, s.signature, f.path,
                    s.start_line, s.end_line, c.start_line, c.end_line,
                    c.fts_name || ' ' || c.fts_path || ' ' || c.fts_body
             FROM chunk_fts
             JOIN symbol_chunks c ON c.id = chunk_fts.rowid
             JOIN symbols s ON s.id = c.symbol_id
             JOIN files f ON f.id = s.file_id
             WHERE chunk_fts MATCH ?1
             ORDER BY bm25(chunk_fts, {BM25_W_NAME}, {BM25_W_PATH}, {BM25_W_BODY})
             LIMIT ?2"
        );
        let mut stmt = conn
            .prepare(&sql)
            .map_err(|e| format!("prepare bm25: {e}"))?;
        // 2x headroom: several chunks of one symbol can occupy top ranks.
        let raw: Vec<LegHit> = stmt
            .query_map(params![match_query, (k * 2) as i64], |row| {
                Ok(LegHit {
                    symbol_id: row.get(0)?,
                    name: row.get(1)?,
                    kind: row.get(2)?,
                    signature: row.get(3)?,
                    file_path: row.get(4)?,
                    sym_start_line: row.get(5)?,
                    sym_end_line: row.get(6)?,
                    chunk_start: row.get(7)?,
                    chunk_end: row.get(8)?,
                    cosine: 0.0,
                    fts_text: row.get(9)?,
                })
            })
            .map_err(|e| format!("query bm25: {e}"))?
            .filter_map(|r| r.ok())
            .collect();
        let mut seen: std::collections::HashSet<i64> = std::collections::HashSet::new();
        let mut hits: Vec<LegHit> = Vec::new();
        for hit in raw {
            if seen.insert(hit.symbol_id) {
                hits.push(hit);
            }
            if hits.len() >= k {
                break;
            }
        }
        Ok(hits)
    }

    pub fn search_hybrid(
        &self,
        query_text: &str,
        query_vec: Option<&[f32]>,
        limit: usize,
    ) -> Result<Vec<SemanticSearchResult>, String> {
        self.search_hybrid_with(query_text, query_vec, limit, &HybridParams::default())
    }

    /// Hybrid retrieval: BM25 over chunk_fts fused with cosine over
    /// symbol_embeddings via normalized Reciprocal Rank Fusion, then the
    /// pre-existing lexical boosts and doc/test penalties. Either leg may be
    /// absent (no vector while the model loads or embeddings are pending; no
    /// BM25 when no query term survives sanitization) — the other leg still
    /// returns results, which is the graceful-degradation story during the
    /// background embedding window.
    pub fn search_hybrid_with(
        &self,
        query_text: &str,
        query_vec: Option<&[f32]>,
        limit: usize,
        params: &HybridParams,
    ) -> Result<Vec<SemanticSearchResult>, String> {
        let tokens = tokenize_query(query_text);
        let match_query = build_fts_match_query(query_text);

        let vector_hits = match query_vec {
            Some(qv) => {
                self.vector_leg_candidates(qv, params.min_cosine_candidate, params.k_candidates)?
            }
            None => vec![],
        };
        let bm25_hits = match &match_query {
            Some(mq) => self.bm25_leg_candidates(mq, params.k_candidates)?,
            None => vec![],
        };

        // Evidence gate for hits the vector leg doesn't corroborate, applied
        // before ranks are assigned so surviving hits keep dense ranks.
        let vector_ids: std::collections::HashSet<i64> =
            vector_hits.iter().map(|h| h.symbol_id).collect();
        let required = required_token_matches(tokens.len(), params.min_bm25_term_matches);
        let bm25_hits: Vec<LegHit> = bm25_hits
            .into_iter()
            .filter(|h| {
                vector_ids.contains(&h.symbol_id)
                    || bm25_only_hit_has_evidence(&tokens, h, required)
            })
            .collect();

        // Normalized RRF: 1.0 = rank 1 in both legs, 0.5 = rank 1 in exactly
        // one. Vector hits are folded first so their chunk anchors the result
        // line range when both legs agree on a symbol.
        struct Fused {
            hit: LegHit,
            rrf: f32,
            in_vector: bool,
            in_bm25: bool,
        }
        let mut fused: std::collections::HashMap<i64, Fused> = std::collections::HashMap::new();
        for (i, hit) in vector_hits.into_iter().enumerate() {
            let contrib = params.w_vector / (params.rrf_k + (i + 1) as f32);
            fused.insert(
                hit.symbol_id,
                Fused {
                    hit,
                    rrf: contrib,
                    in_vector: true,
                    in_bm25: false,
                },
            );
        }
        for (i, hit) in bm25_hits.into_iter().enumerate() {
            let contrib = params.w_bm25 / (params.rrf_k + (i + 1) as f32);
            match fused.entry(hit.symbol_id) {
                std::collections::hash_map::Entry::Occupied(mut e) => {
                    let f = e.get_mut();
                    f.rrf += contrib;
                    f.in_bm25 = true;
                }
                std::collections::hash_map::Entry::Vacant(e) => {
                    e.insert(Fused {
                        hit,
                        rrf: contrib,
                        in_vector: false,
                        in_bm25: true,
                    });
                }
            }
        }
        let norm_denom = (params.w_vector + params.w_bm25) / (params.rrf_k + 1.0);

        let mut scored: Vec<SemanticSearchResult> = Vec::new();
        for f in fused.into_values() {
            let norm = if norm_denom > 0.0 {
                f.rrf / norm_denom
            } else {
                0.0
            };
            let boost = lexical_boost(&tokens, &f.hit.name, &f.hit.file_path);
            let mut penalty = if f.hit.kind == "doc_section" {
                DOC_SECTION_PENALTY
            } else {
                0.0
            };
            if is_test_file(&f.hit.file_path) || is_test_symbol(&f.hit.name) {
                penalty += TEST_FILE_PENALTY;
            }
            let score = (norm + boost - penalty).clamp(0.0, 1.0);
            if score < params.min_hybrid_score {
                continue;
            }
            let (start_line, end_line) = if f.hit.chunk_start > 0 {
                (f.hit.chunk_start, f.hit.chunk_end)
            } else {
                (f.hit.sym_start_line, f.hit.sym_end_line)
            };
            let match_type = match (f.in_vector, f.in_bm25) {
                (true, true) => "hybrid",
                (true, false) => "semantic",
                _ => "lexical",
            };
            scored.push(SemanticSearchResult {
                symbol_id: f.hit.symbol_id,
                name: f.hit.name,
                kind: f.hit.kind,
                file_path: f.hit.file_path,
                start_line,
                end_line,
                signature: f.hit.signature,
                score,
                match_type: match_type.to_string(),
                snippet: None,
            });
        }
        // Deterministic tiebreak on symbol_id: several hits clamp to 1.0, and
        // without it their order follows HashMap drain order — ranks would
        // flap between identical runs.
        scored.sort_by(|a, b| {
            b.score
                .partial_cmp(&a.score)
                .unwrap_or(std::cmp::Ordering::Equal)
                .then(a.symbol_id.cmp(&b.symbol_id))
        });

        // Dedupe by file (keep highest-scoring first) before applying limit,
        // so one large file can't occupy the whole ranking.
        let mut per_file_count: std::collections::HashMap<String, usize> =
            std::collections::HashMap::new();
        scored.retain(|r| {
            let count = per_file_count.entry(r.file_path.clone()).or_insert(0);
            *count += 1;
            *count <= MAX_RESULTS_PER_FILE
        });

        scored.truncate(limit);
        Ok(scored)
    }

    /// Number of indexed files whose embeddings are missing or stale
    /// (`embed_hash` absent or behind the content hash). Non-zero means the
    /// background embedding pass hasn't caught up yet, so semantic search may
    /// silently miss content.
    pub fn embedding_pending_files(&self) -> Result<i64, String> {
        let conn = self.conn.lock().map_err(|e| e.to_string())?;
        conn.query_row(
            "SELECT count(*) FROM files WHERE embed_hash IS NULL OR embed_hash != hash",
            [],
            |row| row.get(0),
        )
        .map_err(|e| format!("embedding_pending_files: {e}"))
    }

    #[allow(dead_code)]
    pub fn index_stats(&self) -> Result<(i64, i64, i64), String> {
        let conn = self.conn.lock().map_err(|e| e.to_string())?;
        let files: i64 = conn
            .query_row("SELECT count(*) FROM files", [], |row| row.get(0))
            .unwrap_or(0);
        let symbols: i64 = conn
            .query_row("SELECT count(*) FROM symbols", [], |row| row.get(0))
            .unwrap_or(0);
        let embeddings: i64 = conn
            .query_row("SELECT count(*) FROM symbol_embeddings", [], |row| {
                row.get(0)
            })
            .unwrap_or(0);
        Ok((files, symbols, embeddings))
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn unit_vec(dims: usize, hot: usize) -> Vec<f32> {
        let mut v = vec![0.0f32; dims];
        v[hot] = 1.0;
        v
    }

    #[test]
    fn lexical_boost_layers_dont_stack() {
        let tokens = tokenize_query("Icon.tsx component");
        // Exact basename match wins the +0.25 layer even though a substring
        // match would also apply.
        let boost = lexical_boost(&tokens, "Icon", "src/ui/Icon.tsx");
        assert_eq!(boost, 0.25);

        // Exact symbol-name match (no basename match) -> the middle +0.15
        // layer: short names collide with ordinary query words too easily
        // to outrank everything.
        let boost = lexical_boost(&tokens, "icon", "src/ui/other.tsx");
        assert_eq!(boost, 0.15);

        // Only a substring relationship -> the lower +0.10 layer.
        let boost = lexical_boost(&tokens, "IconButton", "src/ui/misc.tsx");
        assert_eq!(boost, 0.10);

        // No relationship at all.
        let boost = lexical_boost(&tokens, "unrelatedThing", "src/ui/misc.rs");
        assert_eq!(boost, 0.0);
    }

    #[test]
    fn test_files_are_detected_across_conventions() {
        assert!(is_test_file("src/components/TasksPanel.test.tsx"));
        assert!(is_test_file("src/lib/ipc.spec.ts"));
        assert!(is_test_file("src-tauri/src/agent/persist_test.rs"));
        assert!(is_test_file("src-tauri/tests/integration.rs"));
        assert!(is_test_file("src/__tests__/App.tsx"));
        assert!(!is_test_file("src/components/TasksPanel.tsx"));
        assert!(!is_test_file("src-tauri/src/agent/tests_helper_naming.rs"));
    }

    #[test]
    fn inline_test_symbols_are_detected() {
        assert!(is_test_symbol("test_golden_pending_ids"));
        assert!(is_test_symbol("golden_tests"));
        assert!(is_test_symbol("tests"));
        assert!(is_test_symbol("roundtrip_test"));
        assert!(!is_test_symbol("TasksPanel"));
        assert!(!is_test_symbol("attestation"));
    }

    #[test]
    fn tokenize_query_drops_short_tokens_and_stopwords() {
        let tokens = tokenize_query("the Icon.tsx and for a with X");
        assert_eq!(tokens, vec!["icon".to_string(), "tsx".to_string()]);
    }

    #[test]
    fn hybrid_dedupes_per_file_and_applies_threshold() {
        let db = IndexDb::open(Path::new(":memory:")).expect("open in-memory db");
        let file_id = db
            .upsert_file("src/big_file.ts", "typescript", "hash1", 0, 0)
            .expect("upsert file");

        // Five symbols in the same file, all with a near-perfect embedding
        // match, so dedupe (max 3 per file) — not the score — is what
        // trims them.
        for i in 0..5 {
            let sym_id = db
                .insert_symbol(
                    file_id,
                    &format!("sym{i}"),
                    "function",
                    None,
                    i,
                    0,
                    i,
                    0,
                    None,
                )
                .expect("insert symbol");
            db.upsert_embedding(sym_id, 0, 0, 0, &unit_vec(4, 0))
                .expect("upsert embedding");
        }

        // One symbol in a different file with a low-similarity embedding
        // that falls below the vector-leg cosine gate.
        let other_file_id = db
            .upsert_file("src/other.ts", "typescript", "hash2", 0, 0)
            .expect("upsert file");
        let low_sym_id = db
            .insert_symbol(
                other_file_id,
                "lowMatch",
                "function",
                None,
                0,
                0,
                0,
                0,
                None,
            )
            .expect("insert symbol");
        db.upsert_embedding(low_sym_id, 0, 0, 0, &unit_vec(4, 3))
            .expect("upsert embedding");

        let query_vec = unit_vec(4, 0);
        let results = db
            .search_hybrid("sym", Some(&query_vec), 10)
            .expect("search_hybrid");

        assert_eq!(results.len(), 3, "dedupe should cap results per file at 3");
        assert!(results.iter().all(|r| r.file_path == "src/big_file.ts"));
        let min_score = HybridParams::default().min_hybrid_score;
        assert!(results.iter().all(|r| r.score >= min_score));
        assert!(results.iter().all(|r| r.match_type == "semantic"));
    }

    #[test]
    fn hybrid_can_return_empty() {
        let db = IndexDb::open(Path::new(":memory:")).expect("open in-memory db");
        let file_id = db
            .upsert_file("src/only.ts", "typescript", "hash", 0, 0)
            .expect("upsert file");
        let sym_id = db
            .insert_symbol(file_id, "unrelated", "function", None, 1, 0, 3, 0, None)
            .expect("insert symbol");
        // Orthogonal embedding -> cosine ~0; chunk text shares no word with
        // the query -> the BM25 leg matches nothing either.
        db.upsert_embedding(sym_id, 0, 0, 0, &unit_vec(4, 1))
            .expect("upsert embedding");
        db.insert_chunk(
            sym_id,
            0,
            1,
            3,
            "function: unrelated",
            "unrelated",
            "only.ts only",
            "banana orchard code",
        )
        .unwrap();

        let query_vec = unit_vec(4, 0);
        let results = db
            .search_hybrid("totally different query", Some(&query_vec), 10)
            .expect("search_hybrid");
        assert!(results.is_empty());
    }

    #[test]
    fn fts_match_builder_neutralizes_operators() {
        // Operator-laden inputs must produce valid MATCH strings (or None) —
        // never an SQL error surfaced to the caller.
        let db = IndexDb::open(Path::new(":memory:")).expect("open in-memory db");
        let file_id = db.upsert_file("src/q.ts", "typescript", "h", 0, 0).unwrap();
        db.insert_symbol(file_id, "fooBar", "function", None, 1, 0, 1, 0, None)
            .unwrap();
        for query in [
            "foo(bar)",
            "a AND b OR c*",
            "\"unbalanced",
            "name:x",
            "-neg",
            "NEAR(a b)",
            "col : val",
            "*)(^",
        ] {
            let result = db.search_symbols(query, 10);
            assert!(result.is_ok(), "query {query:?} errored: {result:?}");
        }
        assert_eq!(build_fts_match_query("*)(^"), None);
        assert_eq!(
            build_fts_match_query("delete_symbols_for_file").as_deref(),
            Some("\"delete_symbols_for_file\"")
        );
        // Sanitized query still finds the symbol: "fooBar" survives as a
        // quoted term and unicode61 folds it to the same token as the name.
        // (Camel-splitting is chunk_fts territory, not symbols_fts.)
        let hits = db.search_symbols("fooBar(arg)", 10).unwrap();
        assert_eq!(hits.len(), 1, "fooBar term should match the fooBar symbol");
    }

    #[test]
    fn hybrid_exact_body_term_ranks_without_vectors() {
        // The headline fix: a term that exists only inside a body must be
        // findable with no vector leg at all (model missing / pending).
        let db = IndexDb::open(Path::new(":memory:")).expect("open in-memory db");
        let file_id = db
            .upsert_file("src/thing.ts", "typescript", "h", 0, 0)
            .unwrap();
        let sym_id = db
            .insert_symbol(file_id, "handleThing", "function", None, 1, 0, 9, 0, None)
            .unwrap();
        db.insert_chunk(
            sym_id,
            0,
            1,
            9,
            "embed",
            "handleThing handle thing",
            "thing.ts thing",
            "calls xyzzy_special_case() here",
        )
        .unwrap();

        let results = db
            .search_hybrid("xyzzy_special_case", None, 10)
            .expect("search");
        assert_eq!(results.len(), 1);
        assert_eq!(results[0].name, "handleThing");
        assert_eq!(results[0].match_type, "lexical");
        assert!(results[0].score >= HybridParams::default().min_hybrid_score);
    }

    #[test]
    fn hybrid_fuses_both_legs_above_single_leg_hits() {
        let db = IndexDb::open(Path::new(":memory:")).expect("open in-memory db");
        // A: vector-only rank 1. B: bm25-only rank 1. C: rank 2 in both —
        // fusion must put C first (2/(k+2) > 1/(k+1) for k=60).
        let fa = db.upsert_file("src/a.ts", "typescript", "h", 0, 0).unwrap();
        let fb = db.upsert_file("src/b.ts", "typescript", "h", 0, 0).unwrap();
        let fc = db.upsert_file("src/c.ts", "typescript", "h", 0, 0).unwrap();
        let sa = db
            .insert_symbol(fa, "alphaFn", "function", None, 1, 0, 2, 0, None)
            .unwrap();
        let sb = db
            .insert_symbol(fb, "betaFn", "function", None, 1, 0, 2, 0, None)
            .unwrap();
        let sc = db
            .insert_symbol(fc, "gammaFn", "function", None, 1, 0, 2, 0, None)
            .unwrap();

        db.upsert_embedding(sa, 0, 1, 2, &unit_vec(4, 0)).unwrap();
        let mixed = {
            // cosine 0.8 against unit(4,0) -> vector rank 2.
            let v = vec![0.8f32, 0.6, 0.0, 0.0];
            v
        };
        db.upsert_embedding(sc, 0, 1, 2, &mixed).unwrap();

        // B mentions the term twice -> bm25 rank 1; C once -> rank 2.
        db.insert_chunk(
            sb,
            0,
            1,
            2,
            "e",
            "betaFn beta fn",
            "b.ts b",
            "zebrafinch pattern zebrafinch pattern",
        )
        .unwrap();
        db.insert_chunk(
            sc,
            0,
            1,
            2,
            "e",
            "gammaFn gamma fn",
            "c.ts c",
            "zebrafinch pattern once",
        )
        .unwrap();

        let results = db
            .search_hybrid("zebrafinch pattern", Some(&unit_vec(4, 0)), 10)
            .expect("search");
        assert_eq!(
            results[0].name, "gammaFn",
            "both-legs hit must outrank single-leg hits"
        );
        assert_eq!(results[0].match_type, "hybrid");
        let names: Vec<&str> = results.iter().map(|r| r.name.as_str()).collect();
        assert!(names.contains(&"betaFn"));
        // alphaFn: vector rank 1 but zero lexical evidence — still present as
        // a semantic hit (0.5 norm) since its cosine cleared the gate.
        assert!(names.contains(&"alphaFn"));
    }

    #[test]
    fn hybrid_bm25_only_incidental_word_is_gated() {
        let db = IndexDb::open(Path::new(":memory:")).expect("open in-memory db");
        let file_id = db
            .upsert_file("src/pay.ts", "typescript", "h", 0, 0)
            .unwrap();
        let sym_id = db
            .insert_symbol(file_id, "renderList", "function", None, 1, 0, 9, 0, None)
            .unwrap();
        // Contains exactly one of the three query tokens ("payment") — not
        // enough evidence for a BM25-only hit on a multi-word query.
        db.insert_chunk(
            sym_id,
            0,
            1,
            9,
            "e",
            "renderList render list",
            "pay.ts pay",
            "handles payment display rows",
        )
        .unwrap();

        let results = db
            .search_hybrid("stripe payment webhook", None, 10)
            .expect("search");
        assert!(
            results.is_empty(),
            "single incidental word must not pass the gate: {results:?}"
        );
    }

    #[test]
    fn required_token_matches_scales_with_query_length() {
        assert_eq!(
            required_token_matches(1, 2),
            1,
            "single-token exact-term queries pass"
        );
        assert_eq!(required_token_matches(2, 2), 2);
        assert_eq!(required_token_matches(3, 2), 2);
        assert_eq!(required_token_matches(4, 2), 2);
        assert_eq!(
            required_token_matches(5, 2),
            3,
            "long NL queries need a majority"
        );
        assert_eq!(required_token_matches(6, 2), 3);
    }

    /// Count raw FTS index hits WITHOUT joining the content table — ghost
    /// rows (index entries whose content row is gone) are only visible this
    /// way, since joins silently mask them.
    fn fts_raw_count(db: &IndexDb, table: &str, needle: &str) -> i64 {
        let conn = db.conn.lock().unwrap();
        conn.query_row(
            &format!("SELECT count(*) FROM {table} WHERE {table} MATCH ?1"),
            params![needle],
            |row| row.get(0),
        )
        .unwrap()
    }

    #[test]
    fn symbols_fts_has_no_ghosts_after_incremental_reindex() {
        let db = IndexDb::open(Path::new(":memory:")).expect("open in-memory db");
        let file_id = db
            .upsert_file("src/a.ts", "typescript", "h1", 0, 0)
            .unwrap();
        db.insert_symbol(file_id, "uniqueAlphaFn", "function", None, 1, 0, 2, 0, None)
            .unwrap();
        assert_eq!(fts_raw_count(&db, "symbols_fts", "uniqueAlphaFn"), 1);

        // Incremental reindex: old symbols deleted, new content inserted.
        db.delete_symbols_for_file(file_id).unwrap();
        db.insert_symbol(file_id, "uniqueBetaFn", "function", None, 1, 0, 2, 0, None)
            .unwrap();

        assert_eq!(
            fts_raw_count(&db, "symbols_fts", "uniqueAlphaFn"),
            0,
            "stale FTS row survived an incremental reindex"
        );
        assert_eq!(fts_raw_count(&db, "symbols_fts", "uniqueBetaFn"), 1);
    }

    #[test]
    fn chunk_fts_purged_by_delete_symbols_for_file_and_delete_file() {
        let db = IndexDb::open(Path::new(":memory:")).expect("open in-memory db");
        let file_id = db
            .upsert_file("src/b.ts", "typescript", "h1", 0, 0)
            .unwrap();
        let sym_id = db
            .insert_symbol(file_id, "widgetFactory", "function", None, 1, 0, 9, 0, None)
            .unwrap();
        db.insert_chunk(
            sym_id,
            0,
            1,
            9,
            "embed text",
            "widgetFactory widget factory",
            "b.ts b",
            "xyzzybody content here",
        )
        .unwrap();

        let chunks = db.chunks_for_file(file_id).unwrap();
        assert_eq!(chunks.len(), 1);
        assert_eq!(chunks[0].embed_text, "embed text");
        assert_eq!((chunks[0].symbol_id, chunks[0].chunk_index), (sym_id, 0));
        assert_eq!(fts_raw_count(&db, "chunk_fts", "xyzzybody"), 1);

        db.delete_symbols_for_file(file_id).unwrap();
        assert_eq!(
            fts_raw_count(&db, "chunk_fts", "xyzzybody"),
            0,
            "chunk FTS ghost row"
        );
        assert!(db.chunks_for_file(file_id).unwrap().is_empty());

        // Re-insert, then remove the whole file: everything must go.
        let sym_id = db
            .insert_symbol(file_id, "widgetFactory", "function", None, 1, 0, 9, 0, None)
            .unwrap();
        db.insert_chunk(
            sym_id,
            0,
            1,
            9,
            "embed text",
            "widgetFactory",
            "b.ts",
            "xyzzybody again",
        )
        .unwrap();
        db.delete_file("src/b.ts").unwrap();
        assert!(db.file_by_path("src/b.ts").unwrap().is_none());
        assert_eq!(fts_raw_count(&db, "chunk_fts", "xyzzybody"), 0);
        assert_eq!(fts_raw_count(&db, "symbols_fts", "widgetFactory"), 0);
    }

    #[test]
    fn chunks_written_without_embedder_are_readable_for_later_embedding() {
        // Scan-time behavior with no model loaded: chunks + FTS exist so BM25
        // works, symbol_embeddings stays empty until the background pass.
        let db = IndexDb::open(Path::new(":memory:")).expect("open in-memory db");
        let file_id = db
            .upsert_file("src/c.ts", "typescript", "h1", 0, 0)
            .unwrap();
        let sym_id = db
            .insert_symbol(file_id, "pendingFn", "function", None, 1, 0, 4, 0, None)
            .unwrap();
        db.insert_chunk(
            sym_id,
            0,
            1,
            4,
            "function: pendingFn | body",
            "pendingFn pending fn",
            "c.ts c",
            "body",
        )
        .unwrap();

        let pending = db.embedding_pending_files().unwrap();
        assert_eq!(pending, 1, "file without embed_hash counts as pending");

        // The background pass reads the stored chunk and writes the vector
        // keyed by (symbol_id, chunk_index).
        let chunks = db.chunks_for_file(file_id).unwrap();
        db.upsert_embedding(
            chunks[0].symbol_id,
            chunks[0].chunk_index,
            chunks[0].start_line,
            chunks[0].end_line,
            &unit_vec(4, 0),
        )
        .unwrap();
        db.set_embed_hash(file_id, "h1").unwrap();
        assert_eq!(db.embedding_pending_files().unwrap(), 0);
    }

    #[test]
    fn embed_hash_tracks_content_independently_of_re_scan() {
        let db = IndexDb::open(Path::new(":memory:")).expect("open in-memory db");
        let file_id = db
            .upsert_file("src/foo.ts", "typescript", "hash-v1", 0, 0)
            .expect("upsert file");

        // No embed_hash yet — file has never been embedded.
        let file = db.file_by_path("src/foo.ts").unwrap().unwrap();
        assert_eq!(file.embed_hash, None);
        assert_ne!(file.hash, file.embed_hash);

        // Embedding generation records the hash it embedded from.
        db.set_embed_hash(file_id, "hash-v1")
            .expect("set embed_hash");
        let file = db.file_by_path("src/foo.ts").unwrap().unwrap();
        assert_eq!(file.embed_hash.as_deref(), Some("hash-v1"));
        assert_eq!(
            file.hash, file.embed_hash,
            "hash == embed_hash means the file is up to date"
        );

        // Re-scanning with unchanged content re-upserts the same hash and
        // must NOT disturb embed_hash — this is what lets a second
        // generate_all_embeddings pass skip the file entirely.
        db.upsert_file("src/foo.ts", "typescript", "hash-v1", 1, 0)
            .expect("re-upsert unchanged file");
        let file = db.file_by_path("src/foo.ts").unwrap().unwrap();
        assert_eq!(file.embed_hash.as_deref(), Some("hash-v1"));
        assert_eq!(file.hash, file.embed_hash);

        // Editing the file changes hash but leaves embed_hash pointing at
        // the stale content, so the mismatch correctly flags it as pending.
        db.upsert_file("src/foo.ts", "typescript", "hash-v2", 2, 0)
            .expect("re-upsert changed file");
        let file = db.file_by_path("src/foo.ts").unwrap().unwrap();
        assert_eq!(file.hash.as_deref(), Some("hash-v2"));
        assert_eq!(file.embed_hash.as_deref(), Some("hash-v1"));
        assert_ne!(
            file.hash, file.embed_hash,
            "content changed — embedding is now stale"
        );
    }
}
