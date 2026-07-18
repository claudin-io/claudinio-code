//! Session handoff: creates linked successor sessions when the Brain flips to
//! Builder (plan execution), the golden loop flips mode, or the context crosses
//! the configured threshold. The old session gets a HandoffTo forward pointer;
//! the new session gets a LinkedFrom back-pointer. Both share the same
//! event channel so the frontend renders the chain as one continuous thread.

use crate::agent::persist::{self, now_ms, SessionRecord, SessionStore};
use crate::agent::provider;
use crate::agent::session::{AgentEvent, HandoffReason, HandoffSpec, ModeCtl, SteeringCtl};
use crate::commands::tasks as tasks_cmd;
use crate::state::{SessionHandle, WorkspaceState};
use lru::LruCache;
use std::collections::HashMap;
use std::path::PathBuf;
use std::sync::Arc;
use std::time::Instant;
use tauri::ipc::Channel;
use tokio::sync::Mutex;

/// Shared LRU cache of parsed session records (same shape as
/// `AppState.records_cache`).
pub type RecordsCache =
    Arc<std::sync::Mutex<LruCache<PathBuf, (Vec<SessionRecord>, Instant)>>>;

/// The `AppState` maps a transition needs, cloned before `tokio::spawn` so the
/// driver loop can link sessions without borrowing Tauri state.
#[derive(Clone)]
pub struct TransitionMaps {
    pub steering: Arc<Mutex<HashMap<String, Arc<SteeringCtl>>>>,
    pub modes: Arc<Mutex<HashMap<String, Arc<ModeCtl>>>>,
    pub records_cache: RecordsCache,
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
    let old_records =
        persist::load_records_cached(&old_handle.store_path, &maps.records_cache)
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
    if let Some(sha) = persist::earliest_base_commit(&old_records) {
        new_store.try_append(&SessionRecord::BaseCommit { sha, ts: now_ms() });
    }

    // 5. Copy tasks snapshot (golden tasks included).
    if let Ok(tasks) = tasks_cmd::load_last_tasks(&old_handle.store_path) {
        if !tasks.is_empty() {
            let _ = tasks_cmd::append_tasks(&new_path, &tasks);
        }
    }

    // 6. Append the Mode record for the new session.
    new_store.try_append(&SessionRecord::Mode {
        mode: spec.next_mode.as_str().into(),
        origin: spec.next_origin.as_str().into(),
        ts: now_ms(),
    });

    // 7. Move the SteeringCtl from the old session id to the new one
    // (any queued steering survives the transition). Done BEFORE swapping
    // active_session so a queue_steering racing the swap still finds a ctl.
    {
        let mut steering_map = maps.steering.lock().await;
        if let Some(ctl) = steering_map.remove(&old_handle.id) {
            steering_map.insert(new_id.clone(), ctl);
        }
    }

    // 8. Insert new ModeCtl and remove the old one.
    {
        let mut mode_map = maps.modes.lock().await;
        mode_map.remove(&old_handle.id);
        mode_map.insert(
            new_id.clone(),
            Arc::new(ModeCtl::new(spec.next_mode, spec.next_origin)),
        );
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
        mode: spec.next_mode.as_str().into(),
        first_message: spec.first_message.clone(),
    });

    Ok(new_handle)
}

/// The first user message of the successor session. Plan-execution handoffs
/// (exit_plan_mode / the Continue with Builder button) get the full plan
/// inlined — the plan IS the handoff; the spec's message is a placeholder.
/// Golden-flip and context handoffs already carry their composed message.
pub fn resolve_first_message(
    spec: &HandoffSpec,
    workspace_root: Option<&str>,
    plan_save_path: Option<&str>,
) -> String {
    match spec.reason {
        HandoffReason::PlanExecution | HandoffReason::ManualBuilder => {
            let plan = workspace_root.and_then(|root| {
                crate::agent::tools::write_plan::latest_plan_path(root, plan_save_path)
            });
            let path = plan.as_ref().map(|p| p.to_string_lossy().to_string());
            let content = plan.as_ref().and_then(|p| std::fs::read_to_string(p).ok());
            compose_builder_kickoff(path.as_deref(), content.as_deref())
        }
        HandoffReason::GoldenFlip | HandoffReason::ContextHandoff => {
            spec.first_message.clone()
        }
    }
}

/// The first message for a Builder session kicked off from a Brain plan.
/// Carries the plan inline (the plan is the handoff document) plus the
/// execution contract.
pub fn compose_builder_kickoff(
    plan_file: Option<&str>,
    plan_content: Option<&str>,
) -> String {
    let mut msg = String::from(
        "[system] You are in Builder mode, in a fresh session that continues a \
         planning session. The plan below and the carried-over task list are your \
         worklist - execute them exactly:\n\n\
         1. Call `tasks_get` FIRST to load the task list.\n\
         2. Follow the plan (Solution Design + Low-Level Design).\n\
         3. Execute ONE task at a time in dependency order:\n\
           - Mark it 'doing' BEFORE touching code.\n\
           - Delegate implementation to code-mode subagents.\n\
           - Mark it 'done' AFTER verifying with evidence.\n\
         4. After all tasks, verify the whole build and call `finalize_plan`.\n",
    );
    if let Some(path) = plan_file {
        msg.push_str(&format!("\nPlan file: {path}\n"));
    }
    if let Some(content) = plan_content {
        msg.push_str(&format!(
            "\n--- PLAN START ---\n{content}\n--- PLAN END ---\n"
        ));
    } else if plan_file.is_none() {
        msg.push_str(
            "\nNo plan file was found on disk - call `tasks_get` and continue from \
             the task list alone.\n",
        );
    }
    msg
}

/// Wraps the model-generated handoff document as the first user message of
/// the successor session. Tells the model to read the handoff, load tasks,
/// and continue.
pub fn compose_context_handoff_message(handoff_text: &str, plan_file: Option<&str>) -> String {
    let mut msg = String::from(
        "[system] This is a fresh session that continues previous work whose context \
         reached its limit. The predecessor wrote the handoff document below. Read it, \
         then call `tasks_get` FIRST and continue the in-flight work exactly where it \
         stopped.\n\n",
    );
    if let Some(path) = plan_file {
        msg.push_str(&format!("Plan file: {path}\n\n"));
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
    new_store_path: &std::path::Path,
    new_mode_ctl: Arc<ModeCtl>,
    config: provider::AgentConfig,
) -> crate::agent::tools::ToolContext {
    crate::agent::tools::ToolContext {
        db_path: old_ctx.db_path.clone(),
        lsp_manager: old_ctx.lsp_manager.clone(),
        workspace_root: old_ctx.workspace_root.clone(),
        embedding_model: old_ctx.embedding_model.clone(),
        session_store_path: Some(new_store_path.to_string_lossy().to_string()),
        read_tracker: Arc::new(Mutex::new(crate::agent::tools::ReadTracker::default())),
        interrupt: old_ctx.interrupt.clone(),
        agent_config: Some(config),
        plan_save_path: old_ctx.plan_save_path.clone(),
        base_commit: old_ctx.base_commit.clone(),
        auto_approve_git: old_ctx.auto_approve_git,
        mcp: old_ctx.mcp.clone(),
        mode_ctl: Some(new_mode_ctl),
        index_progress: old_ctx.index_progress.clone(),
        records_cache: old_ctx.records_cache.clone(),
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn builder_kickoff_is_ascii_and_mentions_contract() {
        let msg = compose_builder_kickoff(Some("/tmp/plan.md"), Some("# Plan\nDo X"));
        assert!(msg.is_ascii(), "kickoff must pass reject_non_english");
        assert!(msg.contains("tasks_get"));
        assert!(msg.contains("/tmp/plan.md"));
        assert!(msg.contains("--- PLAN START ---"));
        assert!(msg.contains("finalize_plan"));
    }

    #[test]
    fn builder_kickoff_without_plan_falls_back_to_tasks() {
        let msg = compose_builder_kickoff(None, None);
        assert!(msg.is_ascii());
        assert!(msg.contains("No plan file was found"));
    }

    #[test]
    fn context_handoff_message_wraps_document() {
        let msg = compose_context_handoff_message("## Purpose\nfinish it", Some("/p/plan.md"));
        assert!(msg.is_ascii());
        assert!(msg.contains("--- HANDOFF DOCUMENT START ---"));
        assert!(msg.contains("finish it"));
        assert!(msg.contains("/p/plan.md"));
        assert!(msg.contains("tasks_get"));
    }
}
