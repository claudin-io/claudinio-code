use crate::agent::persist as tasks_store;
use crate::agent::persist::{
    self, AttachmentMeta, SessionRecord, SessionStore, SessionSummary, load_records, now_ms,
};
use crate::agent::provider::{ContentBlock, save_config};
use crate::agent::session::{self, AgentEvent, SteeringEntry};
use crate::agent::tools::{ReadTracker, ToolContext};
use crate::agent::transition;
use crate::imageutil::ImageAttachment;
use crate::state::{AppState, SessionHandle};
use serde::{Deserialize, Serialize};
use serde_json::Value;
use std::path::Path;
use std::sync::Arc;
use tauri::State;
use tauri::ipc::Channel;
use tokio::sync::Mutex;

/// Process attachment inputs into content blocks and their lightweight metadata.
/// Used by both `send_message` and `queue_steering`.
///
/// Returns a vector of (ContentBlock, AttachmentMeta) pairs so the caller can
/// forward the content blocks to the workflow and persist the metadata for
/// UI display (timeline pills).
pub fn process_attachments(atts: &[AttachmentInput]) -> Vec<(ContentBlock, AttachmentMeta)> {
    let mut results = Vec::new();
    for att in atts {
        let file_path = Path::new(&att.path);
        if !file_path.exists() {
            continue;
        }
        let ext = file_path
            .extension()
            .and_then(|e| e.to_str())
            .map(|e| e.to_lowercase())
            .unwrap_or_default();
        let file_name = file_path
            .file_name()
            .and_then(|n| n.to_str())
            .unwrap_or("file")
            .to_string();
        let file_size = file_path.metadata().map(|m| m.len()).unwrap_or(0);

        let is_image = matches!(
            ext.as_str(),
            "png" | "jpg" | "jpeg" | "gif" | "webp" | "bmp"
        );
        let is_text = matches!(
            ext.as_str(),
            "txt"
                | "md"
                | "csv"
                | "json"
                | "yaml"
                | "yml"
                | "toml"
                | "rs"
                | "ts"
                | "tsx"
                | "js"
                | "jsx"
                | "py"
                | "swift"
                | "go"
                | "rb"
                | "html"
                | "htm"
                | "css"
                | "sh"
                | "bash"
                | "sql"
                | "xml"
                | "log"
        );

        let media_type = if is_image {
            match ext.as_str() {
                "png" => "image/png".to_string(),
                "jpg" | "jpeg" => "image/jpeg".to_string(),
                "gif" => "image/gif".to_string(),
                "webp" => "image/webp".to_string(),
                "bmp" => "image/bmp".to_string(),
                _ => "image/png".to_string(),
            }
        } else if is_text {
            "text/plain".to_string()
        } else {
            format!("application/{}", ext)
        };
        let meta = AttachmentMeta {
            name: file_name.clone(),
            media_type,
            size: file_size,
        };

        if is_image {
            let bytes = match std::fs::read(file_path) {
                Ok(b) => b,
                Err(_) => continue,
            };
            let media_type = match ext.as_str() {
                "png" => "image/png",
                "jpg" | "jpeg" => "image/jpeg",
                "gif" => "image/gif",
                "webp" => "image/webp",
                "bmp" => "image/bmp",
                _ => "image/png",
            };
            let att = ImageAttachment::from_bytes(&bytes, media_type, &ext);
            results.push((
                ContentBlock::image(&att.media_type, &att.data, att.width, att.height),
                meta,
            ));
        } else if is_text {
            let text = match std::fs::read_to_string(file_path) {
                Ok(t) => t,
                Err(_) => continue,
            };
            let block_text = format!("[Arquivo anexado: `{file_name}`]\n```\n{text}\n```");
            results.push((ContentBlock::text(block_text), meta));
        } else {
            let size_str = if file_size > 1024 * 1024 {
                format!("{:.1} MB", file_size as f64 / (1024.0 * 1024.0))
            } else if file_size > 1024 {
                format!("{:.1} KB", file_size as f64 / 1024.0)
            } else {
                format!("{file_size} B")
            };
            let block_text = format!("[Arquivo anexado: `{file_name}` ({size_str}) — tipo: {ext}]");
            results.push((ContentBlock::text(block_text), meta));
        }
    }
    results
}

#[derive(Serialize)]
#[serde(rename_all = "camelCase")]
pub struct SessionStarted {
    pub session_id: String,
}

/// An attachment the user wants to send to the agent along with their message.
#[derive(Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct AttachmentInput {
    /// Absolute path to the file on disk.
    pub path: String,
}

#[tauri::command]
pub async fn send_message(
    workspace: String,
    message: String,
    attachments: Option<Vec<AttachmentInput>>,
    event_channel: Channel<AgentEvent>,
    state: State<'_, AppState>,
) -> Result<SessionStarted, String> {
    let config = {
        let cfg = state.config.lock().await;
        if cfg.api_key.is_empty() {
            return Err("API key not configured. Use set_config first.".into());
        }
        cfg.clone()
    };

    let ws = state.workspace(&workspace).await?;
    let workspace_root = Some(ws.root.to_string_lossy().to_string());

    // Load workspace-level config (.claudinio.json) and merge over local config
    let mut config = config;
    if let Some(ref root) = workspace_root
        && let Some(ws_cfg) = crate::agent::provider::read_workspace_config(root)
    {
        crate::agent::provider::merge_workspace_config(&mut config, &ws_cfg);
    }

    // Continue the workspace's active session, or start a fresh one persisted
    // to its own JSONL file.
    let mut session_is_new = false;
    let handle = {
        let mut guard = ws.active_session.lock().await;
        match guard.as_ref() {
            Some(h) => h.clone(),
            None => {
                let id = uuid::Uuid::new_v4().to_string();
                let store = SessionStore::create(&id, workspace_root.as_deref())?;
                let h = SessionHandle {
                    id,
                    store_path: store.path,
                };
                *guard = Some(h.clone());
                session_is_new = true;
                h
            }
        }
    };

    // The JSONL file is the source of truth: rebuild history from it so the
    // conversation continues across turns and restarts.
    let history = load_records(&handle.store_path)
        .map(|recs| persist::history_from_records(&recs))
        .unwrap_or_default();

    let store = SessionStore {
        path: handle.store_path.clone(),
    };

    // Reset this session's steering for the new run, then drain any residual
    // from a race.
    let steering = state.steering_for(&handle.id).await;
    steering.clear();

    let db_path: Option<String> = Some(ws.index_db_path.to_string_lossy().to_string());

    // Capture the git HEAD at run start so finalize_plan can compute the
    // changed files / commits since this plan's work began. Best-effort:
    // None when the workspace is not a git repo (or git is unavailable).
    let base_commit: Option<String> = workspace_root
        .as_ref()
        .and_then(|root| crate::agent::tools::git_head(root));

    let mcp = ws.ensure_mcp_connected(&config).await;

    // Hooks: resolve the set for this run and decide which SessionStart, if
    // any, this message is. The four sources map onto states the command layer
    // already tracks, and the decision has to happen here because it is the
    // only place that knows whether a session was just created, just cleared,
    // or is being picked up after a restart.
    let hook_ctx = {
        // Only the first message of a session starts it. The second one is not
        // a new session, and firing SessionStart per turn would make a memory
        // hook re-introduce itself every time the user typed.
        let first_run_here = state.session_started.lock().await.insert(handle.id.clone());
        let cleared = ws.cleared.swap(false, std::sync::atomic::Ordering::SeqCst);
        let source = if !first_run_here {
            None
        } else if cleared {
            Some(crate::agent::hooks::SessionStartSource::Clear)
        } else if session_is_new {
            Some(crate::agent::hooks::SessionStartSource::Startup)
        } else {
            // The session exists on disk but this process has not run it: the
            // app restarted, or the user opened it from the history list.
            Some(crate::agent::hooks::SessionStartSource::Resume)
        };
        build_hook_ctx(
            ws.resolve_hooks(&config).await,
            state.hook_trust.clone(),
            &handle.id,
            &handle.store_path,
            &ws.root,
            Some(steering.interrupt.clone()),
            source,
        )
    };

    let ctx = ToolContext {
        doc_access: Default::default(),
        hooks: Some(hook_ctx),
        db_path,
        lsp_manager: Some(ws.lsp_manager.clone()),
        workspace_root,
        embedding_model: state.embedding_model.clone(),
        session_store_path: Some(handle.store_path.to_string_lossy().to_string()),
        limits: Default::default(),
        read_tracker: Arc::new(Mutex::new(ReadTracker::default())),
        browser: Some(ws.browser.clone()),
        interrupt: Some(steering.interrupt.clone()),
        agent_config: Some(config.clone()),
        base_commit,
        auto_approve_git: false,
        shell_gate: None,
        mcp: Some(mcp),
        index_progress: Some(ws.index_progress.clone()),
        records_cache: state.records_cache.clone(),
    };

    let residual = steering.drain();
    let message = if residual.is_empty() {
        message
    } else {
        let mut prefix = String::new();
        for r in &residual {
            prefix.push_str(&r.text);
            prefix.push('\n');
        }
        prefix.push_str(&message);
        prefix
    };

    // Collect attachment blocks from any residual steering entries
    let mut attachment_blocks: Vec<ContentBlock> = Vec::new();
    for entry in &residual {
        for (block, _) in &entry.attachments {
            attachment_blocks.push(block.clone());
        }
    }

    // Golden goals: extract <goal>...</goal> tags, strip them from the text
    // sent to the model, and materialize each goal as golden tasks the
    // workflow's golden loop will enforce until done.
    let (cleaned, goals) = session::parse_goals(&message);
    let message = if goals.is_empty() {
        message
    } else {
        let mut tasks = tasks_store::load_last_tasks(&handle.store_path).unwrap_or_default();
        // New goals replace previous golden tasks; normal tasks are preserved.
        tasks.retain(|t| !crate::agent::tools::tasks::is_golden(t));
        tasks.extend(crate::agent::tools::tasks::create_golden_tasks(&goals));
        if let Err(e) = tasks_store::append_tasks(&handle.store_path, &tasks) {
            eprintln!("failed to persist golden tasks: {e}");
        }
        if cleaned.is_empty() {
            format!(
                "Reach the following mandatory goals (tracked as golden tasks): {}",
                goals.join("; ")
            )
        } else {
            cleaned
        }
    };

    // Process attachments into content blocks to prepend to the user message
    let mut attachment_blocks: Vec<ContentBlock> = Vec::new();
    if let Some(atts) = attachments {
        let processed = process_attachments(&atts);
        for (block, _) in &processed {
            attachment_blocks.push(block.clone());
        }
    }

    let session_id = handle.id.clone();
    spawn_run_loop(RunLoopArgs {
        config,
        ws,
        maps: transition_maps(&state),
        approvals: state.approvals.clone(),
        answers: state.answers.clone(),
        chan: event_channel,
        handle,
        store,
        ctx,
        steering,
        history,
        message,
        attachment_blocks,
    });

    Ok(SessionStarted { session_id })
}

/// Build the hook context for a run.
///
/// Every entry point that starts a run — or compacts one — needs this, and the
/// only thing that differs between them is which `SessionStart`, if any, the run
/// should report. Getting it wrong in one place would leave a whole surface
/// (manual compaction, Builder continuation, commit & push) quietly hookless.
#[allow(clippy::too_many_arguments)]
fn build_hook_ctx(
    set: Arc<crate::agent::hooks::HookSet>,
    trust: Arc<crate::agent::hooks::TrustStore>,
    session_id: &str,
    store_path: &std::path::Path,
    workspace_root: &std::path::Path,
    interrupt: Option<Arc<std::sync::atomic::AtomicBool>>,
    session_start: Option<crate::agent::hooks::SessionStartSource>,
) -> Arc<crate::agent::hooks::HookCtx> {
    let ctx = crate::agent::hooks::HookCtx::new(
        set,
        trust,
        session_id,
        store_path.to_path_buf(),
        workspace_root.to_path_buf(),
    )
    .with_store(SessionStore {
        path: store_path.to_path_buf(),
    })
    .with_interrupt(interrupt);
    if let Some(src) = session_start
        && let Ok(mut pending) = ctx.pending_session_start.lock()
    {
        *pending = Some(src);
    }
    Arc::new(ctx)
}

/// The `AppState` maps a session transition needs, cloned so the spawned run
/// loop can link sessions after the Tauri state borrow ends.
fn transition_maps(state: &State<'_, AppState>) -> transition::TransitionMaps {
    transition::TransitionMaps {
        steering: state.steering.clone(),
        records_cache: state.records_cache.clone(),
    }
}

struct RunLoopArgs {
    config: crate::agent::provider::AgentConfig,
    ws: Arc<crate::state::WorkspaceState>,
    maps: transition::TransitionMaps,
    approvals: session::ApprovalMap,
    answers: session::AnswerMap,
    chan: Channel<AgentEvent>,
    handle: SessionHandle,
    store: SessionStore,
    ctx: ToolContext,
    steering: Arc<session::SteeringCtl>,
    history: Vec<crate::agent::provider::Message>,
    message: String,
    attachment_blocks: Vec<ContentBlock>,
}

/// Drive a session run to completion, following handoffs across linked
/// sessions: whenever `run_workflow` returns `RunOutcome::Handoff`, a new
/// linked session is created and the loop continues there with a fresh
/// history — same event channel, so the frontend streams one continuous
/// conversation.
fn spawn_run_loop(args: RunLoopArgs) {
    let RunLoopArgs {
        config: cfg,
        ws,
        maps,
        approvals: appr,
        answers: answ,
        chan,
        mut handle,
        mut store,
        mut ctx,
        steering,
        mut history,
        mut message,
        mut attachment_blocks,
    } = args;

    tokio::spawn(async move {
        loop {
            let msg = std::mem::take(&mut message);
            let atts = std::mem::take(&mut attachment_blocks);
            match session::run_workflow(
                &cfg,
                &mut history,
                msg,
                atts,
                &chan,
                &appr,
                &answ,
                &handle.id,
                &ctx,
                &store,
                &steering,
            )
            .await
            {
                Ok(session::RunOutcome::Completed) => break,
                Ok(session::RunOutcome::Handoff(spec)) => {
                    let spec = *spec;
                    match transition::link_session(&maps, &ws, &handle, &spec, &chan).await {
                        Ok(new_handle) => {
                            ctx = transition::rebuild_tool_context(
                                &ctx,
                                &new_handle.id,
                                &new_handle.store_path,
                                cfg.clone(),
                            );
                            store = SessionStore {
                                path: new_handle.store_path.clone(),
                            };
                            history = Vec::new();
                            message = spec.first_message;
                            handle = new_handle;
                        }
                        Err(e) => {
                            store.try_append(&SessionRecord::Error {
                                message: e.clone(),
                                ts: now_ms(),
                            });
                            let _ = chan.send(AgentEvent::Error(e));
                            break;
                        }
                    }
                }
                Err(e) => {
                    store.try_append(&SessionRecord::Error {
                        message: e.clone(),
                        ts: now_ms(),
                    });
                    let _ = chan.send(AgentEvent::Error(e));
                    break;
                }
            }
        }
        // Run finished (success, error or panic-free return): drop the
        // steering entry so interrupt/steer report "session not running".
        // `handle.id` tracks the FINAL session of the chain — link_session
        // moved the steering entry along with each handoff.
        let mut map = maps.steering.lock().await;
        map.remove(&handle.id);
    });
}

/// Start a new conversation in a workspace: the next `send_message` opens a
/// fresh JSONL session there.
#[tauri::command]
pub async fn new_session(workspace: String, state: State<'_, AppState>) -> Result<(), String> {
    let ws = state.workspace(&workspace).await?;
    let config = state.config.lock().await.clone();
    let mut guard = ws.active_session.lock().await;
    if let Some(h) = guard.as_ref() {
        // SessionEnd, awaited: this is the user clearing the conversation, and
        // a hook whose job is to write down what the session learned has to get
        // to run before the session it learned it in stops being the active one.
        let hooks = crate::agent::hooks::HookCtx::new(
            ws.resolve_hooks(&config).await,
            state.hook_trust.clone(),
            &h.id,
            h.store_path.clone(),
            ws.root.clone(),
        )
        .with_store(SessionStore {
            path: h.store_path.clone(),
        });
        crate::agent::hooks::fire_session_end(
            &hooks,
            crate::agent::hooks::SessionEndReason::Clear,
            None,
        )
        .await;
        state.session_started.lock().await.remove(&h.id);
        state.remove_steering(&h.id).await;
        // A session abandoned before any real content (only meta/mode records,
        // e.g. created lazily by a mode toggle) is noise on disk — delete it
        // instead of leaving an "(empty session)" orphan in the list.
        let is_empty = persist::load_records(&h.store_path)
            .map(|recs| {
                !recs
                    .iter()
                    .any(|r| matches!(r, SessionRecord::User { .. } | SessionRecord::Turn { .. }))
            })
            .unwrap_or(false);
        if is_empty {
            let _ = std::fs::remove_file(&h.store_path);
            persist::invalidate_cache(&h.store_path, &state.records_cache);
        }
    }
    *guard = None;
    // The next run in this workspace is a `clear`, not a `startup`.
    ws.cleared.store(true, std::sync::atomic::Ordering::SeqCst);
    Ok(())
}

/// List saved sessions for a workspace, newest first.
#[tauri::command]
pub async fn list_sessions(
    workspace: String,
    state: State<'_, AppState>,
) -> Result<Vec<SessionSummary>, String> {
    let ws = state.workspace(&workspace).await?;
    let root = ws.root.to_string_lossy().to_string();
    persist::list_sessions(Some(&root))
}

/// Reopen a saved session: makes it the workspace's active conversation and
/// returns its full record stream so the frontend can replay the transcript.
///
/// Linked sessions are resolved as one conversation: the requested id is first
/// followed FORWARD (`handoff_to`) to the chain tip — reopening a superseded
/// link must never resume a stale session — then the tip's ancestry is walked
/// BACKWARD (`linked_from`) and every predecessor's records are prepended, with
/// the `linked_from` markers left in place as chain dividers. The chain tip
/// becomes the active session.
#[tauri::command]
pub async fn load_session(
    workspace: String,
    session_id: String,
    state: State<'_, AppState>,
) -> Result<Vec<SessionRecord>, String> {
    let ws = state.workspace(&workspace).await?;
    let root = ws.root.to_string_lossy().to_string();
    let dir = persist::sessions_dir(Some(&root))?;

    let load_one = |id: &str| -> Result<Vec<SessionRecord>, String> {
        let path = dir.join(format!("{id}.jsonl"));
        if !path.exists() {
            return Err(format!("session '{id}' not found"));
        }
        load_records(&path)
    };

    const MAX_CHAIN_HOPS: usize = 64;
    let mut seen: std::collections::HashSet<String> = std::collections::HashSet::new();

    // Forward to the chain tip.
    let mut tip_id = session_id;
    let mut tip_records = load_one(&tip_id)?;
    seen.insert(tip_id.clone());
    for _ in 0..MAX_CHAIN_HOPS {
        let Some(next) = persist::handoff_to(&tip_records) else {
            break;
        };
        if !seen.insert(next.clone()) {
            break; // cycle guard
        }
        let Ok(next_records) = load_one(&next) else {
            break;
        };
        tip_id = next;
        tip_records = next_records;
    }

    // Backward through the ancestry, prepending each predecessor.
    let mut chain: Vec<Vec<SessionRecord>> = vec![tip_records];
    for _ in 0..MAX_CHAIN_HOPS {
        let earliest = chain.first().map(|v| v.as_slice()).unwrap_or(&[]);
        let Some(info) = persist::linked_from(earliest) else {
            break;
        };
        if !seen.insert(info.prev_session_id.clone()) {
            break; // cycle guard
        }
        let Ok(prev_records) = load_one(&info.prev_session_id) else {
            break;
        };
        chain.insert(0, prev_records);
    }

    let records: Vec<SessionRecord> = chain.into_iter().flatten().collect();

    let tip_path = dir.join(format!("{tip_id}.jsonl"));
    {
        let mut guard = ws.active_session.lock().await;
        *guard = Some(SessionHandle {
            id: tip_id,
            store_path: tip_path,
        });
    }
    Ok(records)
}

#[derive(Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct ApproveArgs {
    pub session_id: String,
    pub tool_id: String,
}

#[tauri::command]
pub async fn approve_tool(args: ApproveArgs, state: State<'_, AppState>) -> Result<(), String> {
    let key = format!("{}:{}", args.session_id, args.tool_id);
    let mut map = state.approvals.lock().await;
    if let Some(sender) = map.remove(&key) {
        sender
            .send(true)
            .map_err(|_| "session already closed".into())
    } else {
        Err("approval request not found or already handled".into())
    }
}

#[tauri::command]
pub async fn reject_tool(args: ApproveArgs, state: State<'_, AppState>) -> Result<(), String> {
    let key = format!("{}:{}", args.session_id, args.tool_id);
    let mut map = state.approvals.lock().await;
    if let Some(sender) = map.remove(&key) {
        sender
            .send(false)
            .map_err(|_| "session already closed".into())
    } else {
        Err("approval request not found or already handled".into())
    }
}

#[derive(Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct SubmitAnswersArgs {
    pub session_id: String,
    pub tool_id: String,
    pub answers: Vec<session::UserAnswer>,
}

/// Resolve a pending ask_user tool call with the user's answers.
#[tauri::command]
pub async fn submit_answers(
    args: SubmitAnswersArgs,
    state: State<'_, AppState>,
) -> Result<(), String> {
    let key = format!("{}:{}", args.session_id, args.tool_id);
    let mut map = state.answers.lock().await;
    if let Some(sender) = map.remove(&key) {
        sender
            .send(args.answers)
            .map_err(|_| "session already closed".into())
    } else {
        Err("question request not found or already handled".into())
    }
}

#[derive(Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct SetConfigArgs {
    pub base_url: Option<String>,
    pub api_key: Option<String>,
    pub model: Option<String>,
    pub max_rounds: Option<Option<usize>>,
    pub sub_max_rounds: Option<Option<usize>>,
    pub yolo_mode: Option<bool>,
    pub yolo_blacklist: Option<Vec<String>>,
    pub keep_awake: Option<bool>,
    pub max_golden_cycles: Option<Option<usize>>,
    pub max_golden_stalls: Option<Option<usize>>,
    pub max_parallel_agents: Option<Option<usize>>,
    pub override_base_url: Option<String>,
    pub override_api_key: Option<String>,
    pub mcp: Option<std::collections::HashMap<String, crate::agent::provider::McpServerEntry>>,
    pub code_intel_enabled: Option<bool>,
    pub preferred_ide: Option<String>,
    pub handoff_context_tokens: Option<Option<u64>>,
    pub thinking_effort: Option<String>,
    pub browser: Option<crate::browser::BrowserPrefs>,
    pub local: Option<crate::llama::LocalPrefs>,
    pub jev_enabled: Option<bool>,
    /// A TypeSafe API key; an empty string removes it.
    pub jev_api_key: Option<String>,
    /// One model's tool surface — see `agent::surface`.
    pub tool_surface: Option<ToolSurfaceArg>,
    /// Whether a new session's first run reads AGENTS.md/CLAUDE.md and the
    /// project memory (lean-harness Phase 9). See `agent::project_memory`.
    pub project_memory: Option<bool>,
}

#[derive(Deserialize)]
pub struct ToolSurfaceArg {
    pub model: String,
    /// "full" | "lean".
    pub surface: String,
}

#[tauri::command]
pub async fn set_config(args: SetConfigArgs, state: State<'_, AppState>) -> Result<(), String> {
    let mut cfg = state.config.lock().await;
    if let Some(url) = args.base_url {
        cfg.base_url = url;
    }
    if let Some(key) = args.api_key {
        cfg.api_key = key;
    }
    if let Some(model) = args.model {
        cfg.model = model;
    }
    if let Some(max_rounds) = args.max_rounds {
        cfg.max_rounds = max_rounds;
    }
    if let Some(sub_max_rounds) = args.sub_max_rounds {
        cfg.sub_max_rounds = sub_max_rounds;
    }
    if let Some(yolo_mode) = args.yolo_mode {
        cfg.yolo_mode = yolo_mode;
    }
    if let Some(yolo_blacklist) = args.yolo_blacklist {
        cfg.yolo_blacklist = yolo_blacklist;
    }
    if let Some(keep_awake) = args.keep_awake {
        cfg.keep_awake = keep_awake;
    }
    if let Some(browser) = args.browser {
        // Clamp on the boundary: these come from settings JSON a user may
        // hand-edit, and a 0-wide viewport is not something to hand a browser.
        let (w, h) = browser.clamped_viewport();
        cfg.browser = crate::browser::BrowserPrefs {
            viewport_width: w,
            viewport_height: h,
            ..browser
        };
    }
    if let Some(local) = args.local {
        // Clamped on the boundary, like `browser` above: these reach the
        // llama-server command line, where a 0-slot server or a hundred
        // resident models is not a setting, it is a hang.
        cfg.local = crate::llama::LocalPrefs {
            parallel: local.parallel.clamp(1, 16),
            max_loaded_models: local.max_loaded_models.clamp(1, 2),
            // Below 2 there is no block to speculate on and the engine refuses
            // to start; above 8 the drafts miss more often than they save.
            draft_block_size: local.draft_block_size.clamp(2, 8),
            ..local
        };
    }
    if let Some(max_golden_cycles) = args.max_golden_cycles {
        cfg.max_golden_cycles = max_golden_cycles;
    }
    if let Some(max_golden_stalls) = args.max_golden_stalls {
        cfg.max_golden_stalls = max_golden_stalls;
    }
    if let Some(max_parallel_agents) = args.max_parallel_agents {
        cfg.max_parallel_agents = max_parallel_agents.map(|n| n.clamp(1, 8));
    }
    if let Some(url) = args.override_base_url {
        cfg.override_base_url = if url.is_empty() { None } else { Some(url) };
    }
    if let Some(key) = args.override_api_key {
        cfg.override_api_key = if key.is_empty() { None } else { Some(key) };
    }
    if let Some(mcp) = args.mcp {
        cfg.mcp = mcp;
    }
    crate::agent::jev::apply_settings(&mut cfg, args.jev_enabled, args.jev_api_key);
    if let Some(choice) = args.tool_surface {
        crate::agent::surface::set_for(&mut cfg, &choice.model, &choice.surface);
    }
    if let Some(code_intel_enabled) = args.code_intel_enabled {
        cfg.code_intel_enabled = code_intel_enabled;
    }
    if let Some(preferred_ide) = args.preferred_ide {
        cfg.preferred_ide = if preferred_ide.is_empty() {
            None
        } else {
            Some(preferred_ide)
        };
    }
    if let Some(handoff_context_tokens) = args.handoff_context_tokens {
        cfg.handoff_context_tokens = handoff_context_tokens.map(|n| n.clamp(120_000, 256_000));
    }
    if let Some(project_memory) = args.project_memory {
        cfg.project_memory = project_memory;
    }
    if let Some(thinking_effort) = args.thinking_effort
        && ["low", "medium", "high", "xhigh", "max"].contains(&thinking_effort.as_str())
    {
        cfg.thinking_effort = thinking_effort;
    }
    save_config(&cfg);
    Ok(())
}

#[tauri::command]
pub async fn get_config(
    workspace: Option<String>,
    state: State<'_, AppState>,
) -> Result<Value, String> {
    let mut cfg = state.config.lock().await;

    // Load workspace config and merge if a workspace is specified
    let workspace_config = if let Some(ref ws_root) = workspace {
        crate::agent::provider::read_workspace_config(ws_root)
    } else {
        None
    };
    if let Some(ref ws) = workspace_config {
        crate::agent::provider::merge_workspace_config(&mut cfg, ws);
    }

    // What the context meter shows before any run has reported its own
    // numbers: the lines the builder model will be held to. The prefix is not
    // built here, so on a small window this is a little generous until the
    // first `SessionStats` event corrects it.
    let budget = crate::agent::budget::ContextBudget::for_model(&cfg, cfg.model(), 0);

    let mut value = serde_json::json!({
        "baseUrl": cfg.base_url,
        "model": cfg.model,
        "hasApiKey": !cfg.api_key.is_empty(),
        "maxContextTokens": budget.ceiling,
        "compactThreshold": budget.hard,
        "maxRounds": cfg.max_rounds,
        "subMaxRounds": cfg.sub_max_rounds,
        "yoloMode": cfg.yolo_mode,
        "yoloBlacklist": cfg.yolo_blacklist,
        "keepAwake": cfg.keep_awake,
        "accountLogin": cfg.account_login,
        "accountTier": cfg.account_tier,
        "maxGoldenCycles": cfg.max_golden_cycles,
        "maxGoldenStalls": cfg.max_golden_stalls,
        "maxParallelAgents": cfg.max_parallel_agents,
        "overrideBaseUrl": cfg.override_base_url,
        "overrideApiKey": cfg.override_api_key,
        "mcp": cfg.mcp,
        "codeIntelEnabled": cfg.code_intel_enabled,
        "preferredIde": cfg.preferred_ide,
        "handoffContextTokens": cfg.handoff_context_tokens,
        "thinkingEffort": cfg.thinking_effort,
        "browser": cfg.browser,
        "local": cfg.local,
        // Which credential Jev uses — never the key.
        "jev": crate::agent::jev::status_json(&cfg),
        // Models set to something other than the full tool catalog.
        "toolSurface": cfg.tool_surface,
        // "compact" when the model's window is too small for the Standard
        // prompt — Settings says so rather than offering a surface that
        // would not apply.
        "profile": crate::agent::surface::effective_profile(
            session::PromptProfile::Standard,
            &cfg,
            false,
        ).as_str(),
        // Connected external providers — never the keys (hasApiKey precedent).
        "providers": cfg.providers.iter().map(|(id, p)| {
            (id.clone(), serde_json::json!({
                "connected": true,
                "baseUrl": p.base_url,
                "label": p.label,
                "protocol": p.protocol,
                "enabledModels": p.enabled_models,
                // What the custom-provider form needs to reopen an entry.
                "custom": p.custom,
                "models": p.custom_models,
                "contextWindow": p.context_window,
                "hasApiKey": !p.api_key.is_empty(),
            }))
        }).collect::<serde_json::Map<String, Value>>(),
        "workspaceConfig": workspace_config,
    });
    // Outside the literal: `json!` is at its macro recursion limit here.
    value["projectMemory"] = Value::from(cfg.project_memory);
    Ok(value)
}

/// Fetch available models from the API. Calls GET {base_url}/v1/models and
/// parses the response, falling back to ["claudinio", "claudius"] on any error.
#[tauri::command]
pub async fn list_models(state: State<'_, AppState>) -> Result<Vec<String>, String> {
    let cfg = state.config.lock().await;
    let base_url = cfg.base_url.trim_end_matches('/').to_string();
    let api_key = cfg.api_key.clone();
    drop(cfg);

    let url = format!("{base_url}/v1/models");
    let _net_guard = crate::net_activity::NetGuard::begin(
        crate::net_activity::NetSource::ListModels,
        "/v1/models",
    );
    let client = crate::http::default_client();
    let response = match client.get(&url).header("x-api-key", &api_key).send().await {
        Ok(r) if r.status().is_success() => r,
        _ => return Ok(vec!["claudinio".into(), "claudius".into()]),
    };

    let body: Value = match response.json().await {
        Ok(v) => v,
        Err(_) => return Ok(vec!["claudinio".into(), "claudius".into()]),
    };

    // Try common response shapes:
    // 1. { data: [{ id: "claudinio" }, { id: "claudius" }] }
    if let Some(data) = body.get("data").and_then(|d| d.as_array()) {
        let models: Vec<String> = data
            .iter()
            .filter_map(|item| {
                item.get("id")
                    .and_then(|id| id.as_str())
                    .map(|s| s.to_string())
            })
            .collect();
        if !models.is_empty() {
            return Ok(models);
        }
    }
    // 2. Array of strings directly
    if let Some(arr) = body.as_array() {
        let models: Vec<String> = arr
            .iter()
            .filter_map(|v| v.as_str().map(|s| s.to_string()))
            .collect();
        if !models.is_empty() {
            return Ok(models);
        }
    }
    // 3. { models: ["claudinio", "claudius"] }
    if let Some(models_arr) = body.get("models").and_then(|m| m.as_array()) {
        let models: Vec<String> = models_arr
            .iter()
            .filter_map(|v| v.as_str().map(|s| s.to_string()))
            .collect();
        if !models.is_empty() {
            return Ok(models);
        }
    }

    Ok(vec!["claudinio".into(), "claudius".into()])
}

/// Push a steering message into the queue for a running session.
#[tauri::command]
pub async fn queue_steering(
    session_id: String,
    text: String,
    attachments: Option<Vec<AttachmentInput>>,
    state: State<'_, AppState>,
) -> Result<(), String> {
    let ctl = {
        let map = state.steering.lock().await;
        map.get(&session_id).cloned()
    };
    match ctl {
        Some(ctl) => {
            let processed = if let Some(ref atts) = attachments {
                process_attachments(atts)
            } else {
                Vec::new()
            };
            ctl.push(SteeringEntry {
                text,
                attachments: processed,
            });
            Ok(())
        }
        None => Err("session not running".into()),
    }
}

/// Manually force context compaction for a workspace's active session.
/// Returns the generated summary.
#[tauri::command]
pub async fn compact_session(
    workspace: String,
    session_id: String,
    event_channel: Channel<AgentEvent>,
    state: State<'_, AppState>,
) -> Result<String, String> {
    let config = {
        let cfg = state.config.lock().await;
        if cfg.api_key.is_empty() {
            return Err("API key not configured.".into());
        }
        cfg.clone()
    };

    let ws = state.workspace(&workspace).await?;
    let workspace_root = Some(ws.root.to_string_lossy().to_string());
    let handle = {
        let guard = ws.active_session.lock().await;
        match guard.as_ref() {
            Some(h) if h.id == session_id => h.clone(),
            _ => return Err("no active session or session mismatch".into()),
        }
    };

    let store = SessionStore {
        path: handle.store_path.clone(),
    };

    // Reuse the running session's controller if present; otherwise use a
    // throwaway one so we don't leave a stale "running" entry in the map.
    let steering = {
        let map = state.steering.lock().await;
        map.get(&handle.id).cloned()
    }
    .unwrap_or_else(|| std::sync::Arc::new(crate::agent::session::SteeringCtl::new()));

    let db_path: Option<String> = Some(ws.index_db_path.to_string_lossy().to_string());

    let ctx = ToolContext {
        // No SessionStart: this is not a run, it is a compaction. PreCompact is
        // the event that belongs here, and it fires with `trigger: "manual"`.
        doc_access: Default::default(),
        hooks: Some(build_hook_ctx(
            ws.resolve_hooks(&config).await,
            state.hook_trust.clone(),
            &handle.id,
            &handle.store_path,
            &ws.root,
            Some(steering.interrupt.clone()),
            None,
        )),
        db_path,
        lsp_manager: Some(ws.lsp_manager.clone()),
        workspace_root,
        embedding_model: state.embedding_model.clone(),
        session_store_path: Some(handle.store_path.to_string_lossy().to_string()),
        limits: Default::default(),
        read_tracker: Arc::new(Mutex::new(ReadTracker::default())),
        browser: Some(ws.browser.clone()),
        interrupt: Some(steering.interrupt.clone()),
        agent_config: Some(config.clone()),
        base_commit: None,
        auto_approve_git: false,
        shell_gate: None,
        // Compaction only summarizes history; it never dispatches tool calls.
        mcp: None,
        index_progress: Some(ws.index_progress.clone()),
        records_cache: state.records_cache.clone(),
    };

    let summary = session::compact_history(
        &config,
        &store,
        &ctx,
        &event_channel,
        &state.approvals,
        &state.answers,
        &handle.id,
        &steering,
        crate::agent::hooks::CompactTrigger::Manual,
        // No run is in flight, so the prompt and tools that make up the prefix
        // are not built here. Without them the tail budget errs slightly
        // generous on a small window and is exact on every other.
        crate::agent::budget::ContextBudget::for_model(&config, config.model(), 0).tail_tokens,
    )
    .await?;

    Ok(summary)
}

#[derive(Serialize)]
#[serde(rename_all = "camelCase")]
pub struct DocumentEntry {
    pub name: String,
    pub path: String,
    pub modified_at: u64,
}

/// The workspace's living documents (`.claudinio/docs/*.md`), newest first:
/// the "Documents" list beside the chat.
#[tauri::command]
pub async fn list_documents(
    workspace: String,
    state: State<'_, AppState>,
) -> Result<Vec<DocumentEntry>, String> {
    let ws = state.workspace(&workspace).await?;
    Ok(documents_in(&ws.root))
}

fn documents_in(root: &std::path::Path) -> Vec<DocumentEntry> {
    let dir = root.join(".claudinio").join("docs");
    let mut docs: Vec<DocumentEntry> = std::fs::read_dir(&dir)
        .map(|entries| {
            entries
                .flatten()
                .filter(|entry| entry.path().extension().and_then(|e| e.to_str()) == Some("md"))
                .filter_map(|entry| {
                    let path = entry.path();
                    let name = path.file_stem()?.to_str()?.to_string();
                    let modified_at = entry
                        .metadata()
                        .ok()?
                        .modified()
                        .ok()?
                        .duration_since(std::time::UNIX_EPOCH)
                        .unwrap_or_default()
                        .as_secs();
                    Some(DocumentEntry {
                        name,
                        path: path.to_string_lossy().to_string(),
                        modified_at,
                    })
                })
                .collect()
        })
        .unwrap_or_default();
    docs.sort_by_key(|d| std::cmp::Reverse(d.modified_at));
    docs
}

/// Set the interrupt flag on a running session's steering controller.
#[tauri::command]
pub async fn interrupt_session(
    session_id: String,
    state: State<'_, AppState>,
) -> Result<(), String> {
    let ctl = {
        let map = state.steering.lock().await;
        map.get(&session_id).cloned()
    };
    match ctl {
        Some(ctl) => {
            ctl.interrupt
                .store(true, std::sync::atomic::Ordering::SeqCst);
            Ok(())
        }
        None => Err("session not running".into()),
    }
}

/// Start a new standalone session that commits and pushes all changes, then
/// cleans up. Unlike send_message, this does NOT attach to the workspace's
/// active session — it creates a temporary session with auto-approval for git
/// commands. The caller receives the session_id to stream events.
#[tauri::command]
pub async fn commit_and_push(
    workspace: String,
    event_channel: Channel<AgentEvent>,
    state: State<'_, AppState>,
) -> Result<SessionStarted, String> {
    let config = {
        let cfg = state.config.lock().await;
        if cfg.api_key.is_empty() {
            return Err("API key not configured.".into());
        }
        cfg.clone()
    };

    let ws = state.workspace(&workspace).await?;
    let workspace_root = Some(ws.root.to_string_lossy().to_string());

    // Merge workspace-level config (.claudinio.json) over local config
    let mut config = config;
    if let Some(ref root) = workspace_root
        && let Some(ws_cfg) = crate::agent::provider::read_workspace_config(root)
    {
        crate::agent::provider::merge_workspace_config(&mut config, &ws_cfg);
    }

    // === FRESH session — NOT attached to ws.active_session ===
    let id = uuid::Uuid::new_v4().to_string();
    let store = SessionStore::create(&id, workspace_root.as_deref())?;

    // History is empty for this brand-new session
    let mut history = Vec::new();

    let steering = state.steering_for(&id).await;

    let db_path: Option<String> = Some(ws.index_db_path.to_string_lossy().to_string());

    let base_commit: Option<String> = workspace_root
        .as_ref()
        .and_then(|root| crate::agent::tools::git_head(root));

    let mcp = ws.ensure_mcp_connected(&config).await;

    let ctx = ToolContext {
        // Commit & push runs `bash` against the user's repository with git
        // auto-approved, which is precisely what a Bash guard hook exists for —
        // so PreToolUse and PostToolUse fire. There is no user prompt and no
        // conversation here, so SessionStart, UserPromptSubmit and Stop do not
        // (see the profile check in `run_workflow_with_profile`).
        doc_access: Default::default(),
        hooks: Some(build_hook_ctx(
            ws.resolve_hooks(&config).await,
            state.hook_trust.clone(),
            &id,
            &store.path,
            &ws.root,
            Some(steering.interrupt.clone()),
            None,
        )),
        db_path,
        lsp_manager: Some(ws.lsp_manager.clone()),
        workspace_root,
        embedding_model: state.embedding_model.clone(),
        session_store_path: Some(store.path.to_string_lossy().to_string()),
        limits: Default::default(),
        read_tracker: Arc::new(Mutex::new(ReadTracker::default())),
        browser: Some(ws.browser.clone()),
        interrupt: Some(steering.interrupt.clone()),
        agent_config: Some(config.clone()),
        base_commit,
        auto_approve_git: true,
        shell_gate: None,
        mcp: Some(mcp),
        index_progress: Some(ws.index_progress.clone()),
        records_cache: state.records_cache.clone(),
    };

    // Register the steering controller so interrupt_session can find it
    {
        let mut map = state.steering.lock().await;
        map.insert(id.clone(), steering.clone());
    }

    let message = "Commit and push all current changes.".to_string();

    let sid = id.clone();
    let chan = event_channel;
    let appr = state.approvals.clone();
    let answ = state.answers.clone();
    let steering_map = state.steering_map();

    tokio::spawn(async move {
        if let Err(e) = session::run_workflow_with_profile(
            &config,
            &mut history,
            message,
            Vec::new(),
            &chan,
            &appr,
            &answ,
            &sid,
            &ctx,
            &store,
            &steering,
            session::PromptProfile::GitSync,
        )
        .await
        {
            store.try_append(&SessionRecord::Error {
                message: e.clone(),
                ts: now_ms(),
            });
            let _ = chan.send(AgentEvent::Error(e));
        }
        // Clean up steering entry on completion
        let mut map = steering_map.lock().await;
        map.remove(&sid);
    });

    Ok(SessionStarted { session_id: id })
}

/// Write a value to the workspace-level `.claudinio.json` config file.
/// Creates the file if it doesn't exist; updates only the specified key.
#[tauri::command]
pub async fn set_workspace_config(
    workspace_root: String,
    plan_save_path: Option<String>,
) -> Result<(), String> {
    let config_path = Path::new(&workspace_root).join(".claudinio.json");
    let mut cfg: Value = if config_path.exists() {
        std::fs::read_to_string(&config_path)
            .ok()
            .and_then(|s| serde_json::from_str(&s).ok())
            .unwrap_or(serde_json::json!({}))
    } else {
        serde_json::json!({})
    };
    if let Some(obj) = cfg.as_object_mut() {
        match plan_save_path {
            Some(ref path) if !path.is_empty() => {
                obj.insert("plan_save_path".into(), Value::String(path.clone()));
            }
            _ => {
                obj.insert("plan_save_path".into(), Value::Null);
            }
        }
    }
    let json = serde_json::to_string_pretty(&cfg)
        .map_err(|e| format!("serialize .claudinio.json: {e}"))?;
    std::fs::write(&config_path, json).map_err(|e| format!("write .claudinio.json: {e}"))?;
    Ok(())
}
