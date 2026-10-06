use crate::agent::permissions;
use crate::agent::provider::{self, AgentConfig, ContentBlock, Message, ToolDescription};
use crate::agent::session::{self, AgentEvent, AnswerMap, ApprovalMap, SteeringCtl};
use crate::agent::tools::{self, ToolContext, ToolDef};
use serde::Deserialize;
use serde_json::Value;
use std::sync::Arc;
use std::sync::atomic::Ordering;
use tauri::ipc::Channel;

pub const MAX_PARALLEL_AGENTS: usize = 4;
pub const MAX_PARALLEL_AGENTS_CAP: usize = 8;

// Invariants the rest of the module relies on, checked at compile time so a
// bad edit fails the build rather than a test.
const _: () = assert!(MAX_PARALLEL_AGENTS >= 1);
const _: () = assert!(MAX_PARALLEL_AGENTS <= MAX_PARALLEL_AGENTS_CAP);

/// Resolve the effective max-parallel-agents limit from config, clamped to
/// [1, MAX_PARALLEL_AGENTS_CAP]. Returns `MAX_PARALLEL_AGENTS` (4) when
/// the config field is `None`.
pub fn effective_max_parallel(config: &AgentConfig) -> usize {
    config
        .max_parallel_agents
        .unwrap_or(MAX_PARALLEL_AGENTS)
        .clamp(1, MAX_PARALLEL_AGENTS_CAP)
}

#[derive(Deserialize, Clone, Copy)]
#[serde(rename_all = "lowercase")]
pub enum SubagentMode {
    Explore,
    Code,
}

#[derive(Deserialize, Clone)]
pub struct SubagentSpec {
    pub name: String,
    pub goal: String,
    pub mode: SubagentMode,
    pub expected_output: Option<String>,
}

pub struct SubagentResult {
    pub status: &'static str,
    pub report: String,
    pub rounds: u32,
    pub in_tok: u32,
    pub out_tok: u32,
    pub cost: f64,
    /// Tool name → calls made. A subagent keeps no transcript, so this is all
    /// that survives of how it worked.
    pub tools: std::collections::BTreeMap<String, u32>,
}

const TOOL_PREFERENCE: &str = "\
The workspace has a pre-indexed symbol database (FTS5). Before brute-forcing with grep or read_file, \
use these tools in this order of preference: \
  \u{2022} code_search  \u{2014} find any symbol/definition by name (faster than grep) \
  \u{2022} file_outline \u{2014} list all symbols in a file (preview structure before reading) \
  \u{2022} go_to_definition / find_references \u{2014} navigate symbol relationships precisely \
   \u{2022} symbol_lookup \u{2014} exact symbol name lookup across workspace \
   \u{2022} semantic_search \u{2014} search by concept/meaning using code & doc embeddings. \
Describe what the code does in natural language: \
'message queue system' finds SteeringCtl.queue/drain/push without \
identifier match. Use this BEFORE grep when you can describe behavior \
but don't know symbol names. \
Accuracy hierarchy: LSP tools (precise) \u{2192} semantic_search (conceptual) \u{2192} \
code_search (keyword) \u{2192} grep/bash (fallback). \
Use grep only when the index doesn't cover what you need. \
Example: to understand an unfamiliar file, call file_outline first, not read_file.";

pub const SUBAGENT_SYSTEM_PROMPT: &str = "\
You are a subagent of Claudinio, an AI coding agent, running inside a larger task. \
You were spawned with a specific goal, given in the first user message. \
You cannot interact with the user: never ask questions, never wait for input — if information \
is missing, state your assumption and proceed, or report what is missing. \
Work autonomously and efficiently: use the fewest tool calls that accomplish the goal.\
";

pub fn subagent_defs(
    mode: SubagentMode,
    mcp_defs: &[ToolDef],
    max_parallel: usize,
    config: &AgentConfig,
) -> Vec<ToolDef> {
    let mut tools: Vec<ToolDef> = tools::get_defs(max_parallel)
        .into_iter()
        .filter(|t| t.name != "spawn_agents" && t.name != "ask_user")
        .collect();
    // A subagent runs on the builder model, so that model's surface is its own.
    crate::agent::surface::retain_for(config.tool_surface_for(&config.builder_model), &mut tools);
    tools.retain(|t| t.name != "web_search" || config.is_claudinio_account());
    match mode {
        // Explore subagents are read-only by design (no edit_file/bash);
        // MCP tools have unknown, potentially non-read-only side effects, so
        // they're excluded here for the same reason.
        SubagentMode::Explore => {
            tools.retain(|t| t.name != "edit_file" && t.name != "bash");
        }
        SubagentMode::Code => {
            tools.extend(mcp_defs.iter().cloned());
        }
    }
    tools
}

fn api_tools(
    mode: SubagentMode,
    mcp_defs: &[ToolDef],
    config: &AgentConfig,
) -> Vec<ToolDescription> {
    subagent_defs(mode, mcp_defs, MAX_PARALLEL_AGENTS, config)
        .iter()
        .map(|t| ToolDescription {
            name: t.name.clone(),
            description: t.description.clone(),
            input_schema: t.input_schema.clone(),
        })
        .collect()
}

/// Create a channel that wraps every event from the subagent into
/// `AgentEvent::Subagent { subagent_id, event }` and forwards it to the parent
/// channel. The subagent sends events to this wrapped channel; the Tauri IPC
/// serializes them, the callback catches them, wraps them, and re-sends on the
/// parent channel that goes to the frontend.
fn wrap_channel(parent: &Channel<AgentEvent>, subagent_id: &str) -> Channel<AgentEvent> {
    let sid = subagent_id.to_string();
    let parent = parent.clone();
    Channel::new(
        move |body: tauri::ipc::InvokeResponseBody| -> tauri::Result<()> {
            let json_str = match &body {
                tauri::ipc::InvokeResponseBody::Json(s) => s.clone(),
                _ => return Ok(()),
            };
            if let Ok(event) = serde_json::from_str::<AgentEvent>(&json_str) {
                let _ = parent.send(AgentEvent::Subagent {
                    subagent_id: sid.clone(),
                    event: Box::new(event),
                });
            }
            Ok(())
        },
    )
}

/// Leniency: models trained on per-agent tools sometimes flatten a single
/// agent spec to the top level. If the input lacks an 'agents' array but
/// looks like one spec (has a string 'goal'), wrap it into {"agents": [input]}
/// instead of failing the round.
pub(crate) fn normalize_spawn_input(tool_input: Value) -> Value {
    if tool_input.get("agents").is_none()
        && tool_input.get("goal").and_then(|v| v.as_str()).is_some()
    {
        serde_json::json!({ "agents": [tool_input] })
    } else {
        tool_input
    }
}

/// Spawn 1-4 parallel subagents, each with their own fresh context.
/// Returns a combined ContentBlock with each agent's report and aggregated
/// token usage.
#[allow(clippy::too_many_arguments)]
pub async fn run_spawn_agents(
    config: &AgentConfig,
    ctx: &ToolContext,
    parent_tool_use_id: &str,
    tool_input: Value,
    event_tx: &Channel<AgentEvent>,
    approvals: &ApprovalMap,
    answers: &AnswerMap,
    session_id: &str,
    steering: &Arc<SteeringCtl>,
) -> (ContentBlock, u32, u32, f64) {
    let tool_input = normalize_spawn_input(tool_input);
    let max = effective_max_parallel(config);
    let agents = match tool_input.get("agents").and_then(|v| v.as_array()) {
        Some(a) if !a.is_empty() && a.len() <= max => a,
        other => {
            let msg = match other {
                Some(a) if a.len() > max => format!(
                    "spawn_agents got {} agents; the maximum is {max} per call. \
                     Nothing was spawned. Re-issue the call now with at most {max} \
                     agents and run the remainder as a follow-up wave — do not stop.",
                    a.len(),
                ),
                _ => format!(
                    "spawn_agents requires an 'agents' array with 1-{} specs: \
                     {{\"agents\": [{{\"name\", \"goal\", \"mode\", \"expected_output\"}}]}}",
                    max
                ),
            };
            let _ = event_tx.send(AgentEvent::ToolResult {
                tool_id: parent_tool_use_id.to_string(),
                tool_name: "spawn_agents".into(),
                output: msg.clone(),
                error: Some("invalid_input".into()),
            });
            return (
                ContentBlock::tool_result(parent_tool_use_id, &msg),
                0,
                0,
                0.0,
            );
        }
    };

    let specs: Vec<SubagentSpec> = agents
        .iter()
        .filter_map(|v| serde_json::from_value::<SubagentSpec>(v.clone()).ok())
        .collect();

    if specs.len() != agents.len() {
        let msg = "failed to parse one or more agent specs".to_string();
        let _ = event_tx.send(AgentEvent::ToolResult {
            tool_id: parent_tool_use_id.to_string(),
            tool_name: "spawn_agents".into(),
            output: msg.clone(),
            error: Some("parse_error".into()),
        });
        return (
            ContentBlock::tool_result(parent_tool_use_id, &msg),
            0,
            0,
            0.0,
        );
    }

    let parent_tx = event_tx.clone();
    let mut handles = Vec::new();

    for (i, spec) in specs.iter().enumerate() {
        let subagent_id = format!("{session_id}:sub:{parent_tool_use_id}:{i}");
        let spec = spec.clone();
        let cfg = config.clone();
        let ctx_clone = ctx.clone();
        let tx = wrap_channel(&parent_tx, &subagent_id);
        let appr = approvals.clone();
        let ans = answers.clone();
        let sid = subagent_id.clone();
        let steer = steering.clone();

        let _ = parent_tx.send(AgentEvent::SubagentStarted {
            subagent_id: subagent_id.clone(),
            parent_tool_id: parent_tool_use_id.to_string(),
            name: spec.name.clone(),
            goal: spec.goal.clone(),
            mode: match spec.mode {
                SubagentMode::Explore => "explore".into(),
                SubagentMode::Code => "code".into(),
            },
        });

        handles.push(tokio::spawn(async move {
            run_subagent(&cfg, &ctx_clone, &spec, &tx, &appr, &ans, &sid, &steer).await
        }));
    }

    let mut reports = Vec::new();
    let mut total_in = 0u32;
    let mut total_out = 0u32;
    let mut total_cost = 0.0f64;

    for (i, handle) in handles.into_iter().enumerate() {
        let result = match handle.await {
            Ok(r) => r,
            Err(e) => SubagentResult {
                status: "failed",
                report: format!("Subagent panicked: {e}"),
                rounds: 0,
                in_tok: 0,
                out_tok: 0,
                cost: 0.0,
                tools: Default::default(),
            },
        };

        let subagent_id = format!("{session_id}:sub:{parent_tool_use_id}:{i}");
        let _ = parent_tx.send(AgentEvent::SubagentDone {
            subagent_id: subagent_id.clone(),
            status: result.status.into(),
            rounds: result.rounds,
            input_tokens: result.in_tok,
            output_tokens: result.out_tok,
            report: result.report.clone(),
            cost: result.cost,
        });

        total_in += result.in_tok;
        total_out += result.out_tok;
        total_cost += result.cost;

        if let Some(path) = ctx.session_store_path.as_deref() {
            let store = crate::agent::persist::SessionStore { path: path.into() };
            store.try_append(&crate::agent::persist::SessionRecord::SubagentRun {
                name: specs.get(i).map(|s| s.name.clone()).unwrap_or_default(),
                mode: match specs.get(i).map(|s| s.mode) {
                    Some(SubagentMode::Code) => "code".into(),
                    _ => "explore".into(),
                },
                status: result.status.into(),
                rounds: result.rounds,
                input_tokens: result.in_tok,
                output_tokens: result.out_tok,
                cost: result.cost,
                tools: result.tools.clone(),
                ts: crate::agent::persist::now_ms(),
            });
            crate::agent::persist::invalidate_cache(&store.path, &ctx.records_cache);
        }

        reports.push(format!(
            "## {} — {} ({} rounds)\n{}\n---",
            specs.get(i).map(|s| s.name.as_str()).unwrap_or("?"),
            result.status,
            result.rounds,
            result.report
        ));
    }

    let combined = reports.join("\n\n");
    (
        ContentBlock::tool_result(parent_tool_use_id, &combined),
        total_in,
        total_out,
        total_cost,
    )
}

/// The system prompt every subagent runs with.
fn subagent_system_prompt(workspace_root: Option<&str>, skills_hint: &str) -> String {
    format!(
        "{SUBAGENT_SYSTEM_PROMPT}\n{TOOL_PREFERENCE}\n\nProject workspace root: {}.{}\
         \n\nLANGUAGE: any language is valid. Write your final report in the language of your goal.",
        workspace_root.unwrap_or("(none)"),
        skills_hint,
    )
}

/// Run a single subagent: a simplified enxuto version of `run_workflow`.
/// No steering injection, no persistence, no `AgentEvent::Done`.
#[allow(clippy::too_many_arguments)]
pub async fn run_subagent(
    config: &AgentConfig,
    ctx: &ToolContext,
    spec: &SubagentSpec,
    event_tx: &Channel<AgentEvent>,
    approvals: &ApprovalMap,
    answers: &AnswerMap,
    session_id: &str,
    steering: &Arc<SteeringCtl>,
) -> SubagentResult {
    let mcp_defs = ctx
        .mcp
        .as_ref()
        .map(|m| m.cached_defs())
        .unwrap_or_default();
    let tools = api_tools(spec.mode, &mcp_defs, config);
    let skill_mgr = crate::agent::skills::SkillManager::with_plugin_prefs(
        ctx.workspace_root.as_ref().map(std::path::PathBuf::from),
        &config.plugins,
    );
    let skills_section = crate::agent::skills::build_skills_system_prompt_section(&skill_mgr);
    let skills_hint = match &skills_section {
        Some(s) => format!("\n{s}"),
        None => String::new(),
    };
    let system = crate::agent::surface::prompt_for(
        config.tool_surface_for(&config.builder_model),
        subagent_system_prompt(ctx.workspace_root.as_deref(), &skills_hint),
    );

    // A subagent runs on the builder model whatever mode its parent is in, so
    // it gets a budget — and result caps — of its own rather than the
    // parent's: a Brain on a 200k model can spawn workers on a 32k one.
    let prefix_tokens = session::estimate_tokens(&[], &system, &tools);
    let budget = crate::agent::budget::ContextBudget::for_model(
        config,
        &config.builder_model,
        prefix_tokens,
    );
    let ctx = &ToolContext {
        limits: Arc::new(crate::agent::budget::RunLimits::for_budget(&budget)),
        ..ctx.clone()
    };
    if budget.prefix_overflows(prefix_tokens) {
        return SubagentResult {
            status: "failed",
            report: budget.refusal(prefix_tokens),
            rounds: 0,
            in_tok: 0,
            out_tok: 0,
            cost: 0.0,
            tools: Default::default(),
        };
    }

    let mut history = vec![Message {
        role: "user".into(),
        content: vec![ContentBlock::text(format!(
            "Your goal:\n{goal}\n\nExpected output:\n{expected}",
            goal = spec.goal,
            expected = spec
                .expected_output
                .as_deref()
                .unwrap_or("A concise report")
        ))],
    }];

    let mut total_in = 0u32;
    let mut total_out = 0u32;
    let mut total_cost = 0.0f64;
    let mut rounds: u32 = 0;
    let mut tool_calls: std::collections::BTreeMap<String, u32> = Default::default();
    // A Stop hook that always blocks would keep a subagent alive forever.
    // Bounded the same way the quality retries and the continuation judge are,
    // and for the same reason.
    let mut stop_hook_active = false;
    let mut stop_hook_blocks: u32 = 0;
    let interrupt = &steering.interrupt;

    // Network-indicator label: the subagent's goal, truncated for the tooltip.
    let net_detail = {
        let goal: String = spec.goal.chars().take(60).collect();
        if goal.len() < spec.goal.len() {
            format!("subagent · {goal}…")
        } else {
            format!("subagent · {goal}")
        }
    };

    let sub_max = config.sub_max_rounds.unwrap_or(usize::MAX);
    // Subagents run unbounded by default too, so they get the same cycle
    // breaker as the parent (see `agent::loop_watch`).
    let mut loop_watch = crate::agent::loop_watch::LoopWatch::default();
    for _ in 0..sub_max {
        // Nothing left to shed and the request cannot be sent: stop with a
        // report the parent can act on — narrow the goal, or split it —
        // instead of failing at the provider.
        if session::estimate_tokens(&history, &system, &tools) >= budget.ceiling {
            return SubagentResult {
                status: "context_full",
                report: format!(
                    "Stopped after {rounds} rounds: the conversation no longer fits this \
                     model's context window ({}k tokens). Split the goal into smaller, \
                     independent subagents.",
                    budget.window / 1000
                ),
                rounds,
                in_tok: total_in,
                out_tok: total_out,
                cost: total_cost,
                tools: tool_calls.clone(),
            };
        }

        let mut assistant_text = String::new();
        let stream_output = match provider::stream_message(
            config,
            &config.builder_model,
            &history,
            &tools,
            Some(system.as_str()),
            event_tx,
            session_id,
            &mut assistant_text,
            interrupt,
            false,
            &net_detail,
        )
        .await
        {
            Ok(o) => o,
            Err(e) => {
                return SubagentResult {
                    status: "failed",
                    report: format!("Provider error: {e}"),
                    rounds,
                    in_tok: total_in,
                    out_tok: total_out,
                    cost: total_cost,
                    tools: tool_calls.clone(),
                };
            }
        };

        if let Some(u) = &stream_output.usage {
            total_in += u.input_tokens;
            total_out += u.output_tokens;
            // Accumulate cost — prefer provider-reported, fall back to estimate
            if let Some(c) = u.cost {
                total_cost += c;
            }
        }
        rounds += 1;

        if stream_output.interrupted {
            let final_cost = if total_cost == 0.0 && (total_in > 0 || total_out > 0) {
                let est =
                    session::cost_breakdown_for(&config.builder_model, total_in, 0, total_out);
                est.input + est.output
            } else {
                total_cost
            };
            return SubagentResult {
                status: "interrupted",
                report: if assistant_text.is_empty() {
                    "Interrupted by user.".into()
                } else {
                    assistant_text
                },
                rounds,
                in_tok: total_in,
                out_tok: total_out,
                cost: final_cost,
                tools: tool_calls.clone(),
            };
        }

        if stream_output.tool_uses.is_empty() {
            // ── Hooks: SubagentStop ─────────────────────────────────────────
            //
            // Here rather than after `SubagentDone` in `run_spawn_agents`,
            // because a blocking SubagentStop has to have something left to
            // continue, and by the time the parent sees the result there is not.
            if let Some(h) = &ctx.hooks
                && stop_hook_blocks < session::MAX_STOP_HOOK_BLOCKS
            {
                let out = crate::agent::hooks::fire_subagent_stop(
                    h,
                    stop_hook_active,
                    session_id,
                    Some(event_tx),
                )
                .await;
                if let Some(reason) = out.blocking_message() {
                    stop_hook_active = true;
                    stop_hook_blocks += 1;
                    history.push(Message {
                        role: "user".into(),
                        content: vec![ContentBlock::text(format!(
                            "<hook-feedback>\n{reason}\n</hook-feedback>"
                        ))],
                    });
                    continue;
                }
            }

            let final_cost = if total_cost == 0.0 && (total_in > 0 || total_out > 0) {
                let est =
                    session::cost_breakdown_for(&config.builder_model, total_in, 0, total_out);
                est.input + est.output
            } else {
                total_cost
            };
            return SubagentResult {
                status: "completed",
                report: if assistant_text.is_empty() {
                    "(no output)".into()
                } else {
                    assistant_text
                },
                rounds,
                in_tok: total_in,
                out_tok: total_out,
                cost: final_cost,
                tools: tool_calls.clone(),
            };
        }

        let mut tool_assistant_blocks: Vec<ContentBlock> = Vec::new();
        let mut tool_result_blocks: Vec<ContentBlock> = Vec::new();
        let mut hook_tool_feedback: Vec<String> = Vec::new();

        if !assistant_text.is_empty() {
            tool_assistant_blocks.push(ContentBlock::text(&assistant_text));
            let _ = event_tx.send(AgentEvent::TextStep {
                text: assistant_text.clone(),
            });
        }

        for tool_use in &stream_output.tool_uses {
            if interrupt.load(Ordering::SeqCst) {
                let tid = tool_use.get("id").and_then(|v| v.as_str()).unwrap_or("");
                let tname = tool_use.get("name").and_then(|v| v.as_str()).unwrap_or("");
                tool_assistant_blocks.push(ContentBlock::tool_use(
                    tid,
                    tname,
                    tool_use.get("input").cloned().unwrap_or(Value::Null),
                ));
                let msg = "Interrupted by user — tool not executed.";
                let _ = event_tx.send(AgentEvent::ToolResult {
                    tool_id: tid.to_string(),
                    tool_name: tname.to_string(),
                    output: msg.into(),
                    error: None,
                });
                tool_result_blocks.push(ContentBlock::tool_result(tid, msg));
                break;
            }

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
            let watched_input = tool_input.clone();
            *tool_calls.entry(tool_name.clone()).or_default() += 1;

            tool_assistant_blocks.push(ContentBlock::tool_use(
                &tool_use_id,
                &tool_name,
                tool_input.clone(),
            ));

            // Hooks apply to a subagent's tool calls exactly as they do to the
            // parent's. A guard that only watched the main loop would be a
            // guard the model can walk around by delegating.
            let hook_input = tool_input.clone();
            let hook_pre = match &ctx.hooks {
                Some(h) => {
                    crate::agent::hooks::fire_pre_tool_use(
                        h,
                        &tool_name,
                        &hook_input,
                        Some(event_tx),
                    )
                    .await
                }
                None => crate::agent::hooks::BatchOutcome::default(),
            };

            let perm = permissions::tool_permission(&tool_name);
            let block =
                if let crate::agent::hooks::PreToolVerdict::Deny { reason } = &hook_pre.verdict {
                    session::deny_tool(
                        &tool_name,
                        &tool_use_id,
                        &hook_input,
                        reason,
                        event_tx,
                        session_id,
                    )
                } else {
                    session::run_tool(
                        &tool_name,
                        &tool_use_id,
                        tool_input,
                        perm,
                        event_tx,
                        approvals,
                        answers,
                        session_id,
                        ctx,
                        config,
                        &hook_pre.verdict,
                    )
                    .await
                };

            if let Some(h) = &ctx.hooks
                && !matches!(
                    hook_pre.verdict,
                    crate::agent::hooks::PreToolVerdict::Deny { .. }
                )
            {
                let response = session::tool_response_for_hook(&block);
                let out = crate::agent::hooks::fire_post_tool_use(
                    h,
                    &tool_name,
                    &hook_input,
                    &response,
                    Some(event_tx),
                )
                .await;
                if let Some(msg) = out.blocking_message() {
                    hook_tool_feedback.push(msg);
                }
                if let Some(text) = out.context() {
                    hook_tool_feedback.push(text);
                }
            }

            if let ContentBlock::ToolResult { content, .. } = &block {
                loop_watch.record(&tool_name, &watched_input, &content.as_text());
            }
            tool_result_blocks.push(block);
        }

        history.push(Message {
            role: "assistant".into(),
            content: tool_assistant_blocks,
        });
        history.push(Message {
            role: "user".into(),
            content: tool_result_blocks,
        });
        if !hook_tool_feedback.is_empty() {
            let text = hook_tool_feedback.join("\n\n");
            history.push(Message {
                role: "user".into(),
                content: vec![ContentBlock::text(format!(
                    "<hook-feedback>\n{text}\n</hook-feedback>"
                ))],
            });
        }

        // Subagents have no handoff and no summary: past the soft line their
        // old tool traffic is dropped verbatim-style (`agent::prune`), in
        // memory — a subagent has no session file to record it in. Nothing
        // gentler comes after this, so it runs as a last resort: by age even
        // when Jev judged the history and that was not enough, and dropping
        // the oldest calls outright when cutting their results is not enough
        // either.
        let limit = budget.soft;
        let estimate = session::estimate_tokens(&history, &system, &tools);
        if estimate >= limit {
            let backend = crate::agent::jev::backend(config);
            let shed = crate::agent::prune::shed(
                &history,
                backend.as_ref(),
                session::shed_options(estimate, limit, &budget, true),
                |outcome, pruned| {
                    session::accept_prune(
                        outcome,
                        session::estimate_tokens(pruned, &system, &tools),
                        limit,
                        budget.prune_floor,
                    )
                },
            )
            .await;
            total_cost += shed.cost;
            if let Some((_, pruned)) = shed.accepted {
                history = pruned;
            }
        }

        let (loop_action, loop_cost) =
            session::loop_verdict(&mut loop_watch, crate::agent::jev::backend(config).as_ref())
                .await;
        total_cost += loop_cost;
        match loop_action {
            crate::agent::loop_watch::LoopAction::None => {}
            crate::agent::loop_watch::LoopAction::Nudge(msg) => {
                // Into the user turn that carries the tool results, so the
                // roles keep alternating.
                match history.last_mut() {
                    Some(last) if last.role == "user" => last.content.push(ContentBlock::text(msg)),
                    _ => history.push(Message {
                        role: "user".into(),
                        content: vec![ContentBlock::text(msg)],
                    }),
                }
            }
            crate::agent::loop_watch::LoopAction::Stop(msg) => {
                return SubagentResult {
                    status: "failed",
                    report: msg,
                    rounds,
                    in_tok: total_in,
                    out_tok: total_out,
                    cost: total_cost,
                    tools: tool_calls.clone(),
                };
            }
        }
    }

    SubagentResult {
        status: "max_rounds",
        report: format!("Reached {sub_max} rounds without completing."),
        rounds,
        in_tok: total_in,
        out_tok: total_out,
        cost: total_cost,
        tools: tool_calls.clone(),
    }
}

/// The goal handed to the summarizer that writes a context handoff.
fn summary_goal(jsonl_path: &str, tail_note: &str) -> String {
    format!(
        "Read the conversation session file at `{}` and produce a structured handoff summary \
         so the agent can seamlessly CONTINUE the work with your summary as its only memory \
         of the earlier conversation. \
         Use `read_file` to read it. The file is in JSONL format: one JSON object per line, each with a \
         `kind` field. Lines with kind=\"user\" contain the user's input in the `text` field. \
         Lines with kind=\"turn\" contain messages sent to/received from the AI — look for role=\"user\" \
         and role=\"assistant\" messages in the `message.content` field. \
         Lines with kind=\"steering\" contain mid-conversation guidance. \
         Lines with kind=\"compacted\" contain previous compactions (summaries of older conversation) — \
         fold their content into yours so nothing is lost. \
         Ignore kind=\"done\" and kind=\"status\" lines (token bookkeeping).\n{}\
         \n\
         Structure the summary with exactly these sections:\n\
         1. Task & intent — what the user is trying to achieve overall.\n\
         2. Current state — what is done and what is in progress right now.\n\
         3. Pending / next steps — an explicit TODO list to continue seamlessly.\n\
         4. Files & symbols touched — file paths, key functions/structs, and what changed in each.\n\
         5. Decisions & constraints — choices made, approaches rejected, user preferences.\n\
         6. Learnings — errors hit, environment quirks, commands that work.\n\
         \n\
         Weight recent messages most heavily — they describe the live state. \
         Write in the language the conversation is in. Be specific and factual — exact file paths, function names, values. \
         Keep it under 600 words. \
         Do NOT mention that you read a JSONL file or that you are a subagent — \
         just write the handoff as a direct description of the work.",
        jsonl_path, tail_note
    )
}

/// Spawn a summary subagent that reads the session JSONL file and produces a
/// concise summary of the conversation. The subagent has a completely fresh
/// context — zero knowledge of the current conversation.
#[allow(clippy::too_many_arguments)]
pub async fn run_summary_agent(
    config: &AgentConfig,
    ctx: &ToolContext,
    jsonl_path: &str,
    tail_turns: usize,
    event_tx: &Channel<AgentEvent>,
    approvals: &ApprovalMap,
    answers: &AnswerMap,
    session_id: &str,
    steering: &Arc<SteeringCtl>,
) -> Result<String, String> {
    let tail_note = if tail_turns > 0 {
        format!(
            "\nNote: the last {tail_turns} \"turn\" records of the file will be kept verbatim \
             in the live context alongside your summary, so do NOT waste words re-describing \
             those final exchanges — summarize everything BEFORE them.\n"
        )
    } else {
        String::new()
    };
    let spec = SubagentSpec {
        name: "summarizer".into(),
        goal: summary_goal(jsonl_path, &tail_note),
        mode: SubagentMode::Explore,
        expected_output: Some(
            "A structured handoff summary (sections: task & intent, current state, pending/next \
             steps, files & symbols, decisions & constraints, learnings) under 600 words"
                .into(),
        ),
    };

    let result = run_subagent(
        config, ctx, &spec, event_tx, approvals, answers, session_id, steering,
    )
    .await;

    if result.status == "completed" {
        Ok(result.report)
    } else {
        Err(format!(
            "Summary agent {}: {}",
            result.status, result.report
        ))
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::agent::provider::AgentConfig;

    // Any language is valid for the model's input and output. A subagent must
    // never be told to think, call tools or report in one language only.
    #[test]
    fn subagent_prompts_do_not_impose_a_language() {
        let system = subagent_system_prompt(Some("/ws"), "");
        let goal = summary_goal("/ws/s.jsonl", "");
        for text in [&system, &goal] {
            assert!(!text.contains("English ONLY"), "got: {text}");
            assert!(!text.contains("Write in English"), "got: {text}");
        }
    }

    // A subagent runs on the builder model: when that model is lean, so is it —
    // in what it is offered and in what its prompt sends it after.
    #[test]
    fn a_lean_builder_model_makes_its_subagents_lean() {
        use crate::agent::surface::{LEAN_DROPPED, ToolSurface, prompt_for};
        let mut config = AgentConfig::default();
        let full = subagent_defs(SubagentMode::Code, &[], MAX_PARALLEL_AGENTS, &config);
        config
            .tool_surface
            .insert(config.builder_model.clone(), "lean".into());
        for mode in [SubagentMode::Explore, SubagentMode::Code] {
            let defs = subagent_defs(mode, &[], MAX_PARALLEL_AGENTS, &config);
            for tool in LEAN_DROPPED {
                assert!(!defs.iter().any(|d| d.name == tool), "{tool}");
            }
            assert!(defs.iter().any(|d| d.name == "semantic_search"));
        }
        // Setting it on the brain model alone changes nothing for a subagent.
        let mut brain_only = AgentConfig::default();
        brain_only
            .tool_surface
            .insert(brain_only.brain_model.clone(), "lean".into());
        let defs = subagent_defs(SubagentMode::Code, &[], MAX_PARALLEL_AGENTS, &brain_only);
        assert_eq!(defs.len(), full.len());

        let prompt = subagent_system_prompt(Some("/ws"), "");
        assert!(prompt.contains("symbol_lookup") && prompt.contains("LSP tools"));
        let lean = prompt_for(ToolSurface::Lean, prompt);
        for tool in LEAN_DROPPED {
            assert!(!lean.contains(tool), "the lean prompt still names {tool}");
        }
        assert!(!lean.contains("LSP"), "{lean}");
        assert!(lean.contains("file_outline") && lean.contains("semantic_search"));
        assert!(
            lean.contains("before reading) \u{2022} semantic_search"),
            "{lean}"
        );
    }

    #[test]
    fn test_subagent_defs_explore_excludes_spawn_and_ask() {
        let config = AgentConfig::default();
        let defs = subagent_defs(SubagentMode::Explore, &[], MAX_PARALLEL_AGENTS, &config);
        for d in &defs {
            assert_ne!(
                d.name, "spawn_agents",
                "explore should not include spawn_agents"
            );
            assert_ne!(d.name, "ask_user", "explore should not include ask_user");
        }
    }

    #[test]
    fn test_subagent_defs_explore_excludes_edit_and_bash() {
        let config = AgentConfig::default();
        let defs = subagent_defs(SubagentMode::Explore, &[], MAX_PARALLEL_AGENTS, &config);
        for d in &defs {
            assert_ne!(d.name, "edit_file", "explore should not include edit_file");
            assert_ne!(d.name, "bash", "explore should not include bash");
        }
    }

    #[test]
    fn test_subagent_defs_code_excludes_spawn_and_ask() {
        let config = AgentConfig::default();
        let defs = subagent_defs(SubagentMode::Code, &[], MAX_PARALLEL_AGENTS, &config);
        for d in &defs {
            assert_ne!(
                d.name, "spawn_agents",
                "code should not include spawn_agents"
            );
            assert_ne!(d.name, "ask_user", "code should not include ask_user");
        }
    }

    #[test]
    fn test_subagent_defs_code_includes_edit_and_bash() {
        let config = AgentConfig::default();
        let defs = subagent_defs(SubagentMode::Code, &[], MAX_PARALLEL_AGENTS, &config);
        let names: Vec<&str> = defs.iter().map(|d| d.name.as_str()).collect();
        assert!(
            names.contains(&"edit_file"),
            "code should include edit_file"
        );
        assert!(names.contains(&"bash"), "code should include bash");
    }

    #[test]
    fn test_normalize_spawn_input_wraps_flattened_spec() {
        // The exact failure shape from session 5fd0dd80: one agent's fields
        // at the top level, no 'agents' wrapper.
        let flat = serde_json::json!({
            "name": "logo-locator",
            "goal": "find the logo",
            "mode": "explore",
            "expected_output": "file:line"
        });
        let normalized = normalize_spawn_input(flat.clone());
        let agents = normalized.get("agents").and_then(|v| v.as_array()).unwrap();
        assert_eq!(agents.len(), 1);
        assert_eq!(agents[0], flat);
        let spec: SubagentSpec = serde_json::from_value(agents[0].clone()).unwrap();
        assert_eq!(spec.name, "logo-locator");
    }

    #[test]
    fn test_normalize_spawn_input_passes_through_valid_and_invalid() {
        // Proper shape is untouched
        let ok = serde_json::json!({ "agents": [{ "name": "a", "goal": "g", "mode": "explore" }] });
        assert_eq!(normalize_spawn_input(ok.clone()), ok);
        // No 'goal' and no 'agents' → left as-is so the caller errors with guidance
        let bad = serde_json::json!({ "name": "x" });
        assert_eq!(normalize_spawn_input(bad.clone()), bad);
    }

    #[test]
    fn test_subagent_spec_parse_valid() {
        let json = serde_json::json!({
            "name": "explorer",
            "goal": "find the main function",
            "mode": "explore",
            "expected_output": "file:line location"
        });
        let spec: SubagentSpec = serde_json::from_value(json).unwrap();
        assert_eq!(spec.name, "explorer");
        assert!(matches!(spec.mode, SubagentMode::Explore));
        assert_eq!(spec.expected_output, Some("file:line location".into()));
    }

    #[test]
    fn test_subagent_spec_parse_code_mode() {
        let json = serde_json::json!({
            "name": "coder",
            "goal": "fix the bug",
            "mode": "code"
        });
        let spec: SubagentSpec = serde_json::from_value(json).unwrap();
        assert_eq!(spec.name, "coder");
        assert!(matches!(spec.mode, SubagentMode::Code));
        assert!(spec.expected_output.is_none());
    }

    #[test]
    fn test_subagent_spec_parse_rejects_invalid_mode() {
        let json = serde_json::json!({
            "name": "bad",
            "goal": "test",
            "mode": "invalid"
        });
        assert!(serde_json::from_value::<SubagentSpec>(json).is_err());
    }

    #[test]
    fn test_max_parallel_constants() {
        // The constants themselves are asserted at compile time next to their
        // definitions; this covers the resolution logic around them.
        let default_cfg = AgentConfig::default();
        assert_eq!(effective_max_parallel(&default_cfg), 4);
        // effective_max_parallel with Some(8) -> 8
        let mut cfg = AgentConfig {
            max_parallel_agents: Some(8),
            ..Default::default()
        };
        assert_eq!(effective_max_parallel(&cfg), 8);
        // effective_max_parallel with Some(0) -> clamped to 1
        cfg.max_parallel_agents = Some(0);
        assert_eq!(effective_max_parallel(&cfg), 1);
        // effective_max_parallel with Some(99) -> clamped to 8
        cfg.max_parallel_agents = Some(99);
        assert_eq!(effective_max_parallel(&cfg), 8);
    }
}

/// End-to-end: a real subagent loop against a scripted model over HTTP, so the
/// context budget is exercised where it matters — in what is actually sent.
#[cfg(test)]
pub(crate) mod scripted_model {
    use serde_json::{Value, json};
    use std::sync::atomic::{AtomicUsize, Ordering};
    use std::sync::{Arc, Mutex};
    use tokio::io::{AsyncReadExt, AsyncWriteExt};
    use tokio::net::TcpListener;

    /// What the scripted model answers one request with.
    #[derive(Clone)]
    pub enum Reply {
        Tool { name: &'static str, input: Value },
        Text(&'static str),
    }

    fn sse(event: &str, data: Value) -> String {
        format!("event: {event}\ndata: {data}\n\n")
    }

    fn render(reply: &Reply, n: usize) -> String {
        let mut out = String::new();
        match reply {
            Reply::Tool { name, input } => {
                out += &sse(
                    "content_block_start",
                    json!({"index": 0, "content_block":
                        {"type": "tool_use", "id": format!("tu_{n}"), "name": name, "input": {}}}),
                );
                out += &sse(
                    "content_block_delta",
                    json!({"index": 0, "delta":
                        {"type": "input_json_delta", "partial_json": input.to_string()}}),
                );
                out += &sse("content_block_stop", json!({"index": 0}));
                out += &sse(
                    "message_delta",
                    json!({"delta": {"stop_reason": "tool_use"}, "usage": {"output_tokens": 20}}),
                );
            }
            Reply::Text(text) => {
                out += &sse(
                    "content_block_delta",
                    json!({"index": 0, "delta": {"type": "text_delta", "text": text}}),
                );
                out += &sse(
                    "message_delta",
                    json!({"delta": {"stop_reason": "end_turn"}, "usage": {"output_tokens": 5}}),
                );
            }
        }
        out
    }

    /// Serve `script` one reply per request (the last one repeats), speaking
    /// the Anthropic streaming protocol. Returns the base URL and the size in
    /// bytes of every request body received, in order.
    pub async fn spawn(script: Vec<Reply>) -> (String, Arc<Mutex<Vec<usize>>>) {
        let (base, sizes, _) = spawn_recording(script).await;
        (base, sizes)
    }

    /// `spawn`, also keeping every request body: for a test about what the
    /// model was sent rather than how much of it.
    pub async fn spawn_recording(
        script: Vec<Reply>,
    ) -> (String, Arc<Mutex<Vec<usize>>>, Arc<Mutex<Vec<String>>>) {
        let listener = TcpListener::bind("127.0.0.1:0").await.unwrap();
        let addr = listener.local_addr().unwrap();
        let sizes = Arc::new(Mutex::new(Vec::new()));
        let seen = sizes.clone();
        let bodies = Arc::new(Mutex::new(Vec::new()));
        let kept = bodies.clone();
        let next = Arc::new(AtomicUsize::new(0));
        tokio::spawn(async move {
            loop {
                let Ok((mut socket, _)) = listener.accept().await else {
                    break;
                };
                // The whole request has to be read: its size is the measurement,
                // and a body left unread resets the connection under the client.
                let mut raw = Vec::new();
                let mut chunk = [0u8; 16 * 1024];
                let body_len = loop {
                    let Ok(n) = socket.read(&mut chunk).await else {
                        break 0;
                    };
                    if n == 0 {
                        break 0;
                    }
                    raw.extend_from_slice(&chunk[..n]);
                    let Some(head_end) = raw.windows(4).position(|w| w == b"\r\n\r\n") else {
                        continue;
                    };
                    let head = String::from_utf8_lossy(&raw[..head_end]).to_ascii_lowercase();
                    let declared = head
                        .lines()
                        .find_map(|l| l.strip_prefix("content-length:"))
                        .and_then(|v| v.trim().parse::<usize>().ok())
                        .unwrap_or(0);
                    if raw.len() >= head_end + 4 + declared {
                        break declared;
                    }
                };
                seen.lock().unwrap().push(body_len);
                let body_start = raw.len().saturating_sub(body_len);
                kept.lock()
                    .unwrap()
                    .push(String::from_utf8_lossy(&raw[body_start..]).into_owned());
                let n = next.fetch_add(1, Ordering::SeqCst);
                let reply = script.get(n).or(script.last()).expect("a script");
                let body = render(reply, n);
                let resp = format!(
                    "HTTP/1.1 200 OK\r\nContent-Type: text/event-stream\r\n\
                     Content-Length: {}\r\nConnection: close\r\n\r\n{}",
                    body.len(),
                    body
                );
                let _ = socket.write_all(resp.as_bytes()).await;
                let _ = socket.flush().await;
            }
        });
        (format!("http://{addr}"), sizes, bodies)
    }
}

#[cfg(test)]
mod context_budget_tests {
    use super::scripted_model::{Reply, spawn};
    use super::*;
    use crate::agent::provider::ProviderEntry;
    use std::sync::Mutex as StdMutex;

    const WINDOW: u32 = 32_768;

    /// A provider called "stub" serving model "m" at `base_url`, with the
    /// window the user typed for it — or none, which is how every model was
    /// treated before windows were tracked.
    fn config_for(base_url: &str, context_window: Option<u32>) -> AgentConfig {
        let mut config = AgentConfig {
            builder_model: "stub/m".into(),
            ..AgentConfig::default()
        };
        config.providers.insert(
            "stub".into(),
            ProviderEntry {
                api_key: "k".into(),
                base_url: base_url.into(),
                protocol: "anthropic".into(),
                enabled_models: vec![],
                label: None,
                model_pricing: Default::default(),
                model_output_limits: [("m".to_string(), 8_192u32)].into_iter().collect(),
                custom: true,
                custom_models: vec!["m".into()],
                model_context_limits: Default::default(),
                context_window,
            },
        );
        config
    }

    /// A workspace holding one file far larger than a small window.
    fn workspace(tag: &str) -> (std::path::PathBuf, String) {
        let dir = std::env::temp_dir().join(format!(
            "claudinio-budget-{tag}-{}-{}",
            std::process::id(),
            crate::agent::persist::now_ms()
        ));
        std::fs::create_dir_all(&dir).unwrap();
        let file = dir.join("big.rs");
        let line = format!("// {}\n", "x".repeat(76));
        std::fs::write(&file, line.repeat(6_000)).unwrap();
        (dir, file.to_string_lossy().to_string())
    }

    /// `reads` different 400-line windows of the file, then a final answer.
    fn script(file: &str, reads: usize) -> Vec<Reply> {
        let mut script: Vec<Reply> = (0..reads)
            .map(|i| Reply::Tool {
                name: "read_file",
                input: serde_json::json!({
                    "path": file,
                    "start_line": 1 + i * 150,
                    "end_line": 400 + i * 150,
                }),
            })
            .collect();
        script.push(Reply::Text("done reading"));
        script
    }

    async fn run(config: &AgentConfig, root: &std::path::Path) -> SubagentResult {
        let ctx = ToolContext {
            workspace_root: Some(root.to_string_lossy().to_string()),
            ..crate::agent::tools::tests_support::ctx()
        };
        let spec = SubagentSpec {
            name: "reader".into(),
            goal: "read the file".into(),
            mode: SubagentMode::Explore,
            expected_output: None,
        };
        let events: Channel<AgentEvent> = Channel::new(|_| Ok(()));
        run_subagent(
            config,
            &ctx,
            &spec,
            &events,
            &ApprovalMap::default(),
            &AnswerMap::default(),
            "test-session",
            &Arc::new(SteeringCtl::new()),
        )
        .await
    }

    /// Tokens of the largest request sent, by the session's own chars/3 rule.
    fn largest_request_tokens(sizes: &StdMutex<Vec<usize>>) -> usize {
        sizes.lock().unwrap().iter().copied().max().unwrap_or(0) / 3
    }

    /// The bug this budget exists for: with the window unknown, the session
    /// sends a 32k model requests several times its size.
    #[tokio::test]
    async fn without_a_known_window_requests_outgrow_a_small_model() {
        let (root, file) = workspace("unknown");
        let (base, sizes) = spawn(script(&file, 10)).await;
        let result = run(&config_for(&base, None), &root).await;
        assert_eq!(result.status, "completed", "{}", result.report);
        assert!(
            largest_request_tokens(&sizes) > WINDOW as usize,
            "largest request: {} tokens",
            largest_request_tokens(&sizes)
        );
        std::fs::remove_dir_all(root).ok();
    }

    /// With the window known, nothing larger than it is ever sent — and with
    /// no Jev to ask, the oldest results are cut by age, so thirty reads of a
    /// file many times the window all happen and the run finishes.
    #[tokio::test]
    async fn with_a_known_window_no_request_exceeds_it_and_the_work_gets_done() {
        let (root, file) = workspace("known");
        let (base, sizes) = spawn(script(&file, 30)).await;
        let result = run(&config_for(&base, Some(WINDOW)), &root).await;
        assert!(
            largest_request_tokens(&sizes) < WINDOW as usize,
            "largest request: {} tokens",
            largest_request_tokens(&sizes)
        );
        assert_eq!(result.status, "completed", "{}", result.report);
        assert_eq!(result.report, "done reading");
        assert_eq!(result.tools.get("read_file").copied(), Some(30));
        // 30 tool rounds and the final answer.
        assert_eq!(sizes.lock().unwrap().len(), 31);
        std::fs::remove_dir_all(root).ok();
    }

    /// When there is nothing to shed — the goal alone does not fit — the
    /// subagent says so before sending anything.
    #[tokio::test]
    async fn a_goal_larger_than_the_window_is_refused_before_any_request() {
        let (root, file) = workspace("refused");
        let (base, sizes) = spawn(script(&file, 1)).await;
        let config = config_for(&base, Some(WINDOW));
        let ctx = ToolContext {
            workspace_root: Some(root.to_string_lossy().to_string()),
            ..crate::agent::tools::tests_support::ctx()
        };
        let spec = SubagentSpec {
            name: "reader".into(),
            goal: "x".repeat(120_000),
            mode: SubagentMode::Explore,
            expected_output: None,
        };
        let events: Channel<AgentEvent> = Channel::new(|_| Ok(()));
        let result = run_subagent(
            &config,
            &ctx,
            &spec,
            &events,
            &ApprovalMap::default(),
            &AnswerMap::default(),
            "test-session",
            &Arc::new(SteeringCtl::new()),
        )
        .await;
        assert_eq!(result.status, "context_full", "{}", result.report);
        assert!(result.report.contains("32k"), "{}", result.report);
        assert!(sizes.lock().unwrap().is_empty(), "nothing was sent");
        std::fs::remove_dir_all(root).ok();
    }
}
