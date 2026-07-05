use crate::agent::permissions;
use crate::agent::persist::{now_ms, SessionRecord, SessionStore};
use crate::agent::provider::{self, AgentConfig, ContentBlock, Message, ToolDescription};
use crate::agent::tools::{self, ToolContext, ToolOutput};
use serde::Serialize;
use serde_json::Value;
use std::collections::HashMap;
use std::sync::Arc;
use tauri::ipc::Channel;
use tokio::sync::{oneshot, Mutex};

/// Cache-stable system prompt. This is the byte-identical prefix of every
/// request in a session — keep it constant so the provider's prefix cache stays
/// warm.
const SYSTEM_PROMPT: &str = "You are Claudinio, an AI coding agent inside the Claudinio Code desktop app. \
Work in a single continuous loop: before each step, judge whether you already have enough to respond, or \
whether another tool call is still needed — don't take steps you don't need. \
If the request is a question, investigate with read-only tools as needed and answer directly; \
do not produce a ceremonial plan or summary for something that needed neither. \
If the request requires changing code, briefly state your plan in a sentence or two, then carry it out — \
each file edit is shown to the user for approval before it lands. When you finish a change, close with a short, \
concrete recap of what changed and how to verify it. \
If you are missing information only the user can supply, or need a decision from them, call the ask_user tool \
with concrete options instead of ending the turn with an open question — do not guess. \
Be focused and concrete. Reply in the user's language (Portuguese if they write in Portuguese).";

/// Build the per-session system prompt. The result is byte-identical for every
/// request in the same workspace, so the provider's prefix cache stays warm.
fn system_prompt(workspace_root: Option<&str>) -> String {
    match workspace_root {
        Some(root) => format!(
            "{SYSTEM_PROMPT}\n\nProject workspace root: {root}. \
The bash tool already runs with this directory as its working directory — run commands directly \
(e.g. \"git status\"), use relative paths, and never cd into guessed paths. \
File tools take absolute paths inside this root."
        ),
        None => SYSTEM_PROMPT.to_string(),
    }
}

/// Safety cap on tool-call rounds per user message to bound runaway loops.
const MAX_ROUNDS: usize = 30;

#[derive(Debug, Clone, Serialize)]
#[serde(tag = "event", content = "data")]
pub enum AgentEvent {
    #[serde(rename = "TextStep")]
    TextStep {
        text: String,
    },
    #[serde(rename = "Thinking")]
    Thinking(String),
    #[serde(rename = "ToolCall")]
    ToolCall {
        #[serde(rename = "sessionId")]
        session_id: String,
        #[serde(rename = "toolId")]
        tool_id: String,
        #[serde(rename = "toolName")]
        tool_name: String,
        args: Value,
        permission: String,
        #[serde(rename = "editProposal")]
        edit_proposal: Option<EditProposalData>,
    },
    #[serde(rename = "ToolResult")]
    ToolResult {
        #[serde(rename = "toolId")]
        tool_id: String,
        #[serde(rename = "toolName")]
        tool_name: String,
        output: String,
        error: Option<String>,
    },
    #[serde(rename = "AskUser")]
    AskUser {
        #[serde(rename = "sessionId")]
        session_id: String,
        #[serde(rename = "toolId")]
        tool_id: String,
        questions: Value,
    },
    #[serde(rename = "Done")]
    Done {
        #[serde(rename = "stopReason")]
        stop_reason: String,
        #[serde(rename = "textOutput")]
        text_output: String,
        #[serde(rename = "inputTokens")]
        input_tokens: u32,
        #[serde(rename = "outputTokens")]
        output_tokens: u32,
    },
    #[serde(rename = "Error")]
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

/// One answered question from the ask_user tool: the frontend echoes the
/// question text back with the option the user picked (or typed).
#[derive(Debug, Clone, Serialize, serde::Deserialize)]
pub struct UserAnswer {
    pub question: String,
    pub answer: String,
}

pub type AnswerMap = Arc<Mutex<HashMap<String, oneshot::Sender<Vec<UserAnswer>>>>>;

fn api_tools() -> Vec<ToolDescription> {
    tools::get_defs()
        .iter()
        .map(|t| ToolDescription {
            name: t.name.clone(),
            description: t.description.clone(),
            input_schema: t.input_schema.clone(),
        })
        .collect()
}

/// Push a message onto history and persist it as a Turn record.
fn push_turn(history: &mut Vec<Message>, store: &SessionStore, message: Message) {
    store.try_append(&SessionRecord::Turn {
        message: message.clone(),
        ts: now_ms(),
    });
    history.push(message);
}

/// Add user-role content, merging into the previous message when it is already a
/// user turn. The Anthropic API requires strictly alternating roles, so this
/// prevents two consecutive user turns (which can happen when the model returns
/// nothing). Merges are intentionally not persisted as a new Turn record,
/// keeping the JSONL history alternating on reopen as well.
fn push_user_blocks(history: &mut Vec<Message>, store: &SessionStore, blocks: Vec<ContentBlock>) {
    if let Some(last) = history.last_mut() {
        if last.role == "user" {
            last.content.extend(blocks);
            return;
        }
    }
    push_turn(
        history,
        store,
        Message {
            role: "user".into(),
            content: blocks,
        },
    );
}

/// Run a single continuous provider→tool loop for one user input, until the
/// model produces a turn with no tool calls (or the safety cap is hit). Shares
/// one conversation history (append-only, cache-friendly) and persists every
/// step to the session JSONL store. The model decides at each round whether it
/// still needs a tool call or can answer directly — there are no forced phases.
#[allow(clippy::too_many_arguments)]
pub async fn run_workflow(
    config: &AgentConfig,
    history: &mut Vec<Message>,
    user_message: String,
    event_tx: &Channel<AgentEvent>,
    approvals: &ApprovalMap,
    answers: &AnswerMap,
    session_id: &str,
    ctx: &ToolContext,
    store: &SessionStore,
) -> Result<(), String> {
    store.try_append(&SessionRecord::User {
        text: user_message.clone(),
        ts: now_ms(),
    });
    push_user_blocks(history, store, vec![ContentBlock::text(&user_message)]);

    let system = system_prompt(ctx.workspace_root.as_deref());
    let tools = api_tools();
    let mut total_in: u32 = 0;
    let mut total_out: u32 = 0;
    let mut last_text = String::new();

    for _ in 0..MAX_ROUNDS {
        let mut assistant_text = String::new();
        let stream_output = provider::stream_message(
            config,
            history,
            &tools,
            Some(system.as_str()),
            event_tx,
            session_id,
            &mut assistant_text,
        )
        .await?;

        let text_output = assistant_text;
        let tool_uses = stream_output.tool_uses;
        if let Some(u) = &stream_output.usage {
            total_in += u.input_tokens;
            total_out += u.output_tokens;
        }

        // Terminal turn: no tool calls — the loop is done, this text is the reply.
        if tool_uses.is_empty() {
            if !text_output.is_empty() {
                push_turn(
                    history,
                    store,
                    Message {
                        role: "assistant".into(),
                        content: vec![ContentBlock::text(&text_output)],
                    },
                );
                last_text = text_output;
            }
            // If the model didn't produce a final text response, provide a
            // generic closing so the user doesn't see a blank answer.
            if last_text.is_empty() {
                last_text = "Pronto. Como posso ajudar mais?".into();
            }
            store.try_append(&SessionRecord::Done {
                input_tokens: total_in,
                output_tokens: total_out,
                ts: now_ms(),
            });
            let _ = event_tx.send(AgentEvent::Done {
                stop_reason: "end_turn".into(),
                text_output: last_text,
                input_tokens: total_in,
                output_tokens: total_out,
            });
            return Ok(());
        }

        // The model wants to use tools: the assistant message carries the
        // (optional) text plus every tool_use block; the following user message
        // carries the paired tool_result blocks. Any text alongside tool calls is
        // an intermediate step (e.g. a stated plan) — surface it in the timeline.
        let mut tool_assistant_blocks: Vec<ContentBlock> = Vec::new();
        if !text_output.is_empty() {
            tool_assistant_blocks.push(ContentBlock::text(&text_output));
            let _ = event_tx.send(AgentEvent::TextStep {
                text: text_output.clone(),
            });
        }
        let mut tool_result_blocks: Vec<ContentBlock> = Vec::new();

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
            let tool_input = tool_use.get("input").cloned().unwrap_or(Value::Null);

            tool_assistant_blocks.push(ContentBlock::tool_use(
                &tool_use_id,
                &tool_name,
                tool_input.clone(),
            ));

            let block = run_tool(
                &tool_name,
                &tool_use_id,
                tool_input,
                permissions::tool_permission(&tool_name),
                event_tx,
                approvals,
                answers,
                session_id,
                ctx,
            )
            .await;
            tool_result_blocks.push(block);
        }

        push_turn(
            history,
            store,
            Message {
                role: "assistant".into(),
                content: tool_assistant_blocks,
            },
        );
        push_turn(
            history,
            store,
            Message {
                role: "user".into(),
                content: tool_result_blocks,
            },
        );
    }

    // Safety cap hit: stop looping and report what we have so far rather than
    // running forever.
    let capped_text = if last_text.is_empty() {
        format!("Parei após {MAX_ROUNDS} rounds de ferramentas sem concluir. Tente reformular o pedido em partes menores.")
    } else {
        format!("{last_text}\n\n(Parei após {MAX_ROUNDS} rounds de ferramentas — pode não estar completo.)")
    };
    store.try_append(&SessionRecord::Done {
        input_tokens: total_in,
        output_tokens: total_out,
        ts: now_ms(),
    });
    let _ = event_tx.send(AgentEvent::Done {
        stop_reason: "max_rounds".into(),
        text_output: capped_text,
        input_tokens: total_in,
        output_tokens: total_out,
    });
    Ok(())
}

/// Execute one tool call (honoring its permission level) and return the
/// `tool_result` block to feed back to the model. Emits the matching UI events.
#[allow(clippy::too_many_arguments)]
async fn run_tool(
    tool_name: &str,
    tool_use_id: &str,
    tool_input: Value,
    perm: permissions::PermissionLevel,
    event_tx: &Channel<AgentEvent>,
    approvals: &ApprovalMap,
    answers: &AnswerMap,
    session_id: &str,
    ctx: &ToolContext,
) -> ContentBlock {
    // ask_user is inherently interactive: it never executes anything, it blocks
    // until the user answers in the UI (or the request is dropped).
    if tool_name == "ask_user" {
        return ask_user(tool_name, tool_use_id, tool_input, event_tx, answers, session_id).await;
    }
    match perm {
        permissions::PermissionLevel::Auto => {
            let _ = event_tx.send(AgentEvent::ToolCall {
                session_id: session_id.to_string(),
                tool_id: tool_use_id.to_string(),
                tool_name: tool_name.to_string(),
                args: tool_input.clone(),
                permission: "auto".into(),
                edit_proposal: None,
            });

            match tools::execute(tool_name, tool_input, ctx).await {
                Ok(ToolOutput::Text { content }) => {
                    let truncated = truncate(&content, 2000);
                    let _ = event_tx.send(AgentEvent::ToolResult {
                        tool_id: tool_use_id.to_string(),
                        tool_name: tool_name.to_string(),
                        output: truncated,
                        error: None,
                    });
                    ContentBlock::tool_result(tool_use_id, &content)
                }
                Ok(ToolOutput::EditProposal { .. }) => {
                    let err_msg = format!(
                        "edit_file for {tool_name} requires UI approval — not applied automatically"
                    );
                    let _ = event_tx.send(AgentEvent::ToolResult {
                        tool_id: tool_use_id.to_string(),
                        tool_name: tool_name.to_string(),
                        output: err_msg.clone(),
                        error: Some("requires approval".into()),
                    });
                    ContentBlock::tool_result(tool_use_id, &err_msg)
                }
                Err(e) => {
                    let _ = event_tx.send(AgentEvent::ToolResult {
                        tool_id: tool_use_id.to_string(),
                        tool_name: tool_name.to_string(),
                        output: String::new(),
                        error: Some(e.clone()),
                    });
                    ContentBlock::tool_result(tool_use_id, &format!("Error: {e}"))
                }
            }
        }
        permissions::PermissionLevel::RequiresApproval if tool_name == "bash" => {
            let command = tool_input
                .get("command")
                .and_then(|v| v.as_str())
                .unwrap_or("");

            match permissions::bash_permission(command) {
                permissions::PermissionLevel::Denied => {
                    let msg = format!("Command blocked by security policy: {command}");
                    let _ = event_tx.send(AgentEvent::ToolCall {
                        session_id: session_id.to_string(),
                        tool_id: tool_use_id.to_string(),
                        tool_name: tool_name.to_string(),
                        args: tool_input.clone(),
                        permission: "denied".into(),
                        edit_proposal: None,
                    });
                    let _ = event_tx.send(AgentEvent::ToolResult {
                        tool_id: tool_use_id.to_string(),
                        tool_name: tool_name.to_string(),
                        output: msg.clone(),
                        error: Some("denied".into()),
                    });
                    ContentBlock::tool_result(tool_use_id, &msg)
                }
                permissions::PermissionLevel::Auto => {
                    let _ = event_tx.send(AgentEvent::ToolCall {
                        session_id: session_id.to_string(),
                        tool_id: tool_use_id.to_string(),
                        tool_name: tool_name.to_string(),
                        args: tool_input.clone(),
                        permission: "auto".into(),
                        edit_proposal: None,
                    });
                    match tools::execute(tool_name, tool_input.clone(), ctx).await {
                        Ok(ToolOutput::Text { content }) => {
                            let truncated = truncate(&content, 2000);
                            let _ = event_tx.send(AgentEvent::ToolResult {
                                tool_id: tool_use_id.to_string(),
                                tool_name: tool_name.to_string(),
                                output: truncated,
                                error: None,
                            });
                            ContentBlock::tool_result(tool_use_id, &content)
                        }
                        _ => {
                            let err = "unexpected output type from bash".to_string();
                            let _ = event_tx.send(AgentEvent::ToolResult {
                                tool_id: tool_use_id.to_string(),
                                tool_name: tool_name.to_string(),
                                output: err.clone(),
                                error: Some("unexpected".into()),
                            });
                            ContentBlock::tool_result(tool_use_id, &err)
                        }
                    }
                }
                permissions::PermissionLevel::RequiresApproval => {
                    let approval_key = format!("{session_id}:{tool_use_id}");
                    let (approve_tx, approve_rx) = oneshot::channel::<bool>();
                    {
                        let mut map = approvals.lock().await;
                        map.insert(approval_key.clone(), approve_tx);
                    }

                    let _ = event_tx.send(AgentEvent::ToolCall {
                        session_id: session_id.to_string(),
                        tool_id: tool_use_id.to_string(),
                        tool_name: tool_name.to_string(),
                        args: tool_input.clone(),
                        permission: "requires_approval".into(),
                        edit_proposal: None,
                    });

                    match approve_rx.await {
                        Ok(true) => match tools::execute(tool_name, tool_input.clone(), ctx).await {
                            Ok(ToolOutput::Text { content }) => {
                                let truncated = truncate(&content, 2000);
                                let _ = event_tx.send(AgentEvent::ToolResult {
                                    tool_id: tool_use_id.to_string(),
                                    tool_name: tool_name.to_string(),
                                    output: truncated,
                                    error: None,
                                });
                                ContentBlock::tool_result(tool_use_id, &content)
                            }
                            Ok(ToolOutput::EditProposal { .. }) => {
                                let err_msg: String =
                                    "bash should not produce edit proposals".into();
                                let _ = event_tx.send(AgentEvent::ToolResult {
                                    tool_id: tool_use_id.to_string(),
                                    tool_name: tool_name.to_string(),
                                    output: err_msg.clone(),
                                    error: Some("unexpected output type".into()),
                                });
                                ContentBlock::tool_result(tool_use_id, &err_msg)
                            }
                            Err(e) => {
                                let _ = event_tx.send(AgentEvent::ToolResult {
                                    tool_id: tool_use_id.to_string(),
                                    tool_name: tool_name.to_string(),
                                    output: String::new(),
                                    error: Some(e.clone()),
                                });
                                ContentBlock::tool_result(tool_use_id, &format!("Error: {e}"))
                            }
                        },
                        Ok(false) => {
                            let msg = "Command rejected by user".to_string();
                            let _ = event_tx.send(AgentEvent::ToolResult {
                                tool_id: tool_use_id.to_string(),
                                tool_name: tool_name.to_string(),
                                output: msg.clone(),
                                error: None,
                            });
                            ContentBlock::tool_result(tool_use_id, &msg)
                        }
                        Err(_) => {
                            ContentBlock::tool_result(tool_use_id, "Approval channel closed")
                        }
                    }
                }
            }
        }
        permissions::PermissionLevel::RequiresApproval => {
            match tools::execute(tool_name, tool_input.clone(), ctx).await {
                Ok(ToolOutput::Text { content }) => {
                    let _ = event_tx.send(AgentEvent::ToolResult {
                        tool_id: tool_use_id.to_string(),
                        tool_name: tool_name.to_string(),
                        output: content.clone(),
                        error: None,
                    });
                    ContentBlock::tool_result(tool_use_id, &content)
                }
                Ok(ToolOutput::EditProposal {
                    path,
                    old_string,
                    new_string,
                    unified_diff,
                }) => {
                    let proposal = EditProposalData {
                        path,
                        old_string,
                        new_string,
                        unified_diff,
                    };

                    let approval_key = format!("{session_id}:{tool_use_id}");
                    let (approve_tx, approve_rx) = oneshot::channel::<bool>();
                    {
                        let mut map = approvals.lock().await;
                        map.insert(approval_key.clone(), approve_tx);
                    }

                    let _ = event_tx.send(AgentEvent::ToolCall {
                        session_id: session_id.to_string(),
                        tool_id: tool_use_id.to_string(),
                        tool_name: tool_name.to_string(),
                        args: tool_input.clone(),
                        permission: "requires_approval".into(),
                        edit_proposal: Some(proposal),
                    });

                    match approve_rx.await {
                        Ok(true) => match tools::apply_edit_with_ctx(tool_input, ctx).await {
                            Ok(msg) => {
                                let _ = event_tx.send(AgentEvent::ToolResult {
                                    tool_id: tool_use_id.to_string(),
                                    tool_name: tool_name.to_string(),
                                    output: msg.clone(),
                                    error: None,
                                });
                                ContentBlock::tool_result(tool_use_id, &msg)
                            }
                            Err(e) => {
                                let _ = event_tx.send(AgentEvent::ToolResult {
                                    tool_id: tool_use_id.to_string(),
                                    tool_name: tool_name.to_string(),
                                    output: String::new(),
                                    error: Some(e.clone()),
                                });
                                ContentBlock::tool_result(tool_use_id, &format!("Error applying: {e}"))
                            }
                        },
                        Ok(false) => {
                            let msg = "Edit rejected by user".to_string();
                            let _ = event_tx.send(AgentEvent::ToolResult {
                                tool_id: tool_use_id.to_string(),
                                tool_name: tool_name.to_string(),
                                output: msg.clone(),
                                error: None,
                            });
                            ContentBlock::tool_result(tool_use_id, &msg)
                        }
                        Err(_) => ContentBlock::tool_result(tool_use_id, "Approval channel closed"),
                    }
                }
                Err(e) => {
                    let _ = event_tx.send(AgentEvent::ToolResult {
                        tool_id: tool_use_id.to_string(),
                        tool_name: tool_name.to_string(),
                        output: String::new(),
                        error: Some(e.clone()),
                    });
                    ContentBlock::tool_result(tool_use_id, &format!("Error: {e}"))
                }
            }
        }
        permissions::PermissionLevel::Denied => {
            let msg = format!("Command '{tool_name}' is blocked by security policy");
            let _ = event_tx.send(AgentEvent::ToolCall {
                session_id: session_id.to_string(),
                tool_id: tool_use_id.to_string(),
                tool_name: tool_name.to_string(),
                args: tool_input.clone(),
                permission: "denied".into(),
                edit_proposal: None,
            });
            let _ = event_tx.send(AgentEvent::ToolResult {
                tool_id: tool_use_id.to_string(),
                tool_name: tool_name.to_string(),
                output: msg.clone(),
                error: Some("denied".into()),
            });
            ContentBlock::tool_result(tool_use_id, &msg)
        }
    }
}

/// Handle the ask_user tool: surface the questions in the UI, wait for the
/// user's answers and return them to the model as compiled question/answer
/// pairs. The ToolCall/ToolResult events keep the step visible in the timeline
/// and the tool_result block persists the answers in the session history.
async fn ask_user(
    tool_name: &str,
    tool_use_id: &str,
    tool_input: Value,
    event_tx: &Channel<AgentEvent>,
    answers: &AnswerMap,
    session_id: &str,
) -> ContentBlock {
    let _ = event_tx.send(AgentEvent::ToolCall {
        session_id: session_id.to_string(),
        tool_id: tool_use_id.to_string(),
        tool_name: tool_name.to_string(),
        args: tool_input.clone(),
        permission: "auto".into(),
        edit_proposal: None,
    });

    let questions = tool_input.get("questions").cloned().unwrap_or(Value::Null);
    if !questions.is_array() {
        let msg = "invalid ask_user input: 'questions' must be an array".to_string();
        let _ = event_tx.send(AgentEvent::ToolResult {
            tool_id: tool_use_id.to_string(),
            tool_name: tool_name.to_string(),
            output: String::new(),
            error: Some(msg.clone()),
        });
        return ContentBlock::tool_result(tool_use_id, &format!("Error: {msg}"));
    }

    let approval_key = format!("{session_id}:{tool_use_id}");
    let (answer_tx, answer_rx) = oneshot::channel::<Vec<UserAnswer>>();
    {
        let mut map = answers.lock().await;
        map.insert(approval_key, answer_tx);
    }

    let _ = event_tx.send(AgentEvent::AskUser {
        session_id: session_id.to_string(),
        tool_id: tool_use_id.to_string(),
        questions,
    });

    let compiled = match answer_rx.await {
        Ok(user_answers) => user_answers
            .iter()
            .map(|a| format!("Pergunta: {}\nResposta: {}", a.question, a.answer))
            .collect::<Vec<_>>()
            .join("\n\n"),
        Err(_) => "O usuário não respondeu.".to_string(),
    };

    let _ = event_tx.send(AgentEvent::ToolResult {
        tool_id: tool_use_id.to_string(),
        tool_name: tool_name.to_string(),
        output: compiled.clone(),
        error: None,
    });
    ContentBlock::tool_result(tool_use_id, &compiled)
}

fn truncate(s: &str, max: usize) -> String {
    if s.len() > max {
        // Respect char boundaries so we never slice mid-codepoint.
        let mut end = max;
        while end < s.len() && !s.is_char_boundary(end) {
            end += 1;
        }
        format!("{}...(truncated, {} chars total)", &s[..end], s.len())
    } else {
        s.to_string()
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn tmp_store() -> SessionStore {
        SessionStore {
            path: std::env::temp_dir().join(format!("claudinio_test_{}.jsonl", now_ms())),
        }
    }

    #[test]
    fn push_user_blocks_merges_consecutive_user_turns() {
        let store = tmp_store();
        let mut history = vec![Message {
            role: "user".into(),
            content: vec![ContentBlock::text("a")],
        }];
        push_user_blocks(&mut history, &store, vec![ContentBlock::text("b")]);
        assert_eq!(history.len(), 1, "second user turn must merge, not append");
        assert_eq!(history[0].content.len(), 2);
        let _ = std::fs::remove_file(&store.path);
    }

    #[test]
    fn push_user_blocks_appends_after_assistant() {
        let store = tmp_store();
        let mut history = vec![Message {
            role: "assistant".into(),
            content: vec![ContentBlock::text("a")],
        }];
        push_user_blocks(&mut history, &store, vec![ContentBlock::text("b")]);
        assert_eq!(history.len(), 2, "user turn after assistant must append");
        assert_eq!(history[1].role, "user");
        let _ = std::fs::remove_file(&store.path);
    }

    #[test]
    fn user_turn_carries_only_the_raw_message_no_injected_directive() {
        let store = tmp_store();
        let mut history: Vec<Message> = Vec::new();
        push_user_blocks(&mut history, &store, vec![ContentBlock::text("O que este projeto faz?")]);
        assert_eq!(history.len(), 1);
        assert_eq!(history[0].content.len(), 1, "no phase directive should be folded in");
        let _ = std::fs::remove_file(&store.path);
    }
}
