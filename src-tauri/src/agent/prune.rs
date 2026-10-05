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

use serde_json::{json, Value};
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

#[derive(Debug, Clone, Default)]
pub struct Outcome {
    pub decision: Decision,
    pub chars_before: usize,
    pub chars_after: usize,
    pub requests: usize,
    pub stage: &'static str,
    pub cost: f64,
    /// (tool_use_id, P(keep call), P(keep result)) for every judged call.
    pub judged: Vec<(String, f64, f64)>,
}

impl Outcome {
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
            if let ContentBlock::ToolResult { tool_use_id, content, .. } = b {
                let text = content.as_text();
                results.insert(tool_use_id.clone(), (i, text.len(), text.starts_with("Error")));
            }
        }
    }
    let total = history.len();
    let mut calls = Vec::new();
    for (i, m) in history.iter().enumerate() {
        for b in &m.content {
            let ContentBlock::ToolUse { id, name, input, .. } = b else {
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
    let input = clip(&c.input.to_string().split_whitespace().collect::<Vec<_>>().join(" "), INPUT_CHARS[2]);
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
        let h: Vec<Value> = es.iter().filter(|e| !e.left_out).map(Entry::to_json).collect();
        Some((
            json!({"context": STATE_CONTEXT, "goal": goal, "history": h}).to_string(),
            stage,
        ))
    };

    let mut es = build(INPUT_CHARS[0]);
    if size(&es) <= MAX_STATE_CHARS {
        return render(&es, "full");
    }
    for (limit, stage) in [(INPUT_CHARS[1], "inputs<=200"), (INPUT_CHARS[2], "inputs<=60")] {
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
        es[k].calls = calls.iter().filter(|c| c.call_msg == i).map(call_line).collect();
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
    let cut: HashSet<&str> = decision.truncate_results.iter().map(String::as_str).collect();
    let mut out: Vec<Message> = Vec::with_capacity(history.len());
    for m in history {
        let content: Vec<ContentBlock> = m
            .content
            .iter()
            .filter(|b| match b {
                ContentBlock::ToolUse { id, .. } => !drop.contains(id.as_str()),
                ContentBlock::ToolResult { tool_use_id, .. } => !drop.contains(tool_use_id.as_str()),
                _ => true,
            })
            .map(|b| match b {
                ContentBlock::ToolResult { tool_use_id, content, .. }
                    if cut.contains(tool_use_id.as_str()) =>
                {
                    let text = content.as_text();
                    if text.len() <= head_chars + 120 && matches!(content, ToolResultContent::Text(_)) {
                        return b.clone();
                    }
                    let removed = text.len().saturating_sub(head_chars);
                    let error = if text.starts_with("Error") { " (error)" } else { "" };
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
pub async fn plan(
    history: &[Message],
    backend: &crate::agent::jev::JevBackend,
) -> Option<Outcome> {
    use futures::StreamExt;
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
    let answered: Vec<Option<crate::agent::jev::Decision>> =
        futures::stream::iter(batches.iter().map(|batch| {
            let mut q = serde_json::Map::new();
            for c in batch {
                if let Value::Object(m) = questions_for(c) {
                    q.extend(m);
                }
            }
            let state = &state;
            async move { crate::agent::jev::decide(backend, state, &Value::Object(q)).await }
        }))
        .buffer_unordered(CONCURRENCY)
        .collect()
        .await;
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
        cost,
        judged,
    })
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
        h.push(assistant_call("", "toolu_edit", "edit_file", json!({"path": "src/file_3.rs"})));
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
        let edit = calls.iter().find(|c| c.tool_use_id == "toolu_edit").unwrap();
        assert!(edit.pinned, "a call in the newest messages is never touched");
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
        assert!(!state.contains("fn body()"), "results must be notes, not contents");
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
        assert!(out.windows(2).all(|w| w[0].role != w[1].role), "roles must alternate");
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
                ContentBlock::ToolResult { tool_use_id, content, .. } if tool_use_id == "toolu_2" => {
                    Some(content.as_text().into_owned())
                }
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
            drop_calls: vec!["toolu_0".into(), "toolu_1".into(), "toolu_2".into(), "toolu_3".into()],
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
            h.push(assistant_call("", &format!("toolu_{i}"), "read_file", json!({"path": path})));
            h.push(result(&format!("toolu_{i}"), body));
        }
        h.push(assistant_call("", "toolu_t", "bash", json!({"command": "npm test -- login"})));
        h.push(result("toolu_t", "FAIL login.test.ts > refreshes token\n  expected valid, got expired\n"));
        h.push(assistant("The bug is in src/auth/token.ts: exp is seconds, Date.now() is ms."));
        h.push(user("Go ahead and fix it."));
        h.push(assistant_call("", "toolu_e", "edit_file", json!({"path": "src/auth/token.ts"})));
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
            eprintln!("{} {} {} -> {action} {:?}", c.id, c.tool, c.input, p.map(|j| (j.1, j.2)));
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
        let mut h = vec![user("Write the SQL migrations and the repository layer for invoices, following our schema exactly.")];
        h.push(assistant_call("", "toolu_s", "read_file", json!({"path": "db/schema.sql"})));
        h.push(result("toolu_s", schema));
        h.push(assistant_call("", "toolu_l", "list_dir", json!({"path": "src"})));
        h.push(result("toolu_l", "src/main.rs\nsrc/db/\nsrc/http/\n"));
        h.push(assistant_call("", "toolu_c", "bash", json!({"command": "cargo build"})));
        h.push(result("toolu_c", &"   Compiling dep v1.0\n".repeat(80)));
        h.push(assistant_call("", "toolu_r", "read_file", json!({"path": "src/http/routes.rs"})));
        h.push(result("toolu_r", &"fn route() {}\n".repeat(100)));
        h.push(assistant("Migration 001 written. Next: the InvoiceRepository with the exact column names from the schema."));
        h.push(user("Continue."));
        h.push(assistant_call("", "toolu_w", "edit_file", json!({"path": "src/db/invoices.rs"})));
        h.push(result("toolu_w", "ok"));
        h.push(assistant("Writing the queries now."));
        h.push(user("ok"));
        let out = plan(&h, &b).await.expect("outcome");
        for j in &out.judged {
            eprintln!("{} call={:.2} result={:.2}", j.0, j.1, j.2);
        }
    }
}
