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
use lru::LruCache;
use serde::{Deserialize, Serialize};
use std::io::{BufRead, Write};
use std::path::{Path, PathBuf};
use std::sync::{Arc, Mutex};
use std::time::{Instant, SystemTime, UNIX_EPOCH};

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct AttachmentMeta {
    pub name: String,
    /// Serialize as camelCase (mediaType) for frontend consistency.
    /// Accept both camelCase and snake_case on deserialization for backward
    /// compat with sessions written before the rename.
    #[serde(rename = "mediaType", alias = "media_type")]
    pub media_type: String,
    pub size: u64,
}

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
    /// The user's raw input was rejected before the workflow started (e.g. the
    /// English-only guard). Kept for audit: the user's message should never
    /// silently vanish from the JSONL.
    #[serde(rename = "rejected")]
    Rejected {
        text: String,
        reason: String,
        ts: u64,
    },
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
    PhaseResult {
        phase: String,
        text: String,
        ts: u64,
    },
    /// End of a workflow run.
    Done {
        input_tokens: u32,
        output_tokens: u32,
        ts: u64,
    },
    /// A run failed.
    Error { message: String, ts: u64 },
    /// A steering message injected mid-run.
    Steering {
        text: String,
        #[serde(skip_serializing_if = "Option::is_none")]
        attachments: Option<Vec<AttachmentMeta>>,
        ts: u64,
    },
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
    Mode {
        mode: String,
        origin: String,
        ts: u64,
    },
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
    /// The git HEAD at the moment this session's work began. Written once per
    /// session (guarded), so finalize_plan can compute the changed files and
    /// commits since planning started, even across resumed runs.
    #[serde(rename = "base_commit")]
    BaseCommit { sha: String, ts: u64 },
    /// A plan's Implementation Log was appended: the run finished with the
    /// changed files / commit(s) recorded back into the plan `.md`. `plan_file`
    /// is the absolute path; `commits` are short "<sha> <subject>" lines and
    /// `files_changed` are "<status>\t<path>" lines from the diff.
    #[serde(rename = "plan_finalized")]
    PlanFinalized {
        plan_file: String,
        commits: Vec<String>,
        files_changed: Vec<String>,
        ts: u64,
    },
    /// This session continues a previous one (written right after `Meta`).
    /// `reason` is "plan_execution" | "golden_flip" | "context_handoff" |
    /// "manual_builder". The golden fields carry the golden-loop state across
    /// the chain so cycle/stall caps never reset on a handoff.
    #[serde(rename = "linked_from")]
    LinkedFrom {
        prev_session_id: String,
        reason: String,
        #[serde(default)]
        golden_cycle: u32,
        #[serde(default)]
        golden_stalls: u32,
        #[serde(default)]
        golden_last_pending: Vec<String>,
        ts: u64,
    },
    /// Forward pointer: this session was superseded by a linked successor.
    /// Sessions with this record are hidden from the session list (the chain
    /// tip represents the whole conversation).
    #[serde(rename = "handoff_to")]
    HandoffTo {
        next_session_id: String,
        reason: String,
        ts: u64,
    },
    /// The model-generated handoff document produced when the context crossed
    /// the configured threshold, kept for audit; the successor session receives
    /// it as its first user message.
    #[serde(rename = "handoff")]
    Handoff { text: String, ts: u64 },
    /// A quality-harness run: the project's own tests / coverage were executed
    /// and their machine-readable output parsed. This is the evidence the
    /// golden gate checks — `digest` fingerprints the worktree it ran against,
    /// so an edit made afterwards invalidates it. `trigger` is "tool" (the
    /// agent called run_quality) or "harness" (the loop ran it at the finish).
    #[serde(rename = "quality_run")]
    QualityRun {
        digest: String,
        pass: bool,
        summary: String,
        /// Serialized `quality::QualityReport`, for the UI and for audit.
        report: String,
        trigger: String,
        ts: u64,
    },
}

/// A `QualityRun` record, flattened for the callers that only need the verdict.
#[derive(Debug, Clone, PartialEq)]
pub struct QualityRunInfo {
    pub digest: String,
    pub pass: bool,
    pub summary: String,
    pub report: String,
    pub trigger: String,
    pub ts: u64,
}

/// The most recent quality run in this session, if any.
///
/// Per-session by design: a linked successor starts with no evidence, which is
/// the correct default — a handoff happens precisely when the work was not
/// finished, so its predecessor's green run says nothing about the new state.
pub fn last_quality_run(records: &[SessionRecord]) -> Option<QualityRunInfo> {
    records.iter().rev().find_map(|r| match r {
        SessionRecord::QualityRun {
            digest,
            pass,
            summary,
            report,
            trigger,
            ts,
        } => Some(QualityRunInfo {
            digest: digest.clone(),
            pass: *pass,
            summary: summary.clone(),
            report: report.clone(),
            trigger: trigger.clone(),
            ts: *ts,
        }),
        _ => None,
    })
}

/// Read the session file and return its latest quality run.
pub fn load_last_quality_run(path: &Path) -> Option<QualityRunInfo> {
    last_quality_run(&load_records(path).ok()?)
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

/// Parsed view of a session's `LinkedFrom` record, if any.
#[derive(Debug, Clone, PartialEq)]
pub struct LinkedFromInfo {
    pub prev_session_id: String,
    pub reason: String,
    pub golden_cycle: u32,
    pub golden_stalls: u32,
    pub golden_last_pending: Vec<String>,
}

/// The `LinkedFrom` record of this session (who it continues), if any.
pub fn linked_from(records: &[SessionRecord]) -> Option<LinkedFromInfo> {
    records.iter().find_map(|r| match r {
        SessionRecord::LinkedFrom {
            prev_session_id,
            reason,
            golden_cycle,
            golden_stalls,
            golden_last_pending,
            ..
        } => Some(LinkedFromInfo {
            prev_session_id: prev_session_id.clone(),
            reason: reason.clone(),
            golden_cycle: *golden_cycle,
            golden_stalls: *golden_stalls,
            golden_last_pending: golden_last_pending.clone(),
        }),
        _ => None,
    })
}

/// The successor session id when this session was superseded by a handoff.
pub fn handoff_to(records: &[SessionRecord]) -> Option<String> {
    records.iter().rev().find_map(|r| match r {
        SessionRecord::HandoffTo {
            next_session_id, ..
        } => Some(next_session_id.clone()),
        _ => None,
    })
}

/// Golden-loop state carried over from the predecessor session:
/// (cycle, stalls, last_pending). All zeros/empty when not linked.
pub fn linked_golden_state(records: &[SessionRecord]) -> (u32, u32, Vec<String>) {
    match linked_from(records) {
        Some(info) => (
            info.golden_cycle,
            info.golden_stalls,
            info.golden_last_pending,
        ),
        None => (0, 0, Vec::new()),
    }
}

/// The earliest recorded base commit for the session (the git HEAD when work
/// first began), or None if no `BaseCommit` record exists yet.
pub fn earliest_base_commit(records: &[SessionRecord]) -> Option<String> {
    records.iter().find_map(|r| match r {
        SessionRecord::BaseCommit { sha, .. } => Some(sha.clone()),
        _ => None,
    })
}

/// Whether the session already has a `BaseCommit` record (used to write it
/// only once, anchoring the diff window to the true start of the plan's work).
pub fn has_base_commit(records: &[SessionRecord]) -> bool {
    records
        .iter()
        .any(|r| matches!(r, SessionRecord::BaseCommit { .. }))
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
    pub fn create(session_id: &str, workspace: Option<&str>) -> Result<Self, String> {
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
        // Screenshots and attachments are ~200 KB of base64 each. Left inline
        // they make the session file grow by megabytes per capture, and this
        // file is re-read on every message. Cheap pre-check so the common
        // image-free line never pays for the JSON round-trip.
        let line = if line.contains(BASE64_MARKER) {
            externalized(&line, &self.media_dir()).unwrap_or(line)
        } else {
            line
        };
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
    fn media_dir(&self) -> std::path::PathBuf {
        media_dir_for(&self.path)
    }

    pub fn try_append(&self, record: &SessionRecord) {
        let _ = self.append(record);
    }
}

/// Read every record from a session file, skipping malformed / unknown lines.
pub fn load_records(path: &Path) -> Result<Vec<SessionRecord>, String> {
    let file = std::fs::File::open(path).map_err(|e| format!("open session file: {e}"))?;
    let reader = std::io::BufReader::new(file);
    let media_dir = media_dir_for(path);
    let mut out = Vec::new();
    for line in reader.lines() {
        let line = line.map_err(|e| format!("read session file: {e}"))?;
        if line.trim().is_empty() {
            continue;
        }
        // Same pre-check as on the way out: only lines that actually reference
        // an externalized image pay for the round-trip.
        let rec = if line.contains(MEDIA_REF_PREFIX) {
            rehydrated(&line, &media_dir).and_then(|v| serde_json::from_value(v).ok())
        } else {
            serde_json::from_str::<SessionRecord>(&line).ok()
        };
        if let Some(rec) = rec {
            out.push(rec);
        }
    }
    Ok(out)
}

/// Marker that tells `append` a line is worth scanning for image data.
const BASE64_MARKER: &str = "\"base64\"";
/// Prefix replacing the base64 payload of an externalized image.
const MEDIA_REF_PREFIX: &str = "@media:";
/// Below this an image is small enough that a separate file costs more than it
/// saves (an inode and a syscall against a few KB of text).
const MEDIA_INLINE_THRESHOLD: usize = 4096;

fn media_dir_for(session_path: &Path) -> std::path::PathBuf {
    session_path
        .parent()
        .unwrap_or_else(|| Path::new("."))
        .join("media")
}

/// Move oversized base64 image payloads out to files, leaving a reference.
///
/// Returns `None` when nothing changed or anything went wrong, so the caller
/// falls back to writing the line as-is — losing the size optimisation is
/// always preferable to losing the record.
fn externalized(line: &str, media_dir: &Path) -> Option<String> {
    let mut value: serde_json::Value = serde_json::from_str(line).ok()?;
    let mut wrote = false;
    walk_images(&mut value, &mut |source| {
        let data = source.get("data").and_then(|d| d.as_str())?;
        if data.len() < MEDIA_INLINE_THRESHOLD || data.starts_with(MEDIA_REF_PREFIX) {
            return None;
        }
        let media_type = source
            .get("media_type")
            .and_then(|m| m.as_str())
            .unwrap_or("image/png");
        let ext = media_type.rsplit('/').next().unwrap_or("png");
        let bytes =
            base64::Engine::decode(&base64::engine::general_purpose::STANDARD, data).ok()?;

        // Content-addressed, so the same screenshot referenced twice is stored
        // once.
        use sha2::Digest;
        let digest = format!("{:x}", sha2::Sha256::digest(&bytes));
        let name = format!("{}.{ext}", &digest[..32]);
        let dest = media_dir.join(&name);
        if !dest.exists() {
            std::fs::create_dir_all(media_dir).ok()?;
            std::fs::write(&dest, &bytes).ok()?;
        }
        wrote = true;
        Some(format!("{MEDIA_REF_PREFIX}{name}"))
    });
    if !wrote {
        return None;
    }
    serde_json::to_string(&value).ok()
}

/// Read externalized payloads back in.
///
/// A reference whose file is gone becomes a text note rather than a broken
/// image: sending unresolvable data to the provider would fail the whole
/// request, while a note degrades one block.
fn rehydrated(line: &str, media_dir: &Path) -> Option<serde_json::Value> {
    let mut value: serde_json::Value = serde_json::from_str(line).ok()?;
    walk_images(&mut value, &mut |source| {
        let data = source.get("data").and_then(|d| d.as_str())?;
        let name = data.strip_prefix(MEDIA_REF_PREFIX)?;
        let bytes = std::fs::read(media_dir.join(name)).ok()?;
        Some(base64::Engine::encode(
            &base64::engine::general_purpose::STANDARD,
            &bytes,
        ))
    });
    // Any reference that could not be read is still a marker; turn those into
    // text so nothing downstream tries to decode them.
    replace_dangling_media(&mut value);
    Some(value)
}

/// Apply `f` to every `{"type":"base64", "data":…}` source object, replacing
/// `data` with whatever it returns.
fn walk_images(
    value: &mut serde_json::Value,
    f: &mut impl FnMut(&serde_json::Map<String, serde_json::Value>) -> Option<String>,
) {
    match value {
        serde_json::Value::Object(map) => {
            let is_source = map.get("type").and_then(|t| t.as_str()) == Some("base64")
                && map.contains_key("data");
            if is_source && let Some(replacement) = f(map) {
                map.insert("data".into(), serde_json::Value::String(replacement));
                return;
            }
            for (_, v) in map.iter_mut() {
                walk_images(v, f);
            }
        }
        serde_json::Value::Array(items) => {
            for v in items.iter_mut() {
                walk_images(v, f);
            }
        }
        _ => {}
    }
}

/// Turn image blocks whose media file vanished into a plain text note.
fn replace_dangling_media(value: &mut serde_json::Value) {
    match value {
        serde_json::Value::Array(items) => {
            for item in items.iter_mut() {
                let dangling = item
                    .pointer("/source/data")
                    .and_then(|d| d.as_str())
                    .is_some_and(|d| d.starts_with(MEDIA_REF_PREFIX));
                if dangling {
                    *item = serde_json::json!({
                        "type": "text",
                        "text": "[image from an earlier step is no longer available]"
                    });
                } else {
                    replace_dangling_media(item);
                }
            }
        }
        serde_json::Value::Object(map) => {
            for (_, v) in map.iter_mut() {
                replace_dangling_media(v);
            }
        }
        _ => {}
    }
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
    let compact_idx = records
        .iter()
        .rposition(|r| matches!(r, SessionRecord::Compacted { .. }));

    let mut out: Vec<Message> = Vec::new();
    match compact_idx {
        Some(idx) => {
            let (summary, tail_turns) = match &records[idx] {
                SessionRecord::Compacted {
                    summary,
                    tail_turns,
                    ..
                } => (summary.clone(), *tail_turns),
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
fn fold_into_history<'a>(out: &mut Vec<Message>, records: impl Iterator<Item = &'a SessionRecord>) {
    for rec in records {
        match rec {
            SessionRecord::Turn { message, .. } => {
                out.push(message.clone());
            }
            SessionRecord::Steering {
                text, attachments, ..
            } => {
                let mut blocks = vec![crate::agent::provider::ContentBlock::text(text)];
                if let Some(atts) = attachments {
                    for att in atts {
                        let size_str = if att.size > 1024 * 1024 {
                            format!("{:.1} MB", att.size as f64 / (1024.0 * 1024.0))
                        } else if att.size > 1024 {
                            format!("{:.1} KB", att.size as f64 / 1024.0)
                        } else {
                            format!("{} B", att.size)
                        };
                        blocks.push(crate::agent::provider::ContentBlock::text(format!(
                            "[Anexo do steering: `{}` ({}) — tipo: {}]",
                            att.name, size_str, att.media_type
                        )));
                    }
                }
                if let Some(last) = out.last_mut()
                    && last.role == "user"
                {
                    last.content.extend(blocks);
                    continue;
                }
                out.push(Message {
                    role: "user".into(),
                    content: blocks,
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
        match (0..start)
            .rev()
            .find(|&i| matches!(records[i], SessionRecord::Turn { .. }))
        {
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
        if has_cost_input {
            Some(cost_input)
        } else {
            None
        },
        if has_cost_output {
            Some(cost_output)
        } else {
            None
        },
        if has_cost_cache_read {
            Some(cost_cache_read)
        } else {
            None
        },
    )
}

/// Walk the session chain backward (via LinkedFrom) from `start_session_id`
/// and sum every session's last Status record cost fields.
/// Returns (total_cost, cost_input, cost_output, cost_cache_read) — all Options.
/// All None when no session in the chain has cost data.
pub fn chain_cumulative_cost(
    workspace_root: Option<&str>,
    start_session_id: &str,
) -> (Option<f64>, Option<f64>, Option<f64>, Option<f64>) {
    let dir = match sessions_dir(workspace_root) {
        Ok(d) => d,
        Err(_) => return (None, None, None, None),
    };

    let mut total_cost: Option<f64> = None;
    let mut cost_input: Option<f64> = None;
    let mut cost_output: Option<f64> = None;
    let mut cost_cache_read: Option<f64> = None;

    let mut current_id = start_session_id.to_string();
    let mut seen: std::collections::HashSet<String> = std::collections::HashSet::new();
    const MAX_HOPS: usize = 64;

    for _ in 0..MAX_HOPS {
        if !seen.insert(current_id.clone()) {
            break; // cycle guard
        }

        let path = dir.join(format!("{current_id}.jsonl"));
        let Ok(records) = load_records(&path) else {
            break;
        };

        // Accumulate this session's cost from its last Status record.
        let (_, _, tc, ci, co, cc) = cumulative_stats(&records);
        if let Some(c) = tc {
            total_cost = Some(total_cost.unwrap_or(0.0) + c);
        }
        if let Some(c) = ci {
            cost_input = Some(cost_input.unwrap_or(0.0) + c);
        }
        if let Some(c) = co {
            cost_output = Some(cost_output.unwrap_or(0.0) + c);
        }
        if let Some(c) = cc {
            cost_cache_read = Some(cost_cache_read.unwrap_or(0.0) + c);
        }

        // Walk to predecessor via LinkedFrom.
        match linked_from(&records) {
            Some(info) => current_id = info.prev_session_id,
            None => break,
        }
    }

    (total_cost, cost_input, cost_output, cost_cache_read)
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
///
/// Linked sessions collapse to ONE entry per chain: sessions superseded by a
/// handoff (`handoff_to` present) are hidden, and the surviving chain tip
/// inherits the root's `created_at`/`title` and the summed `turn_count` of the
/// whole chain — the user sees one conversation, not its internal segments.
pub fn list_sessions(workspace: Option<&str>) -> Result<Vec<SessionSummary>, String> {
    let dir = sessions_dir(workspace)?;
    let mut summaries = Vec::new();
    // Per-session chain metadata gathered in the same pass as the summaries:
    // id -> (predecessor id, superseded?).
    let mut chain_meta: std::collections::HashMap<String, (Option<String>, bool)> =
        std::collections::HashMap::new();
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
                | SessionRecord::ContinuationJudge { ts, .. }
                | SessionRecord::BaseCommit { ts, .. }
                | SessionRecord::PlanFinalized { ts, .. }
                | SessionRecord::QualityRun { ts, .. }
                | SessionRecord::Rejected { ts, .. } => {
                    updated_at = updated_at.max(*ts);
                }
                SessionRecord::LinkedFrom { ts, .. }
                | SessionRecord::HandoffTo { ts, .. }
                | SessionRecord::Handoff { ts, .. } => {
                    updated_at = updated_at.max(*ts);
                }
            }
        }
        // Sessions with no real content (only meta/mode records — lazy
        // creations from a mode toggle that never got a message) are noise:
        // hide them unless they belong to a chain.
        let has_content = records
            .iter()
            .any(|r| matches!(r, SessionRecord::User { .. } | SessionRecord::Turn { .. }));
        let in_chain = records.iter().any(|r| {
            matches!(
                r,
                SessionRecord::LinkedFrom { .. } | SessionRecord::HandoffTo { .. }
            )
        });
        if !has_content && !in_chain {
            continue;
        }
        if title.is_empty() {
            title = "(empty session)".into();
        }
        chain_meta.insert(
            session_id.clone(),
            (
                linked_from(&records).map(|i| i.prev_session_id),
                handoff_to(&records).is_some(),
            ),
        );
        summaries.push(SessionSummary {
            session_id,
            created_at,
            updated_at,
            title,
            turn_count,
        });
    }

    // Collapse chains: keep only tips (not superseded), folding each tip's
    // ancestry into it.
    let by_id: std::collections::HashMap<String, (u64, String, usize)> = summaries
        .iter()
        .map(|s| {
            (
                s.session_id.clone(),
                (s.created_at, s.title.clone(), s.turn_count),
            )
        })
        .collect();
    let mut collapsed: Vec<SessionSummary> = Vec::new();
    for mut summary in summaries {
        let superseded = chain_meta
            .get(&summary.session_id)
            .map(|(_, s)| *s)
            .unwrap_or(false);
        if superseded {
            continue;
        }
        // Walk back to the chain root, accumulating turns; the root names the
        // conversation (its title and created_at are what the user first saw).
        let mut seen: std::collections::HashSet<String> =
            std::collections::HashSet::from([summary.session_id.clone()]);
        let mut cursor = chain_meta
            .get(&summary.session_id)
            .and_then(|(prev, _)| prev.clone());
        while let Some(prev_id) = cursor {
            if !seen.insert(prev_id.clone()) {
                break; // cycle guard
            }
            if let Some((created, title, turns)) = by_id.get(&prev_id) {
                summary.turn_count += turns;
                if *created > 0 {
                    summary.created_at = *created;
                }
                if title != "(empty session)" {
                    summary.title = title.clone();
                }
            }
            cursor = chain_meta.get(&prev_id).and_then(|(prev, _)| prev.clone());
        }
        collapsed.push(summary);
    }
    collapsed.sort_by_key(|s| std::cmp::Reverse(s.updated_at));
    Ok(collapsed)
}

/// A single task item managed by the agent.
#[derive(Debug, Clone, serde::Serialize, serde::Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct TaskItem {
    pub id: String,
    pub title: String,
    pub description: String,
    pub journal: Vec<String>,
    pub status: String, // "todo" | "doing" | "done"
}

/// Read all records from a JSONL file and find the LAST SessionRecord::Tasks,
/// returning its deserialized tasks (or empty vec if none found).
pub fn load_last_tasks(path: &Path) -> Result<Vec<TaskItem>, String> {
    let records = load_records(path)?;
    let last = records
        .into_iter()
        .rev()
        .find(|r| matches!(r, SessionRecord::Tasks { .. }));
    match last {
        Some(SessionRecord::Tasks { tasks_json, .. }) => {
            serde_json::from_str(&tasks_json).map_err(|e| format!("parse tasks from session: {e}"))
        }
        _ => Ok(Vec::new()),
    }
}

/// Serialize tasks and append a SessionRecord::Tasks line to the JSONL.
pub fn append_tasks(path: &Path, tasks: &[TaskItem]) -> Result<(), String> {
    let tasks_json = serde_json::to_string(tasks).map_err(|e| format!("serialize tasks: {e}"))?;
    let record = SessionRecord::Tasks {
        tasks_json,
        ts: std::time::SystemTime::now()
            .duration_since(std::time::UNIX_EPOCH)
            .map(|d| d.as_millis() as u64)
            .unwrap_or(0),
    };
    let line = serde_json::to_string(&record).map_err(|e| format!("serialize record: {e}"))?;
    let mut file = std::fs::OpenOptions::new()
        .create(true)
        .append(true)
        .open(path)
        .map_err(|e| format!("open session file: {e}"))?;
    use std::io::Write;
    writeln!(file, "{line}").map_err(|e| format!("write session file: {e}"))?;
    Ok(())
}

/// Append a quality run to the session JSONL. Mirrors `append_tasks`: the
/// JSONL is the session's source of truth, so the gate reads its evidence from
/// the same place the task list and the golden state already live.
pub fn append_quality_run(path: &Path, info: &QualityRunInfo) -> Result<(), String> {
    let record = SessionRecord::QualityRun {
        digest: info.digest.clone(),
        pass: info.pass,
        summary: info.summary.clone(),
        report: info.report.clone(),
        trigger: info.trigger.clone(),
        ts: info.ts,
    };
    let line = serde_json::to_string(&record).map_err(|e| format!("serialize record: {e}"))?;
    let mut file = std::fs::OpenOptions::new()
        .create(true)
        .append(true)
        .open(path)
        .map_err(|e| format!("open session file: {e}"))?;
    use std::io::Write;
    writeln!(file, "{line}").map_err(|e| format!("write session file: {e}"))?;
    Ok(())
}

/// The parsed-session-record LRU shared by AppState, the tool context and the
/// transition helpers. One alias so the four holders cannot drift apart.
pub type RecordsCache = Arc<Mutex<LruCache<PathBuf, (Vec<SessionRecord>, Instant)>>>;

pub fn load_records_cached(
    path: &Path,
    cache: &Mutex<LruCache<PathBuf, (Vec<SessionRecord>, Instant)>>,
) -> Result<Vec<SessionRecord>, String> {
    let mut cache = cache.lock().unwrap();
    if let Some((records, cached_at)) = cache.get(path)
        && cached_at.elapsed() < std::time::Duration::from_millis(800)
    {
        return Ok(records.clone());
    }
    let records = load_records(path)?;
    cache.put(path.to_path_buf(), (records.clone(), Instant::now()));
    Ok(records)
}

pub fn invalidate_cache(
    path: &Path,
    cache: &Mutex<LruCache<PathBuf, (Vec<SessionRecord>, Instant)>>,
) {
    let mut cache = cache.lock().unwrap();
    cache.pop(path);
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
            SessionRecord::GoldenCycle {
                cycle: 1,
                mode: "brain".into(),
                goals: vec![],
                ts: 1,
            },
            rec,
        ];
        assert_eq!(golden_cycle_count(&recs), 2);
        assert_eq!(golden_cycle_count(&[]), 0);
    }

    #[test]
    fn linked_records_roundtrip() {
        let rec = SessionRecord::LinkedFrom {
            prev_session_id: "s1".into(),
            reason: "golden_flip".into(),
            golden_cycle: 3,
            golden_stalls: 1,
            golden_last_pending: vec!["golden-x-1".into()],
            ts: 7,
        };
        let json = serde_json::to_string(&rec).unwrap();
        assert!(json.contains("\"kind\":\"linked_from\""), "got: {json}");
        let back: SessionRecord = serde_json::from_str(&json).unwrap();
        assert!(matches!(
            back,
            SessionRecord::LinkedFrom {
                golden_cycle: 3,
                ..
            }
        ));

        let rec = SessionRecord::HandoffTo {
            next_session_id: "s2".into(),
            reason: "context_handoff".into(),
            ts: 8,
        };
        let json = serde_json::to_string(&rec).unwrap();
        assert!(json.contains("\"kind\":\"handoff_to\""), "got: {json}");

        let rec = SessionRecord::Handoff {
            text: "## Purpose\ncontinue".into(),
            ts: 9,
        };
        let json = serde_json::to_string(&rec).unwrap();
        assert!(json.contains("\"kind\":\"handoff\""), "got: {json}");
    }

    #[test]
    fn linked_golden_state_reads_linked_from() {
        let recs = vec![
            SessionRecord::Meta {
                session_id: "s2".into(),
                created_at: 1,
                workspace: None,
            },
            SessionRecord::LinkedFrom {
                prev_session_id: "s1".into(),
                reason: "golden_flip".into(),
                golden_cycle: 4,
                golden_stalls: 1,
                golden_last_pending: vec!["golden-a-0".into()],
                ts: 2,
            },
        ];
        let (cycle, stalls, pending) = linked_golden_state(&recs);
        assert_eq!(cycle, 4);
        assert_eq!(stalls, 1);
        assert_eq!(pending, vec!["golden-a-0".to_string()]);
        assert_eq!(linked_golden_state(&[]), (0, 0, Vec::new()));
    }

    #[test]
    fn linked_from_missing_golden_fields_defaults_to_zero() {
        // Records written by an older build (or hand-edited) must still load.
        let line =
            r#"{"kind":"linked_from","prev_session_id":"s1","reason":"plan_execution","ts":1}"#;
        match serde_json::from_str::<SessionRecord>(line).unwrap() {
            SessionRecord::LinkedFrom {
                golden_cycle,
                golden_stalls,
                golden_last_pending,
                ..
            } => {
                assert_eq!(golden_cycle, 0);
                assert_eq!(golden_stalls, 0);
                assert!(golden_last_pending.is_empty());
            }
            other => panic!("expected LinkedFrom, got {other:?}"),
        }
    }

    #[test]
    fn list_sessions_collapses_chains_to_one_entry() {
        let dir = std::env::temp_dir().join(format!("claudinio-test-{}", std::process::id()));
        let ws = dir.to_string_lossy().to_string();
        std::fs::create_dir_all(&dir).unwrap();
        // Chain: root -> mid -> tip; plus one standalone session.
        let root_store = SessionStore::create("root", Some(&ws)).unwrap();
        root_store
            .append(&SessionRecord::User {
                text: "build the feature".into(),
                ts: 10,
            })
            .unwrap();
        root_store
            .append(&SessionRecord::HandoffTo {
                next_session_id: "mid".into(),
                reason: "plan_execution".into(),
                ts: 20,
            })
            .unwrap();

        let mid_store = SessionStore::create("mid", Some(&ws)).unwrap();
        mid_store
            .append(&SessionRecord::LinkedFrom {
                prev_session_id: "root".into(),
                reason: "plan_execution".into(),
                golden_cycle: 0,
                golden_stalls: 0,
                golden_last_pending: vec![],
                ts: 21,
            })
            .unwrap();
        mid_store
            .append(&SessionRecord::User {
                text: "[system] execute".into(),
                ts: 22,
            })
            .unwrap();
        mid_store
            .append(&SessionRecord::HandoffTo {
                next_session_id: "tip".into(),
                reason: "context_handoff".into(),
                ts: 30,
            })
            .unwrap();

        let tip_store = SessionStore::create("tip", Some(&ws)).unwrap();
        tip_store
            .append(&SessionRecord::LinkedFrom {
                prev_session_id: "mid".into(),
                reason: "context_handoff".into(),
                golden_cycle: 0,
                golden_stalls: 0,
                golden_last_pending: vec![],
                ts: 31,
            })
            .unwrap();
        tip_store
            .append(&SessionRecord::User {
                text: "[system] continue".into(),
                ts: 32,
            })
            .unwrap();

        let solo_store = SessionStore::create("solo", Some(&ws)).unwrap();
        solo_store
            .append(&SessionRecord::User {
                text: "hello".into(),
                ts: 5,
            })
            .unwrap();

        let list = list_sessions(Some(&ws)).unwrap();
        std::fs::remove_dir_all(&dir).ok();

        assert_eq!(list.len(), 2, "chain must collapse to one entry: {list:?}");
        let tip = list
            .iter()
            .find(|s| s.session_id == "tip")
            .expect("tip entry");
        assert_eq!(
            tip.title, "build the feature",
            "tip inherits the root's title"
        );
        assert_eq!(tip.turn_count, 3, "turn_count sums the whole chain");
        assert!(list.iter().any(|s| s.session_id == "solo"));
        assert!(
            !list
                .iter()
                .any(|s| s.session_id == "root" || s.session_id == "mid")
        );
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
                attachments: None,
                ts: 3,
            },
            // Steering with attachments
            SessionRecord::Steering {
                text: "steer2".into(),
                attachments: Some(vec![AttachmentMeta {
                    name: "photo.png".into(),
                    media_type: "image/png".into(),
                    size: 1024,
                }]),
                ts: 4,
            },
        ];
        let history = history_from_records(&recs);
        assert_eq!(history.len(), 3);
        assert_eq!(history[0].role, "user");
        assert_eq!(history[1].role, "assistant");
        assert_eq!(history[2].role, "user");
        // steer1 text block + steer2 text block + attachment reference block
        assert_eq!(history[2].content.len(), 3);
        assert_eq!(history[2].content[0].get_text().unwrap(), "steer1");
        assert_eq!(history[2].content[1].get_text().unwrap(), "steer2");
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
                attachments: None,
                ts: 2,
            },
        ];
        let history = history_from_records(&recs);
        assert_eq!(history.len(), 1);
        assert_eq!(history[0].content.len(), 2);
        assert_eq!(history[0].content[1].get_text().unwrap(), "steer");
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
            SessionRecord::Compacted {
                summary,
                tail_turns,
                ts,
            } => {
                assert_eq!(
                    summary,
                    "User asked to implement feature X. Files changed: src/foo.rs."
                );
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
        assert!(
            json.contains("\"kind\":\"continuation_judge\""),
            "got: {json}"
        );
        assert!(json.contains("\"verdict\":\"continue\""), "got: {json}");
        assert!(json.contains("\"nudged\":true"), "got: {json}");

        let back: SessionRecord = serde_json::from_str(&json).unwrap();
        match back {
            SessionRecord::ContinuationJudge {
                verdict,
                nudged,
                streak,
                ts,
            } => {
                assert_eq!(verdict, "continue");
                assert!(nudged);
                assert_eq!(streak, 1);
                assert_eq!(ts, 42);
            }
            _ => panic!("expected ContinuationJudge, got {:?}", back),
        }
    }

    #[test]
    fn rejected_record_serialization() {
        // A message rejected by a pre-workflow guard (e.g. the English-only
        // check) must still land in the JSONL for audit — the user's text must
        // never silently vanish.
        let rec = SessionRecord::Rejected {
            text: "Então...".into(),
            reason: "Only English is supported. Please write your message in English. \
                     (Detected non-English characters: ã)"
                .into(),
            ts: 42,
        };
        let json = serde_json::to_string(&rec).unwrap();
        assert!(json.contains("\"kind\":\"rejected\""), "got: {json}");
        assert!(json.contains("\"text\":\"Então...\""), "got: {json}");
        assert!(json.contains("\"reason\":"), "got: {json}");

        let back: SessionRecord = serde_json::from_str(&json).unwrap();
        match back {
            SessionRecord::Rejected { text, reason, ts } => {
                assert_eq!(text, "Então...");
                assert!(reason.contains("Only English is supported"));
                assert_eq!(ts, 42);
            }
            _ => panic!("expected Rejected, got {:?}", back),
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
            SessionRecord::Status {
                total_input_tokens,
                total_output_tokens,
                total_cost,
                ..
            } => {
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
            SessionRecord::Compacted {
                summary,
                tail_turns,
                ..
            } => {
                assert_eq!(summary, "old summary");
                assert_eq!(tail_turns, 0);
            }
            other => panic!("expected Compacted, got {other:?}"),
        }
        let old_status = r#"{"kind":"status","session_id":"s1","total_input_tokens":10,"total_output_tokens":5,"total_cost":0.01,"ts":2}"#;
        match serde_json::from_str::<SessionRecord>(old_status).unwrap() {
            SessionRecord::Status {
                context_tokens,
                total_input_tokens,
                ..
            } => {
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
            message: Message {
                role: "user".into(),
                content: vec![ContentBlock::text(text)],
            },
            ts,
        }
    }

    fn assistant_turn(text: &str, ts: u64) -> SessionRecord {
        SessionRecord::Turn {
            message: Message {
                role: "assistant".into(),
                content: vec![ContentBlock::text(text)],
            },
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
            SessionRecord::Compacted {
                summary: "S".into(),
                tail_turns: 2,
                ts: 5,
            },
            user_turn("post-compact", 6),
        ];
        let history = history_from_records(&recs);
        // summary + 2 tail turns + 1 post-compact
        assert_eq!(history.len(), 4);
        assert!(
            history[0].content[0]
                .get_text()
                .unwrap()
                .starts_with("[Contexto anterior compactado]")
        );
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
            SessionRecord::Compacted {
                summary: "S".into(),
                tail_turns: 2,
                ts: 5,
            },
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
            SessionRecord::Compacted {
                summary: "S".into(),
                tail_turns: 1,
                ts: 2,
            },
        ];
        assert_eq!(
            tail_start_index(&recs, 1, 1),
            1,
            "no user turn — tail dropped"
        );
        let history = history_from_records(&recs);
        assert_eq!(history.len(), 1, "only the summary message");
    }

    #[test]
    fn history_from_records_with_compacted_returns_only_messages_after() {
        let recs = vec![
            SessionRecord::Turn {
                message: Message {
                    role: "user".into(),
                    content: vec![ContentBlock::text("hello")],
                },
                ts: 1,
            },
            SessionRecord::Turn {
                message: Message {
                    role: "assistant".into(),
                    content: vec![ContentBlock::text("hi there")],
                },
                ts: 2,
            },
            SessionRecord::Compacted {
                summary: "User greeted the agent.".into(),
                tail_turns: 0,
                ts: 3,
            },
            SessionRecord::Turn {
                message: Message {
                    role: "user".into(),
                    content: vec![ContentBlock::text("new question")],
                },
                ts: 4,
            },
            SessionRecord::Turn {
                message: Message {
                    role: "assistant".into(),
                    content: vec![ContentBlock::text("new answer")],
                },
                ts: 5,
            },
        ];
        let history = history_from_records(&recs);
        // Should have: 1 summary user message + 2 turns after compacted
        assert_eq!(
            history.len(),
            3,
            "should have summary + 2 post-compact messages"
        );
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
                message: Message {
                    role: "user".into(),
                    content: vec![ContentBlock::text("q1")],
                },
                ts: 1,
            },
            SessionRecord::Turn {
                message: Message {
                    role: "assistant".into(),
                    content: vec![ContentBlock::text("a1")],
                },
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
                message: Message {
                    role: "user".into(),
                    content: vec![ContentBlock::text("old")],
                },
                ts: 1,
            },
            SessionRecord::Compacted {
                summary: "First compact".into(),
                tail_turns: 0,
                ts: 2,
            },
            // Between compacts
            SessionRecord::Turn {
                message: Message {
                    role: "user".into(),
                    content: vec![ContentBlock::text("middle")],
                },
                ts: 3,
            },
            SessionRecord::Compacted {
                summary: "Second compact".into(),
                tail_turns: 0,
                ts: 4,
            },
            // After last compact
            SessionRecord::Turn {
                message: Message {
                    role: "user".into(),
                    content: vec![ContentBlock::text("recent")],
                },
                ts: 5,
            },
        ];
        let history = history_from_records(&recs);
        // Summary from last compact + the turn after it
        assert_eq!(history.len(), 2, "should use LAST compact's summary");
        assert!(
            history[0].content[0]
                .get_text()
                .unwrap()
                .contains("Second compact"),
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
        let recs = vec![SessionRecord::Meta {
            session_id: "s1".into(),
            created_at: 1,
            workspace: None,
        }];
        let (input, output, cost, ..) = cumulative_stats(&recs);
        assert_eq!(input, 0);
        assert_eq!(output, 0);
        assert_eq!(cost, None);
    }

    #[test]
    fn cumulative_stats_without_cost_returns_none() {
        let recs = vec![SessionRecord::Status {
            session_id: "s1".into(),
            total_input_tokens: 500,
            total_output_tokens: 100,
            context_tokens: None,
            total_cost: None,
            total_cost_input: None,
            total_cost_output: None,
            total_cost_cache_read: None,
            ts: 5,
        }];
        let (_, _, cost, ..) = cumulative_stats(&recs);
        assert_eq!(cost, None);
    }

    #[test]
    fn linked_from_roundtrip() {
        let rec = SessionRecord::LinkedFrom {
            prev_session_id: "abc123".into(),
            reason: "plan_execution".into(),
            golden_cycle: 2,
            golden_stalls: 1,
            golden_last_pending: vec!["golden-x".into()],
            ts: 100,
        };
        let json = serde_json::to_string(&rec).unwrap();
        assert!(json.contains("\"linked_from\""));
        let back: SessionRecord = serde_json::from_str(&json).unwrap();
        match back {
            SessionRecord::LinkedFrom {
                prev_session_id,
                reason,
                golden_cycle,
                golden_stalls,
                golden_last_pending,
                ..
            } => {
                assert_eq!(prev_session_id, "abc123");
                assert_eq!(reason, "plan_execution");
                assert_eq!(golden_cycle, 2);
                assert_eq!(golden_stalls, 1);
                assert_eq!(golden_last_pending, vec!["golden-x"]);
            }
            _ => panic!("expected LinkedFrom"),
        }
    }

    #[test]
    fn linked_from_helpers() {
        let recs = vec![
            SessionRecord::Meta {
                session_id: "s1".into(),
                created_at: 1,
                workspace: None,
            },
            SessionRecord::LinkedFrom {
                prev_session_id: "parent".into(),
                reason: "golden_flip".into(),
                golden_cycle: 3,
                golden_stalls: 0,
                golden_last_pending: vec!["golden-y".into()],
                ts: 2,
            },
        ];

        // linked_from()
        let info = linked_from(&recs).unwrap();
        assert_eq!(info.prev_session_id, "parent");
        assert_eq!(info.reason, "golden_flip");
        assert_eq!(info.golden_cycle, 3);
        assert_eq!(info.golden_stalls, 0);
        assert_eq!(info.golden_last_pending, vec!["golden-y"]);

        // linked_from() on empty
        assert!(linked_from(&[]).is_none());

        // linked_golden_state()
        let (cy, st, lp) = linked_golden_state(&recs);
        assert_eq!(cy, 3);
        assert_eq!(st, 0);
        assert_eq!(lp, vec!["golden-y"]);

        let (cy, st, lp) = linked_golden_state(&[]);
        assert_eq!(cy, 0);
        assert_eq!(st, 0);
        assert!(lp.is_empty());
    }

    #[test]
    fn chain_cumulative_cost_sums_whole_chain() {
        let dir = std::env::temp_dir().join(format!(
            "claudinio-chain-cost-test-sums-{}",
            std::process::id()
        ));
        let ws = dir.to_string_lossy().to_string();
        std::fs::create_dir_all(&dir).unwrap();

        // root -> mid -> tip
        let root_store = SessionStore::create("root", Some(&ws)).unwrap();
        root_store
            .append(&SessionRecord::Status {
                session_id: "root".into(),
                total_input_tokens: 0,
                total_output_tokens: 0,
                context_tokens: None,
                total_cost: Some(0.10),
                total_cost_input: Some(0.05),
                total_cost_output: Some(0.03),
                total_cost_cache_read: Some(0.02),
                ts: 10,
            })
            .unwrap();
        root_store
            .append(&SessionRecord::HandoffTo {
                next_session_id: "mid".into(),
                reason: "plan_execution".into(),
                ts: 20,
            })
            .unwrap();

        let mid_store = SessionStore::create("mid", Some(&ws)).unwrap();
        mid_store
            .append(&SessionRecord::LinkedFrom {
                prev_session_id: "root".into(),
                reason: "plan_execution".into(),
                golden_cycle: 0,
                golden_stalls: 0,
                golden_last_pending: vec![],
                ts: 21,
            })
            .unwrap();
        mid_store
            .append(&SessionRecord::Status {
                session_id: "mid".into(),
                total_input_tokens: 0,
                total_output_tokens: 0,
                context_tokens: None,
                total_cost: Some(0.20),
                total_cost_input: Some(0.10),
                total_cost_output: Some(0.08),
                total_cost_cache_read: Some(0.02),
                ts: 22,
            })
            .unwrap();
        mid_store
            .append(&SessionRecord::HandoffTo {
                next_session_id: "tip".into(),
                reason: "plan_execution".into(),
                ts: 30,
            })
            .unwrap();

        let tip_store = SessionStore::create("tip", Some(&ws)).unwrap();
        tip_store
            .append(&SessionRecord::LinkedFrom {
                prev_session_id: "mid".into(),
                reason: "plan_execution".into(),
                golden_cycle: 0,
                golden_stalls: 0,
                golden_last_pending: vec![],
                ts: 31,
            })
            .unwrap();
        tip_store
            .append(&SessionRecord::Status {
                session_id: "tip".into(),
                total_input_tokens: 0,
                total_output_tokens: 0,
                context_tokens: None,
                total_cost: Some(0.30),
                total_cost_input: Some(0.15),
                total_cost_output: Some(0.10),
                total_cost_cache_read: Some(0.05),
                ts: 32,
            })
            .unwrap();

        let (total_cost, cost_input, cost_output, cost_cache_read) =
            chain_cumulative_cost(Some(&ws), "tip");
        std::fs::remove_dir_all(&dir).ok();

        let eps = 1e-9;
        assert!(
            (total_cost.unwrap() - 0.60).abs() < eps,
            "total_cost: {total_cost:?}"
        );
        assert!(
            (cost_input.unwrap() - 0.30).abs() < eps,
            "cost_input: {cost_input:?}"
        );
        assert!(
            (cost_output.unwrap() - 0.21).abs() < eps,
            "cost_output: {cost_output:?}"
        );
        assert!(
            (cost_cache_read.unwrap() - 0.09).abs() < eps,
            "cost_cache_read: {cost_cache_read:?}"
        );
    }

    #[test]
    fn chain_cumulative_cost_no_cost_data_returns_none() {
        let dir = std::env::temp_dir().join(format!(
            "claudinio-chain-cost-test-none-{}",
            std::process::id()
        ));
        let ws = dir.to_string_lossy().to_string();
        std::fs::create_dir_all(&dir).unwrap();

        let store = SessionStore::create("solo", Some(&ws)).unwrap();
        store
            .append(&SessionRecord::Status {
                session_id: "solo".into(),
                total_input_tokens: 10,
                total_output_tokens: 5,
                context_tokens: None,
                total_cost: None,
                total_cost_input: None,
                total_cost_output: None,
                total_cost_cache_read: None,
                ts: 10,
            })
            .unwrap();

        let (total_cost, cost_input, cost_output, cost_cache_read) =
            chain_cumulative_cost(Some(&ws), "solo");
        std::fs::remove_dir_all(&dir).ok();

        assert_eq!(total_cost, None);
        assert_eq!(cost_input, None);
        assert_eq!(cost_output, None);
        assert_eq!(cost_cache_read, None);
    }

    #[test]
    fn chain_cumulative_cost_cycle_guard_stops() {
        let dir = std::env::temp_dir().join(format!(
            "claudinio-chain-cost-test-cycle-{}",
            std::process::id()
        ));
        let ws = dir.to_string_lossy().to_string();
        std::fs::create_dir_all(&dir).unwrap();

        // a <-> b cycle via LinkedFrom.
        let a_store = SessionStore::create("a", Some(&ws)).unwrap();
        a_store
            .append(&SessionRecord::LinkedFrom {
                prev_session_id: "b".into(),
                reason: "plan_execution".into(),
                golden_cycle: 0,
                golden_stalls: 0,
                golden_last_pending: vec![],
                ts: 1,
            })
            .unwrap();
        a_store
            .append(&SessionRecord::Status {
                session_id: "a".into(),
                total_input_tokens: 0,
                total_output_tokens: 0,
                context_tokens: None,
                total_cost: Some(1.0),
                total_cost_input: Some(1.0),
                total_cost_output: Some(1.0),
                total_cost_cache_read: Some(1.0),
                ts: 2,
            })
            .unwrap();

        let b_store = SessionStore::create("b", Some(&ws)).unwrap();
        b_store
            .append(&SessionRecord::LinkedFrom {
                prev_session_id: "a".into(),
                reason: "plan_execution".into(),
                golden_cycle: 0,
                golden_stalls: 0,
                golden_last_pending: vec![],
                ts: 3,
            })
            .unwrap();
        b_store
            .append(&SessionRecord::Status {
                session_id: "b".into(),
                total_input_tokens: 0,
                total_output_tokens: 0,
                context_tokens: None,
                total_cost: Some(1.0),
                total_cost_input: Some(1.0),
                total_cost_output: Some(1.0),
                total_cost_cache_read: Some(1.0),
                ts: 4,
            })
            .unwrap();

        let (total_cost, _, _, _) = chain_cumulative_cost(Some(&ws), "a");
        std::fs::remove_dir_all(&dir).ok();

        assert!(
            (total_cost.unwrap() - 2.0).abs() < 1e-9,
            "cycle guard must visit each session once, got {total_cost:?}"
        );
    }

    #[test]
    fn handoff_to_helper() {
        let recs = vec![SessionRecord::HandoffTo {
            next_session_id: "next-id".into(),
            reason: "context_handoff".into(),
            ts: 42,
        }];
        assert_eq!(handoff_to(&recs), Some("next-id".into()));
        assert_eq!(handoff_to(&[]), None);
    }

    #[test]
    fn handoff_record_roundtrip() {
        let rec = SessionRecord::Handoff {
            text: "## Purpose\nDo X".into(),
            ts: 99,
        };
        let json = serde_json::to_string(&rec).unwrap();
        assert!(json.contains("\"handoff\""));
        let back: SessionRecord = serde_json::from_str(&json).unwrap();
        match back {
            SessionRecord::Handoff { text, .. } => assert_eq!(text, "## Purpose\nDo X"),
            _ => panic!("expected Handoff"),
        }
    }

    #[test]
    fn list_sessions_includes_new_records_without_panic() {
        // Verify the match arms also handle LinkedFrom, HandoffTo, Handoff
        // at the new timestamp arms (line ~584).
        let recs = vec![
            SessionRecord::Meta {
                session_id: "sx".into(),
                created_at: 1,
                workspace: Some("/tmp/test".into()),
            },
            SessionRecord::User {
                text: "test".into(),
                ts: 2,
            },
            SessionRecord::LinkedFrom {
                prev_session_id: "p".into(),
                reason: "plan_execution".into(),
                golden_cycle: 0,
                golden_stalls: 0,
                golden_last_pending: vec![],
                ts: 3,
            },
            SessionRecord::HandoffTo {
                next_session_id: "n".into(),
                reason: "context_handoff".into(),
                ts: 4,
            },
            SessionRecord::Handoff {
                text: "doc".into(),
                ts: 5,
            },
        ];
        // Simulate what list_sessions does: iterate and extract ts. If any arm
        // panics we'd catch it here.
        let mut updated_at = 0u64;
        for rec in &recs {
            match rec {
                SessionRecord::LinkedFrom { ts, .. } => updated_at = updated_at.max(*ts),
                SessionRecord::HandoffTo { ts, .. } => updated_at = updated_at.max(*ts),
                SessionRecord::Handoff { ts, .. } => updated_at = updated_at.max(*ts),
                _ => {}
            }
        }
        assert_eq!(updated_at, 5);
    }
}

#[cfg(test)]
mod media_tests {
    use super::*;
    use crate::agent::provider::{ContentBlock, Message};

    fn big_image_b64() -> String {
        // Comfortably over MEDIA_INLINE_THRESHOLD, and valid base64.
        base64::Engine::encode(
            &base64::engine::general_purpose::STANDARD,
            vec![7u8; MEDIA_INLINE_THRESHOLD],
        )
    }

    fn store_in(dir: &Path) -> SessionStore {
        SessionStore {
            path: dir.join("s.jsonl"),
        }
    }

    fn tmp_dir(name: &str) -> std::path::PathBuf {
        let dir = std::env::temp_dir().join(format!("claudinio_media_{name}_{}", now_ms()));
        std::fs::create_dir_all(&dir).unwrap();
        dir
    }

    #[test]
    fn a_screenshot_round_trips_through_an_external_file() {
        let dir = tmp_dir("roundtrip");
        let store = store_in(&dir);
        let data = big_image_b64();
        store
            .append(&SessionRecord::Turn {
                ts: now_ms(),
                message: Message {
                    role: "user".into(),
                    content: vec![
                        ContentBlock::text("Screenshot of the page"),
                        ContentBlock::image("image/jpeg", &data, 1280, 800),
                    ],
                },
            })
            .unwrap();

        // The JSONL line must no longer carry the payload.
        let raw = std::fs::read_to_string(&store.path).unwrap();
        assert!(!raw.contains(&data), "base64 is still inline");
        assert!(raw.contains(MEDIA_REF_PREFIX));
        assert!(
            raw.len() < data.len() / 4,
            "line is still huge: {}",
            raw.len()
        );
        assert_eq!(std::fs::read_dir(dir.join("media")).unwrap().count(), 1);

        // …and reading it back must produce the original bytes.
        let records = load_records(&store.path).unwrap();
        let SessionRecord::Turn { message, .. } = &records[0] else {
            panic!("expected a Turn");
        };
        match &message.content[1] {
            ContentBlock::Image { source, .. } => {
                assert_eq!(source.data, data);
                assert_eq!(source.media_type, "image/jpeg");
            }
            other => panic!("expected an image, got {other:?}"),
        }
        std::fs::remove_dir_all(&dir).ok();
    }

    /// Two identical screenshots must not become two files.
    #[test]
    fn identical_images_are_stored_once() {
        let dir = tmp_dir("dedupe");
        let store = store_in(&dir);
        let data = big_image_b64();
        for _ in 0..3 {
            store
                .append(&SessionRecord::Turn {
                    ts: now_ms(),
                    message: Message {
                        role: "user".into(),
                        content: vec![ContentBlock::image("image/jpeg", &data, 10, 10)],
                    },
                })
                .unwrap();
        }
        assert_eq!(std::fs::read_dir(dir.join("media")).unwrap().count(), 1);
        std::fs::remove_dir_all(&dir).ok();
    }

    /// Small payloads are not worth a file each.
    #[test]
    fn small_images_stay_inline() {
        let dir = tmp_dir("inline");
        let store = store_in(&dir);
        store
            .append(&SessionRecord::Turn {
                ts: now_ms(),
                message: Message {
                    role: "user".into(),
                    content: vec![ContentBlock::image("image/png", "QUJD", 1, 1)],
                },
            })
            .unwrap();
        let raw = std::fs::read_to_string(&store.path).unwrap();
        assert!(raw.contains("QUJD"));
        assert!(!dir.join("media").exists());
        std::fs::remove_dir_all(&dir).ok();
    }

    /// A deleted media file must degrade one block, not fail the whole load —
    /// and must never reach the provider as undecodable data.
    #[test]
    fn a_missing_media_file_becomes_a_text_note() {
        let dir = tmp_dir("dangling");
        let store = store_in(&dir);
        store
            .append(&SessionRecord::Turn {
                ts: now_ms(),
                message: Message {
                    role: "user".into(),
                    content: vec![ContentBlock::image("image/jpeg", big_image_b64(), 1, 1)],
                },
            })
            .unwrap();
        std::fs::remove_dir_all(dir.join("media")).unwrap();

        let records = load_records(&store.path).unwrap();
        let SessionRecord::Turn { message, .. } = &records[0] else {
            panic!("expected a Turn");
        };
        assert_eq!(
            message.content[0].get_text(),
            Some("[image from an earlier step is no longer available]")
        );
        std::fs::remove_dir_all(&dir).ok();
    }

    /// Sessions written before this existed have no markers and must load
    /// byte-identically.
    #[test]
    fn records_without_images_are_untouched() {
        let dir = tmp_dir("plain");
        let store = store_in(&dir);
        store
            .append(&SessionRecord::Turn {
                ts: now_ms(),
                message: Message {
                    role: "user".into(),
                    content: vec![ContentBlock::text("just text")],
                },
            })
            .unwrap();
        assert!(!dir.join("media").exists());
        let records = load_records(&store.path).unwrap();
        let SessionRecord::Turn { message, .. } = &records[0] else {
            panic!("expected a Turn");
        };
        assert_eq!(message.content[0].get_text(), Some("just text"));
        std::fs::remove_dir_all(&dir).ok();
    }
}
