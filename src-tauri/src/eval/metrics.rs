//! What one eval run cost, read back from the session files it wrote.
//!
//! Nothing here is estimated from events in flight: every number comes from
//! the `usage` records and turns persisted under the task's
//! `.claudinio/sessions/` — the main session's files (one per linked session,
//! when a run hands off) and the subagent transcripts in `<id>.sub/`. If the
//! files do not show it, the run did not do it.

use crate::agent::persist::{SessionRecord, load_records};
use crate::agent::provider::ContentBlock;
use serde::Serialize;
use std::collections::{BTreeMap, BTreeSet, HashMap};
use std::path::{Path, PathBuf};

/// USD per million tokens, for providers that report no cost. Cache prices
/// default to Anthropic's ratios (reads 0.1×, writes 1.25× the input price).
#[derive(Debug, Clone, Copy, Default)]
pub(crate) struct Prices {
    pub input: Option<f64>,
    pub output: Option<f64>,
    pub cache_read: Option<f64>,
    pub cache_write: Option<f64>,
}

impl Prices {
    fn estimate(&self, u: &RequestUsage) -> Option<f64> {
        let (input, output) = (self.input?, self.output?);
        let read = self.cache_read.unwrap_or(input * 0.1);
        let write = self.cache_write.unwrap_or(input * 1.25);
        Some(
            (f64::from(u.input) * input
                + f64::from(u.output) * output
                + f64::from(u.cache_read) * read
                + f64::from(u.cache_creation) * write)
                / 1_000_000.0,
        )
    }
}

struct RequestUsage {
    input: u32,
    cache_read: u32,
    cache_creation: u32,
    output: u32,
    cost: Option<f64>,
}

#[derive(Debug, Clone, Default, Serialize, PartialEq)]
pub(crate) struct RunMetrics {
    /// Provider requests, main session and subagents together.
    pub rounds: u32,
    pub main_rounds: u32,
    pub subagent_rounds: u32,
    pub input_tokens: u64,
    pub cache_read_tokens: u64,
    pub cache_creation_tokens: u64,
    pub output_tokens: u64,
    /// Tokens (all four kinds) spent by subagents.
    pub subagent_tokens: u64,
    /// Reported cost, with `Prices` filling in requests that reported none.
    pub cost: f64,
    /// False when some request had neither a reported cost nor a price.
    pub cost_known: bool,
    pub tool_calls: BTreeMap<String, u32>,
    /// Subagents spawned with `spawn_agents`.
    pub subagents: u32,
    /// Compaction summaries, each a subagent run of its own.
    pub compactions: u32,
    pub sessions: u32,
    /// M4, main session: tool calls, and the results among them that are
    /// errors ([`is_error_result`]).
    pub main_tool_calls: u32,
    pub main_tool_errors: u32,
    /// M4, subagents (spawned ones and compaction summarizers alike).
    pub subagent_tool_calls: u32,
    pub subagent_tool_errors: u32,
    /// Tool name → error results, main session and subagents together.
    pub tool_errors: BTreeMap<String, u32>,
    /// M6: living documents linked by the task's sessions, and their
    /// sections as the files stand at the end of the task — in all, written
    /// (a `## heading`, not a `[placeholder]`), and written by a subagent
    /// (`by=sub:<name>`) or by the main session (`by=main`).
    pub documents: u32,
    pub doc_sections: u32,
    pub doc_sections_written: u32,
    pub doc_sections_by_sub: u32,
    pub doc_sections_by_main: u32,
}

/// Collect the metrics of every session under `sessions_dir`.
pub(crate) fn collect(sessions_dir: &Path, prices: &Prices) -> RunMetrics {
    let mut m = RunMetrics {
        cost_known: true,
        ..RunMetrics::default()
    };
    let (main, subs) = session_files(sessions_dir);
    m.sessions = main.len() as u32;
    m.compactions = subs.iter().filter(|p| is_compaction(p)).count() as u32;
    m.subagents = subs.len() as u32 - m.compactions;
    let mut linked = BTreeSet::new();
    for (path, is_sub) in main
        .iter()
        .map(|p| (p, false))
        .chain(subs.iter().map(|p| (p, true)))
    {
        let records = load_records(path).unwrap_or_default();
        if !is_sub && let Some(rel) = crate::agent::persist::linked_document(&records) {
            linked.insert(rel);
        }
        // Tool-use id → tool name, for the results that answer them.
        let mut tool_of: HashMap<String, String> = HashMap::new();
        for record in records {
            match record {
                SessionRecord::Usage {
                    input_tokens,
                    cache_read_tokens,
                    cache_creation_tokens,
                    output_tokens,
                    cost,
                    ..
                } => {
                    let u = RequestUsage {
                        input: input_tokens,
                        cache_read: cache_read_tokens,
                        cache_creation: cache_creation_tokens,
                        output: output_tokens,
                        cost,
                    };
                    m.rounds += 1;
                    let tokens = u64::from(u.input)
                        + u64::from(u.cache_read)
                        + u64::from(u.cache_creation)
                        + u64::from(u.output);
                    if is_sub {
                        m.subagent_rounds += 1;
                        m.subagent_tokens += tokens;
                    } else {
                        m.main_rounds += 1;
                    }
                    m.input_tokens += u64::from(u.input);
                    m.cache_read_tokens += u64::from(u.cache_read);
                    m.cache_creation_tokens += u64::from(u.cache_creation);
                    m.output_tokens += u64::from(u.output);
                    match u.cost.or_else(|| prices.estimate(&u)) {
                        Some(c) => m.cost += c,
                        None => m.cost_known = false,
                    }
                }
                SessionRecord::Turn { message, .. } => {
                    for block in &message.content {
                        match block {
                            ContentBlock::ToolUse { id, name, .. } => {
                                *m.tool_calls.entry(name.clone()).or_default() += 1;
                                if is_sub {
                                    m.subagent_tool_calls += 1;
                                } else {
                                    m.main_tool_calls += 1;
                                }
                                tool_of.insert(id.clone(), name.clone());
                            }
                            ContentBlock::ToolResult {
                                tool_use_id,
                                content,
                                ..
                            } => {
                                let name = tool_of.get(tool_use_id).map_or("?", String::as_str);
                                if is_error_result(name, &content.as_text()) {
                                    *m.tool_errors.entry(name.to_string()).or_default() += 1;
                                    if is_sub {
                                        m.subagent_tool_errors += 1;
                                    } else {
                                        m.main_tool_errors += 1;
                                    }
                                }
                            }
                            _ => {}
                        }
                    }
                }
                _ => {}
            }
        }
    }
    // The sessions directory is `<workspace>/.claudinio/sessions`, and a
    // linked document's path is relative to the workspace.
    if let Some(workspace) = sessions_dir.parent().and_then(Path::parent) {
        for rel in &linked {
            count_sections(&mut m, &workspace.join(rel));
        }
    }
    m
}

/// Whether a tool result is an error (M4): it starts with `Error`, the prefix
/// every tool arm and `tool_exec` write on failure (and the one `prune` and
/// the hook payload already read, there being no error flag on a
/// `tool_result` block), or it is a `spawn_agents` call that started none of
/// its agents — each agent's report then reads `not started: …` and the
/// result never says `Error` (single-timeline plan, F1). Refusals by a person
/// or by the eval's approval policy are the gates working, not errors.
/// `scripts/session-stats.py --meters` applies the same rule.
pub(crate) fn is_error_result(tool: &str, text: &str) -> bool {
    if text.starts_with("Error") {
        return true;
    }
    if tool != "spawn_agents" {
        return false;
    }
    let agents = text.lines().filter(|l| l.starts_with("## ")).count();
    let not_started = text
        .lines()
        .filter(|l| l.starts_with("not started: "))
        .count();
    agents > 0 && not_started >= agents
}

/// M6: one living document's sections as the file stands, by who wrote them.
/// A file that cannot be read still counts as a document, with no sections.
fn count_sections(m: &mut RunMetrics, path: &Path) {
    m.documents += 1;
    let Ok(text) = std::fs::read_to_string(path) else {
        return;
    };
    for section in crate::agent::document::parse(&text).sections() {
        m.doc_sections += 1;
        if !section.written {
            continue;
        }
        m.doc_sections_written += 1;
        match section.owner.as_deref() {
            Some("main") => m.doc_sections_by_main += 1,
            Some(owner) if owner.starts_with("sub:") => m.doc_sections_by_sub += 1,
            _ => {}
        }
    }
}

/// A compaction summarizer's transcript (`compact-<n>.jsonl`), as opposed to
/// a spawned subagent's (`<tool-use id>-<index>.jsonl`). Its requests count as
/// subagent rounds and tokens — it is one — but not as a spawned subagent.
fn is_compaction(path: &Path) -> bool {
    path.file_stem()
        .and_then(|s| s.to_str())
        .and_then(|s| s.rsplit_once('-'))
        .is_some_and(|(prefix, _)| prefix == crate::agent::subagent::COMPACTION)
}

/// The main session files (`*.jsonl` at the top) and subagent transcripts
/// (`*.sub/*.jsonl`), each in path order.
fn session_files(dir: &Path) -> (Vec<PathBuf>, Vec<PathBuf>) {
    let sorted = |dir: &Path| -> Vec<PathBuf> {
        let mut v: Vec<PathBuf> = std::fs::read_dir(dir)
            .map(|rd| rd.flatten().map(|e| e.path()).collect())
            .unwrap_or_default();
        v.sort();
        v
    };
    let mut main = Vec::new();
    let mut subs = Vec::new();
    for path in sorted(dir) {
        if path.is_dir() && path.extension().is_some_and(|e| e == "sub") {
            subs.extend(
                sorted(&path)
                    .into_iter()
                    .filter(|p| p.extension().is_some_and(|e| e == "jsonl")),
            );
        } else if path.extension().is_some_and(|e| e == "jsonl") {
            main.push(path);
        }
    }
    (main, subs)
}

/// Cache reads as a share of all input the provider processed.
pub(crate) fn cache_read_share(input: u64, cache_read: u64, cache_creation: u64) -> Option<f64> {
    let total = input + cache_read + cache_creation;
    (total > 0).then(|| cache_read as f64 / total as f64)
}

/// The median of `values` (the mean of the middle two for an even count).
pub(crate) fn median(values: &[u32]) -> Option<f64> {
    let mut v = values.to_vec();
    v.sort_unstable();
    match v.len() {
        0 => None,
        n if n % 2 == 1 => Some(f64::from(v[n / 2])),
        n => Some((f64::from(v[n / 2 - 1]) + f64::from(v[n / 2])) / 2.0),
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::agent::persist::SessionStore;
    use crate::agent::provider::{ContentBlock, Message, Usage};

    fn usage(input: u32, read: u32, created: u32, out: u32, cost: Option<f64>) -> Usage {
        Usage {
            input_tokens: input,
            output_tokens: out,
            cache_read_input_tokens: read,
            cache_creation_input_tokens: created,
            cost,
            ..Usage::default()
        }
    }

    #[test]
    fn metrics_sum_the_main_session_and_its_subagent_transcripts() {
        let dir = tempfile::tempdir().unwrap();
        let main = SessionStore {
            path: dir.path().join("s1.jsonl"),
        };
        main.append(&SessionRecord::usage(
            1,
            "m",
            Some(&usage(100, 900, 0, 10, Some(0.01))),
            None,
        ))
        .unwrap();
        main.append(&SessionRecord::Turn {
            message: Message {
                role: "assistant".into(),
                content: vec![ContentBlock::tool_use(
                    "t1",
                    "spawn_agents",
                    serde_json::json!({}),
                )],
            },
            ts: 1,
        })
        .unwrap();
        main.append(&SessionRecord::usage(2, "m", None, None))
            .unwrap();
        std::fs::create_dir_all(dir.path().join("s1.sub")).unwrap();
        let sub = SessionStore {
            path: dir.path().join("s1.sub").join("t1-0.jsonl"),
        };
        sub.append(&SessionRecord::usage(
            1,
            "m",
            Some(&usage(50, 0, 400, 5, None)),
            Some("t1-0".into()),
        ))
        .unwrap();

        // Without prices the subagent's request has no cost: unknown.
        let m = collect(dir.path(), &Prices::default());
        assert_eq!((m.rounds, m.main_rounds, m.subagent_rounds), (3, 2, 1));
        assert_eq!(m.input_tokens, 150);
        assert_eq!(m.cache_read_tokens, 900);
        assert_eq!(m.cache_creation_tokens, 400);
        assert_eq!(m.output_tokens, 15);
        assert_eq!(m.subagent_tokens, 455);
        assert_eq!((m.sessions, m.subagents), (1, 1));
        assert_eq!(m.tool_calls.get("spawn_agents"), Some(&1));
        assert!(!m.cost_known);

        // With prices it is estimated: (50·3 + 5·15 + 400·3.75) / 1e6.
        let prices = Prices {
            input: Some(3.0),
            output: Some(15.0),
            ..Prices::default()
        };
        let m = collect(dir.path(), &prices);
        assert!(m.cost_known);
        let expected = 0.01 + (50.0 * 3.0 + 5.0 * 15.0 + 400.0 * 3.75) / 1e6;
        assert!((m.cost - expected).abs() < 1e-12, "{}", m.cost);
    }

    /// A 90% cache hit on an OpenAI-protocol provider (`prompt_tokens` 1,000,
    /// 900 of them cached) is a 90% cache-read share, and its estimated cost
    /// bills the 900 at the cache-read price and the 100 at the input price.
    #[test]
    fn an_openai_request_reads_back_with_its_cache_counted_once() {
        let dir = tempfile::tempdir().unwrap();
        let store = SessionStore {
            path: dir.path().join("s1.jsonl"),
        };
        let openai = Usage {
            input_includes_cache_reads: true,
            ..usage(1_000, 900, 0, 10, None)
        };
        store
            .append(&SessionRecord::usage(1, "m", Some(&openai), None))
            .unwrap();
        let prices = Prices {
            input: Some(3.0),
            output: Some(15.0),
            ..Prices::default()
        };
        let m = collect(dir.path(), &prices);
        assert_eq!((m.input_tokens, m.cache_read_tokens), (100, 900));
        assert_eq!(
            cache_read_share(m.input_tokens, m.cache_read_tokens, m.cache_creation_tokens),
            Some(0.9)
        );
        let expected = (100.0 * 3.0 + 900.0 * 0.3 + 10.0 * 15.0) / 1e6;
        assert!((m.cost - expected).abs() < 1e-12, "{}", m.cost);
    }

    fn turn(store: &SessionStore, role: &str, content: Vec<ContentBlock>) {
        store
            .append(&SessionRecord::Turn {
                message: Message {
                    role: role.into(),
                    content,
                },
                ts: 1,
            })
            .unwrap();
    }

    /// One tool call and its result, as the loop persists them: the call in
    /// an assistant turn, the result in the user turn that follows.
    fn call(store: &SessionStore, id: &str, tool: &str, result: &str) {
        turn(
            store,
            "assistant",
            vec![ContentBlock::tool_use(id, tool, serde_json::json!({}))],
        );
        turn(store, "user", vec![ContentBlock::tool_result(id, result)]);
    }

    /// M4: a tool result is an error when the tool says so (`Error…`, the
    /// prefix every tool arm and `tool_exec` write, and the one the prune and
    /// hook code already read), or when it is a `spawn_agents` call that
    /// started none of its agents — the "section": "" failure of the
    /// single-timeline plan (F1), which reports `not started:` per agent and
    /// never says `Error`. Main session and subagents are counted apart,
    /// and every error is booked to its tool.
    #[test]
    fn tool_errors_are_counted_per_tool_and_apart_for_main_and_subagents() {
        let dir = tempfile::tempdir().unwrap();
        let main = SessionStore {
            path: dir.path().join("s1.jsonl"),
        };
        call(
            &main,
            "t1",
            "read_file",
            "Error: end_line must be >= start_line",
        );
        call(&main, "t2", "grep", "src/a.py:3: def load()");
        call(
            &main,
            "t3",
            "spawn_agents",
            "## config — failed (0 rounds)\nnot started: the document has no section ''. \
             Two agents cannot hold one section.\n---",
        );
        call(
            &main,
            "t4",
            "spawn_agents",
            "## retry — done (5 rounds)\nWrote s2.\n---",
        );
        std::fs::create_dir_all(dir.path().join("s1.sub")).unwrap();
        let sub = SessionStore {
            path: dir.path().join("s1.sub").join("t4-0.jsonl"),
        };
        call(
            &sub,
            "u1",
            "doc_read",
            "Error: you were given section s2 only; s3 belongs to another agent",
        );
        call(&sub, "u2", "read_file", "def retry(): ...");
        call(&sub, "u3", "doc_write", "Wrote s2.");

        let m = collect(dir.path(), &Prices::default());
        assert_eq!((m.main_tool_calls, m.main_tool_errors), (4, 2));
        assert_eq!((m.subagent_tool_calls, m.subagent_tool_errors), (3, 1));
        assert_eq!(
            m.tool_errors,
            BTreeMap::from([
                ("doc_read".to_string(), 1),
                ("read_file".to_string(), 1),
                ("spawn_agents".to_string(), 1),
            ])
        );
    }

    /// M6: the sections of the session's living document, by who wrote them,
    /// read from the file itself at the end of the task — main's rewrite of a
    /// subagent's section makes it main's (F4), and that is what is counted.
    #[test]
    fn the_linked_documents_sections_are_counted_by_owner() {
        let ws = tempfile::tempdir().unwrap();
        let sessions = ws.path().join(".claudinio").join("sessions");
        std::fs::create_dir_all(&sessions).unwrap();
        let docs = ws.path().join(".claudinio").join("docs");
        std::fs::create_dir_all(&docs).unwrap();
        std::fs::write(
            docs.join("notes.md"),
            "# Notes\n\n\
             ## Config\n<!-- claudinio: id=s1 status=done by=sub:config -->\nenv wins\n\
             <!-- claudinio: end -->\n\n\
             ## Retry\n<!-- claudinio: id=s2 status=done by=sub:retry -->\n429 and 503\n\
             <!-- claudinio: end -->\n\n\
             ## CLI\n<!-- claudinio: id=s3 status=done by=main -->\n--dry-run\n\
             <!-- claudinio: end -->\n\n\
             [Open questions] <!-- claudinio: id=s4 status=todo -->\n",
        )
        .unwrap();
        let main = SessionStore {
            path: sessions.join("s1.jsonl"),
        };
        main.append(&SessionRecord::DocumentLinked {
            path: ".claudinio/docs/notes.md".into(),
            ts: 1,
        })
        .unwrap();

        let m = collect(&sessions, &Prices::default());
        assert_eq!(m.documents, 1);
        assert_eq!(
            (
                m.doc_sections,
                m.doc_sections_written,
                m.doc_sections_by_sub,
                m.doc_sections_by_main
            ),
            (4, 3, 2, 1)
        );
    }

    #[test]
    fn a_task_without_a_document_counts_no_sections() {
        let ws = tempfile::tempdir().unwrap();
        let sessions = ws.path().join(".claudinio").join("sessions");
        std::fs::create_dir_all(&sessions).unwrap();
        let main = SessionStore {
            path: sessions.join("s1.jsonl"),
        };
        call(&main, "t1", "grep", "nothing");
        let m = collect(&sessions, &Prices::default());
        assert_eq!(
            (m.documents, m.doc_sections, m.doc_sections_written),
            (0, 0, 0)
        );
    }

    #[test]
    fn median_and_cache_share() {
        assert_eq!(median(&[]), None);
        assert_eq!(median(&[5, 1, 3]), Some(3.0));
        assert_eq!(median(&[4, 1, 3, 2]), Some(2.5));
        assert_eq!(cache_read_share(0, 0, 0), None);
        assert_eq!(cache_read_share(10, 80, 10), Some(0.8));
    }
}
