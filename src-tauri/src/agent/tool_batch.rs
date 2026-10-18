//! Running a round's read-only tool calls side by side.
//!
//! A model that reads files "in parallel" sends several `read_file` /
//! `grep` / search calls in one turn, and the loop used to run them one after
//! another — each round paid for the sum of its reads instead of the slowest
//! one (plan C6, `docs/plans/2026-10-18_lean-harness-audit-and-plan.md`).
//!
//! The rule is deliberately narrow, so the transcript cannot tell the
//! difference:
//!
//! - Only a *contiguous* run of two or more calls that are all
//!   [`tools::PARALLEL_SAFE_TOOLS`], run without a prompt, and are watched by no
//!   tool hook is run ahead, at most [`MAX_CONCURRENT_TOOLS`] at a time. Any
//!   other call ends the run and is left to the loop's ordinary one-call path,
//!   so a read that follows an edit still sees the edit.
//! - Results land in the slot of the call that asked for them; the loop then
//!   walks the round in the model's order as before, appending each result
//!   and doing the order-dependent bookkeeping (loop watch, the repeated-read
//!   pointer, PostToolUse) in that order.
//! - A pause during a run starts nothing new and waits for the calls already
//!   under way (as it always waited for the one running call); the calls it
//!   kept from starting are answered the way a call skipped by a pause always
//!   was.

use crate::agent::events::AgentEvent;
use crate::agent::provider::{AgentConfig, ContentBlock};
use crate::agent::tools::{self, ToolContext};
use futures::FutureExt;
use futures::future::BoxFuture;
use serde_json::Value;
use std::collections::HashMap;
use std::sync::atomic::{AtomicBool, Ordering};
use tauri::ipc::Channel;
use tokio::task::JoinSet;

/// At most this many calls of one round run at the same time. Four is what the
/// plan settled on: enough to cover the usual fan-out of a model reading a
/// handful of files, few enough that a burst of searches does not take every
/// worker thread (the file tools read synchronously) from the provider stream
/// and the UI.
pub(crate) const MAX_CONCURRENT_TOOLS: usize = 4;

/// The `tool_result` of a call a pause kept from running (or from finishing).
pub(crate) const INTERRUPTED: &str = "Interrupted by the user — the tool was not run.";

/// Whether the call `name(input)` may be part of a parallel run.
///
/// `compact` is the Compact profile's direct-edit session, which refuses any
/// tool outside `surface::COMPACT_TOOLS`: such a call has to reach that
/// refusal, not run.
pub(crate) fn runs_in_parallel(
    name: &str,
    input: &Value,
    config: &AgentConfig,
    ctx: &ToolContext,
    compact: bool,
) -> bool {
    if !tools::is_parallel_safe(name) {
        return false;
    }
    if compact && !crate::agent::surface::COMPACT_TOOLS.contains(&name) {
        return false;
    }
    if ctx.hooks.as_ref().is_some_and(|h| h.watches_tool(name)) {
        return false;
    }
    // No hook watches the call, so no hook verdict can change this.
    crate::agent::tool_exec::effective_permission(
        crate::agent::permissions::tool_permission(name),
        name,
        input,
        config,
        ctx,
        &crate::agent::hooks::PreToolVerdict::None,
    ) == crate::agent::permissions::PermissionLevel::Auto
}

/// Run ahead the parallel run that starts at `start`, if there is one.
///
/// The run is the longest stretch of consecutive calls from `start` for which
/// `eligible` holds. Shorter than two, nothing happens and `0` is returned:
/// the loop runs the call itself, exactly as before this existed. Otherwise
/// every call in it is handed to `exec` in the model's order, the futures run
/// at most [`MAX_CONCURRENT_TOOLS`] at a time, and each finished result is put
/// in `ready` at its call's index. Returns the length of the run.
///
/// When `interrupt` is raised mid-run, the calls already started finish and
/// keep their results; the ones still waiting for a turn are never started and
/// their slots stay empty, for [`answer_unrun`] to answer.
pub(crate) async fn run_ahead<E, X>(
    tool_uses: &[Value],
    start: usize,
    ready: &mut [Option<ContentBlock>],
    eligible: E,
    exec: X,
    interrupt: &AtomicBool,
    event_tx: &Channel<AgentEvent>,
) -> usize
where
    E: Fn(&str, &Value) -> bool,
    X: Fn(&str, &str, Value) -> BoxFuture<'static, ContentBlock>,
{
    let end = (start..tool_uses.len())
        .find(|&i| !eligible(field(&tool_uses[i], "name"), &input_of(&tool_uses[i])))
        .unwrap_or(tool_uses.len());
    if end - start < 2 {
        return 0;
    }
    let run = &tool_uses[start..end];
    let jobs: Vec<_> = run
        .iter()
        .map(|t| exec(field(t, "name"), field(t, "id"), input_of(t)))
        .collect();
    for (offset, slot) in run_bounded(jobs, MAX_CONCURRENT_TOOLS, interrupt)
        .await
        .into_iter()
        .enumerate()
    {
        let tool_use = &run[offset];
        ready[start + offset] = match slot {
            Slot::Done(block) => Some(block),
            Slot::Unfinished => None,
            Slot::Panicked => {
                let (id, name) = (field(tool_use, "id"), field(tool_use, "name"));
                let error = "the tool stopped unexpectedly".to_string();
                let _ = event_tx.send(AgentEvent::ToolResult {
                    tool_id: id.to_string(),
                    tool_name: name.to_string(),
                    output: String::new(),
                    error: Some(error.clone()),
                });
                Some(ContentBlock::tool_result(id, format!("Error: {error}")))
            }
        };
    }
    end - start
}

/// The executor the loop hands [`run_ahead`]: announces the call (in the
/// model's order, since `run_ahead` calls this in that order before any job
/// starts) and returns a job that runs it as the loop's auto-approved path
/// does. The job owns clones of what it needs, so it can run on a task of its
/// own.
pub(crate) fn executor<'a>(
    event_tx: &'a Channel<AgentEvent>,
    session_id: &'a str,
    ctx: &'a ToolContext,
) -> impl Fn(&str, &str, Value) -> BoxFuture<'static, ContentBlock> + 'a {
    move |name, id, input| {
        let _ = event_tx.send(AgentEvent::ToolCall {
            session_id: session_id.to_string(),
            tool_id: id.to_string(),
            tool_name: name.to_string(),
            args: input.clone(),
            permission: "auto".into(),
            edit_proposal: None,
        });
        let (name, id) = (name.to_string(), id.to_string());
        let (event_tx, session_id, ctx) = (event_tx.clone(), session_id.to_string(), ctx.clone());
        async move {
            crate::agent::tool_exec::execute_reported(
                &name,
                &id,
                input,
                "auto",
                &event_tx,
                &session_id,
                &ctx,
            )
            .await
        }
        .boxed()
    }
}

/// How one job of a run ended.
enum Slot<T> {
    Done(T),
    /// Never started: the pause came while it waited for a turn.
    Unfinished,
    Panicked,
}

/// Run `jobs` on tasks of their own, at most `limit` at a time, and return how
/// each ended, in the order given.
///
/// Once `interrupt` is raised no further job is started, and every job that
/// had already started is waited for. That is the pause as it always was — the
/// call that was running finished, the ones after it were not run — and it is
/// the only honest choice: a tool body does its work synchronously and reports
/// its own `ToolResult` (`execute_reported`), so an abort cannot stop it, only
/// stop us from hearing it finish. Answering such a call as "not run" gave it a
/// second, contradicting result while its side effects (the read tracker)
/// stood.
async fn run_bounded<T: Send + 'static>(
    jobs: Vec<BoxFuture<'static, T>>,
    limit: usize,
    interrupt: &AtomicBool,
) -> Vec<Slot<T>> {
    let mut slots: Vec<Slot<T>> = (0..jobs.len()).map(|_| Slot::Unfinished).collect();
    let mut waiting = jobs.into_iter().enumerate();
    let mut set = JoinSet::new();
    let mut ids = HashMap::new();
    loop {
        while set.len() < limit && !interrupt.load(Ordering::SeqCst) {
            let Some((index, job)) = waiting.next() else {
                break;
            };
            ids.insert(set.spawn(job).id(), index);
        }
        match set.join_next_with_id().await {
            Some(Ok((id, value))) => slots[ids[&id]] = Slot::Done(value),
            Some(Err(e)) => {
                if e.is_panic() {
                    slots[ids[&e.id()]] = Slot::Panicked;
                }
            }
            None => break,
        }
    }
    slots
}

/// Answer every call in `tool_uses` after a pause: the ones a parallel run
/// already finished keep their result (their `ToolResult` event went out when
/// they finished), the rest get `message` and a `ToolResult` event now.
pub(crate) fn answer_unrun(
    tool_uses: &[Value],
    ready: &mut [Option<ContentBlock>],
    message: &str,
    event_tx: &Channel<AgentEvent>,
    calls: &mut Vec<ContentBlock>,
    results: &mut Vec<ContentBlock>,
) {
    for (tool_use, slot) in tool_uses.iter().zip(ready.iter_mut()) {
        let (id, name) = (field(tool_use, "id"), field(tool_use, "name"));
        calls.push(ContentBlock::tool_use(id, name, input_of(tool_use)));
        if let Some(block) = slot.take() {
            results.push(block);
            continue;
        }
        let _ = event_tx.send(AgentEvent::ToolResult {
            tool_id: id.to_string(),
            tool_name: name.to_string(),
            output: message.into(),
            error: None,
        });
        results.push(ContentBlock::tool_result(id, message));
    }
}

fn field<'a>(tool_use: &'a Value, key: &str) -> &'a str {
    tool_use.get(key).and_then(|v| v.as_str()).unwrap_or("")
}

fn input_of(tool_use: &Value) -> Value {
    tool_use.get("input").cloned().unwrap_or(Value::Null)
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::sync::atomic::AtomicUsize;
    use std::sync::{Arc, Mutex};
    use std::time::Duration;
    use tauri::ipc::InvokeResponseBody;

    fn call(i: usize, name: &str, input: Value) -> Value {
        serde_json::json!({ "id": format!("tu_{i}"), "name": name, "input": input })
    }

    fn reads(n: usize) -> Vec<Value> {
        (0..n)
            .map(|i| {
                call(
                    i,
                    "read_file",
                    serde_json::json!({ "path": format!("/f{i}") }),
                )
            })
            .collect()
    }

    fn text(block: &ContentBlock) -> (String, String) {
        match block {
            ContentBlock::ToolResult {
                tool_use_id,
                content,
                ..
            } => (tool_use_id.clone(), content.as_text().into_owned()),
            other => panic!("not a tool_result: {other:?}"),
        }
    }

    /// Every event the run sent, decoded.
    fn recording_channel() -> (Channel<AgentEvent>, Arc<Mutex<Vec<AgentEvent>>>) {
        let seen = Arc::new(Mutex::new(Vec::new()));
        let kept = seen.clone();
        let channel = Channel::new(move |body: InvokeResponseBody| -> tauri::Result<()> {
            if let InvokeResponseBody::Json(json) = &body
                && let Ok(event) = serde_json::from_str::<AgentEvent>(json)
            {
                kept.lock().unwrap().push(event);
            }
            Ok(())
        });
        (channel, seen)
    }

    fn tool_result_ids(events: &Mutex<Vec<AgentEvent>>) -> Vec<String> {
        events
            .lock()
            .unwrap()
            .iter()
            .filter_map(|e| match e {
                AgentEvent::ToolResult { tool_id, .. } => Some(tool_id.clone()),
                _ => None,
            })
            .collect()
    }

    /// A fake executor: each call sleeps for the milliseconds `delay` gives
    /// its index, then answers `ran <id>`. Records the order calls finish in
    /// and the most that were ever running at once.
    struct Fake {
        finished: Arc<Mutex<Vec<String>>>,
        running: Arc<AtomicUsize>,
        peak: Arc<AtomicUsize>,
        started: Arc<Mutex<Vec<String>>>,
    }

    impl Fake {
        fn new() -> Self {
            Self {
                finished: Arc::default(),
                running: Arc::default(),
                peak: Arc::default(),
                started: Arc::default(),
            }
        }

        fn exec(
            &self,
            delay: impl Fn(usize) -> u64,
        ) -> impl Fn(&str, &str, Value) -> BoxFuture<'static, ContentBlock> {
            let (finished, running, peak, started) = (
                self.finished.clone(),
                self.running.clone(),
                self.peak.clone(),
                self.started.clone(),
            );
            move |_name, id, _input| {
                let index: usize = id.trim_start_matches("tu_").parse().unwrap();
                let ms = delay(index);
                let id = id.to_string();
                let (finished, running, peak) = (finished.clone(), running.clone(), peak.clone());
                started.lock().unwrap().push(id.clone());
                async move {
                    let now = running.fetch_add(1, Ordering::SeqCst) + 1;
                    peak.fetch_max(now, Ordering::SeqCst);
                    tokio::time::sleep(Duration::from_millis(ms)).await;
                    running.fetch_sub(1, Ordering::SeqCst);
                    finished.lock().unwrap().push(id.clone());
                    ContentBlock::tool_result(&id, format!("ran {id}"))
                }
                .boxed()
            }
        }
    }

    fn any_call(_: &str, _: &Value) -> bool {
        true
    }

    #[tokio::test(start_paused = true)]
    async fn results_keep_the_models_order_when_later_calls_finish_first() {
        let calls = reads(4);
        let mut ready = vec![None; 4];
        let fake = Fake::new();
        let (events, _) = recording_channel();
        let ran = run_ahead(
            &calls,
            0,
            &mut ready,
            any_call,
            fake.exec(|i| 400 - 100 * i as u64),
            &AtomicBool::new(false),
            &events,
        )
        .await;

        assert_eq!(ran, 4);
        assert_eq!(
            *fake.finished.lock().unwrap(),
            ["tu_3", "tu_2", "tu_1", "tu_0"],
            "the last call finished first"
        );
        let got: Vec<_> = ready.iter().map(|b| text(b.as_ref().unwrap())).collect();
        for (i, (id, body)) in got.iter().enumerate() {
            assert_eq!(id, &format!("tu_{i}"));
            assert_eq!(body, &format!("ran tu_{i}"));
        }
    }

    #[tokio::test(start_paused = true)]
    async fn four_slow_reads_take_as_long_as_one() {
        let calls = reads(4);
        let mut ready = vec![None; 4];
        let fake = Fake::new();
        let (events, _) = recording_channel();
        let started = tokio::time::Instant::now();
        run_ahead(
            &calls,
            0,
            &mut ready,
            any_call,
            fake.exec(|_| 100),
            &AtomicBool::new(false),
            &events,
        )
        .await;
        let took = started.elapsed();
        assert!(took < Duration::from_millis(200), "took {took:?}");
        assert_eq!(fake.peak.load(Ordering::SeqCst), 4);
        assert!(ready.iter().all(Option::is_some));
    }

    #[tokio::test(start_paused = true)]
    async fn no_more_than_four_calls_run_at_once() {
        let calls = reads(6);
        let mut ready = vec![None; 6];
        let fake = Fake::new();
        let (events, _) = recording_channel();
        let started = tokio::time::Instant::now();
        let ran = run_ahead(
            &calls,
            0,
            &mut ready,
            any_call,
            fake.exec(|_| 100),
            &AtomicBool::new(false),
            &events,
        )
        .await;
        assert_eq!(ran, 6);
        assert_eq!(fake.peak.load(Ordering::SeqCst), MAX_CONCURRENT_TOOLS);
        let took = started.elapsed();
        assert!(
            took >= Duration::from_millis(200) && took < Duration::from_millis(300),
            "two waves of 100 ms, took {took:?}"
        );
        assert!(ready.iter().all(Option::is_some));
    }

    // A lone read gains nothing from a task of its own; the loop's one-call
    // path runs it, as it did before runs existed.
    #[tokio::test(start_paused = true)]
    async fn a_lone_read_is_left_to_the_loop() {
        let calls = vec![
            call(0, "read_file", serde_json::json!({ "path": "/a" })),
            call(1, "bash", serde_json::json!({ "command": "ls" })),
        ];
        let mut ready = vec![None; 2];
        let fake = Fake::new();
        let (events, _) = recording_channel();
        let ran = run_ahead(
            &calls,
            0,
            &mut ready,
            |name, _| name == "read_file",
            fake.exec(|_| 100),
            &AtomicBool::new(false),
            &events,
        )
        .await;
        assert_eq!(ran, 0);
        assert!(fake.started.lock().unwrap().is_empty());
        assert!(ready.iter().all(Option::is_none));
    }

    #[tokio::test(start_paused = true)]
    async fn a_call_that_needs_approval_splits_the_run() {
        let config = AgentConfig::default();
        let ctx = crate::agent::tools::tests_support::ctx();
        let eligible =
            |name: &str, input: &Value| runs_in_parallel(name, input, &config, &ctx, false);
        let calls = vec![
            call(0, "read_file", serde_json::json!({ "path": "/a" })),
            call(1, "grep", serde_json::json!({ "pattern": "x" })),
            call(2, "bash", serde_json::json!({ "command": "cargo build" })),
            call(3, "list_dir", serde_json::json!({ "path": "/" })),
            call(4, "code_search", serde_json::json!({ "query": "y" })),
        ];
        let mut ready = vec![None; 5];
        let fake = Fake::new();
        let (events, _) = recording_channel();
        let interrupt = AtomicBool::new(false);

        let first = run_ahead(
            &calls,
            0,
            &mut ready,
            eligible,
            fake.exec(|_| 10),
            &interrupt,
            &events,
        )
        .await;
        assert_eq!(first, 2);
        assert_eq!(*fake.started.lock().unwrap(), ["tu_0", "tu_1"]);
        assert!(ready[0].is_some() && ready[1].is_some());
        assert!(
            ready[2..].iter().all(Option::is_none),
            "the bash call and what follows it wait"
        );

        // The loop runs the bash call itself, then reaches the second run.
        let second = run_ahead(
            &calls,
            3,
            &mut ready,
            eligible,
            fake.exec(|_| 10),
            &interrupt,
            &events,
        )
        .await;
        assert_eq!(second, 2);
        assert_eq!(
            *fake.started.lock().unwrap(),
            ["tu_0", "tu_1", "tu_3", "tu_4"]
        );
        assert!(ready[2].is_none());
    }

    // A pause starts nothing new and waits for what is already under way, as
    // it always waited for the one running call; every call is then answered
    // once, in the model's order.
    #[tokio::test(start_paused = true)]
    async fn a_pause_during_a_parallel_run_answers_every_call() {
        let calls = reads(6);
        let mut ready = vec![None; 6];
        let fake = Fake::new();
        let (events, seen) = recording_channel();
        let interrupt = Arc::new(AtomicBool::new(false));
        let stop = interrupt.clone();
        tokio::spawn(async move {
            tokio::time::sleep(Duration::from_millis(50)).await;
            stop.store(true, Ordering::SeqCst);
        });
        let started = tokio::time::Instant::now();
        run_ahead(
            &calls,
            0,
            &mut ready,
            any_call,
            fake.exec(|_| 100),
            &interrupt,
            &events,
        )
        .await;
        assert!(
            started.elapsed() < Duration::from_millis(200),
            "the calls kept from starting did not run after the pause: {:?}",
            started.elapsed()
        );
        let mut ran = fake.finished.lock().unwrap().clone();
        ran.sort();
        assert_eq!(ran, ["tu_0", "tu_1", "tu_2", "tu_3"], "the first four ran");
        assert!(ready[..4].iter().all(Option::is_some));
        assert!(ready[4..].iter().all(Option::is_none));

        let (mut uses, mut results) = (Vec::new(), Vec::new());
        answer_unrun(
            &calls,
            &mut ready,
            INTERRUPTED,
            &events,
            &mut uses,
            &mut results,
        );
        let use_ids: Vec<_> = uses
            .iter()
            .map(|b| match b {
                ContentBlock::ToolUse { id, .. } => id.clone(),
                other => panic!("not a tool_use: {other:?}"),
            })
            .collect();
        assert_eq!(use_ids, ["tu_0", "tu_1", "tu_2", "tu_3", "tu_4", "tu_5"]);
        let answered: Vec<_> = results.iter().map(text).collect();
        let expected: Vec<_> = (0..6)
            .map(|i| {
                let body = if i < 4 {
                    format!("ran tu_{i}")
                } else {
                    INTERRUPTED.to_string()
                };
                (format!("tu_{i}"), body)
            })
            .collect();
        assert_eq!(answered, expected);
        // The calls that ran reported themselves when they finished (the fake
        // sends nothing); the two kept from starting are reported once each,
        // here.
        assert_eq!(tool_result_ids(&seen), ["tu_4", "tu_5"]);
    }

    // The real tools do their work synchronously and report their own result
    // (`execute_reported` sends the `ToolResult` before returning), and an
    // abort cannot stop a body that never yields. A call that had started when
    // the pause came therefore finishes and reports anyway: the run has to wait
    // for it and keep that result, never answer it a second time as not run.
    #[tokio::test(flavor = "multi_thread", worker_threads = 4)]
    async fn a_pause_never_answers_a_call_twice_when_its_body_cannot_be_stopped() {
        let calls = reads(2);
        let mut ready = vec![None; 2];
        let (events, seen) = recording_channel();
        let reporter = events.clone();
        let exec = move |_: &str, id: &str, _: Value| -> BoxFuture<'static, ContentBlock> {
            let (id, events) = (id.to_string(), reporter.clone());
            async move {
                std::thread::sleep(Duration::from_millis(200));
                let _ = events.send(AgentEvent::ToolResult {
                    tool_id: id.clone(),
                    tool_name: "read_file".into(),
                    output: format!("ran {id}"),
                    error: None,
                });
                ContentBlock::tool_result(&id, format!("ran {id}"))
            }
            .boxed()
        };
        let interrupt = Arc::new(AtomicBool::new(false));
        let stop = interrupt.clone();
        std::thread::spawn(move || {
            std::thread::sleep(Duration::from_millis(30));
            stop.store(true, Ordering::SeqCst);
        });
        run_ahead(&calls, 0, &mut ready, any_call, exec, &interrupt, &events).await;
        let (mut uses, mut results) = (Vec::new(), Vec::new());
        answer_unrun(
            &calls,
            &mut ready,
            INTERRUPTED,
            &events,
            &mut uses,
            &mut results,
        );
        // Long enough for any body still running to have reported.
        tokio::time::sleep(Duration::from_millis(400)).await;

        let mut reported = tool_result_ids(&seen);
        reported.sort();
        assert_eq!(reported, ["tu_0", "tu_1"], "one result each");
        let answered: Vec<_> = results.iter().map(text).collect();
        assert_eq!(
            answered,
            [
                ("tu_0".to_string(), "ran tu_0".to_string()),
                ("tu_1".to_string(), "ran tu_1".to_string()),
            ],
            "the model hears what the timeline shows"
        );
    }

    #[tokio::test(start_paused = true)]
    async fn a_call_that_panics_is_answered_with_an_error() {
        let calls = reads(2);
        let mut ready = vec![None; 2];
        let (events, seen) = recording_channel();
        let exec = |_: &str, id: &str, _: Value| -> BoxFuture<'static, ContentBlock> {
            let id = id.to_string();
            async move {
                if id == "tu_1" {
                    panic!("tool blew up");
                }
                ContentBlock::tool_result(&id, "fine")
            }
            .boxed()
        };
        run_ahead(
            &calls,
            0,
            &mut ready,
            any_call,
            exec,
            &AtomicBool::new(false),
            &events,
        )
        .await;
        let (id, body) = text(ready[1].as_ref().expect("answered"));
        assert_eq!(id, "tu_1");
        assert!(body.starts_with("Error"), "{body}");
        assert_eq!(text(ready[0].as_ref().unwrap()).1, "fine");
        assert_eq!(tool_result_ids(&seen), ["tu_1"]);
    }

    #[test]
    fn only_read_only_calls_that_run_unasked_join_a_run() {
        let ctx = crate::agent::tools::tests_support::ctx();
        let config = AgentConfig::default();
        let yolo = AgentConfig {
            yolo_mode: true,
            ..AgentConfig::default()
        };
        let none = serde_json::json!({});
        for name in [
            "read_file",
            "grep",
            "semantic_search",
            "web_search",
            "tasks_get",
        ] {
            assert!(
                runs_in_parallel(name, &none, &config, &ctx, false),
                "{name}"
            );
        }
        // YOLO lifts the prompt; it does not make a writer or a command a reader.
        for name in [
            "bash",
            "edit_file",
            "browser",
            "mcp__github__search",
            "tasks_set",
        ] {
            assert!(!runs_in_parallel(name, &none, &yolo, &ctx, false), "{name}");
        }
        // The Compact session refuses what it was not offered; that refusal is
        // the loop's to give.
        assert!(runs_in_parallel("read_file", &none, &config, &ctx, true));
        assert!(!runs_in_parallel("code_search", &none, &config, &ctx, true));
    }

    // A PreToolUse hook may deny or ask, and its answer has to come before the
    // call runs; a call a hook watches therefore runs on its own.
    #[cfg(unix)]
    #[test]
    fn a_call_a_hook_watches_runs_on_its_own() {
        let ws = std::env::temp_dir().join(format!(
            "claudinio-batch-hook-{}-{}",
            std::process::id(),
            crate::agent::persist::now_ms()
        ));
        std::fs::create_dir_all(&ws).unwrap();
        std::fs::write(
            ws.join(".claudinio.json"),
            r#"{"hooks":{"PostToolUse":[{"matcher":"Read","hooks":[{"type":"command","command":"true"}]}]}}"#,
        )
        .unwrap();
        let config = AgentConfig {
            hooks_enabled: true,
            ..AgentConfig::default()
        };
        let set = Arc::new(crate::agent::hooks::discovery::resolve_with_home(
            Some(&ws),
            &config,
            Some(&ws.join("_home")),
        ));
        let trust = Arc::new(crate::agent::hooks::TrustStore::with_path(
            ws.join("trust.json"),
        ));
        let ctx = ToolContext {
            hooks: Some(Arc::new(crate::agent::hooks::HookCtx::new(
                set,
                trust,
                "s",
                ws.join("s.jsonl"),
                ws.clone(),
            ))),
            ..crate::agent::tools::tests_support::ctx()
        };
        let none = serde_json::json!({});
        assert!(!runs_in_parallel("read_file", &none, &config, &ctx, false));
        assert!(runs_in_parallel("grep", &none, &config, &ctx, false));
        std::fs::remove_dir_all(&ws).ok();
    }
}
