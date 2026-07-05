use crate::agent::provider::save_config;
use crate::agent::session::{self, AgentEvent};
use crate::agent::tools::ToolContext;
use crate::state::AppState;
use serde::{Deserialize, Serialize};
use serde_json::Value;
use tauri::ipc::Channel;
use tauri::State;

#[derive(Serialize)]
#[serde(rename_all = "camelCase")]
pub struct SessionStarted {
    pub session_id: String,
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

    let session_id = uuid::Uuid::new_v4().to_string();
    let approvals = state.approvals.clone();
    let lsp_manager = state.lsp_manager.clone();

    let db_path: Option<String> = {
        let ws = state.workspace_root.lock().await;
        ws.as_ref().map(|p| p.join(".claudinio_index.db").to_string_lossy().to_string())
    };

    let workspace_root: Option<String> = {
        let ws = state.workspace_root.lock().await;
        ws.as_ref().map(|p| p.to_string_lossy().to_string())
    };

    let ctx = ToolContext {
        db_path,
        lsp_manager: Some(lsp_manager),
        workspace_root,
    };

    let mut history = Vec::new();

    let cfg = config.clone();
    let sid = session_id.clone();
    let chan = event_channel;
    let appr = approvals.clone();

    tokio::spawn(async move {
        if let Err(e) = session::run_session(&cfg, &mut history, message, &chan, &appr, &sid, &ctx).await {
            let _ = chan.send(AgentEvent::Error(e));
        }
    });

    Ok(SessionStarted { session_id })
}

#[derive(Deserialize)]
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
