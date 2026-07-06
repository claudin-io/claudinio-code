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
const SCHEMA_VERSION: i64 = 1;

impl IndexDb {
    pub fn open(db_path: &Path) -> Result<Self, String> {
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
                size INTEGER
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
                symbol_id INTEGER PRIMARY KEY,
                embedding BLOB NOT NULL,
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
            .prepare("SELECT id, path, language, hash, last_modified, size FROM files ORDER BY id")
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
                })
            })
            .map_err(|e| format!("query: {e}"))?
            .filter_map(|r| r.ok())
            .collect();
        Ok(results)
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
            .prepare("SELECT id, path, language, hash, last_modified, size FROM files WHERE path = ?1")
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

    pub fn upsert_embedding(&self, symbol_id: i64, embedding: &[f32]) -> Result<(), String> {
        let bytes: Vec<u8> = embedding
            .iter()
            .flat_map(|f| f.to_le_bytes())
            .collect();
        let conn = self.conn.lock().map_err(|e| e.to_string())?;
        conn.execute(
            "INSERT OR REPLACE INTO symbol_embeddings (symbol_id, embedding) VALUES (?1, ?2)",
            params![symbol_id, bytes],
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

    pub fn load_all_embeddings(&self) -> Result<Vec<(SymbolRecord, Vec<f32>)>, String> {
        let conn = self.conn.lock().map_err(|e| e.to_string())?;
        let mut stmt = conn
            .prepare(
                "SELECT s.id, s.file_id, s.name, s.kind, s.signature,
                        s.start_line, s.start_col, s.end_line, s.end_col, f.path,
                        e.embedding
                 FROM symbols s
                 JOIN files f ON f.id = s.file_id
                 JOIN symbol_embeddings e ON e.symbol_id = s.id",
            )
            .map_err(|e| format!("prepare: {e}"))?;
        let results = stmt
            .query_map([], |row| {
                let blob: Vec<u8> = row.get(10)?;
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
                    embedding,
                ))
            })
            .map_err(|e| format!("query: {e}"))?
            .filter_map(|r| r.ok())
            .collect();
        Ok(results)
    }

    pub fn search_by_embedding(&self, query_vec: &[f32], limit: usize) -> Result<Vec<SemanticSearchResult>, String> {
        let all = self.load_all_embeddings()?;
        let mut scored: Vec<(f32, SemanticSearchResult)> = all
            .into_iter()
            .filter(|(_, emb)| emb.len() == query_vec.len())
            .map(|(sym, emb)| {
                let dot: f32 = query_vec.iter().zip(emb.iter()).map(|(a, b)| a * b).sum();
                let score = dot.max(0.0).min(1.0);
                (
                    score,
                    SemanticSearchResult {
                        symbol_id: sym.id,
                        name: sym.name,
                        kind: sym.kind,
                        file_path: sym.file_path.unwrap_or_default(),
                        start_line: sym.start_line,
                        end_line: sym.end_line,
                        signature: sym.signature,
                        score,
                        snippet: None,
                    },
                )
            })
            .collect();
        scored.sort_by(|a, b| b.0.partial_cmp(&a.0).unwrap_or(std::cmp::Ordering::Equal));
        scored.truncate(limit);
        Ok(scored.into_iter().map(|(_, r)| r).collect())
    }

    #[allow(dead_code)]
    pub fn index_stats(&self) -> Result<(i64, i64), String> {
        let conn = self.conn.lock().map_err(|e| e.to_string())?;
        let files: i64 = conn
            .query_row("SELECT count(*) FROM files", [], |row| row.get(0))
            .unwrap_or(0);
        let symbols: i64 = conn
            .query_row("SELECT count(*) FROM symbols", [], |row| row.get(0))
            .unwrap_or(0);
        Ok((files, symbols))
    }
}
