use rusqlite::{params, Connection};
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
    /// Source excerpt of the symbol, filled in by the tool layer for top hits.
    pub snippet: Option<String>,
}

/// Bump when the index format changes (schema, embedding layout, ignore
/// rules). A mismatched on-disk index is deleted and rebuilt from scratch.
const SCHEMA_VERSION: i64 = 6;

/// Minimum final score (cosine similarity + lexical boost, clamped to
/// [0, 1]) for a semantic search hit to be returned at all. Calibrated
/// empirically against bge-small/MiniLM-class 384-dim embeddings — tune if
/// the embedding model changes and score distributions shift.
const MIN_SEMANTIC_SCORE: f32 = 0.35;

/// Doc sections (markdown headings) embed dense natural language, so they
/// consistently out-score code symbols on NL queries; this penalty keeps them
/// in the results without letting them crowd out the code the agent needs.
const DOC_SECTION_PENALTY: f32 = 0.12;

/// Applied to results living in test files (see `is_test_file`).
const TEST_FILE_PENALTY: f32 = 0.10;

/// Max results kept per source file before `limit` is applied, so a single
/// large file (many symbols) can't dominate the whole ranking.
const MAX_RESULTS_PER_FILE: usize = 3;

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
            .query_row("SELECT count(*) FROM sqlite_master WHERE type='table'", [], |row| {
                row.get::<_, i64>(0)
            })
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

        conn.execute_batch(&format!(
            "PRAGMA journal_mode=WAL; PRAGMA foreign_keys=ON; PRAGMA user_version={SCHEMA_VERSION};"
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
            ",
        )
        .map_err(|e| format!("schema: {e}"))?;

        let has_fts: bool = conn
            .query_row(
                "SELECT count(*) FROM sqlite_master WHERE type='table' AND name='symbols_fts'",
                [],
                |row| row.get::<_, i64>(0),
            )
            .map(|c| c > 0)
            .unwrap_or(false);

        if !has_fts {
            conn.execute_batch(
                "CREATE VIRTUAL TABLE symbols_fts USING fts5(
                    name, signature,
                    content='symbols',
                    content_rowid='id'
                );",
            )
            .map_err(|e| format!("fts5: {e}"))?;
        }

        Ok(())
    }

    pub fn upsert_file(&self, path: &str, language: &str, hash: &str, modified: i64, size: i64) -> Result<i64, String> {
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
            .query_row("SELECT id FROM files WHERE path = ?1", params![path], |row| row.get(0))
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
    pub fn prune_files_not_in(&self, keep: &std::collections::HashSet<String>) -> Result<i64, String> {
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
            // External-content FTS keeps ghost rows after cascade deletes.
            let _ = conn.execute("INSERT INTO symbols_fts(symbols_fts) VALUES('rebuild')", []);
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
        conn.execute("DELETE FROM symbols WHERE file_id = ?1", params![file_id])
            .map_err(|e| format!("delete symbols: {e}"))?;
        Ok(())
    }

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

    pub fn update_fts_for_file(&self, file_id: i64) -> Result<(), String> {
        let conn = self.conn.lock().map_err(|e| e.to_string())?;
        conn.execute(
            "INSERT INTO symbols_fts(rowid, name, signature)
             SELECT id, name, signature FROM symbols WHERE file_id = ?1",
            params![file_id],
        )
        .map_err(|e| format!("update fts: {e}"))?;
        Ok(())
    }

    pub fn search_symbols(&self, query: &str, limit: i64) -> Result<Vec<SearchResult>, String> {
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
            .query_map(params![query, limit], |row| {
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
    pub fn callers_of(&self, symbol_name: &str, file_path: &str) -> Result<Vec<SymbolRecord>, String> {
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

    pub fn upsert_embedding(
        &self,
        symbol_id: i64,
        chunk_index: i64,
        start_line: i64,
        end_line: i64,
        embedding: &[f32],
    ) -> Result<(), String> {
        let bytes: Vec<u8> = embedding
            .iter()
            .flat_map(|f| f.to_le_bytes())
            .collect();
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

    /// One embedded chunk of a symbol, as stored. `chunk_start_line`/`chunk_end_line`
    /// are 0 for whole-symbol embeddings (headers, small bodies).
    pub fn load_all_embeddings(&self) -> Result<Vec<(SymbolRecord, i64, i64, Vec<f32>)>, String> {
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

    pub fn search_by_embedding(
        &self,
        query_text: &str,
        query_vec: &[f32],
        limit: usize,
    ) -> Result<Vec<SemanticSearchResult>, String> {
        let tokens = tokenize_query(query_text);
        let all = self.load_all_embeddings()?;
        // Score every chunk, then keep only the best chunk per symbol so a
        // long function split into many chunks still yields one result —
        // pointed at the chunk's line range, not the whole symbol.
        let mut best_per_symbol: std::collections::HashMap<i64, SemanticSearchResult> =
            std::collections::HashMap::new();
        for (sym, chunk_start, chunk_end, emb) in all {
            if emb.len() != query_vec.len() {
                continue;
            }
            let dot: f32 = query_vec.iter().zip(emb.iter()).map(|(a, b)| a * b).sum();
            let cosine = dot.max(0.0).min(1.0);
            let file_path = sym.file_path.unwrap_or_default();
            let boost = lexical_boost(&tokens, &sym.name, &file_path);
            let mut penalty = if sym.kind == "doc_section" { DOC_SECTION_PENALTY } else { 0.0 };
            if is_test_file(&file_path) || is_test_symbol(&sym.name) {
                penalty += TEST_FILE_PENALTY;
            }
            let score = (cosine + boost - penalty).clamp(0.0, 1.0);
            if score < MIN_SEMANTIC_SCORE {
                continue;
            }
            let (start_line, end_line) = if chunk_start > 0 {
                (chunk_start, chunk_end)
            } else {
                (sym.start_line, sym.end_line)
            };
            let candidate = SemanticSearchResult {
                symbol_id: sym.id,
                name: sym.name,
                kind: sym.kind,
                file_path,
                start_line,
                end_line,
                signature: sym.signature,
                score,
                snippet: None,
            };
            match best_per_symbol.entry(sym.id) {
                std::collections::hash_map::Entry::Occupied(mut e) => {
                    if candidate.score > e.get().score {
                        e.insert(candidate);
                    }
                }
                std::collections::hash_map::Entry::Vacant(e) => {
                    e.insert(candidate);
                }
            }
        }
        let mut scored: Vec<SemanticSearchResult> = best_per_symbol.into_values().collect();
        scored.sort_by(|a, b| b.score.partial_cmp(&a.score).unwrap_or(std::cmp::Ordering::Equal));

        // Dedupe by file (keep highest-scoring first) before applying limit,
        // so one large file can't occupy the whole ranking.
        let mut per_file_count: std::collections::HashMap<String, usize> = std::collections::HashMap::new();
        scored.retain(|r| {
            let count = per_file_count.entry(r.file_path.clone()).or_insert(0);
            *count += 1;
            *count <= MAX_RESULTS_PER_FILE
        });

        scored.truncate(limit);
        Ok(scored)
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
            .query_row("SELECT count(*) FROM symbol_embeddings", [], |row| row.get(0))
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
    fn search_by_embedding_dedupes_per_file_and_applies_threshold() {
        let db = IndexDb::open(Path::new(":memory:")).expect("open in-memory db");
        let file_id = db
            .upsert_file("src/big_file.ts", "typescript", "hash1", 0, 0)
            .expect("upsert file");

        // Five symbols in the same file, all with a near-perfect embedding
        // match, so dedupe (max 3 per file) — not the score — is what
        // trims them.
        for i in 0..5 {
            let sym_id = db
                .insert_symbol(file_id, &format!("sym{i}"), "function", None, i, 0, i, 0, None)
                .expect("insert symbol");
            db.upsert_embedding(sym_id, 0, 0, 0, &unit_vec(4, 0)).expect("upsert embedding");
        }

        // One symbol in a different file with a low-similarity embedding
        // that should be dropped by MIN_SEMANTIC_SCORE.
        let other_file_id = db
            .upsert_file("src/other.ts", "typescript", "hash2", 0, 0)
            .expect("upsert file");
        let low_sym_id = db
            .insert_symbol(other_file_id, "lowMatch", "function", None, 0, 0, 0, 0, None)
            .expect("insert symbol");
        db.upsert_embedding(low_sym_id, 0, 0, 0, &unit_vec(4, 3)).expect("upsert embedding");

        let query_vec = unit_vec(4, 0);
        let results = db
            .search_by_embedding("sym", &query_vec, 10)
            .expect("search_by_embedding");

        assert_eq!(results.len(), 3, "dedupe should cap results per file at 3");
        assert!(results.iter().all(|r| r.file_path == "src/big_file.ts"));
        assert!(results.iter().all(|r| r.score >= MIN_SEMANTIC_SCORE));
    }

    #[test]
    fn search_by_embedding_can_return_empty() {
        let db = IndexDb::open(Path::new(":memory:")).expect("open in-memory db");
        let file_id = db
            .upsert_file("src/only.ts", "typescript", "hash", 0, 0)
            .expect("upsert file");
        let sym_id = db
            .insert_symbol(file_id, "unrelated", "function", None, 0, 0, 0, 0, None)
            .expect("insert symbol");
        // Orthogonal embedding -> cosine ~0, no lexical overlap -> no boost.
        db.upsert_embedding(sym_id, 0, 0, 0, &unit_vec(4, 1)).expect("upsert embedding");

        let query_vec = unit_vec(4, 0);
        let results = db
            .search_by_embedding("totally different query", &query_vec, 10)
            .expect("search_by_embedding");
        assert!(results.is_empty());
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
        db.set_embed_hash(file_id, "hash-v1").expect("set embed_hash");
        let file = db.file_by_path("src/foo.ts").unwrap().unwrap();
        assert_eq!(file.embed_hash.as_deref(), Some("hash-v1"));
        assert_eq!(file.hash, file.embed_hash, "hash == embed_hash means the file is up to date");

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
        assert_ne!(file.hash, file.embed_hash, "content changed — embedding is now stale");
    }
}
