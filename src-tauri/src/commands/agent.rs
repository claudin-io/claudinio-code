use crate::agent::persist::{
    self, load_records, now_ms, SessionRecord, SessionStore, SessionSummary,
};
use crate::agent::provider::{save_config, ContentBlock};
use crate::agent::session::{self, AgentEvent};
use crate::agent::tools::ToolContext;
use crate::state::{AppState, SessionHandle};
use serde::{Deserialize, Serialize};
use serde_json::Value;
use std::path::Path;
use tauri::ipc::Channel;
use tauri::State;
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

async fn workspace_string(state: &State<'_, AppState>) -> Option<String> {
    let ws = state.workspace_root.lock().await;
    ws.as_ref().map(|p| p.to_string_lossy().to_string())
}

#[tauri::command]
pub async fn send_message(
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

    let workspace_root = workspace_string(&state).await;

    // Continue the active session, or start a fresh one persisted to its own
    // JSONL file.
    let handle = {
        let mut guard = state.active_session.lock().await;
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

    let db_path: Option<String> = workspace_root
        .as_ref()
        .map(|p| format!("{p}/.claudinio_index.db"));

    let ctx = ToolContext {
        db_path,
        lsp_manager: Some(state.lsp_manager.clone()),
        workspace_root,
        embedding_model: state.embedding_model.clone(),
        session_store_path: Some(handle.store_path.to_string_lossy().to_string()),
    };

    // Reset steering for the new run, then drain any residual from a race.
    state.steering.clear();
    let residual = state.steering.drain();
    let mut message = if residual.is_empty() {
        message
    } else {
        let mut prefix = String::new();
        for r in &residual {
            prefix.push_str(r);
            prefix.push('\n');
        }
        prefix.push_str(&message);
        prefix
    };

    // Process attachments into content blocks to prepend to the user message
    let mut attachment_blocks: Vec<ContentBlock> = Vec::new();
    if let Some(atts) = attachments {
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
            let is_image = matches!(ext.as_str(), "png" | "jpg" | "jpeg" | "gif" | "webp" | "bmp");
            let is_text = matches!(
                ext.as_str(),
                "txt" | "md" | "csv" | "json" | "yaml" | "yml" | "toml"
                    | "rs" | "ts" | "tsx" | "js" | "jsx" | "py" | "swift"
                    | "go" | "rb" | "html" | "htm" | "css" | "sh" | "bash"
                    | "sql" | "xml" | "toml" | "log"
            );

            if is_image {
                // Read image file and create an Image content block
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
                // Compress large images to reduce token consumption
                let (compressed_bytes, final_media_type) = compress_image(&bytes, &media_type, &ext);
                let data = base64::engine::general_purpose::STANDARD.encode(&compressed_bytes);
                attachment_blocks.push(ContentBlock::image(&final_media_type, &data));
            } else if is_text {
                // Read text file contents
                let text = match std::fs::read_to_string(file_path) {
                    Ok(t) => t,
                    Err(_) => continue,
                };
                let file_name = file_path
                    .file_name()
                    .and_then(|n| n.to_str())
                    .unwrap_or("file");
                let block_text = format!("[Arquivo anexado: `{file_name}`]\n```\n{text}\n```");
                attachment_blocks.push(ContentBlock::text(block_text));
            } else {
                // For PDFs, audio, video, and other binary files: read name only
                let file_name = file_path
                    .file_name()
                    .and_then(|n| n.to_str())
                    .unwrap_or("file");
                let file_size = file_path.metadata().map(|m| m.len()).unwrap_or(0);
                let size_str = if file_size > 1024 * 1024 {
                    format!("{:.1} MB", file_size as f64 / (1024.0 * 1024.0))
                } else if file_size > 1024 {
                    format!("{:.1} KB", file_size as f64 / 1024.0)
                } else {
                    format!("{file_size} B")
                };
                let block_text =
                    format!("[Arquivo anexado: `{file_name}` ({size_str}) — tipo: {ext}]");
                attachment_blocks.push(ContentBlock::text(block_text));
            }
        }
    }

    let cfg = config;
    let sid = handle.id.clone();
    let chan = event_channel;
    let appr = state.approvals.clone();
    let answ = state.answers.clone();
    let steering = state.steering.clone();

    tokio::spawn(async move {
        if let Err(e) = session::run_workflow(
            &cfg, &mut history, message, attachment_blocks, &chan, &appr, &answ, &sid, &ctx, &store, &steering,
        )
        .await
        {
            store.try_append(&SessionRecord::Error {
                message: e.clone(),
                ts: now_ms(),
            });
            let _ = chan.send(AgentEvent::Error(e));
        }
    });

    Ok(SessionStarted {
        session_id: handle.id,
    })
}

/// Start a new conversation: the next `send_message` opens a fresh JSONL session.
#[tauri::command]
pub async fn new_session(state: State<'_, AppState>) -> Result<(), String> {
    state.steering.clear();
    let mut guard = state.active_session.lock().await;
    *guard = None;
    Ok(())
}

/// List saved sessions for the current workspace, newest first.
#[tauri::command]
pub async fn list_sessions(state: State<'_, AppState>) -> Result<Vec<SessionSummary>, String> {
    let workspace_root = workspace_string(&state).await;
    persist::list_sessions(workspace_root.as_deref())
}

/// Reopen a saved session: makes it the active conversation and returns its full
/// record stream so the frontend can replay the transcript.
#[tauri::command]
pub async fn load_session(
    session_id: String,
    state: State<'_, AppState>,
) -> Result<Vec<SessionRecord>, String> {
    let workspace_root = workspace_string(&state).await;
    let dir = persist::sessions_dir(workspace_root.as_deref())?;
    let path = dir.join(format!("{session_id}.jsonl"));
    if !path.exists() {
        return Err(format!("session '{session_id}' not found"));
    }
    let records = load_records(&path)?;
    state.steering.clear();
    {
        let mut guard = state.active_session.lock().await;
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
    pub model: Option<String>,
    pub max_rounds: Option<Option<usize>>,
    pub sub_max_rounds: Option<Option<usize>>,
    pub yolo_mode: Option<bool>,
    pub yolo_blacklist: Option<Vec<String>>,
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
    save_config(&cfg);
    Ok(())
}

#[tauri::command]
pub async fn get_config(
    state: State<'_, AppState>,
) -> Result<Value, String> {
    let cfg = state.config.lock().await;
    Ok(serde_json::json!({
        "baseUrl": cfg.base_url,
        "model": cfg.model,
        "hasApiKey": !cfg.api_key.is_empty(),
        "maxContextTokens": session::MAX_CONTEXT_TOKENS,
        "compactThreshold": session::COMPACT_THRESHOLD,
        "maxRounds": cfg.max_rounds,
        "subMaxRounds": cfg.sub_max_rounds,
        "yoloMode": cfg.yolo_mode,
        "yoloBlacklist": cfg.yolo_blacklist,
    }))
}

/// Push a steering message into the queue for the active session.
#[tauri::command]
pub async fn queue_steering(
    session_id: String,
    text: String,
    state: State<'_, AppState>,
) -> Result<(), String> {
    {
        let guard = state.active_session.lock().await;
        match guard.as_ref() {
            Some(h) if h.id == session_id => {}
            _ => return Err("no active session or session mismatch".into()),
        }
    }
    state.steering.push(text);
    Ok(())
}

/// Manually force context compaction for the active session.
/// Returns the generated summary.
#[tauri::command]
pub async fn compact_session(
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

    let workspace_root = workspace_string(&state).await;
    let handle = {
        let guard = state.active_session.lock().await;
        match guard.as_ref() {
            Some(h) if h.id == session_id => h.clone(),
            _ => return Err("no active session or session mismatch".into()),
        }
    };

    let store = SessionStore {
        path: handle.store_path.clone(),
    };

    let db_path: Option<String> = workspace_root
        .as_ref()
        .map(|p| format!("{p}/.claudinio_index.db"));

    let ctx = ToolContext {
        db_path,
        lsp_manager: Some(state.lsp_manager.clone()),
        workspace_root,
        embedding_model: state.embedding_model.clone(),
        session_store_path: Some(handle.store_path.to_string_lossy().to_string()),
    };

    let summary = session::compact_history(
        &config,
        &store,
        &ctx,
        &event_channel,
        &state.approvals,
        &state.answers,
        &handle.id,
        &state.steering,
    )
    .await?;

    Ok(summary)
}

/// Set the interrupt flag on the active session's steering controller.
#[tauri::command]
pub async fn interrupt_session(
    session_id: String,
    state: State<'_, AppState>,
) -> Result<(), String> {
    {
        let guard = state.active_session.lock().await;
        match guard.as_ref() {
            Some(h) if h.id == session_id => {}
            _ => return Err("no active session or session mismatch".into()),
        }
    }
    state
        .steering
        .interrupt
        .store(true, std::sync::atomic::Ordering::SeqCst);
    Ok(())
}
