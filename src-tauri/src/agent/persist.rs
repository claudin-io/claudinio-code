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
    /// A steering message injected mid-run.
    Steering { text: String, ts: u64 },
    /// Context was compacted: earlier turns replaced by a summary.
    /// The frontend renders this as a collapsible archive block.
    /// `tail_turns` Turn records immediately BEFORE this marker stay live
    /// (verbatim) instead of being folded into the summary.
    #[serde(rename = "compacted")]
    Compacted {
        summary: String,
        #[serde(default)]
        tail_turns: usize,
        ts: u64,
    },
    /// Tasks snapshot written by the agent (tool-level tasks_get/tasks_set).
    #[serde(rename = "tasks")]
    Tasks {
        #[serde(rename = "tasksJson")]
        tasks_json: String,
        ts: u64,
    },
    /// The session's operating mode changed: "pensador" (read-only planning)
    /// or "constructor" (execution). `origin` records who switched:
    /// "human" (UI toggle) or "agent" (enter_plan_mode/exit_plan_mode tools).
    #[serde(rename = "mode")]
    Mode { mode: String, origin: String, ts: u64 },
    /// Periodic status snapshot: cumulative tokens and estimated cost.
    /// Written after every Done record. `context_tokens` is the size of the
    /// context for the NEXT request (drops after compaction), as opposed to
    /// the cumulative totals which are monotonic.
    #[serde(rename = "status")]
    Status {
        session_id: String,
        total_input_tokens: u64,
        total_output_tokens: u64,
        total_cost: Option<f64>,
        #[serde(default)]
        total_cost_input: Option<f64>,
        #[serde(default)]
        total_cost_output: Option<f64>,
        #[serde(default)]
        total_cost_cache_read: Option<f64>,
        #[serde(default)]
        context_tokens: Option<u64>,
        ts: u64,
    },
    /// One iteration of the golden-goals loop: the run ended with golden
    /// tasks still pending, so the workflow flipped mode and continued.
    /// `goals` holds the pending golden task ids at the moment of the flip.
    #[serde(rename = "golden_cycle")]
    GoldenCycle {
        cycle: u32,
        mode: String,
        goals: Vec<String>,
        ts: u64,
    },
    /// The completion judge ran on a terminal `end_turn` (no tool call) and
    /// decided whether the turn was genuinely finished or merely announced a
    /// next step it never took. Persisted for observability even though the UI
    /// does not render it: it is transparent to the user but auditable in the
    /// JSONL. `verdict` is "done" | "continue"; `nudged` is true when the loop
    /// injected a continuation nudge as a result.
    #[serde(rename = "continuation_judge")]
    ContinuationJudge {
        verdict: String,
        nudged: bool,
        streak: u32,
        ts: u64,
    },
}

/// Number of golden cycles already run in this session (the highest
/// `cycle` recorded, or 0 when the loop never ran).
pub fn golden_cycle_count(records: &[SessionRecord]) -> u32 {
    records
        .iter()
        .filter_map(|r| match r {
            SessionRecord::GoldenCycle { cycle, .. } => Some(*cycle),
            _ => None,
        })
        .max()
        .unwrap_or(0)
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
/// Steering records are merged into the last user turn (or create a new one),
/// mirroring push_user_blocks in session.rs.
///
/// When the session has been compacted, only records AFTER the last
/// `Compacted` marker are included, with the summary injected as the
/// opening user message so the model retains context of earlier work.
pub fn history_from_records(records: &[SessionRecord]) -> Vec<Message> {
    // Find the last compaction point
    let compact_idx = records.iter().rposition(|r| matches!(r, SessionRecord::Compacted { .. }));

    let mut out: Vec<Message> = Vec::new();
    match compact_idx {
        Some(idx) => {
            let (summary, tail_turns) = match &records[idx] {
                SessionRecord::Compacted { summary, tail_turns, .. } => {
                    (summary.clone(), *tail_turns)
                }
                _ => (String::new(), 0),
            };
            if !summary.is_empty() {
                out.push(Message {
                    role: "user".into(),
                    content: vec![crate::agent::provider::ContentBlock::text(format!(
                        "[Contexto anterior compactado]\n{}",
                        summary
                    ))],
                });
            }
            // Kept-verbatim tail before the marker, then everything after it.
            let tail_start = tail_start_index(records, idx, tail_turns);
            fold_into_history(&mut out, records[tail_start..idx].iter());
            fold_into_history(&mut out, records.iter().skip(idx + 1));
        }
        None => fold_into_history(&mut out, records.iter()),
    }
    out
}

/// Fold Turn/Steering records into a message history, merging steering text
/// into the last user turn (mirrors push_user_blocks in session.rs).
fn fold_into_history<'a>(
    out: &mut Vec<Message>,
    records: impl Iterator<Item = &'a SessionRecord>,
) {
    for rec in records {
        match rec {
            SessionRecord::Turn { message, .. } => {
                out.push(message.clone());
            }
            SessionRecord::Steering { text, .. } => {
                let block = crate::agent::provider::ContentBlock::text(text);
                if let Some(last) = out.last_mut() {
                    if last.role == "user" {
                        last.content.push(block);
                        continue;
                    }
                }
                out.push(Message {
                    role: "user".into(),
                    content: vec![block],
                });
            }
            _ => {}
        }
    }
}

/// A Turn that starts a real user exchange: role "user" whose first block is
/// plain text (not a tool_result continuation).
pub fn is_real_user_turn(rec: &SessionRecord) -> bool {
    match rec {
        SessionRecord::Turn { message, .. } => {
            message.role == "user"
                && matches!(
                    message.content.first(),
                    Some(crate::agent::provider::ContentBlock::Text { .. })
                )
        }
        _ => false,
    }
}

/// Index where the kept-verbatim tail begins for a `Compacted` marker at
/// `compact_idx` with `tail_turns`. The tail is expanded backwards so the
/// live history never starts on an assistant turn or splits a
/// tool_use/tool_result pair: it must begin at a real user turn, otherwise
/// the tail is dropped entirely (returns `compact_idx`).
pub fn tail_start_index(records: &[SessionRecord], compact_idx: usize, tail_turns: usize) -> usize {
    if tail_turns == 0 {
        return compact_idx;
    }
    // Walk backwards collecting Turn records.
    let mut start = compact_idx;
    let mut count = 0usize;
    for i in (0..compact_idx).rev() {
        if matches!(records[i], SessionRecord::Turn { .. }) {
            start = i;
            count += 1;
            if count >= tail_turns {
                break;
            }
        }
    }
    if count == 0 {
        return compact_idx;
    }
    // Expand backwards until the tail begins at a real user turn.
    loop {
        if is_real_user_turn(&records[start]) {
            return start;
        }
        match (0..start).rev().find(|&i| matches!(records[i], SessionRecord::Turn { .. })) {
            Some(prev) => start = prev,
            None => return compact_idx, // no user turn found — drop the tail
        }
    }
}

/// The mode recorded by the most recent Mode record, if any: (mode, origin).
pub fn last_mode(records: &[SessionRecord]) -> Option<(String, String)> {
    records.iter().rev().find_map(|r| match r {
        SessionRecord::Mode { mode, origin, .. } => Some((mode.clone(), origin.clone())),
        _ => None,
    })
}

/// The context size recorded by the most recent Status record, if any.
pub fn last_context_tokens(records: &[SessionRecord]) -> Option<u64> {
    records.iter().rev().find_map(|r| match r {
        SessionRecord::Status { context_tokens, .. } => *context_tokens,
        _ => None,
    })
}

/// Compute cumulative token/cost stats from Status records.
/// Returns (input_tokens, output_tokens, total_cost, cost_input, cost_output, cost_cache_read).
pub fn cumulative_stats(
    records: &[SessionRecord],
) -> (u64, u64, Option<f64>, Option<f64>, Option<f64>, Option<f64>) {
    let mut total_in = 0u64;
    let mut total_out = 0u64;
    let mut total_cost = 0.0f64;
    let mut has_cost = false;
    let mut cost_input = 0.0f64;
    let mut has_cost_input = false;
    let mut cost_output = 0.0f64;
    let mut has_cost_output = false;
    let mut cost_cache_read = 0.0f64;
    let mut has_cost_cache_read = false;
    for rec in records {
        if let SessionRecord::Status {
            total_input_tokens,
            total_output_tokens,
            total_cost: cost,
            total_cost_input: ci,
            total_cost_output: co,
            total_cost_cache_read: cc,
            ..
        } = rec
        {
            total_in = *total_input_tokens;
            total_out = *total_output_tokens;
            if let Some(c) = cost {
                total_cost = *c;
                has_cost = true;
            }
            if let Some(c) = ci {
                cost_input = *c;
                has_cost_input = true;
            }
            if let Some(c) = co {
                cost_output = *c;
                has_cost_output = true;
            }
            if let Some(c) = cc {
                cost_cache_read = *c;
                has_cost_cache_read = true;
            }
        }
    }
    (
        total_in,
        total_out,
        if has_cost { Some(total_cost) } else { None },
        if has_cost_input { Some(cost_input) } else { None },
        if has_cost_output { Some(cost_output) } else { None },
        if has_cost_cache_read { Some(cost_cache_read) } else { None },
    )
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
                | SessionRecord::Error { ts, .. }
                | SessionRecord::Steering { ts, .. }
                | SessionRecord::Compacted { ts, .. }
                | SessionRecord::Tasks { ts, .. }
                | SessionRecord::Mode { ts, .. }
                | SessionRecord::Status { ts, .. }
                | SessionRecord::GoldenCycle { ts, .. }
                | SessionRecord::ContinuationJudge { ts, .. } => {
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
    fn golden_cycle_roundtrip_and_count() {
        let rec = SessionRecord::GoldenCycle {
            cycle: 2,
            mode: "brain".into(),
            goals: vec!["golden-coverage-80-0".into()],
            ts: 42,
        };
        let json = serde_json::to_string(&rec).unwrap();
        assert!(json.contains("\"golden_cycle\""));
        let back: SessionRecord = serde_json::from_str(&json).unwrap();
        assert!(matches!(back, SessionRecord::GoldenCycle { cycle: 2, .. }));

        let recs = vec![
            SessionRecord::GoldenCycle { cycle: 1, mode: "brain".into(), goals: vec![], ts: 1 },
            rec,
        ];
        assert_eq!(golden_cycle_count(&recs), 2);
        assert_eq!(golden_cycle_count(&[]), 0);
    }

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
    fn history_from_records_with_steering_merges_into_last_user() {
        let recs = vec![
            SessionRecord::Turn {
                message: Message {
                    role: "user".into(),
                    content: vec![ContentBlock::text("hi")],
                },
                ts: 1,
            },
            SessionRecord::Turn {
                message: Message {
                    role: "assistant".into(),
                    content: vec![ContentBlock::text("hello")],
                },
                ts: 2,
            },
            // Steering after assistant -> new user turn
            SessionRecord::Steering {
                text: "steer1".into(),
                ts: 3,
            },
            // Steering again -> merge into the new user turn
            SessionRecord::Steering {
                text: "steer2".into(),
                ts: 4,
            },
        ];
        let history = history_from_records(&recs);
        assert_eq!(history.len(), 3);
        assert_eq!(history[0].role, "user");
        assert_eq!(history[1].role, "assistant");
        assert_eq!(history[2].role, "user");
        assert_eq!(history[2].content.len(), 2);
        assert_eq!(
            history[2].content[0].get_text().unwrap(),
            "steer1"
        );
        assert_eq!(
            history[2].content[1].get_text().unwrap(),
            "steer2"
        );
    }

    #[test]
    fn history_from_records_steering_merges_into_existing_user() {
        let recs = vec![
            SessionRecord::Turn {
                message: Message {
                    role: "user".into(),
                    content: vec![ContentBlock::text("original")],
                },
                ts: 1,
            },
            // Steering should merge into the existing user turn
            SessionRecord::Steering {
                text: "steer".into(),
                ts: 2,
            },
        ];
        let history = history_from_records(&recs);
        assert_eq!(history.len(), 1);
        assert_eq!(history[0].content.len(), 2);
        assert_eq!(
            history[0].content[1].get_text().unwrap(),
            "steer"
        );
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

    #[test]
    fn compacted_record_serialization() {
        let rec = SessionRecord::Compacted {
            summary: "User asked to implement feature X. Files changed: src/foo.rs.".into(),
            tail_turns: 0,
            ts: 100,
        };
        let json = serde_json::to_string(&rec).unwrap();
        assert!(json.contains("\"kind\":\"compacted\""), "got: {json}");
        assert!(json.contains("feature X"), "got: {json}");

        // Round-trip
        let back: SessionRecord = serde_json::from_str(&json).unwrap();
        match back {
            SessionRecord::Compacted { summary, tail_turns, ts } => {
                assert_eq!(summary, "User asked to implement feature X. Files changed: src/foo.rs.");
                assert_eq!(tail_turns, 0);
                assert_eq!(ts, 100);
            }
            _ => panic!("expected Compacted, got {:?}", back),
        }
    }

    #[test]
    fn continuation_judge_record_serialization() {
        // The judge decision is transparent to the user (UI renders nothing) but
        // MUST be auditable in the JSONL — guard the on-disk shape.
        let rec = SessionRecord::ContinuationJudge {
            verdict: "continue".into(),
            nudged: true,
            streak: 1,
            ts: 42,
        };
        let json = serde_json::to_string(&rec).unwrap();
        assert!(json.contains("\"kind\":\"continuation_judge\""), "got: {json}");
        assert!(json.contains("\"verdict\":\"continue\""), "got: {json}");
        assert!(json.contains("\"nudged\":true"), "got: {json}");

        let back: SessionRecord = serde_json::from_str(&json).unwrap();
        match back {
            SessionRecord::ContinuationJudge { verdict, nudged, streak, ts } => {
                assert_eq!(verdict, "continue");
                assert!(nudged);
                assert_eq!(streak, 1);
                assert_eq!(ts, 42);
            }
            _ => panic!("expected ContinuationJudge, got {:?}", back),
        }
    }

    #[test]
    fn status_record_serialization() {
        let rec = SessionRecord::Status {
            session_id: "s1".into(),
            total_input_tokens: 1500,
            total_output_tokens: 300,
            context_tokens: None,
            total_cost: Some(0.0045),
            total_cost_input: None,
            total_cost_output: None,
            total_cost_cache_read: None,
            ts: 200,
        };
        let json = serde_json::to_string(&rec).unwrap();
        assert!(json.contains("\"kind\":\"status\""), "got: {json}");
        assert!(json.contains("0.0045"), "got: {json}");

        let back: SessionRecord = serde_json::from_str(&json).unwrap();
        match back {
            SessionRecord::Status { total_input_tokens, total_output_tokens, total_cost, .. } => {
                assert_eq!(total_input_tokens, 1500);
                assert_eq!(total_output_tokens, 300);
                assert_eq!(total_cost, Some(0.0045));
            }
            _ => panic!("expected Status, got {:?}", back),
        }
    }

    #[test]
    fn old_format_records_still_deserialize() {
        // Lines written before tail_turns / context_tokens existed must load.
        let old_compacted = r#"{"kind":"compacted","summary":"old summary","ts":1}"#;
        match serde_json::from_str::<SessionRecord>(old_compacted).unwrap() {
            SessionRecord::Compacted { summary, tail_turns, .. } => {
                assert_eq!(summary, "old summary");
                assert_eq!(tail_turns, 0);
            }
            other => panic!("expected Compacted, got {other:?}"),
        }
        let old_status = r#"{"kind":"status","session_id":"s1","total_input_tokens":10,"total_output_tokens":5,"total_cost":0.01,"ts":2}"#;
        match serde_json::from_str::<SessionRecord>(old_status).unwrap() {
            SessionRecord::Status { context_tokens, total_input_tokens, .. } => {
                assert_eq!(context_tokens, None);
                assert_eq!(total_input_tokens, 10);
            }
            other => panic!("expected Status, got {other:?}"),
        }
    }

    #[test]
    fn last_context_tokens_reads_most_recent_status() {
        let recs = vec![
            SessionRecord::Status {
                session_id: "s1".into(),
                total_input_tokens: 10,
                total_output_tokens: 5,
                total_cost: None,
                total_cost_input: None,
                total_cost_output: None,
                total_cost_cache_read: None,
                context_tokens: Some(9000),
                ts: 1,
            },
            SessionRecord::Status {
                session_id: "s1".into(),
                total_input_tokens: 20,
                total_output_tokens: 10,
                total_cost: None,
                total_cost_input: None,
                total_cost_output: None,
                total_cost_cache_read: None,
                context_tokens: Some(1500),
                ts: 2,
            },
        ];
        assert_eq!(last_context_tokens(&recs), Some(1500));
        assert_eq!(last_context_tokens(&[]), None);
    }

    fn user_turn(text: &str, ts: u64) -> SessionRecord {
        SessionRecord::Turn {
            message: Message { role: "user".into(), content: vec![ContentBlock::text(text)] },
            ts,
        }
    }

    fn assistant_turn(text: &str, ts: u64) -> SessionRecord {
        SessionRecord::Turn {
            message: Message { role: "assistant".into(), content: vec![ContentBlock::text(text)] },
            ts,
        }
    }

    fn tool_result_turn(ts: u64) -> SessionRecord {
        SessionRecord::Turn {
            message: Message {
                role: "user".into(),
                content: vec![ContentBlock::tool_result("t1", "result")],
            },
            ts,
        }
    }

    #[test]
    fn history_with_tail_turns_keeps_recent_exchanges_verbatim() {
        let recs = vec![
            user_turn("old question", 1),
            assistant_turn("old answer", 2),
            user_turn("recent question", 3),
            assistant_turn("recent answer", 4),
            SessionRecord::Compacted { summary: "S".into(), tail_turns: 2, ts: 5 },
            user_turn("post-compact", 6),
        ];
        let history = history_from_records(&recs);
        // summary + 2 tail turns + 1 post-compact
        assert_eq!(history.len(), 4);
        assert!(history[0].content[0].get_text().unwrap().starts_with("[Contexto anterior compactado]"));
        assert_eq!(history[1].content[0].get_text().unwrap(), "recent question");
        assert_eq!(history[2].content[0].get_text().unwrap(), "recent answer");
        assert_eq!(history[3].content[0].get_text().unwrap(), "post-compact");
    }

    #[test]
    fn tail_never_starts_on_assistant_or_tool_result() {
        // tail_turns=2 lands on (tool_result, assistant) — must expand back to
        // the real user turn so the API sees a valid alternating history.
        let recs = vec![
            user_turn("q1", 1),
            assistant_turn("calling tool", 2),
            tool_result_turn(3),
            assistant_turn("final answer", 4),
            SessionRecord::Compacted { summary: "S".into(), tail_turns: 2, ts: 5 },
        ];
        let start = tail_start_index(&recs, 4, 2);
        assert_eq!(start, 0, "tail must expand back to the real user turn q1");
        let history = history_from_records(&recs);
        assert_eq!(history.len(), 5); // summary + all 4 turns
        assert_eq!(history[1].content[0].get_text().unwrap(), "q1");
    }

    #[test]
    fn tail_dropped_when_no_user_turn_exists() {
        let recs = vec![
            assistant_turn("orphan assistant", 1),
            SessionRecord::Compacted { summary: "S".into(), tail_turns: 1, ts: 2 },
        ];
        assert_eq!(tail_start_index(&recs, 1, 1), 1, "no user turn — tail dropped");
        let history = history_from_records(&recs);
        assert_eq!(history.len(), 1, "only the summary message");
    }

    #[test]
    fn history_from_records_with_compacted_returns_only_messages_after() {
        let recs = vec![
            SessionRecord::Turn {
                message: Message { role: "user".into(), content: vec![ContentBlock::text("hello")] },
                ts: 1,
            },
            SessionRecord::Turn {
                message: Message { role: "assistant".into(), content: vec![ContentBlock::text("hi there")] },
                ts: 2,
            },
            SessionRecord::Compacted {
                summary: "User greeted the agent.".into(),
                tail_turns: 0,
                ts: 3,
            },
            SessionRecord::Turn {
                message: Message { role: "user".into(), content: vec![ContentBlock::text("new question")] },
                ts: 4,
            },
            SessionRecord::Turn {
                message: Message { role: "assistant".into(), content: vec![ContentBlock::text("new answer")] },
                ts: 5,
            },
        ];
        let history = history_from_records(&recs);
        // Should have: 1 summary user message + 2 turns after compacted
        assert_eq!(history.len(), 3, "should have summary + 2 post-compact messages");
        assert_eq!(
            history[0].content[0].get_text().unwrap(),
            "[Contexto anterior compactado]\nUser greeted the agent."
        );
        assert_eq!(history[1].role, "user");
        assert_eq!(history[1].content[0].get_text().unwrap(), "new question");
        assert_eq!(history[2].role, "assistant");
        assert_eq!(history[2].content[0].get_text().unwrap(), "new answer");
    }

    #[test]
    fn history_from_records_without_compacted_returns_all() {
        let recs = vec![
            SessionRecord::Turn {
                message: Message { role: "user".into(), content: vec![ContentBlock::text("q1")] },
                ts: 1,
            },
            SessionRecord::Turn {
                message: Message { role: "assistant".into(), content: vec![ContentBlock::text("a1")] },
                ts: 2,
            },
        ];
        let history = history_from_records(&recs);
        assert_eq!(history.len(), 2);
        assert_eq!(history[0].content[0].get_text().unwrap(), "q1");
        assert_eq!(history[1].content[0].get_text().unwrap(), "a1");
    }

    #[test]
    fn history_from_records_multiple_compacted_uses_last() {
        let recs = vec![
            // Before first compact
            SessionRecord::Turn {
                message: Message { role: "user".into(), content: vec![ContentBlock::text("old")] },
                ts: 1,
            },
            SessionRecord::Compacted { summary: "First compact".into(), tail_turns: 0, ts: 2 },
            // Between compacts
            SessionRecord::Turn {
                message: Message { role: "user".into(), content: vec![ContentBlock::text("middle")] },
                ts: 3,
            },
            SessionRecord::Compacted { summary: "Second compact".into(), tail_turns: 0, ts: 4 },
            // After last compact
            SessionRecord::Turn {
                message: Message { role: "user".into(), content: vec![ContentBlock::text("recent")] },
                ts: 5,
            },
        ];
        let history = history_from_records(&recs);
        // Summary from last compact + the turn after it
        assert_eq!(history.len(), 2, "should use LAST compact's summary");
        assert!(
            history[0].content[0].get_text().unwrap().contains("Second compact"),
            "summary should be from the last compact"
        );
        assert_eq!(history[1].content[0].get_text().unwrap(), "recent");
    }

    #[test]
    fn cumulative_stats_from_status_records() {
        let recs = vec![
            SessionRecord::Status {
                session_id: "s1".into(),
                total_input_tokens: 1000,
                total_output_tokens: 200,
                context_tokens: None,
                total_cost: Some(0.003),
                total_cost_input: None,
                total_cost_output: None,
                total_cost_cache_read: None,
                ts: 10,
            },
            SessionRecord::Status {
                session_id: "s1".into(),
                total_input_tokens: 2500,
                total_output_tokens: 500,
                context_tokens: None,
                total_cost: Some(0.009),
                total_cost_input: None,
                total_cost_output: None,
                total_cost_cache_read: None,
                ts: 20,
            },
        ];
        let (input, output, cost, ..) = cumulative_stats(&recs);
        assert_eq!(input, 2500, "should be the last Status value");
        assert_eq!(output, 500);
        assert_eq!(cost, Some(0.009));
    }

    #[test]
    fn cumulative_stats_no_status_records() {
        let recs = vec![
            SessionRecord::Meta { session_id: "s1".into(), created_at: 1, workspace: None },
        ];
        let (input, output, cost, ..) = cumulative_stats(&recs);
        assert_eq!(input, 0);
        assert_eq!(output, 0);
        assert_eq!(cost, None);
    }

    #[test]
    fn cumulative_stats_without_cost_returns_none() {
        let recs = vec![
            SessionRecord::Status {
                session_id: "s1".into(),
                total_input_tokens: 500,
                total_output_tokens: 100,
                context_tokens: None,
                total_cost: None,
                total_cost_input: None,
                total_cost_output: None,
                total_cost_cache_read: None,
                ts: 5,
            },
        ];
        let (_, _, cost, ..) = cumulative_stats(&recs);
        assert_eq!(cost, None);
    }
}
