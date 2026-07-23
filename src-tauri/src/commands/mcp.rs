use crate::agent::mcp::{McpManager, McpServerStatus};
use crate::agent::provider::McpServerEntry;
use crate::state::AppState;
use tauri::State;

/// Status of every configured MCP server for a workspace (or just the
/// globally configured ones if no workspace is given / open yet).
#[tauri::command]
pub async fn mcp_list_servers(
    workspace: Option<String>,
    state: State<'_, AppState>,
) -> Result<Vec<McpServerStatus>, String> {
    let cfg = state.config.lock().await;
    let mut effective = cfg.clone();
    drop(cfg);

    if let Some(ref root) = workspace {
        if let Some(ws_cfg) = crate::agent::provider::read_workspace_config(root) {
            crate::agent::provider::merge_workspace_config(&mut effective, &ws_cfg);
        }
    }

    // If the workspace is open and already has a connected manager, report
    // its live status instead of re-connecting.
    if let Some(ref root) = workspace {
        if let Ok(ws) = state.workspace(root).await {
            let mgr = ws.mcp.lock().await.clone();
            if let Some(mgr) = mgr {
                return Ok(mgr.statuses());
            }
        }
    }

    // Not connected yet (or no workspace context): report configured-but-not-
    // connected servers so the UI still has something to render.
    Ok(effective
        .mcp
        .iter()
        .map(|(name, entry)| McpServerStatus {
            name: name.clone(),
            connected: false,
            tool_count: 0,
            tool_names: Vec::new(),
            error: if entry.enabled {
                None
            } else {
                Some("disabled".to_string())
            },
        })
        .collect())
}

/// One-off connection test for a server config that hasn't been saved yet
/// (used by a "Test" button in the settings UI before committing).
#[tauri::command]
pub async fn mcp_test_server(
    name: String,
    entry: McpServerEntry,
    workspace: Option<String>,
) -> Result<McpServerStatus, String> {
    let servers = std::collections::HashMap::from([(name, entry)]);
    let manager = McpManager::connect_all(&servers, workspace.as_deref()).await;
    let statuses = manager.statuses();
    manager.shutdown().await;
    statuses
        .into_iter()
        .next()
        .ok_or_else(|| "no status returned for test connection".to_string())
}

/// Force a reconnect of all MCP servers configured for a workspace, e.g.
/// after editing server config or to recover from a crashed stdio server.
#[tauri::command]
pub async fn mcp_reconnect(
    workspace: String,
    state: State<'_, AppState>,
) -> Result<Vec<McpServerStatus>, String> {
    let ws = state.workspace(&workspace).await?;
    let stale = ws.mcp.lock().await.take();
    if let Some(mgr) = stale {
        mgr.shutdown().await;
    }
    *ws.mcp_fingerprint.lock().await = None;

    let cfg = state.config.lock().await;
    let mut effective = cfg.clone();
    drop(cfg);
    if let Some(ws_cfg) = crate::agent::provider::read_workspace_config(&workspace) {
        crate::agent::provider::merge_workspace_config(&mut effective, &ws_cfg);
    }

    let manager = ws.ensure_mcp_connected(&effective).await;
    Ok(manager.statuses())
}
