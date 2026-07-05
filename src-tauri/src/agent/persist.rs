//! JSONL session persistence.
//!
//! Every session is a single append-only `.jsonl` file under
//! `<workspace>/.claudinio/sessions/<id>.jsonl` (or the user config dir when no
//! workspace is open). One JSON record per line. The stream is enough to both
//! (a) reconstruct the conversation history to continue the session with the
//! model, and (b) replay a human-readable trace for debugging.
//!
//! Records are tagged (`kind`) so the format can grow without breaking readers:
//! unknown kinds are simply skipped on load.

use crate::agent::provider::Message;
use serde::{Deserialize, Serialize};
use std::io::{BufRead, Write};
use std::path::{Path, PathBuf};
use std::time::{SystemTime, UNIX_EPOCH};

/// One line of a session JSONL file.
#[derive(Debug, Clone, Serialize, Deserialize)]
#[serde(tag = "kind", rename_all = "snake_case")]
pub enum SessionRecord {
    /// First line of every session file.
    Meta {
        session_id: String,
        created_at: u64,
        workspace: Option<String>,
    },
    /// A user turn (the raw input the user typed).
    User { text: String, ts: u64 },
    /// A workflow phase boundary: "plan" | "execute" | "summary".
    Phase { phase: String, ts: u64 },
    /// A conversation message exactly as sent to / received from the model.
    /// Collecting these in order reconstructs the model history.
    Turn {
        #[serde(flatten)]
        message: Message,
        ts: u64,
    },
    /// The text a phase produced (the plan, or the final summary).
    PhaseResult { phase: String, text: String, ts: u64 },
    /// End of a workflow run.
    Done {
        input_tokens: u32,
        output_tokens: u32,
        ts: u64,
    },
    /// A run failed.
    Error { message: String, ts: u64 },
}

pub fn now_ms() -> u64 {
    SystemTime::now()
        .duration_since(UNIX_EPOCH)
        .map(|d| d.as_millis() as u64)
        .unwrap_or(0)
}

/// Directory that holds session files for the given workspace (or global).
pub fn sessions_dir(workspace: Option<&str>) -> Result<PathBuf, String> {
    let dir = match workspace {
        Some(root) => Path::new(root).join(".claudinio").join("sessions"),
        None => dirs::config_dir()
            .ok_or("no config dir")?
            .join("claudinio-code")
            .join("sessions"),
    };
    std::fs::create_dir_all(&dir).map_err(|e| format!("create sessions dir: {e}"))?;
    Ok(dir)
}

/// Append-only handle to one session's JSONL file. Cheap to clone (holds a path);
/// each `append` opens the file in append mode, so it is safe to hold across
/// async await points.
#[derive(Debug, Clone)]
pub struct SessionStore {
    pub path: PathBuf,
}

impl SessionStore {
    /// Create (or attach to) the file for `session_id`, writing the `Meta`
    /// header when the file is new.
    pub fn create(
        session_id: &str,
        workspace: Option<&str>,
    ) -> Result<Self, String> {
        let dir = sessions_dir(workspace)?;
        let path = dir.join(format!("{session_id}.jsonl"));
        let is_new = !path.exists();
        let store = SessionStore { path };
        if is_new {
            store.append(&SessionRecord::Meta {
                session_id: session_id.to_string(),
                created_at: now_ms(),
                workspace: workspace.map(|w| w.to_string()),
            })?;
        }
        Ok(store)
    }

    pub fn append(&self, record: &SessionRecord) -> Result<(), String> {
        let line = serde_json::to_string(record).map_err(|e| format!("serialize record: {e}"))?;
        let mut file = std::fs::OpenOptions::new()
            .create(true)
            .append(true)
            .open(&self.path)
            .map_err(|e| format!("open session file: {e}"))?;
        writeln!(file, "{line}").map_err(|e| format!("write session file: {e}"))?;
        Ok(())
    }

    /// A best-effort append that never propagates errors — used inside the hot
    /// loop where a persistence hiccup must not abort the agent run.
    pub fn try_append(&self, record: &SessionRecord) {
        let _ = self.append(record);
    }
}

/// Read every record from a session file, skipping malformed / unknown lines.
pub fn load_records(path: &Path) -> Result<Vec<SessionRecord>, String> {
    let file = std::fs::File::open(path).map_err(|e| format!("open session file: {e}"))?;
    let reader = std::io::BufReader::new(file);
    let mut out = Vec::new();
    for line in reader.lines() {
        let line = line.map_err(|e| format!("read session file: {e}"))?;
        if line.trim().is_empty() {
            continue;
        }
        if let Ok(rec) = serde_json::from_str::<SessionRecord>(&line) {
            out.push(rec);
        }
    }
    Ok(out)
}

/// Rebuild the model conversation history from a session's records.
pub fn history_from_records(records: &[SessionRecord]) -> Vec<Message> {
    records
        .iter()
        .filter_map(|r| match r {
            SessionRecord::Turn { message, .. } => Some(message.clone()),
            _ => None,
        })
        .collect()
}

/// Lightweight summary shown in the session list.
#[derive(Debug, Clone, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct SessionSummary {
    pub session_id: String,
    pub created_at: u64,
    pub updated_at: u64,
    pub title: String,
    pub turn_count: usize,
}

/// List all sessions for a workspace, newest first.
pub fn list_sessions(workspace: Option<&str>) -> Result<Vec<SessionSummary>, String> {
    let dir = sessions_dir(workspace)?;
    let mut summaries = Vec::new();
    let entries = match std::fs::read_dir(&dir) {
        Ok(e) => e,
        Err(_) => return Ok(summaries),
    };
    for entry in entries.flatten() {
        let path = entry.path();
        if path.extension().and_then(|e| e.to_str()) != Some("jsonl") {
            continue;
        }
        let records = match load_records(&path) {
            Ok(r) => r,
            Err(_) => continue,
        };
        let session_id = path
            .file_stem()
            .and_then(|s| s.to_str())
            .unwrap_or("")
            .to_string();

        let mut created_at = 0u64;
        let mut updated_at = 0u64;
        let mut title = String::new();
        let mut turn_count = 0usize;
        for rec in &records {
            match rec {
                SessionRecord::Meta { created_at: c, .. } => {
                    created_at = *c;
                    updated_at = updated_at.max(*c);
                }
                SessionRecord::User { text, ts } => {
                    if title.is_empty() {
                        title = text.chars().take(80).collect();
                    }
                    turn_count += 1;
                    updated_at = updated_at.max(*ts);
                }
                SessionRecord::Phase { ts, .. }
                | SessionRecord::Turn { ts, .. }
                | SessionRecord::PhaseResult { ts, .. }
                | SessionRecord::Done { ts, .. }
                | SessionRecord::Error { ts, .. } => {
                    updated_at = updated_at.max(*ts);
                }
            }
        }
        if title.is_empty() {
            title = "(empty session)".into();
        }
        summaries.push(SessionSummary {
            session_id,
            created_at,
            updated_at,
            title,
            turn_count,
        });
    }
    summaries.sort_by(|a, b| b.updated_at.cmp(&a.updated_at));
    Ok(summaries)
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::agent::provider::ContentBlock;

    #[test]
    fn roundtrip_history_from_records() {
        let recs = vec![
            SessionRecord::Meta {
                session_id: "s1".into(),
                created_at: 1,
                workspace: None,
            },
            SessionRecord::User {
                text: "hi".into(),
                ts: 2,
            },
            SessionRecord::Turn {
                message: Message {
                    role: "user".into(),
                    content: vec![ContentBlock::text("hi")],
                },
                ts: 3,
            },
            SessionRecord::Turn {
                message: Message {
                    role: "assistant".into(),
                    content: vec![ContentBlock::text("hello")],
                },
                ts: 4,
            },
        ];
        let history = history_from_records(&recs);
        assert_eq!(history.len(), 2);
        assert_eq!(history[0].role, "user");
        assert_eq!(history[1].role, "assistant");
    }

    #[test]
    fn record_tag_is_stable() {
        let rec = SessionRecord::Phase {
            phase: "plan".into(),
            ts: 10,
        };
        let json = serde_json::to_string(&rec).unwrap();
        assert!(json.contains("\"kind\":\"phase\""), "got: {json}");
        assert!(json.contains("\"phase\":\"plan\""), "got: {json}");
    }
}
