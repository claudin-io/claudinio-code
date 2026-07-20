use crate::agent::persist::{
    self, load_records, now_ms, AttachmentMeta, SessionRecord, SessionStore, SessionSummary,
};
use crate::agent::provider::{save_config, ContentBlock};
use crate::agent::session::{self, AgentEvent, EventTx, SteeringEntry};
use crate::tauri_sinks::ChannelSink;
use crate::agent::transition;
use crate::agent::tools::{ReadTracker, ToolContext};
use crate::commands::tasks as tasks_cmd;
use crate::state::{AppState, SessionHandle};
use serde::{Deserialize, Serialize};
use serde_json::Value;
use std::path::Path;
use std::sync::Arc;
use tauri::ipc::Channel;
use tauri::State;
use tokio::sync::Mutex;

/// Process attachment inputs into content blocks and their lightweight metadata.
/// Used by both `send_message` and `queue_steering`. Thin wrapper over the
/// shared `core::agent::attachments::process_attachments` (moved there so the
/// CLI/TUI builds identical attachment blocks — true parity).
pub fn process_attachments(atts: &[AttachmentInput]) -> Vec<(ContentBlock, AttachmentMeta)> {
    let paths: Vec<String> = atts.iter().map(|a| a.path.clone()).collect();
    crate::agent::attachments::process_attachments(&paths)
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
    mode: Option<String>,
    event_channel: Channel<AgentEvent>,
    state: State<'_, AppState>,
) -> Result<SessionStarted, String> {
    let event_channel: EventTx = Arc::new(ChannelSink(event_channel));
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
    if let Some(ref root) = workspace_root {
        if let Some(ws_cfg) = crate::agent::provider::read_workspace_config(root) {
            crate::agent::provider::merge_workspace_config(&mut config, &ws_cfg);
        }
    }

    // Continue the workspace's active session, or start a fresh one persisted
    // to its own JSONL file.
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

    // Sync the session's mode with what the UI toggle sent: a human-set value
    // that differs from the current one is persisted before the run starts.
    let mode_ctl = state.mode_for(&handle.id, &handle.store_path).await;
    if let Some(m) = mode.as_deref().and_then(session::SessionMode::parse) {
        if mode_ctl.get().0 != m {
            mode_ctl.set(m, session::ModeOrigin::Human);
            let store = SessionStore { path: handle.store_path.clone() };
            store.try_append(&SessionRecord::Mode {
                mode: m.as_str().into(),
                origin: session::ModeOrigin::Human.as_str().into(),
                ts: now_ms(),
            });
        }
    }

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

    let ctx = ToolContext {
        db_path,
        lsp_manager: Some(ws.lsp_manager.clone()),
        workspace_root,
        embedding_model: state.embedding_model.clone(),
        session_store_path: Some(handle.store_path.to_string_lossy().to_string()),
        read_tracker: Arc::new(Mutex::new(ReadTracker::default())),
        interrupt: Some(steering.interrupt.clone()),
        agent_config: Some(config.clone()),
        plan_save_path: config.plan_save_path.clone(),
        base_commit,
        auto_approve_git: false,
        mcp: Some(mcp),
        mode_ctl: Some(mode_ctl.clone()),
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
        let mut tasks = tasks_cmd::load_last_tasks(&handle.store_path).unwrap_or_default();
        // New goals replace previous golden tasks; normal tasks are preserved.
        tasks.retain(|t| !crate::agent::tools::tasks::is_golden(t));
        tasks.extend(crate::agent::tools::tasks::create_golden_tasks(&goals));
        if let Err(e) = tasks_cmd::append_tasks(&handle.store_path, &tasks) {
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
        mode_ctl,
        steering,
        history,
        message,
        attachment_blocks,
    });

    Ok(SessionStarted { session_id })
}

/// The `AppState` maps a session transition needs, cloned so the spawned run
/// loop can link sessions after the Tauri state borrow ends.
fn transition_maps(state: &State<'_, AppState>) -> transition::TransitionMaps {
    transition::TransitionMaps {
        steering: state.steering.clone(),
        modes: state.modes.clone(),
        records_cache: state.records_cache.clone(),
    }
}

// O driver de execução com handoff vive no core (compartilhado com o CLI).
// Aliases mantêm os call sites abaixo inalterados.
use claudinio_core::run::{drive as spawn_run_loop, RunArgs as RunLoopArgs};

/// Start a new conversation in a workspace: the next `send_message` opens a
/// fresh JSONL session there.
#[tauri::command]
pub async fn new_session(
    workspace: String,
    state: State<'_, AppState>,
) -> Result<(), String> {
    let ws = state.workspace(&workspace).await?;
    let mut guard = ws.active_session.lock().await;
    if let Some(h) = guard.as_ref() {
        state.remove_steering(&h.id).await;
        state.modes.lock().await.remove(&h.id);
        // A session abandoned before any real content (only meta/mode records,
        // e.g. created lazily by a mode toggle) is noise on disk — delete it
        // instead of leaving an "(empty session)" orphan in the list.
        let is_empty = persist::load_records(&h.store_path)
            .map(|recs| {
                !recs.iter().any(|r| {
                    matches!(r, SessionRecord::User { .. } | SessionRecord::Turn { .. })
                })
            })
            .unwrap_or(false);
        if is_empty {
            let _ = std::fs::remove_file(&h.store_path);
            persist::invalidate_cache(&h.store_path, &state.records_cache);
        }
    }
    *guard = None;
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
        let Some(next) = persist::handoff_to(&tip_records) else { break };
        if !seen.insert(next.clone()) {
            break; // cycle guard
        }
        let Ok(next_records) = load_one(&next) else { break };
        tip_id = next;
        tip_records = next_records;
    }

    // Backward through the ancestry, prepending each predecessor.
    let mut chain: Vec<Vec<SessionRecord>> = vec![tip_records];
    for _ in 0..MAX_CHAIN_HOPS {
        let earliest = chain.first().map(|v| v.as_slice()).unwrap_or(&[]);
        let Some(info) = persist::linked_from(earliest) else { break };
        if !seen.insert(info.prev_session_id.clone()) {
            break; // cycle guard
        }
        let Ok(prev_records) = load_one(&info.prev_session_id) else { break };
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
pub async fn approve_tool(
    args: ApproveArgs,
    state: State<'_, AppState>,
) -> Result<(), String> {
    let key = format!("{}:{}", args.session_id, args.tool_id);
    let mut map = state.approvals.lock().await;
    if let Some(sender) = map.remove(&key) {
        sender.send(true).map_err(|_| "session already closed".into())
    } else {
        Err("approval request not found or already handled".into())
    }
}

#[tauri::command]
pub async fn reject_tool(
    args: ApproveArgs,
    state: State<'_, AppState>,
) -> Result<(), String> {
    let key = format!("{}:{}", args.session_id, args.tool_id);
    let mut map = state.approvals.lock().await;
    if let Some(sender) = map.remove(&key) {
        sender.send(false).map_err(|_| "session already closed".into())
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
    pub brain_model: Option<String>,
    pub builder_model: Option<String>,
    pub max_rounds: Option<Option<usize>>,
    pub sub_max_rounds: Option<Option<usize>>,
    pub yolo_mode: Option<bool>,
    pub yolo_blacklist: Option<Vec<String>>,
    pub keep_awake: Option<bool>,
    pub max_golden_cycles: Option<Option<usize>>,
    pub max_golden_stalls: Option<Option<usize>>,
    pub max_parallel_agents: Option<Option<usize>>,
    pub plan_save_path: Option<String>,
    pub override_base_url: Option<String>,
    pub override_api_key: Option<String>,
    pub mcp: Option<std::collections::HashMap<String, crate::agent::provider::McpServerEntry>>,
    pub code_intel_enabled: Option<bool>,
    pub preferred_ide: Option<String>,
    pub handoff_context_tokens: Option<Option<u64>>,
    pub auto_commit_plan: Option<bool>,
    pub thinking_effort: Option<String>,
}

#[tauri::command]
pub async fn set_config(
    args: SetConfigArgs,
    state: State<'_, AppState>,
) -> Result<(), String> {
    let mut cfg = state.config.lock().await;
    if let Some(url) = args.base_url {
        cfg.base_url = url;
    }
    if let Some(key) = args.api_key {
        cfg.api_key = key;
    }
    if let Some(brain_model) = args.brain_model {
        cfg.brain_model = brain_model;
    }
    if let Some(builder_model) = args.builder_model {
        cfg.builder_model = builder_model;
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
    if let Some(max_golden_cycles) = args.max_golden_cycles {
        cfg.max_golden_cycles = max_golden_cycles;
    }
    if let Some(max_golden_stalls) = args.max_golden_stalls {
        cfg.max_golden_stalls = max_golden_stalls;
    }
    if let Some(max_parallel_agents) = args.max_parallel_agents {
        cfg.max_parallel_agents = max_parallel_agents.map(|n| n.clamp(1, 8));
    }
    if let Some(plan_save_path) = args.plan_save_path {
        cfg.plan_save_path = if plan_save_path.is_empty() {
            None
        } else {
            Some(plan_save_path)
        };
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
    if let Some(auto_commit_plan) = args.auto_commit_plan {
        cfg.auto_commit_plan = auto_commit_plan;
    }
    if let Some(thinking_effort) = args.thinking_effort {
        if ["low", "medium", "high", "xhigh", "max"].contains(&thinking_effort.as_str()) {
            cfg.thinking_effort = thinking_effort;
        }
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

    Ok(serde_json::json!({
        "baseUrl": cfg.base_url,
        "brainModel": cfg.brain_model,
        "builderModel": cfg.builder_model,
        "hasApiKey": !cfg.api_key.is_empty(),
        "maxContextTokens": session::MAX_CONTEXT_TOKENS,
        "compactThreshold": session::COMPACT_THRESHOLD,
        "maxRounds": cfg.max_rounds,
        "subMaxRounds": cfg.sub_max_rounds,
        "yoloMode": cfg.yolo_mode,
        "yoloBlacklist": cfg.yolo_blacklist,
        "keepAwake": cfg.keep_awake,
        "accountLogin": cfg.account_login,
        "accountTier": cfg.account_tier,
        "autoCommitPlan": cfg.auto_commit_plan,
        "maxGoldenCycles": cfg.max_golden_cycles,
        "maxGoldenStalls": cfg.max_golden_stalls,
        "maxParallelAgents": cfg.max_parallel_agents,
        "planSavePath": cfg.plan_save_path,
        "overrideBaseUrl": cfg.override_base_url,
        "overrideApiKey": cfg.override_api_key,
        "mcp": cfg.mcp,
        "codeIntelEnabled": cfg.code_intel_enabled,
        "preferredIde": cfg.preferred_ide,
        "handoffContextTokens": cfg.handoff_context_tokens,
        "thinkingEffort": cfg.thinking_effort,
        // Connected external providers — never the keys (hasApiKey precedent).
        "providers": cfg.providers.iter().map(|(id, p)| {
            (id.clone(), serde_json::json!({
                "connected": true,
                "baseUrl": p.base_url,
                "label": p.label,
                "protocol": p.protocol,
                "enabledModels": p.enabled_models,
            }))
        }).collect::<serde_json::Map<String, Value>>(),
        "workspaceConfig": workspace_config,
    }))
}

/// Fetch available models from the API. Calls GET {base_url}/v1/models and
/// parses the response, falling back to ["claudinio", "claudius"] on any error.
#[tauri::command]
pub async fn list_models(
    state: State<'_, AppState>,
) -> Result<Vec<String>, String> {
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
    let response = match client
        .get(&url)
        .header("x-api-key", &api_key)
        .send()
        .await
    {
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
            .filter_map(|item| item.get("id").and_then(|id| id.as_str()).map(|s| s.to_string()))
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
    let event_channel: EventTx = Arc::new(ChannelSink(event_channel));
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
        db_path,
        lsp_manager: Some(ws.lsp_manager.clone()),
        workspace_root,
        embedding_model: state.embedding_model.clone(),
        session_store_path: Some(handle.store_path.to_string_lossy().to_string()),
        read_tracker: Arc::new(Mutex::new(ReadTracker::default())),
        interrupt: Some(steering.interrupt.clone()),
        agent_config: Some(config.clone()),
        plan_save_path: config.plan_save_path.clone(),
        base_commit: None,
        auto_approve_git: false,
        // Compaction only summarizes history; it never dispatches tool calls.
        mcp: None,
        mode_ctl: None,
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
    )
    .await?;

    Ok(summary)
}

/// Switch the workspace's active session between Brain and Builder.
/// Always human-originated (the UI toggle); a running workflow picks the new
/// mode up on its next round. Creates the session lazily so the toggle can be
/// flipped before the first message.
#[tauri::command]
pub async fn set_session_mode(
    workspace: String,
    mode: String,
    state: State<'_, AppState>,
) -> Result<SessionStarted, String> {
    let m = session::SessionMode::parse(&mode)
        .ok_or_else(|| format!("invalid mode '{mode}' (expected 'pensador' or 'constructor')"))?;

    let ws = state.workspace(&workspace).await?;
    let workspace_root = ws.root.to_string_lossy().to_string();
    let handle = {
        let mut guard = ws.active_session.lock().await;
        match guard.as_ref() {
            Some(h) => h.clone(),
            None => {
                let id = uuid::Uuid::new_v4().to_string();
                let store = SessionStore::create(&id, Some(&workspace_root))?;
                let h = SessionHandle { id, store_path: store.path };
                *guard = Some(h.clone());
                h
            }
        }
    };

    let mode_ctl = state.mode_for(&handle.id, &handle.store_path).await;
    if mode_ctl.get().0 != m {
        mode_ctl.set(m, session::ModeOrigin::Human);
        let store = SessionStore { path: handle.store_path.clone() };
        store.try_append(&SessionRecord::Mode {
            mode: m.as_str().into(),
            origin: session::ModeOrigin::Human.as_str().into(),
            ts: now_ms(),
        });
    }
    Ok(SessionStarted { session_id: handle.id })
}

/// Approve the Brain's plan and continue in a NEW linked Builder session whose
/// first prompt carries the plan. Replaces the old in-session mode flip: the
/// planning context stays behind, the Builder starts fresh with just the plan
/// and the carried-over task list.
#[tauri::command]
pub async fn continue_with_builder(
    workspace: String,
    event_channel: Channel<AgentEvent>,
    state: State<'_, AppState>,
) -> Result<SessionStarted, String> {
    let event_channel: EventTx = Arc::new(ChannelSink(event_channel));
    let config = {
        let cfg = state.config.lock().await;
        if cfg.api_key.is_empty() {
            return Err("API key not configured. Use set_config first.".into());
        }
        cfg.clone()
    };

    let ws = state.workspace(&workspace).await?;
    let workspace_root = Some(ws.root.to_string_lossy().to_string());

    let mut config = config;
    if let Some(ref root) = workspace_root {
        if let Some(ws_cfg) = crate::agent::provider::read_workspace_config(root) {
            crate::agent::provider::merge_workspace_config(&mut config, &ws_cfg);
        }
    }

    let old_handle = ws
        .active_session
        .lock()
        .await
        .clone()
        .ok_or("no active session to hand off from")?;
    let old_mode = state
        .mode_for(&old_handle.id, &old_handle.store_path)
        .await
        .get()
        .0;
    if old_mode != session::SessionMode::Brain {
        return Err("active session is not in Brain mode".into());
    }

    // Compose the kickoff (plan inline) before linking so the SessionLinked
    // event already carries the real first message.
    let mut spec = session::HandoffSpec {
        reason: session::HandoffReason::ManualBuilder,
        next_mode: session::SessionMode::Builder,
        next_origin: session::ModeOrigin::Human,
        first_message: String::new(),
        golden_cycle: 0,
        golden_stalls: 0,
        golden_last_pending: Vec::new(),
    };
    spec.first_message = transition::resolve_first_message(
        &spec,
        workspace_root.as_deref(),
        config.plan_save_path.as_deref(),
    );

    let maps = transition_maps(&state);
    let new_handle =
        transition::link_session(&maps, &ws, &old_handle, &spec, &event_channel).await?;

    // Fresh run on the new session: build its ToolContext from scratch (no old
    // running context exists — this command fires from an idle Brain session).
    let steering = state.steering_for(&new_handle.id).await;
    steering.clear();
    let mode_ctl = state.mode_for(&new_handle.id, &new_handle.store_path).await;

    let db_path: Option<String> = Some(ws.index_db_path.to_string_lossy().to_string());
    let base_commit: Option<String> = workspace_root
        .as_ref()
        .and_then(|root| crate::agent::tools::git_head(root));
    let mcp = ws.ensure_mcp_connected(&config).await;

    let ctx = ToolContext {
        db_path,
        lsp_manager: Some(ws.lsp_manager.clone()),
        workspace_root,
        embedding_model: state.embedding_model.clone(),
        session_store_path: Some(new_handle.store_path.to_string_lossy().to_string()),
        read_tracker: Arc::new(Mutex::new(ReadTracker::default())),
        interrupt: Some(steering.interrupt.clone()),
        agent_config: Some(config.clone()),
        plan_save_path: config.plan_save_path.clone(),
        base_commit,
        auto_approve_git: false,
        mcp: Some(mcp),
        mode_ctl: Some(mode_ctl.clone()),
        index_progress: Some(ws.index_progress.clone()),
        records_cache: state.records_cache.clone(),
    };

    let store = SessionStore {
        path: new_handle.store_path.clone(),
    };
    let session_id = new_handle.id.clone();
    let message = spec.first_message;
    spawn_run_loop(RunLoopArgs {
        config,
        ws,
        maps,
        approvals: state.approvals.clone(),
        answers: state.answers.clone(),
        chan: event_channel,
        handle: new_handle,
        store,
        ctx,
        mode_ctl,
        steering,
        history: Vec::new(),
        message,
        attachment_blocks: Vec::new(),
    });

    Ok(SessionStarted { session_id })
}

/// The current mode of the workspace's active session (for UI init).
#[tauri::command]
pub async fn get_session_mode(
    workspace: String,
    state: State<'_, AppState>,
) -> Result<Value, String> {
    let ws = state.workspace(&workspace).await?;
    let handle = { ws.active_session.lock().await.clone() };
    match handle {
        Some(h) => {
            let mode_ctl = state.mode_for(&h.id, &h.store_path).await;
            let (mode, origin) = mode_ctl.get();
            Ok(serde_json::json!({ "mode": mode.as_str(), "origin": origin.as_str() }))
        }
        None => Ok(serde_json::json!({ "mode": "constructor", "origin": "human" })),
    }
}

/// Check whether a plan file (.md) exists on disk for this workspace.
/// Used by the frontend to decide whether to show the "Continue with Builder" button
/// when the user manually switches to Brain mode.
#[tauri::command]
pub async fn check_plan_exists(
    workspace: String,
    state: State<'_, AppState>,
) -> Result<bool, String> {
    let ws = state.workspace(&workspace).await?;
    let workspace_root = ws.root.to_string_lossy().to_string();

    // Resolve plan_save_path respecting workspace-level override
    let cfg = state.config.lock().await;
    let global_plan_save_path = cfg.plan_save_path.clone();
    drop(cfg);

    let ws_config = crate::agent::provider::read_workspace_config(&workspace_root);
    let effective_plan_save_path = ws_config
        .as_ref()
        .and_then(|w| w.get("plan_save_path"))
        .and_then(|v| v.as_str())
        .map(|s| s.to_string())
        .or(global_plan_save_path);

    let dir = crate::agent::tools::write_plan::plans_dir(
        &workspace_root,
        effective_plan_save_path.as_deref(),
    );

    if !dir.exists() {
        return Ok(false);
    }

    let has_md = std::fs::read_dir(&dir)
        .map(|entries| {
            entries.flatten().any(|entry| {
                entry
                    .path()
                    .extension()
                    .and_then(|e| e.to_str())
                    == Some("md")
            })
        })
        .unwrap_or(false);

    Ok(has_md)
}

#[derive(Serialize)]
#[serde(rename_all = "camelCase")]
pub struct PlanEntry {
    pub name: String,
    pub path: String,
    pub modified_at: u64,
}

#[tauri::command]
pub async fn list_plans(
    workspace: String,
    state: State<'_, AppState>,
) -> Result<Vec<PlanEntry>, String> {
    let ws = state.workspace(&workspace).await?;
    let workspace_root = ws.root.to_string_lossy().to_string();

    // Resolve plan_save_path respecting workspace-level override
    let cfg = state.config.lock().await;
    let global_plan_save_path = cfg.plan_save_path.clone();
    drop(cfg);

    let ws_config = crate::agent::provider::read_workspace_config(&workspace_root);
    let effective_plan_save_path = ws_config
        .as_ref()
        .and_then(|w| w.get("plan_save_path"))
        .and_then(|v| v.as_str())
        .map(|s| s.to_string())
        .or(global_plan_save_path);

    let dir = crate::agent::tools::write_plan::plans_dir(
        &workspace_root,
        effective_plan_save_path.as_deref(),
    );

    if !dir.exists() {
        return Ok(vec![]);
    }

    let mut plans: Vec<PlanEntry> = std::fs::read_dir(&dir)
        .map(|entries| {
            entries
                .flatten()
                .filter(|entry| {
                    entry
                        .path()
                        .extension()
                        .and_then(|e| e.to_str())
                        == Some("md")
                })
                .filter_map(|entry| {
                    let path = entry.path();
                    let name = path
                        .file_stem()
                        .and_then(|s| s.to_str())
                        .map(|s| s.to_string())?;
                    let modified_at = entry
                        .metadata()
                        .ok()?
                        .modified()
                        .ok()?
                        .duration_since(std::time::UNIX_EPOCH)
                        .unwrap_or_default()
                        .as_secs();
                    Some(PlanEntry {
                        name,
                        path: path.to_string_lossy().to_string(),
                        modified_at,
                    })
                })
                .collect()
        })
        .unwrap_or_default();

    plans.sort_by(|a, b| b.modified_at.cmp(&a.modified_at));

    Ok(plans)
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
    let event_channel: EventTx = Arc::new(ChannelSink(event_channel));
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
    if let Some(ref root) = workspace_root {
        if let Some(ws_cfg) = crate::agent::provider::read_workspace_config(root) {
            crate::agent::provider::merge_workspace_config(&mut config, &ws_cfg);
        }
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
        db_path,
        lsp_manager: Some(ws.lsp_manager.clone()),
        workspace_root,
        embedding_model: state.embedding_model.clone(),
        session_store_path: Some(store.path.to_string_lossy().to_string()),
        read_tracker: Arc::new(Mutex::new(ReadTracker::default())),
        interrupt: Some(steering.interrupt.clone()),
        agent_config: Some(config.clone()),
        plan_save_path: config.plan_save_path.clone(),
        base_commit,
        auto_approve_git: true,
        mcp: Some(mcp),
        mode_ctl: None,
        index_progress: Some(ws.index_progress.clone()),
        records_cache: state.records_cache.clone(),
    };

    // Register the steering controller so interrupt_session can find it
    {
        let mut map = state.steering.lock().await;
        map.insert(id.clone(), steering.clone());
    }

    let message = "Commit and push all current changes.".to_string();

    let mode_ctl = state.mode_for(&id, &store.path).await;

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
            &mode_ctl,
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

    Ok(SessionStarted {
        session_id: id,
    })
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
    std::fs::write(&config_path, json)
        .map_err(|e| format!("write .claudinio.json: {e}"))?;
    Ok(())
}
