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
//! way, in its first message, after the handoff document.

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

/// The block for the session `ctx` writes to, as its task list stands now.
pub fn run_start_block(ctx: &ToolContext) -> Option<String> {
    let path = ctx.session_store_path.as_deref()?;
    let tasks = crate::agent::persist::load_last_tasks(std::path::Path::new(path)).ok()?;
    tasks_block(&tasks)
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
        assert_eq!(run_start_block(&ctx), None, "no list yet");
        crate::agent::persist::append_tasks(&store, &[task("t1", "todo", "first")]).unwrap();
        crate::agent::persist::append_tasks(&store, &[task("t1", "doing", "first")]).unwrap();
        assert_eq!(
            run_start_block(&ctx).as_deref(),
            Some("<tasks>\nt1 \u{b7} doing \u{b7} first\n</tasks>"),
            "the latest snapshot, not the first"
        );
        // A context with no session file has nothing to recite.
        assert_eq!(
            run_start_block(&crate::agent::tools::tests_support::ctx()),
            None
        );
    }
}

#[cfg(test)]
mod loop_tests {
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

    pub(super) struct Workspace {
        pub root: std::path::PathBuf,
        pub store: SessionStore,
    }

    pub(super) fn workspace(tag: &str, tasks: &[TaskItem]) -> Workspace {
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
    pub(super) async fn run(
        ws: &Workspace,
        profile: PromptProfile,
        message: &str,
        script: Vec<Reply>,
    ) -> (Result<RunOutcome, String>, Vec<String>) {
        let (base, _, bodies) = spawn_recording(script).await;
        let config: AgentConfig = crate::agent::run_loop::surface_tests::config_for(&base, None);
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
    pub(super) fn last_message_texts(body: &str) -> Vec<String> {
        let body: Value = serde_json::from_str(body).unwrap();
        let last = body["messages"].as_array().unwrap().last().unwrap().clone();
        last["content"]
            .as_array()
            .unwrap()
            .iter()
            .filter_map(|b| b["text"].as_str().map(str::to_string))
            .collect()
    }

    pub(super) fn todo(id: &str, title: &str) -> TaskItem {
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
}
