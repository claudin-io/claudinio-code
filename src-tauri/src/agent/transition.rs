//! Session handoff: creates linked successor sessions when the context
//! crosses the configured threshold or a new top-level topic opens a new
//! document. The old session gets a HandoffTo forward pointer;
//! the new session gets a LinkedFrom back-pointer. Both share the same
//! event channel so the frontend renders the chain as one continuous thread.

use crate::agent::persist as tasks_store;
use crate::agent::persist::{self, SessionRecord, SessionStore, now_ms};
use crate::agent::provider;
use crate::agent::session::{AgentEvent, HandoffReason, HandoffSpec, SteeringCtl};
use crate::state::{SessionHandle, WorkspaceState};
use std::collections::HashMap;
use std::sync::Arc;
use tauri::ipc::Channel;
use tokio::sync::Mutex;

pub use crate::agent::persist::RecordsCache;

/// The `AppState` maps a transition needs, cloned before `tokio::spawn` so the
/// driver loop can link sessions without borrowing Tauri state.
#[derive(Clone)]
pub struct TransitionMaps {
    pub steering: Arc<Mutex<HashMap<String, Arc<SteeringCtl>>>>,

    pub records_cache: RecordsCache,
}

/// The predecessor's tasks a successor starts with. Every handoff carries
/// the whole list, except a new top-level topic (`NewDocument`): its work is
/// a new document with a list of its own, so the old topic's tasks stay
/// behind — but the golden ones go along. They are the user's mandatory
/// goals (`<goal>` tags), and the golden gate only lets the work finish when
/// each is done; dropping them would let a second `doc_create` end the run
/// with goals still pending.
pub fn tasks_for_successor(
    tasks: Vec<persist::TaskItem>,
    reason: &HandoffReason,
) -> Vec<persist::TaskItem> {
    if *reason != HandoffReason::NewDocument {
        return tasks;
    }
    tasks
        .into_iter()
        .filter(crate::agent::tools::tasks::is_golden)
        .collect()
}

/// Create a new linked session and wire it as the workspace's active session.
/// Returns the new session handle so the caller can rebuild the ToolContext and
/// continue the run loop.
pub async fn link_session(
    maps: &TransitionMaps,
    ws: &Arc<WorkspaceState>,
    old_handle: &SessionHandle,
    spec: &HandoffSpec,
    event_tx: &Channel<AgentEvent>,
) -> Result<SessionHandle, String> {
    let workspace_root = Some(ws.root.to_string_lossy().to_string());
    let new_id = uuid::Uuid::new_v4().to_string();

    // Load old records once for copying tasks and base_commit.
    let old_records = persist::load_records_cached(&old_handle.store_path, &maps.records_cache)
        .unwrap_or_default();

    // 1. Append HandoffTo on the old session (superseded).
    let old_store = SessionStore {
        path: old_handle.store_path.clone(),
    };
    old_store.try_append(&SessionRecord::HandoffTo {
        next_session_id: new_id.clone(),
        reason: spec.reason.as_str().into(),
        ts: now_ms(),
    });
    persist::invalidate_cache(&old_handle.store_path, &maps.records_cache);

    // 2. Create the new session file.
    let new_store = SessionStore::create(&new_id, workspace_root.as_deref())?;
    let new_path = new_store.path.clone();

    // 3. Append LinkedFrom right after Meta.
    new_store.try_append(&SessionRecord::LinkedFrom {
        prev_session_id: old_handle.id.clone(),
        reason: spec.reason.as_str().into(),
        golden_cycle: spec.golden_cycle,
        golden_stalls: spec.golden_stalls,
        golden_last_pending: spec.golden_last_pending.clone(),
        ts: now_ms(),
    });

    // 4. Copy BaseCommit from old session (anchors the Implementation Log range).
    // The timestamp travels with it: it marks when the work began, and the
    // quality gate reads commits older than that as not this session's own.
    // Stamping the handoff time instead would disown everything the run
    // committed before handing off.
    if let Some((sha, ts)) = persist::earliest_base_commit_at(&old_records) {
        new_store.try_append(&SessionRecord::BaseCommit { sha, ts });
    }

    // 5. Copy tasks snapshot (golden tasks included; a new top-level topic
    // keeps only those — see `tasks_for_successor`).
    if let Ok(tasks) = tasks_store::load_last_tasks(&old_handle.store_path) {
        let tasks = tasks_for_successor(tasks, &spec.reason);
        if !tasks.is_empty() {
            let _ = tasks_store::append_tasks(&new_path, &tasks);
        }
    }

    // 5½. The living document: the predecessor's travels with it, or the new
    // one for a new topic (lean-harness Phase 4b).
    crate::agent::doc_session::carry_into(
        &old_handle.store_path,
        &old_records,
        &new_store,
        spec.document.as_deref(),
    );

    // 6½. Seed the successor's cost ledger with cumulative chain cost
    // so the footer never drops to zero on handoff — each session
    // inherits the cost of its entire ancestry.
    {
        let chain_cost = persist::chain_cumulative_cost(workspace_root.as_deref(), &old_handle.id);
        if chain_cost.0.is_some()
            || chain_cost.1.is_some()
            || chain_cost.2.is_some()
            || chain_cost.3.is_some()
        {
            new_store.try_append(&SessionRecord::Status {
                session_id: new_id.clone(),
                total_input_tokens: 0,
                total_output_tokens: 0,
                total_cost: chain_cost.0,
                total_cost_input: chain_cost.1,
                total_cost_output: chain_cost.2,
                total_cost_cache_read: chain_cost.3,
                context_tokens: None,
                ts: now_ms(),
            });
        }
    }

    // 7. Move the SteeringCtl from the old session id to the new one
    // (any queued steering survives the transition). Done BEFORE swapping
    // active_session so a queue_steering racing the swap still finds a ctl.
    {
        let mut steering_map = maps.steering.lock().await;
        if let Some(ctl) = steering_map.remove(&old_handle.id) {
            steering_map.insert(new_id.clone(), ctl);
        }
    }

    // 9. Swap the active session on the workspace.
    let new_handle = SessionHandle {
        id: new_id.clone(),
        store_path: new_path,
    };
    {
        let mut guard = ws.active_session.lock().await;
        *guard = Some(new_handle.clone());
    }

    // 10. Notify the frontend so it stitches the new session into the thread.
    let _ = event_tx.send(AgentEvent::SessionLinked {
        prev_session_id: old_handle.id.clone(),
        session_id: new_id.clone(),
        reason: spec.reason.as_str().into(),
        first_message: spec.first_message.clone(),
    });

    Ok(new_handle)
}

/// Wraps the model-generated handoff document as the first user message of
/// the successor session. Tells the model to read the handoff and the task
/// list the run appends after it (`agent::task_recital`), and continue. The
/// run appends one only when the session has a list, so the text says what
/// its absence means instead of promising it.
pub fn compose_context_handoff_message(handoff_text: &str, plan_file: Option<&str>) -> String {
    let mut msg = String::from(
        "[system] This is a fresh session that continues previous work whose context \
         reached its limit. The predecessor wrote the handoff document below; any \
         current task list follows it as a `<tasks>` block at the end of this message \
         (no `<tasks>` block means there is no list). Read them and continue the \
         in-flight work exactly where it stopped.\n\n",
    );
    if let Some(path) = plan_file {
        msg.push_str(&format!("Document: {path}\n\n"));
    }
    msg.push_str(&format!(
        "--- HANDOFF DOCUMENT START ---\n{handoff_text}\n--- HANDOFF DOCUMENT END ---"
    ));
    msg
}

/// Build the ToolContext for a newly linked session, reusing workspace-level
/// resources (MCP, LSP, embeddings) from the old context but pointing at the
/// new session's store path.
pub fn rebuild_tool_context(
    old_ctx: &crate::agent::tools::ToolContext,
    new_session_id: &str,
    new_store_path: &std::path::Path,
    config: provider::AgentConfig,
) -> crate::agent::tools::ToolContext {
    crate::agent::tools::ToolContext {
        // The successor inherits the same resolved set and the same approval —
        // a handoff is one run continuing under a new id, not a new run — but
        // re-points at the successor's transcript and is told to report a
        // SessionStart. Its source is `compact`: the successor starts with an
        // empty history and a handoff document, which is a compacted context,
        // not a user opening the app (`startup`) and not a human action
        // (`resume`).
        doc_access: Default::default(),
        hooks: old_ctx.hooks.as_ref().map(|h| {
            let next = h
                .for_session(new_session_id, new_store_path.to_path_buf())
                .with_store(crate::agent::persist::SessionStore {
                    path: new_store_path.to_path_buf(),
                });
            if let Ok(mut pending) = next.pending_session_start.lock() {
                *pending = Some(crate::agent::hooks::SessionStartSource::Compact);
            }
            Arc::new(next)
        }),
        db_path: old_ctx.db_path.clone(),
        lsp_manager: old_ctx.lsp_manager.clone(),
        workspace_root: old_ctx.workspace_root.clone(),
        embedding_model: old_ctx.embedding_model.clone(),
        session_store_path: Some(new_store_path.to_string_lossy().to_string()),
        limits: Default::default(),
        read_tracker: Arc::new(Mutex::new(crate::agent::tools::ReadTracker::default())),
        browser: None,
        interrupt: old_ctx.interrupt.clone(),
        agent_config: Some(config),
        base_commit: old_ctx.base_commit.clone(),
        auto_approve_git: old_ctx.auto_approve_git,
        shell_gate: old_ctx.shell_gate.clone(),
        mcp: old_ctx.mcp.clone(),
        index_progress: old_ctx.index_progress.clone(),
        records_cache: old_ctx.records_cache.clone(),
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn a_task() -> persist::TaskItem {
        persist::TaskItem {
            id: "t1".into(),
            title: "Parse the config".into(),
            description: "Use the nom grammar from the spike".into(),
            journal: vec!["the spike lives in spikes/nom.rs".into()],
            status: "todo".into(),
        }
    }

    /// A new top-level topic leaves the old topic's tasks behind but takes
    /// the golden ones, recognised by the golden prefix itself; every other
    /// handoff carries the whole list.
    #[test]
    fn a_new_document_successor_keeps_only_the_golden_tasks() {
        let golden = persist::TaskItem {
            id: format!("{}tests-pass-1", crate::agent::session::GOLDEN_TASK_PREFIX),
            ..a_task()
        };
        let tasks = vec![a_task(), golden.clone()];
        let kept = tasks_for_successor(tasks.clone(), &HandoffReason::NewDocument);
        assert_eq!(kept.iter().map(|t| &t.id).collect::<Vec<_>>(), [&golden.id]);
        assert_eq!(
            tasks_for_successor(tasks.clone(), &HandoffReason::ContextHandoff).len(),
            2
        );
    }

    #[test]
    fn context_handoff_message_wraps_document() {
        let msg = compose_context_handoff_message("## Purpose\nfinish it", Some("/p/doc.md"));
        assert!(msg.contains("--- HANDOFF DOCUMENT START ---"));
        assert!(msg.contains("finish it"));
        assert!(msg.contains("/p/doc.md"));
        assert!(!msg.contains("tasks_get"), "{msg}");
        assert!(msg.contains("`<tasks>` block"), "{msg}");
    }

    // The run appends a `<tasks>` block only when the session has a list
    // (single-step work has none). A kickoff written before the successor's
    // first run cannot know, so it must not promise a block that may not
    // come: a model told a list "ends this message" that finds none goes
    // looking for it with `tasks_get`, which is deferred — a load and a
    // prefix rewrite (Phase 4 review). It says that no block means no list.
    #[test]
    fn handoff_kickoffs_do_not_promise_a_task_list_the_session_may_not_have() {
        let msg = compose_context_handoff_message("## Purpose\nfinish it", None);
        assert!(!msg.contains("block that ends this message"), "{msg}");
        assert!(msg.contains("no `<tasks>` block means"), "{msg}");
    }
}
