//! Verbatim compaction: drop the tool calls and tool results Jev says are no
//! longer needed, keep everything else exactly as it was.
//!
//! Ported from tamaratran/fast-jev-compaction. The summarizing compaction and
//! the context handoff both replace the conversation with prose a model wrote,
//! and prose loses things — a file path, the exact error, a constraint the user
//! gave twenty turns ago. Here nothing is rewritten: user and assistant text
//! stay verbatim and in order; only old tool traffic goes, and Jev decides
//! which, while seeing the whole conversation.
//!
//! For every tool call outside the pinned first and newest messages Jev answers
//! two `noul`s: should the **call** stay (knowing it was made, with its input,
//! still matters), and should its **result** stay verbatim (the contents are
//! still needed and re-running the tool would not do). Then, against
//! [`KEEP_RESULT_THRESHOLD`] / [`KEEP_CALL_THRESHOLD`]: keep both · keep the call with the result cut to a short
//! head · drop the call together with its result.
//!
//! The state is the whole history oldest-first with tool results replaced by a
//! one-line note, fitted into [`MAX_STATE_CHARS`] in stages. It is resent with
//! every batch of [`CALLS_PER_REQUEST`] calls (two questions each), which keeps
//! each request inside what every backend accepts, the claudin.io plan
//! endpoint included. Anything going wrong yields `None` and the caller falls
//! back to the handoff or the summarizing compaction.
//!
//! Jev is one of three stages ([`shed`]), and the only one that needs a
//! credential:
//!
//! 0. **Lossless** ([`plan_lossless`]) — what a newer copy in the same history
//!    already supersedes: older task-list snapshots, and a file read again
//!    later with the same range. Nothing is judged because nothing is lost.
//! 1. **Judged** ([`plan`]) — the Jev pass above, on what stage 0 left.
//! 2. **By age** ([`plan_by_age`]) — no judge to ask: the oldest tool results
//!    are cut to their head until enough is freed. Outputs go, calls stay:
//!    knowing a call was made is cheap and stops the model from making it
//!    again. Only as a last resort — a subagent, which has no handoff and no
//!    summary behind it — are the oldest calls dropped outright, because the
//!    stubs of a long run add up too and the alternative is to overflow.
//!
//! Stage 2 is what a session without Jev gets instead of nothing.

use serde_json::{Value, json};
use std::collections::HashSet;

use crate::agent::provider::{ContentBlock, Message, ToolResultContent};

/// P(keep result) from which a tool output stays verbatim. Upstream uses 0.5;
/// our live probes (2026-10-05) put a schema the current task still depended
/// on at 0.41 and every no-longer-needed output at 0.07–0.24, so the line
/// sits between them — deleting a needed output costs a re-read, keeping a
/// stale one only costs context.
pub const KEEP_RESULT_THRESHOLD: f64 = 0.35;
/// P(keep call) from which a call stays (with its output cut to a head) even
/// when its output goes. Relevant-but-done steps scored 0.23–0.37, unrelated
/// reads 0.11–0.13.
pub const KEEP_CALL_THRESHOLD: f64 = 0.2;
/// Newest messages never touched; the first message is always kept too.
pub const PRESERVE_RECENT_MESSAGES: usize = 6;
/// Characters of a dropped result that stay, before the note.
pub const TRUNCATE_HEAD_CHARS: usize = 300;
/// Ceiling for the serialized state. Under the 60k the Jev client clips at
/// and the 64k the plan endpoint accepts, and well under Jev's 32k tokens.
pub const MAX_STATE_CHARS: usize = 56_000;
/// Two questions per call, eight per request — the plan endpoint's cap.
pub const CALLS_PER_REQUEST: usize = 4;
/// At most this many calls are judged per compaction (oldest first); the rest
/// stay. Bounds the bill at ~40 requests.
pub const MAX_JUDGED_CALLS: usize = 160;
/// A compaction that frees less than this share of the characters is not
/// worth breaking the cache for; the caller falls back.
pub const MIN_REDUCTION: f64 = 0.25;
const CONCURRENCY: usize = 8;
const INPUT_CHARS: [usize; 3] = [1000, 200, 60];
const TEXT_HEAD: usize = 400;
const TEXT_TAIL: usize = 150;

pub const STATE_CONTEXT: &str = "A coding assistant conversation is being compacted to free context. `history` is the whole conversation so far, oldest first; tool outputs are replaced by a short `result` note and long texts may be abridged. Each question asks whether one tool call, or the full output of that call, still needs to stay in the history verbatim. Whatever is not kept is deleted permanently, but the assistant can always re-run a tool or re-read a file.";

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

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Action {
    Keep,
    DropResult,
    DropCall,
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
    pub cost: f64,
    /// (tool_use_id, P(keep call), P(keep result)) for every judged call.
    pub judged: Vec<(String, f64, f64)>,
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
                .map(|(id, c, r)| json!([id, c, r]))
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
/// candidates.
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

/// The two questions for one call.
pub fn questions_for(call: &ToolCall) -> Value {
    json!({
        format!("call_{}", call.id): crate::agent::jev::noul(
            &format!(
                "Tool call {} ({}) should stay in the history: knowing this call was made, \
                 with its input, still matters for what the assistant does next",
                call.id, call.tool
            ),
            "Keep the call",
            "The call no longer matters",
        ),
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

pub fn decide_call(pinned: bool, keep_call: f64, keep_result: f64) -> Action {
    if pinned || keep_result >= KEEP_RESULT_THRESHOLD {
        Action::Keep
    } else if keep_call >= KEEP_CALL_THRESHOLD {
        Action::DropResult
    } else {
        Action::DropCall
    }
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

/// Remove dropped calls with their results, cut truncated results to a head
/// and a note, drop messages left empty, and merge neighbours left with the
/// same role so the roles still alternate.
pub fn apply(history: &[Message], decision: &Decision, head_chars: usize) -> Vec<Message> {
    let drop: HashSet<&str> = decision.drop_calls.iter().map(String::as_str).collect();
    let cut: HashSet<&str> = decision
        .truncate_results
        .iter()
        .map(String::as_str)
        .collect();
    let mut out: Vec<Message> = Vec::with_capacity(history.len());
    for m in history {
        let content: Vec<ContentBlock> = m
            .content
            .iter()
            .filter(|b| match b {
                ContentBlock::ToolUse { id, .. } => !drop.contains(id.as_str()),
                ContentBlock::ToolResult { tool_use_id, .. } => {
                    !drop.contains(tool_use_id.as_str())
                }
                _ => true,
            })
            .map(|b| match b {
                ContentBlock::ToolResult {
                    tool_use_id,
                    content,
                    ..
                } if cut.contains(tool_use_id.as_str()) => {
                    let text = content.as_text();
                    if text.len() <= head_chars + 120
                        && matches!(content, ToolResultContent::Text(_))
                    {
                        return b.clone();
                    }
                    let removed = text.len().saturating_sub(head_chars);
                    let error = if text.starts_with("Error") {
                        " (error)"
                    } else {
                        ""
                    };
                    ContentBlock::tool_result(
                        tool_use_id.clone(),
                        format!(
                            "{}\n[compacted: {removed} chars of this tool result removed{error}; \
                             re-run the tool if needed]",
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

/// Ask Jev about every unpinned call and return what to remove. `None` on any
/// failure — no backend answer, a state that does not fit, nothing to judge.
pub async fn plan(history: &[Message], backend: &crate::agent::jev::JevBackend) -> Option<Outcome> {
    let calls = collect_tool_calls(history);
    let candidates: Vec<&ToolCall> = calls
        .iter()
        .filter(|c| !c.pinned)
        .take(MAX_JUDGED_CALLS)
        .collect();
    if candidates.is_empty() {
        return None;
    }
    let (state, stage) = fit_state(history, &calls)?;
    let batches: Vec<Vec<&ToolCall>> = candidates
        .chunks(CALLS_PER_REQUEST)
        .map(<[&ToolCall]>::to_vec)
        .collect();
    let requests = batches.len();
    let question_sets: Vec<Value> = batches
        .iter()
        .map(|batch| {
            let mut q = serde_json::Map::new();
            for c in batch {
                if let Value::Object(m) = questions_for(c) {
                    q.extend(m);
                }
            }
            Value::Object(q)
        })
        .collect();
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
        let d = d?; // one failed batch fails the plan: no half-judged history
        cost += d.cost;
        answers.extend(d.answers);
    }
    let p = |key: String| answers.get(&key)?.get("noul")?.as_f64();
    let mut decision = Decision::default();
    let mut judged = Vec::with_capacity(candidates.len());
    for c in &candidates {
        let keep_call = p(format!("call_{}", c.id))?;
        let keep_result = p(format!("result_{}", c.id))?;
        judged.push((c.tool_use_id.clone(), keep_call, keep_result));
        match decide_call(false, keep_call, keep_result) {
            Action::Keep => {}
            Action::DropResult => decision.truncate_results.push(c.tool_use_id.clone()),
            Action::DropCall => decision.drop_calls.push(c.tool_use_id.clone()),
        }
    }
    let after = apply(history, &decision, TRUNCATE_HEAD_CHARS);
    Some(Outcome {
        decision,
        chars_before: history_chars(history),
        chars_after: history_chars(&after),
        requests,
        stage,
        source: SOURCE_JEV,
        cost,
        judged,
    })
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
    // Results that are not the whole of what the tool returned: cut by an
    // earlier prune, or a pointer to an earlier read (`repeated_read`).
    let mut compacted: HashSet<&str> = HashSet::new();
    for b in history.iter().flat_map(|m| m.content.iter()) {
        if let ContentBlock::ToolResult {
            tool_use_id,
            content,
            ..
        } = b
        {
            let text = content.as_text();
            if text.contains(COMPACTED_MARK) || text.starts_with(UNCHANGED_MARK) {
                compacted.insert(tool_use_id.as_str());
            }
        }
    }
    let mut decision = Decision::default();

    let latest_snapshot = calls
        .iter()
        .rposition(|c| TASK_SNAPSHOT_TOOLS.contains(&c.tool.as_str()) && !c.is_error);
    if let Some(latest) = latest_snapshot {
        for c in &calls[..latest] {
            if !c.pinned && TASK_TOOLS.contains(&c.tool.as_str()) {
                decision.drop_calls.push(c.tool_use_id.clone());
            }
        }
    }

    // Newest first, so each read is checked against the reads after it.
    let mut newer_reads = HashSet::new();
    for c in calls.iter().rev() {
        if c.tool != "read_file" {
            continue;
        }
        let Some(key) = read_key(&c.input) else {
            continue;
        };
        let full_copy = !c.is_error && !compacted.contains(c.tool_use_id.as_str());
        if newer_reads.contains(&key) {
            if !c.pinned && full_copy {
                decision.truncate_results.push(c.tool_use_id.clone());
            }
        } else if full_copy {
            // Only a result that is still whole can stand in for an older one.
            newer_reads.insert(key);
        }
    }
    decision.truncate_results.reverse();
    decision
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

/// What cutting one result to its head frees, or `None` when `apply` would
/// leave it as it is.
fn freed_by_cut(result_chars: usize) -> Option<usize> {
    (result_chars > TRUNCATE_HEAD_CHARS + 120).then(|| result_chars - TRUNCATE_HEAD_CHARS)
}

/// Roughly what `apply` leaves of a cut result: the head and its note.
const CUT_RESULT_CHARS: usize = TRUNCATE_HEAD_CHARS + 100;

/// Stage 2: cut the oldest tool results to their head until `free_chars` are
/// freed, leaving the first message and the recent window alone.
///
/// The blunt rule, for when there is no judge: age is a poor proxy for "no
/// longer needed", which is why Jev goes first when it can. But the cost of a
/// wrong cut is a re-read, and the alternative here is a handoff, a summary or
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
    let eligible: Vec<ToolCall> = collect_tool_calls(history)
        .into_iter()
        .filter(|c| {
            result_msg
                .get(c.tool_use_id.as_str())
                .is_some_and(|&ri| ri != 0 && ri < recent)
        })
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
        freed += c.input.to_string().len() + c.result_chars.min(CUT_RESULT_CHARS);
        decision.truncate_results.retain(|id| id != &c.tool_use_id);
        decision.drop_calls.push(c.tool_use_id.clone());
    }
    decision
}

/// How [`shed`] should go about it.
#[derive(Debug, Clone, Copy)]
pub struct ShedOptions {
    /// What stage 2 aims to free, counted from the history as given.
    pub free_chars: usize,
    /// Size of the recent window stage 2 leaves alone.
    pub recent_chars: usize,
    /// Nothing gentler comes after this shed: run stage 2 even when Jev judged
    /// the history and it was not enough, and let it drop the oldest calls
    /// outright when cutting results falls short.
    ///
    /// False for the main session, which hands off next — what Jev chose to
    /// keep is kept, and no call disappears. True for a subagent, which has
    /// nothing after this but running out of window.
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

/// Free context in three stages, stopping at the first one `accept` takes:
/// lossless, judged, by age. Each stage builds on the lossless one, and the
/// outcome is cumulative — one decision that, applied to `history`, gives the
/// returned history.
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

    let mut cost = 0.0;
    let mut requests = 0;
    if let Some(backend) = backend
        && let Some(judged) = plan(&after_lossless, backend).await
    {
        cost = judged.cost;
        requests = judged.requests;
        let decision = lossless.merged(&judged.decision);
        let after = apply(history, &decision, TRUNCATE_HEAD_CHARS);
        let outcome = Outcome {
            decision,
            chars_before,
            chars_after: history_chars(&after),
            ..judged
        };
        if accept(&outcome, &after) {
            return Shed {
                accepted: Some((outcome, after)),
                cost,
            };
        }
        if !opts.last_resort {
            return Shed {
                accepted: None,
                cost,
            };
        }
    }

    let already_freed = chars_before - history_chars(&after_lossless).min(chars_before);
    let by_age = plan_by_age(
        &after_lossless,
        opts.free_chars.saturating_sub(already_freed),
        opts.recent_chars,
        opts.last_resort,
    );
    if by_age.is_empty() {
        return Shed {
            accepted: None,
            cost,
        };
    }
    let decision = lossless.merged(&by_age);
    let after = apply(history, &decision, TRUNCATE_HEAD_CHARS);
    let outcome = Outcome {
        decision,
        chars_before,
        chars_after: history_chars(&after),
        requests,
        stage: SOURCE_RULES,
        source: SOURCE_RULES,
        cost,
        judged: Vec::new(),
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

    #[test]
    fn the_live_probes_map_to_conservative_decisions() {
        // (P(keep call), P(keep result)) from the two live probes, 2026-10-05.
        // The schema the current task depends on: kept whole.
        assert_eq!(decide_call(false, 0.50, 0.41), Action::Keep);
        // The file with the bug, already fixed: the call stays, its output is cut.
        assert_eq!(decide_call(false, 0.31, 0.23), Action::DropResult);
        // A build log nobody needs any more: the call stays, the log goes.
        assert_eq!(decide_call(false, 0.23, 0.14), Action::DropResult);
        // README / unrelated UI files: gone.
        assert_eq!(decide_call(false, 0.13, 0.11), Action::DropCall);
        assert_eq!(decide_call(false, 0.11, 0.07), Action::DropCall);
    }

    #[test]
    fn the_outcome_reports_its_own_diagnosis() {
        let o = Outcome {
            chars_before: 100,
            chars_after: 40,
            requests: 3,
            stage: "full",
            cost: 2e-5,
            judged: vec![("toolu_1".into(), 0.31, 0.23)],
            ..Default::default()
        };
        let v = o.stats_json();
        assert_eq!(v["requests"], 3);
        assert_eq!(v["stage"], "full");
        assert_eq!(v["reduction"], 0.6);
        assert_eq!(v["judged"][0], json!(["toolu_1", 0.31, 0.23]));
    }

    #[test]
    fn decisions_follow_the_two_probabilities() {
        assert_eq!(decide_call(false, 0.9, 0.9), Action::Keep);
        assert_eq!(decide_call(false, 0.9, 0.1), Action::DropResult);
        assert_eq!(decide_call(false, 0.1, 0.1), Action::DropCall);
        assert_eq!(decide_call(true, 0.0, 0.0), Action::Keep);
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
    fn apply_drops_a_call_with_its_result_and_keeps_roles_alternating() {
        let h = session(3);
        let d = Decision {
            drop_calls: vec!["toolu_1".into()],
            truncate_results: vec![],
        };
        let out = apply(&h, &d, TRUNCATE_HEAD_CHARS);
        let text = serde_json::to_string(&out).unwrap();
        assert!(!text.contains("toolu_1"));
        assert!(text.contains("toolu_0") && text.contains("toolu_2"));
        assert!(
            out.windows(2).all(|w| w[0].role != w[1].role),
            "roles must alternate"
        );
        assert!(out.iter().all(|m| !m.content.is_empty()));
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

    #[tokio::test]
    async fn plan_asks_jev_and_returns_what_to_drop() {
        // Every call gets keep_call 0.1 / keep_result 0.1 → dropped.
        let mut answers = serde_json::Map::new();
        for i in 1..=4 {
            answers.insert(format!("call_t{i}"), json!({"type": "noul", "noul": 0.1}));
            answers.insert(format!("result_t{i}"), json!({"type": "noul", "noul": 0.1}));
        }
        let body: &'static str = Box::leak(
            json!({"answers": answers, "usage": {"input_tokens": 100, "cost": 1e-6}})
                .to_string()
                .into_boxed_str(),
        );
        let (url, h) = spawn_stub(200, body);
        let hist = session(4);
        let out = plan(&hist, &stub_backend(&url)).await.expect("outcome");
        h.join().unwrap();
        assert_eq!(out.requests, 1);
        assert_eq!(out.decision.drop_calls.len(), 4);
        assert!(out.reduction() > MIN_REDUCTION, "{}", out.reduction());
        assert!(out.cost > 0.0);
    }

    #[tokio::test]
    async fn a_jev_failure_is_no_plan() {
        let (url, h) = spawn_stub(529, r#"{"error":"overloaded"}"#);
        assert!(plan(&session(4), &stub_backend(&url)).await.is_none());
        h.join().unwrap();
    }

    #[tokio::test]
    async fn nothing_to_judge_is_no_plan() {
        let h = vec![user("hi"), assistant("hello")];
        let b = stub_backend("http://127.0.0.1:9/unused");
        assert!(plan(&h, &b).await.is_none());
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
        assert!(!ids.contains(&"set_0".to_string()));
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
        let mut answers = serde_json::Map::new();
        for i in 1..=calls {
            answers.insert(format!("call_t{i}"), json!({"type": "noul", "noul": 0.9}));
            answers.insert(format!("result_t{i}"), json!({"type": "noul", "noul": 0.9}));
        }
        Box::leak(
            json!({"answers": answers, "usage": {"input_tokens": 100, "cost": 1e-6}})
                .to_string()
                .into_boxed_str(),
        )
    }

    #[tokio::test]
    async fn what_jev_chose_to_keep_is_kept_in_a_main_session() {
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
        // Jev said every result is still needed. Cutting them by age anyway
        // would override the one judge that read the conversation; the caller
        // hands off instead.
        assert!(shed.accepted.is_none());
        assert!(shed.cost > 0.0, "the question was still paid for");
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

    #[tokio::test]
    async fn the_judged_stage_builds_on_the_lossless_one() {
        // Two superseded task snapshots, then three reads Jev says to drop.
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
        // After stage 0 Jev sees set_c and the three reads — t1..t4, which is
        // one request. It keeps the task list and lets the reads go.
        let mut answers = serde_json::Map::new();
        for i in 1..=4 {
            let p = if i == 1 { 0.9 } else { 0.1 };
            answers.insert(format!("call_t{i}"), json!({"type": "noul", "noul": p}));
            answers.insert(format!("result_t{i}"), json!({"type": "noul", "noul": p}));
        }
        let body: &'static str = Box::leak(
            json!({"answers": answers, "usage": {"input_tokens": 100, "cost": 1e-6}})
                .to_string()
                .into_boxed_str(),
        );
        let (url, stub) = spawn_stub(200, body);
        // Stage 0 alone is not enough here, so the ladder goes on to Jev.
        let shed = shed(&h, Some(&stub_backend(&url)), opts(false), frees(0.9)).await;
        let request = stub.join().unwrap();
        let (outcome, after) = shed.accepted.expect("both stages together");
        assert_eq!(outcome.source, SOURCE_JEV);
        assert_eq!(
            outcome.decision.drop_calls,
            vec!["set_a", "set_b", "r0", "r1", "r2"]
        );
        // Jev was never asked about what stage 0 had already removed.
        assert!(request.contains("call_t4"), "{request}");
        assert!(
            !request.contains("call_t5"),
            "four calls were left to judge"
        );
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
    async fn live_plan_on_a_realistic_session() {
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
        let out = plan(&h, &b).await.expect("outcome");
        let calls = collect_tool_calls(&h);
        for c in &calls {
            let action = if out.decision.drop_calls.contains(&c.tool_use_id) {
                "DROP"
            } else if out.decision.truncate_results.contains(&c.tool_use_id) {
                "TRUNC"
            } else {
                "keep"
            };
            let p = out.judged.iter().find(|j| j.0 == c.tool_use_id);
            eprintln!(
                "{} {} {} -> {action} {:?}",
                c.id,
                c.tool,
                c.input,
                p.map(|j| (j.1, j.2))
            );
        }
        eprintln!(
            "reduction {:.0}% requests {} stage {} cost ${:.6}",
            out.reduction() * 100.0,
            out.requests,
            out.stage,
            out.cost
        );
    }

    /// Second live probe: an old result the current work still depends on.
    #[tokio::test]
    #[ignore]
    async fn live_plan_keeps_what_the_current_work_needs() {
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
        let out = plan(&h, &b).await.expect("outcome");
        for j in &out.judged {
            eprintln!("{} call={:.2} result={:.2}", j.0, j.1, j.2);
        }
    }
}
