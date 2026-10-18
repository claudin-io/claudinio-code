//! Running one tool call the model asked for: the permission decision, the
//! approval gate, `ask_user` and mode switches, and shaping what the tool
//! returned into the `tool_result` block the next round carries.

use crate::agent::events::{AgentEvent, EditProposalData};
use crate::agent::handoff::{HandoffReason, HandoffSpec};
use crate::agent::permissions;
use crate::agent::permissions::PermissionLevel;
use crate::agent::persist::{SessionRecord, SessionStore, now_ms};
use crate::agent::provider::{AgentConfig, ContentBlock};
use crate::agent::session::{AnswerMap, ApprovalMap, ModeCtl, ModeOrigin, SessionMode, UserAnswer};
use crate::agent::subagent;
use crate::agent::tools::{self, ToolContext, ToolOutput};
use serde_json::Value;
use std::sync::Arc;
use tauri::ipc::Channel;
use tokio::sync::oneshot;

/// Resolve what a tool call actually needs, given the permission table, YOLO
/// mode and whatever a `PreToolUse` hook said.
///
/// Pulled out of [`run_tool`] because it is the security decision and deserves
/// to be tested as one rather than through a Tauri channel.
///
/// Two rules, and both of them are the same rule:
///
/// - **A relaxation is a relaxation of the prompt, never of the policy.** YOLO
///   mode and a hook's `allow` do the same thing and get the same carve-out:
///   `bash`'s deny-list and `browser`'s scheme check still refuse.
/// - **`ask` can only tighten.** It promotes an automatic tool into a prompt.
///   It cannot un-deny anything.
pub(super) fn effective_permission(
    perm: PermissionLevel,
    tool_name: &str,
    tool_input: &Value,
    config: &AgentConfig,
    ctx: &ToolContext,
    hook_verdict: &crate::agent::hooks::PreToolVerdict,
) -> PermissionLevel {
    let hook_allows = matches!(
        hook_verdict,
        crate::agent::hooks::PreToolVerdict::Allow { .. }
    );
    let hook_asks = matches!(
        hook_verdict,
        crate::agent::hooks::PreToolVerdict::Ask { .. }
    );
    let yolo_relaxes = config.yolo_mode && !config.yolo_blacklist.iter().any(|b| b == tool_name);

    let relaxed =
        if matches!(perm, PermissionLevel::RequiresApproval) && (yolo_relaxes || hook_allows) {
            if tool_name == "bash" {
                let command = tool_input
                    .get("command")
                    .and_then(|v| v.as_str())
                    .unwrap_or("");
                match permissions::bash_permission(command, ctx.auto_approve_git) {
                    PermissionLevel::Denied => PermissionLevel::Denied,
                    _ => PermissionLevel::Auto,
                }
            } else if tool_name == "browser" {
                match permissions::browser_permission(
                    tool_input
                        .get("action")
                        .and_then(|v| v.as_str())
                        .unwrap_or(""),
                    tool_input.get("url").and_then(|v| v.as_str()),
                ) {
                    PermissionLevel::Denied => PermissionLevel::Denied,
                    _ => PermissionLevel::Auto,
                }
            } else {
                PermissionLevel::Auto
            }
        } else {
            perm
        };

    match (hook_asks, relaxed) {
        (true, PermissionLevel::Auto) => PermissionLevel::RequiresApproval,
        (_, other) => other,
    }
}

/// How many times a `Stop` / `SubagentStop` hook may refuse to let a run end.
///
/// The protocol's own answer is `stop_hook_active`: a hook is told it already
/// blocked once and is expected to relent. That is a convention, and this is
/// what happens when a hook does not follow it. Same shape and same reasoning
/// as the quality-retry and continuation-judge caps in `run_loop`.
pub(crate) const MAX_STOP_HOOK_BLOCKS: u32 = 3;

/// A tool's result in the shape a `PostToolUse` hook expects.
///
/// `success` is derived the way the rest of this file derives it — from the
/// `Error`/`Error applying:` prefixes the tool arms write — because there is no
/// separate error flag on a `tool_result` block. Imperfect, and the same
/// imperfection the golden-loop and finalize-plan gates already live with.
pub(crate) fn tool_response_for_hook(block: &ContentBlock) -> Value {
    let text = match block {
        ContentBlock::ToolResult { content, .. } => content.as_text().into_owned(),
        _ => String::new(),
    };
    let ok = !text.starts_with("Error")
        && !text.starts_with("Tool call rejected")
        && !text.starts_with("Edit rejected");
    crate::agent::hooks::payload::tool_response(
        ok,
        &text,
        crate::agent::hooks::runner::MAX_OUTPUT_BYTES,
    )
}

/// Fire the `Notification` hook for a point where the agent has stopped and is
/// waiting on the user.
///
/// Spawned, never awaited: these are the two moments the user is already
/// waiting, and making the approval dialog wait on a notifier as well would be
/// the feature working against its own purpose.
fn notify_waiting(ctx: &ToolContext, message: &str, event_tx: &Channel<AgentEvent>) {
    if let Some(h) = &ctx.hooks {
        crate::agent::hooks::fire_notification(h, message, Some(event_tx));
    }
}

/// Execute a tool and emit the matching UI events, with no permission gate.
///
/// Extracted verbatim from the `Auto` arm of [`run_tool`] so a second caller can
/// have it: a PreToolUse hook answering `ask` needs *prompt, then execute*, and
/// the generic `RequiresApproval` arm does the opposite — it runs the tool and
/// only then gates the edit proposal. Reusing that arm for `ask` would run the
/// side effect and then ask permission for it.
#[allow(clippy::too_many_arguments)]
async fn execute_and_report(
    tool_name: &str,
    tool_use_id: &str,
    tool_input: Value,
    permission_label: &str,
    event_tx: &Channel<AgentEvent>,
    session_id: &str,
    ctx: &ToolContext,
) -> ContentBlock {
    let _ = event_tx.send(AgentEvent::ToolCall {
        session_id: session_id.to_string(),
        tool_id: tool_use_id.to_string(),
        tool_name: tool_name.to_string(),
        args: tool_input.clone(),
        permission: permission_label.into(),
        edit_proposal: None,
    });
    execute_reported(
        tool_name,
        tool_use_id,
        tool_input,
        permission_label,
        event_tx,
        session_id,
        ctx,
    )
    .await
}

/// The second half of [`execute_and_report`]: run the tool and send its
/// `ToolResult`, the `ToolCall` having been sent already. Split out for
/// `tool_batch`, which announces every call of a parallel run in the model's
/// order before any of them starts, so the timeline lists them in that order
/// however they finish.
#[allow(clippy::too_many_arguments)]
pub(super) async fn execute_reported(
    tool_name: &str,
    tool_use_id: &str,
    tool_input: Value,
    permission_label: &str,
    event_tx: &Channel<AgentEvent>,
    session_id: &str,
    ctx: &ToolContext,
) -> ContentBlock {
    match tools::execute(tool_name, tool_input, ctx).await {
        Ok(ToolOutput::Text { content }) => {
            let truncated = truncate(&content, 2000);
            let _ = event_tx.send(AgentEvent::ToolResult {
                tool_id: tool_use_id.to_string(),
                tool_name: tool_name.to_string(),
                output: truncated,
                error: None,
            });
            tool_result_block(tool_use_id, &content, ctx.limits.tool_result_chars())
        }
        Ok(ToolOutput::Rich { content, images }) => rich_result_block(
            tool_use_id,
            tool_name,
            &content,
            images,
            event_tx,
            ctx.limits.tool_result_chars(),
        ),
        Ok(ToolOutput::EditProposal {
            path,
            old_string,
            new_string,
            unified_diff,
        }) => {
            let proposal = EditProposalData {
                path: path.clone(),
                old_string: old_string.clone(),
                new_string: new_string.clone(),
                unified_diff,
            };
            // Reconstruct args since tool_input was moved into execute()
            let args = serde_json::json!({
                "path": path,
                "old_string": old_string,
                "new_string": new_string,
            });
            let _ = event_tx.send(AgentEvent::ToolCall {
                session_id: session_id.to_string(),
                tool_id: tool_use_id.to_string(),
                tool_name: tool_name.to_string(),
                args: args.clone(),
                permission: permission_label.into(),
                edit_proposal: Some(proposal),
            });
            match tools::apply_edit_with_ctx(args, ctx).await {
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
                    ContentBlock::tool_result(tool_use_id, format!("Error applying: {e}"))
                }
            }
        }
        Err(e) => {
            let _ = event_tx.send(AgentEvent::ToolResult {
                tool_id: tool_use_id.to_string(),
                tool_name: tool_name.to_string(),
                output: String::new(),
                error: Some(e.clone()),
            });
            ContentBlock::tool_result(tool_use_id, format!("Error: {e}"))
        }
    }
}

/// Execute one tool call (honoring its permission level) and return the
/// `tool_result` block to feed back to the model. Emits the matching UI events.
/// When `yolo_mode` is true and the tool is not in `yolo_blacklist`, tools that
/// normally require approval are auto-approved instead.
#[allow(clippy::too_many_arguments)]
pub(crate) async fn run_tool(
    tool_name: &str,
    tool_use_id: &str,
    tool_input: Value,
    perm: permissions::PermissionLevel,
    event_tx: &Channel<AgentEvent>,
    approvals: &ApprovalMap,
    answers: &AnswerMap,
    session_id: &str,
    ctx: &ToolContext,
    config: &AgentConfig,
    hook_verdict: &crate::agent::hooks::PreToolVerdict,
) -> ContentBlock {
    // ask_user is inherently interactive: it never executes anything, it blocks
    // until the user answers in the UI (or the request is dropped).
    if tool_name == "ask_user" {
        return ask_user(
            tool_name,
            tool_use_id,
            tool_input,
            event_tx,
            answers,
            session_id,
            ctx,
        )
        .await;
    }

    let effective_perm =
        effective_permission(perm, tool_name, &tool_input, config, ctx, hook_verdict);
    let hook_asks = matches!(
        hook_verdict,
        crate::agent::hooks::PreToolVerdict::Ask { .. }
    );

    match effective_perm {
        permissions::PermissionLevel::Auto => {
            execute_and_report(
                tool_name,
                tool_use_id,
                tool_input,
                "auto",
                event_tx,
                session_id,
                ctx,
            )
            .await
        }
        // A hook asked for a prompt on a tool that would not normally get one.
        // Placed above the bash / browser / MCP arms so `ask` beats their own
        // re-resolution (an allowlisted `git status` still prompts when a hook
        // says to), and below nothing — a `Denied` never reaches this match.
        permissions::PermissionLevel::RequiresApproval if hook_asks => {
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
            notify_waiting(
                ctx,
                &format!("Claudinio needs your permission to use {tool_name}"),
                event_tx,
            );
            match approve_rx.await {
                Ok(true) => {
                    execute_and_report(
                        tool_name,
                        tool_use_id,
                        tool_input,
                        "requires_approval",
                        event_tx,
                        session_id,
                        ctx,
                    )
                    .await
                }
                Ok(false) => {
                    let msg = "Tool call rejected by user";
                    let _ = event_tx.send(AgentEvent::ToolResult {
                        tool_id: tool_use_id.to_string(),
                        tool_name: tool_name.to_string(),
                        output: msg.into(),
                        error: Some("rejected".into()),
                    });
                    ContentBlock::tool_result(tool_use_id, msg)
                }
                Err(_) => ContentBlock::tool_result(tool_use_id, "Approval channel closed"),
            }
        }
        permissions::PermissionLevel::RequiresApproval if tool_name == "bash" => {
            let command = tool_input
                .get("command")
                .and_then(|v| v.as_str())
                .unwrap_or("");

            let resolved = if ctx.limits.writes_files() {
                permissions::bash_permission_writing(command, ctx.auto_approve_git)
            } else {
                permissions::bash_permission(command, ctx.auto_approve_git)
            };
            // An unattended caller decides every command itself; see
            // `ToolContext::shell_gate`.
            let resolved = match resolved {
                permissions::PermissionLevel::Auto if ctx.shell_gate.is_some() => {
                    permissions::PermissionLevel::RequiresApproval
                }
                other => other,
            };
            match resolved {
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
                            tool_result_block(tool_use_id, &content, ctx.limits.tool_result_chars())
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

                    // The run has stopped and the user is the only thing that can
                    // restart it. That is what a Notification hook is for.
                    notify_waiting(
                        ctx,
                        &format!("Claudinio needs your permission to use {tool_name}"),
                        event_tx,
                    );
                    match approve_rx.await {
                        Ok(true) => {
                            match tools::execute(tool_name, tool_input.clone(), ctx).await {
                                Ok(ToolOutput::Text { content }) => {
                                    let truncated = truncate(&content, 2000);
                                    let _ = event_tx.send(AgentEvent::ToolResult {
                                        tool_id: tool_use_id.to_string(),
                                        tool_name: tool_name.to_string(),
                                        output: truncated,
                                        error: None,
                                    });
                                    tool_result_block(
                                        tool_use_id,
                                        &content,
                                        ctx.limits.tool_result_chars(),
                                    )
                                }
                                Ok(_) => {
                                    let err_msg: String =
                                        "bash should only produce text output".into();
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
                                    ContentBlock::tool_result(tool_use_id, format!("Error: {e}"))
                                }
                            }
                        }
                        Ok(false) => {
                            let msg = ctx
                                .shell_gate
                                .as_ref()
                                .map_or("Command rejected by user", |g| g.rejection.as_str())
                                .to_string();
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
            }
        }
        // Same reason as the MCP arm below: the generic `RequiresApproval` arm
        // executes Text-producing tools BEFORE approval, and navigating to a
        // site is exactly the thing that must not happen until the user says
        // so. Resolve per action/URL, then approve, then execute.
        permissions::PermissionLevel::RequiresApproval if tool_name == "browser" => {
            let action = tool_input
                .get("action")
                .and_then(|v| v.as_str())
                .unwrap_or("");
            let url = tool_input.get("url").and_then(|v| v.as_str());
            // What gets typed into a page is often a password. The tool result
            // already reports only a character count; the event carries the raw
            // args, and it is persisted to the session JSONL and rendered in the
            // timeline, so redact here too.
            let shown_args = redact_typed_text(&tool_input);

            match permissions::browser_permission(action, url) {
                permissions::PermissionLevel::Denied => {
                    let msg = match url {
                        Some(u) => format!("Blocked by security policy: refusing to open {u}"),
                        None => format!("Blocked by security policy: browser action '{action}'"),
                    };
                    let _ = event_tx.send(AgentEvent::ToolCall {
                        session_id: session_id.to_string(),
                        tool_id: tool_use_id.to_string(),
                        tool_name: tool_name.to_string(),
                        args: shown_args.clone(),
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
                        args: shown_args.clone(),
                        permission: "auto".into(),
                        edit_proposal: None,
                    });
                    run_and_report(tool_name, tool_use_id, tool_input, ctx, event_tx).await
                }
                permissions::PermissionLevel::RequiresApproval => {
                    let approval_key = format!("{session_id}:{tool_use_id}");
                    let (approve_tx, approve_rx) = oneshot::channel::<bool>();
                    approvals.lock().await.insert(approval_key, approve_tx);

                    let _ = event_tx.send(AgentEvent::ToolCall {
                        session_id: session_id.to_string(),
                        tool_id: tool_use_id.to_string(),
                        tool_name: tool_name.to_string(),
                        args: shown_args.clone(),
                        permission: "requires_approval".into(),
                        edit_proposal: None,
                    });

                    // The run has stopped and the user is the only thing that can
                    // restart it. That is what a Notification hook is for.
                    notify_waiting(
                        ctx,
                        &format!("Claudinio needs your permission to use {tool_name}"),
                        event_tx,
                    );
                    match approve_rx.await {
                        Ok(true) => {
                            run_and_report(tool_name, tool_use_id, tool_input, ctx, event_tx).await
                        }
                        Ok(false) => {
                            let msg = "Navigation rejected by user".to_string();
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
            }
        }
        // MCP tools return `ToolOutput::Text`, not `EditProposal` — the
        // generic `RequiresApproval` arm below executes Text-producing tools
        // BEFORE approval (it only gates edit proposals), so MCP needs its
        // own approve-before-execute arm here, same shape as the bash one.
        permissions::PermissionLevel::RequiresApproval if tool_name.starts_with("mcp__") => {
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

            // The run has stopped and the user is the only thing that can
            // restart it. That is what a Notification hook is for.
            notify_waiting(
                ctx,
                &format!("Claudinio needs your permission to use {tool_name}"),
                event_tx,
            );
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
                        tool_result_block(tool_use_id, &content, ctx.limits.tool_result_chars())
                    }
                    Ok(ToolOutput::Rich { content, images }) => rich_result_block(
                        tool_use_id,
                        tool_name,
                        &content,
                        images,
                        event_tx,
                        ctx.limits.tool_result_chars(),
                    ),
                    Ok(ToolOutput::EditProposal { .. }) => {
                        let err_msg = "MCP tools should not produce edit proposals".to_string();
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
                        ContentBlock::tool_result(tool_use_id, format!("Error: {e}"))
                    }
                },
                Ok(false) => {
                    let msg = "Tool call rejected by user".to_string();
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
        permissions::PermissionLevel::RequiresApproval => {
            match tools::execute(tool_name, tool_input.clone(), ctx).await {
                Ok(ToolOutput::Text { content }) => {
                    let _ = event_tx.send(AgentEvent::ToolResult {
                        tool_id: tool_use_id.to_string(),
                        tool_name: tool_name.to_string(),
                        output: content.clone(),
                        error: None,
                    });
                    tool_result_block(tool_use_id, &content, ctx.limits.tool_result_chars())
                }
                Ok(ToolOutput::Rich { content, images }) => rich_result_block(
                    tool_use_id,
                    tool_name,
                    &content,
                    images,
                    event_tx,
                    ctx.limits.tool_result_chars(),
                ),
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

                    // The run has stopped and the user is the only thing that can
                    // restart it. That is what a Notification hook is for.
                    notify_waiting(
                        ctx,
                        &format!("Claudinio needs your permission to use {tool_name}"),
                        event_tx,
                    );
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
                                ContentBlock::tool_result(
                                    tool_use_id,
                                    format!("Error applying: {e}"),
                                )
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
                    ContentBlock::tool_result(tool_use_id, format!("Error: {e}"))
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

/// Emit a denied ToolCall/ToolResult pair and return the tool_result block.
/// Used for tools blocked by the current mode (no approval prompt — a hard no).
pub(crate) fn deny_tool(
    tool_name: &str,
    tool_use_id: &str,
    tool_input: &Value,
    msg: &str,
    event_tx: &Channel<AgentEvent>,
    session_id: &str,
) -> ContentBlock {
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
        output: msg.to_string(),
        error: Some("denied".into()),
    });
    ContentBlock::tool_result(tool_use_id, msg)
}

/// Rewrite a spawn_agents input so every agent runs in 'explore' mode.
/// Brain must never spawn code-mode (write-capable) subagents.
/// Normalizes flattened single-spec inputs first so they can't bypass the
/// rewrite and reach run_spawn_agents still carrying mode='code'.
pub(super) fn force_explore_mode(tool_input: Value) -> Value {
    let mut tool_input = subagent::normalize_spawn_input(tool_input);
    if let Some(agents) = tool_input.get_mut("agents").and_then(|v| v.as_array_mut()) {
        for agent in agents {
            if let Some(obj) = agent.as_object_mut() {
                obj.insert("mode".into(), Value::String("explore".into()));
            }
        }
    }
    tool_input
}

/// Handle the agent-initiated mode switch tools. enter_plan_mode always
/// works (origin becomes Agent); exit_plan_mode only works when the agent
/// itself entered Brain — a human-initiated Brain is exited only by
/// the human toggle.
#[allow(clippy::too_many_arguments)]
pub(super) fn handle_mode_switch(
    tool_name: &str,
    tool_use_id: &str,
    tool_input: &Value,
    mode_ctl: &Arc<ModeCtl>,
    store: &SessionStore,
    ctx: &ToolContext,
    event_tx: &Channel<AgentEvent>,
    session_id: &str,
    pending_handoff: &mut Option<HandoffSpec>,
) -> ContentBlock {
    let _ = event_tx.send(AgentEvent::ToolCall {
        session_id: session_id.to_string(),
        tool_id: tool_use_id.to_string(),
        tool_name: tool_name.to_string(),
        args: tool_input.clone(),
        permission: "auto".into(),
        edit_proposal: None,
    });

    let (mode, origin) = mode_ctl.get();
    let (result, error): (String, Option<String>) = match tool_name {
        "enter_plan_mode" => {
            if mode == SessionMode::Brain {
                ("Already in Brain mode.".into(), Some("invalid".into()))
            } else {
                let reason = tool_input
                    .get("reason")
                    .and_then(|v| v.as_str())
                    .map(|s| s.to_string());
                mode_ctl.set(SessionMode::Brain, ModeOrigin::Agent);
                store.try_append(&SessionRecord::Mode {
                    mode: SessionMode::Brain.as_str().into(),
                    origin: ModeOrigin::Agent.as_str().into(),
                    ts: now_ms(),
                });
                crate::agent::persist::invalidate_cache(&store.path, &ctx.records_cache);
                let _ = event_tx.send(AgentEvent::ModeChanged {
                    mode: SessionMode::Brain.as_str().into(),
                    origin: ModeOrigin::Agent.as_str().into(),
                    reason,
                });
                (
                    "Entered Brain mode (read-only planning). Editing tools are now \
                     disabled. Explore, interview the user, write the plan with write_plan, \
                     create tasks with tasks_set, then call exit_plan_mode to build."
                        .into(),
                    None,
                )
            }
        }
        "exit_plan_mode" => {
            if mode != SessionMode::Brain {
                ("Not in Brain mode.".into(), Some("invalid".into()))
            } else if origin != ModeOrigin::Agent {
                (
                    "The USER enabled Brain mode — only they can switch back to \
                     Builder. Finish the plan and tasks, then end your turn telling \
                     the user everything is ready for them to flip the toggle."
                        .into(),
                    Some("denied".into()),
                )
            } else {
                store.try_append(&SessionRecord::Mode {
                    mode: SessionMode::Builder.as_str().into(),
                    origin: ModeOrigin::Agent.as_str().into(),
                    ts: now_ms(),
                });
                crate::agent::persist::invalidate_cache(&store.path, &ctx.records_cache);
                *pending_handoff = Some(HandoffSpec {
                    reason: HandoffReason::PlanExecution,
                    next_mode: SessionMode::Builder,
                    next_origin: ModeOrigin::Agent,
                    first_message: "Plan approved. Read the plan and execute the tasks.".into(),
                    golden_cycle: 0,
                    golden_stalls: 0,
                    golden_last_pending: vec![],
                });
                // Auto-commit the latest plan when exiting Brain mode
                let auto_commit = ctx
                    .agent_config
                    .as_ref()
                    .map(|c| c.auto_commit_plan)
                    .unwrap_or(true);
                if auto_commit && let Some(root) = &ctx.workspace_root {
                    let plan_save_path = ctx.plan_save_path.as_deref();
                    if let Some(plan_path) =
                        crate::agent::tools::write_plan::latest_plan_path(root, plan_save_path)
                    {
                        let fname = plan_path
                            .file_stem()
                            .and_then(|s| s.to_str())
                            .unwrap_or("plan");
                        // filename format: YYYY-MM-DD_slug.md — strip date prefix
                        let slug = if fname.len() > 11 && &fname[4..5] == "-" {
                            &fname[11..] // after "YYYY-MM-DD_"
                        } else {
                            fname
                        };
                        let commit_msg = format!("docs(plan): {}", slug);
                        let add = std::process::Command::new("git")
                            .arg("-C")
                            .arg(root)
                            .arg("add")
                            .arg(plan_path.to_string_lossy().as_ref())
                            .output();
                        if add.is_ok() {
                            let _ = std::process::Command::new("git")
                                .arg("-C")
                                .arg(root)
                                .arg("commit")
                                .arg("-m")
                                .arg(&commit_msg)
                                .output();
                        }
                    }
                }
                (
                    "Plan approved. Switching to Builder mode via new session... End your turn."
                        .into(),
                    None,
                )
            }
        }
        _ => ("unknown mode tool".into(), Some("invalid".into())),
    };

    let _ = event_tx.send(AgentEvent::ToolResult {
        tool_id: tool_use_id.to_string(),
        tool_name: tool_name.to_string(),
        output: result.clone(),
        error,
    });
    ContentBlock::tool_result(tool_use_id, &result)
}

/// Await the user's answer, guarding against the oneshot being dropped early.
/// If the sender goes away (aborted task, lifecycle churn) the channel is
/// recreated up to a retry limit — the gate must only be released by a real
/// answer or by exhausting the retries.
async fn await_user_answer(answers: &AnswerMap, session_id: &str, tool_use_id: &str) -> String {
    let key = format!("{session_id}:{tool_use_id}");
    let mut retries = 10usize;
    loop {
        let (answer_tx, answer_rx) = oneshot::channel::<Vec<UserAnswer>>();
        {
            let mut map = answers.lock().await;
            map.insert(key.clone(), answer_tx);
        }

        match answer_rx.await {
            Ok(answers) => {
                return answers
                    .iter()
                    .map(|a| format!("Pergunta: {}\nResposta: {}", a.question, a.answer))
                    .collect::<Vec<_>>()
                    .join("\n\n");
            }
            Err(_recv_err) => {
                eprintln!(
                    "[ask_user] oneshot dropped for {}:{} — retries left: {}",
                    session_id, tool_use_id, retries
                );
                if retries == 0 {
                    return "The user did not respond.".to_string();
                }
                retries -= 1;
                tokio::time::sleep(std::time::Duration::from_millis(100)).await;
            }
        }
    }
}

/// Handle the ask_user tool: surface the questions in the UI, wait for the
/// user's answers and return them to the model as compiled question/answer
/// pairs. The ToolCall/ToolResult events keep the step visible in the timeline
/// and the tool_result block persists the answers in the session history.
/// Normalize and validate the `questions` payload for ask_user before surfacing
/// it to the UI. Each option is normalized to a `{ label, description? }` object:
/// plain strings become `{ label }`, and the richer AskUserQuestion object shape
/// models naturally reach for (`{ label/value/title, description }`, sometimes
/// description-only) is mapped onto that shape rather than rejected. Trims and
/// de-duplicates on the label, and returns Err(detail) for anything the user
/// could not answer sensibly (empty text, <2 options, blank/duplicate options, an
/// object with no usable text) so the caller can hand the error back to the model
/// to retry instead of rendering a broken card. On success it returns a cleaned
/// `questions` array whose options are `{ label, description? }` objects.
fn normalize_ask_user_questions(questions: &Value) -> Result<Value, String> {
    let arr = questions
        .as_array()
        .ok_or_else(|| "'questions' must be an array".to_string())?;
    if arr.is_empty() {
        return Err("'questions' must contain at least one question".to_string());
    }
    let mut out = Vec::with_capacity(arr.len());
    for (qi, q) in arr.iter().enumerate() {
        let at = |s: &str| format!("question {}: {s}", qi + 1);
        let obj = q
            .as_object()
            .ok_or_else(|| at("each question must be an object"))?;
        let text = obj.get("question").and_then(|v| v.as_str()).unwrap_or("");
        if text.trim().is_empty() {
            return Err(at("'question' text must not be empty"));
        }
        let opts = obj
            .get("options")
            .and_then(|v| v.as_array())
            .ok_or_else(|| at("'options' must be an array"))?;
        if opts.len() < 2 {
            return Err(at("provide at least 2 options"));
        }
        let mut seen: std::collections::HashSet<String> = std::collections::HashSet::new();
        let mut norm_opts = Vec::with_capacity(opts.len());
        for opt in opts {
            // Each option becomes { label, description? }. Models frequently reach
            // for the richer AskUserQuestion object shape ({ label, description },
            // sometimes description-only) — accept all of it instead of rejecting.
            let (label, description): (String, Option<String>) = match opt {
                Value::String(s) => (s.trim().to_string(), None),
                Value::Object(o) => {
                    let pick = |k: &str| {
                        o.get(k)
                            .and_then(|v| v.as_str())
                            .map(str::trim)
                            .filter(|s| !s.is_empty())
                            .map(str::to_string)
                    };
                    let short = pick("label")
                        .or_else(|| pick("value"))
                        .or_else(|| pick("title"))
                        .or_else(|| pick("header"));
                    let desc = pick("description").or_else(|| pick("text"));
                    match (short, desc) {
                        (Some(short), desc) => {
                            // Drop a description that merely repeats the label.
                            let desc = desc.filter(|d| d.to_lowercase() != short.to_lowercase());
                            (short, desc)
                        }
                        // Description-only (the shape that used to be rejected):
                        // fall back to it as the label so the button is still usable.
                        (None, Some(desc)) => (desc, None),
                        (None, None) => {
                            return Err(at(
                                "option objects must carry a 'label', 'value', 'title', or 'description' string",
                            ));
                        }
                    }
                }
                _ => {
                    return Err(at(
                        "every option must be a string or a {label, description} object",
                    ));
                }
            };
            if label.is_empty() {
                return Err(at("options must not be empty or blank"));
            }
            if !seen.insert(label.to_lowercase()) {
                return Err(at(&format!(
                    "duplicate option '{label}' — options must be distinct"
                )));
            }
            let mut opt_obj = serde_json::Map::new();
            opt_obj.insert("label".into(), Value::String(label));
            if let Some(desc) = description {
                opt_obj.insert("description".into(), Value::String(desc));
            }
            norm_opts.push(Value::Object(opt_obj));
        }
        let mut new_q = serde_json::Map::new();
        new_q.insert("question".into(), Value::String(text.trim().to_string()));
        if let Some(ms) = obj.get("multi_select") {
            new_q.insert("multi_select".into(), ms.clone());
        }
        new_q.insert("options".into(), Value::Array(norm_opts));
        out.push(Value::Object(new_q));
    }
    Ok(Value::Array(out))
}

/// Build a clear, actionable error for a rejected ask_user call: it states what
/// was wrong (`detail`) and then shows a correct, copy-pasteable example that
/// reuses the model's own question text, so the fix is concrete rather than
/// abstract. Prevents the "retry the same broken call" loop.
fn ask_user_error_with_example(detail: &str, raw_questions: &Value) -> String {
    let question = raw_questions
        .as_array()
        .and_then(|a| a.first())
        .and_then(|q| q.get("question"))
        .and_then(|v| v.as_str())
        .map(str::trim)
        .filter(|s| !s.is_empty())
        .unwrap_or("Your question here?");
    let example = serde_json::json!({
        "questions": [{
            "question": question,
            "options": ["First concrete choice", "Second concrete choice"],
            "multi_select": false
        }]
    });
    let example_str = serde_json::to_string_pretty(&example).unwrap_or_default();
    format!(
        "invalid ask_user input: {detail}.\n\n\
         How to fix: provide 2-4 distinct, non-empty options. Each option is EITHER a plain \
         string, OR an object {{ \"label\": \"concise choice\", \"description\": \"optional one-line detail\" }} \
         (label required, description optional). Do NOT add an \"Other\" option — the UI always \
         appends a free-text \"Other\" for you. If the answer is open-ended, still offer the most \
         likely concrete choices.\n\n\
         Example of a correct call for your question:\n{example_str}\n\n\
         Replace the example option text with the real, distinct choices and call ask_user again."
    )
}

async fn ask_user(
    tool_name: &str,
    tool_use_id: &str,
    tool_input: Value,
    event_tx: &Channel<AgentEvent>,
    answers: &AnswerMap,
    session_id: &str,
    ctx: &ToolContext,
) -> ContentBlock {
    let _ = event_tx.send(AgentEvent::ToolCall {
        session_id: session_id.to_string(),
        tool_id: tool_use_id.to_string(),
        tool_name: tool_name.to_string(),
        args: tool_input.clone(),
        permission: "auto".into(),
        edit_proposal: None,
    });

    let raw_questions = tool_input.get("questions").cloned().unwrap_or(Value::Null);
    let questions = match normalize_ask_user_questions(&raw_questions) {
        Ok(q) => q,
        Err(detail) => {
            let msg = ask_user_error_with_example(&detail, &raw_questions);
            let _ = event_tx.send(AgentEvent::ToolResult {
                tool_id: tool_use_id.to_string(),
                tool_name: tool_name.to_string(),
                output: String::new(),
                error: Some(msg.clone()),
            });
            return ContentBlock::tool_result(tool_use_id, format!("Error: {msg}"));
        }
    };

    let _ = event_tx.send(AgentEvent::AskUser {
        session_id: session_id.to_string(),
        tool_id: tool_use_id.to_string(),
        questions,
    });
    notify_waiting(ctx, "Claudinio is waiting for your input", event_tx);

    let compiled = await_user_answer(answers, session_id, tool_use_id).await;

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

/// Build a truncated tool_result block for the conversation history.
/// The event stream already truncates to MAX_EVENT_CHARS (~2k) for display;
/// this cap limits the history copy so a giant tool result (e.g. a subagent
/// report, file read, or search) can't blow up the context.
/// Replace the `text` of a `browser` type action with a placeholder.
///
/// Everything else about the call stays visible — the selector matters for
/// reading the transcript; the characters do not, and they are frequently a
/// credential.
fn redact_typed_text(args: &Value) -> Value {
    if args.get("action").and_then(|v| v.as_str()) != Some("type") {
        return args.clone();
    }
    let mut copy = args.clone();
    if let Some(text) = copy.get("text").and_then(|v| v.as_str()) {
        let count = text.chars().count();
        copy["text"] = Value::String(format!("<{count} characters hidden>"));
    }
    copy
}

/// Execute a tool and turn its output into a `tool_result` block, emitting the
/// matching UI events. Shared by the approve-before-execute arms, which all
/// need the same three-way handling once the gate has been cleared.
async fn run_and_report(
    tool_name: &str,
    tool_use_id: &str,
    tool_input: Value,
    ctx: &ToolContext,
    event_tx: &Channel<AgentEvent>,
) -> ContentBlock {
    match tools::execute(tool_name, tool_input, ctx).await {
        Ok(ToolOutput::Text { content }) => {
            let _ = event_tx.send(AgentEvent::ToolResult {
                tool_id: tool_use_id.to_string(),
                tool_name: tool_name.to_string(),
                output: truncate(&content, 2000),
                error: None,
            });
            tool_result_block(tool_use_id, &content, ctx.limits.tool_result_chars())
        }
        Ok(ToolOutput::Rich { content, images }) => rich_result_block(
            tool_use_id,
            tool_name,
            &content,
            images,
            event_tx,
            ctx.limits.tool_result_chars(),
        ),
        Ok(ToolOutput::EditProposal { .. }) => {
            let msg = format!("{tool_name} should not produce edit proposals");
            let _ = event_tx.send(AgentEvent::ToolResult {
                tool_id: tool_use_id.to_string(),
                tool_name: tool_name.to_string(),
                output: msg.clone(),
                error: Some("unexpected output type".into()),
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
            ContentBlock::tool_result(tool_use_id, format!("Error: {e}"))
        }
    }
}

/// The history copy of a tool result, cut to `max_chars`
/// (`ContextBudget::tool_result_chars`) so one large report, file read or
/// search cannot take the context — a cap that follows the model's window.
pub(super) fn tool_result_block(
    tool_use_id: &str,
    content: &str,
    max_chars: usize,
) -> ContentBlock {
    ContentBlock::tool_result(tool_use_id, truncate(content, max_chars))
}

/// What `truncate` appends, split around the size it reports.
const CAP_NOTE: (&str, &str) = ("...(truncated, ", " chars total)");

/// A result's text without the note an earlier cap left on it, and the size
/// the result had before any cap.
pub(super) fn uncapped(text: &str) -> (&str, usize) {
    let (open, close) = CAP_NOTE;
    if let Some(at) = text.rfind(open)
        && let Some(total) = text[at + open.len()..]
            .strip_suffix(close)
            .and_then(|n| n.parse::<usize>().ok())
    {
        return (&text[..at], total);
    }
    (text, text.len())
}

/// The least a result is cut to: below this it says nothing but its own note.
const MIN_ROUND_SHARE: usize = 200;

/// Hold the results of one round to `round_chars` together
/// (`ContextBudget::round_chars`). The cap per result does not bound a round:
/// a model that reads four files in one turn gets four results, and the round
/// just read is the one thing no prune will cut.
///
/// The room is shared the way water fills glasses: a small result keeps all of
/// itself, and what it does not use goes to the larger ones. A result the cap
/// per result already cut is cut from its text, not from text plus note, and
/// still reports the size it really had — that note is the only way the model
/// learns how much of a file it has not seen. The user's answers (`ask_user`,
/// looked up in `calls`, the assistant turn these results answer) are never
/// cut.
pub(crate) fn fit_round(blocks: &mut [ContentBlock], calls: &[ContentBlock], round_chars: usize) {
    use crate::agent::provider::ToolResultContent;
    if round_chars == usize::MAX {
        return;
    }
    let answers: std::collections::HashSet<&str> = calls
        .iter()
        .filter_map(|b| match b {
            ContentBlock::ToolUse { id, name, .. } if name == "ask_user" => Some(id.as_str()),
            _ => None,
        })
        .collect();
    let mut kept_whole = 0usize;
    let mut bodies: Vec<usize> = Vec::new();
    for b in blocks.iter() {
        if let ContentBlock::ToolResult {
            tool_use_id,
            content: ToolResultContent::Text(text),
            ..
        } = b
        {
            if answers.contains(tool_use_id.as_str()) {
                kept_whole += text.len();
            } else {
                bodies.push(uncapped(text).0.len());
            }
        }
    }
    let mut room = round_chars.saturating_sub(kept_whole);
    if bodies.iter().sum::<usize>() <= room {
        return;
    }
    // The level every larger result is cut to.
    bodies.sort_unstable();
    let mut level = 0;
    for (i, len) in bodies.iter().enumerate() {
        let share = room / (bodies.len() - i);
        if *len > share {
            level = share;
            break;
        }
        room -= len;
    }
    let level = level.max(MIN_ROUND_SHARE);
    for b in blocks.iter_mut() {
        if let ContentBlock::ToolResult {
            tool_use_id,
            content: ToolResultContent::Text(text),
            ..
        } = b
            && !answers.contains(tool_use_id.as_str())
        {
            let (body, total) = uncapped(text);
            if body.len() > level {
                let mut end = level;
                while end < body.len() && !body.is_char_boundary(end) {
                    end += 1;
                }
                *text = format!("{}{}{total}{}", &body[..end], CAP_NOTE.0, CAP_NOTE.1);
            }
        }
    }
}

/// Build the tool_result block for a `ToolOutput::Rich`, emitting both the
/// normal `ToolResult` event and the follow-up `ToolResultImages` one.
///
/// The text half is truncated exactly like any other tool result; the images
/// are passed through untouched because the producing tool already capped and
/// compressed them (`crate::imageutil`).
fn rich_result_block(
    tool_use_id: &str,
    tool_name: &str,
    content: &str,
    images: Vec<crate::imageutil::ImageAttachment>,
    event_tx: &Channel<AgentEvent>,
    max_chars: usize,
) -> ContentBlock {
    let _ = event_tx.send(AgentEvent::ToolResult {
        tool_id: tool_use_id.to_string(),
        tool_name: tool_name.to_string(),
        output: truncate(content, 2000),
        error: None,
    });
    if images.is_empty() {
        return tool_result_block(tool_use_id, content, max_chars);
    }
    let _ = event_tx.send(AgentEvent::ToolResultImages {
        tool_id: tool_use_id.to_string(),
        images: images.clone(),
    });

    let text = truncate(content, max_chars);
    let mut blocks = Vec::with_capacity(images.len() + 1);
    // Anthropic rejects empty text blocks, and a tool that returns only an
    // image (a bare screenshot) is a normal case, not an error.
    blocks.push(ContentBlock::text(if text.trim().is_empty() {
        format!("[{} image(s) attached]", images.len())
    } else {
        text
    }));
    for img in images {
        blocks.push(ContentBlock::image(
            img.media_type,
            img.data,
            img.width,
            img.height,
        ));
    }
    ContentBlock::tool_result_blocks(tool_use_id, blocks)
}

#[cfg(test)]
mod tests {
    use super::*;
    use lru::LruCache;
    use serde_json::json;
    use std::num::NonZeroUsize;

    // ── Hooks: the permission decision ───────────────────────────────────

    fn verdict_none() -> crate::agent::hooks::PreToolVerdict {
        crate::agent::hooks::PreToolVerdict::None
    }
    fn verdict_allow() -> crate::agent::hooks::PreToolVerdict {
        crate::agent::hooks::PreToolVerdict::Allow { reason: None }
    }
    fn verdict_ask() -> crate::agent::hooks::PreToolVerdict {
        crate::agent::hooks::PreToolVerdict::Ask { reason: None }
    }

    #[test]
    fn with_no_hook_the_permission_table_decides_exactly_as_before() {
        let ctx = dummy_ctx();
        let cfg = AgentConfig::default();
        for (tool, perm) in [
            ("read_file", PermissionLevel::Auto),
            ("bash", PermissionLevel::RequiresApproval),
            ("edit_file", PermissionLevel::RequiresApproval),
        ] {
            assert_eq!(
                effective_permission(perm, tool, &json!({}), &cfg, &ctx, &verdict_none()),
                perm,
                "{tool}"
            );
        }
    }

    #[test]
    fn a_hook_allow_skips_the_prompt_the_way_yolo_does() {
        let ctx = dummy_ctx();
        let cfg = AgentConfig::default();
        assert_eq!(
            effective_permission(
                PermissionLevel::RequiresApproval,
                "edit_file",
                &json!({"path": "a"}),
                &cfg,
                &ctx,
                &verdict_allow(),
            ),
            PermissionLevel::Auto
        );
    }

    #[test]
    fn a_hook_allow_does_not_override_the_bash_deny_list() {
        // The whole security posture of this feature in one assertion: hooks
        // may relax a prompt, never a policy.
        let ctx = dummy_ctx();
        let cfg = AgentConfig::default();
        assert_eq!(
            effective_permission(
                PermissionLevel::RequiresApproval,
                "bash",
                &json!({"command": "sudo rm -rf /"}),
                &cfg,
                &ctx,
                &verdict_allow(),
            ),
            PermissionLevel::Denied
        );
    }

    #[test]
    fn a_hook_allow_does_not_override_the_browser_scheme_check() {
        let ctx = dummy_ctx();
        let cfg = AgentConfig::default();
        assert_eq!(
            effective_permission(
                PermissionLevel::RequiresApproval,
                "browser",
                &json!({"action": "navigate", "url": "file:///etc/passwd"}),
                &cfg,
                &ctx,
                &verdict_allow(),
            ),
            PermissionLevel::Denied
        );
    }

    #[test]
    fn a_hook_ask_promotes_an_automatic_tool_into_a_prompt() {
        let ctx = dummy_ctx();
        let cfg = AgentConfig::default();
        assert_eq!(
            effective_permission(
                PermissionLevel::Auto,
                "read_file",
                &json!({"path": "a"}),
                &cfg,
                &ctx,
                &verdict_ask(),
            ),
            PermissionLevel::RequiresApproval
        );
    }

    #[test]
    fn a_hook_ask_cannot_un_deny_anything() {
        let ctx = dummy_ctx();
        let cfg = AgentConfig::default();
        assert_eq!(
            effective_permission(
                PermissionLevel::Denied,
                "edit_file",
                &json!({}),
                &cfg,
                &ctx,
                &verdict_ask(),
            ),
            PermissionLevel::Denied
        );
    }

    #[test]
    fn a_hook_ask_beats_yolo_mode() {
        // Otherwise a hook that exists to force a human decision would be
        // silently disabled by a switch in Settings.
        let ctx = dummy_ctx();
        let cfg = AgentConfig {
            yolo_mode: true,
            ..AgentConfig::default()
        };
        assert_eq!(
            effective_permission(
                PermissionLevel::RequiresApproval,
                "edit_file",
                &json!({"path": "a"}),
                &cfg,
                &ctx,
                &verdict_ask(),
            ),
            PermissionLevel::RequiresApproval
        );
    }

    #[test]
    fn a_stop_hook_cannot_block_forever() {
        // The cap is what stands between a badly written Stop hook and a
        // session that never ends.
        let mut guards = crate::agent::run_state::GuardState::default();
        assert!(!guards.stop_hook_active);
        for _ in 0..10 {
            if guards.stop_hook_blocks < MAX_STOP_HOOK_BLOCKS {
                guards.stop_hook_active = true;
                guards.stop_hook_blocks += 1;
            }
        }
        assert_eq!(guards.stop_hook_blocks, MAX_STOP_HOOK_BLOCKS);
    }

    fn dummy_ctx() -> ToolContext {
        ToolContext {
            hooks: None,
            records_cache: std::sync::Arc::new(std::sync::Mutex::new(LruCache::new(
                NonZeroUsize::new(8).unwrap(),
            ))),
            db_path: None,
            lsp_manager: None,
            workspace_root: None,
            embedding_model: Default::default(),
            session_store_path: None,
            limits: Default::default(),
            read_tracker: Default::default(),
            browser: None,
            interrupt: None,
            agent_config: None,
            plan_save_path: None,
            base_commit: None,
            auto_approve_git: false,
            shell_gate: None,
            mcp: None,
            mode_ctl: None,
            index_progress: None,
        }
    }

    #[test]
    fn force_explore_mode_rewrites_all_agents() {
        let input = json!({ "agents": [
            { "name": "a", "goal": "g", "mode": "code" },
            { "name": "b", "goal": "g", "mode": "explore" }
        ]});
        let out = force_explore_mode(input);
        for agent in out["agents"].as_array().unwrap() {
            assert_eq!(agent["mode"], "explore");
        }
    }

    #[test]
    fn force_explore_mode_normalizes_flattened_code_spec() {
        // A flattened single spec must not bypass the explore rewrite.
        let input = json!({ "name": "a", "goal": "g", "mode": "code" });
        let out = force_explore_mode(input);
        let agents = out["agents"].as_array().unwrap();
        assert_eq!(agents.len(), 1);
        assert_eq!(agents[0]["mode"], "explore");
    }

    #[test]
    fn ask_user_normalizes_object_options_to_label_description() {
        // The model often sends options as {label/value, description} objects
        // instead of plain strings — those become { label, description? }, with
        // plain strings promoted to { label } and the short key winning over
        // description for the label.
        let q = json!([{
            "question": "Pick one",
            "multi_select": false,
            "options": [
                {"value": "Integrar Mermaid", "description": "add support"},
                {"label": "So gerar .md"},
                "Plain string option"
            ]
        }]);
        let out = normalize_ask_user_questions(&q).expect("should normalize");
        let opts = out[0]["options"].as_array().unwrap();
        assert_eq!(
            opts[0],
            json!({"label": "Integrar Mermaid", "description": "add support"})
        );
        assert_eq!(opts[1], json!({"label": "So gerar .md"}));
        assert_eq!(opts[2], json!({"label": "Plain string option"}));
        assert_eq!(out[0]["question"], json!("Pick one"));
        assert_eq!(out[0]["multi_select"], json!(false));
    }

    #[test]
    fn ask_user_accepts_description_only_options() {
        // Regression for session b024da97: the model sent options carrying ONLY a
        // `description` (no label), which the old validator rejected 4× in a row
        // until the agent gave up. They must now normalize to { label: <description> }.
        // The payload is reproduced verbatim, Portuguese and all — normalization
        // must not depend on the language, and a translated fixture would no
        // longer be the input that actually broke.
        let q = json!([{
            "question": "O que voce quer dizer com \"lancar um novo patch em release\"?",
            "multi_select": false,
            "options": [
                {"description": "Commitar as mudanças pendentes, criar branch release/patch-x.y.z, tag semver e push — fluxo completo de release."},
                {"description": "Apenas criar/mover a tag de patch (ex.: v0.0.10 → v0.0.11) sem tocar nas mudanças não commitadas."},
                {"description": "Apenas subir a branch de hotfix/release para o remoto, sem tag nem commit adicional."},
                {"description": "Não é o Claudinio Code; você está no diretório errado. Cancele este patch."}
            ]
        }]);
        let out =
            normalize_ask_user_questions(&q).expect("description-only options must normalize");
        let opts = out[0]["options"].as_array().unwrap();
        assert_eq!(opts.len(), 4);
        assert_eq!(
            opts[0],
            json!({"label": "Commitar as mudanças pendentes, criar branch release/patch-x.y.z, tag semver e push — fluxo completo de release."})
        );
        assert_eq!(
            opts[3],
            json!({"label": "Não é o Claudinio Code; você está no diretório errado. Cancele este patch."})
        );
        // No stray `description` key when it was the source of the label.
        assert!(opts[0].get("description").is_none());
    }

    #[test]
    fn ask_user_rejects_empty_and_duplicate_options() {
        // Blank option.
        let blank = json!([{"question": "q", "options": ["ok", "  "]}]);
        assert!(normalize_ask_user_questions(&blank).is_err());
        // Duplicate options (case/whitespace-insensitive).
        let dup = json!([{"question": "q", "options": ["Yes", " yes "]}]);
        assert!(normalize_ask_user_questions(&dup).is_err());
        // Fewer than 2 options.
        let one = json!([{"question": "q", "options": ["only"]}]);
        assert!(normalize_ask_user_questions(&one).is_err());
        // Empty question text.
        let noq = json!([{"question": "  ", "options": ["a", "b"]}]);
        assert!(normalize_ask_user_questions(&noq).is_err());
        // Object option with no usable text key (label/value/title/description).
        let bad_obj = json!([{"question": "q", "options": [{"note": "x"}, "b"]}]);
        assert!(normalize_ask_user_questions(&bad_obj).is_err());
        // An empty object is likewise unusable.
        let empty_obj = json!([{"question": "q", "options": [{}, "b"]}]);
        assert!(normalize_ask_user_questions(&empty_obj).is_err());
    }

    #[test]
    fn ask_user_error_includes_contextual_example() {
        // Empty options should produce a clear error that echoes the model's own
        // question and shows a corrected, plain-string example — so it can fix
        // the call instead of looping on the same broken one.
        let raw = json!([{
            "question": "Rodar `claudinio` sem argumentos ja abre o Chat. O que voce quer?",
            "multi_select": false,
            "options": ["", "", ""]
        }]);
        let detail = normalize_ask_user_questions(&raw).unwrap_err();
        let msg = ask_user_error_with_example(&detail, &raw);
        assert!(msg.contains("options must not be empty"));
        // Echoes the real question text.
        assert!(msg.contains("O que voce quer?"));
        // Shows a concrete corrected example with plain-string options.
        assert!(msg.contains("First concrete choice"));
        assert!(msg.to_lowercase().contains("example"));
    }

    /// Answer the first approval request that appears with `answer`; report
    /// whether one appeared.
    fn answer_first(approvals: &ApprovalMap, answer: bool) -> tokio::task::JoinHandle<bool> {
        let approvals = approvals.clone();
        tokio::spawn(async move {
            for _ in 0..200 {
                let pending = {
                    let mut map = approvals.lock().await;
                    let key = map.keys().next().cloned();
                    key.and_then(|k| map.remove(&k))
                };
                if let Some(tx) = pending {
                    let _ = tx.send(answer);
                    return true;
                }
                tokio::time::sleep(std::time::Duration::from_millis(10)).await;
            }
            false
        })
    }

    async fn bash(ctx: &ToolContext, approvals: &ApprovalMap, command: &str) -> String {
        let events: Channel<AgentEvent> = Channel::new(|_| Ok(()));
        let block = run_tool(
            "bash",
            "t1",
            json!({ "command": command }),
            PermissionLevel::RequiresApproval,
            &events,
            approvals,
            &AnswerMap::default(),
            "s",
            ctx,
            &AgentConfig::default(),
            &verdict_none(),
        )
        .await;
        match block {
            ContentBlock::ToolResult { content, .. } => content.as_text().into_owned(),
            other => panic!("a tool result, got {other:?}"),
        }
    }

    // An unattended caller cannot let the allowlist decide for it: `echo` and
    // `node ` are allowlisted because a person sees them run. With a shell
    // gate every command reaches the approval channel, and a refusal says
    // what the gate says.
    #[tokio::test]
    async fn with_a_shell_gate_even_an_allowlisted_command_asks_first() {
        let dir = std::env::temp_dir().join(format!(
            "claudinio-shell-gate-{}-{}",
            std::process::id(),
            crate::agent::persist::now_ms()
        ));
        std::fs::create_dir_all(&dir).unwrap();
        let gated = ToolContext {
            workspace_root: Some(dir.to_string_lossy().to_string()),
            shell_gate: Some(crate::agent::tools::ShellGate {
                rejection: "Not run: use run_quality for checks.".into(),
            }),
            ..dummy_ctx()
        };

        let approvals = ApprovalMap::default();
        let asked = answer_first(&approvals, false);
        let refused = bash(&gated, &approvals, "echo gate-was-here").await;
        assert!(
            asked.await.unwrap(),
            "the command was put to the approval channel"
        );
        assert_eq!(refused, "Not run: use run_quality for checks.");

        let asked = answer_first(&approvals, true);
        let approved = bash(&gated, &approvals, "echo gate-was-here").await;
        assert!(asked.await.unwrap());
        assert!(approved.contains("gate-was-here"), "{approved}");

        // The denylist still wins: nothing is asked, nothing runs.
        let asked = answer_first(&approvals, true);
        let denied = bash(&gated, &approvals, "rm -rf /").await;
        assert!(denied.contains("blocked by security policy"), "{denied}");
        assert!(!asked.await.unwrap());

        // Without a gate the allowlist decides, as it always has.
        let open = ToolContext {
            shell_gate: None,
            ..gated.clone()
        };
        let asked = answer_first(&approvals, false);
        let ran = bash(&open, &approvals, "echo gate-was-here").await;
        assert!(ran.contains("gate-was-here"));
        assert!(!asked.await.unwrap());
        std::fs::remove_dir_all(&dir).ok();
    }
}
