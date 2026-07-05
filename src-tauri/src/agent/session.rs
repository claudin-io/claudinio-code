use crate::agent::permissions;
use crate::agent::provider::{self, AgentConfig, ContentBlock, Message, ToolDescription};
use crate::agent::tools::{self, ToolContext, ToolOutput};
use serde::Serialize;
use serde_json::Value;
use std::collections::HashMap;
use std::sync::Arc;
use tauri::ipc::Channel;
use tokio::sync::{Mutex, oneshot};

#[derive(Debug, Clone, Serialize)]
#[serde(tag = "event", content = "data")]
pub enum AgentEvent {
    Thinking(String),
    ToolCall {
        session_id: String,
        tool_id: String,
        tool_name: String,
        args: Value,
        permission: String,
        edit_proposal: Option<EditProposalData>,
    },
    ToolResult {
        tool_id: String,
        tool_name: String,
        output: String,
        error: Option<String>,
    },
    Done {
        stop_reason: String,
        text_output: String,
        input_tokens: u32,
        output_tokens: u32,
    },
    Error(String),
}

#[derive(Debug, Clone, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct EditProposalData {
    pub path: String,
    pub old_string: String,
    pub new_string: String,
    pub unified_diff: String,
}

pub type ApprovalMap = Arc<Mutex<HashMap<String, oneshot::Sender<bool>>>>;

#[allow(unused)]
pub async fn run_session(
    config: &AgentConfig,
    history: &mut Vec<Message>,
    user_message: String,
    event_tx: &Channel<AgentEvent>,
    approvals: &ApprovalMap,
    session_id: &str,
    ctx: &ToolContext,
) -> Result<(), String> {
    history.push(Message {
        role: "user".into(),
        content: vec![ContentBlock::text(&user_message)],
    });

    let tool_defs = tools::get_defs();
    let api_tools: Vec<ToolDescription> = tool_defs
        .iter()
        .map(|t| ToolDescription {
            name: t.name.clone(),
            description: t.description.clone(),
            input_schema: t.input_schema.clone(),
        })
        .collect();

    loop {
        let mut assistant_text = String::new();

        let stream_output = provider::stream_message(
            config,
            history,
            &api_tools,
            event_tx,
            session_id,
            &mut assistant_text,
        )
        .await?;

        let text_output = assistant_text;
        let tool_uses = stream_output.tool_uses;
        let stop_reason = stream_output.stop_reason.unwrap_or_default();

        if !text_output.is_empty() {
            history.push(Message {
                role: "assistant".into(),
                content: vec![ContentBlock::text(&text_output)],
            });
        }

        if tool_uses.is_empty() {
            let _ = event_tx.send(AgentEvent::Done {
                stop_reason: stop_reason.clone(),
                text_output,
                input_tokens: stream_output.usage.as_ref().map(|u| u.input_tokens).unwrap_or(0),
                output_tokens: stream_output.usage.as_ref().map(|u| u.output_tokens).unwrap_or(0),
            });
            break;
        }

        let mut tool_result_blocks: Vec<ContentBlock> = Vec::new();
        let mut tool_assistant_blocks: Vec<ContentBlock> = Vec::new();

        if !text_output.is_empty() {
            tool_assistant_blocks.push(ContentBlock::text(&text_output));
        }

        for tool_use in &tool_uses {
            let tool_use_id = tool_use
                .get("id")
                .and_then(|v| v.as_str())
                .unwrap_or("")
                .to_string();
            let tool_name = tool_use
                .get("name")
                .and_then(|v| v.as_str())
                .unwrap_or("")
                .to_string();
            let tool_input = tool_use
                .get("input")
                .cloned()
                .unwrap_or(Value::Null);

            tool_assistant_blocks.push(ContentBlock::tool_use(
                &tool_use_id,
                &tool_name,
                tool_input.clone(),
            ));

            let perm = permissions::tool_permission(&tool_name);

            match perm {
                permissions::PermissionLevel::Auto => {
                    let _ = event_tx.send(AgentEvent::ToolCall {
                        session_id: session_id.to_string(),
                        tool_id: tool_use_id.clone(),
                        tool_name: tool_name.clone(),
                        args: tool_input.clone(),
                        permission: "auto".into(),
                        edit_proposal: None,
                    });

                    let result = tools::execute(&tool_name, tool_input.clone(), ctx).await;
                    match result {
                        Ok(ToolOutput::Text { content }) => {
                            let truncated = if content.len() > 2000 {
                                format!("{}...(truncated, {} chars total)", &content[..2000], content.len())
                            } else {
                                content.clone()
                            };
                            tool_result_blocks.push(ContentBlock::tool_result(&tool_use_id, &content));
                            let _ = event_tx.send(AgentEvent::ToolResult {
                                tool_id: tool_use_id.clone(),
                                tool_name: tool_name.clone(),
                                output: truncated,
                                error: None,
                            });
                        }
                        Ok(ToolOutput::EditProposal { .. }) => {
                            let err_msg = format!(
                                "edit_file for {tool_name} requires UI approval — not applied automatically"
                            );
                            tool_result_blocks.push(ContentBlock::tool_result(
                                &tool_use_id,
                                &err_msg,
                            ));
                            let _ = event_tx.send(AgentEvent::ToolResult {
                                tool_id: tool_use_id.clone(),
                                tool_name: tool_name.clone(),
                                output: err_msg,
                                error: Some("requires approval".into()),
                            });
                        }
                        Err(e) => {
                            tool_result_blocks.push(ContentBlock::tool_result(
                                &tool_use_id,
                                &format!("Error: {e}"),
                            ));
                            let _ = event_tx.send(AgentEvent::ToolResult {
                                tool_id: tool_use_id.clone(),
                                tool_name: tool_name.clone(),
                                output: String::new(),
                                error: Some(e),
                            });
                        }
                    }
                }
                permissions::PermissionLevel::RequiresApproval => {
                    let result = tools::execute(&tool_name, tool_input.clone(), ctx).await;
                    match result {
                        Ok(ToolOutput::Text { content }) => {
                            tool_result_blocks.push(ContentBlock::tool_result(&tool_use_id, &content));
                            let _ = event_tx.send(AgentEvent::ToolResult {
                                tool_id: tool_use_id.clone(),
                                tool_name: tool_name.clone(),
                                output: content,
                                error: None,
                            });
                        }
                        Ok(ToolOutput::EditProposal { path, old_string, new_string, unified_diff }) => {
                            let proposal = EditProposalData {
                                path,
                                old_string,
                                new_string,
                                unified_diff,
                            };

                            let approval_key = format!("{}:{}", session_id, tool_use_id);
                            let (approve_tx, approve_rx) = oneshot::channel::<bool>();
                            {
                                let mut map = approvals.lock().await;
                                map.insert(approval_key.clone(), approve_tx);
                            }

                            let _ = event_tx.send(AgentEvent::ToolCall {
                                session_id: session_id.to_string(),
                                tool_id: tool_use_id.clone(),
                                tool_name: tool_name.clone(),
                                args: tool_input.clone(),
                                permission: "requires_approval".into(),
                                edit_proposal: Some(proposal),
                            });

                            match approve_rx.await {
                                Ok(true) => {
                                    let result = tools::apply_edit_with_ctx(tool_input.clone(), ctx).await;
                                    match result {
                                        Ok(msg) => {
                                            tool_result_blocks.push(ContentBlock::tool_result(&tool_use_id, &msg));
                                            let _ = event_tx.send(AgentEvent::ToolResult {
                                                tool_id: tool_use_id.clone(),
                                                tool_name: tool_name.clone(),
                                                output: msg,
                                                error: None,
                                            });
                                        }
                                        Err(e) => {
                                            tool_result_blocks.push(ContentBlock::tool_result(
                                                &tool_use_id,
                                                &format!("Error applying: {e}"),
                                            ));
                                            let _ = event_tx.send(AgentEvent::ToolResult {
                                                tool_id: tool_use_id.clone(),
                                                tool_name: tool_name.clone(),
                                                output: String::new(),
                                                error: Some(e),
                                            });
                                        }
                                    }
                                }
                                Ok(false) => {
                                    let msg = "Edit rejected by user".to_string();
                                    tool_result_blocks.push(ContentBlock::tool_result(&tool_use_id, &msg));
                                    let _ = event_tx.send(AgentEvent::ToolResult {
                                        tool_id: tool_use_id.clone(),
                                        tool_name: tool_name.clone(),
                                        output: msg,
                                        error: None,
                                    });
                                }
                                Err(_) => {
                                    let msg = "Approval channel closed".to_string();
                                    tool_result_blocks.push(ContentBlock::tool_result(&tool_use_id, &msg));
                                }
                            }
                        }
                        Err(e) => {
                            tool_result_blocks.push(ContentBlock::tool_result(
                                &tool_use_id,
                                &format!("Error: {e}"),
                            ));
                            let _ = event_tx.send(AgentEvent::ToolResult {
                                tool_id: tool_use_id.clone(),
                                tool_name: tool_name.clone(),
                                output: String::new(),
                                error: Some(e),
                            });
                        }
                    }
                }
            }
        }

        history.push(Message {
            role: "assistant".into(),
            content: tool_assistant_blocks,
        });

        history.push(Message {
            role: "user".into(),
            content: tool_result_blocks,
        });
    }

    Ok(())
}
