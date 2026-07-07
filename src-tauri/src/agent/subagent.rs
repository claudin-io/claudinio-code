use crate::agent::permissions;
use crate::agent::provider::{self, AgentConfig, ContentBlock, Message, ToolDescription};
use crate::agent::session::{self, AgentEvent, ApprovalMap, AnswerMap, SteeringCtl};
use crate::agent::tools::{self, ToolContext, ToolDef};
use serde::Deserialize;
use serde_json::Value;
use std::sync::atomic::Ordering;
use std::sync::Arc;
use tauri::ipc::Channel;

pub const MAX_PARALLEL_AGENTS: usize = 4;

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
}

const TOOL_PREFERENCE: &str = "\
The workspace has a pre-indexed symbol database (FTS5). Before brute-forcing with grep or read_file, \
use these tools in this order of preference: \
  \u{2022} code_search  \u{2014} find any symbol/definition by name (faster than grep) \
  \u{2022} file_outline \u{2014} list all symbols in a file (preview structure before reading) \
  \u{2022} go_to_definition / find_references \u{2014} navigate symbol relationships precisely \
   \u{2022} symbol_lookup \u{2014} exact symbol name lookup across workspace \
   \u{2022} semantic_search \u{2014} search by concept/meaning using LateOn code embeddings. \
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
Work autonomously and efficiently: use the fewest tool calls that accomplish the goal. \
";

pub fn subagent_defs(mode: SubagentMode) -> Vec<ToolDef> {
    let mut tools: Vec<ToolDef> = tools::get_defs()
        .into_iter()
        .filter(|t| t.name != "spawn_agents" && t.name != "ask_user")
        .collect();
    match mode {
        SubagentMode::Explore => {
            tools.retain(|t| t.name != "edit_file" && t.name != "bash");
        }
        SubagentMode::Code => {}
    }
    tools
}

fn api_tools(mode: SubagentMode) -> Vec<ToolDescription> {
    subagent_defs(mode)
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
    Channel::new(move |body: tauri::ipc::InvokeResponseBody| -> tauri::Result<()> {
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
    })
}

/// Spawn 1-4 parallel subagents, each with their own fresh context.
/// Returns a combined ContentBlock with each agent's report and aggregated
/// token usage.
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
) -> (ContentBlock, u32, u32) {
    let agents = match tool_input.get("agents").and_then(|v| v.as_array()) {
        Some(a) if !a.is_empty() && a.len() <= MAX_PARALLEL_AGENTS => a,
        _ => {
            let msg = format!(
                "spawn_agents requires 1-{} agents in the 'agents' array",
                MAX_PARALLEL_AGENTS
            );
            let _ = event_tx.send(AgentEvent::ToolResult {
                tool_id: parent_tool_use_id.to_string(),
                tool_name: "spawn_agents".into(),
                output: msg.clone(),
                error: Some("invalid_input".into()),
            });
            return (ContentBlock::tool_result(parent_tool_use_id, &msg), 0, 0);
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
        return (ContentBlock::tool_result(parent_tool_use_id, &msg), 0, 0);
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

    for (i, handle) in handles.into_iter().enumerate() {
        let result = match handle.await {
            Ok(r) => r,
            Err(e) => SubagentResult {
                status: "failed",
                report: format!("Subagent panicked: {e}"),
                rounds: 0,
                in_tok: 0,
                out_tok: 0,
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
        });

        total_in += result.in_tok;
        total_out += result.out_tok;

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
    )
}

/// Run a single subagent: a simplified enxuto version of `run_workflow`.
/// No steering injection, no persistence, no `AgentEvent::Done`.
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
    let tools = api_tools(spec.mode);
    let skill_mgr = crate::agent::skills::SkillManager::new(
        ctx.workspace_root.as_ref().map(std::path::PathBuf::from)
    );
    let skills_section = crate::agent::skills::build_skills_system_prompt_section(&skill_mgr);
    let skills_hint = match &skills_section {
        Some(s) => format!("\n{s}"),
        None => String::new(),
    };
    let system = format!(
        "{SUBAGENT_SYSTEM_PROMPT}\n{TOOL_PREFERENCE}\n\nProject workspace root: {}.{}",
        ctx.workspace_root.as_deref().unwrap_or("(none)"),
        skills_hint,
    );

    let mut history = vec![Message {
        role: "user".into(),
        content: vec![ContentBlock::text(format!(
            "Your goal:\n{goal}\n\nExpected output:\n{expected}",
            goal = spec.goal,
            expected = spec.expected_output.as_deref().unwrap_or("A concise report")
        ))],
    }];

    let mut total_in = 0u32;
    let mut total_out = 0u32;
    let mut rounds: u32 = 0;
    let interrupt = &steering.interrupt;

    let sub_max = config.sub_max_rounds.unwrap_or(usize::MAX);
    for _ in 0..sub_max {
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
                };
            }
        };

        if let Some(u) = &stream_output.usage {
            total_in += u.input_tokens;
            total_out += u.output_tokens;
        }
        rounds += 1;

        if stream_output.interrupted {
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
            };
        }

        if stream_output.tool_uses.is_empty() {
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
            };
        }

        let mut tool_assistant_blocks: Vec<ContentBlock> = Vec::new();
        let mut tool_result_blocks: Vec<ContentBlock> = Vec::new();

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

            let tool_use_id = tool_use.get("id").and_then(|v| v.as_str()).unwrap_or("").to_string();
            let tool_name = tool_use.get("name").and_then(|v| v.as_str()).unwrap_or("").to_string();
            let tool_input = tool_use.get("input").cloned().unwrap_or(Value::Null);

            tool_assistant_blocks.push(ContentBlock::tool_use(
                &tool_use_id,
                &tool_name,
                tool_input.clone(),
            ));

            let perm = permissions::tool_permission(&tool_name);
            let block = session::run_tool(
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
            )
            .await;
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
    }

    SubagentResult {
        status: "max_rounds",
        report: format!("Reached {sub_max} rounds without completing."),
        rounds,
        in_tok: total_in,
        out_tok: total_out,
    }
}

/// Spawn a summary subagent that reads the session JSONL file and produces a
/// concise summary of the conversation. The subagent has a completely fresh
/// context — zero knowledge of the current conversation.
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
        goal: format!(
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
             Write in English. Be specific and factual — exact file paths, function names, values. \
             Keep it under 600 words. \
             Do NOT mention that you read a JSONL file or that you are a subagent — \
             just write the handoff as a direct description of the work.",
            jsonl_path, tail_note
        ),
        mode: SubagentMode::Explore,
        expected_output: Some(
            "A structured handoff summary (sections: task & intent, current state, pending/next \
             steps, files & symbols, decisions & constraints, learnings) under 600 words"
                .into(),
        ),
    };

    let result = run_subagent(config, ctx, &spec, event_tx, approvals, answers, session_id, steering).await;

    if result.status == "completed" {
        Ok(result.report)
    } else {
        Err(format!("Summary agent {}: {}", result.status, result.report))
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn test_subagent_defs_explore_excludes_spawn_and_ask() {
        let defs = subagent_defs(SubagentMode::Explore);
        for d in &defs {
            assert_ne!(d.name, "spawn_agents", "explore should not include spawn_agents");
            assert_ne!(d.name, "ask_user", "explore should not include ask_user");
        }
    }

    #[test]
    fn test_subagent_defs_explore_excludes_edit_and_bash() {
        let defs = subagent_defs(SubagentMode::Explore);
        for d in &defs {
            assert_ne!(d.name, "edit_file", "explore should not include edit_file");
            assert_ne!(d.name, "bash", "explore should not include bash");
        }
    }

    #[test]
    fn test_subagent_defs_code_excludes_spawn_and_ask() {
        let defs = subagent_defs(SubagentMode::Code);
        for d in &defs {
            assert_ne!(d.name, "spawn_agents", "code should not include spawn_agents");
            assert_ne!(d.name, "ask_user", "code should not include ask_user");
        }
    }

    #[test]
    fn test_subagent_defs_code_includes_edit_and_bash() {
        let defs = subagent_defs(SubagentMode::Code);
        let names: Vec<&str> = defs.iter().map(|d| d.name.as_str()).collect();
        assert!(names.contains(&"edit_file"), "code should include edit_file");
        assert!(names.contains(&"bash"), "code should include bash");
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
        assert!(MAX_PARALLEL_AGENTS >= 1);
        assert!(MAX_PARALLEL_AGENTS <= 4);
    }
}
