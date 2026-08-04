//! Cross-session history for the quality harness.
//!
//! Every earlier layer answers a question about *this* run, so the session
//! JSONL was the right home for it: per-session scope, no second source of
//! truth, no migrations. Complexity is the first question that spans sessions —
//! "is this codebase getting better or worse?" cannot be answered from one
//! conversation — so it is the first thing that earns a database.
//!
//! Deliberately **not** `index.db`: that one is dropped and rebuilt whenever
//! `SCHEMA_VERSION` changes, which is fine for a derivable index and fatal for
//! history. This store migrates instead, and every migration is additive.

use rusqlite::Connection;
use serde::{Deserialize, Serialize};

use super::metrics::MetricsSummary;

/// One recorded measurement of the codebase.
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct MetricsPoint {
    pub ts: u64,
    /// The commit the workspace was on, when it was a git repository.
    #[serde(default)]
    pub commit: Option<String>,
    pub functions: u32,
    pub max_complexity: u32,
    pub mean_complexity: f64,
    /// How many functions exceeded the configured budget, if there was one.
    pub over_budget: u32,
}

pub struct QualityStore {
    conn: Connection,
}

impl QualityStore {
    /// Open (creating if needed) and bring the schema up to date.
    pub fn open(path: &std::path::Path) -> Result<Self, String> {
        if let Some(dir) = path.parent() {
            std::fs::create_dir_all(dir).map_err(|e| format!("create quality db dir: {e}"))?;
        }
        let conn = Connection::open(path).map_err(|e| format!("open quality db: {e}"))?;
        // WAL for the same reason the index uses it: a reader must not block
        // while a run is writing.
        let _ = conn.pragma_update(None, "journal_mode", "WAL");
        let store = Self { conn };
        store.migrate()?;
        Ok(store)
    }

    #[cfg(test)]
    fn open_in_memory() -> Result<Self, String> {
        let conn = Connection::open_in_memory().map_err(|e| format!("open: {e}"))?;
        let store = Self { conn };
        store.migrate()?;
        Ok(store)
    }

    /// Apply every migration this binary knows about, from wherever the file
    /// currently is. Additive only: an older build must still be able to read a
    /// database a newer build wrote.
    fn migrate(&self) -> Result<(), String> {
        self.conn
            .execute_batch(
                "CREATE TABLE IF NOT EXISTS schema_migrations (
                     version INTEGER PRIMARY KEY,
                     applied_at INTEGER NOT NULL
                 );",
            )
            .map_err(|e| format!("quality db migrations table: {e}"))?;

        for (version, sql) in MIGRATIONS {
            if self.has_migration(*version)? {
                continue;
            }
            self.conn
                .execute_batch(sql)
                .map_err(|e| format!("quality db migration {version}: {e}"))?;
            self.conn
                .execute(
                    "INSERT INTO schema_migrations (version, applied_at) VALUES (?1, ?2)",
                    rusqlite::params![version, super::now_ms() as i64],
                )
                .map_err(|e| format!("record migration {version}: {e}"))?;
        }
        Ok(())
    }

    fn has_migration(&self, version: i64) -> Result<bool, String> {
        self.conn
            .query_row(
                "SELECT COUNT(*) FROM schema_migrations WHERE version = ?1",
                rusqlite::params![version],
                |row| row.get::<_, i64>(0),
            )
            .map(|n| n > 0)
            .map_err(|e| format!("check migration {version}: {e}"))
    }

    /// Record one measurement.
    pub fn record(
        &self,
        ts: u64,
        commit: Option<&str>,
        summary: &MetricsSummary,
    ) -> Result<(), String> {
        self.conn
            .execute(
                "INSERT INTO metrics_history
                   (ts, commit_sha, functions, max_complexity, mean_complexity, over_budget)
                 VALUES (?1, ?2, ?3, ?4, ?5, ?6)",
                rusqlite::params![
                    ts as i64,
                    commit,
                    summary.functions,
                    summary.max_complexity,
                    summary.mean_complexity(),
                    summary.over_budget.len() as u32,
                ],
            )
            .map(|_| ())
            .map_err(|e| format!("record metrics: {e}"))
    }

    /// The most recent measurements, newest first.
    pub fn history(&self, limit: usize) -> Result<Vec<MetricsPoint>, String> {
        let mut stmt = self
            .conn
            .prepare(
                "SELECT ts, commit_sha, functions, max_complexity, mean_complexity, over_budget
                 FROM metrics_history ORDER BY ts DESC, id DESC LIMIT ?1",
            )
            .map_err(|e| format!("prepare history: {e}"))?;
        let rows = stmt
            .query_map(rusqlite::params![limit as i64], |row| {
                Ok(MetricsPoint {
                    ts: row.get::<_, i64>(0)? as u64,
                    commit: row.get(1)?,
                    functions: row.get(2)?,
                    max_complexity: row.get(3)?,
                    mean_complexity: row.get(4)?,
                    over_budget: row.get(5)?,
                })
            })
            .map_err(|e| format!("query history: {e}"))?;
        rows.collect::<Result<Vec<_>, _>>()
            .map_err(|e| format!("read history: {e}"))
    }

    /// The measurement before the latest one, for reporting direction.
    pub fn previous(&self) -> Option<MetricsPoint> {
        self.history(2).ok()?.into_iter().nth(1)
    }
}

/// Additive migrations, applied in order. Never edit one that has shipped —
/// add a new one, or a database in the wild ends up in a state no version
/// describes.
const MIGRATIONS: &[(i64, &str)] = &[(
    1,
    "CREATE TABLE metrics_history (
         id INTEGER PRIMARY KEY AUTOINCREMENT,
         ts INTEGER NOT NULL,
         commit_sha TEXT,
         functions INTEGER NOT NULL,
         max_complexity INTEGER NOT NULL,
         mean_complexity REAL NOT NULL,
         over_budget INTEGER NOT NULL
     );
     CREATE INDEX idx_metrics_history_ts ON metrics_history (ts);",
)];

#[cfg(test)]
mod tests {
    use super::*;
    use crate::quality::metrics::FunctionMetric;

    fn summary(functions: u32, max: u32, total: u32, over: usize) -> MetricsSummary {
        MetricsSummary {
            functions,
            max_complexity: max,
            total_complexity: total,
            over_budget: (0..over)
                .map(|i| FunctionMetric {
                    file: "a.rs".into(),
                    name: format!("f{i}"),
                    line: 1,
                    complexity: max,
                    loc: 10,
                })
                .collect(),
        }
    }

    #[test]
    fn a_fresh_database_is_migrated_and_empty() {
        let store = QualityStore::open_in_memory().unwrap();
        assert!(store.history(10).unwrap().is_empty());
        assert!(store.previous().is_none());
    }

    #[test]
    fn migrations_are_idempotent() {
        // Opening an existing database must not reapply anything, or the
        // second launch would fail on "table already exists".
        let dir = std::env::temp_dir().join(format!("cq-store-{}", std::process::id()));
        std::fs::remove_dir_all(&dir).ok();
        let path = dir.join("quality.db");

        let first = QualityStore::open(&path).unwrap();
        first.record(1, Some("abc"), &summary(3, 5, 9, 0)).unwrap();
        drop(first);

        let second = QualityStore::open(&path).expect("reopen must succeed");
        assert_eq!(second.history(10).unwrap().len(), 1, "history must survive");
        std::fs::remove_dir_all(&dir).ok();
    }

    #[test]
    fn history_comes_back_newest_first() {
        let store = QualityStore::open_in_memory().unwrap();
        store
            .record(100, Some("old"), &summary(1, 2, 2, 0))
            .unwrap();
        store
            .record(200, Some("mid"), &summary(2, 4, 6, 1))
            .unwrap();
        store
            .record(300, Some("new"), &summary(3, 6, 12, 2))
            .unwrap();

        let history = store.history(10).unwrap();
        let commits: Vec<_> = history.iter().map(|p| p.commit.as_deref()).collect();
        assert_eq!(commits, vec![Some("new"), Some("mid"), Some("old")]);
        assert_eq!(history[0].functions, 3);
        assert_eq!(history[0].over_budget, 2);
        assert_eq!(history[0].mean_complexity, 4.0);
    }

    #[test]
    fn previous_is_the_one_before_the_latest() {
        // This is what turns a number into a direction.
        let store = QualityStore::open_in_memory().unwrap();
        store.record(100, None, &summary(1, 3, 3, 0)).unwrap();
        store.record(200, None, &summary(1, 9, 9, 0)).unwrap();
        assert_eq!(store.previous().unwrap().max_complexity, 3);
    }

    #[test]
    fn a_single_measurement_has_no_previous_to_compare_against() {
        let store = QualityStore::open_in_memory().unwrap();
        store.record(100, None, &summary(1, 3, 3, 0)).unwrap();
        assert!(store.previous().is_none());
    }

    #[test]
    fn the_limit_is_respected() {
        let store = QualityStore::open_in_memory().unwrap();
        for i in 0..20 {
            store.record(i, None, &summary(1, 1, 1, 0)).unwrap();
        }
        assert_eq!(store.history(5).unwrap().len(), 5);
    }

    #[test]
    fn a_workspace_without_git_records_no_commit() {
        let store = QualityStore::open_in_memory().unwrap();
        store.record(1, None, &summary(1, 1, 1, 0)).unwrap();
        assert!(store.history(1).unwrap()[0].commit.is_none());
    }
}
