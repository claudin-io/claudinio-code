//! MCP (Model Context Protocol) client support: connects to configured
//! external servers (stdio or Streamable HTTP), discovers their tools, and
//! exposes them to the agent loop namespaced as `mcp__<server>__<tool>`.

use crate::agent::provider::{McpServerEntry, McpTransportConfig};
use crate::agent::tools::ToolDef;
use crate::procutil;
use rmcp::model::{CallToolRequestParams, CallToolResult, ContentBlock};
use rmcp::service::RunningService;
use rmcp::transport::streamable_http_client::StreamableHttpClientTransportConfig;
use rmcp::transport::{ConfigureCommandExt, StreamableHttpClientTransport, TokioChildProcess};
use rmcp::{RoleClient, ServiceExt};
use std::collections::HashMap;
use std::sync::{Arc, RwLock};
use std::time::Duration;
use tokio::sync::Mutex;

const CONNECT_TIMEOUT: Duration = Duration::from_secs(15);
const CALL_TIMEOUT: Duration = Duration::from_secs(120);

struct McpConnection {
    service: RunningService<RoleClient, ()>,
    /// namespaced tool name (e.g. `mcp__github__search`) -> original tool
    /// name as understood by this server (e.g. `search`).
    tool_names: HashMap<String, String>,
}

/// Connection outcome for one configured server, kept for the settings UI.
#[derive(Debug, Clone, serde::Serialize)]
#[serde(rename_all = "camelCase")]
pub struct McpServerStatus {
    pub name: String,
    pub connected: bool,
    pub tool_count: usize,
    pub tool_names: Vec<String>,
    pub error: Option<String>,
}

pub struct McpManager {
    connections: Mutex<Vec<McpConnection>>,
    /// Snapshot of all connected servers' tool defs, kept in sync with
    /// `connections` so sync callers (get_defs/api_tools) never need to
    /// touch the async mutex.
    defs_snapshot: RwLock<Vec<ToolDef>>,
    status_snapshot: RwLock<Vec<McpServerStatus>>,
}

impl McpManager {
    fn empty() -> Arc<Self> {
        Arc::new(Self {
            connections: Mutex::new(Vec::new()),
            defs_snapshot: RwLock::new(Vec::new()),
            status_snapshot: RwLock::new(Vec::new()),
        })
    }

    /// Connect to every enabled server in `servers` (keyed by server name). A
    /// failure on one server never aborts the batch — it's recorded in the
    /// status snapshot so the run proceeds with whatever did connect.
    pub async fn connect_all(
        servers: &HashMap<String, McpServerEntry>,
        workspace_root: Option<&str>,
    ) -> Arc<Self> {
        let manager = Self::empty();
        let mut connections = Vec::new();
        let mut statuses = Vec::new();
        let mut all_defs = Vec::new();

        for (name, entry) in servers {
            if !entry.enabled {
                continue;
            }
            match tokio::time::timeout(CONNECT_TIMEOUT, connect_one(name, entry, workspace_root))
                .await
            {
                Ok(Ok((conn, defs))) => {
                    statuses.push(McpServerStatus {
                        name: name.clone(),
                        connected: true,
                        tool_count: defs.len(),
                        tool_names: defs.iter().map(|d| d.name.clone()).collect(),
                        error: None,
                    });
                    all_defs.extend(defs);
                    connections.push(conn);
                }
                Ok(Err(e)) => {
                    statuses.push(McpServerStatus {
                        name: name.clone(),
                        connected: false,
                        tool_count: 0,
                        tool_names: Vec::new(),
                        error: Some(e),
                    });
                }
                Err(_) => {
                    statuses.push(McpServerStatus {
                        name: name.clone(),
                        connected: false,
                        tool_count: 0,
                        tool_names: Vec::new(),
                        error: Some(format!(
                            "connection timed out after {}s",
                            CONNECT_TIMEOUT.as_secs()
                        )),
                    });
                }
            }
        }

        *manager.connections.lock().await = connections;
        *manager.defs_snapshot.write().unwrap() = all_defs;
        *manager.status_snapshot.write().unwrap() = statuses;
        manager
    }

    /// Sync snapshot of every discovered MCP tool, ready to merge into
    /// `tools::get_defs()` / `api_tools()` output.
    pub fn cached_defs(&self) -> Vec<ToolDef> {
        self.defs_snapshot.read().unwrap().clone()
    }

    pub fn statuses(&self) -> Vec<McpServerStatus> {
        self.status_snapshot.read().unwrap().clone()
    }

    /// Dispatch a namespaced tool call (`mcp__server__tool`) to the owning
    /// connection and convert the result to plain text.
    pub async fn call(
        &self,
        namespaced_name: &str,
        args: serde_json::Value,
    ) -> Result<String, String> {
        let mut connections = self.connections.lock().await;
        let conn = connections
            .iter_mut()
            .find(|c| c.tool_names.contains_key(namespaced_name))
            .ok_or_else(|| format!("no MCP server owns tool '{namespaced_name}'"))?;
        let original_name = conn
            .tool_names
            .get(namespaced_name)
            .cloned()
            .unwrap_or_default();

        let mut params = CallToolRequestParams::new(original_name);
        params.arguments = args.as_object().cloned();

        let result = tokio::time::timeout(CALL_TIMEOUT, conn.service.call_tool(params))
            .await
            .map_err(|_| format!("MCP tool call to '{namespaced_name}' timed out"))?
            .map_err(|e| format!("MCP tool call failed: {e}"))?;

        result_to_text(result)
    }

    /// Cancel every underlying connection (kills stdio child processes).
    pub async fn shutdown(self: &Arc<Self>) {
        let connections = std::mem::take(&mut *self.connections.lock().await);
        for conn in connections {
            let _ = conn.service.cancel().await;
        }
    }
}

/// Sanitize `mcp__<server>__<tool>` into a valid Anthropic-style tool name:
/// alphanumerics, underscore and dash only, capped at 64 chars.
fn sanitize_tool_name(server: &str, tool: &str) -> String {
    let raw = format!("mcp__{server}__{tool}");
    let sanitized: String = raw
        .chars()
        .map(|c| {
            if c.is_ascii_alphanumeric() || c == '_' || c == '-' {
                c
            } else {
                '_'
            }
        })
        .collect();
    if sanitized.len() > 64 {
        sanitized[..64].to_string()
    } else {
        sanitized
    }
}

async fn connect_one(
    name: &str,
    entry: &McpServerEntry,
    workspace_root: Option<&str>,
) -> Result<(McpConnection, Vec<ToolDef>), String> {
    let service: RunningService<RoleClient, ()> = match &entry.transport {
        McpTransportConfig::Stdio { command, args, env } => {
            let workdir = workspace_root.map(|s| s.to_string());
            let cmd = tokio::process::Command::new(command).configure(|c| {
                c.args(args);
                for (k, v) in env {
                    c.env(k, v);
                }
                if let Some(dir) = &workdir {
                    c.current_dir(dir);
                }
                procutil::no_window_tokio(c);
            });
            let transport = TokioChildProcess::new(cmd)
                .map_err(|e| format!("failed to spawn '{command}': {e}"))?;
            ().serve(transport)
                .await
                .map_err(|e| format!("MCP initialize failed for '{name}': {e}"))?
        }
        McpTransportConfig::Remote { url, headers } => {
            let _net_guard = crate::net_activity::NetGuard::begin(
                crate::net_activity::NetSource::Mcp,
                format!("{name} ({url})"),
            );
            let mut header_map = HashMap::new();
            for (k, v) in headers {
                if let (Ok(hname), Ok(value)) = (
                    http::HeaderName::from_bytes(k.as_bytes()),
                    http::HeaderValue::from_str(v),
                ) {
                    header_map.insert(hname, value);
                }
            }
            let config = StreamableHttpClientTransportConfig::with_uri(url.clone())
                .custom_headers(header_map);
            let transport = StreamableHttpClientTransport::from_config(config);
            ().serve(transport)
                .await
                .map_err(|e| format!("MCP initialize failed for '{name}': {e}"))?
        }
    };

    let tools = service
        .list_all_tools()
        .await
        .map_err(|e| format!("tools/list failed for '{name}': {e}"))?;

    let mut tool_names = HashMap::new();
    let mut defs = Vec::with_capacity(tools.len());
    for tool in &tools {
        let namespaced = sanitize_tool_name(name, &tool.name);
        tool_names.insert(namespaced.clone(), tool.name.to_string());
        defs.push(ToolDef {
            name: namespaced,
            description: tool
                .description
                .clone()
                .map(|d| d.to_string())
                .unwrap_or_default(),
            input_schema: serde_json::Value::Object((*tool.input_schema).clone()),
        });
    }

    Ok((
        McpConnection {
            service,
            tool_names,
        },
        defs,
    ))
}

fn result_to_text(result: CallToolResult) -> Result<String, String> {
    let mut parts = Vec::new();
    for block in &result.content {
        match block {
            ContentBlock::Text(t) => parts.push(t.text.clone()),
            ContentBlock::Image(img) => {
                parts.push(format!("[image: {}, base64 data omitted]", img.mime_type));
            }
            ContentBlock::Audio(audio) => {
                parts.push(format!("[audio: {}, base64 data omitted]", audio.mime_type));
            }
            ContentBlock::Resource(res) => match &res.resource {
                rmcp::model::ResourceContents::TextResourceContents { text, uri, .. } => {
                    parts.push(format!("[resource {uri}]\n{text}"));
                }
                rmcp::model::ResourceContents::BlobResourceContents { uri, .. } => {
                    parts.push(format!("[resource {uri}: binary data omitted]"));
                }
                _ => parts.push("[resource: unsupported content]".to_string()),
            },
            ContentBlock::ResourceLink(link) => {
                parts.push(format!("[resource link: {}]", link.uri));
            }
            _ => parts.push("[unsupported content block]".to_string()),
        }
    }
    let text = parts.join("\n\n");
    if result.is_error == Some(true) {
        Err(if text.is_empty() {
            "tool call failed".to_string()
        } else {
            text
        })
    } else {
        Ok(text)
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn test_sanitize_tool_name_replaces_invalid_chars_and_caps_length() {
        let name = sanitize_tool_name("my server!", "do/thing");
        assert!(
            name.chars()
                .all(|c| c.is_ascii_alphanumeric() || c == '_' || c == '-')
        );
        assert!(name.len() <= 64);
        assert!(name.starts_with("mcp__"));
    }

    #[test]
    fn test_result_to_text_joins_text_blocks_and_respects_is_error() {
        let ok = CallToolResult::success(vec![ContentBlock::Text(rmcp::model::TextContent::new(
            "hello",
        ))]);
        assert_eq!(result_to_text(ok).unwrap(), "hello");

        let err = CallToolResult::error(vec![ContentBlock::Text(rmcp::model::TextContent::new(
            "boom",
        ))]);
        assert_eq!(result_to_text(err).unwrap_err(), "boom");
    }
}
