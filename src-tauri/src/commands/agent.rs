use crate::agent::persist::{
    self, load_records, now_ms, SessionRecord, SessionStore, SessionSummary,
};
use crate::agent::provider::save_config;
use crate::agent::session::{self, AgentEvent};
use crate::agent::tools::ToolContext;
use crate::state::{AppState, SessionHandle};
use serde::{Deserialize, Serialize};
use serde_json::Value;
use tauri::ipc::Channel;
use tauri::State;

#[derive(Serialize)]
#[serde(rename_all = "camelCase")]
pub struct SessionStarted {
    pub session_id: String,
}

async fn workspace_string(state: &State<'_, AppState>) -> Option<String> {
    let ws = state.workspace_root.lock().await;
    ws.as_ref().map(|p| p.to_string_lossy().to_string())
}

#[tauri::command]
pub async fn send_message(
    message: String,
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
    };

    // Reset steering for the new run, then drain any residual from a race.
    state.steering.clear();
    let residual = state.steering.drain();
    let message = if residual.is_empty() {
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

    let cfg = config;
    let sid = handle.id.clone();
    let chan = event_channel;
    let appr = state.approvals.clone();
    let answ = state.answers.clone();
    let steering = state.steering.clone();

    tokio::spawn(async move {
        if let Err(e) = session::run_workflow(
            &cfg, &mut history, message, &chan, &appr, &answ, &sid, &ctx, &store, &steering,
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
