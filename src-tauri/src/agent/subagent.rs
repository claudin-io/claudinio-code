use crate::agent::permissions;
use crate::agent::provider::{self, AgentConfig, ContentBlock, Message, ToolDescription};
use crate::agent::session::{self, AgentEvent, AnswerMap, ApprovalMap, SteeringCtl};
use crate::agent::tool_batch;
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
    /// Set by `run_spawn_agents` for an explore subagent: its system prompt
    /// asks for the three-section report the parent carries
    /// (`delegation::EXPLORE_REPORT_FORMAT`). Never read from the model's
    /// spec, and false for the compaction summarizer, which runs in explore
    /// mode but has a report shape of its own.
    #[serde(skip)]
    pub reports_to_parent: bool,
    /// A section of the session's living document this subagent writes
    /// (lean-harness Phase 4b, decision D9). Claimed for it when it is
    /// spawned; it gets `doc_write`/`doc_read` for that section only.
    #[serde(default)]
    pub section: Option<String>,
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
    // Never the three LSP tools (lean-harness Phase 2): a subagent defers
    // nothing, so what it is given it carries on every round, and those three
    // were called 12 times in 16,945. This is the Lean surface whatever the
    // builder model's setting, which can only ever have asked for the same.
    crate::agent::surface::retain_for(crate::agent::surface::ToolSurface::Lean, &mut tools);
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

/// The stem prefix of a compaction summarizer's transcript in `<id>.sub/`
/// (`compact-<n>.jsonl`), where a spawned subagent's is its parent tool-use
/// id. Readers of the directory (`eval::metrics`, `scripts/session-stats.py`)
/// tell the two apart by it.
pub(crate) const COMPACTION: &str = "compact";

/// A subagent's transcript file (`persist::subagent_transcript_path`): its
/// turns and one `Usage` record per provider request.
///
/// Before this, a subagent kept nothing but a `subagent_run` summary in its
/// parent's file, so the share of a task's tokens spent re-exploring in a
/// fresh context — the cost the lean-harness plan's C3 is about — had never
/// been measured. Best-effort like every other session write: a transcript
/// that cannot be written never fails the subagent.
pub(crate) struct Transcript {
    store: crate::agent::persist::SessionStore,
    /// `<parent_tool_use_id>-<index>`, stamped on each `Usage` record.
    label: String,
}

impl Transcript {
    /// Open the transcript of subagent `index` of the `spawn_agents` call
    /// `parent_tool_use_id`, or `None` when the parent has no session file.
    pub(crate) fn open(
        ctx: &ToolContext,
        parent_tool_use_id: &str,
        index: usize,
        subagent_id: &str,
    ) -> Option<Self> {
        let parent = std::path::Path::new(ctx.session_store_path.as_deref()?);
        let path =
            crate::agent::persist::subagent_transcript_path(parent, parent_tool_use_id, index);
        Self::at(path, ctx, subagent_id)
    }

    /// Open a fresh transcript for a compaction's summarizer:
    /// `<id>.sub/compact-<n>.jsonl`, `n` the first number not yet taken.
    ///
    /// The summarizer is a subagent like any other — fresh context, its own
    /// provider requests, billed — and before it had a transcript its rounds
    /// were written nowhere, so a run that compacted reported less than the
    /// provider charged. A new file per attempt rather than one per
    /// `Compacted` record, because a failed attempt writes no `Compacted` and
    /// its requests were billed all the same.
    pub(crate) fn open_compaction(ctx: &ToolContext, summarizer_id: &str) -> Option<Self> {
        let parent = std::path::Path::new(ctx.session_store_path.as_deref()?);
        let path = (0..)
            .map(|n| crate::agent::persist::subagent_transcript_path(parent, COMPACTION, n))
            .find(|p| !p.exists())?;
        Self::at(path, ctx, summarizer_id)
    }

    fn at(path: std::path::PathBuf, ctx: &ToolContext, subagent_id: &str) -> Option<Self> {
        std::fs::create_dir_all(path.parent()?).ok()?;
        let label = path.file_stem()?.to_str()?.to_string();
        let store = crate::agent::persist::SessionStore { path };
        store.try_append(&crate::agent::persist::SessionRecord::Meta {
            session_id: subagent_id.to_string(),
            created_at: crate::agent::persist::now_ms(),
            workspace: ctx.workspace_root.clone(),
        });
        Some(Self { store, label })
    }

    fn turn(&self, message: &Message) {
        self.store
            .try_append(&crate::agent::persist::SessionRecord::Turn {
                message: message.clone(),
                ts: crate::agent::persist::now_ms(),
            });
    }

    fn usage(&self, round: u32, model: &str, usage: Option<&provider::Usage>) {
        self.store
            .try_append(&crate::agent::persist::SessionRecord::usage(
                round,
                model,
                usage,
                Some(self.label.clone()),
            ));
    }
}

/// Record `message` in the transcript, if there is one, and push it.
fn push_recorded(history: &mut Vec<Message>, transcript: Option<&Transcript>, message: Message) {
    if let Some(t) = transcript {
        t.turn(&message);
    }
    history.push(message);
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
        let msg = format!(
            "failed to parse one or more agent specs. Each spec needs a string 'name' (a short \
             label) and 'goal' (self-contained instructions: paths, symbols, constraints, exact \
             values), a 'mode' of 'explore' (read-only tools) or 'code' (can edit files and run \
             bash, with approval), and optionally 'expected_output'. All of them go in ONE call: \
             {{\"agents\": [{{\"name\", \"goal\", \"mode\", \"expected_output\"}}]}}, 1-{max} per call."
        );
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
    // Sections claimed for this call's subagents, by spec index: released
    // when each one ends (`doc_session::release`).
    let mut claimed: Vec<Option<(String, String)>> = vec![None; specs.len()];
    // Every code subagent of the call starts from the same facts.
    let brief = if specs.iter().any(|s| matches!(s.mode, SubagentMode::Code)) {
        crate::agent::delegation::brief_for(ctx).await
    } else {
        None
    };

    for (i, spec) in specs.iter().enumerate() {
        let subagent_id = format!("{session_id}:sub:{parent_tool_use_id}:{i}");
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
        // The timeline shows the goal the model wrote; the subagent gets it
        // behind the brief (code) or with the report format (explore).
        let mut spec = spec.clone();
        match spec.mode {
            SubagentMode::Explore => spec.reports_to_parent = true,
            SubagentMode::Code => {
                if let Some(brief) = &brief {
                    spec.goal = format!("{brief}\n\n{}", spec.goal);
                }
            }
        }
        // A section is claimed before the subagent starts; one that cannot
        // be (held by another agent, given twice in this call, no such id)
        // means this subagent does not run.
        if let Some(id) = spec.section.clone() {
            let owner = crate::agent::doc_session::subagent_owner(&spec.name);
            let twice = specs[..i].iter().any(|s| s.section.as_deref() == Some(&id));
            let claim = if twice {
                Err(format!("{id} is given to another agent in this call"))
            } else {
                crate::agent::doc_session::claim(ctx, &id, &owner)
            };
            match claim {
                Ok(topic) => {
                    crate::agent::doc_session::send_update(ctx, &parent_tx, session_id);
                    spec.goal
                        .push_str(&crate::agent::delegation::section_brief(&id, &topic));
                    spec.reports_to_parent = false;
                    claimed[i] = Some((id, owner));
                }
                Err(e) => {
                    let report = format!("not started: {e}. Two agents cannot hold one section.");
                    handles.push(tokio::spawn(async move {
                        SubagentResult {
                            status: "failed",
                            report,
                            rounds: 0,
                            in_tok: 0,
                            out_tok: 0,
                            cost: 0.0,
                            tools: Default::default(),
                        }
                    }));
                    continue;
                }
            }
        }

        let transcript = Transcript::open(ctx, parent_tool_use_id, i, &subagent_id);
        handles.push(tokio::spawn(async move {
            run_subagent_recorded(
                &cfg,
                &ctx_clone,
                &spec,
                &tx,
                &appr,
                &ans,
                &sid,
                &steer,
                transcript.as_ref(),
            )
            .await
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
        if let Some((id, owner)) = &claimed[i] {
            crate::agent::doc_session::release(ctx, id, owner);
            crate::agent::doc_session::send_update(ctx, &parent_tx, session_id);
        }
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

        // The timeline got the whole report above; the parent's context
        // carries at most `REPORT_CHARS` of it.
        reports.push(format!(
            "## {} — {} ({} rounds)\n{}\n---",
            specs.get(i).map(|s| s.name.as_str()).unwrap_or("?"),
            result.status,
            result.rounds,
            crate::agent::delegation::cap_report(&result.report)
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

/// The system prompt every subagent runs with; `report_format` adds the
/// report shape a spawned explore subagent hands back.
fn subagent_system_prompt(
    workspace_root: Option<&str>,
    skills_hint: &str,
    report_format: bool,
) -> String {
    // Lean, like the tool list: the prompt must not send a subagent after an
    // LSP tool it does not have (`subagent_defs`).
    crate::agent::surface::prompt_for(
        crate::agent::surface::ToolSurface::Lean,
        format!(
            "{SUBAGENT_SYSTEM_PROMPT}\n{TOOL_PREFERENCE}\n\nProject workspace root: {}.{}{}\
             \n\nLANGUAGE: any language is valid. Write your final report in the language of your goal.",
            workspace_root.unwrap_or("(none)"),
            skills_hint,
            if report_format {
                crate::agent::delegation::EXPLORE_REPORT_FORMAT
            } else {
                ""
            },
        ),
    )
}

/// Run a single subagent: a simplified enxuto version of `run_workflow`.
/// No steering injection, no `AgentEvent::Done`, and no transcript — see
/// `run_subagent_recorded` for the one `spawn_agents` and the compaction
/// summarizer keep. Test-only: every production subagent is a session's
/// provider requests, and those are always recorded.
#[cfg(test)]
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
    run_subagent_recorded(
        config, ctx, spec, event_tx, approvals, answers, session_id, steering, None,
    )
    .await
}

/// `run_subagent`, writing its turns and per-request usage to `transcript`.
#[allow(clippy::too_many_arguments)]
pub(crate) async fn run_subagent_recorded(
    config: &AgentConfig,
    ctx: &ToolContext,
    spec: &SubagentSpec,
    event_tx: &Channel<AgentEvent>,
    approvals: &ApprovalMap,
    answers: &AnswerMap,
    session_id: &str,
    steering: &Arc<SteeringCtl>,
    transcript: Option<&Transcript>,
) -> SubagentResult {
    let mcp_defs = ctx
        .mcp
        .as_ref()
        .map(|m| m.cached_defs())
        .unwrap_or_default();
    let mut tools = api_tools(spec.mode, &mcp_defs, config);
    // The section grant (decision D9): the two tools for one section.
    if spec.section.is_some() {
        tools.push(tools::doc::doc_write_def().into());
        tools.push(tools::doc::doc_read_def().into());
    }
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
        subagent_system_prompt(
            ctx.workspace_root.as_deref(),
            &skills_hint,
            spec.reports_to_parent,
        ),
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
    let limits = crate::agent::budget::RunLimits::for_budget(&budget);
    let ctx = &ToolContext {
        limits: Arc::new(limits),
        // Never the parent's `Main`: a subagent writes only a section it
        // was given, and without one none.
        doc_access: match &spec.section {
            Some(id) => crate::agent::doc_session::DocAccess::Section {
                id: id.clone(),
                owner: crate::agent::doc_session::subagent_owner(&spec.name),
            },
            None => crate::agent::doc_session::DocAccess::None,
        },
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

    let mut history = Vec::new();
    push_recorded(
        &mut history,
        transcript,
        Message {
            role: "user".into(),
            content: vec![ContentBlock::text(format!(
                "Your goal:\n{goal}\n\nExpected output:\n{expected}",
                goal = spec.goal,
                expected = spec
                    .expected_output
                    .as_deref()
                    .unwrap_or("A concise report")
            ))],
        },
    );

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
        // Everything that could be shed has been (the end of the previous
        // round takes any prune at all once past the hard line) and the
        // request still cannot be sent: stop with a report the parent can act
        // on — narrow the goal, or split it — instead of failing at the
        // provider.
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
        if let Some(t) = transcript {
            t.usage(rounds, &config.builder_model, stream_output.usage.as_ref());
        }

        if stream_output.interrupted {
            // What the model said before the stop is part of the conversation
            // the transcript records, as the main loop keeps it in its file.
            if let Some(t) = transcript
                && !assistant_text.is_empty()
            {
                t.turn(&Message {
                    role: "assistant".into(),
                    content: vec![ContentBlock::text(&assistant_text)],
                });
            }
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
            // The terminal turn joins the history before SubagentStop fires,
            // as it does in the main loop: a blocking hook's feedback answers
            // this turn, and the model has to see the turn it is answering for.
            if !assistant_text.is_empty() {
                push_recorded(
                    &mut history,
                    transcript,
                    Message {
                        role: "assistant".into(),
                        content: vec![ContentBlock::text(&assistant_text)],
                    },
                );
            }
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
                    push_recorded(
                        &mut history,
                        transcript,
                        Message {
                            role: "user".into(),
                            content: vec![ContentBlock::text(format!(
                                "<hook-feedback>\n{reason}\n</hook-feedback>"
                            ))],
                        },
                    );
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

        let tool_uses = &stream_output.tool_uses;
        // Only what the subagent was offered runs: an explore subagent is
        // read-only by its tool list, and Brain relies on that.
        let offered = |name: &str| tools.iter().any(|t| t.name == name);
        // Read-only calls run ahead together, as in the parent's loop.
        let mut ready: Vec<Option<ContentBlock>> = vec![None; tool_uses.len()];
        for (ti, tool_use) in tool_uses.iter().enumerate() {
            if ready[ti].is_none() && !interrupt.load(Ordering::SeqCst) {
                tool_batch::run_ahead(
                    tool_uses,
                    ti,
                    &mut ready,
                    |name, input| {
                        offered(name)
                            && tool_batch::runs_in_parallel(name, input, config, ctx, false)
                    },
                    tool_batch::executor(event_tx, session_id, ctx),
                    interrupt,
                    event_tx,
                )
                .await;
            }
            // Every call from here on is answered: a call a parallel run had
            // already finished keeps its result, the rest are reported as not
            // run.
            if interrupt.load(Ordering::SeqCst) {
                tool_batch::answer_unrun(
                    &tool_uses[ti..],
                    &mut ready[ti..],
                    "Interrupted by user — tool not executed.",
                    event_tx,
                    &mut tool_assistant_blocks,
                    &mut tool_result_blocks,
                    |name, input, block| {
                        *tool_calls.entry(name.to_string()).or_default() += 1;
                        if let ContentBlock::ToolResult { content, .. } = &block {
                            loop_watch.record(name, input, &content.as_text());
                        }
                        crate::agent::prune::history_copy(&history, name, input, block)
                    },
                );
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
                        &tool_use_id,
                        &hook_input,
                        Some(event_tx),
                    )
                    .await
                }
                None => crate::agent::hooks::BatchOutcome::default(),
            };

            let perm = permissions::tool_permission(&tool_name);
            let block = if let Some(block) = ready[ti].take() {
                block
            } else if !offered(&tool_name) {
                let names: Vec<&str> = tools.iter().map(|t| t.name.as_str()).collect();
                session::deny_tool(
                    &tool_name,
                    &tool_use_id,
                    &hook_input,
                    &format!(
                        "{tool_name} is not available to this subagent. Your tools are: {}.",
                        names.join(", ")
                    ),
                    event_tx,
                    session_id,
                )
            } else if let crate::agent::hooks::PreToolVerdict::Deny { reason } = &hook_pre.verdict {
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
                    &tool_use_id,
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
            tool_result_blocks.push(crate::agent::prune::history_copy(
                &history,
                &tool_name,
                &watched_input,
                block,
            ));
        }

        push_recorded(
            &mut history,
            transcript,
            Message {
                role: "assistant".into(),
                content: tool_assistant_blocks,
            },
        );
        let calls = history.last().map_or(&[][..], |m| m.content.as_slice());
        session::fit_round(&mut tool_result_blocks, calls, budget.round_chars);
        push_recorded(
            &mut history,
            transcript,
            Message {
                role: "user".into(),
                content: tool_result_blocks,
            },
        );
        if !hook_tool_feedback.is_empty() {
            let text = hook_tool_feedback.join("\n\n");
            push_recorded(
                &mut history,
                transcript,
                Message {
                    role: "user".into(),
                    content: vec![ContentBlock::text(format!(
                        "<hook-feedback>\n{text}\n</hook-feedback>"
                    ))],
                },
            );
        }

        // Subagents have no handoff and no summary: past the soft line their
        // old tool traffic is dropped verbatim-style (`agent::prune`), in
        // memory — a subagent has no session file to record it in. The same
        // ladder as the main session (lossless, then masking by age with Jev
        // arbitrating the newest rounds' cuts), run as a last resort: nothing
        // gentler comes after this, so the oldest calls are dropped outright
        // when cutting their results is not enough.
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
            let mut accepted = shed.accepted;
            // The rule above is about cost: a prune that frees little or lands
            // near the line is not worth the cached prefix it throws away —
            // for a session that has a handoff to fall back on. Past the hard
            // line a subagent has nothing to fall back on but stopping, so
            // there any prune that frees something is taken: the lossless
            // stage if it gets back under the line by itself, otherwise the
            // last thing the shed has to offer, however little. Jev was
            // already asked above.
            if accepted.is_none() && estimate >= budget.hard {
                accepted = crate::agent::prune::shed(
                    &history,
                    None,
                    session::shed_options(estimate, limit, &budget, true),
                    |outcome, pruned| {
                        let left = session::estimate_tokens(pruned, &system, &tools);
                        left < estimate && (left < limit || outcome.last)
                    },
                )
                .await
                .accepted;
            }
            if let Some((_, pruned)) = accepted {
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

/// The compaction summarizer's goal. Lean-harness Phase 6 made it a note
/// written for recall: the old "under 600 words" summary in six loose sections
/// dropped exactly what a successor then re-read files to recover — a path,
/// the line, the error text, what the user said in their own words. Anthropic
/// ("Effective context engineering for AI agents") tunes compaction prompts by
/// maximizing recall first and only then trimming; Manus keeps the errors in,
/// because a model that no longer sees a failure repeats it.
fn summary_goal(jsonl_path: &str, tail_note: &str) -> String {
    format!(
        "Read the conversation session file at `{}` and write the note the agent will \
         CONTINUE the work from: it becomes its only memory of the earlier conversation. \
         Use `read_file` to read it. The file is in JSONL format: one JSON object per line, each with a \
         `kind` field. Lines with kind=\"user\" contain the user's input in the `text` field. \
         Lines with kind=\"turn\" contain messages sent to/received from the AI — look for role=\"user\" \
         and role=\"assistant\" messages in the `message.content` field, and tool_result blocks for \
         what each tool returned (errors included). \
         Lines with kind=\"steering\" contain mid-conversation guidance. \
         Lines with kind=\"compacted\" contain previous compactions (notes on older conversation) — \
         fold their content into yours so nothing is lost. \
         Ignore kind=\"done\" and kind=\"status\" lines (token bookkeeping).\n{}\
         \n\
         Write the note in exactly these Markdown sections:\n\
         ## Goal — what the user is trying to achieve, and what done looks like.\n\
         ## Decisions (and why) — each choice made or approach rejected, with its reason.\n\
         ## Files touched — one line per file as `path:lines`, with what was read or changed \
         there and the key functions or types.\n\
         ## What failed — every error, failing test or refused command that mattered, with the \
         exact error text (quote it; do not paraphrase), and whether it is fixed.\n\
         ## Left to do — the next steps, in order, specific enough to start on without re-reading.\n\
         ## Constraints the user stated — quoted verbatim, one per line.\n\
         \n\
         Recall first: put in everything the work still depends on — paths, line numbers, names, \
         values, commands that worked, error messages — then cut only what is redundant or \
         superseded. Keep the errors in. Target at most ~1,500 words; never drop a fact the next \
         step needs to stay under it. Weight recent messages most heavily — they describe the \
         live state. Write in the language the conversation is in. \
         Do NOT mention that you read a JSONL file or that you are a subagent — \
         just write the note as a direct description of the work.",
        jsonl_path, tail_note
    )
}

/// The compaction summarizer: an explore subagent with [`summary_goal`],
/// told how many turns stay verbatim after its note.
fn summary_spec(jsonl_path: &str, tail_turns: usize) -> SubagentSpec {
    let tail_note = if tail_turns > 0 {
        format!(
            "\nNote: the last {tail_turns} \"turn\" records of the file will be kept verbatim \
             in the live context alongside your note, so do NOT waste words re-describing \
             those final exchanges — cover everything BEFORE them.\n"
        )
    } else {
        String::new()
    };
    SubagentSpec {
        name: "summarizer".into(),
        goal: summary_goal(jsonl_path, &tail_note),
        mode: SubagentMode::Explore,
        expected_output: Some(
            "A Markdown note in six sections (## Goal, ## Decisions (and why), ## Files touched, \
             ## What failed, ## Left to do, ## Constraints the user stated), recall first, at most \
             ~1,500 words"
                .into(),
        ),
        reports_to_parent: false,
        section: None,
    }
}

/// Spawn a summary subagent that reads the session JSONL file and writes the
/// compaction note ([`summary_goal`]). The subagent has a completely fresh
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
    let spec = summary_spec(jsonl_path, tail_turns);

    let transcript = Transcript::open_compaction(ctx, &format!("{session_id}-summarizer"));
    let result = run_subagent_recorded(
        config,
        ctx,
        &spec,
        event_tx,
        approvals,
        answers,
        session_id,
        steering,
        transcript.as_ref(),
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
        let system = subagent_system_prompt(Some("/ws"), "", false);
        let goal = summary_goal("/ws/s.jsonl", "");
        for text in [&system, &goal] {
            assert!(!text.contains("English ONLY"), "got: {text}");
            assert!(!text.contains("Write in English"), "got: {text}");
        }
    }

    // Lean-harness Phase 6: the compaction note is written for recall first
    // (Anthropic, "Effective context engineering": maximize recall, then trim
    // redundancy), keeps the errors in (Manus), and is structured so the
    // successor finds what it needs without re-reading files.
    #[test]
    fn the_compaction_note_asks_for_recall_first_in_six_sections() {
        let spec = summary_spec("/ws/s.jsonl", 0);
        let goal = &spec.goal;
        for section in [
            "## Goal",
            "## Decisions",
            "## Files touched",
            "## What failed",
            "## Left to do",
            "## Constraints the user stated",
        ] {
            assert!(goal.contains(section), "missing {section}: {goal}");
        }
        assert!(goal.contains("path:lines"), "{goal}");
        assert!(goal.contains("exact error text"), "{goal}");
        assert!(goal.contains("verbatim"), "{goal}");
        assert!(goal.contains("1,500 words"), "{goal}");
        assert!(
            !goal.contains("600 words"),
            "the old cap lost what recall needs"
        );
        let expected = spec.expected_output.as_deref().unwrap_or_default();
        assert!(!expected.contains("600 words"), "{expected}");
        assert!(expected.contains("## What failed"), "{expected}");
        assert!(!spec.reports_to_parent);
        assert!(!goal.contains("kept verbatim in the live context"));
        let with_tail = summary_spec("/ws/s.jsonl", 3);
        assert!(with_tail.goal.contains("the last 3 \"turn\" records"));
    }

    // Phase 2 of the lean-harness plan: subagents defer nothing (a fresh
    // context has no earlier load to build on, and a load round would be a
    // second round trip in every one of them), but the three LSP tools —
    // 12 calls in 16,945 measured — leave their list and their prompt. The
    // index searches stay: they are what an explorer reads with.
    #[test]
    fn subagents_never_get_the_three_lsp_tools_but_keep_the_index_searches() {
        use crate::agent::surface::LEAN_DROPPED;
        let config = AgentConfig::default();
        for (label, mode) in [
            ("explore", SubagentMode::Explore),
            ("code", SubagentMode::Code),
        ] {
            let defs = subagent_defs(mode, &[], MAX_PARALLEL_AGENTS, &config);
            let has = |n: &str| defs.iter().any(|d| d.name == n);
            for tool in LEAN_DROPPED {
                assert!(!has(tool), "{label} has {tool}");
                assert!(
                    defs.iter().all(|d| !d.description.contains(tool)),
                    "{label}: a description still points at {tool}"
                );
            }
            for keep in ["file_outline", "code_search", "semantic_search", "skills"] {
                assert!(has(keep), "{label} lacks {keep}");
            }
            assert!(!has("load_tools"), "{label} defers nothing");
        }
        let prompt = subagent_system_prompt(Some("/ws"), "", false);
        for tool in LEAN_DROPPED {
            assert!(!prompt.contains(tool), "the prompt still names {tool}");
        }
        assert!(prompt.contains("file_outline") && prompt.contains("semantic_search"));
    }

    // A subagent is lean whatever surface the user set for the builder model
    // it runs on (Phase 2): the three LSP tools are never offered, and the
    // index searches always are — a fresh context has nothing to load them on.
    #[test]
    fn a_subagent_is_lean_whatever_surface_its_builder_model_has() {
        use crate::agent::surface::{LEAN_DROPPED, ToolSurface, prompt_for};
        let names = |config: &AgentConfig, mode| -> Vec<String> {
            subagent_defs(mode, &[], MAX_PARALLEL_AGENTS, config)
                .into_iter()
                .map(|d| d.name)
                .collect()
        };
        let default = AgentConfig::default();
        let mut configs = Vec::new();
        for (on_brain, surface) in [(false, "full"), (false, "lean"), (true, "lean")] {
            let mut config = AgentConfig::default();
            let model = if on_brain {
                config.brain_model.clone()
            } else {
                config.builder_model.clone()
            };
            config.tool_surface.insert(model, surface.into());
            configs.push(config);
        }
        for mode in [SubagentMode::Explore, SubagentMode::Code] {
            let expected = names(&default, mode);
            for tool in LEAN_DROPPED {
                assert!(!expected.iter().any(|n| n == tool), "{tool}");
            }
            for tool in ["semantic_search", "code_search", "file_outline"] {
                assert!(expected.iter().any(|n| n == tool), "{tool}");
            }
            for config in &configs {
                assert_eq!(names(config, mode), expected, "{:?}", config.tool_surface);
            }
        }

        // The prompt is built lean since Phase 2; the rewrite is what does it.
        assert!(TOOL_PREFERENCE.contains("symbol_lookup") && TOOL_PREFERENCE.contains("LSP tools"));
        let prompt = subagent_system_prompt(Some("/ws"), "", false);
        let lean = prompt_for(ToolSurface::Lean, prompt.clone());
        assert_eq!(lean, prompt);
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
        Tool {
            name: &'static str,
            input: Value,
        },
        /// Several calls in one turn, as a model reading files in parallel makes.
        Tools(Vec<(&'static str, Value)>),
        Text(&'static str),
        /// The inner reply, sent after this many milliseconds: a slow
        /// provider, for tests about what happens while a request is in flight.
        Delayed(u64, Box<Reply>),
        /// This text, and then nothing: the response is never finished and the
        /// connection stays open, as a provider mid-generation does when the
        /// user presses stop.
        Hang(&'static str),
        /// `Hang`, with a thinking delta after the text: the client reports
        /// it as a `Thinking` event, which can only come once the text before
        /// it was read — a test's signal that the words are in.
        HangSignalled(&'static str),
        /// An HTTP error: this status and this JSON body, no stream. A
        /// provider refusing the request outright.
        Status(u16, &'static str),
    }

    /// Every model a scripted run can call — the builder, and the Brain the
    /// completion judge and Brain-mode turns run on — is served by the stub at
    /// `base`. A test that left one on its default would send its requests to
    /// the real provider and pass only because that call fails open.
    pub fn assert_only_the_stub_is_called(
        config: &crate::agent::provider::AgentConfig,
        base: &str,
    ) {
        for model in [&config.builder_model, &config.brain_model] {
            assert_eq!(config.resolve_provider(model).base_url, base, "{model}");
        }
    }

    fn sse(event: &str, data: Value) -> String {
        format!("event: {event}\ndata: {data}\n\n")
    }

    fn render(reply: &Reply, n: usize) -> String {
        let mut out = String::new();
        let tool_use = |index: usize, name: &str, input: &Value| {
            sse(
                "content_block_start",
                json!({"index": index, "content_block":
                    {"type": "tool_use", "id": format!("tu_{n}_{index}"), "name": name, "input": {}}}),
            ) + &sse(
                "content_block_delta",
                json!({"index": index, "delta":
                    {"type": "input_json_delta", "partial_json": input.to_string()}}),
            ) + &sse("content_block_stop", json!({"index": index}))
        };
        let stop = sse(
            "message_delta",
            json!({"delta": {"stop_reason": "tool_use"}, "usage": {"output_tokens": 20}}),
        );
        match reply {
            Reply::Delayed(_, inner) => return render(inner, n),
            Reply::Status(_, body) => return body.to_string(),
            Reply::Tool { name, input } => {
                out += &tool_use(0, name, input);
                out += &stop;
            }
            Reply::Tools(calls) => {
                for (index, (name, input)) in calls.iter().enumerate() {
                    out += &tool_use(index, name, input);
                }
                out += &stop;
            }
            Reply::Hang(text) => {
                out += &sse(
                    "content_block_delta",
                    json!({"index": 0, "delta": {"type": "text_delta", "text": text}}),
                );
            }
            Reply::HangSignalled(text) => {
                out += &render(&Reply::Hang(text), n);
                out += &sse(
                    "content_block_delta",
                    json!({"index": 1, "delta": {"type": "thinking_delta", "thinking": "."}}),
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
    /// bytes of every conversation request body received, in order (the
    /// completion judge's requests are answered, not recorded).
    pub async fn spawn(script: Vec<Reply>) -> (String, Arc<Mutex<Vec<usize>>>) {
        let (base, sizes, _) = spawn_recording(script).await;
        (base, sizes)
    }

    /// `spawn`, also keeping every request body: for a test about what the
    /// model was sent rather than how much of it.
    pub async fn spawn_recording(
        script: Vec<Reply>,
    ) -> (String, Arc<Mutex<Vec<usize>>>, Arc<Mutex<Vec<String>>>) {
        spawn_with_usage(script, None).await
    }

    /// `spawn_recording`, with every reply opening on a `message_start` that
    /// carries `usage` — the shape Anthropic reports input and cache tokens in.
    /// Opt-in, because a reported input size feeds the context meter and would
    /// move every test that measures it.
    pub async fn spawn_with_usage(
        script: Vec<Reply>,
        usage: Option<Value>,
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
                let body_start = raw.len().saturating_sub(body_len);
                let request = String::from_utf8_lossy(&raw[body_start..]);
                // A non-streaming request is the completion judge asking
                // whether a terminal turn is finished. It gets a plain JSON
                // "DONE", does not consume a scripted reply and is not
                // recorded — the script is the conversation, and the judge is
                // not part of it.
                let judge = request.contains("\"stream\":false");
                if !judge {
                    seen.lock().unwrap().push(body_len);
                    kept.lock().unwrap().push(request.clone().into_owned());
                }
                let (content_type, body) = if judge {
                    let mut message = json!({"content": [{"type": "text", "text": "DONE"}]});
                    if let Some(usage) = &usage {
                        message["usage"] = usage.clone();
                    }
                    ("application/json", message.to_string())
                } else {
                    let n = next.fetch_add(1, Ordering::SeqCst);
                    let reply = script.get(n).or(script.last()).expect("a script");
                    // A delayed reply is the inner one, late: a delayed hang
                    // still hangs.
                    let reply = match reply {
                        Reply::Delayed(ms, inner) => {
                            tokio::time::sleep(std::time::Duration::from_millis(*ms)).await;
                            &**inner
                        }
                        other => other,
                    };
                    let mut body = render(reply, n);
                    if let Some(usage) = &usage {
                        body = sse(
                            "message_start",
                            json!({"message": {"role": "assistant", "usage": usage}}),
                        ) + &body;
                    }
                    if let Reply::Status(code, body) = reply {
                        let resp = format!(
                            "HTTP/1.1 {code} Error\r\nContent-Type: application/json\r\n\
                             Content-Length: {}\r\nConnection: close\r\n\r\n{body}",
                            body.len()
                        );
                        let _ = socket.write_all(resp.as_bytes()).await;
                        let _ = socket.flush().await;
                        continue;
                    }
                    if let Reply::Hang(_) | Reply::HangSignalled(_) = reply {
                        // Promise more than is sent, then hold the line.
                        let head = format!(
                            "HTTP/1.1 200 OK\r\nContent-Type: text/event-stream\r\n\
                             Content-Length: {}\r\n\r\n",
                            body.len() + 1_000_000
                        );
                        let _ = socket.write_all(head.as_bytes()).await;
                        let _ = socket.write_all(body.as_bytes()).await;
                        let _ = socket.flush().await;
                        tokio::spawn(async move {
                            tokio::time::sleep(std::time::Duration::from_secs(30)).await;
                            drop(socket);
                        });
                        continue;
                    }
                    ("text/event-stream", body)
                };
                let resp = format!(
                    "HTTP/1.1 200 OK\r\nContent-Type: {content_type}\r\n\
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
            brain_model: "stub/m".into(),
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

    #[test]
    fn the_scripted_config_calls_no_model_but_the_stub() {
        let base = "http://127.0.0.1:9";
        super::scripted_model::assert_only_the_stub_is_called(&config_for(base, None), base);
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
            reports_to_parent: false,
            section: None,
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

    /// An explore subagent is read-only by its tool list, and Brain spawns
    /// only those; the loop ran whatever name the model sent, so an `echo >`
    /// (allowlisted by prefix) wrote a file unasked and `finalize_plan` wrote
    /// the plan log (final branch review). A tool it was not offered is refused.
    #[tokio::test]
    async fn an_explore_subagent_runs_no_tool_it_was_not_offered() {
        let (root, _file) = workspace("unoffered");
        let target = root.join("pwned.txt");
        let (base, _, bodies) = super::scripted_model::spawn_recording(vec![
            Reply::Tool {
                name: "bash",
                input: serde_json::json!({
                    "command": format!("echo pwned > {}", target.display()),
                }),
            },
            Reply::Tool {
                name: "edit_file",
                input: serde_json::json!({
                    "path": target.to_string_lossy(), "old_string": "", "new_string": "x\n",
                }),
            },
            Reply::Text("done"),
        ])
        .await;
        let result = run(&config_for(&base, None), &root).await;
        assert_eq!(result.status, "completed", "{}", result.report);
        assert!(!target.exists(), "an unoffered tool wrote a file");
        let bodies = bodies.lock().unwrap().clone();
        assert!(bodies[1].contains("bash is not available"), "{}", bodies[1]);
        assert!(
            bodies[2].contains("edit_file is not available"),
            "{}",
            bodies[2]
        );
        std::fs::remove_dir_all(&root).ok();
    }

    /// Reading the same lines twice: the second request carries the file, the
    /// third carries it once and a pointer — not twice.
    #[tokio::test]
    async fn a_repeated_read_is_carried_once() {
        let (root, file) = workspace("repeat");
        let read = Reply::Tool {
            name: "read_file",
            input: serde_json::json!({ "path": file, "start_line": 1, "end_line": 200 }),
        };
        let (base, _, bodies) =
            super::scripted_model::spawn_recording(vec![read.clone(), read, Reply::Text("done")])
                .await;
        let result = run(&config_for(&base, None), &root).await;
        assert_eq!(result.status, "completed", "{}", result.report);

        let bodies = bodies.lock().unwrap().clone();
        assert_eq!(bodies.len(), 3);
        let file_lines = |body: &str| body.matches(&"x".repeat(76)).count();
        assert_eq!(
            file_lines(&bodies[1]),
            200,
            "the first read is there in full"
        );
        assert_eq!(
            file_lines(&bodies[2]),
            200,
            "the repeat added no second copy"
        );
        assert!(bodies[2].contains(crate::agent::prune::UNCHANGED_MARK));
        assert!(bodies[2].len() < bodies[1].len() + 1_500);
        std::fs::remove_dir_all(&root).ok();
    }

    // Several files a round, each as large as one result may be. Two is more
    // than the strict rule for taking a prune can accept, since the round just
    // read is never cut: the subagent used to stop with `context_full` on its
    // second round while a prune that put it back under its line had been
    // computed and thrown away. Five is more than the window holds at all,
    // unless the round as a whole is held to a share of it.
    #[tokio::test]
    async fn parallel_reads_on_a_small_window_are_shed_rather_than_fatal() {
        for per_round in [2usize, 5] {
            let (root, file) = workspace(&format!("parallel{per_round}"));
            let round = |i: usize| {
                Reply::Tools(
                    (0..per_round)
                        .map(|k| {
                            let first = 1 + (i * per_round + k) * 300;
                            (
                                "read_file",
                                serde_json::json!({
                                    "path": file, "start_line": first, "end_line": first + 299,
                                }),
                            )
                        })
                        .collect(),
                )
            };
            let mut script: Vec<Reply> = (0..3).map(round).collect();
            script.push(Reply::Text("read all of it"));
            let (base, sizes) = spawn(script).await;
            let result = run(&config_for(&base, Some(WINDOW)), &root).await;

            assert_eq!(result.status, "completed", "{per_round}: {}", result.report);
            assert_eq!(result.rounds, 4, "{per_round}");
            assert!(
                largest_request_tokens(&sizes) < WINDOW as usize,
                "{per_round} a round: largest request {} tokens",
                largest_request_tokens(&sizes)
            );
            std::fs::remove_dir_all(&root).ok();
        }
    }

    // An explore subagent is where reading happens; its reads in one turn
    // are taken as one parallel run as the parent's are — all announced
    // before any answers — and come back in the order asked. The overlap
    // itself is measured in `tool_batch`'s tests.
    #[tokio::test]
    async fn a_subagents_reads_in_one_turn_are_announced_together_and_answered_in_order() {
        let (root, file) = workspace("together");
        let dir = root.to_string_lossy().to_string();
        let (base, _, bodies) = super::scripted_model::spawn_recording(vec![
            Reply::Tools(vec![
                (
                    "read_file",
                    serde_json::json!({ "path": file, "start_line": 1, "end_line": 50 }),
                ),
                ("list_dir", serde_json::json!({ "path": dir })),
                (
                    "read_file",
                    serde_json::json!({ "path": file, "start_line": 51, "end_line": 99 }),
                ),
            ]),
            Reply::Text("read it"),
        ])
        .await;
        let seen = Arc::new(StdMutex::new(Vec::new()));
        let events: Channel<AgentEvent> = {
            let seen = seen.clone();
            Channel::new(
                move |body: tauri::ipc::InvokeResponseBody| -> tauri::Result<()> {
                    if let tauri::ipc::InvokeResponseBody::Json(json) = &body
                        && let Ok(event) = serde_json::from_str::<AgentEvent>(json)
                    {
                        seen.lock().unwrap().push(event);
                    }
                    Ok(())
                },
            )
        };
        let ctx = ToolContext {
            workspace_root: Some(dir.clone()),
            ..crate::agent::tools::tests_support::ctx()
        };
        let spec = SubagentSpec {
            name: "reader".into(),
            goal: "read the file".into(),
            mode: SubagentMode::Explore,
            expected_output: None,
            reports_to_parent: false,
            section: None,
        };
        let result = run_subagent(
            &config_for(&base, None),
            &ctx,
            &spec,
            &events,
            &ApprovalMap::default(),
            &AnswerMap::default(),
            "test-session",
            &Arc::new(SteeringCtl::new()),
        )
        .await;
        assert_eq!(result.status, "completed", "{}", result.report);

        let kinds: Vec<String> = seen
            .lock()
            .unwrap()
            .iter()
            .filter_map(|e| match e {
                AgentEvent::ToolCall { tool_id, .. } => Some(format!("call {tool_id}")),
                AgentEvent::ToolResult { tool_id, .. } => Some(format!("result {tool_id}")),
                _ => None,
            })
            .collect();
        assert_eq!(
            kinds[..3],
            ["call tu_0_0", "call tu_0_1", "call tu_0_2"],
            "all three announced before any answered: {kinds:?}"
        );
        assert_eq!(kinds.len(), 6, "{kinds:?}");
        let second: Value = serde_json::from_str(&bodies.lock().unwrap()[1]).unwrap();
        let answered: Vec<&str> = second["messages"].as_array().unwrap().last().unwrap()["content"]
            .as_array()
            .unwrap()
            .iter()
            .filter_map(|b| b["tool_use_id"].as_str())
            .collect();
        assert_eq!(answered, ["tu_0_0", "tu_0_1", "tu_0_2"]);
        std::fs::remove_dir_all(&root).ok();
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
            reports_to_parent: false,
            section: None,
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

/// A subagent's own transcript: the turns and per-request usage that used to
/// vanish when it returned, so the delegation tax can finally be measured.
#[cfg(test)]
mod transcript_tests {
    use super::scripted_model::{Reply, spawn_with_usage};
    use super::*;
    use crate::agent::persist::{SessionRecord, SessionStore, load_records};
    use crate::agent::provider::ProviderEntry;

    fn config_for(base_url: &str) -> AgentConfig {
        let mut config = AgentConfig {
            builder_model: "stub/m".into(),
            brain_model: "stub/m".into(),
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
                context_window: None,
            },
        );
        config
    }

    #[test]
    fn the_scripted_config_calls_no_model_but_the_stub() {
        let base = "http://127.0.0.1:9";
        super::scripted_model::assert_only_the_stub_is_called(&config_for(base), base);
    }

    #[tokio::test]
    async fn a_subagent_writes_its_turns_and_usage_beside_the_session() {
        let ws = std::env::temp_dir().join(format!(
            "claudinio-transcript-{}-{}",
            std::process::id(),
            crate::agent::persist::now_ms()
        ));
        std::fs::create_dir_all(&ws).unwrap();
        let file = ws.join("notes.txt");
        std::fs::write(&file, "the answer is 42\n").unwrap();
        let ws_str = ws.to_string_lossy().to_string();
        let store = SessionStore::create("parent", Some(&ws_str)).unwrap();

        let (base, _, _) = spawn_with_usage(
            vec![
                Reply::Tool {
                    name: "read_file",
                    input: serde_json::json!({ "path": file.to_string_lossy() }),
                },
                Reply::Text("the answer is 42"),
            ],
            Some(serde_json::json!({"input_tokens": 50, "cache_read_input_tokens": 700})),
        )
        .await;
        let config = config_for(&base);
        let ctx = ToolContext {
            workspace_root: Some(ws_str.clone()),
            session_store_path: Some(store.path.to_string_lossy().to_string()),
            ..crate::agent::tools::tests_support::ctx()
        };
        let events: Channel<AgentEvent> = Channel::new(|_| Ok(()));
        let (_, _, _, _) = run_spawn_agents(
            &config,
            &ctx,
            "tu_parent",
            serde_json::json!({"agents": [
                {"name": "reader", "goal": "find the answer", "mode": "explore"}
            ]}),
            &events,
            &ApprovalMap::default(),
            &AnswerMap::default(),
            "parent",
            &Arc::new(SteeringCtl::new()),
        )
        .await;

        let path = crate::agent::persist::subagent_transcript_path(&store.path, "tu_parent", 0);
        assert_eq!(
            path,
            store
                .path
                .parent()
                .unwrap()
                .join("parent.sub")
                .join("tu_parent-0.jsonl")
        );
        let records = load_records(&path).expect("a transcript file");
        let kinds: Vec<String> = records
            .iter()
            .map(|r| {
                serde_json::to_value(r).unwrap()["kind"]
                    .as_str()
                    .unwrap()
                    .to_string()
            })
            .collect();
        assert_eq!(
            kinds,
            ["meta", "turn", "usage", "turn", "turn", "usage", "turn"],
            "goal, then per round: usage, the model's turn, the tool results"
        );
        let history = crate::agent::persist::history_from_records(&records);
        assert_eq!(history.len(), 4);
        assert!(format!("{:?}", history[0]).contains("find the answer"));
        assert!(format!("{:?}", history[2]).contains("the answer is 42"));
        assert!(format!("{:?}", history[3]).contains("the answer is 42"));
        for r in &records {
            if let SessionRecord::Usage {
                subagent,
                cache_read_tokens,
                model,
                ..
            } = r
            {
                assert_eq!(subagent.as_deref(), Some("tu_parent-0"));
                assert_eq!(*cache_read_tokens, 700);
                assert_eq!(model, "stub/m");
            }
        }
        // The parent file keeps its summary record and gains no subagent turns.
        let parent = load_records(&store.path).unwrap();
        assert!(
            parent
                .iter()
                .any(|r| matches!(r, SessionRecord::SubagentRun { .. }))
        );
        assert!(
            !parent
                .iter()
                .any(|r| matches!(r, SessionRecord::Turn { .. }))
        );

        // And the session list still shows one conversation, not the transcript.
        let listed = crate::agent::persist::list_sessions(Some(&ws_str)).unwrap();
        assert_eq!(listed.len(), 0, "the parent has no user turn of its own");
        store
            .append(&SessionRecord::User {
                text: "go".into(),
                ts: 1,
            })
            .unwrap();
        let listed = crate::agent::persist::list_sessions(Some(&ws_str)).unwrap();
        assert_eq!(listed.len(), 1);
        assert_eq!(listed[0].session_id, "parent");
        std::fs::remove_dir_all(&ws).ok();
    }

    fn kinds_and_texts(path: &std::path::Path) -> Vec<String> {
        load_records(path)
            .expect("a transcript file")
            .iter()
            .map(|r| match r {
                SessionRecord::Turn { message, .. } => {
                    let text: String = message
                        .content
                        .iter()
                        .filter_map(|b| match b {
                            ContentBlock::Text { text, .. } => Some(text.as_str()),
                            _ => None,
                        })
                        .collect();
                    // The goal message is wrapped in boilerplate; keep the goal.
                    let text = text
                        .strip_prefix("Your goal:\n")
                        .and_then(|t| t.split("\n\n").next())
                        .unwrap_or(&text);
                    format!("{}:{}", message.role, text)
                }
                other => serde_json::to_value(other).unwrap()["kind"]
                    .as_str()
                    .unwrap()
                    .to_string(),
            })
            .collect()
    }

    fn session_in(tag: &str) -> (std::path::PathBuf, String, SessionStore) {
        let ws = std::env::temp_dir().join(format!(
            "claudinio-transcript-{tag}-{}-{}",
            std::process::id(),
            crate::agent::persist::now_ms()
        ));
        std::fs::create_dir_all(&ws).unwrap();
        let ws_str = ws.to_string_lossy().to_string();
        let store = SessionStore::create("parent", Some(&ws_str)).unwrap();
        (ws, ws_str, store)
    }

    // Stop pressed while a subagent's reads run together: the reads already
    // under way finish, keep their results and count as calls made; the ones
    // the pause kept from starting are answered as not run. Every call of the
    // turn gets exactly one result, in the transcript and on the timeline.
    #[tokio::test]
    async fn a_pause_during_a_subagents_parallel_reads_answers_every_call_once() {
        let (ws, ws_str, store) = session_in("parallel-pause");
        let file = ws.join("notes.txt");
        std::fs::write(&file, "one\ntwo\nthree\nfour\nfive\nsix\n").unwrap();
        let path = file.to_string_lossy().to_string();
        let reads: Vec<(&str, Value)> = (1..=6)
            .map(|line| {
                (
                    "read_file",
                    serde_json::json!({ "path": path, "start_line": line, "end_line": line }),
                )
            })
            .collect();
        let (base, _, _) =
            spawn_with_usage(vec![Reply::Tools(reads), Reply::Text("unreachable")], None).await;
        let config = config_for(&base);
        let ctx = ToolContext {
            workspace_root: Some(ws_str),
            session_store_path: Some(store.path.to_string_lossy().to_string()),
            ..crate::agent::tools::tests_support::ctx()
        };
        let steering = Arc::new(SteeringCtl::new());
        let results = Arc::new(std::sync::Mutex::new(Vec::<(String, String)>::new()));
        let events: Channel<AgentEvent> = {
            let (results, stop) = (results.clone(), steering.interrupt.clone());
            Channel::new(
                move |body: tauri::ipc::InvokeResponseBody| -> tauri::Result<()> {
                    if let tauri::ipc::InvokeResponseBody::Json(json) = &body
                        && let Ok(AgentEvent::ToolResult {
                            tool_id, output, ..
                        }) = serde_json::from_str::<AgentEvent>(json)
                    {
                        // The first read to finish is when the user stops.
                        stop.store(true, Ordering::SeqCst);
                        results.lock().unwrap().push((tool_id, output));
                    }
                    Ok(())
                },
            )
        };
        let spec = SubagentSpec {
            name: "reader".into(),
            goal: "read the notes".into(),
            mode: SubagentMode::Explore,
            expected_output: None,
            reports_to_parent: false,
            section: None,
        };
        let transcript = Transcript::open(&ctx, "tu_par", 0, "sub");
        let result = run_subagent_recorded(
            &config,
            &ctx,
            &spec,
            &events,
            &ApprovalMap::default(),
            &AnswerMap::default(),
            "parent",
            &steering,
            transcript.as_ref(),
        )
        .await;
        assert_eq!(result.status, "interrupted", "{}", result.report);

        const NOT_RUN: &str = "Interrupted by user — tool not executed.";
        let ids: Vec<String> = (0..6).map(|k| format!("tu_0_{k}")).collect();
        let mut reported = results.lock().unwrap().clone();
        reported.sort();
        assert_eq!(
            reported
                .iter()
                .map(|(id, _)| id.clone())
                .collect::<Vec<_>>(),
            ids,
            "one ToolResult each: {reported:?}"
        );
        let not_run: Vec<&str> = reported
            .iter()
            .filter(|(_, out)| out == NOT_RUN)
            .map(|(id, _)| id.as_str())
            .collect();
        assert_eq!(not_run, ["tu_0_4", "tu_0_5"], "the two kept from starting");
        assert_eq!(
            result.tools.get("read_file"),
            Some(&4),
            "the four reads that ran are counted"
        );

        let path = crate::agent::persist::subagent_transcript_path(&store.path, "tu_par", 0);
        let answered: Vec<String> = load_records(&path)
            .unwrap()
            .into_iter()
            .rev()
            .find_map(|r| match r {
                SessionRecord::Turn { message, .. } if message.role == "user" => Some(message),
                _ => None,
            })
            .expect("the answered turn")
            .content
            .iter()
            .filter_map(|b| match b {
                ContentBlock::ToolResult { tool_use_id, .. } => Some(tool_use_id.clone()),
                _ => None,
            })
            .collect();
        assert_eq!(
            answered, ids,
            "the turn is answered call for call, in order"
        );
        std::fs::remove_dir_all(&ws).ok();
    }

    // The spawn_agents description no longer spells out a spec's fields
    // (Phase 2 description diet); a spec that does not parse gets them back
    // in the error, where they cost nothing until a call needs them.
    #[tokio::test]
    async fn a_spec_that_does_not_parse_is_answered_with_the_spec_shape() {
        let (ws, ws_str, store) = session_in("badspec");
        let config = config_for("http://127.0.0.1:1");
        let ctx = ToolContext {
            workspace_root: Some(ws_str),
            session_store_path: Some(store.path.to_string_lossy().to_string()),
            ..crate::agent::tools::tests_support::ctx()
        };
        let events: Channel<AgentEvent> = Channel::new(|_| Ok(()));
        let (block, _, _, _) = run_spawn_agents(
            &config,
            &ctx,
            "tu_bad",
            serde_json::json!({"agents": [{"name": "r", "goal": "explain", "mode": "write"}]}),
            &events,
            &ApprovalMap::default(),
            &AnswerMap::default(),
            "parent",
            &Arc::new(SteeringCtl::new()),
        )
        .await;
        let text = format!("{block:?}");
        for part in ["'name'", "'goal'", "'explore'", "'code'", "expected_output"] {
            assert!(text.contains(part), "{part} missing: {text}");
        }
        std::fs::remove_dir_all(&ws).ok();
    }

    // What the model said before the user stopped it is part of the
    // subagent's conversation, and the transcript is that conversation.
    #[tokio::test]
    async fn an_interrupted_subagent_keeps_what_it_had_said_in_its_transcript() {
        let (ws, ws_str, store) = session_in("interrupted");
        // The first words arrive late, as from a slow or loaded provider: the
        // stop must follow them, not a clock.
        let (base, _, _) = spawn_with_usage(
            vec![Reply::Delayed(
                700,
                Box::new(Reply::HangSignalled("halfway through the")),
            )],
            None,
        )
        .await;
        let config = config_for(&base);
        let ctx = ToolContext {
            workspace_root: Some(ws_str),
            session_store_path: Some(store.path.to_string_lossy().to_string()),
            ..crate::agent::tools::tests_support::ctx()
        };
        let steering = Arc::new(SteeringCtl::new());
        // The person presses stop once the subagent's first words are in
        // (`HangSignalled`).
        let stop = steering.interrupt.clone();
        let events: Channel<AgentEvent> =
            Channel::new(move |body: tauri::ipc::InvokeResponseBody| {
                if let tauri::ipc::InvokeResponseBody::Json(json) = &body
                    && let Ok(AgentEvent::Subagent { event, .. }) =
                        serde_json::from_str::<AgentEvent>(json)
                    && matches!(*event, AgentEvent::Thinking(_))
                {
                    stop.store(true, Ordering::SeqCst);
                }
                Ok(())
            });
        let (block, _, _, _) = run_spawn_agents(
            &config,
            &ctx,
            "tu_stop",
            serde_json::json!({"agents": [{"name": "r", "goal": "explain", "mode": "explore"}]}),
            &events,
            &ApprovalMap::default(),
            &AnswerMap::default(),
            "parent",
            &steering,
        )
        .await;
        assert!(format!("{block:?}").contains("interrupted"), "{block:?}");
        let path = crate::agent::persist::subagent_transcript_path(&store.path, "tu_stop", 0);
        assert_eq!(
            kinds_and_texts(&path),
            [
                "meta",
                "user:explain",
                "usage",
                "assistant:halfway through the"
            ]
        );
        std::fs::remove_dir_all(&ws).ok();
    }

    // A blocking SubagentStop answers the turn the model just finished; the
    // transcript holds that turn before the feedback, never two user turns
    // in a row.
    #[cfg(unix)]
    #[tokio::test]
    async fn a_stop_hook_block_follows_the_turn_it_answers_in_the_transcript() {
        use std::os::unix::fs::PermissionsExt;
        let (ws, ws_str, store) = session_in("stophook");
        let mark = ws.join("blocked-once");
        let script = ws.join("stop.sh");
        std::fs::write(
            &script,
            format!(
                "#!/bin/sh\nif [ -e '{m}' ]; then exit 0; fi\ntouch '{m}'\n\
                 echo 'cite the file' >&2\nexit 2\n",
                m = mark.display()
            ),
        )
        .unwrap();
        std::fs::set_permissions(&script, std::fs::Permissions::from_mode(0o755)).unwrap();
        std::fs::write(
            ws.join(".claudinio.json"),
            format!(
                r#"{{"hooks":{{"SubagentStop":[{{"hooks":[{{"type":"command","command":"{}"}}]}}]}}}}"#,
                script.display()
            ),
        )
        .unwrap();
        let hook_cfg = AgentConfig {
            hooks_enabled: true,
            ..Default::default()
        };
        let set = Arc::new(crate::agent::hooks::discovery::resolve_with_home(
            Some(&ws),
            &hook_cfg,
            Some(&ws.join("_home")),
        ));
        let trust = Arc::new(crate::agent::hooks::TrustStore::with_path(
            ws.join("trust.json"),
        ));
        trust
            .approve(&ws.display().to_string(), &set.fingerprint, vec![], 1)
            .unwrap();
        let hooks = Arc::new(crate::agent::hooks::HookCtx::new(
            set,
            trust,
            "parent",
            store.path.clone(),
            ws.clone(),
        ));

        let (base, _, _) = spawn_with_usage(
            vec![Reply::Text("first answer"), Reply::Text("second answer")],
            None,
        )
        .await;
        let config = config_for(&base);
        let ctx = ToolContext {
            workspace_root: Some(ws_str),
            session_store_path: Some(store.path.to_string_lossy().to_string()),
            doc_access: Default::default(),
            hooks: Some(hooks),
            ..crate::agent::tools::tests_support::ctx()
        };
        let events: Channel<AgentEvent> = Channel::new(|_| Ok(()));
        run_spawn_agents(
            &config,
            &ctx,
            "tu_hook",
            serde_json::json!({"agents": [{"name": "r", "goal": "explain", "mode": "explore"}]}),
            &events,
            &ApprovalMap::default(),
            &AnswerMap::default(),
            "parent",
            &Arc::new(SteeringCtl::new()),
        )
        .await;
        let path = crate::agent::persist::subagent_transcript_path(&store.path, "tu_hook", 0);
        assert_eq!(
            kinds_and_texts(&path),
            [
                "meta",
                "user:explain",
                "usage",
                "assistant:first answer",
                "user:<hook-feedback>\ncite the file\n</hook-feedback>",
                "usage",
                "assistant:second answer",
            ]
        );
        std::fs::remove_dir_all(&ws).ok();
    }
}

/// Lean-harness Phase 3: what a subagent is asked to hand back, what of it
/// the parent carries, and the brief a code subagent starts from.
#[cfg(test)]
mod delegation_tests {
    use super::scripted_model::{Reply, spawn_recording};
    use super::*;
    use crate::agent::persist::{SessionStore, TaskItem};
    use crate::agent::provider::ProviderEntry;

    fn config_for(base_url: &str) -> AgentConfig {
        let mut config = AgentConfig {
            builder_model: "stub/m".into(),
            brain_model: "stub/m".into(),
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
                context_window: None,
            },
        );
        config
    }

    #[test]
    fn the_scripted_config_calls_no_model_but_the_stub() {
        let base = "http://127.0.0.1:9";
        super::scripted_model::assert_only_the_stub_is_called(&config_for(base), base);
    }

    fn workspace(tag: &str) -> (std::path::PathBuf, String, SessionStore) {
        let ws = std::env::temp_dir().join(format!(
            "claudinio-delegation-{tag}-{}-{}",
            std::process::id(),
            crate::agent::persist::now_ms()
        ));
        std::fs::create_dir_all(&ws).unwrap();
        let ws_str = ws.to_string_lossy().to_string();
        let store = SessionStore::create("parent", Some(&ws_str)).unwrap();
        (ws, ws_str, store)
    }

    /// Spawn `agents` against a model that answers every request with
    /// `reply`; the tool result, every event and every request body.
    async fn spawn(
        ctx: &ToolContext,
        reply: &'static str,
        agents: Value,
    ) -> (String, Vec<AgentEvent>, Vec<String>) {
        spawn_script(ctx, vec![Reply::Text(reply)], agents).await
    }

    /// `spawn`, against a model that follows `script`.
    async fn spawn_script(
        ctx: &ToolContext,
        script: Vec<Reply>,
        agents: Value,
    ) -> (String, Vec<AgentEvent>, Vec<String>) {
        let (base, _, bodies) = spawn_recording(script).await;
        let config = config_for(&base);
        let seen = Arc::new(std::sync::Mutex::new(Vec::new()));
        let events: Channel<AgentEvent> = {
            let seen = seen.clone();
            Channel::new(move |body: tauri::ipc::InvokeResponseBody| {
                if let tauri::ipc::InvokeResponseBody::Json(json) = &body
                    && let Ok(event) = serde_json::from_str::<AgentEvent>(json)
                {
                    seen.lock().unwrap().push(event);
                }
                Ok(())
            })
        };
        let (block, _, _, _) = run_spawn_agents(
            &config,
            ctx,
            "tu_parent",
            serde_json::json!({ "agents": agents }),
            &events,
            &ApprovalMap::default(),
            &AnswerMap::default(),
            "parent",
            &Arc::new(SteeringCtl::new()),
        )
        .await;
        let text = match block {
            ContentBlock::ToolResult { content, .. } => content.as_text().into_owned(),
            other => panic!("not a tool result: {other:?}"),
        };
        let events = seen.lock().unwrap().clone();
        let bodies = bodies.lock().unwrap().clone();
        (text, events, bodies)
    }

    #[tokio::test]
    async fn an_explore_subagent_is_asked_for_a_short_report_in_three_sections() {
        let (ws, ws_str, store) = workspace("format");
        let ctx = ToolContext {
            workspace_root: Some(ws_str),
            session_store_path: Some(store.path.to_string_lossy().to_string()),
            ..crate::agent::tools::tests_support::ctx()
        };
        let (_, _, bodies) = spawn(
            &ctx,
            "done",
            serde_json::json!([{"name": "r", "goal": "look around", "mode": "explore"}]),
        )
        .await;
        let first: Value = serde_json::from_str(&bodies[0]).unwrap();
        let system = first["system"].to_string();
        for part in [
            "1,500 tokens",
            "## Findings",
            "## Files",
            "path:line",
            "## Open questions",
        ] {
            assert!(system.contains(part), "{part} missing from {system}");
        }
        std::fs::remove_dir_all(&ws).ok();
    }

    // The compaction summarizer runs in explore mode but writes a handoff of
    // its own shape; the explore report format must not compete with it.
    #[tokio::test]
    async fn the_compaction_summarizer_is_not_asked_for_the_explore_report() {
        let (ws, ws_str, store) = workspace("summary");
        let (base, _, bodies) = spawn_recording(vec![Reply::Text("summary")]).await;
        let config = config_for(&base);
        let ctx = ToolContext {
            workspace_root: Some(ws_str),
            session_store_path: Some(store.path.to_string_lossy().to_string()),
            ..crate::agent::tools::tests_support::ctx()
        };
        let events: Channel<AgentEvent> = Channel::new(|_| Ok(()));
        let summary = run_summary_agent(
            &config,
            &ctx,
            &store.path.to_string_lossy(),
            0,
            &events,
            &ApprovalMap::default(),
            &AnswerMap::default(),
            "parent",
            &Arc::new(SteeringCtl::new()),
        )
        .await;
        assert_eq!(summary.as_deref(), Ok("summary"));
        let first: Value = serde_json::from_str(&bodies.lock().unwrap()[0]).unwrap();
        assert!(!first["system"].to_string().contains("## Findings"));
        std::fs::remove_dir_all(&ws).ok();
    }

    #[tokio::test]
    async fn the_parent_carries_a_long_report_cut_in_the_middle_and_the_timeline_all_of_it() {
        let (ws, ws_str, store) = workspace("cap");
        let ctx = ToolContext {
            workspace_root: Some(ws_str),
            session_store_path: Some(store.path.to_string_lossy().to_string()),
            ..crate::agent::tools::tests_support::ctx()
        };
        let long: &'static str = Box::leak(
            format!(
                "## Findings\n{}## Files\nsrc/lib.rs:1 the end\n",
                "a finding worth a line of its own\n".repeat(600)
            )
            .into_boxed_str(),
        );
        let (text, events, _) = spawn(
            &ctx,
            long,
            serde_json::json!([{"name": "r", "goal": "look around", "mode": "explore"}]),
        )
        .await;
        assert!(
            text.contains("characters of this report omitted"),
            "{}",
            &text[..200]
        );
        assert!(text.contains("## Findings") && text.contains("src/lib.rs:1 the end"));
        assert!(
            text.chars().count() <= crate::agent::delegation::REPORT_CHARS + 200,
            "{} chars carried",
            text.chars().count()
        );
        let shown = events
            .iter()
            .find_map(|e| match e {
                AgentEvent::SubagentDone { report, .. } => Some(report.clone()),
                _ => None,
            })
            .expect("the subagent finished");
        assert_eq!(shown, long, "the person still sees the whole report");
        std::fs::remove_dir_all(&ws).ok();
    }

    /// A code subagent creates a file the same way the main session does
    /// (lean-harness Phase 3b): the baseline eval's subagents reported
    /// "BLOCKED — could not be created" for every new file they were given.
    #[tokio::test]
    async fn a_code_subagent_creates_a_file_in_a_missing_folder_and_edits_it() {
        let (ws, ws_str, store) = workspace("create");
        let ctx = ToolContext {
            workspace_root: Some(ws_str),
            session_store_path: Some(store.path.to_string_lossy().to_string()),
            ..crate::agent::tools::tests_support::ctx()
        };
        let path = ws.join("src/lib/path.ts").to_string_lossy().to_string();
        let (base, _, _) = spawn_recording(vec![
            Reply::Tool {
                name: "edit_file",
                input: serde_json::json!({
                    "path": path, "old_string": "", "new_string": "export const a = 1;\n",
                }),
            },
            Reply::Tool {
                name: "edit_file",
                input: serde_json::json!({
                    "path": path, "old_string": "= 1", "new_string": "= 2",
                }),
            },
            Reply::Text("created src/lib/path.ts"),
        ])
        .await;
        let mut config = config_for(&base);
        config.yolo_mode = true;
        let events: Channel<AgentEvent> = Channel::new(|_| Ok(()));
        run_spawn_agents(
            &config,
            &ctx,
            "tu_parent",
            serde_json::json!({ "agents": [
                {"name": "writer", "goal": "create src/lib/path.ts", "mode": "code"}
            ]}),
            &events,
            &ApprovalMap::default(),
            &AnswerMap::default(),
            "parent",
            &Arc::new(SteeringCtl::new()),
        )
        .await;
        assert_eq!(
            std::fs::read_to_string(&path).ok().as_deref(),
            Some("export const a = 2;\n")
        );
        std::fs::remove_dir_all(&ws).ok();
    }

    /// A code subagent edits files, so a shell command that writes one is
    /// asked about like an edit even when its prefix is allowlisted — as it is
    /// in the Builder that delegated to it. Delegating must not be a way
    /// around the approval card.
    #[tokio::test]
    async fn a_code_subagents_allowlisted_shell_write_is_asked_about() {
        let (ws, ws_str, store) = workspace("shell-write");
        let ctx = ToolContext {
            workspace_root: Some(ws_str.clone()),
            session_store_path: Some(store.path.to_string_lossy().to_string()),
            ..crate::agent::tools::tests_support::ctx()
        };
        let target = ws.join("out.txt");
        let (base, _, _) = spawn_recording(vec![
            Reply::Tool {
                name: "bash",
                input: serde_json::json!({ "command": "echo x > out.txt", "workdir": ws_str }),
            },
            Reply::Text("done"),
        ])
        .await;
        let config = config_for(&base);
        let asked = Arc::new(std::sync::Mutex::new(Vec::<String>::new()));
        let events: Channel<AgentEvent> = {
            let asked = asked.clone();
            Channel::new(move |body: tauri::ipc::InvokeResponseBody| {
                if let tauri::ipc::InvokeResponseBody::Json(json) = &body
                    && let Ok(AgentEvent::Subagent { event, .. }) =
                        serde_json::from_str::<AgentEvent>(json)
                    && let AgentEvent::ToolCall {
                        tool_name,
                        permission,
                        ..
                    } = *event
                {
                    asked
                        .lock()
                        .unwrap()
                        .push(format!("{tool_name}:{permission}"));
                }
                Ok(())
            })
        };
        // The person turns the card down as soon as it appears.
        let approvals = ApprovalMap::default();
        let refuser = {
            let approvals = approvals.clone();
            tokio::spawn(async move {
                loop {
                    let pending: Vec<_> = approvals.lock().await.drain().collect();
                    for (_, tx) in pending {
                        let _ = tx.send(false);
                    }
                    tokio::time::sleep(std::time::Duration::from_millis(5)).await;
                }
            })
        };
        run_spawn_agents(
            &config,
            &ctx,
            "tu_parent",
            serde_json::json!({ "agents": [
                {"name": "writer", "goal": "write out.txt", "mode": "code"}
            ]}),
            &events,
            &approvals,
            &AnswerMap::default(),
            "parent",
            &Arc::new(SteeringCtl::new()),
        )
        .await;
        refuser.abort();
        assert_eq!(
            asked.lock().unwrap().clone(),
            vec!["bash:requires_approval".to_string()]
        );
        assert!(!target.exists(), "a refused write must not run");
        std::fs::remove_dir_all(&ws).ok();
    }

    #[tokio::test]
    async fn a_code_subagent_starts_from_the_shared_brief_and_an_explore_one_does_not() {
        let (ws, ws_str, store) = workspace("brief");
        crate::agent::persist::append_tasks(
            &store.path,
            &[TaskItem {
                id: "t1".into(),
                title: "Rename the greeting".into(),
                description: "Say goodbye instead of hello.".into(),
                journal: vec![],
                status: "doing".into(),
            }],
        )
        .unwrap();
        let plans = ws.join(".claudinio").join("plans");
        std::fs::create_dir_all(&plans).unwrap();
        std::fs::write(plans.join("greeting.md"), "# Plan\n").unwrap();
        let ctx = ToolContext {
            workspace_root: Some(ws_str.clone()),
            session_store_path: Some(store.path.to_string_lossy().to_string()),
            ..crate::agent::tools::tests_support::ctx()
        };
        {
            let mut tracker = ctx.read_tracker.lock().await;
            tracker.record_read(&format!("{ws_str}/src/b.rs"), None, None, "fn b() {}");
            tracker.record_read(&format!("{ws_str}/src/a.rs"), Some(1), Some(2), "fn a() {}");
        }
        let (_, events, bodies) = spawn(
            &ctx,
            "done",
            serde_json::json!([
                {"name": "writer", "goal": "change the greeting", "mode": "code"},
                {"name": "reader", "goal": "look around", "mode": "explore"}
            ]),
        )
        .await;
        let first_user = |body: &String| -> String {
            let body: Value = serde_json::from_str(body).unwrap();
            body["messages"][0].to_string()
        };
        let code = bodies
            .iter()
            .map(first_user)
            .find(|m| m.contains("change the greeting"))
            .expect("the code subagent asked");
        let explore = bodies
            .iter()
            .map(first_user)
            .find(|m| m.contains("look around"))
            .expect("the explore subagent asked");
        assert!(!explore.contains("<brief>"), "{explore}");
        assert!(code.contains("<brief>"), "{code}");
        assert!(code.contains("Rename the greeting"), "{code}");
        assert!(code.contains(".claudinio/plans/greeting.md"), "{code}");
        let (a, b) = (
            code.find("- src/a.rs").expect("a read path"),
            code.find("- src/b.rs").expect("a read path"),
        );
        assert!(a < b, "sorted");
        assert!(!code.contains("fn a()"), "paths only");
        assert!(
            code.find("</brief>").unwrap() < code.find("change the greeting").unwrap(),
            "the brief comes before the goal"
        );
        // The timeline shows the goal the model wrote.
        let started: Vec<String> = events
            .iter()
            .filter_map(|e| match e {
                AgentEvent::SubagentStarted { goal, .. } => Some(goal.clone()),
                _ => None,
            })
            .collect();
        assert!(
            started.contains(&"change the greeting".to_string()),
            "{started:?}"
        );
        std::fs::remove_dir_all(&ws).ok();
    }

    /// A workspace whose session has a living document with these topics.
    fn doc_workspace(tag: &str, topics: &[&str]) -> (std::path::PathBuf, ToolContext) {
        let (ws, ws_str, store) = workspace(tag);
        let topics: Vec<String> = topics.iter().map(|t| t.to_string()).collect();
        let (rel, _) =
            crate::agent::doc_session::create(&ws, Some(&store.path), "Pudim de leite", &topics)
                .unwrap();
        let ctx = ToolContext {
            workspace_root: Some(ws_str),
            session_store_path: Some(store.path.to_string_lossy().to_string()),
            ..crate::agent::tools::tests_support::ctx()
        };
        crate::agent::doc_session::link(&store, &ctx.records_cache, &rel);
        (ws, ctx)
    }

    fn doc_text(ws: &std::path::Path) -> String {
        std::fs::read_to_string(ws.join(".claudinio/docs/pudim-de-leite.md")).unwrap()
    }

    fn tool_names(body: &str) -> Vec<String> {
        let body: Value = serde_json::from_str(body).unwrap();
        body["tools"]
            .as_array()
            .unwrap()
            .iter()
            .filter_map(|t| t["name"].as_str().map(str::to_string))
            .collect()
    }

    /// Decision D9: an explore subagent given a section writes it itself —
    /// with `doc_write`/`doc_read` and nothing else of the document — so the
    /// parent reads the section, not a report about it.
    #[tokio::test]
    async fn a_subagent_given_a_section_writes_it_under_its_own_name() {
        let (ws, ctx) = doc_workspace("section", &["origem", "ingredientes"]);
        let (text, events, bodies) = spawn_script(
            &ctx,
            vec![
                Reply::Tool {
                    name: "doc_write",
                    input: serde_json::json!({"id": "s2", "body": "leite condensado, ovos"}),
                },
                Reply::Text("Wrote s2."),
            ],
            serde_json::json!([{"name": "ingredientes", "goal": "list the ingredients", "mode": "explore", "section": "s2"}]),
        )
        .await;
        let names = tool_names(&bodies[0]);
        assert!(
            names.contains(&"doc_write".to_string()) && names.contains(&"doc_read".to_string())
        );
        assert!(
            !names.iter().any(|n| n == "doc_create" || n == "doc_add"),
            "{names:?}"
        );
        let goal: Value = serde_json::from_str(&bodies[0]).unwrap();
        let goal = goal["messages"][0].to_string();
        assert!(
            goal.contains("s2") && goal.contains("ingredientes"),
            "{goal}"
        );
        assert!(
            doc_text(&ws).contains(
                "## ingredientes\n<!-- claudinio: id=s2 status=done by=sub:ingredientes -->\nleite condensado, ovos"
            ),
            "{}",
            doc_text(&ws)
        );
        assert!(text.contains("Wrote s2."), "{text}");
        assert!(
            events
                .iter()
                .any(|e| matches!(e, AgentEvent::DocumentUpdated { .. })),
            "the panel hears of the claim"
        );
        std::fs::remove_dir_all(&ws).ok();
    }

    #[tokio::test]
    async fn a_subagent_cannot_write_a_section_it_was_not_given() {
        let (ws, ctx) = doc_workspace("other-section", &["origem", "ingredientes"]);
        let (_, _, bodies) = spawn_script(
            &ctx,
            vec![
                Reply::Tool {
                    name: "doc_write",
                    input: serde_json::json!({"id": "s1", "body": "not mine"}),
                },
                Reply::Text("tried"),
            ],
            serde_json::json!([{"name": "ingredientes", "goal": "list them", "mode": "explore", "section": "s2"}]),
        )
        .await;
        assert!(!doc_text(&ws).contains("not mine"));
        assert!(
            bodies[1].contains("you were given section s2 only"),
            "{}",
            bodies[1]
        );
        std::fs::remove_dir_all(&ws).ok();
    }

    #[tokio::test]
    async fn two_agents_cannot_claim_the_same_section() {
        let (ws, ctx) = doc_workspace("same-section", &["origem"]);
        let (text, _, bodies) = spawn_script(
            &ctx,
            vec![Reply::Text("done")],
            serde_json::json!([
                {"name": "a", "goal": "write it", "mode": "explore", "section": "s1"},
                {"name": "b", "goal": "write it too", "mode": "explore", "section": "s1"}
            ]),
        )
        .await;
        assert_eq!(bodies.len(), 1, "only one of them ran");
        assert!(
            text.contains("not started") && text.contains("s1"),
            "{text}"
        );
        // A section the main session holds is not given away either.
        crate::agent::doc_session::write(&ctx, "s1", None, crate::agent::document::Status::Doing)
            .unwrap();
        let (text, _, bodies) = spawn_script(
            &ctx,
            vec![Reply::Text("done")],
            serde_json::json!([{"name": "c", "goal": "write it", "mode": "explore", "section": "s1"}]),
        )
        .await;
        assert!(bodies.is_empty(), "{bodies:?}");
        assert!(text.contains("main"), "{text}");
        std::fs::remove_dir_all(&ws).ok();
    }

    #[tokio::test]
    async fn a_section_left_unwritten_is_released_and_one_without_a_section_gets_no_document_tools()
    {
        let (ws, ctx) = doc_workspace("release", &["origem"]);
        let (_, _, bodies) = spawn_script(
            &ctx,
            vec![Reply::Text("gave up")],
            serde_json::json!([{"name": "a", "goal": "write it", "mode": "explore", "section": "s1"}]),
        )
        .await;
        assert_eq!(bodies.len(), 1);
        assert!(
            doc_text(&ws).contains("[origem] <!-- claudinio: id=s1 status=todo -->"),
            "{}",
            doc_text(&ws)
        );
        let (_, _, bodies) = spawn_script(
            &ctx,
            vec![Reply::Text("looked")],
            serde_json::json!([{"name": "r", "goal": "look around", "mode": "explore"}]),
        )
        .await;
        assert!(!tool_names(&bodies[0]).iter().any(|n| n.starts_with("doc_")));
        std::fs::remove_dir_all(&ws).ok();
    }
}
