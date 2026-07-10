use crate::agent::persist::{
    self, load_records, now_ms, AttachmentMeta, SessionRecord, SessionStore, SessionSummary,
};
use crate::agent::provider::{save_config, ContentBlock};
use crate::agent::session::{self, AgentEvent, SteeringEntry};
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
use base64::Engine;
use std::io::Cursor;
use image::GenericImageView;

/// Compress an image to reduce its token footprint before base64-encoding.
///
/// Rules (in order):
/// 1. If the raw bytes are under 200 KB, return as-is.
/// 2. If the longest edge exceeds 2048 px, resize down.
/// 3. Re-encode: JPEG at quality 80; convert large PNG/BMP to JPEG.
/// 4. On any error, fall back to the original bytes silently.
fn compress_image(bytes: &[u8], media_type: &str, ext: &str) -> (Vec<u8>, String) {
    if bytes.len() < 200 * 1024 {
        return (bytes.to_vec(), media_type.to_string());
    }
    let img = match image::load_from_memory(bytes) {
        Ok(img) => img,
        Err(_) => return (bytes.to_vec(), media_type.to_string()),
    };
    let (w, h) = img.dimensions();
    let max_dim = 2048u32;
    let (new_w, new_h) = if w > max_dim || h > max_dim {
        let ratio = (w as f64).max(h as f64) / max_dim as f64;
        ((w as f64 / ratio).round() as u32, (h as f64 / ratio).round() as u32)
    } else {
        (w, h)
    };
    let resized = if (new_w, new_h) != (w, h) {
        img.resize_exact(new_w, new_h, image::imageops::FilterType::Lanczos3)
    } else {
        img
    };
    let encode_as_jpeg = ext == "png" || ext == "bmp";
    let out_type = if encode_as_jpeg { "image/jpeg" } else { media_type };
    let mut out = Vec::new();
    let result = if out_type == "image/jpeg" {
        let mut enc = image::codecs::jpeg::JpegEncoder::new_with_quality(&mut out, 80);
        enc.encode(&resized.to_rgb8(), resized.width(), resized.height(), image::ColorType::Rgb8.into())
    } else if out_type == "image/webp" {
        let enc = image::codecs::webp::WebPEncoder::new_lossless(&mut out);
        enc.encode(&resized.to_rgba8(), resized.width(), resized.height(), image::ColorType::Rgba8.into())
    } else {
        resized.write_to(&mut Cursor::new(&mut out), image::ImageFormat::Png)
    };
    match result {
        Ok(_) if out.len() < bytes.len() => (out, out_type.to_string()),
        _ => (bytes.to_vec(), media_type.to_string()),
    }
}

/// Process attachment inputs into content blocks and their lightweight metadata.
/// Used by both `send_message` and `queue_steering`.
///
/// Returns a vector of (ContentBlock, AttachmentMeta) pairs so the caller can
/// forward the content blocks to the workflow and persist the metadata for
/// UI display (timeline pills).
pub fn process_attachments(
    atts: &[AttachmentInput],
) -> Vec<(ContentBlock, AttachmentMeta)> {
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

        let is_image = matches!(ext.as_str(), "png" | "jpg" | "jpeg" | "gif" | "webp" | "bmp");
        let is_text = matches!(
            ext.as_str(),
            "txt" | "md" | "csv" | "json" | "yaml" | "yml" | "toml"
                | "rs" | "ts" | "tsx" | "js" | "jsx" | "py" | "swift"
                | "go" | "rb" | "html" | "htm" | "css" | "sh" | "bash"
                | "sql" | "xml" | "log"
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
            let (compressed_bytes, final_media_type) = compress_image(&bytes, &media_type, &ext);
            let data = base64::engine::general_purpose::STANDARD.encode(&compressed_bytes);
            results.push((ContentBlock::image(&final_media_type, &data), meta));
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
            let block_text =
                format!("[Arquivo anexado: `{file_name}` ({size_str}) — tipo: {ext}]");
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
    mode: Option<String>,
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
    let mut history = load_records(&handle.store_path)
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

    let db_path: Option<String> = workspace_root
        .as_ref()
        .map(|p| format!("{p}/.claudinio_index.db"));

    // Capture the git HEAD at run start so finalize_plan can compute the
    // changed files / commits since this plan's work began. Best-effort:
    // None when the workspace is not a git repo (or git is unavailable).
    let base_commit: Option<String> = workspace_root
        .as_ref()
        .and_then(|root| crate::agent::tools::git_head(root));

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

    let cfg = config;
    let sid = handle.id.clone();
    let chan = event_channel;
    let appr = state.approvals.clone();
    let answ = state.answers.clone();
    let steering_map = state.steering_map();

    tokio::spawn(async move {
        if let Err(e) = session::run_workflow(
            &cfg, &mut history, message, attachment_blocks, &chan, &appr, &answ, &sid, &ctx, &store, &steering, &mode_ctl,
        )
        .await
        {
            store.try_append(&SessionRecord::Error {
                message: e.clone(),
                ts: now_ms(),
            });
            let _ = chan.send(AgentEvent::Error(e));
        }
        // Run finished (success, error or panic-free return): drop the
        // steering entry so interrupt/steer report "session not running".
        let mut map = steering_map.lock().await;
        map.remove(&sid);
    });

    Ok(SessionStarted {
        session_id: handle.id,
    })
}

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
#[tauri::command]
pub async fn load_session(
    workspace: String,
    session_id: String,
    state: State<'_, AppState>,
) -> Result<Vec<SessionRecord>, String> {
    let ws = state.workspace(&workspace).await?;
    let root = ws.root.to_string_lossy().to_string();
    let dir = persist::sessions_dir(Some(&root))?;
    let path = dir.join(format!("{session_id}.jsonl"));
    if !path.exists() {
        return Err(format!("session '{session_id}' not found"));
    }
    let records = load_records(&path)?;
    {
        let mut guard = ws.active_session.lock().await;
        *guard = Some(SessionHandle {
            id: session_id,
            store_path: path,
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
    pub max_golden_cycles: Option<Option<usize>>,
    pub max_golden_stalls: Option<Option<usize>>,
    pub plan_save_path: Option<String>,
    pub override_base_url: Option<String>,
    pub override_api_key: Option<String>,
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
    if let Some(max_golden_cycles) = args.max_golden_cycles {
        cfg.max_golden_cycles = max_golden_cycles;
    }
    if let Some(max_golden_stalls) = args.max_golden_stalls {
        cfg.max_golden_stalls = max_golden_stalls;
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
        "accountLogin": cfg.account_login,
        "accountTier": cfg.account_tier,
        "maxGoldenCycles": cfg.max_golden_cycles,
        "maxGoldenStalls": cfg.max_golden_stalls,
        "planSavePath": cfg.plan_save_path,
        "overrideBaseUrl": cfg.override_base_url,
        "overrideApiKey": cfg.override_api_key,
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
    let client = reqwest::Client::new();
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

    let db_path: Option<String> = workspace_root
        .as_ref()
        .map(|p| format!("{p}/.claudinio_index.db"));

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
