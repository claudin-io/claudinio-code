//! Verbatim compaction: cut or drop old tool traffic, keep everything else
//! exactly as it was. A dropped call is stubbed in place — its `tool_use`
//! block stays, emptied, and its result becomes a one-line note — because
//! removing a block breaks DeepSeek's thinking mode (see [`apply`]).
//!
//! The summarizing compaction and the context handoff both replace the
//! conversation with prose a model wrote, and prose loses things — a file
//! path, the exact error, a constraint the user gave twenty turns ago. Here
//! nothing is rewritten: user and assistant text stay verbatim and in order;
//! only old tool outputs go.
//!
//! [`shed`] runs the ladder, masking first (lean-harness Phase 6):
//!
//! 0. **Lossless** ([`plan_lossless`]) — what a newer copy in the same history
//!    already supersedes: older task-list snapshots, and a file read again
//!    later with the same range. Nothing is judged because nothing is lost.
//! 2. **Masking by age** ([`plan_by_age`]) — the oldest tool results are cut
//!    to their head until enough is freed. Outputs go, calls stay: knowing a
//!    call was made is cheap and stops the model from making it again. Only as
//!    a last resort — a subagent, which has no handoff and no summary behind
//!    it — are the oldest calls dropped outright, because the stubs of a long
//!    run add up too and the alternative is to overflow.
//!
//! **Jev arbitrates, it no longer plans** ([`arbitrate`]). When a Jev backend
//! exists, it is asked only about the cuts masking would make in the
//! [`ARBITRATION_ROUNDS`] newest rounds — the outputs most likely still in
//! use — one `noul` each: should this output stay verbatim. What it keeps
//! comes out of the cut. Older cuts are not asked about: plain observation
//! masking costs about half of LLM summarization at an equal or slightly
//! better solve rate (Lindenbauer et al., *The Complexity Trap*, JetBrains,
//! arXiv 2508.21433), so the judge's money goes where age is the worst
//! proxy. Before Phase 6 Jev judged every old call first (up to 160, ~40
//! requests) and masking ran only when Jev was absent or not enough.
//!
//! The state Jev reads is the whole history oldest-first with tool results
//! replaced by a one-line note, fitted into [`MAX_STATE_CHARS`] in stages. It
//! is resent with every batch of [`CALLS_PER_REQUEST`] questions, which keeps
//! each request inside what every backend accepts, the claudin.io plan
//! endpoint included. Anything going wrong yields `None` and masking stands
//! as it is. (The Jev pass is ported from tamaratran/fast-jev-compaction.)

use serde_json::{Value, json};
use std::collections::HashSet;

use crate::agent::provider::{ContentBlock, Message, ToolResultContent};

/// P(keep result) from which a tool output masking would cut stays verbatim.
/// Upstream uses 0.5; our live probes (2026-10-05) put a schema the current
/// task still depended on at 0.41 and every no-longer-needed output at
/// 0.07–0.24, so the line sits between them — deleting a needed output costs
/// a re-read, keeping a stale one only costs context.
pub const KEEP_RESULT_THRESHOLD: f64 = 0.35;
/// Newest messages never touched; the first message is always kept too.
pub const PRESERVE_RECENT_MESSAGES: usize = 6;
/// Characters of a dropped result that stay, before the note.
pub const TRUNCATE_HEAD_CHARS: usize = 300;
/// Ceiling for the serialized state. Under the 60k the Jev client clips at
/// and the 64k the plan endpoint accepts, and well under Jev's 32k tokens.
pub const MAX_STATE_CHARS: usize = 56_000;
/// One question per call, eight per request — the plan endpoint's cap.
pub const CALLS_PER_REQUEST: usize = 8;
/// Jev arbitrates masking's cuts in this many newest rounds (assistant
/// messages that made tool calls); older cuts stand without a question. The
/// recent window masking never touches is at most three rounds, so this is
/// the five rounds just before it.
pub const ARBITRATION_ROUNDS: usize = 8;
/// At most this many calls are arbitrated per shed (oldest first); the rest
/// stay cut. Bounds the bill at 20 requests.
pub const MAX_JUDGED_CALLS: usize = 160;
/// A compaction that frees less than this share of the characters is not
/// worth breaking the cache for; the caller falls back.
pub const MIN_REDUCTION: f64 = 0.25;
const CONCURRENCY: usize = 8;
const INPUT_CHARS: [usize; 3] = [1000, 200, 60];
const TEXT_HEAD: usize = 400;
const TEXT_TAIL: usize = 150;

pub const STATE_CONTEXT: &str = "A coding assistant conversation is being compacted to free context. `history` is the whole conversation so far, oldest first; tool outputs are replaced by a short `result` note and long texts may be abridged. Each question asks whether the full output of one recent tool call, which would otherwise be cut to its first lines, still needs to stay in the history verbatim. Whatever is not kept is cut permanently, but the assistant can always re-run a tool or re-read a file.";

/// One tool call paired with its result.
#[derive(Debug, Clone)]
pub struct ToolCall {
    /// Short id shown to Jev (`t1`, `t2`, …).
    pub id: String,
    pub tool_use_id: String,
    pub tool: String,
    pub input: Value,
    pub call_msg: usize,
    pub result_chars: usize,
    pub is_error: bool,
    pub pinned: bool,
}

/// What to remove. Persisted as `SessionRecord::Pruned` and re-applied on load.
#[derive(Debug, Clone, Default, PartialEq, Eq)]
pub struct Decision {
    pub drop_calls: Vec<String>,
    pub truncate_results: Vec<String>,
}

impl Decision {
    pub fn is_empty(&self) -> bool {
        self.drop_calls.is_empty() && self.truncate_results.is_empty()
    }

    /// Both decisions as one. A call dropped by either is dropped — cutting
    /// the result of a call that is gone means nothing.
    pub fn merged(&self, other: &Decision) -> Decision {
        let mut seen = HashSet::new();
        let drop_calls: Vec<String> = self
            .drop_calls
            .iter()
            .chain(&other.drop_calls)
            .filter(|id| seen.insert(id.as_str()))
            .cloned()
            .collect();
        let truncate_results = self
            .truncate_results
            .iter()
            .chain(&other.truncate_results)
            .filter(|id| seen.insert(id.as_str()))
            .cloned()
            .collect();
        Decision {
            drop_calls,
            truncate_results,
        }
    }
}

/// Which stage of [`shed`] produced an outcome.
pub const SOURCE_LOSSLESS: &str = "lossless";
pub const SOURCE_JEV: &str = "jev";
pub const SOURCE_RULES: &str = "rules";

#[derive(Debug, Clone, Default)]
pub struct Outcome {
    pub decision: Decision,
    pub chars_before: usize,
    pub chars_after: usize,
    pub requests: usize,
    pub stage: &'static str,
    /// [`SOURCE_LOSSLESS`] | [`SOURCE_JEV`] | [`SOURCE_RULES`]: the last stage
    /// that contributed. Empty for an outcome straight out of [`plan`].
    pub source: &'static str,
    /// True when nothing comes after this outcome: taking it or not is the
    /// last decision this shed has to offer.
    pub last: bool,
    pub cost: f64,
    /// (tool_use_id, P(keep result)) for every call Jev arbitrated.
    pub judged: Vec<(String, f64)>,
}

impl Outcome {
    /// For the `Pruned` record: what was judged and how, so a session file
    /// says why each call went.
    pub fn stats_json(&self) -> Value {
        json!({
            "requests": self.requests,
            "stage": self.stage,
            "source": self.source,
            "reduction": (self.reduction() * 1000.0).round() / 1000.0,
            "chars_before": self.chars_before,
            "chars_after": self.chars_after,
            "cost": self.cost,
            "judged": self
                .judged
                .iter()
                .map(|(id, r)| json!([id, r]))
                .collect::<Vec<_>>(),
        })
    }

    pub fn reduction(&self) -> f64 {
        if self.chars_before == 0 {
            0.0
        } else {
            (self.chars_before - self.chars_after.min(self.chars_before)) as f64
                / self.chars_before as f64
        }
    }
}

pub fn is_pinned(index: usize, total: usize) -> bool {
    index == 0 || index + PRESERVE_RECENT_MESSAGES >= total
}

/// Pair every tool_use with its tool_result. Calls without a result are not
/// candidates, and neither are calls an earlier prune already dropped (their
/// result is [`DROPPED_RESULT_NOTE`]): nothing of them is left to judge, cut
/// or drop.
pub fn collect_tool_calls(history: &[Message]) -> Vec<ToolCall> {
    let mut results = std::collections::HashMap::new();
    for (i, m) in history.iter().enumerate() {
        for b in &m.content {
            if let ContentBlock::ToolResult {
                tool_use_id,
                content,
                ..
            } = b
            {
                let text = content.as_text();
                if text == DROPPED_RESULT_NOTE {
                    continue;
                }
                results.insert(
                    tool_use_id.clone(),
                    (i, text.len(), text.starts_with("Error")),
                );
            }
        }
    }
    let total = history.len();
    let mut calls = Vec::new();
    for (i, m) in history.iter().enumerate() {
        for b in &m.content {
            let ContentBlock::ToolUse {
                id, name, input, ..
            } = b
            else {
                continue;
            };
            let Some(&(ri, chars, is_error)) = results.get(id) else {
                continue;
            };
            calls.push(ToolCall {
                id: format!("t{}", calls.len() + 1),
                tool_use_id: id.clone(),
                tool: name.clone(),
                input: input.clone(),
                call_msg: i,
                result_chars: chars,
                is_error,
                pinned: is_pinned(i, total) || is_pinned(ri, total),
            });
        }
    }
    calls
}

/// The question for one call masking would cut.
pub fn question_for(call: &ToolCall) -> Value {
    json!({
        format!("result_{}", call.id): crate::agent::jev::noul(
            &format!(
                "The full output of tool call {} ({}, {} chars) should stay in the history \
                 verbatim: the assistant still needs its contents and re-running the tool \
                 would not do",
                call.id, call.tool, call.result_chars
            ),
            "Keep the full output",
            "The output is no longer needed verbatim",
        ),
    })
}

/// Whether Jev's P(keep result) keeps an output masking would cut.
pub fn keeps_result(p_keep: f64) -> bool {
    p_keep >= KEEP_RESULT_THRESHOLD
}

fn clip(s: &str, max: usize) -> String {
    if s.len() <= max {
        return s.to_string();
    }
    let mut end = max.saturating_sub(1);
    while !s.is_char_boundary(end) {
        end -= 1;
    }
    format!("{}…", &s[..end])
}

fn head_at(s: &str, max: usize) -> &str {
    let mut end = max.min(s.len());
    while !s.is_char_boundary(end) {
        end -= 1;
    }
    &s[..end]
}

fn tail_at(s: &str, max: usize) -> &str {
    let mut start = s.len().saturating_sub(max);
    while !s.is_char_boundary(start) {
        start += 1;
    }
    &s[start..]
}

fn message_text(m: &Message) -> String {
    m.content
        .iter()
        .filter_map(|b| b.get_text())
        .collect::<Vec<_>>()
        .join("\n")
}

fn is_prompt(m: &Message) -> bool {
    m.role == "user"
        && !m.content.is_empty()
        && m.content
            .iter()
            .all(|b| !matches!(b, ContentBlock::ToolResult { .. }))
}

/// The last three user prompts — what the session is for.
fn goal(history: &[Message]) -> String {
    let prompts: Vec<String> = history
        .iter()
        .filter(|m| is_prompt(m))
        .map(|m| clip(&message_text(m), 500))
        .filter(|t| !t.trim().is_empty())
        .collect();
    prompts[prompts.len().saturating_sub(3)..].join("\n")
}

fn result_note(c: &ToolCall) -> String {
    format!(
        "{}, {} chars (omitted)",
        if c.is_error { "error" } else { "ok" },
        c.result_chars
    )
}

fn call_entry(c: &ToolCall, input_chars: usize) -> Value {
    json!({
        "id": c.id,
        "tool": c.tool,
        "input": clip(&c.input.to_string(), input_chars),
        "result": result_note(c),
    })
}

/// One call as a single line, for when the structured form is too costly.
fn call_line(c: &ToolCall) -> Value {
    let input = clip(
        &c.input
            .to_string()
            .split_whitespace()
            .collect::<Vec<_>>()
            .join(" "),
        INPUT_CHARS[2],
    );
    Value::String(format!(
        "{} {} {} → {} {}ch",
        c.id,
        c.tool,
        input,
        if c.is_error { "error" } else { "ok" },
        c.result_chars
    ))
}

struct Entry {
    i: usize,
    role: String,
    text: String,
    calls: Vec<Value>,
    pinned: bool,
    left_out: bool,
}

impl Entry {
    fn to_json(&self) -> Value {
        let mut v = json!({"i": self.i, "role": self.role, "text": self.text});
        if !self.calls.is_empty() {
            v["tool_calls"] = Value::Array(self.calls.clone());
        }
        v
    }
    fn size(&self) -> usize {
        if self.left_out {
            0
        } else {
            self.to_json().to_string().len() + 1
        }
    }
}

/// The state Jev reads, fitted into [`MAX_STATE_CHARS`]. `None` when even the
/// last stage does not fit.
pub fn fit_state(history: &[Message], calls: &[ToolCall]) -> Option<(String, &'static str)> {
    let goal = goal(history);
    let base = json!({"context": STATE_CONTEXT, "goal": goal, "history": []})
        .to_string()
        .len();
    let total = history.len();
    let build = |input_chars: usize| -> Vec<Entry> {
        history
            .iter()
            .enumerate()
            .filter_map(|(i, m)| {
                let text = message_text(m);
                let own: Vec<Value> = calls
                    .iter()
                    .filter(|c| c.call_msg == i)
                    .map(|c| call_entry(c, input_chars))
                    .collect();
                if text.trim().is_empty() && own.is_empty() {
                    return None;
                }
                Some(Entry {
                    i,
                    role: m.role.clone(),
                    text,
                    calls: own,
                    pinned: is_pinned(i, total),
                    left_out: false,
                })
            })
            .collect()
    };
    let size = |es: &[Entry]| base + es.iter().map(Entry::size).sum::<usize>();
    let render = |es: &[Entry], stage: &'static str| {
        let h: Vec<Value> = es
            .iter()
            .filter(|e| !e.left_out)
            .map(Entry::to_json)
            .collect();
        Some((
            json!({"context": STATE_CONTEXT, "goal": goal, "history": h}).to_string(),
            stage,
        ))
    };

    let mut es = build(INPUT_CHARS[0]);
    if size(&es) <= MAX_STATE_CHARS {
        return render(&es, "full");
    }
    for (limit, stage) in [
        (INPUT_CHARS[1], "inputs<=200"),
        (INPUT_CHARS[2], "inputs<=60"),
    ] {
        es = build(limit);
        if size(&es) <= MAX_STATE_CHARS {
            return render(&es, stage);
        }
    }
    // Old (unpinned) entries first, oldest first; pinned ones last.
    let mut order: Vec<usize> = (0..es.len()).filter(|&k| !es[k].pinned).collect();
    order.extend((0..es.len()).filter(|&k| es[k].pinned));
    let mut current = size(&es);

    for &k in &order {
        let t = &es[k].text;
        if t.len() <= TEXT_HEAD + TEXT_TAIL + 40 {
            continue;
        }
        let before = es[k].size();
        let omitted = t.len() - TEXT_HEAD - TEXT_TAIL;
        es[k].text = format!(
            "{}\n[… {omitted} chars omitted …]\n{}",
            head_at(t, TEXT_HEAD),
            tail_at(t, TEXT_TAIL)
        );
        current = current + es[k].size() - before;
        if current <= MAX_STATE_CHARS {
            return render(&es, "texts abridged");
        }
    }
    for &k in &order {
        if es[k].pinned || es[k].text.is_empty() {
            continue;
        }
        let before = es[k].size();
        let original = message_text(&history[es[k].i]).len();
        es[k].text = format!("[… {original} chars omitted …]");
        current = current + es[k].size() - before;
        if current <= MAX_STATE_CHARS {
            return render(&es, "old messages collapsed");
        }
    }
    for &k in &order {
        if es[k].pinned || es[k].calls.is_empty() {
            continue;
        }
        let before = es[k].size();
        let i = es[k].i;
        es[k].calls = calls
            .iter()
            .filter(|c| c.call_msg == i)
            .map(call_line)
            .collect();
        current = current + es[k].size() - before;
        if current <= MAX_STATE_CHARS {
            return render(&es, "old calls compacted");
        }
    }
    for &k in &order {
        if es[k].pinned || !es[k].calls.is_empty() {
            continue;
        }
        current -= es[k].size();
        es[k].left_out = true;
        if current <= MAX_STATE_CHARS {
            return render(&es, "old messages left out");
        }
    }
    None
}

/// What a dropped call's result becomes. Short, because it is paid for on
/// every request after the prune; it says why the output is gone so the model
/// does not read it as a failure and run the call again.
pub const DROPPED_RESULT_NOTE: &str = "[dropped to save context: no longer needed]";

/// Stub dropped calls and cut truncated results to a head and a note.
/// Neither removes a block, so no decision empties a message or breaks the
/// role alternation; the pass that skips empty messages and merges
/// same-role neighbours is a safeguard for a history that came in malformed,
/// not a consequence of a drop.
///
/// The price of the stub is also a size floor: a call no larger than `{}`
/// plus the note is never chosen for a drop (`plan_by_age`).
///
/// **A dropped call is stubbed, never removed.** Its `tool_use` block stays in
/// the assistant message with the same id and name and its input replaced by
/// `{}`, and its `tool_result` becomes [`DROPPED_RESULT_NOTE`]; the message
/// count and the role alternation are unchanged. DeepSeek's thinking mode
/// (claudin.io's models are DeepSeek V4.1 Flash) re-attaches its server-side
/// reasoning by matching the tool_use blocks of earlier assistant messages,
/// and the harness stores no thinking blocks it could send back instead. In
/// the first live eval (2026-10-18, task hist-windows-folder-name) removing
/// the first of two tool_use blocks of one message got HTTP 400 "The
/// `content[].thinking` in the thinking mode must be passed back to the API";
/// replayed against the API, the same block kept with input `{}` and a stub
/// result was accepted, as was any edit of an input or a text block.
///
/// The price is a few tokens per dropped call (the empty block and the note)
/// that the old removal reclaimed; the bulk is still freed, because the
/// result goes and the input — the whole new content of a big `edit_file` —
/// is emptied.
pub fn apply(history: &[Message], decision: &Decision, head_chars: usize) -> Vec<Message> {
    let drop: HashSet<&str> = decision.drop_calls.iter().map(String::as_str).collect();
    let cut: HashSet<&str> = decision
        .truncate_results
        .iter()
        .map(String::as_str)
        .collect();
    // A subagent's report is not something to "re-run": the work it describes
    // was done, and spawning it again would do it twice.
    let reports: HashSet<&str> = history
        .iter()
        .flat_map(|m| m.content.iter())
        .filter_map(|b| match b {
            ContentBlock::ToolUse { id, name, .. } if name == "spawn_agents" => Some(id.as_str()),
            _ => None,
        })
        .collect();
    let mut out: Vec<Message> = Vec::with_capacity(history.len());
    for m in history {
        let content: Vec<ContentBlock> = m
            .content
            .iter()
            .map(|b| match b {
                ContentBlock::ToolUse { id, name, .. } if drop.contains(id.as_str()) => {
                    ContentBlock::tool_use(id.clone(), name.clone(), json!({}))
                }
                ContentBlock::ToolResult { tool_use_id, .. }
                    if drop.contains(tool_use_id.as_str()) =>
                {
                    ContentBlock::tool_result(tool_use_id.clone(), DROPPED_RESULT_NOTE)
                }
                ContentBlock::ToolResult {
                    tool_use_id,
                    content,
                    ..
                } if cut.contains(tool_use_id.as_str()) => {
                    let text = content.as_text();
                    // Already cut, by this decision's earlier application or
                    // by an earlier prune: cutting the cut would shorten the
                    // head by the length of the note each time, and make the
                    // live history drift from what a reload rebuilds.
                    let already_cut = text
                        .rsplit('\n')
                        .next()
                        .is_some_and(|last| last.starts_with(COMPACTED_MARK));
                    if already_cut
                        || (text.len() <= head_chars + (MAX_CUT_CHARS - TRUNCATE_HEAD_CHARS)
                            && matches!(content, ToolResultContent::Text(_)))
                    {
                        return b.clone();
                    }
                    let removed = text.len().saturating_sub(head_chars);
                    let error = if text.starts_with("Error") {
                        " (error)"
                    } else {
                        ""
                    };
                    // Short: see `MAX_CUT_CHARS`.
                    let advice = if reports.contains(tool_use_id.as_str()) {
                        "the work is done, do not spawn it again"
                    } else {
                        "re-run the tool if needed"
                    };
                    ContentBlock::tool_result(
                        tool_use_id.clone(),
                        format!(
                            "{}\n[compacted: {removed} chars of this tool result removed{error}; \
                             {advice}]",
                            head_at(&text, head_chars)
                        ),
                    )
                }
                _ => b.clone(),
            })
            .collect();
        if content.is_empty() {
            continue;
        }
        match out.last_mut() {
            Some(last) if last.role == m.role => last.content.extend(content),
            _ => out.push(Message {
                role: m.role.clone(),
                content,
            }),
        }
    }
    out
}

/// Characters of text, tool input and tool output in a history.
pub fn history_chars(history: &[Message]) -> usize {
    history
        .iter()
        .flat_map(|m| m.content.iter())
        .map(|b| match b {
            ContentBlock::Text { text, .. } => text.len(),
            ContentBlock::ToolUse { input, .. } => input.to_string().len(),
            ContentBlock::ToolResult { content, .. } => content.as_text().len(),
            _ => 0,
        })
        .sum()
}

/// The calls `decision` (masking's) cuts or drops whose call sits in one of
/// the [`ARBITRATION_ROUNDS`] newest rounds, in history order: what Jev is
/// asked about.
pub fn arbitration_candidates(history: &[Message], decision: &Decision) -> Vec<String> {
    let rounds: Vec<usize> = history
        .iter()
        .enumerate()
        .filter(|(_, m)| {
            m.role == "assistant"
                && m.content
                    .iter()
                    .any(|b| matches!(b, ContentBlock::ToolUse { .. }))
        })
        .map(|(i, _)| i)
        .collect();
    let Some(&first) = rounds.get(rounds.len().saturating_sub(ARBITRATION_ROUNDS)) else {
        return Vec::new();
    };
    let cut: HashSet<&str> = decision
        .truncate_results
        .iter()
        .chain(&decision.drop_calls)
        .map(String::as_str)
        .collect();
    collect_tool_calls(history)
        .into_iter()
        .filter(|c| c.call_msg >= first && cut.contains(c.tool_use_id.as_str()))
        .map(|c| c.tool_use_id)
        .take(MAX_JUDGED_CALLS)
        .collect()
}

/// What Jev answered about masking's recent cuts.
#[derive(Debug, Clone, Default)]
pub struct Arbitration {
    /// The candidates whose output stays verbatim, in history order.
    pub keep: Vec<String>,
    /// (tool_use_id, P(keep result)) for every candidate.
    pub judged: Vec<(String, f64)>,
    pub cost: f64,
    pub requests: usize,
    /// How the state had to be fitted ([`fit_state`]).
    pub stage: &'static str,
}

/// Ask Jev whether the output of each call in `ids` must stay verbatim.
/// `None` on any failure — no backend answer, a state that does not fit,
/// nothing to ask — and masking then stands as it is.
pub async fn arbitrate(
    history: &[Message],
    ids: &[String],
    backend: &crate::agent::jev::JevBackend,
) -> Option<Arbitration> {
    let calls = collect_tool_calls(history);
    let candidates: Vec<&ToolCall> = calls
        .iter()
        .filter(|c| ids.contains(&c.tool_use_id))
        .collect();
    if candidates.is_empty() {
        return None;
    }
    let (state, stage) = fit_state(history, &calls)?;
    let question_sets: Vec<Value> = candidates
        .chunks(CALLS_PER_REQUEST)
        .map(|batch| {
            let mut q = serde_json::Map::new();
            for c in batch {
                if let Value::Object(m) = question_for(c) {
                    q.extend(m);
                }
            }
            Value::Object(q)
        })
        .collect();
    let requests = question_sets.len();
    let mut answered: Vec<Option<crate::agent::jev::Decision>> = Vec::with_capacity(requests);
    for chunk in question_sets.chunks(CONCURRENCY) {
        let calls = chunk
            .iter()
            .map(|q| crate::agent::jev::decide(backend, &state, q));
        answered.extend(futures::future::join_all(calls).await);
    }
    let mut cost = 0.0;
    let mut answers = serde_json::Map::new();
    for d in answered {
        let d = d?; // one failed batch fails the arbitration: no half-asked cut
        cost += d.cost;
        answers.extend(d.answers);
    }
    let mut out = Arbitration {
        cost,
        requests,
        stage,
        ..Default::default()
    };
    for c in &candidates {
        let p = answers
            .get(&format!("result_{}", c.id))?
            .get("noul")?
            .as_f64()?;
        out.judged.push((c.tool_use_id.clone(), p));
        if keeps_result(p) {
            out.keep.push(c.tool_use_id.clone());
        }
    }
    Some(out)
}

/// Tools whose call or result carries the whole task list.
const TASK_SNAPSHOT_TOOLS: [&str; 2] = ["tasks_get", "tasks_set"];
/// Every tool that reads or changes the task list.
const TASK_TOOLS: [&str; 3] = ["tasks_get", "tasks_set", "tasks_update"];
/// What `apply` appends to a result it cut; a result carrying it is no longer
/// a full copy of anything.
const COMPACTED_MARK: &str = "[compacted: ";

/// The file and range a `read_file` call asked for.
fn read_key(input: &Value) -> Option<(String, Option<u64>, Option<u64>)> {
    let path = input
        .get("path")
        .or_else(|| input.get("file_path"))?
        .as_str()?;
    let line = |k: &str| input.get(k).and_then(Value::as_u64);
    Some((path.to_string(), line("start_line"), line("end_line")))
}

/// Stage 0: what a newer copy in the same history already supersedes.
///
/// - **Task snapshots.** `tasks_get` returns the whole list and `tasks_set`
///   sends it, so every task call before the latest successful snapshot repeats
///   information the latest one holds in full. The calls go, results and all:
///   the arguments are where the bulk is.
/// - **Repeated reads.** A `read_file` with the same path and range as a later
///   successful one is cut to its head. The call stays, so the history still
///   shows the file was looked at then.
///
/// Measured on 509 real sessions before this existed: `tasks_set` arguments
/// were 24% of everything the main session wrote, and 10% of `read_file` calls
/// repeated an earlier one exactly.
pub fn plan_lossless(history: &[Message]) -> Decision {
    let calls = collect_tool_calls(history);
    let results = ResultKinds::of(history);
    let whole = |c: &ToolCall| !results.partial.contains(c.tool_use_id.as_str());
    let mut decision = Decision::default();

    if let Some(latest) = results.whole_task_snapshot(&calls) {
        for c in &calls[..latest] {
            if !c.pinned && TASK_TOOLS.contains(&c.tool.as_str()) {
                decision.drop_calls.push(c.tool_use_id.clone());
            }
        }
    }

    // Newest first, so each read is checked against the reads after it. The
    // value is the least an older copy must be covered by: all of it when the
    // newer one was cut by the result cap (a cap that shrank with the model
    // must not replace a fuller copy), half of it otherwise — a newer copy
    // much smaller than the old one is more likely a refusal in words this
    // module does not know (a hook's own reason) than the same file. Keeping
    // both costs tokens; cutting the only real one costs the file.
    let mut newer_reads: std::collections::HashMap<_, usize> = std::collections::HashMap::new();
    for c in calls.iter().rev() {
        if c.tool != "read_file" {
            continue;
        }
        let Some(key) = read_key(&c.input) else {
            continue;
        };
        match newer_reads.get(&key) {
            Some(&covers) => {
                if !c.pinned && whole(c) && c.result_chars <= covers {
                    decision.truncate_results.push(c.tool_use_id.clone());
                }
            }
            // Only a result that is still whole can stand in for an older one.
            None if whole(c) => {
                let capped = results.capped.contains(c.tool_use_id.as_str());
                let covers = if capped {
                    c.result_chars
                } else {
                    c.result_chars.saturating_mul(2)
                };
                newer_reads.insert(key, covers);
            }
            None => {}
        }
    }
    decision.truncate_results.reverse();
    decision
}

/// What the session writes as a result when the tool never ran.
const NOT_RUN_PREFIXES: [&str; 4] = [
    "Error",
    "Tool call rejected",
    "Edit rejected",
    "Interrupted by the user",
];

/// Which results in a history are less than what their tool returned.
struct ResultKinds<'a> {
    /// The tool failed or never ran, a prune cut the text, or it is a pointer
    /// to an earlier read. None of these can stand in for another result.
    partial: HashSet<&'a str>,
    /// Cut by the result cap (`result_cap`, either marker): a real result,
    /// but not all of it.
    capped: HashSet<&'a str>,
}

impl<'a> ResultKinds<'a> {
    fn of(history: &'a [Message]) -> Self {
        let mut kinds = ResultKinds {
            partial: HashSet::new(),
            capped: HashSet::new(),
        };
        for b in history.iter().flat_map(|m| m.content.iter()) {
            let ContentBlock::ToolResult {
                tool_use_id,
                content,
                ..
            } = b
            else {
                continue;
            };
            let text = content.as_text();
            if text.contains(COMPACTED_MARK)
                || text.starts_with(UNCHANGED_MARK)
                || NOT_RUN_PREFIXES.iter().any(|p| text.starts_with(p))
            {
                kinds.partial.insert(tool_use_id.as_str());
            } else if crate::agent::result_cap::parse(&text).is_some() {
                kinds.capped.insert(tool_use_id.as_str());
            }
        }
        kinds
    }

    /// Index in `calls` of the newest task snapshot that holds the whole
    /// list: the one every earlier task call only repeats part of.
    fn whole_task_snapshot(&self, calls: &[ToolCall]) -> Option<usize> {
        calls.iter().rposition(|c| {
            let id = c.tool_use_id.as_str();
            TASK_SNAPSHOT_TOOLS.contains(&c.tool.as_str())
                && !self.partial.contains(id)
                && !self.capped.contains(id)
        })
    }

    /// Index in `calls` of the newest task snapshot the model was actually
    /// shown, whole or cut by the result cap: the list as it knows it. A list
    /// too long for the cap is never whole, and it is still the current one —
    /// with every older copy of it as old as any other result.
    fn current_task_snapshot(&self, calls: &[ToolCall]) -> Option<usize> {
        calls.iter().rposition(|c| {
            TASK_SNAPSHOT_TOOLS.contains(&c.tool.as_str())
                && !self.partial.contains(c.tool_use_id.as_str())
        })
    }
}

/// How the history copy of a repeated read starts.
pub const UNCHANGED_MARK: &str = "[unchanged: ";

/// A read shorter than this is repeated as it is: a pointer would save nothing.
const POINTER_MIN_CHARS: usize = 600;

/// What to put in history, instead of `content`, for a `read_file` whose
/// result the conversation already holds word for word.
///
/// The check is on the text itself, not on a record of what was read when: the
/// most recent read of the same file and range must still be in `history` and
/// be exactly `content`. So it cannot point at a result a prune has cut, a
/// compaction has summarized or another agent's history holds, and a file that
/// changed in between is simply returned.
///
/// A pointer is never answered with another pointer. A model that asks again
/// right after one is saying it could not use it, and gets the file.
///
/// Measured before this existed, on 509 real sessions: one `read_file` in ten
/// repeated an earlier one exactly, and read results were 44% of everything
/// carried from request to request.
pub fn repeated_read(history: &[Message], input: &Value, content: &str) -> Option<String> {
    if content.len() < POINTER_MIN_CHARS || content.starts_with("Error") {
        return None;
    }
    let key = read_key(input)?;
    let result_of = |id: &str| {
        history
            .iter()
            .flat_map(|m| m.content.iter())
            .find_map(|b| match b {
                ContentBlock::ToolResult {
                    tool_use_id,
                    content,
                    ..
                } if tool_use_id == id => Some(content.as_text()),
                _ => None,
            })
    };
    let previous = history
        .iter()
        .rev()
        .flat_map(|m| m.content.iter().rev())
        .find_map(|b| match b {
            ContentBlock::ToolUse {
                id, name, input, ..
            } if name == "read_file" && read_key(input).as_ref() == Some(&key) => result_of(id),
            _ => None,
        })?;
    if previous.starts_with(UNCHANGED_MARK) || previous != content {
        return None;
    }
    let (path, start, end) = key;
    let range = match (start, end) {
        (Some(s), Some(e)) => format!(" lines {s}-{e}"),
        _ => String::new(),
    };
    Some(format!(
        "{UNCHANGED_MARK}{path}{range} is identical to what your earlier read_file of it \
         returned ({} lines). That result is still in this conversation and is not repeated \
         here. If you cannot find it, call read_file again and the content is returned.]",
        content.lines().count()
    ))
}

/// `block` as it should enter `history`: a `read_file` result the conversation
/// already holds becomes a pointer to it ([`repeated_read`]); anything else is
/// returned untouched.
pub fn history_copy(
    history: &[Message],
    tool: &str,
    input: &Value,
    block: ContentBlock,
) -> ContentBlock {
    if tool != "read_file" {
        return block;
    }
    let ContentBlock::ToolResult {
        tool_use_id,
        content: ToolResultContent::Text(text),
        ..
    } = &block
    else {
        return block;
    };
    match repeated_read(history, input, text) {
        Some(pointer) => ContentBlock::tool_result(tool_use_id.clone(), pointer),
        None => block,
    }
}

/// Characters of one message, counted as [`history_chars`] counts them.
fn message_chars(m: &Message) -> usize {
    history_chars(std::slice::from_ref(m))
}

/// Index of the first message of the recent window: the newest messages that
/// fit in `recent_chars`, never fewer than the last round (two messages) and
/// never more than [`PRESERVE_RECENT_MESSAGES`].
///
/// A fixed message count is the wrong pin on a small window: three rounds of
/// tool results can be most of what the model can hold, which would leave
/// nothing old enough to cut.
fn recent_start(history: &[Message], recent_chars: usize) -> usize {
    let total = history.len();
    let mut start = total;
    let mut used = 0usize;
    while start > 0 && total - start < PRESERVE_RECENT_MESSAGES {
        let next = message_chars(&history[start - 1]);
        if total - start >= 2 && used + next > recent_chars {
            break;
        }
        used += next;
        start -= 1;
    }
    start
}

/// The most `apply` leaves of a cut result: its head and a note. A result at
/// or under this is left as it is — so a note must never be longer than the
/// difference, or a cut result would look worth cutting again.
const MAX_CUT_CHARS: usize = TRUNCATE_HEAD_CHARS + 120;

/// What cutting one result to its head frees, or `None` when `apply` would
/// leave it as it is.
fn freed_by_cut(result_chars: usize) -> Option<usize> {
    (result_chars > MAX_CUT_CHARS).then(|| result_chars - TRUNCATE_HEAD_CHARS)
}

/// Roughly what `apply` leaves of a cut result: the head and its note.
const CUT_RESULT_CHARS: usize = TRUNCATE_HEAD_CHARS + 100;

/// Stage 2: cut the oldest tool results to their head until `free_chars` are
/// freed, leaving the first message and the recent window alone.
///
/// The rule every shed starts from since lean-harness Phase 6: age is a poor
/// proxy for "no longer needed", which is why Jev, when there is one, may
/// keep the cuts this plan makes in the newest rounds ([`arbitrate`]). But
/// the cost of a wrong cut is a re-read, and the alternative here is a handoff, a summary or
/// an overflow. On the stored sessions that reached 96k it frees a median 70%
/// of the history.
///
/// With `drop_calls`, and only when cutting every eligible result still falls
/// short, the oldest calls are dropped outright, oldest first, until enough is
/// freed.
pub fn plan_by_age(
    history: &[Message],
    free_chars: usize,
    recent_chars: usize,
    drop_calls: bool,
) -> Decision {
    let recent = recent_start(history, recent_chars);
    let mut result_msg = std::collections::HashMap::new();
    for (i, m) in history.iter().enumerate() {
        for b in &m.content {
            if let ContentBlock::ToolResult { tool_use_id, .. } = b {
                result_msg.insert(tool_use_id.as_str(), i);
            }
        }
    }
    let calls = collect_tool_calls(history);
    // The current task list, and every change made to it since, stay: with
    // them cut the model works from a list it can no longer see. Everything
    // before that snapshot is the lossless stage's to drop.
    let current_tasks: HashSet<&str> = ResultKinds::of(history)
        .current_task_snapshot(&calls)
        .map(|latest| {
            calls[latest..]
                .iter()
                .filter(|c| TASK_TOOLS.contains(&c.tool.as_str()))
                .map(|c| c.tool_use_id.as_str())
                .collect()
        })
        .unwrap_or_default();
    let eligible: Vec<&ToolCall> = calls
        .iter()
        .filter(|c| {
            result_msg
                .get(c.tool_use_id.as_str())
                .is_some_and(|&ri| ri != 0 && ri < recent)
        })
        // An `ask_user` result is the user's answer. It is their words in a
        // tool result's clothes, and no tool can be re-run to get it back.
        .filter(|c| c.tool != "ask_user" && !current_tasks.contains(c.tool_use_id.as_str()))
        .collect();
    let mut decision = Decision::default();
    let mut freed = 0usize;
    for c in &eligible {
        if freed >= free_chars {
            return decision;
        }
        if let Some(gain) = freed_by_cut(c.result_chars) {
            freed += gain;
            decision.truncate_results.push(c.tool_use_id.clone());
        }
    }
    if !drop_calls {
        return decision;
    }
    for c in &eligible {
        if freed >= free_chars {
            break;
        }
        // A dropped call is stubbed, not removed (see `apply`): the input
        // shrinks to `{}` and the result to the note. A call no larger than
        // that stub would grow the history, so it stays as it is.
        let size = c.input.to_string().len() + c.result_chars.min(CUT_RESULT_CHARS);
        let stub = "{}".len() + DROPPED_RESULT_NOTE.len();
        if size <= stub {
            continue;
        }
        freed += size - stub;
        decision.truncate_results.retain(|id| id != &c.tool_use_id);
        decision.drop_calls.push(c.tool_use_id.clone());
    }
    decision
}

/// How [`shed`] should go about it.
#[derive(Debug, Clone, Copy)]
pub struct ShedOptions {
    /// What masking aims to free, counted from the history as given.
    pub free_chars: usize,
    /// Size of the recent window masking leaves alone.
    pub recent_chars: usize,
    /// Nothing gentler comes after this shed: let masking drop the oldest
    /// calls outright when cutting their results falls short.
    ///
    /// False for the main session, which hands off next — no call
    /// disappears. True for a subagent, which has nothing after this but
    /// running out of window.
    pub last_resort: bool,
}

/// What [`shed`] came to.
#[derive(Debug, Default)]
pub struct Shed {
    /// The accepted outcome and the history it leaves, when a stage was enough.
    pub accepted: Option<(Outcome, Vec<Message>)>,
    /// What Jev cost, whether or not its answer was used.
    pub cost: f64,
}

impl Decision {
    /// This decision without the calls in `keep`.
    fn without(&self, keep: &[String]) -> Decision {
        let kept = |id: &String| !keep.contains(id);
        Decision {
            drop_calls: self
                .drop_calls
                .iter()
                .filter(|id| kept(id))
                .cloned()
                .collect(),
            truncate_results: self
                .truncate_results
                .iter()
                .filter(|id| kept(id))
                .cloned()
                .collect(),
        }
    }
}

/// Free context, stopping at the first outcome `accept` takes: lossless;
/// masking with Jev's keeps taken out of its recent cuts (when a backend
/// exists and masking cuts anything in the [`ARBITRATION_ROUNDS`] newest
/// rounds); masking alone, offered last. Each stage builds on the lossless
/// one, and the outcome is cumulative — one decision that, applied to
/// `history`, gives the returned history.
///
/// Masking alone comes after Jev's version even in a main session: what Jev
/// keeps is honoured when the cut still frees enough, but a handoff or a
/// summary — what follows a refused shed there — loses far more than a few
/// outputs the model can read again.
pub async fn shed<F>(
    history: &[Message],
    backend: Option<&crate::agent::jev::JevBackend>,
    opts: ShedOptions,
    accept: F,
) -> Shed
where
    F: Fn(&Outcome, &[Message]) -> bool,
{
    let chars_before = history_chars(history);
    let lossless = plan_lossless(history);
    let after_lossless = apply(history, &lossless, TRUNCATE_HEAD_CHARS);
    if !lossless.is_empty() {
        let outcome = Outcome {
            decision: lossless.clone(),
            chars_before,
            chars_after: history_chars(&after_lossless),
            stage: SOURCE_LOSSLESS,
            source: SOURCE_LOSSLESS,
            ..Default::default()
        };
        if accept(&outcome, &after_lossless) {
            return Shed {
                accepted: Some((outcome, after_lossless)),
                cost: 0.0,
            };
        }
    }

    let already_freed = chars_before - history_chars(&after_lossless).min(chars_before);
    let masking = plan_by_age(
        &after_lossless,
        opts.free_chars.saturating_sub(already_freed),
        opts.recent_chars,
        opts.last_resort,
    );
    if masking.is_empty() {
        // Nothing to cut by age. What the lossless stage found was offered
        // above as one stage of several; it is offered again as all there is.
        let outcome = Outcome {
            decision: lossless,
            chars_before,
            chars_after: history_chars(&after_lossless),
            stage: SOURCE_LOSSLESS,
            source: SOURCE_LOSSLESS,
            last: true,
            ..Default::default()
        };
        let take = !outcome.decision.is_empty() && accept(&outcome, &after_lossless);
        return Shed {
            accepted: take.then_some((outcome, after_lossless)),
            cost: 0.0,
        };
    }

    let mut cost = 0.0;
    let mut requests = 0;
    let mut judged = Vec::new();
    let candidates = match backend {
        Some(_) => arbitration_candidates(&after_lossless, &masking),
        None => Vec::new(),
    };
    if let Some(backend) = backend
        && !candidates.is_empty()
        && let Some(arbitration) = arbitrate(&after_lossless, &candidates, backend).await
    {
        cost = arbitration.cost;
        requests = arbitration.requests;
        judged = arbitration.judged.clone();
        // Jev cut everything it was asked about: this is masking alone.
        let last = arbitration.keep.is_empty();
        let decision = lossless.merged(&masking.without(&arbitration.keep));
        if !decision.is_empty() {
            let after = apply(history, &decision, TRUNCATE_HEAD_CHARS);
            let outcome = Outcome {
                decision,
                chars_before,
                chars_after: history_chars(&after),
                requests,
                stage: arbitration.stage,
                source: SOURCE_JEV,
                last,
                cost,
                judged: arbitration.judged,
            };
            if accept(&outcome, &after) {
                return Shed {
                    accepted: Some((outcome, after)),
                    cost,
                };
            }
        }
        if last {
            return Shed {
                accepted: None,
                cost,
            };
        }
    }

    let decision = lossless.merged(&masking);
    let after = apply(history, &decision, TRUNCATE_HEAD_CHARS);
    let outcome = Outcome {
        decision,
        chars_before,
        chars_after: history_chars(&after),
        requests,
        stage: SOURCE_RULES,
        source: SOURCE_RULES,
        last: true,
        cost,
        judged,
    };
    let accepted = accept(&outcome, &after).then_some((outcome, after));
    Shed { accepted, cost }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::agent::jev::test_support::{spawn_stub, stub_backend};

    fn user(text: &str) -> Message {
        Message {
            role: "user".into(),
            content: vec![ContentBlock::text(text)],
        }
    }
    fn assistant_call(text: &str, id: &str, tool: &str, input: Value) -> Message {
        let mut content = Vec::new();
        if !text.is_empty() {
            content.push(ContentBlock::text(text));
        }
        content.push(ContentBlock::tool_use(id, tool, input));
        Message {
            role: "assistant".into(),
            content,
        }
    }
    fn result(id: &str, text: &str) -> Message {
        Message {
            role: "user".into(),
            content: vec![ContentBlock::tool_result(id, text)],
        }
    }
    fn assistant(text: &str) -> Message {
        Message {
            role: "assistant".into(),
            content: vec![ContentBlock::text(text)],
        }
    }

    /// A session with `n` read_file round-trips before a recent tail.
    fn session(n: usize) -> Vec<Message> {
        let mut h = vec![user("Fix the failing test. Never edit src/generated.")];
        for i in 0..n {
            h.push(assistant_call(
                if i == 0 { "Let me look around." } else { "" },
                &format!("toolu_{i}"),
                "read_file",
                json!({"path": format!("src/file_{i}.rs")}),
            ));
            h.push(result(&format!("toolu_{i}"), &"fn body() {}\n".repeat(400)));
        }
        h.push(assistant("Found it: the bug is in file_3."));
        h.push(user("Great, fix it."));
        h.push(assistant_call(
            "",
            "toolu_edit",
            "edit_file",
            json!({"path": "src/file_3.rs"}),
        ));
        h.push(result("toolu_edit", "ok"));
        h.push(assistant("Fixed."));
        h.push(user("Thanks"));
        h
    }

    #[test]
    fn the_first_and_the_newest_messages_are_pinned() {
        assert!(is_pinned(0, 20));
        assert!(!is_pinned(1, 20));
        assert!(!is_pinned(13, 20));
        assert!(is_pinned(14, 20));
        assert!(is_pinned(19, 20));
    }

    #[test]
    fn calls_are_paired_with_their_results_and_the_tail_is_pinned() {
        let h = session(5);
        let calls = collect_tool_calls(&h);
        assert_eq!(calls.len(), 6);
        assert_eq!(calls[0].id, "t1");
        assert_eq!(calls[0].tool_use_id, "toolu_0");
        assert_eq!(calls[0].tool, "read_file");
        assert_eq!(calls[0].result_chars, "fn body() {}\n".repeat(400).len());
        assert!(!calls[0].pinned);
        let edit = calls
            .iter()
            .find(|c| c.tool_use_id == "toolu_edit")
            .unwrap();
        assert!(
            edit.pinned,
            "a call in the newest messages is never touched"
        );
    }

    #[test]
    fn a_call_without_a_result_is_not_a_candidate() {
        let h = vec![
            user("go"),
            assistant_call("", "toolu_x", "bash", json!({"command": "ls"})),
        ];
        assert!(collect_tool_calls(&h).is_empty());
    }

    /// A stubbed call has nothing left to judge, cut or drop: offering it
    /// again would bill Jev for it and let the last resort count the stub as
    /// context it could still free.
    #[test]
    fn a_call_already_dropped_is_not_a_candidate_again() {
        let h = session(3);
        let d = Decision {
            drop_calls: vec!["toolu_1".into()],
            truncate_results: vec![],
        };
        let after = apply(&h, &d, TRUNCATE_HEAD_CHARS);
        let ids: Vec<String> = collect_tool_calls(&after)
            .into_iter()
            .map(|c| c.tool_use_id)
            .collect();
        assert!(!ids.contains(&"toolu_1".to_string()), "{ids:?}");
        assert!(ids.contains(&"toolu_0".to_string()) && ids.contains(&"toolu_2".to_string()));
        let again = plan_by_age(&after, usize::MAX, usize::MAX, true);
        assert!(!again.drop_calls.contains(&"toolu_1".to_string()));
        assert!(!again.truncate_results.contains(&"toolu_1".to_string()));
    }

    // Behaviour deliberately changed by lean-harness Phase 6: Jev no longer
    // chooses between keep / cut / drop for every call, it only says whether a
    // result masking would cut must stay. The P(keep result) of the two live
    // probes (2026-10-05) still decides it against the same line.
    #[test]
    fn the_live_probes_map_to_conservative_arbitration() {
        // The schema the current task depends on: kept whole.
        assert!(keeps_result(0.41));
        // The file with the bug, already fixed; a build log nobody needs any
        // more; README and unrelated UI files: masking cuts them.
        for p in [0.23, 0.14, 0.11, 0.07] {
            assert!(!keeps_result(p), "{p}");
        }
    }

    #[test]
    fn the_outcome_reports_its_own_diagnosis() {
        let o = Outcome {
            chars_before: 100,
            chars_after: 40,
            requests: 3,
            stage: "full",
            cost: 2e-5,
            judged: vec![("toolu_1".into(), 0.23)],
            ..Default::default()
        };
        let v = o.stats_json();
        assert_eq!(v["requests"], 3);
        assert_eq!(v["stage"], "full");
        assert_eq!(v["reduction"], 0.6);
        assert_eq!(v["judged"][0], json!(["toolu_1", 0.23]));
    }

    #[test]
    fn the_state_shows_every_call_and_no_result_content() {
        let h = session(5);
        let calls = collect_tool_calls(&h);
        let (state, stage) = fit_state(&h, &calls).expect("fits");
        assert_eq!(stage, "full");
        assert!(state.contains("Never edit src/generated."));
        assert!(state.contains("\"t1\""));
        assert!(state.contains("src/file_0.rs"));
        assert!(state.contains("chars (omitted)"));
        assert!(
            !state.contains("fn body()"),
            "results must be notes, not contents"
        );
    }

    #[test]
    fn a_long_history_is_fitted_in_stages() {
        let mut h = session(150);
        // Long assistant prose in the old part forces the text stages.
        h.insert(2, assistant(&"reasoning ".repeat(6000)));
        let calls = collect_tool_calls(&h);
        let (state, stage) = fit_state(&h, &calls).expect("fits after shrinking");
        assert!(state.len() <= MAX_STATE_CHARS, "{}", state.len());
        assert_ne!(stage, "full");
    }

    #[test]
    fn apply_stubs_a_dropped_call_and_its_result_and_keeps_every_message() {
        let h = session(3);
        let d = Decision {
            drop_calls: vec!["toolu_1".into()],
            truncate_results: vec![],
        };
        let out = apply(&h, &d, TRUNCATE_HEAD_CHARS);
        assert_eq!(out.len(), h.len(), "a drop removes no message");
        let text = serde_json::to_string(&out).unwrap();
        // The call is still there by id, but what it read and its input are gone.
        assert!(text.contains("toolu_1"));
        assert!(!text.contains("src/file_1.rs"));
        assert!(text.contains(DROPPED_RESULT_NOTE));
        assert!(text.contains("src/file_0.rs") && text.contains("src/file_2.rs"));
        assert!(history_chars(&out) < history_chars(&h));
        assert!(
            out.windows(2).all(|w| w[0].role != w[1].role),
            "roles must alternate"
        );
        assert!(out.iter().all(|m| !m.content.is_empty()));
    }

    /// The harness never removes a `tool_use` block from an assistant message
    /// once it was sent. DeepSeek's thinking mode re-attaches its server-side
    /// reasoning by the tool_use blocks of earlier assistant messages: in the
    /// first live eval (2026-10-18, deepseek-flash) a prune that removed the
    /// first of two tool_use blocks got HTTP 400 "The `content[].thinking` in
    /// the thinking mode must be passed back to the API", while the same block
    /// kept with an emptied input and a stub result was accepted. A dropped
    /// call therefore keeps its block (same id, same name) with input `{}`,
    /// its result becomes a short stub, and its neighbour is untouched.
    #[test]
    fn a_dropped_call_keeps_its_tool_use_block_with_an_empty_input_and_a_stub_result() {
        let h = vec![
            user("Rename the windows folder."),
            Message {
                role: "assistant".into(),
                content: vec![
                    ContentBlock::text("Reading the module and listing the tree."),
                    ContentBlock::tool_use(
                        "toolu_read",
                        "read_file",
                        json!({"path": "src/lib.rs"}),
                    ),
                    ContentBlock::tool_use("toolu_bash", "bash", json!({"command": "ls"})),
                ],
            },
            Message {
                role: "user".into(),
                content: vec![
                    ContentBlock::tool_result("toolu_read", "fn body() {}\n".repeat(400)),
                    ContentBlock::tool_result("toolu_bash", "a\nb"),
                ],
            },
            assistant("Done."),
            user("Thanks"),
        ];
        let d = Decision {
            drop_calls: vec!["toolu_read".into()],
            truncate_results: vec![],
        };
        let out = apply(&h, &d, TRUNCATE_HEAD_CHARS);
        assert_eq!(out.len(), h.len(), "a drop removes no message");
        assert!(out.iter().zip(&h).all(|(a, b)| a.role == b.role));
        assert_eq!(out[1].content.len(), 3, "both tool_use blocks remain");
        match &out[1].content[1] {
            ContentBlock::ToolUse {
                id, name, input, ..
            } => {
                assert_eq!(id, "toolu_read");
                assert_eq!(name, "read_file");
                assert_eq!(input, &json!({}), "the dropped call's input is emptied");
            }
            other => panic!("expected the dropped tool_use block, got {other:?}"),
        }
        let json_of = |b: &ContentBlock| serde_json::to_string(b).unwrap();
        assert_eq!(json_of(&out[1].content[0]), json_of(&h[1].content[0]));
        assert_eq!(json_of(&out[1].content[2]), json_of(&h[1].content[2]));
        assert_eq!(out[2].content.len(), 2, "both results remain");
        match &out[2].content[0] {
            ContentBlock::ToolResult {
                tool_use_id,
                content,
                ..
            } => {
                assert_eq!(tool_use_id, "toolu_read");
                let text = content.as_text();
                assert!(text.starts_with("[dropped to save context"), "{text}");
                assert!(text.len() < 80, "the stub is short: {text}");
            }
            other => panic!("expected the stub result, got {other:?}"),
        }
        assert_eq!(json_of(&out[2].content[1]), json_of(&h[2].content[1]));
        // Applying the same decision again changes nothing: a reload replays
        // every `pruned` record over a history that may already hold stubs.
        let twice = apply(&out, &d, TRUNCATE_HEAD_CHARS);
        assert_eq!(
            serde_json::to_string(&twice).unwrap(),
            serde_json::to_string(&out).unwrap()
        );
    }

    #[test]
    fn apply_keeps_the_text_of_a_message_whose_call_is_dropped() {
        let h = session(3);
        let d = Decision {
            drop_calls: vec!["toolu_0".into()],
            truncate_results: vec![],
        };
        let out = apply(&h, &d, TRUNCATE_HEAD_CHARS);
        let text = serde_json::to_string(&out).unwrap();
        assert!(text.contains("Let me look around."));
        assert!(out.windows(2).all(|w| w[0].role != w[1].role));
    }

    #[test]
    fn apply_truncates_a_result_to_a_head_and_a_note() {
        let h = session(3);
        let d = Decision {
            drop_calls: vec![],
            truncate_results: vec!["toolu_2".into()],
        };
        let out = apply(&h, &d, TRUNCATE_HEAD_CHARS);
        let r = out
            .iter()
            .flat_map(|m| m.content.iter())
            .find_map(|b| match b {
                ContentBlock::ToolResult {
                    tool_use_id,
                    content,
                    ..
                } if tool_use_id == "toolu_2" => Some(content.as_text().into_owned()),
                _ => None,
            })
            .unwrap();
        assert!(r.len() < TRUNCATE_HEAD_CHARS + 200, "{}", r.len());
        assert!(r.starts_with("fn body()"));
        assert!(r.contains("re-run the tool"));
    }

    #[test]
    fn apply_never_touches_user_or_assistant_text() {
        let h = session(4);
        let d = Decision {
            drop_calls: vec![
                "toolu_0".into(),
                "toolu_1".into(),
                "toolu_2".into(),
                "toolu_3".into(),
            ],
            truncate_results: vec![],
        };
        let out = apply(&h, &d, TRUNCATE_HEAD_CHARS);
        let texts = |h: &[Message]| -> Vec<String> {
            h.iter()
                .flat_map(|m| m.content.iter())
                .filter_map(|b| b.get_text().map(str::to_string))
                .collect()
        };
        assert_eq!(texts(&out), texts(&h));
    }

    #[test]
    fn history_chars_counts_text_inputs_and_results() {
        let h = vec![user("abc"), result("t", "12345")];
        assert_eq!(history_chars(&h), 8);
    }

    /// Jev's answers: P(keep result) per short call id (`t1`, `t2`, …).
    fn jev_answers(results: &[(usize, f64)]) -> &'static str {
        let mut answers = serde_json::Map::new();
        for (i, p) in results {
            answers.insert(format!("result_t{i}"), json!({"type": "noul", "noul": p}));
        }
        Box::leak(
            json!({"answers": answers, "usage": {"input_tokens": 100, "cost": 1e-6}})
                .to_string()
                .into_boxed_str(),
        )
    }

    // Behaviour deliberately changed by lean-harness Phase 6 (this was
    // `plan_asks_jev_and_returns_what_to_drop`): Jev is asked one question per
    // call masking would cut — does its output still have to stay verbatim —
    // and answers which of them stay.
    #[tokio::test]
    async fn arbitration_asks_one_question_per_candidate_and_returns_what_stays() {
        let (url, h) = spawn_stub(200, jev_answers(&[(2, 0.9), (3, 0.1)]));
        let hist = session(4);
        let ids = vec!["toolu_1".to_string(), "toolu_2".to_string()];
        let out = arbitrate(&hist, &ids, &stub_backend(&url))
            .await
            .expect("an answer");
        let request = h.join().unwrap();
        assert_eq!(out.requests, 1);
        assert_eq!(out.keep, vec!["toolu_1".to_string()]);
        assert_eq!(
            out.judged,
            vec![("toolu_1".to_string(), 0.9), ("toolu_2".to_string(), 0.1)]
        );
        assert!(out.cost > 0.0);
        assert!(request.contains("result_t2") && request.contains("result_t3"));
        assert!(
            !request.contains("result_t1") && !request.contains("call_t"),
            "only the candidates are asked about, and only about their output"
        );
    }

    #[tokio::test]
    async fn a_jev_failure_is_no_arbitration() {
        let (url, h) = spawn_stub(529, r#"{"error":"overloaded"}"#);
        let ids = vec!["toolu_1".to_string()];
        assert!(
            arbitrate(&session(4), &ids, &stub_backend(&url))
                .await
                .is_none()
        );
        h.join().unwrap();
    }

    #[tokio::test]
    async fn nothing_to_arbitrate_is_no_arbitration() {
        let b = stub_backend("http://127.0.0.1:9/unused");
        assert!(arbitrate(&session(4), &[], &b).await.is_none());
    }

    // ── Stage 0: lossless ────────────────────────────────────────────────────

    fn tasks(n: usize) -> Value {
        json!({"tasks": (0..n).map(|i| json!({
            "id": format!("t{i}"), "title": "task", "description": "d".repeat(200),
            "journal": [], "status": "todo"
        })).collect::<Vec<_>>()})
    }

    /// A Builder run: the list is loaded, then re-sent whole on every status
    /// change, with some unrelated work in between and a recent tail.
    fn task_session() -> Vec<Message> {
        let mut h = vec![user("Build the feature.")];
        h.push(assistant_call("", "get_1", "tasks_get", json!({})));
        h.push(result("get_1", &tasks(8).to_string()));
        for i in 0..4 {
            h.push(assistant_call(
                "",
                &format!("set_{i}"),
                "tasks_set",
                tasks(8),
            ));
            h.push(result(&format!("set_{i}"), "Tasks updated."));
            h.push(assistant_call(
                "",
                &format!("bash_{i}"),
                "bash",
                json!({"command": "cargo test"}),
            ));
            h.push(result(&format!("bash_{i}"), "ok"));
        }
        // A rejected write is not a snapshot of anything.
        h.push(assistant_call("", "set_bad", "tasks_set", tasks(8)));
        h.push(result("set_bad", "Error: quality gate is red"));
        for i in 0..4 {
            h.push(assistant(&format!("note {i}")));
            h.push(user("go on"));
        }
        h
    }

    #[test]
    fn task_snapshots_older_than_the_latest_good_one_are_superseded() {
        let h = task_session();
        let d = plan_lossless(&h);
        // set_3 is the latest successful snapshot: it stays, and so does the
        // rejected write after it. Everything about tasks before it goes.
        assert_eq!(d.drop_calls, vec!["get_1", "set_0", "set_1", "set_2"]);
        assert!(d.truncate_results.is_empty());
        let after = apply(&h, &d, TRUNCATE_HEAD_CHARS);
        let ids: Vec<String> = after
            .iter()
            .flat_map(|m| m.content.iter())
            .filter_map(|b| match b {
                ContentBlock::ToolUse { id, .. } => Some(id.clone()),
                _ => None,
            })
            .collect();
        assert!(ids.contains(&"set_3".to_string()));
        assert!(ids.contains(&"set_bad".to_string()));
        // A superseded snapshot keeps its tool_use block, emptied.
        let input_of = |id: &str| {
            after
                .iter()
                .flat_map(|m| m.content.iter())
                .find_map(|b| match b {
                    ContentBlock::ToolUse { id: i, input, .. } if i == id => Some(input.clone()),
                    _ => None,
                })
        };
        assert_eq!(input_of("set_0"), Some(json!({})));
        assert_eq!(input_of("set_3"), Some(tasks(8)));
        // Unrelated calls are not this stage's business.
        assert_eq!(ids.iter().filter(|id| id.starts_with("bash_")).count(), 4);
        assert!(history_chars(&after) < history_chars(&h) / 2);
    }

    #[test]
    fn deltas_after_the_latest_snapshot_stay_and_older_ones_go() {
        let mut h = vec![user("Build.")];
        h.push(assistant_call(
            "",
            "upd_old",
            "tasks_update",
            json!({"id": "t0"}),
        ));
        h.push(result("upd_old", "t0: doing"));
        h.push(assistant_call("", "set_1", "tasks_set", tasks(3)));
        h.push(result("set_1", "Tasks updated."));
        h.push(assistant_call(
            "",
            "upd_new",
            "tasks_update",
            json!({"id": "t1"}),
        ));
        h.push(result("upd_new", "t1: done"));
        for _ in 0..4 {
            h.push(assistant("…"));
            h.push(user("go on"));
        }
        // The delta before the snapshot is folded into it; the one after is
        // the only record of that change.
        assert_eq!(plan_lossless(&h).drop_calls, vec!["upd_old"]);
    }

    #[test]
    fn a_read_repeated_later_with_the_same_range_is_cut_to_its_head() {
        let body = "fn body() {}\n".repeat(400);
        let mut h = vec![user("Look at lib.rs.")];
        let read = |h: &mut Vec<Message>, id: &str, input: Value, text: &str| {
            h.push(assistant_call("", id, "read_file", input));
            h.push(result(id, text));
        };
        read(&mut h, "r1", json!({"path": "src/lib.rs"}), &body);
        read(
            &mut h,
            "r2",
            json!({"path": "src/lib.rs", "start_line": 1, "end_line": 50}),
            &body,
        );
        read(&mut h, "r3", json!({"path": "src/other.rs"}), &body);
        // `file_path` is an accepted alias of `path`.
        read(&mut h, "r4", json!({"file_path": "src/lib.rs"}), &body);
        read(
            &mut h,
            "r5",
            json!({"path": "src/lib.rs", "start_line": 1, "end_line": 50}),
            &body,
        );
        for _ in 0..4 {
            h.push(assistant("…"));
            h.push(user("go on"));
        }
        let d = plan_lossless(&h);
        // r1 is repeated by r4 and r2 by r5. r3 was read once.
        assert_eq!(d.truncate_results, vec!["r1", "r2"]);
        assert!(d.drop_calls.is_empty(), "the calls stay visible");
    }

    #[test]
    fn a_read_is_not_superseded_by_a_failed_or_an_already_cut_one() {
        let body = "fn body() {}\n".repeat(400);
        let mut h = vec![user("Look.")];
        h.push(assistant_call(
            "",
            "r1",
            "read_file",
            json!({"path": "a.rs"}),
        ));
        h.push(result("r1", &body));
        h.push(assistant_call(
            "",
            "r2",
            "read_file",
            json!({"path": "a.rs"}),
        ));
        h.push(result("r2", "Error: cannot access: No such file"));
        h.push(assistant_call(
            "",
            "r3",
            "read_file",
            json!({"path": "b.rs"}),
        ));
        h.push(result("r3", &body));
        h.push(assistant_call(
            "",
            "r4",
            "read_file",
            json!({"path": "b.rs"}),
        ));
        h.push(result("r4", "fn body() {}\n[compacted: 5000 chars of this tool result removed; re-run the tool if needed]"));
        for _ in 0..4 {
            h.push(assistant("…"));
            h.push(user("go on"));
        }
        // r1 and r3 are each the only whole copy left of their file.
        assert!(plan_lossless(&h).is_empty());
    }

    /// A history in which `reads` (id, input, result) happened in order.
    fn history_of_reads(reads: &[(&str, Value, &str)]) -> Vec<Message> {
        let mut h = vec![user("Look at lib.rs.")];
        for (id, input, text) in reads {
            h.push(assistant_call("", id, "read_file", input.clone()));
            h.push(result(id, text));
        }
        h
    }

    #[test]
    fn a_read_the_conversation_already_holds_becomes_a_pointer_to_it() {
        let body = "fn body() {}\n".repeat(400);
        let lib = json!({"path": "src/lib.rs"});
        let h = history_of_reads(&[("r1", lib.clone(), &body)]);

        let pointer = repeated_read(&h, &lib, &body).expect("an exact repeat");
        assert!(pointer.starts_with(UNCHANGED_MARK));
        assert!(pointer.contains("src/lib.rs") && pointer.contains("400 lines"));
        assert!(
            pointer.len() < 400,
            "{} chars for {}",
            pointer.len(),
            body.len()
        );
        // `file_path` is the same argument under its other name.
        assert!(repeated_read(&h, &json!({"file_path": "src/lib.rs"}), &body).is_some());

        // A range is named, so the model knows which earlier read is meant.
        let part = json!({"path": "src/lib.rs", "start_line": 10, "end_line": 90});
        let h = history_of_reads(&[("r1", part.clone(), &body)]);
        assert!(
            repeated_read(&h, &part, &body)
                .unwrap()
                .contains("src/lib.rs lines 10-90 is identical")
        );
    }

    #[test]
    fn a_read_is_returned_whole_when_the_earlier_one_cannot_stand_in_for_it() {
        let body = "fn body() {}\n".repeat(400);
        let lib = json!({"path": "src/lib.rs"});

        // Nothing read yet, or only another file or another range.
        assert!(repeated_read(&history_of_reads(&[]), &lib, &body).is_none());
        let other = history_of_reads(&[
            ("r1", json!({"path": "src/other.rs"}), &body),
            (
                "r2",
                json!({"path": "src/lib.rs", "start_line": 1, "end_line": 50}),
                &body,
            ),
        ]);
        assert!(repeated_read(&other, &lib, &body).is_none());

        // The file changed in between: the model edited it, or someone did.
        let edited = format!("{body}fn added() {{}}\n");
        let h = history_of_reads(&[("r1", lib.clone(), &body)]);
        assert!(repeated_read(&h, &lib, &edited).is_none());

        // The earlier result was cut by a prune: what it pointed at is gone.
        let cut = format!(
            "{}\n{COMPACTED_MARK}5000 chars of this tool result removed; re-run the tool if needed]",
            &body[..300]
        );
        let h = history_of_reads(&[("r1", lib.clone(), &cut)]);
        assert!(repeated_read(&h, &lib, &body).is_none());

        // Too small to be worth a pointer.
        let small = "fn main() {}\n";
        let h = history_of_reads(&[("r1", lib.clone(), small)]);
        assert!(repeated_read(&h, &lib, small).is_none());

        // A read that failed twice is two errors, not a repeat.
        let error = format!("Error: cannot read as text: {}", "x".repeat(700));
        let h = history_of_reads(&[("r1", lib.clone(), &error)]);
        assert!(repeated_read(&h, &lib, &error).is_none());
    }

    // The model asked again right after being pointed back: it could not use
    // the pointer. One wasted round is the most this may ever cost.
    #[test]
    fn a_pointer_is_never_answered_with_another_pointer() {
        let body = "fn body() {}\n".repeat(400);
        let lib = json!({"path": "src/lib.rs"});
        let pointer = repeated_read(
            &history_of_reads(&[("r1", lib.clone(), &body)]),
            &lib,
            &body,
        )
        .unwrap();

        let asked_again =
            history_of_reads(&[("r1", lib.clone(), &body), ("r2", lib.clone(), &pointer)]);
        assert!(repeated_read(&asked_again, &lib, &body).is_none());

        // …and once it has the file again, that copy can be pointed at.
        let given_again = history_of_reads(&[
            ("r1", lib.clone(), &body),
            ("r2", lib.clone(), &pointer),
            ("r3", lib.clone(), &body),
        ]);
        assert!(repeated_read(&given_again, &lib, &body).is_some());
    }

    // The lossless stage cuts a read that a later one repeats. A pointer is a
    // later read of the same range — and the one thing it must never do is get
    // the result it points at removed.
    #[test]
    fn a_pointer_does_not_supersede_the_read_it_points_at() {
        let body = "fn body() {}\n".repeat(400);
        let lib = json!({"path": "src/lib.rs"});
        let pointer = repeated_read(
            &history_of_reads(&[("r1", lib.clone(), &body)]),
            &lib,
            &body,
        )
        .unwrap();
        let mut h = history_of_reads(&[("r1", lib.clone(), &body), ("r2", lib.clone(), &pointer)]);
        for _ in 0..4 {
            h.push(assistant("…"));
            h.push(user("go on"));
        }
        assert!(plan_lossless(&h).is_empty(), "r1 is the only copy there is");

        // A real later copy still supersedes the first; the pointer between
        // them is left alone — there is nothing in it to cut.
        let mut h = history_of_reads(&[
            ("r1", lib.clone(), &body),
            ("r2", lib.clone(), &pointer),
            ("r3", lib.clone(), &body),
        ]);
        for _ in 0..4 {
            h.push(assistant("…"));
            h.push(user("go on"));
        }
        assert_eq!(plan_lossless(&h).truncate_results, vec!["r1"]);
    }

    #[test]
    fn only_a_repeated_read_is_swapped_for_its_history_copy() {
        let body = "fn body() {}\n".repeat(400);
        let lib = json!({"path": "src/lib.rs"});
        let h = history_of_reads(&[("r1", lib.clone(), &body)]);
        let text_of = |b: ContentBlock| match b {
            ContentBlock::ToolResult {
                tool_use_id,
                content,
                ..
            } => (tool_use_id, content.as_text().into_owned()),
            _ => panic!("a tool result"),
        };

        let (id, text) = text_of(history_copy(
            &h,
            "read_file",
            &lib,
            ContentBlock::tool_result("r2", body.clone()),
        ));
        assert_eq!(id, "r2", "the pointer answers the call that asked");
        assert!(text.starts_with(UNCHANGED_MARK));

        // Another tool returning the same text is not a read of that file.
        let (_, text) = text_of(history_copy(
            &h,
            "bash",
            &json!({"command": "cat src/lib.rs"}),
            ContentBlock::tool_result("b1", body.clone()),
        ));
        assert_eq!(text, body);
    }

    /// `reads`, then enough later turns that none of them is pinned as recent.
    fn old_reads(reads: &[(&str, Value, &str)]) -> Vec<Message> {
        let mut h = history_of_reads(reads);
        for _ in 0..4 {
            h.push(assistant("…"));
            h.push(user("go on"));
        }
        h
    }

    // A later call that never ran says nothing about the file. Cutting the
    // earlier read on its account would remove the only copy there is.
    #[test]
    fn a_read_that_never_ran_does_not_supersede_the_one_that_did() {
        let body = "fn body() {}\n".repeat(400);
        let lib = json!({"path": "src/lib.rs"});
        for not_run in [
            "Interrupted by the user — the tool was not run.",
            "Tool call rejected by user",
            // A hook's own reason: no prefix to know it by, only its size.
            "This path is off limits until the migration lands. Ask in #infra first.",
        ] {
            let h = old_reads(&[("r1", lib.clone(), &body), ("r2", lib.clone(), not_run)]);
            assert!(plan_lossless(&h).is_empty(), "{not_run}");
        }
    }

    // The result cap follows the model, and a mode switch can change the
    // model: the same file read again under a smaller cap is less of it.
    #[test]
    fn a_read_cut_by_a_smaller_cap_does_not_replace_a_fuller_copy() {
        let body = "fn body() {}\n".repeat(400);
        let lib = json!({"path": "src/lib.rs"});
        let capped = |chars: usize| {
            format!(
                "{}...(truncated, {} chars total)",
                &body[..chars],
                body.len()
            )
        };

        let shrunk = old_reads(&[
            ("r1", lib.clone(), &body),
            ("r2", lib.clone(), &capped(2_000)),
        ]);
        assert!(plan_lossless(&shrunk).is_empty());

        // Under the same cap the two are the same text, and one is enough.
        let same = old_reads(&[
            ("r1", lib.clone(), &capped(2_000)),
            ("r2", lib.clone(), &capped(2_000)),
        ]);
        assert_eq!(plan_lossless(&same).truncate_results, vec!["r1"]);
    }

    // The same rule for a read the Phase 6 cap cut (head, marker, tail): it
    // is a real result, but not all of the file.
    #[test]
    fn a_read_cut_by_the_head_and_tail_cap_does_not_replace_a_fuller_copy() {
        let body = "fn body() {}\n".repeat(400);
        let lib = json!({"path": "src/lib.rs"});
        // More than half the file: only knowing it was capped keeps the
        // fuller copy.
        let capped = crate::agent::result_cap::cap(
            crate::agent::result_cap::Call::tool("read_file"),
            &body,
            3_000,
        );
        assert!(capped.len() * 2 > body.len());
        let shrunk = old_reads(&[("r1", lib.clone(), &body), ("r2", lib.clone(), &capped)]);
        assert!(plan_lossless(&shrunk).is_empty());
        let same = old_reads(&[("r1", lib.clone(), &capped), ("r2", lib.clone(), &capped)]);
        assert_eq!(plan_lossless(&same).truncate_results, vec!["r1"]);
    }

    #[test]
    fn a_task_call_that_never_ran_is_not_the_current_list() {
        let list = json!({"tasks": [{"id": "t1", "title": "x".repeat(300), "status": "todo"}]});
        let mut h = vec![user("Do it.")];
        h.push(assistant_call("", "set1", "tasks_set", list.clone()));
        h.push(result("set1", "Tasks updated (1 total)"));
        h.push(assistant_call("", "set2", "tasks_set", list));
        h.push(result(
            "set2",
            "Interrupted by the user — the tool was not run.",
        ));
        for _ in 0..4 {
            h.push(assistant("…"));
            h.push(user("go on"));
        }
        // set1 is still the list the session is working from.
        assert!(plan_lossless(&h).drop_calls.is_empty());
    }

    // What the user said through `ask_user`, and the task list as it stands,
    // are not "old tool output": neither can be had again by re-running a tool
    // on the same terms.
    // A dropped call becomes `{}` and a 43-character note. A call already
    // smaller than that (`ls` answered with two names) would grow the history
    // when stubbed, and cost prefix for nothing: it is never dropped, while a
    // call that does shrink still is.
    #[test]
    fn by_age_never_stubs_a_call_smaller_than_its_stub() {
        let big = "x".repeat(5_000);
        let mut h = vec![user("Tidy the repo.")];
        h.push(assistant_call("", "ls", "bash", json!({"command": "ls"})));
        h.push(result("ls", "a\nb"));
        h.push(assistant_call(
            "",
            "read",
            "read_file",
            json!({"path": "a"}),
        ));
        h.push(result("read", &big));
        for _ in 0..4 {
            h.push(assistant("…"));
            h.push(user("go on"));
        }
        let d = plan_by_age(&h, usize::MAX, usize::MAX, true);
        assert!(!d.drop_calls.iter().any(|id| id == "ls"), "{d:?}");
        assert!(d.drop_calls.iter().any(|id| id == "read"), "{d:?}");
    }

    #[test]
    fn by_age_never_cuts_the_users_answers_or_the_current_task_list() {
        let big = "x".repeat(5_000);
        let mut h = vec![user("Plan the export feature.")];
        let call = |h: &mut Vec<Message>, id: &str, tool: &str, text: &str| {
            h.push(assistant_call("", id, tool, json!({"n": id})));
            h.push(result(id, text));
        };
        call(
            &mut h,
            "ask1",
            "ask_user",
            &format!("Question: format?\nAnswer: CSV. {big}"),
        );
        call(&mut h, "old_get", "tasks_get", &big);
        call(&mut h, "read1", "read_file", &big);
        call(&mut h, "get", "tasks_get", &big);
        call(&mut h, "upd", "tasks_update", &big);
        call(&mut h, "read2", "read_file", &big);
        for _ in 0..4 {
            h.push(assistant("…"));
            h.push(user("go on"));
        }
        for drop_calls in [false, true] {
            let d = plan_by_age(&h, usize::MAX, usize::MAX, drop_calls);
            let touched: Vec<&String> = d.truncate_results.iter().chain(&d.drop_calls).collect();
            for kept in ["ask1", "get", "upd"] {
                assert!(
                    !touched.iter().any(|id| *id == kept),
                    "{kept} ({drop_calls})"
                );
            }
            // Ordinary results, and a task list a newer one replaced, still go.
            for cut in ["old_get", "read1", "read2"] {
                assert!(touched.iter().any(|id| *id == cut), "{cut} ({drop_calls})");
            }
        }
    }

    // A task list too long for the result cap is never "whole", so the
    // lossless stage has no snapshot to drop the older copies against. They
    // must then be as cuttable by age as any old result — only the newest one
    // is the list the model is working from.
    #[test]
    fn older_copies_of_a_task_list_too_long_for_the_cap_are_still_cut_by_age() {
        let capped = format!("{}...(truncated, 40000 chars total)", "t".repeat(15_000));
        let mut h = vec![user("Work through the list.")];
        h.push(assistant_call("", "set", "tasks_set", json!({"tasks": []})));
        h.push(result("set", "Tasks updated (60 total)"));
        for i in 0..4 {
            h.push(assistant_call(
                "",
                &format!("get{i}"),
                "tasks_get",
                json!({}),
            ));
            h.push(result(&format!("get{i}"), &capped));
            h.push(assistant_call(
                "",
                &format!("upd{i}"),
                "tasks_update",
                // A journal entry: a bare status change is smaller than the
                // stub a drop leaves, and is never dropped.
                json!({"id": "t1", "journal": ["wired the CSV writer into the export"]}),
            ));
            h.push(result(&format!("upd{i}"), "'t1': todo -> doing"));
        }
        for _ in 0..4 {
            h.push(assistant("…"));
            h.push(user("go on"));
        }
        let d = plan_by_age(&h, usize::MAX, usize::MAX, false);
        assert_eq!(d.truncate_results, vec!["get0", "get1", "get2"]);
        // The newest list and the change made after it are what is current.
        let d = plan_by_age(&h, usize::MAX, usize::MAX, true);
        let touched: Vec<&String> = d.truncate_results.iter().chain(&d.drop_calls).collect();
        assert!(
            !touched.iter().any(|id| *id == "get3" || *id == "upd3"),
            "{touched:?}"
        );
        assert!(touched.iter().any(|id| *id == "upd0"));
    }

    // Applying a decision to a history it was already applied to changes
    // nothing: a session applies each prune live and again, with every later
    // one, on reload.
    #[test]
    fn a_result_that_was_cut_is_not_cut_again() {
        let report =
            "## worker — completed (9 rounds)\n".to_string() + &"edited a file\n".repeat(900);
        let mut h = vec![user("Build it.")];
        h.push(assistant_call(
            "",
            "sp",
            "spawn_agents",
            json!({"agents": []}),
        ));
        h.push(result("sp", &report));
        h.push(assistant_call(
            "",
            "rd",
            "read_file",
            json!({"path": "a.rs"}),
        ));
        h.push(result("rd", &format!("Error: {report}")));
        for _ in 0..4 {
            h.push(assistant("…"));
            h.push(user("go on"));
        }
        let d = Decision {
            truncate_results: vec!["sp".into(), "rd".into()],
            ..Default::default()
        };
        let once = apply(&h, &d, TRUNCATE_HEAD_CHARS);
        let twice = apply(&once, &d, TRUNCATE_HEAD_CHARS);
        assert_eq!(
            serde_json::to_string(&once).unwrap(),
            serde_json::to_string(&twice).unwrap()
        );
        // And cutting by age does not pick an already cut result again.
        assert!(plan_by_age(&once, usize::MAX, usize::MAX, false).is_empty());
        for m in &once {
            for b in &m.content {
                if let ContentBlock::ToolResult { content, .. } = b {
                    assert!(content.as_text().len() <= MAX_CUT_CHARS);
                }
            }
        }
    }

    // A subagent past its hard line takes whatever the shed can still offer.
    // When nothing is old enough to cut by age, that is the lossless stage —
    // offered once more, as the last thing there is.
    #[tokio::test]
    async fn the_lossless_stage_is_offered_as_the_last_one_when_nothing_else_can_be_cut() {
        let list = json!({"tasks": [{"id": "t1", "title": "x".repeat(4_000), "status": "todo"}]});
        let mut h = vec![user("Do it.")];
        for i in 0..5 {
            h.push(assistant_call(
                "",
                &format!("set{i}"),
                "tasks_set",
                list.clone(),
            ));
            h.push(result(&format!("set{i}"), "Tasks updated (1 total)"));
        }
        for _ in 0..4 {
            h.push(assistant("…"));
            h.push(user("go on"));
        }
        let opts = ShedOptions {
            free_chars: usize::MAX,
            recent_chars: usize::MAX,
            last_resort: true,
        };
        let before = history_chars(&h);
        // "Only if it is the last offer": refused as a first stage, taken as the last.
        let shed = shed(&h, None, opts, |outcome, _| outcome.last).await;
        let (outcome, after) = shed.accepted.expect("the older task lists can go");
        assert_eq!(outcome.source, SOURCE_LOSSLESS);
        assert!(history_chars(&after) < before / 2);
        // With nothing to free at all there is still nothing to accept.
        let empty = vec![user("Do it."), assistant("ok")];
        assert!(shed_nothing(&empty, opts).await);
    }

    async fn shed_nothing(history: &[Message], opts: ShedOptions) -> bool {
        shed(history, None, opts, |_, _| true)
            .await
            .accepted
            .is_none()
    }

    #[test]
    fn a_cut_subagent_report_does_not_invite_running_the_work_again() {
        let report =
            "## worker — completed (9 rounds)\n".to_string() + &"edited a file\n".repeat(300);
        let mut h = vec![user("Build it.")];
        h.push(assistant_call(
            "",
            "sp",
            "spawn_agents",
            json!({"agents": []}),
        ));
        h.push(result("sp", &report));
        h.push(assistant_call(
            "",
            "rd",
            "read_file",
            json!({"path": "a.rs"}),
        ));
        h.push(result("rd", &report));
        let d = Decision {
            truncate_results: vec!["sp".into(), "rd".into()],
            ..Default::default()
        };
        let out = apply(&h, &d, TRUNCATE_HEAD_CHARS);
        let text = |i: usize| match &out[i].content[0] {
            ContentBlock::ToolResult { content, .. } => content.as_text().into_owned(),
            _ => panic!("a tool result"),
        };
        assert!(text(2).contains(COMPACTED_MARK) && !text(2).contains("re-run"));
        assert!(text(2).contains("do not spawn it again"));
        assert!(text(4).contains("re-run the tool if needed"));
    }

    #[test]
    fn the_newest_messages_are_never_touched_by_the_lossless_stage() {
        let body = "fn body() {}\n".repeat(400);
        let mut h = vec![user("Look.")];
        for id in ["r1", "r2"] {
            h.push(assistant_call("", id, "read_file", json!({"path": "a.rs"})));
            h.push(result(id, &body));
        }
        // Five messages: both reads are inside the pinned window.
        assert!(plan_lossless(&h).is_empty());
    }

    // ── Stage 2: by age ──────────────────────────────────────────────────────

    #[test]
    fn by_age_cuts_the_oldest_results_first_and_stops_when_enough_is_freed() {
        let h = session(8);
        let one = "fn body() {}\n".repeat(400).len();
        // Asking for a little more than one result frees exactly two.
        let d = plan_by_age(&h, one + 10, usize::MAX, false);
        assert_eq!(d.truncate_results, vec!["toolu_0", "toolu_1"]);
        assert!(d.drop_calls.is_empty(), "by age never drops a call");
        // Asking for everything stops at the recent window.
        let all = plan_by_age(&h, usize::MAX, usize::MAX, false);
        assert_eq!(all.truncate_results.len(), 8);
        assert!(!all.truncate_results.contains(&"toolu_edit".to_string()));
    }

    #[test]
    fn by_age_skips_results_too_small_to_be_worth_a_note() {
        let mut h = vec![user("go")];
        for i in 0..6 {
            h.push(assistant_call(
                "",
                &format!("b{i}"),
                "bash",
                json!({"command": "true"}),
            ));
            h.push(result(&format!("b{i}"), "ok"));
        }
        for _ in 0..3 {
            h.push(assistant("…"));
            h.push(user("go on"));
        }
        assert!(plan_by_age(&h, usize::MAX, usize::MAX, false).is_empty());
    }

    #[test]
    fn the_recent_window_shrinks_to_fit_its_budget_but_keeps_the_last_round() {
        let big = "x".repeat(9_000);
        let mut h = vec![user("go")];
        for i in 0..5 {
            h.push(assistant_call(
                "",
                &format!("r{i}"),
                "read_file",
                json!({"path": format!("f{i}")}),
            ));
            h.push(result(&format!("r{i}"), &big));
        }
        // Eleven messages. A roomy budget pins the usual six: r2, r3, r4.
        assert_eq!(recent_start(&h, usize::MAX), 5);
        assert_eq!(
            plan_by_age(&h, usize::MAX, usize::MAX, false).truncate_results,
            vec!["r0", "r1"]
        );
        // A budget one result wide pins only the last round, so the two
        // before it become old enough to cut — which is what lets a small
        // window shed anything at all.
        assert_eq!(recent_start(&h, 10_000), 9);
        assert_eq!(
            plan_by_age(&h, usize::MAX, 10_000, false).truncate_results,
            vec!["r0", "r1", "r2", "r3"]
        );
        // No budget at all still leaves the last round alone.
        assert_eq!(recent_start(&h, 0), 9);
    }

    #[test]
    fn only_a_last_resort_drops_calls_and_only_when_cutting_falls_short() {
        let h = session(8);
        let one = "fn body() {}\n".repeat(400).len();
        // Cutting is enough: no call is dropped even when allowed.
        let enough = plan_by_age(&h, one * 3, usize::MAX, true);
        assert!(enough.drop_calls.is_empty());
        assert_eq!(enough.truncate_results.len(), 4);
        // Cutting all eight results frees less than eight whole results, so
        // the oldest calls go — oldest first, and no more than needed.
        let all_cut = 8 * (one - TRUNCATE_HEAD_CHARS);
        let short = plan_by_age(&h, all_cut + 500, usize::MAX, true);
        assert_eq!(short.drop_calls, vec!["toolu_0", "toolu_1"]);
        assert_eq!(short.truncate_results.len(), 6);
        assert!(!short.truncate_results.contains(&"toolu_0".to_string()));
        // Not allowed: the same shortfall leaves every call in place.
        let kept = plan_by_age(&h, all_cut + 500, usize::MAX, false);
        assert!(kept.drop_calls.is_empty());
        assert_eq!(kept.truncate_results.len(), 8);
    }

    // ── The ladder ───────────────────────────────────────────────────────────

    fn opts(last_resort: bool) -> ShedOptions {
        ShedOptions {
            // More than cutting every old result can free, so a last resort
            // has to go on to dropping calls.
            free_chars: usize::MAX,
            recent_chars: usize::MAX,
            last_resort,
        }
    }

    /// Accept what frees at least `min` of the history.
    fn frees(min: f64) -> impl Fn(&Outcome, &[Message]) -> bool {
        move |o, _| o.reduction() >= min
    }

    #[tokio::test]
    async fn without_jev_the_ladder_falls_back_to_age_and_touches_no_prose() {
        let h = session(8);
        let shed = shed(&h, None, opts(false), frees(MIN_REDUCTION)).await;
        let (outcome, after) = shed.accepted.expect("by age frees enough");
        assert_eq!(outcome.source, SOURCE_RULES);
        assert_eq!(outcome.requests, 0);
        assert_eq!(shed.cost, 0.0);
        // The decision alone reproduces the history: that is what makes a
        // resumed session identical to the live one.
        let replayed = apply(&h, &outcome.decision, TRUNCATE_HEAD_CHARS);
        assert_eq!(
            serde_json::to_string(&after).unwrap(),
            serde_json::to_string(&replayed).unwrap()
        );
        // Every word a person or the model wrote is still there, in order.
        let prose = |h: &[Message]| -> Vec<String> {
            h.iter()
                .flat_map(|m| m.content.iter())
                .filter_map(|b| b.get_text().map(str::to_string))
                .collect()
        };
        assert_eq!(prose(&after), prose(&h));
        assert!(prose(&after)[0].contains("Never edit src/generated"));
    }

    #[tokio::test]
    async fn the_lossless_stage_alone_is_taken_when_it_frees_enough() {
        let h = task_session();
        // No backend is passed, and none is needed: nothing was judged.
        let shed = shed(&h, None, opts(false), frees(MIN_REDUCTION)).await;
        let (outcome, _) = shed.accepted.expect("superseded snapshots are most of it");
        assert_eq!(outcome.source, SOURCE_LOSSLESS);
        assert_eq!(outcome.decision, plan_lossless(&h));
        assert_eq!(outcome.stats_json()["source"], "lossless");
    }

    fn jev_keeps_everything(calls: usize) -> &'static str {
        let all: Vec<(usize, f64)> = (1..=calls).map(|i| (i, 0.9)).collect();
        jev_answers(&all)
    }

    // Behaviour deliberately changed by lean-harness Phase 6 (this was
    // `what_jev_chose_to_keep_is_kept_in_a_main_session`, which handed off):
    // masking comes first and Jev only arbitrates its recent cuts. When what
    // Jev keeps leaves too little freed, the cuts masking alone would make are
    // offered last — a handoff or a summary loses far more than a few
    // re-readable outputs.
    #[tokio::test]
    async fn when_what_jev_keeps_is_not_enough_masking_alone_is_offered_last() {
        let (url, stub) = spawn_stub(200, jev_keeps_everything(4));
        let h = session(4);
        let shed = shed(
            &h,
            Some(&stub_backend(&url)),
            opts(false),
            frees(MIN_REDUCTION),
        )
        .await;
        stub.join().unwrap();
        let (outcome, _) = shed.accepted.expect("masking alone frees enough");
        assert_eq!(outcome.source, SOURCE_RULES);
        assert!(outcome.last);
        assert_eq!(outcome.decision.truncate_results.len(), 4);
        assert!(shed.cost > 0.0, "the question was still paid for");
    }

    /// `session(12)` has twelve reads and an edit; the edit sits in the recent
    /// window, so the eight newest rounds hold the edit and reads 5–11.
    #[test]
    fn the_arbitration_window_is_what_masking_cuts_in_the_eight_newest_rounds() {
        let h = session(12);
        let all = plan_by_age(&h, usize::MAX, usize::MAX, false);
        assert_eq!(all.truncate_results.len(), 12);
        let ids = arbitration_candidates(&h, &all);
        let expected: Vec<String> = (5..12).map(|i| format!("toolu_{i}")).collect();
        assert_eq!(ids, expected);
        // Masking that stops among the old rounds leaves Jev nothing to say.
        let one = "fn body() {}\n".repeat(400).len();
        let old_only = plan_by_age(&h, (one - TRUNCATE_HEAD_CHARS) * 3, usize::MAX, false);
        assert_eq!(old_only.truncate_results.len(), 3);
        assert!(arbitration_candidates(&h, &old_only).is_empty());
    }

    /// Two calls in one assistant message are one round, not two.
    #[test]
    fn a_round_of_several_calls_counts_once_in_the_arbitration_window() {
        let body = "fn body() {}\n".repeat(400);
        let mut h = vec![user("Look.")];
        for i in 0..9 {
            let mut content = vec![ContentBlock::tool_use(
                format!("a{i}"),
                "read_file",
                json!({"path": format!("a{i}")}),
            )];
            content.push(ContentBlock::tool_use(
                format!("b{i}"),
                "read_file",
                json!({"path": format!("b{i}")}),
            ));
            h.push(Message {
                role: "assistant".into(),
                content,
            });
            h.push(Message {
                role: "user".into(),
                content: vec![
                    ContentBlock::tool_result(format!("a{i}"), body.clone()),
                    ContentBlock::tool_result(format!("b{i}"), body.clone()),
                ],
            });
        }
        for _ in 0..3 {
            h.push(assistant("…"));
            h.push(user("go on"));
        }
        let all = plan_by_age(&h, usize::MAX, usize::MAX, false);
        let ids = arbitration_candidates(&h, &all);
        assert_eq!(ids.len(), 16, "{ids:?}");
        assert!(!ids.contains(&"a0".to_string()) && !ids.contains(&"b0".to_string()));
    }

    #[tokio::test]
    async fn jev_only_arbitrates_what_masking_cuts_in_the_eight_newest_rounds() {
        // t6..t12 are reads 5..11; Jev keeps the two newest.
        let answers: Vec<(usize, f64)> = (6..=12)
            .map(|i| (i, if i >= 11 { 0.9 } else { 0.1 }))
            .collect();
        let (url, stub) = spawn_stub(200, jev_answers(&answers));
        let h = session(12);
        let shed = shed(
            &h,
            Some(&stub_backend(&url)),
            opts(false),
            frees(MIN_REDUCTION),
        )
        .await;
        let request = stub.join().unwrap();
        let (outcome, after) = shed.accepted.expect("masking with Jev's keeps");
        assert_eq!(outcome.source, SOURCE_JEV);
        assert_eq!(outcome.requests, 1);
        assert_eq!(outcome.judged.len(), 7);
        let cut: Vec<String> = (0..10).map(|i| format!("toolu_{i}")).collect();
        assert_eq!(outcome.decision.truncate_results, cut);
        assert!(outcome.decision.drop_calls.is_empty());
        for i in 6..=12 {
            assert!(
                request.contains(&format!("result_t{i}\"")),
                "t{i}: {request}"
            );
        }
        for i in [1, 5, 13] {
            assert!(
                !request.contains(&format!("result_t{i}\"")),
                "t{i} is outside the window or not cut"
            );
        }
        // The decision alone reproduces the history.
        let replayed = apply(&h, &outcome.decision, TRUNCATE_HEAD_CHARS);
        assert_eq!(
            serde_json::to_string(&after).unwrap(),
            serde_json::to_string(&replayed).unwrap()
        );
    }

    // No Jev: the ladder is lossless then masking, nothing else, and the
    // decision is exactly what those two stages make.
    #[tokio::test]
    async fn without_jev_the_decision_is_the_lossless_stage_then_masking() {
        let mut h = task_session();
        let tail = h.split_off(h.len() - 6);
        for i in 0..6 {
            h.push(assistant_call(
                "",
                &format!("r{i}"),
                "read_file",
                json!({"path": format!("f{i}")}),
            ));
            h.push(result(&format!("r{i}"), &"fn body() {}\n".repeat(400)));
        }
        h.extend(tail);
        let shed = shed(&h, None, opts(false), |o, _| o.last).await;
        let (outcome, _) = shed.accepted.expect("masking is offered last");
        let lossless = plan_lossless(&h);
        let after_lossless = apply(&h, &lossless, TRUNCATE_HEAD_CHARS);
        let masking = plan_by_age(&after_lossless, usize::MAX, usize::MAX, false);
        assert_eq!(outcome.decision, lossless.merged(&masking));
        assert_eq!(outcome.source, SOURCE_RULES);
        assert!(outcome.judged.is_empty() && outcome.requests == 0);
        assert_eq!(shed.cost, 0.0);
    }

    #[tokio::test]
    async fn a_subagent_cuts_by_age_even_after_jev_kept_everything() {
        let (url, stub) = spawn_stub(200, jev_keeps_everything(4));
        let h = session(4);
        let shed = shed(
            &h,
            Some(&stub_backend(&url)),
            opts(true),
            frees(MIN_REDUCTION),
        )
        .await;
        stub.join().unwrap();
        let (outcome, _) = shed.accepted.expect("nothing gentler comes after this");
        assert_eq!(outcome.source, SOURCE_RULES);
        assert!(shed.cost > 0.0);
    }

    #[tokio::test]
    async fn a_jev_outage_falls_through_to_age_instead_of_giving_up() {
        let (url, stub) = spawn_stub(529, r#"{"error":"overloaded"}"#);
        let h = session(8);
        let shed = shed(
            &h,
            Some(&stub_backend(&url)),
            opts(false),
            frees(MIN_REDUCTION),
        )
        .await;
        stub.join().unwrap();
        assert_eq!(shed.accepted.expect("by age").0.source, SOURCE_RULES);
    }

    // Behaviour deliberately changed by lean-harness Phase 6 (this was
    // `the_judged_stage_builds_on_the_lossless_one`, where Jev judged every
    // call stage 0 left): Jev now sees only what masking would cut after
    // stage 0, and its keeps come out of that cut.
    #[tokio::test]
    async fn the_arbitration_builds_on_the_lossless_stage() {
        // Two superseded task snapshots, then three reads masking would cut.
        let mut h = vec![user("Build.")];
        for id in ["set_a", "set_b", "set_c"] {
            h.push(assistant_call("", id, "tasks_set", tasks(4)));
            h.push(result(id, "Tasks updated."));
        }
        for i in 0..3 {
            h.push(assistant_call(
                "",
                &format!("r{i}"),
                "read_file",
                json!({"path": format!("f{i}")}),
            ));
            h.push(result(&format!("r{i}"), &"fn body() {}\n".repeat(400)));
        }
        for _ in 0..3 {
            h.push(assistant("…"));
            h.push(user("go on"));
        }
        // After stage 0, set_c is t1 (the current list: masking keeps it) and
        // the reads are t2..t4. Jev keeps the newest read.
        let (url, stub) = spawn_stub(200, jev_answers(&[(2, 0.1), (3, 0.1), (4, 0.9)]));
        // Stage 0 alone is not enough here, so the ladder goes on.
        let shed = shed(&h, Some(&stub_backend(&url)), opts(false), frees(0.5)).await;
        let request = stub.join().unwrap();
        let (outcome, after) = shed.accepted.expect("both stages together");
        assert_eq!(outcome.source, SOURCE_JEV);
        assert_eq!(outcome.decision.drop_calls, vec!["set_a", "set_b"]);
        assert_eq!(outcome.decision.truncate_results, vec!["r0", "r1"]);
        // Jev was never asked about what stage 0 removed or masking kept.
        assert!(request.contains("result_t4"), "{request}");
        assert!(!request.contains("result_t1"), "{request}");
        assert!(!request.contains("result_t5"), "{request}");
        // The decision alone reproduces the history: that is what makes a
        // resumed session identical to the live one.
        let replayed = apply(&h, &outcome.decision, TRUNCATE_HEAD_CHARS);
        assert_eq!(
            serde_json::to_string(&after).unwrap(),
            serde_json::to_string(&replayed).unwrap()
        );
    }

    #[test]
    fn merging_decisions_drops_once_and_never_cuts_what_is_dropped() {
        let a = Decision {
            drop_calls: vec!["x".into(), "y".into()],
            truncate_results: vec!["z".into()],
        };
        let b = Decision {
            drop_calls: vec!["y".into(), "w".into()],
            truncate_results: vec!["x".into(), "z".into(), "v".into()],
        };
        let m = a.merged(&b);
        assert_eq!(m.drop_calls, vec!["x", "y", "w"]);
        assert_eq!(m.truncate_results, vec!["z", "v"]);
        assert!(Decision::default().is_empty());
        assert!(!m.is_empty());
    }

    /// Replay the Jev-less ladder over real session files — the gate for
    /// turning it on. Point `CLAUDINIO_REPLAY_SESSIONS` at a `sessions`
    /// directory:
    ///
    ///   CLAUDINIO_REPLAY_SESSIONS=~/proj/.claudinio/sessions \
    ///     cargo test --lib -- --ignored replay_the_ladder --nocapture
    ///
    /// Every session whose history grew past ~60k tokens is taken as it stood
    /// before its first summarizing compaction, shed without a judge, and
    /// checked: at least `MIN_REDUCTION` freed, and no prose changed.
    #[tokio::test]
    #[ignore]
    async fn replay_the_ladder_on_stored_sessions() {
        let dir = std::env::var("CLAUDINIO_REPLAY_SESSIONS").expect("CLAUDINIO_REPLAY_SESSIONS");
        let prose = |h: &[Message]| -> String {
            h.iter()
                .flat_map(|m| m.content.iter())
                .filter_map(|b| b.get_text())
                .collect::<Vec<_>>()
                .join("\u{1}")
        };
        let (mut seen, mut short, mut reductions) = (0usize, 0usize, Vec::new());
        let mut by_source = std::collections::BTreeMap::<&str, usize>::new();
        for entry in std::fs::read_dir(&dir).expect("readable sessions directory") {
            let path = entry.unwrap().path();
            if path.extension().and_then(|e| e.to_str()) != Some("jsonl") {
                continue;
            }
            let mut records = crate::agent::persist::load_records(&path).unwrap_or_default();
            if let Some(cut) = records
                .iter()
                .position(|r| matches!(r, crate::agent::persist::SessionRecord::Compacted { .. }))
            {
                records.truncate(cut);
            }
            let history = crate::agent::persist::history_from_records(&records);
            if history_chars(&history) < 180_000 {
                continue;
            }
            seen += 1;
            let options = ShedOptions {
                free_chars: history_chars(&history) / 2,
                recent_chars: 60_000,
                last_resort: false,
            };
            let shed = shed(&history, None, options, |o, _| {
                o.reduction() >= MIN_REDUCTION
            })
            .await;
            let Some((outcome, after)) = shed.accepted else {
                short += 1;
                println!("NOT ENOUGH {}", path.display());
                continue;
            };
            assert_eq!(prose(&after), prose(&history), "{}", path.display());
            *by_source.entry(outcome.source).or_default() += 1;
            reductions.push(outcome.reduction());
        }
        reductions.sort_by(|a, b| a.partial_cmp(b).unwrap());
        let at =
            |q: f64| reductions[((reductions.len() as f64 * q) as usize).min(reductions.len() - 1)];
        println!(
            "sessions past 60k tokens: {seen} | freed enough: {} {by_source:?} | fell short: {short}",
            reductions.len()
        );
        if !reductions.is_empty() {
            println!(
                "reduction: min {:.0}% p10 {:.0}% median {:.0}% max {:.0}%",
                reductions[0] * 100.0,
                at(0.1) * 100.0,
                at(0.5) * 100.0,
                reductions[reductions.len() - 1] * 100.0
            );
        }
        assert!(
            seen > 0,
            "no session in {dir} is long enough to say anything"
        );
        assert_eq!(short, 0, "the ladder must free enough on every one");
    }

    /// Live probe against the real Jev. Needs JEV_LIVE_OPENROUTER_KEY.
    #[tokio::test]
    #[ignore]
    async fn live_arbitration_on_a_realistic_session() {
        let key = std::env::var("JEV_LIVE_OPENROUTER_KEY").expect("JEV_LIVE_OPENROUTER_KEY");
        let b = crate::agent::jev::JevBackend {
            source: crate::agent::jev::JevSource::OpenRouter,
            url: crate::agent::jev::OPENROUTER_URL.into(),
            api_key: key,
            model: crate::agent::jev::OPENROUTER_MODEL.into(),
        };
        let mut h = vec![user(
            "The login test fails with 'token expired'. Fix it. Never edit src/generated.",
        )];
        let reads = [
            ("README.md", "# App\nA demo app.\n".repeat(80)),
            ("package.json", "{\"name\":\"app\",\"scripts\":{\"test\":\"vitest\"}}".to_string()),
            ("src/ui/Button.tsx", "export const Button = () => <button/>;\n".repeat(60)),
            (
                "src/auth/token.ts",
                "export function isExpired(t: Token) {\n  return t.exp < Date.now(); // BUG: exp is in seconds, Date.now() is ms\n}\n"
                    .repeat(3),
            ),
            ("src/ui/Card.tsx", "export const Card = () => <div/>;\n".repeat(60)),
        ];
        for (i, (path, body)) in reads.iter().enumerate() {
            h.push(assistant_call(
                "",
                &format!("toolu_{i}"),
                "read_file",
                json!({"path": path}),
            ));
            h.push(result(&format!("toolu_{i}"), body));
        }
        h.push(assistant_call(
            "",
            "toolu_t",
            "bash",
            json!({"command": "npm test -- login"}),
        ));
        h.push(result(
            "toolu_t",
            "FAIL login.test.ts > refreshes token\n  expected valid, got expired\n",
        ));
        h.push(assistant(
            "The bug is in src/auth/token.ts: exp is seconds, Date.now() is ms.",
        ));
        h.push(user("Go ahead and fix it."));
        h.push(assistant_call(
            "",
            "toolu_e",
            "edit_file",
            json!({"path": "src/auth/token.ts"}),
        ));
        h.push(result("toolu_e", "ok"));
        h.push(assistant("Fixed; re-running the test."));
        h.push(user("ok"));
        let calls = collect_tool_calls(&h);
        let ids: Vec<String> = calls
            .iter()
            .filter(|c| !c.pinned)
            .map(|c| c.tool_use_id.clone())
            .collect();
        let out = arbitrate(&h, &ids, &b).await.expect("outcome");
        for c in &calls {
            let action = if out.keep.contains(&c.tool_use_id) {
                "keep"
            } else {
                "cut"
            };
            let p = out.judged.iter().find(|j| j.0 == c.tool_use_id);
            eprintln!(
                "{} {} {} -> {action} {:?}",
                c.id,
                c.tool,
                c.input,
                p.map(|j| j.1)
            );
        }
        eprintln!(
            "requests {} stage {} cost ${:.6}",
            out.requests, out.stage, out.cost
        );
    }

    /// Second live probe: an old result the current work still depends on.
    #[tokio::test]
    #[ignore]
    async fn live_arbitration_keeps_what_the_current_work_needs() {
        let key = std::env::var("JEV_LIVE_OPENROUTER_KEY").expect("JEV_LIVE_OPENROUTER_KEY");
        let b = crate::agent::jev::JevBackend {
            source: crate::agent::jev::JevSource::OpenRouter,
            url: crate::agent::jev::OPENROUTER_URL.into(),
            api_key: key,
            model: crate::agent::jev::OPENROUTER_MODEL.into(),
        };
        let schema = "CREATE TABLE users (id uuid primary key, email text unique not null, plan text not null default 'free', created_at timestamptz);\nCREATE TABLE invoices (id uuid, user_id uuid references users(id), amount_cents int, status text, period_end timestamptz);\n";
        let mut h = vec![user(
            "Write the SQL migrations and the repository layer for invoices, following our schema exactly.",
        )];
        h.push(assistant_call(
            "",
            "toolu_s",
            "read_file",
            json!({"path": "db/schema.sql"}),
        ));
        h.push(result("toolu_s", schema));
        h.push(assistant_call(
            "",
            "toolu_l",
            "list_dir",
            json!({"path": "src"}),
        ));
        h.push(result("toolu_l", "src/main.rs\nsrc/db/\nsrc/http/\n"));
        h.push(assistant_call(
            "",
            "toolu_c",
            "bash",
            json!({"command": "cargo build"}),
        ));
        h.push(result("toolu_c", &"   Compiling dep v1.0\n".repeat(80)));
        h.push(assistant_call(
            "",
            "toolu_r",
            "read_file",
            json!({"path": "src/http/routes.rs"}),
        ));
        h.push(result("toolu_r", &"fn route() {}\n".repeat(100)));
        h.push(assistant("Migration 001 written. Next: the InvoiceRepository with the exact column names from the schema."));
        h.push(user("Continue."));
        h.push(assistant_call(
            "",
            "toolu_w",
            "edit_file",
            json!({"path": "src/db/invoices.rs"}),
        ));
        h.push(result("toolu_w", "ok"));
        h.push(assistant("Writing the queries now."));
        h.push(user("ok"));
        let ids: Vec<String> = collect_tool_calls(&h)
            .into_iter()
            .filter(|c| !c.pinned)
            .map(|c| c.tool_use_id)
            .collect();
        let out = arbitrate(&h, &ids, &b).await.expect("outcome");
        for j in &out.judged {
            eprintln!("{} result={:.2}", j.0, j.1);
        }
    }
}
