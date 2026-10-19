//! Tasks as recitation, not RPC (lean-harness plan, Phase 4 / C4).
//!
//! The Standard prompt used to make `tasks_get` the first call of every run,
//! a `tasks_update` to `doing` before anything was touched, one to `done`
//! after, and a final update before the last message — "never batch". That
//! is three to five extra rounds per task, each a full re-read of the
//! context, to move state the harness already holds. Manus's "recitation"
//! argument is a reason to keep the list in front of the model, near the end
//! of the context where attention is highest; it is not a reason to make the
//! model fetch it.
//!
//! So the harness recites it. A run's user turn ends with a `<tasks>` block
//! (one line per task: `id · status · title`), added where hook context is
//! added and persisted inside the same `Turn`, so a reload replays it and the
//! history stays append-only. A context handoff's successor gets it the same
//! way, in its first message, after the handoff document; a summarizing
//! compaction restates it after the summary ([`recite_after_compaction`]).

use crate::agent::persist::TaskItem;
use crate::agent::tools::ToolContext;

/// The tag the block opens with. The frontend never renders a user turn's
/// blocks as the user's words (the bubble is the `user` record), and the
/// prompt tells the model what the block is.
pub const TASKS_OPEN: &str = "<tasks>";
pub const TASKS_CLOSE: &str = "</tasks>";

/// A title longer than this is cut with an ellipsis: the block names the
/// task, the description is in the task itself.
const TITLE_CHARS: usize = 160;

/// The block for `tasks`, or `None` when there are none (a session without a
/// list is sent nothing — single-step work needs no list).
pub fn tasks_block(tasks: &[TaskItem]) -> Option<String> {
    if tasks.is_empty() {
        return None;
    }
    let mut out = format!("{TASKS_OPEN}\n");
    for t in tasks {
        let title = t.title.split_whitespace().collect::<Vec<_>>().join(" ");
        let title = match title.char_indices().nth(TITLE_CHARS) {
            Some((cut, _)) => format!("{}\u{2026}", &title[..cut]),
            None => title,
        };
        out.push_str(&format!("{} \u{b7} {} \u{b7} {title}", t.id, t.status));
        if crate::agent::tools::tasks::is_golden(t) {
            out.push_str(" \u{b7} golden");
        }
        out.push('\n');
    }
    out.push_str(TASKS_CLOSE);
    Some(out)
}

/// The task list of the session `ctx` writes to, as it stands now (empty
/// when there is none, or no session file). The loop reads it once at the
/// start of a run: that snapshot is both the run's `<tasks>` block and what
/// the finish line compares against ([`still_doing_nudge`]).
pub fn session_tasks(ctx: &ToolContext) -> Vec<TaskItem> {
    ctx.session_store_path
        .as_deref()
        .and_then(|p| crate::agent::persist::load_last_tasks(std::path::Path::new(p)).ok())
        .unwrap_or_default()
}

/// The nudge for a turn that ended with tasks still `doing` that this run
/// put there or worked on, or `None` when there are none.
///
/// `at_start` is the list as the run found it. A task that is `doing` now
/// exactly as it was then is one an earlier run left behind — the user
/// pressed Stop mid-task and then asked something unrelated — and telling
/// the model to go and finish it would start work nobody asked for in this
/// run (in Brain, work it cannot even do). The run's `<tasks>` block already
/// shows it as `doing`, so it is not lost. Only a task this run moved to
/// `doing`, or wrote to while it was `doing`, is named.
///
/// That is read off the status and the journal's length against the start
/// snapshot, by id — not the whole task: a reworded title or description is
/// not work on the task (Phase 4 final review). What a snapshot cannot see
/// stays unseen: a task this run moved `doing` → `todo` → `doing` and wrote
/// nothing to looks exactly as it started, and is not named.
pub fn still_doing_nudge(ctx: &ToolContext, at_start: &[TaskItem]) -> Option<String> {
    let worked_on = |t: &TaskItem| {
        !at_start
            .iter()
            .any(|s| s.id == t.id && s.status == t.status && s.journal.len() == t.journal.len())
    };
    let doing: Vec<TaskItem> = session_tasks(ctx)
        .into_iter()
        .filter(|t| t.status == "doing" && worked_on(t))
        .collect();
    (!doing.is_empty()).then(|| crate::agent::nudges::still_doing(&doing))
}

/// After a summarizing compaction, restate `tasks` as the last block of the
/// rebuilt context — after the summary and the kept tail, where attention is
/// highest (Manus's recitation). `persist::history_from_records` calls it at
/// the compaction point with the list as it stood then, so the loop's rebuild
/// right after compacting and any later reload produce the same history.
/// A session with a living document gets its `<document>` outline after the
/// list, as the compaction recorded it (`Compacted::document`, read from the
/// file when the compaction ran — the freshest outline there is), in the
/// order a run's turn carries them (lean-harness Phase 4f).
///
/// Only a context that ends on a user message gets it: a compaction inside
/// a run (the tail ends on the round's tool results, or on the run's own
/// user turn), or one between runs that kept no tail at all (the tail shrinks
/// to zero when it alone is over budget), where the history is the summary
/// alone. A context that ends on the model's answer was compacted between
/// runs, and the next run's user turn carries the list anyway; adding a user
/// message after the answer would put words in the conversation no loop
/// wrote. A tail that already ends with this exact list (and outline) — the
/// run's user turn, when a run compacts before its first request — is left
/// as it is.
/// In the tailless between-runs case the next run's user turn merges into
/// the summary's message, and [`take_trailing_recitals`] keeps the list from
/// being said twice.
pub fn recite_after_compaction(
    history: &mut [crate::agent::provider::Message],
    tasks: &[TaskItem],
    document: Option<&str>,
) {
    let blocks: Vec<String> = tasks_block(tasks)
        .into_iter()
        .chain(document.map(str::to_string))
        .collect();
    if blocks.is_empty() {
        return;
    }
    let Some(last) = history.last_mut().filter(|m| m.role == "user") else {
        return;
    };
    // What the message already ends with — all of the recital, or its first
    // blocks (a run turn sent before the document existed) — is not said
    // again; the rest follows it. A run turn may end with a `<doc-changed>`
    // note after its recital (the person edited the document): the match
    // looks past it (Phase 6f).
    let mut texts: Vec<Option<&str>> = last
        .content
        .iter()
        .map(|b| match b {
            crate::agent::provider::ContentBlock::Text { text, .. } => Some(text.as_str()),
            _ => None,
        })
        .collect();
    let doc_changed = crate::agent::doc_session::DOC_CHANGED_OPEN;
    while texts
        .last()
        .is_some_and(|t| t.is_some_and(|t| t.starts_with(doc_changed)))
    {
        texts.pop();
    }
    let said = (1..=blocks.len())
        .rev()
        .find(|&k| {
            texts.len() >= k
                && texts[texts.len() - k..]
                    .iter()
                    .zip(&blocks[..k])
                    .all(|(t, b)| *t == Some(b.as_str()))
        })
        .unwrap_or(0);
    for block in &blocks[said..] {
        last.content
            .push(crate::agent::provider::ContentBlock::text(block.clone()));
    }
}

/// Before a run appends `block` to its user turn: when the history already
/// ends with a user message whose last block is this same list — a tailless
/// compaction between runs left `[summary, list]` — remove it, because the
/// run's turn merges into that message (`session::push_user_blocks`) and
/// appends the list again. The model then reads it once, last, after the
/// new message. Only the in-memory copy changes: the recital is rebuilt
/// from the compaction record on every replay.
///
/// The same for every block the run appends that a compaction restates —
/// the list, then the document's outline: the trailing ones of a kind the
/// run appends are removed, last first, so `[summary, list, outline]`
/// followed by the run's turn reads each once. By kind, not by equal text:
/// the person can edit the document between the compaction and the run, and
/// the run's fresh outline then differs from the recorded one (Phase 6f).
pub fn take_trailing_recitals(history: &mut [crate::agent::provider::Message], blocks: &[String]) {
    let Some(last) = history.last_mut().filter(|m| m.role == "user") else {
        return;
    };
    let kinds: Vec<&str> = [TASKS_OPEN, crate::agent::doc_session::DOCUMENT_OPEN]
        .into_iter()
        .filter(|open| blocks.iter().any(|b| b.starts_with(open)))
        .collect();
    while last.content.len() > 1
        && matches!(
            last.content.last(),
            Some(crate::agent::provider::ContentBlock::Text { text, .. })
                if kinds.iter().any(|open| text.starts_with(open))
        )
    {
        last.content.pop();
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn task(id: &str, status: &str, title: &str) -> TaskItem {
        TaskItem {
            id: id.into(),
            title: title.into(),
            description: "a description the block never carries".into(),
            journal: vec!["a journal entry the block never carries".into()],
            status: status.into(),
        }
    }

    #[test]
    fn the_block_is_one_line_per_task_with_golden_ones_marked() {
        let block = tasks_block(&[
            task("golden-ship-it-1", "todo", "ship it"),
            task("task-1", "doing", "wire the UI"),
            task("task-2", "done", "add the test"),
        ])
        .expect("a list gives a block");
        assert_eq!(
            block,
            "<tasks>\n\
             golden-ship-it-1 \u{b7} todo \u{b7} ship it \u{b7} golden\n\
             task-1 \u{b7} doing \u{b7} wire the UI\n\
             task-2 \u{b7} done \u{b7} add the test\n\
             </tasks>"
        );
        assert!(!block.contains("description") && !block.contains("journal"));
    }

    #[test]
    fn no_tasks_no_block() {
        assert_eq!(tasks_block(&[]), None);
    }

    // A title is a line of the block; one that spans lines or runs on would
    // make the next task look like part of it.
    #[test]
    fn a_title_stays_on_one_bounded_line() {
        let long = "x".repeat(400);
        let block = tasks_block(&[
            task("a", "todo", "two\nlines  and\tspaces"),
            task("b", "todo", &long),
        ])
        .unwrap();
        let lines: Vec<&str> = block.lines().collect();
        assert_eq!(lines.len(), 4, "{block}");
        assert_eq!(lines[1], "a \u{b7} todo \u{b7} two lines and spaces");
        let title = lines[2].strip_prefix("b \u{b7} todo \u{b7} ").unwrap();
        assert_eq!(title.chars().count(), TITLE_CHARS + 1);
        assert!(title.ends_with('\u{2026}'));
    }

    #[test]
    fn the_run_start_block_reads_the_sessions_current_list() {
        let dir = std::env::temp_dir().join(format!(
            "claudinio-recital-{}-{}",
            std::process::id(),
            crate::agent::persist::now_ms()
        ));
        std::fs::create_dir_all(&dir).unwrap();
        let store = dir.join("session.jsonl");
        std::fs::write(&store, "").unwrap();
        let ctx = ToolContext {
            session_store_path: Some(store.to_string_lossy().to_string()),
            ..crate::agent::tools::tests_support::ctx()
        };
        assert_eq!(tasks_block(&session_tasks(&ctx)), None, "no list yet");
        crate::agent::persist::append_tasks(&store, &[task("t1", "todo", "first")]).unwrap();
        crate::agent::persist::append_tasks(&store, &[task("t1", "doing", "first")]).unwrap();
        assert_eq!(
            tasks_block(&session_tasks(&ctx)).as_deref(),
            Some("<tasks>\nt1 \u{b7} doing \u{b7} first\n</tasks>"),
            "the latest snapshot, not the first"
        );
        // A context with no session file has nothing to recite.
        assert_eq!(
            tasks_block(&session_tasks(&crate::agent::tools::tests_support::ctx())),
            None
        );
    }

    // A title is cut at TITLE_CHARS characters; in Portuguese, CJK or emoji
    // the byte at that count is inside a character, so the cut is by
    // characters and keeps every one whole.
    #[test]
    fn a_long_multibyte_title_is_cut_on_characters() {
        for unit in ["é", "—", "😀", "日"] {
            let block = tasks_block(&[task("t1", "todo", &unit.repeat(400))]).unwrap();
            let line = block.lines().find(|l| l.starts_with("t1 ")).unwrap();
            let title = line.rsplit(" \u{b7} ").next().unwrap();
            assert_eq!(title.chars().count(), TITLE_CHARS + 1, "{unit}");
            assert!(title.ends_with('\u{2026}'));
        }
    }
}

#[cfg(test)]
pub(crate) mod loop_tests {
    use super::*;
    use crate::agent::events::AgentEvent;
    use crate::agent::handoff::RunOutcome;
    use crate::agent::persist::{SessionRecord, SessionStore, now_ms};
    use crate::agent::provider::AgentConfig;
    use crate::agent::provider::ContentBlock;
    use crate::agent::session::{
        AnswerMap, ApprovalMap, ModeCtl, ModeOrigin, PromptProfile, SessionMode, SteeringCtl,
    };
    use crate::agent::subagent::scripted_model::{Reply, spawn_recording};
    use serde_json::Value;
    use std::sync::Arc;
    use tauri::ipc::Channel;

    pub(crate) struct Workspace {
        pub root: std::path::PathBuf,
        pub store: SessionStore,
    }

    pub(crate) fn workspace(tag: &str, tasks: &[TaskItem]) -> Workspace {
        let root = std::env::temp_dir().join(format!(
            "claudinio-recital-loop-{tag}-{}-{}",
            std::process::id(),
            now_ms()
        ));
        std::fs::create_dir_all(&root).unwrap();
        std::fs::write(root.join("greet.txt"), "hello world\n").unwrap();
        let store = SessionStore {
            path: root.join("session.jsonl"),
        };
        std::fs::write(&store.path, "").unwrap();
        if !tasks.is_empty() {
            crate::agent::persist::append_tasks(&store.path, tasks).unwrap();
        }
        Workspace { root, store }
    }

    /// One run of `message` under `profile` against a model following
    /// `script`. Returns the outcome and every request body sent.
    pub(crate) async fn run(
        ws: &Workspace,
        profile: PromptProfile,
        message: &str,
        script: Vec<Reply>,
    ) -> (Result<RunOutcome, String>, Vec<String>) {
        run_configured(ws, profile, message, script, |_| {}).await
    }

    /// `run`, with the test's config adjusted by `adjust` first.
    pub(crate) async fn run_configured(
        ws: &Workspace,
        profile: PromptProfile,
        message: &str,
        script: Vec<Reply>,
        adjust: impl FnOnce(&mut AgentConfig),
    ) -> (Result<RunOutcome, String>, Vec<String>) {
        let (base, _, bodies) = spawn_recording(script).await;
        let mut config: AgentConfig =
            crate::agent::run_loop::surface_tests::config_for(&base, None);
        adjust(&mut config);
        let ctx = crate::agent::tools::ToolContext {
            workspace_root: Some(ws.root.to_string_lossy().to_string()),
            session_store_path: Some(ws.store.path.to_string_lossy().to_string()),
            ..crate::agent::tools::tests_support::ctx()
        };
        let events: Channel<AgentEvent> = Channel::new(|_| Ok(()));
        let mut history = crate::agent::persist::history_from_records(
            &crate::agent::persist::load_records(&ws.store.path).unwrap_or_default(),
        );
        let outcome = crate::agent::run_loop::run_workflow_with_profile(
            &config,
            &mut history,
            message.into(),
            Vec::new(),
            &events,
            &ApprovalMap::default(),
            &AnswerMap::default(),
            "recital-test",
            &ctx,
            &ws.store,
            &Arc::new(SteeringCtl::new()),
            &Arc::new(ModeCtl::new(SessionMode::Builder, ModeOrigin::Human)),
            profile,
        )
        .await;
        let bodies = bodies.lock().unwrap().clone();
        (outcome, bodies)
    }

    /// The text blocks of the last message of a request body.
    pub(crate) fn last_message_texts(body: &str) -> Vec<String> {
        let body: Value = serde_json::from_str(body).unwrap();
        let last = body["messages"].as_array().unwrap().last().unwrap().clone();
        last["content"]
            .as_array()
            .unwrap()
            .iter()
            .filter_map(|b| b["text"].as_str().map(str::to_string))
            .collect()
    }

    pub(crate) fn todo(id: &str, title: &str) -> TaskItem {
        TaskItem {
            id: id.into(),
            title: title.into(),
            description: "d".into(),
            journal: vec![],
            status: "todo".into(),
        }
    }

    #[tokio::test]
    async fn a_run_on_a_session_with_tasks_ends_its_user_turn_with_the_list() {
        let tasks = [todo("t1", "wire the UI"), todo("t2", "add the test")];
        let ws = workspace("inject", &tasks);
        let (outcome, bodies) = run(
            &ws,
            PromptProfile::Standard,
            "go on",
            vec![Reply::Text("done")],
        )
        .await;
        assert!(
            matches!(outcome, Ok(RunOutcome::Completed)),
            "{:?}",
            outcome.err()
        );
        let texts = last_message_texts(&bodies[0]);
        assert_eq!(texts.first().map(String::as_str), Some("go on"));
        assert_eq!(texts.last(), tasks_block(&tasks).as_ref(), "{texts:?}");
        // Persisted inside the user's Turn, so a reload replays the same
        // request; the `user` record (what the chat shows) is the raw text.
        let records = crate::agent::persist::load_records(&ws.store.path).unwrap();
        let turn_has_block = records.iter().any(|r| {
            match r {
            SessionRecord::Turn { message, .. } => {
                message.role == "user"
                    && message.content.iter().any(|b| {
                        matches!(b, ContentBlock::Text { text, .. } if text.starts_with(TASKS_OPEN))
                    })
            }
            _ => false,
        }
        });
        assert!(turn_has_block);
        assert!(records.iter().any(|r| matches!(
            r,
            SessionRecord::User { text, .. } if text == "go on"
        )));
        let replayed = crate::agent::persist::history_from_records(&records);
        let first_user = replayed.iter().find(|m| m.role == "user").unwrap();
        assert!(matches!(
            first_user.content.last(),
            Some(ContentBlock::Text { text, .. }) if Some(text) == tasks_block(&tasks).as_ref()
        ));
    }

    /// The gate of Phase 4: a Builder run on a session that already has
    /// tasks is handed the list in its first request and is not told to
    /// fetch it — `tasks_get` is not among the tools sent, the prompt names it
    /// only in the deferred catalog — so a model that goes straight to work
    /// finishes in its own rounds, with no round spent on the list and no
    /// `tasks_get` load.
    #[tokio::test]
    async fn a_run_with_existing_tasks_sees_the_list_and_spends_no_round_fetching_it() {
        let tasks = [todo("t1", "say goodbye"), todo("t2", "check it")];
        let ws = workspace("gate", &tasks);
        let path = ws.root.join("greet.txt").to_string_lossy().to_string();
        let (outcome, bodies) = run(
            &ws,
            PromptProfile::Standard,
            "go on",
            vec![
                Reply::Tools(vec![
                    (
                        "tasks_update",
                        serde_json::json!({"id": "t1", "status": "doing"}),
                    ),
                    ("read_file", serde_json::json!({"path": path})),
                ]),
                Reply::Tool {
                    name: "tasks_update",
                    input: serde_json::json!({"updates": [
                        {"id": "t1", "status": "done"},
                        {"id": "t2", "status": "done"}
                    ]}),
                },
                Reply::Text("done"),
            ],
        )
        .await;
        assert!(
            matches!(outcome, Ok(RunOutcome::Completed)),
            "{:?}",
            outcome.err()
        );
        assert_eq!(bodies.len(), 3, "three rounds of work, none for the list");
        let first: Value = serde_json::from_str(&bodies[0]).unwrap();
        let tools: Vec<&str> = first["tools"]
            .as_array()
            .unwrap()
            .iter()
            .filter_map(|t| t["name"].as_str())
            .collect();
        assert!(!tools.contains(&"tasks_get"), "{tools:?}");
        assert!(tools.contains(&"tasks_update"));
        let system = first["system"].to_string();
        assert!(!system.contains("call `tasks_get`"));
        assert_eq!(
            last_message_texts(&bodies[0]).last(),
            tasks_block(&tasks).as_ref()
        );
        let records = crate::agent::persist::load_records(&ws.store.path).unwrap();
        assert!(
            !crate::agent::deferred::loaded_names(&records)
                .iter()
                .any(|n| n == "tasks_get")
        );
        let path = std::path::Path::new(&ws.store.path);
        let after = crate::agent::persist::load_last_tasks(path).unwrap();
        assert!(after.iter().all(|t| t.status == "done"), "{after:?}");
    }

    #[tokio::test]
    async fn a_session_without_tasks_is_sent_no_block() {
        let ws = workspace("no-tasks", &[]);
        let (_, bodies) = run(
            &ws,
            PromptProfile::Standard,
            "hi",
            vec![Reply::Text("hello")],
        )
        .await;
        let texts = last_message_texts(&bodies[0]);
        assert_eq!(texts, ["hi"]);
    }

    // Commit & push has no task system: the git operator is never handed one.
    #[tokio::test]
    async fn the_git_operator_is_never_sent_the_list() {
        let ws = workspace("git", &[todo("t1", "wire the UI")]);
        let (_, bodies) = run(
            &ws,
            PromptProfile::GitSync,
            "commit and push",
            vec![Reply::Text("pushed")],
        )
        .await;
        let texts = last_message_texts(&bodies[0]);
        assert!(texts.iter().all(|t| !t.contains(TASKS_OPEN)), "{texts:?}");
    }

    // A successor session (context handoff, golden flip, Brain to Builder)
    // starts with the tasks `link_session` copied over and its kickoff as the
    // first message: the list is the last thing it reads, after the handoff
    // document.
    #[tokio::test]
    async fn a_successor_sessions_first_message_ends_with_the_list() {
        let tasks = [todo("t1", "wire the UI")];
        let ws = workspace("successor", &tasks);
        let kickoff = crate::agent::transition::compose_context_handoff_message(
            "## Purpose of next session\nfinish it",
            None,
        );
        let (_, bodies) = run(
            &ws,
            PromptProfile::Standard,
            &kickoff,
            vec![Reply::Text("done")],
        )
        .await;
        let texts = last_message_texts(&bodies[0]);
        assert_eq!(texts.first(), Some(&kickoff));
        assert_eq!(texts.last(), tasks_block(&tasks).as_ref());
    }

    // A context handoff from a session with no list: the successor's first
    // message is the kickoff alone, and the kickoff says that is how a
    // session without a list looks, so nothing sends the model looking.
    #[tokio::test]
    async fn a_successor_without_tasks_is_told_no_block_means_no_list() {
        let ws = workspace("successor-no-tasks", &[]);
        let kickoff = crate::agent::transition::compose_context_handoff_message(
            "## Purpose of next session\nfinish it",
            None,
        );
        let (_, bodies) = run(
            &ws,
            PromptProfile::Standard,
            &kickoff,
            vec![Reply::Text("done")],
        )
        .await;
        let texts = last_message_texts(&bodies[0]);
        assert_eq!(texts, std::slice::from_ref(&kickoff));
        assert!(kickoff.contains("no `<tasks>` block means"), "{kickoff}");
    }

    // A compaction between runs that kept no tail (the tail shrinks to zero
    // when it alone is over budget) leaves the history as one user message:
    // the summary, then the recited list. The next run's user turn merges
    // into that message, and the model must still read the list once, not
    // twice (Phase 4 review).
    // The same holds for the document's outline: after a tailless
    // compaction the next run's turn merges into the summary's message, and
    // the outline the compaction restated is not read a second time before
    // the run's own, fresh one.
    #[tokio::test]
    async fn a_run_after_a_tailless_compaction_sees_the_outline_once() {
        let tasks = [todo("t1", "wire the UI")];
        let ws = workspace("tailless-doc", &tasks);
        let (rel, _) = crate::agent::doc_session::create(
            &ws.root,
            Some(&ws.store.path),
            "Pudim",
            &["origem".to_string()],
        )
        .unwrap();
        ws.store.try_append(&SessionRecord::DocumentLinked {
            path: rel.clone(),
            ts: now_ms(),
        });
        let ctx = crate::agent::tools::ToolContext {
            workspace_root: Some(ws.root.to_string_lossy().to_string()),
            session_store_path: Some(ws.store.path.to_string_lossy().to_string()),
            ..crate::agent::tools::tests_support::ctx()
        };
        let outline = crate::agent::doc_session::outline_block(&ctx).unwrap();
        ws.store.try_append(&SessionRecord::Compacted {
            summary: "## Summary\nwork started".into(),
            tail_turns: 0,
            document: Some(outline.clone()),
            ts: now_ms(),
        });
        let (_, bodies) = run(
            &ws,
            PromptProfile::Standard,
            "what next?",
            vec![Reply::Text("t1")],
        )
        .await;
        let body: Value = serde_json::from_str(&bodies[0]).unwrap();
        let outlines = body["messages"]
            .as_array()
            .unwrap()
            .iter()
            .flat_map(|m| m["content"].as_array().cloned().unwrap_or_default())
            .filter(|b| {
                b["text"]
                    .as_str()
                    .is_some_and(|t| t.starts_with("<document "))
            })
            .count();
        assert_eq!(outlines, 1, "{body}");
        let texts = last_message_texts(&bodies[0]);
        assert_eq!(texts.last(), Some(&outline), "{texts:?}");
        assert_eq!(
            texts.iter().filter(|t| t.starts_with(TASKS_OPEN)).count(),
            1,
            "{texts:?}"
        );
    }

    #[tokio::test]
    async fn a_run_after_a_tailless_compaction_sees_the_list_once() {
        let tasks = [todo("t1", "wire the UI")];
        let ws = workspace("tailless", &tasks);
        let turn = |role: &str, text: &str| SessionRecord::Turn {
            message: crate::agent::provider::Message {
                role: role.into(),
                content: vec![ContentBlock::text(text)],
            },
            ts: now_ms(),
        };
        ws.store.try_append(&turn("user", "start it"));
        ws.store.try_append(&turn("assistant", "started"));
        ws.store.try_append(&SessionRecord::Compacted {
            summary: "## Summary\nwork started".into(),
            tail_turns: 0,
            document: None,
            ts: now_ms(),
        });
        let (_, bodies) = run(
            &ws,
            PromptProfile::Standard,
            "what next?",
            vec![Reply::Text("t1")],
        )
        .await;
        let body: Value = serde_json::from_str(&bodies[0]).unwrap();
        let blocks = body["messages"]
            .as_array()
            .unwrap()
            .iter()
            .flat_map(|m| m["content"].as_array().cloned().unwrap_or_default())
            .filter(|b| {
                b["text"]
                    .as_str()
                    .is_some_and(|t| t.starts_with(TASKS_OPEN))
            })
            .count();
        assert_eq!(blocks, 1, "{body}");
        // Once, and last: after the new message, where attention is highest.
        let texts = last_message_texts(&bodies[0]);
        assert_eq!(texts.last(), tasks_block(&tasks).as_ref(), "{texts:?}");
        assert!(texts.iter().any(|t| t == "what next?"));
    }

    fn doing(id: &str, title: &str) -> TaskItem {
        TaskItem {
            status: "doing".into(),
            ..todo(id, title)
        }
    }

    /// The `[system]` texts of the last message of a request body.
    fn nudges_in(body: &str) -> Vec<String> {
        last_message_texts(body)
            .into_iter()
            .filter(|t| t.starts_with("[system]"))
            .collect()
    }

    // The finish line used to be held by a prompt rule ("a final
    // `tasks_update` before your final text"). Now the harness checks: a turn
    // that ends with a task this run set `doing` still `doing` is sent back
    // once, naming the task.
    #[tokio::test]
    async fn a_turn_that_ends_with_a_task_still_doing_is_nudged_once() {
        let ws = workspace(
            "doing-nudge",
            &[todo("t1", "wire the UI"), todo("t2", "add the test")],
        );
        let (outcome, bodies) = run(
            &ws,
            PromptProfile::Standard,
            "go on",
            vec![
                Reply::Tool {
                    name: "tasks_update",
                    input: serde_json::json!({"id": "t1", "status": "doing"}),
                },
                Reply::Text("all done"),
                Reply::Text("really done"),
            ],
        )
        .await;
        assert!(
            matches!(outcome, Ok(RunOutcome::Completed)),
            "{:?}",
            outcome.err()
        );
        // One nudge, then the run is let finish even though t1 is still doing.
        assert_eq!(bodies.len(), 3, "the work, one nudge, one more request");
        let nudge = nudges_in(&bodies[2]);
        assert_eq!(nudge.len(), 1, "{nudge:?}");
        assert!(nudge[0].contains("'t1' (wire the UI)"), "{}", nudge[0]);
        assert!(nudge[0].contains("doing") && nudge[0].contains("`tasks_update`"));
        assert!(
            !nudge[0].contains("'t2'"),
            "only the task left doing: {}",
            nudge[0]
        );
    }

    // A task left `doing` by an earlier run — the user pressed Stop mid-task —
    // is not this run's business: a run that answers an unrelated question
    // and never touches the list is not told to go and finish that work
    // (Phase 4 review). The block already shows the task as `doing`.
    #[tokio::test]
    async fn a_task_left_doing_by_an_earlier_run_does_not_nudge_an_unrelated_answer() {
        let ws = workspace("stale-doing", &[doing("t1", "wire the UI")]);
        let (outcome, bodies) = run(
            &ws,
            PromptProfile::Standard,
            "what does foo() do?",
            vec![Reply::Text("foo() greets the world")],
        )
        .await;
        assert!(
            matches!(outcome, Ok(RunOutcome::Completed)),
            "{:?}",
            outcome.err()
        );
        assert_eq!(bodies.len(), 1, "the answer ends the run, no nudge");
    }

    #[tokio::test]
    async fn a_turn_that_ends_with_nothing_doing_is_not_nudged() {
        let mut closed = doing("t1", "wire the UI");
        closed.status = "done".into();
        let ws = workspace("no-doing", &[closed, todo("t2", "add the test")]);
        let (outcome, bodies) = run(
            &ws,
            PromptProfile::Standard,
            "go on",
            vec![Reply::Text("all done")],
        )
        .await;
        assert!(
            matches!(outcome, Ok(RunOutcome::Completed)),
            "{:?}",
            outcome.err()
        );
        assert_eq!(bodies.len(), 1);
    }

    // A task an earlier run left `doing` that this run works on — a journal
    // entry — is this run's task: ending with it still `doing` is nudged.
    #[tokio::test]
    async fn a_task_this_run_writes_to_while_doing_is_nudged() {
        let ws = workspace("doing-journal", &[doing("t1", "wire the UI")]);
        let (_, bodies) = run(
            &ws,
            PromptProfile::Standard,
            "go on",
            vec![
                Reply::Tool {
                    name: "tasks_update",
                    input: serde_json::json!({"id": "t1", "journal": ["wired the button"]}),
                },
                Reply::Text("all done"),
                Reply::Text("really done"),
            ],
        )
        .await;
        assert_eq!(bodies.len(), 3);
        let nudge = nudges_in(&bodies[2]);
        assert_eq!(nudge.len(), 1, "{nudge:?}");
        assert!(nudge[0].contains("'t1'"), "{}", nudge[0]);
    }

    // "Worked on" means moved to `doing` or written to while `doing` — a
    // status or a journal entry. A task an earlier run left `doing` whose
    // title this run only rewords (resending the list through `tasks_set`)
    // has had no work done on it here, so the answer is not sent back.
    #[tokio::test]
    async fn rewording_a_task_left_doing_by_an_earlier_run_does_not_nudge() {
        let ws = workspace("doing-reworded", &[doing("t1", "wire the UI")]);
        let (outcome, bodies) = run(
            &ws,
            PromptProfile::Standard,
            "rename that task",
            vec![
                Reply::Tool {
                    name: "tasks_set",
                    input: serde_json::json!({"tasks": [{
                        "id": "t1", "title": "wire the settings UI",
                        "description": "", "journal": [], "status": "doing"
                    }]}),
                },
                Reply::Text("renamed it"),
            ],
        )
        .await;
        assert!(
            matches!(outcome, Ok(RunOutcome::Completed)),
            "{:?}",
            outcome.err()
        );
        assert_eq!(bodies.len(), 2, "the rename and the answer, no nudge");
    }

    // A model that closes the task when nudged ends the run on its answer.
    #[tokio::test]
    async fn a_nudged_model_that_closes_the_task_finishes_normally() {
        let ws = workspace("doing-closed", &[todo("t1", "wire the UI")]);
        let (outcome, bodies) = run(
            &ws,
            PromptProfile::Standard,
            "go on",
            vec![
                Reply::Tool {
                    name: "tasks_update",
                    input: serde_json::json!({"id": "t1", "status": "doing"}),
                },
                Reply::Text("all done"),
                Reply::Tool {
                    name: "tasks_update",
                    input: serde_json::json!({"id": "t1", "status": "done"}),
                },
                Reply::Text("closed it"),
            ],
        )
        .await;
        assert!(
            matches!(outcome, Ok(RunOutcome::Completed)),
            "{:?}",
            outcome.err()
        );
        assert_eq!(bodies.len(), 4);
        let path = std::path::Path::new(&ws.store.path);
        assert_eq!(
            crate::agent::persist::load_last_tasks(path).unwrap()[0].status,
            "done"
        );
    }
}

#[cfg(test)]
mod compaction_tests {
    use super::*;
    use crate::agent::persist::{SessionRecord, history_from_records};
    use crate::agent::provider::{ContentBlock, Message};

    fn list() -> Vec<TaskItem> {
        vec![TaskItem {
            id: "t1".into(),
            title: "wire the UI".into(),
            description: "d".into(),
            journal: vec![],
            status: "doing".into(),
        }]
    }

    fn tasks_record(tasks: &[TaskItem]) -> SessionRecord {
        SessionRecord::Tasks {
            tasks_json: serde_json::to_string(tasks).unwrap(),
            ts: 1,
        }
    }

    fn turn(role: &str, content: Vec<ContentBlock>) -> SessionRecord {
        SessionRecord::Turn {
            message: Message {
                role: role.into(),
                content,
            },
            ts: 1,
        }
    }

    fn compacted(tail_turns: usize) -> SessionRecord {
        SessionRecord::Compacted {
            summary: "## Summary\nwhat happened".into(),
            tail_turns,
            document: None,
            ts: 2,
        }
    }

    fn last_block_text(history: &[Message]) -> Option<String> {
        match history.last()?.content.last()? {
            ContentBlock::Text { text, .. } => Some(text.clone()),
            _ => None,
        }
    }

    fn count_blocks(history: &[Message]) -> usize {
        history
            .iter()
            .flat_map(|m| &m.content)
            .filter(
                |b| matches!(b, ContentBlock::Text { text, .. } if text.starts_with(TASKS_OPEN)),
            )
            .count()
    }

    // A compaction in the middle of a run: the kept tail ends on the round's
    // tool results, and the list goes after them — the last thing the model
    // reads, where attention is highest. The loop rebuilds its history with
    // `history_from_records` after compacting, exactly as a reload does, so
    // this one function is what both see.
    #[test]
    fn a_compaction_mid_run_restates_the_list_as_the_last_block() {
        let tasks = list();
        let records = vec![
            tasks_record(&tasks),
            turn("user", vec![ContentBlock::text("do it")]),
            turn(
                "assistant",
                vec![ContentBlock::tool_use(
                    "r1",
                    "read_file",
                    serde_json::json!({}),
                )],
            ),
            turn("user", vec![ContentBlock::tool_result("r1", "contents")]),
            compacted(3),
        ];
        let history = history_from_records(&records);
        assert_eq!(last_block_text(&history), tasks_block(&tasks));
        assert!(matches!(
            history.last().unwrap().content[0],
            ContentBlock::ToolResult { .. }
        ));
        assert_eq!(count_blocks(&history), 1);
    }

    // A session with a living document gets its outline restated the same
    // way, after the list: the compaction recorded the outline as the file
    // stood then, and a mid-run context ends on it. An idle compaction ends
    // on the model's answer and leaves both to the next run.
    #[test]
    fn a_compaction_mid_run_restates_the_document_outline_after_the_list() {
        let tasks = list();
        let outline = "<document path=\".claudinio/docs/p.md\">\n# P\ns1 \u{b7} done \u{b7} main \u{b7} origem\n</document>".to_string();
        let compacted_with_doc = |tail_turns| match compacted(tail_turns) {
            SessionRecord::Compacted { summary, ts, .. } => SessionRecord::Compacted {
                summary,
                tail_turns,
                document: Some(outline.clone()),
                ts,
            },
            other => other,
        };
        let mid_run = vec![
            tasks_record(&tasks),
            turn("user", vec![ContentBlock::text("do it")]),
            turn(
                "assistant",
                vec![ContentBlock::tool_use(
                    "r1",
                    "read_file",
                    serde_json::json!({}),
                )],
            ),
            turn("user", vec![ContentBlock::tool_result("r1", "contents")]),
            compacted_with_doc(3),
        ];
        let history = history_from_records(&mid_run);
        let last = &history.last().unwrap().content;
        let texts: Vec<&str> = last
            .iter()
            .filter_map(|b| match b {
                ContentBlock::Text { text, .. } => Some(text.as_str()),
                _ => None,
            })
            .collect();
        assert_eq!(
            texts,
            [tasks_block(&tasks).unwrap().as_str(), outline.as_str()],
            "the list, then the outline, last"
        );
        let idle = vec![
            turn("user", vec![ContentBlock::text("do it")]),
            turn("assistant", vec![ContentBlock::text("done")]),
            compacted_with_doc(2),
        ];
        assert_eq!(
            history_from_records(&idle).last().unwrap().role,
            "assistant"
        );
    }

    // A compaction at a run's start keeps the run's user turn, which already
    // ends with the list and the outline it was sent with: neither is said
    // twice.
    #[test]
    fn a_tail_that_already_ends_with_the_list_and_the_outline_gets_neither_again() {
        let tasks = list();
        let block = tasks_block(&tasks).unwrap();
        let outline = "<document path=\".claudinio/docs/p.md\">\n# P\n</document>".to_string();
        let records = vec![
            tasks_record(&tasks),
            turn(
                "user",
                vec![
                    ContentBlock::text("go on"),
                    ContentBlock::text(block.clone()),
                    ContentBlock::text(outline.clone()),
                ],
            ),
            SessionRecord::Compacted {
                summary: "## Summary\nwhat happened".into(),
                tail_turns: 1,
                document: Some(outline.clone()),
                ts: 2,
            },
        ];
        let history = history_from_records(&records);
        assert_eq!(count_blocks(&history), 1);
        assert_eq!(history.last().unwrap().content.len(), 3, "{history:?}");
        assert_eq!(last_block_text(&history), Some(outline));
    }

    // A compaction at a run's start whose turn ends with a `<doc-changed>`
    // note (the person edited the document since the model last heard): the
    // list and the outline before the note are the ones the compaction would
    // restate, and neither is said twice.
    #[test]
    fn a_tail_ending_with_a_doc_changed_note_after_the_recital_gets_nothing_again() {
        let tasks = list();
        let block = tasks_block(&tasks).unwrap();
        let outline = "<document path=\".claudinio/docs/p.md\">\n# P\n</document>".to_string();
        let note = "<doc-changed>\nThe user changed the document: s1.\n</doc-changed>".to_string();
        let records = vec![
            tasks_record(&tasks),
            turn(
                "user",
                vec![
                    ContentBlock::text("go on"),
                    ContentBlock::text(block.clone()),
                    ContentBlock::text(outline.clone()),
                    ContentBlock::text(note.clone()),
                ],
            ),
            SessionRecord::Compacted {
                summary: "## Summary\nwhat happened".into(),
                tail_turns: 1,
                document: Some(outline.clone()),
                ts: 2,
            },
        ];
        let history = history_from_records(&records);
        assert_eq!(count_blocks(&history), 1, "{history:?}");
        assert_eq!(history.last().unwrap().content.len(), 4, "{history:?}");
        assert_eq!(last_block_text(&history), Some(note));
    }

    // After a tailless compaction the history ends `[summary, list, outline]`
    // as the compaction recorded them. If the person edits the document
    // before the next run, that run's outline differs from the recorded one;
    // the recital blocks are still the compaction's restatement, and the run
    // appends its own: they go by kind, not by equal text, or the model reads
    // the list and two outlines.
    #[test]
    fn the_run_drops_a_trailing_recital_whose_outline_changed_since_the_compaction() {
        let tasks = list();
        let block = tasks_block(&tasks).unwrap();
        let recorded = "<document path=\".claudinio/docs/p.md\">\n# P\ns1 \u{b7} todo\n</document>";
        let fresh = "<document path=\".claudinio/docs/p.md\">\n# P\ns1 \u{b7} done\n</document>";
        let mut history = vec![Message {
            role: "user".into(),
            content: vec![
                ContentBlock::text("## Summary\nwhat happened"),
                ContentBlock::text(block.clone()),
                ContentBlock::text(recorded),
            ],
        }];
        take_trailing_recitals(&mut history, &[block, fresh.to_string()]);
        assert_eq!(history[0].content.len(), 1, "{history:?}");
        assert!(matches!(
            &history[0].content[0],
            ContentBlock::Text { text, .. } if text.starts_with("## Summary")
        ));
    }

    // No tail kept: the list follows the summary, in the same message.
    #[test]
    fn with_no_tail_the_list_follows_the_summary() {
        let tasks = list();
        let records = vec![
            tasks_record(&tasks),
            turn("user", vec![ContentBlock::text("do it")]),
            compacted(0),
        ];
        let history = history_from_records(&records);
        assert_eq!(history.len(), 1);
        let ContentBlock::Text { text, .. } = &history[0].content[0] else {
            panic!("the summary first")
        };
        assert!(text.contains("what happened"));
        assert_eq!(last_block_text(&history), tasks_block(&tasks));
    }

    // A compaction at a run's start keeps the run's user turn, which already
    // ends with the list it was sent with: it is not said twice.
    #[test]
    fn a_tail_that_already_ends_with_the_list_does_not_get_a_second_copy() {
        let tasks = list();
        let block = tasks_block(&tasks).unwrap();
        let records = vec![
            tasks_record(&tasks),
            turn(
                "user",
                vec![
                    ContentBlock::text("go on"),
                    ContentBlock::text(block.clone()),
                ],
            ),
            compacted(1),
        ];
        let history = history_from_records(&records);
        assert_eq!(count_blocks(&history), 1);
        assert_eq!(last_block_text(&history), Some(block));
    }

    // The list recited is the one that stood when the compaction ran; what
    // happens after it is appended after it, as the loop appended it.
    #[test]
    fn the_list_is_the_one_at_the_compaction_and_later_turns_follow_it() {
        let before = list();
        let mut after = list();
        after[0].status = "done".into();
        let records = vec![
            tasks_record(&before),
            turn("user", vec![ContentBlock::text("do it")]),
            compacted(1),
            turn("assistant", vec![ContentBlock::text("finished")]),
            tasks_record(&after),
        ];
        let history = history_from_records(&records);
        assert_eq!(
            history.len(),
            3,
            "summary, the tail's user turn, the answer"
        );
        assert_eq!(
            last_block_text(&history[..2]),
            tasks_block(&before),
            "recited at the compaction point"
        );
        assert_eq!(last_block_text(&history), Some("finished".into()));
    }

    // An idle compaction (the /compact button between runs) ends on the
    // model's answer; the next run's user turn carries the list, so nothing
    // is added here — an assistant turn is never followed by a user message
    // the loop did not write.
    #[test]
    fn an_idle_compaction_leaves_the_list_to_the_next_run() {
        let records = vec![
            tasks_record(&list()),
            turn("user", vec![ContentBlock::text("do it")]),
            turn("assistant", vec![ContentBlock::text("done")]),
            compacted(2),
        ];
        let history = history_from_records(&records);
        assert_eq!(count_blocks(&history), 0);
        assert_eq!(history.last().unwrap().role, "assistant");
    }

    // Sessions without a list, and every session that was never compacted,
    // replay exactly as before.
    #[test]
    fn without_a_list_or_a_compaction_nothing_changes() {
        let records = vec![
            turn("user", vec![ContentBlock::text("do it")]),
            turn(
                "assistant",
                vec![ContentBlock::tool_use(
                    "r1",
                    "read_file",
                    serde_json::json!({}),
                )],
            ),
            turn("user", vec![ContentBlock::tool_result("r1", "x")]),
            compacted(3),
        ];
        assert_eq!(count_blocks(&history_from_records(&records)), 0);
        let mut uncompacted = vec![tasks_record(&list())];
        uncompacted.extend(records.into_iter().take(3));
        assert_eq!(count_blocks(&history_from_records(&uncompacted)), 0);
    }
}
