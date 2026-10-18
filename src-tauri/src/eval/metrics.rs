//! What one eval run cost, read back from the session files it wrote.
//!
//! Nothing here is estimated from events in flight: every number comes from
//! the `usage` records and turns persisted under the task's
//! `.claudinio/sessions/` — the main session's files (one per linked session,
//! when a run hands off) and the subagent transcripts in `<id>.sub/`. If the
//! files do not show it, the run did not do it.

use crate::agent::persist::{SessionRecord, load_records};
use serde::Serialize;
use std::collections::BTreeMap;
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
    for (path, is_sub) in main
        .iter()
        .map(|p| (p, false))
        .chain(subs.iter().map(|p| (p, true)))
    {
        for record in load_records(path).unwrap_or_default() {
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
                SessionRecord::Turn { message, .. } if message.role == "assistant" => {
                    for block in &message.content {
                        if let crate::agent::provider::ContentBlock::ToolUse { name, .. } = block {
                            *m.tool_calls.entry(name.clone()).or_default() += 1;
                        }
                    }
                }
                _ => {}
            }
        }
    }
    m
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

    #[test]
    fn median_and_cache_share() {
        assert_eq!(median(&[]), None);
        assert_eq!(median(&[5, 1, 3]), Some(3.0));
        assert_eq!(median(&[4, 1, 3, 2]), Some(2.5));
        assert_eq!(cache_read_share(0, 0, 0), None);
        assert_eq!(cache_read_share(10, 80, 10), Some(0.8));
    }
}
