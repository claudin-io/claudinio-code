use crate::agent::permissions;
use crate::agent::persist::{now_ms, SessionRecord, SessionStore};
use crate::agent::provider::{self, AgentConfig, ContentBlock, Message, ToolDescription};
use crate::agent::subagent;
use crate::agent::tools::{self, ToolContext, ToolOutput};
use serde::{Deserialize, Serialize};
use serde_json::Value;
use std::collections::HashMap;
use std::sync::atomic::{AtomicBool, Ordering};
use std::sync::{Arc, Mutex as StdMutex};
use tauri::ipc::Channel;
use tokio::sync::{oneshot, Mutex};

/// Context window of the supported models (claudinio and claudius: 256K).
pub const MAX_CONTEXT_TOKENS: u64 = 256_000;

/// Threshold for auto-compaction: if the context exceeds this before a
/// request, the history is compacted first (75% of the window).
pub const COMPACT_THRESHOLD: u64 = MAX_CONTEXT_TOKENS * 75 / 100;

/// Rough token estimation: count chars / 3 + per-message overhead + system prompt + tools.
fn estimate_message_tokens(msg: &Message) -> u64 {
    let json = serde_json::to_string(msg).unwrap_or_default();
    json.len() as u64 / 3 + 4 // +4 for role/format overhead
}

fn estimate_tokens(history: &[Message], system: &str, tools: &[ToolDescription]) -> u64 {
    let mut total = system.len() as u64 / 3;
    if !tools.is_empty() {
        total += serde_json::to_string(tools).unwrap_or_default().len() as u64 / 3;
    }
    // Per-message overhead (~4 tokens each for role markers + turn formatting)
    total += (history.len() as u64) * 8;
    for msg in history {
        total += estimate_message_tokens(msg);
    }
    total
}

/// How many recent user↔agent exchanges stay verbatim after a compaction.
const TAIL_USER_TURNS: usize = 2;
/// Budget for the kept tail; if the recent exchanges alone exceed this, the
/// tail shrinks (down to zero) so compaction still frees the context.
const TAIL_MAX_TOKENS: u64 = 20_000;

/// Number of Turn records (counted back from the end) to keep verbatim when
/// compacting: the last `TAIL_USER_TURNS` real user exchanges, bounded by
/// `TAIL_MAX_TOKENS`. Only looks at records after the previous compaction.
fn compute_tail_turns(records: &[SessionRecord]) -> usize {
    let start = records
        .iter()
        .rposition(|r| matches!(r, SessionRecord::Compacted { .. }))
        .map(|i| i + 1)
        .unwrap_or(0);
    let mut turns = 0usize;
    let mut exchanges = 0usize;
    let mut tokens = 0u64;
    let mut best = 0usize;
    for rec in records[start..].iter().rev() {
        let SessionRecord::Turn { message, .. } = rec else { continue };
        turns += 1;
        tokens += estimate_message_tokens(message);
        if tokens > TAIL_MAX_TOKENS {
            break;
        }
        if crate::agent::persist::is_real_user_turn(rec) {
            exchanges += 1;
            best = turns; // a window starting at this user turn fits the budget
            if exchanges >= TAIL_USER_TURNS {
                break;
            }
        }
    }
    best
}

/// Compact the conversation history by spawning a subagent to read the JSONL
/// file and produce a summary. The subagent has a completely fresh context.
/// The last `TAIL_USER_TURNS` exchanges are kept verbatim (recorded as
/// `tail_turns` on the Compacted marker). Returns the generated summary.
pub async fn compact_history(
    config: &AgentConfig,
    store: &SessionStore,
    ctx: &ToolContext,
    event_tx: &Channel<AgentEvent>,
    approvals: &ApprovalMap,
    answers: &AnswerMap,
    session_id: &str,
    steering: &Arc<SteeringCtl>,
) -> Result<String, String> {
    let jsonl_path = store.path.to_string_lossy().to_string();
    let records = crate::agent::persist::load_records(&store.path).unwrap_or_default();
    let tail_turns = compute_tail_turns(&records);

    let summary = subagent::run_summary_agent(
        config,
        ctx,
        &jsonl_path,
        tail_turns,
        event_tx,
        approvals,
        answers,
        session_id,
        steering,
    )
    .await?;

    // If the summary agent somehow found an existing Compacted record and
    // returned it, still write ours — the format is append-only and the
    // history_from_records logic picks the LAST one.
    store.append(&SessionRecord::Compacted {
        summary: summary.clone(),
        tail_turns,
        ts: now_ms(),
    })?;

    // Record the post-compaction context size so the UI meter drops even for
    // manual compaction (no run in flight). The estimate excludes the system
    // prompt/tools; the next run's Status corrects it with the real number.
    let new_recs = crate::agent::persist::load_records(&store.path).unwrap_or_default();
    let new_history = crate::agent::persist::history_from_records(&new_recs);
    let (ci, co, cc) = crate::agent::persist::cumulative_stats(&new_recs);
    let new_context = estimate_tokens(&new_history, "", &[]);
    write_status(store, session_id, ci, co, cc, Some(new_context));

    Ok(summary)
}

/// Steering: a queue of mid-run user messages and an interrupt flag.
/// Thread-safe; the Mutex is never held across await.
pub struct SteeringCtl {
    pub queue: StdMutex<Vec<String>>,
    pub interrupt: AtomicBool,
}

impl SteeringCtl {
    pub fn new() -> Self {
        Self {
            queue: StdMutex::new(Vec::new()),
            interrupt: AtomicBool::new(false),
        }
    }

    pub fn drain(&self) -> Vec<String> {
        let mut q = self.queue.lock().unwrap();
        std::mem::take(&mut *q)
    }

    pub fn push(&self, text: String) {
        let mut q = self.queue.lock().unwrap();
        q.push(text);
    }

    pub fn clear(&self) {
        self.queue.lock().unwrap().clear();
        self.interrupt.store(false, Ordering::SeqCst);
    }
}

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
\
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
Example: to understand an unfamiliar file, call file_outline first, not read_file. \
\
Be focused and concrete. \
\
You can delegate work to parallel subagents with the spawn_agents tool. A subagent is a copy of you with \
a fresh, empty context, its own goal, and the same tools (except spawn_agents and ask_user — subagents \
cannot ask the user anything or spawn further agents). Each subagent runs independently and returns only \
its final report to you; its intermediate work never enters your context. Use subagents to keep your own \
context clean and to parallelize. \
WHEN to use subagents: (1) broad investigation that would require reading many files — spawn 2-4 'explore' \
agents, each covering a distinct area, and synthesize their reports; (2) independent, atomic code tasks \
that touch disjoint files — spawn 'code' agents, one per task; (3) any task whose intermediate output \
would bloat your context but whose conclusion is short. WHEN NOT to: trivial lookups (a single read_file \
or grep is faster and cheaper), tasks that depend on each other's results (run them yourself sequentially, \
or spawn in sequential waves), or anything needing a user decision mid-task — resolve that with ask_user \
BEFORE spawning. \
HOW to write good subagent goals: each goal must be fully self-contained — the subagent knows nothing \
about this conversation. Include the concrete question or change, relevant file paths and symbol names \
you already know, constraints, and what to leave alone. Always set expected_output to describe the report \
you need (e.g. 'list of file:line locations with a one-line explanation each'). Modes: 'explore' = \
read-only investigation; 'code' = may edit files and run commands (edits still require user approval). \
Prefer 'explore' unless the agent must change something. Spawn all independent agents in ONE spawn_agents \
call so they run in parallel; give agents non-overlapping scopes so parallel workers never edit the same file. \
\
IMPORTANT — Language policy: ALL communication must be in English. Write in English and ONLY in English, \
regardless of the language the user writes in. If the user writes in a non-English language, \
treat it as if they asked in English — respond in English only.";

/// Build the per-session system prompt. The result is byte-identical for every
/// request in the same workspace, so the provider's prefix cache stays warm.
fn system_prompt(workspace_root: Option<&str>, skills_section: Option<&str>) -> String {
    let base = match workspace_root {
        Some(root) => format!(
            "{SYSTEM_PROMPT}\n\nProject workspace root: {root}. \
The bash tool already runs with this directory as its working directory — run commands directly \
(e.g. \"git status\"), use relative paths, and never cd into guessed paths. \
File tools take absolute paths inside this root."
        ),
        None => SYSTEM_PROMPT.to_string(),
    };
    match skills_section {
        Some(s) if !s.is_empty() => format!("{base}\n{s}"),
        _ => base,
    }
}



#[derive(Debug, Clone, Serialize, Deserialize)]
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
    #[serde(rename = "SteeringInjected")]
    SteeringInjected {
        text: String,
    },
    #[serde(rename = "Error")]
    Error(String),
    #[serde(rename = "SubagentStarted")]
    SubagentStarted {
        #[serde(rename = "subagentId")]
        subagent_id: String,
        #[serde(rename = "parentToolId")]
        parent_tool_id: String,
        name: String,
        goal: String,
        mode: String,
    },
    #[serde(rename = "SubagentDone")]
    SubagentDone {
        #[serde(rename = "subagentId")]
        subagent_id: String,
        status: String,
        rounds: u32,
        #[serde(rename = "inputTokens")]
        input_tokens: u32,
        #[serde(rename = "outputTokens")]
        output_tokens: u32,
    },
    #[serde(rename = "Subagent")]
    Subagent {
        #[serde(rename = "subagentId")]
        subagent_id: String,
        event: Box<AgentEvent>,
    },
    #[serde(rename = "SessionStats")]
    SessionStats {
        #[serde(rename = "inputTokens")]
        input_tokens: u32,
        #[serde(rename = "outputTokens")]
        output_tokens: u32,
        #[serde(rename = "cumulativeCost")]
        cumulative_cost: Option<f64>,
        #[serde(rename = "contextTokens")]
        context_tokens: u64,
        #[serde(rename = "maxContextTokens")]
        max_context_tokens: u64,
        #[serde(rename = "compactThreshold")]
        compact_threshold: u64,
    },
}

#[derive(Debug, Clone, Serialize, Deserialize)]
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

/// Drain the steering queue, persist each message, merge into the last user turn
/// (or create a new one), and emit SteeringInjected events. Returns true if any
/// steering was injected.
fn inject_steering(
    history: &mut Vec<Message>,
    store: &SessionStore,
    steering: &SteeringCtl,
    event_tx: &Channel<AgentEvent>,
) -> bool {
    let messages = steering.drain();
    if messages.is_empty() {
        return false;
    }
    for text in &messages {
        store.try_append(&SessionRecord::Steering {
            text: text.clone(),
            ts: now_ms(),
        });
        push_user_blocks(history, store, vec![ContentBlock::text(text)]);
        let _ = event_tx.send(AgentEvent::SteeringInjected {
            text: text.clone(),
        });
    }
    true
}

/// Run a single continuous provider→tool loop for one user input, until the
/// model produces a turn with no tool calls. Shares one conversation history
/// (append-only, cache-friendly) and persists every step to the session JSONL
/// store. The model decides at each round whether it still needs a tool call
/// or can answer directly — there are no forced phases.
#[allow(clippy::too_many_arguments)]
/// Reject messages that are not written in English.
fn reject_non_english(msg: &str) -> Result<(), String> {
    let non_ascii: Vec<char> = msg.chars().filter(|&c| c > '\u{7E}').collect();
    if non_ascii.is_empty() {
        return Ok(());
    }
    let total = msg.chars().count() as f64;
    let ratio = non_ascii.len() as f64 / total;
    if ratio > 0.10 {
        let sample: String = non_ascii.iter().take(5).collect();
        return Err(format!(
            "Only English is supported. Please write your message in English. \
             (Detected non-English characters: {})",
            sample
        ));
    }
    Ok(())
}

/// Write a Status record with cumulative token/cost stats and the size of
/// the context for the next request.
fn write_status(
    store: &SessionStore,
    session_id: &str,
    cumul_in: u64,
    cumul_out: u64,
    cumul_cost: Option<f64>,
    context_tokens: Option<u64>,
) {
    store.try_append(&SessionRecord::Status {
        session_id: session_id.to_string(),
        total_input_tokens: cumul_in,
        total_output_tokens: cumul_out,
        total_cost: cumul_cost,
        context_tokens,
        ts: now_ms(),
    });
}

/// Per-million-token rates for a model (claudin.io official pricing).
struct Pricing {
    input: f64,
    cache_read: f64,
    output: f64,
}

fn model_pricing(model: &str) -> Pricing {
    if model.contains("claudius") {
        Pricing { input: 3.00, cache_read: 0.90, output: 8.00 }
    } else {
        // claudinio and unknown models: balanced tier
        Pricing { input: 0.50, cache_read: 0.15, output: 2.00 }
    }
}

/// Estimate cost for provider calls when the provider does not report cost.
fn cost_for(model: &str, input: u32, cache_read: u32, output: u32) -> f64 {
    let p = model_pricing(model);
    (input as f64 * p.input + cache_read as f64 * p.cache_read + output as f64 * p.output)
        / 1_000_000.0
}

pub async fn run_workflow(
    config: &AgentConfig,
    history: &mut Vec<Message>,
    user_message: String,
    attachment_blocks: Vec<ContentBlock>,
    event_tx: &Channel<AgentEvent>,
    approvals: &ApprovalMap,
    answers: &AnswerMap,
    session_id: &str,
    ctx: &ToolContext,
    store: &SessionStore,
    steering: &Arc<SteeringCtl>,
) -> Result<(), String> {
    reject_non_english(&user_message)?;
    store.try_append(&SessionRecord::User {
        text: user_message.clone(),
        ts: now_ms(),
    });
    let mut blocks = vec![ContentBlock::text(&user_message)];
    blocks.extend(attachment_blocks);
    push_user_blocks(history, store, blocks);

    let skill_mgr = crate::agent::skills::SkillManager::new(
        ctx.workspace_root.as_ref().map(std::path::PathBuf::from)
    );
    let skills_section = crate::agent::skills::build_skills_system_prompt_section(&skill_mgr);
    let system = system_prompt(ctx.workspace_root.as_deref(), skills_section.as_deref());
    let tools = api_tools();

    // Auto-compact when the context exceeds the threshold. Prefer the real
    // input_tokens the API reported for the last request; the char-based
    // estimate is the fallback (take the max of the two for safety).
    let records = crate::agent::persist::load_records(&store.path).unwrap_or_default();
    let estimated = estimate_tokens(history, &system, &tools)
        .max(crate::agent::persist::last_context_tokens(&records).unwrap_or(0));
    if estimated >= COMPACT_THRESHOLD {
        let _ = event_tx.send(AgentEvent::TextStep {
            text: format!(
                "📦 Contexto em ~{}k/{}k tokens — compactando…",
                estimated / 1000,
                MAX_CONTEXT_TOKENS / 1000
            ),
        });
        match compact_history(config, store, ctx, event_tx, approvals, answers, session_id, steering).await {
            Ok(_) => {
                // Rebuild the history exactly as a session reload would:
                // summary + kept-verbatim tail (which already contains the
                // just-persisted user message) + nothing else.
                *history = crate::agent::persist::history_from_records(
                    &crate::agent::persist::load_records(&store.path).unwrap_or_default(),
                );
                let new_context = estimate_tokens(history, &system, &tools);
                let (ci, co, cc) = crate::agent::persist::cumulative_stats(
                    &crate::agent::persist::load_records(&store.path).unwrap_or_default(),
                );
                write_status(store, session_id, ci, co, cc, Some(new_context));
                let _ = event_tx.send(AgentEvent::SessionStats {
                    input_tokens: ci as u32,
                    output_tokens: co as u32,
                    cumulative_cost: cc,
                    context_tokens: new_context,
                    max_context_tokens: MAX_CONTEXT_TOKENS,
                    compact_threshold: COMPACT_THRESHOLD,
                });
                let _ = event_tx.send(AgentEvent::TextStep {
                    text: format!(
                        "✅ Contexto compactado: ~{}k → ~{}k tokens.",
                        estimated / 1000,
                        new_context / 1000
                    ),
                });
            }
            Err(e) => {
                let _ = event_tx.send(AgentEvent::TextStep {
                    text: format!("⚠️ Falha na compactação: {e} — continuando com contexto cheio."),
                });
            }
        }
    }

    // Load cumulative totals from the last Status record
    let cumul = crate::agent::persist::cumulative_stats(
        &crate::agent::persist::load_records(&store.path).unwrap_or_default()
    );
    let mut cumul_in: u64 = cumul.0;
    let mut cumul_out: u64 = cumul.1;
    let mut cumul_cost: Option<f64> = cumul.2;

    let mut total_in: u32 = 0;
    let mut total_out: u32 = 0;
    let mut total_cache: u32 = 0;
    let mut run_cost: Option<f64> = None;
    let mut last_text = String::new();
    // Size of the context for the next request: the real number reported by
    // the API when available, the char-based estimate otherwise.
    let mut last_context: u64 = estimate_tokens(history, &system, &tools);
    let mut truncation_streak: u32 = 0;

    let max_rounds = config.max_rounds.unwrap_or(usize::MAX);
    for _ in 0..max_rounds {
        let mut assistant_text = String::new();
        let stream_output = provider::stream_message(
            config,
            history,
            &tools,
            Some(system.as_str()),
            event_tx,
            session_id,
            &mut assistant_text,
            &steering.interrupt,
        )
        .await?;

        let text_output = assistant_text;
        let tool_uses = stream_output.tool_uses;
        let was_interrupted = stream_output.interrupted;
        if let Some(u) = &stream_output.usage {
            total_in += u.input_tokens;
            total_out += u.output_tokens;
            total_cache += u.cache_read_input_tokens;
            // Use provider-reported cost if available, otherwise estimate
            if let Some(c) = u.cost {
                run_cost = Some(run_cost.unwrap_or(0.0) + c);
            }
        }
        // Context for the next request = the history just sent + this round's
        // output. Providers behind a prefix cache (claudin.io) report only
        // cache-miss tokens in input_tokens — verified: a fully cached 3k
        // prompt reports input_tokens=74 — so the char-based estimate is the
        // floor and the API number can only raise it, never shrink it.
        let out_tok = stream_output
            .usage
            .as_ref()
            .map(|u| u.output_tokens as u64)
            .unwrap_or(0);
        let api_ctx = stream_output
            .usage
            .as_ref()
            .map(|u| (u.input_tokens + u.cache_read_input_tokens + u.output_tokens) as u64)
            .unwrap_or(0);
        last_context = (estimate_tokens(history, &system, &tools) + out_tok).max(api_ctx);

        // Live stats for the context bar
        let round_cost = run_cost.unwrap_or_else(|| cost_for(&config.model, total_in, total_cache, total_out));
        let live_cost = cumul_cost.unwrap_or(0.0) + round_cost;
        let _ = event_tx.send(AgentEvent::SessionStats {
            input_tokens: total_in + cumul_in as u32,
            output_tokens: total_out + cumul_out as u32,
            cumulative_cost: Some(live_cost),
            context_tokens: last_context,
            max_context_tokens: MAX_CONTEXT_TOKENS,
            compact_threshold: COMPACT_THRESHOLD,
        });

        // A — Interrupt no meio do stream: persistir texto parcial se não vazio,
        //     resetar flag, tentar steering ou pausar.
        if was_interrupted {
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
            steering.interrupt.store(false, Ordering::SeqCst);
            if inject_steering(history, store, steering, event_tx) {
                continue;
            }
            if last_text.is_empty() {
                last_text = "Pausado pelo usuário.".into();
            }
            store.try_append(&SessionRecord::Done {
                input_tokens: total_in,
                output_tokens: total_out,
                ts: now_ms(),
            });
            cumul_in += total_in as u64;
            cumul_out += total_out as u64;
            let cost = run_cost.unwrap_or_else(|| cost_for(&config.model, total_in, total_cache, total_out));
            cumul_cost = Some(cumul_cost.unwrap_or(0.0) + cost);
            write_status(store, session_id, cumul_in, cumul_out, cumul_cost, Some(last_context));
            let _ = event_tx.send(AgentEvent::Done {
                stop_reason: "interrupted".into(),
                text_output: last_text,
                input_tokens: total_in,
                output_tokens: total_out,
            });
            return Ok(());
        }

        // Truncated at the output-token cap (stop_reason "max_tokens"): the
        // model was cut off mid-generation, so an empty tool list here does
        // NOT mean the turn is done. Persist any partial text and nudge the
        // model to continue instead of silently abandoning the task. When
        // complete tool calls did come through, fall through and run them —
        // the loop continues naturally.
        let truncated = stream_output.stop_reason.as_deref() == Some("max_tokens");
        if truncated && tool_uses.is_empty() {
            truncation_streak += 1;
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
            if truncation_streak < 3 {
                push_user_blocks(
                    history,
                    store,
                    vec![ContentBlock::text(
                        "[system] Your previous response was cut off at the output token \
                         limit before completing a tool call or final answer. Continue from \
                         where you stopped, working in smaller steps — if you were emitting \
                         a large tool call (e.g. a whole-file edit), split it into several \
                         smaller edits.",
                    )],
                );
                continue;
            }
            // Three consecutive fruitless truncations: stop honestly instead
            // of burning the whole round budget.
            if last_text.is_empty() {
                last_text = "A resposta estourou o limite de tokens repetidamente sem concluir. \
                             Tente dividir o pedido em partes menores."
                    .into();
            } else {
                last_text =
                    format!("{last_text}\n\n(Resposta truncada no limite de tokens — pode não estar completa.)");
            }
            store.try_append(&SessionRecord::Done {
                input_tokens: total_in,
                output_tokens: total_out,
                ts: now_ms(),
            });
            cumul_in += total_in as u64;
            cumul_out += total_out as u64;
            let cost = run_cost.unwrap_or_else(|| cost_for(&config.model, total_in, total_cache, total_out));
            cumul_cost = Some(cumul_cost.unwrap_or(0.0) + cost);
            write_status(store, session_id, cumul_in, cumul_out, cumul_cost, Some(last_context));
            let _ = event_tx.send(AgentEvent::Done {
                stop_reason: "max_tokens".into(),
                text_output: last_text,
                input_tokens: total_in,
                output_tokens: total_out,
            });
            return Ok(());
        }
        if !truncated {
            truncation_streak = 0;
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
            // B — Antes de encerrar, verificar steering. Se houver, continuar.
            if inject_steering(history, store, steering, event_tx) {
                continue;
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
            cumul_in += total_in as u64;
            cumul_out += total_out as u64;
            let cost = run_cost.unwrap_or_else(|| cost_for(&config.model, total_in, total_cache, total_out));
            cumul_cost = Some(cumul_cost.unwrap_or(0.0) + cost);
            write_status(store, session_id, cumul_in, cumul_out, cumul_cost, Some(last_context));
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

        for (ti, tool_use) in tool_uses.iter().enumerate() {
            // C — Entre tools: checar interrupt. Se setado, sintetizar
            // tool_result "interrompido" para este e todos os tool_uses restantes.
            if steering.interrupt.load(Ordering::SeqCst) {
                for remaining in tool_uses.iter().skip(ti) {
                    let tid = remaining
                        .get("id")
                        .and_then(|v| v.as_str())
                        .unwrap_or("")
                        .to_string();
                    let tname = remaining
                        .get("name")
                        .and_then(|v| v.as_str())
                        .unwrap_or("")
                        .to_string();
                    tool_assistant_blocks.push(ContentBlock::tool_use(
                        &tid,
                        &tname,
                        remaining.get("input").cloned().unwrap_or(Value::Null),
                    ));
                    let msg = "Interrompido pelo usuário — ferramenta não executada.";
                    let _ = event_tx.send(AgentEvent::ToolResult {
                        tool_id: tid.clone(),
                        tool_name: tname,
                        output: msg.into(),
                        error: None,
                    });
                    tool_result_blocks.push(ContentBlock::tool_result(&tid, msg));
                }
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

            tool_assistant_blocks.push(ContentBlock::tool_use(
                &tool_use_id,
                &tool_name,
                tool_input.clone(),
            ));

            let block = if tool_name == "spawn_agents" {
                let (block, sub_in, sub_out) = subagent::run_spawn_agents(
                    config,
                    ctx,
                    &tool_use_id,
                    tool_input,
                    event_tx,
                    approvals,
                    answers,
                    session_id,
                    steering,
                )
                .await;
                total_in += sub_in;
                total_out += sub_out;
                block
            } else {
                run_tool(
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
                .await
            };
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

        // D — Pós tool_results: checar interrupt ou steering.
        if steering.interrupt.swap(false, Ordering::SeqCst) {
            if inject_steering(history, store, steering, event_tx) {
                continue;
            }
            if last_text.is_empty() {
                last_text = "Pausado pelo usuário.".into();
            }
            store.try_append(&SessionRecord::Done {
                input_tokens: total_in,
                output_tokens: total_out,
                ts: now_ms(),
            });
            cumul_in += total_in as u64;
            cumul_out += total_out as u64;
            let cost = run_cost.unwrap_or_else(|| cost_for(&config.model, total_in, total_cache, total_out));
            cumul_cost = Some(cumul_cost.unwrap_or(0.0) + cost);
            write_status(store, session_id, cumul_in, cumul_out, cumul_cost, Some(last_context));
            let _ = event_tx.send(AgentEvent::Done {
                stop_reason: "interrupted".into(),
                text_output: last_text,
                input_tokens: total_in,
                output_tokens: total_out,
            });
            return Ok(());
        }
        inject_steering(history, store, steering, event_tx);
    }

    // Safety cap hit: stop looping and report what we have so far rather than
    // running forever. Only reachable when config.max_rounds is set.
    let capped_text = if last_text.is_empty() {
        format!("Parei após {max_rounds} rounds de ferramentas sem concluir. Tente reformular o pedido em partes menores.")
    } else {
        format!("{last_text}\n\n(Parei após {max_rounds} rounds de ferramentas — pode não estar completo.)")
    };
    store.try_append(&SessionRecord::Done {
        input_tokens: total_in,
        output_tokens: total_out,
        ts: now_ms(),
    });
    cumul_in += total_in as u64;
    cumul_out += total_out as u64;
    let cost = run_cost.unwrap_or_else(|| cost_for(&config.model, total_in, total_cache, total_out));
    cumul_cost = Some(cumul_cost.unwrap_or(0.0) + cost);
    write_status(store, session_id, cumul_in, cumul_out, cumul_cost, Some(last_context));
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
    use serde_json::json;

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

    #[test]
    fn agent_event_round_trip_text_step() {
        let ev = AgentEvent::TextStep { text: "hello".into() };
        let json = serde_json::to_value(&ev).unwrap();
        let back: AgentEvent = serde_json::from_value(json).unwrap();
        assert!(matches!(back, AgentEvent::TextStep { text } if text == "hello"));
    }

    #[test]
    fn agent_event_round_trip_thinking() {
        let ev = AgentEvent::Thinking("thinking text".into());
        let json = serde_json::to_value(&ev).unwrap();
        let back: AgentEvent = serde_json::from_value(json).unwrap();
        assert!(matches!(back, AgentEvent::Thinking(t) if t == "thinking text"));
    }

    #[test]
    fn agent_event_round_trip_tool_call() {
        let ev = AgentEvent::ToolCall {
            session_id: "s1".into(),
            tool_id: "t1".into(),
            tool_name: "read_file".into(),
            args: json!({"path": "/foo"}),
            permission: "auto".into(),
            edit_proposal: None,
        };
        let json = serde_json::to_value(&ev).unwrap();
        let back: AgentEvent = serde_json::from_value(json).unwrap();
        match back {
            AgentEvent::ToolCall { session_id, tool_id, tool_name, .. } => {
                assert_eq!(session_id, "s1");
                assert_eq!(tool_id, "t1");
                assert_eq!(tool_name, "read_file");
            }
            _ => panic!("expected ToolCall"),
        }
    }

    #[test]
    fn agent_event_round_trip_tool_result() {
        let ev = AgentEvent::ToolResult {
            tool_id: "t1".into(),
            tool_name: "read_file".into(),
            output: "content".into(),
            error: None,
        };
        let json = serde_json::to_value(&ev).unwrap();
        let back: AgentEvent = serde_json::from_value(json).unwrap();
        assert!(matches!(back, AgentEvent::ToolResult { tool_id, .. } if tool_id == "t1"));
    }

    #[test]
    fn agent_event_round_trip_done() {
        let ev = AgentEvent::Done {
            stop_reason: "end_turn".into(),
            text_output: "done".into(),
            input_tokens: 10,
            output_tokens: 20,
        };
        let json = serde_json::to_value(&ev).unwrap();
        let back: AgentEvent = serde_json::from_value(json).unwrap();
        match back {
            AgentEvent::Done { stop_reason, input_tokens, output_tokens, .. } => {
                assert_eq!(stop_reason, "end_turn");
                assert_eq!(input_tokens, 10);
                assert_eq!(output_tokens, 20);
            }
            _ => panic!("expected Done"),
        }
    }

    #[test]
    fn agent_event_round_trip_subagent_started() {
        let ev = AgentEvent::SubagentStarted {
            subagent_id: "sa1".into(),
            parent_tool_id: "pt1".into(),
            name: "explorer".into(),
            goal: "find stuff".into(),
            mode: "explore".into(),
        };
        let json = serde_json::to_value(&ev).unwrap();
        let back: AgentEvent = serde_json::from_value(json).unwrap();
        match back {
            AgentEvent::SubagentStarted { subagent_id, name, .. } => {
                assert_eq!(subagent_id, "sa1");
                assert_eq!(name, "explorer");
            }
            _ => panic!("expected SubagentStarted"),
        }
    }

    #[test]
    fn agent_event_round_trip_subagent_done() {
        let ev = AgentEvent::SubagentDone {
            subagent_id: "sa1".into(),
            status: "completed".into(),
            rounds: 5,
            input_tokens: 100,
            output_tokens: 50,
        };
        let json = serde_json::to_value(&ev).unwrap();
        let back: AgentEvent = serde_json::from_value(json).unwrap();
        match back {
            AgentEvent::SubagentDone { subagent_id, status, rounds, .. } => {
                assert_eq!(subagent_id, "sa1");
                assert_eq!(status, "completed");
                assert_eq!(rounds, 5);
            }
            _ => panic!("expected SubagentDone"),
        }
    }

    #[test]
    fn agent_event_round_trip_subagent_wrapped() {
        let inner = AgentEvent::Thinking("inner thought".into());
        let ev = AgentEvent::Subagent {
            subagent_id: "sa1".into(),
            event: Box::new(inner),
        };
        let json = serde_json::to_value(&ev).unwrap();
        let back: AgentEvent = serde_json::from_value(json).unwrap();
        match back {
            AgentEvent::Subagent { subagent_id, event } => {
                assert_eq!(subagent_id, "sa1");
                assert!(matches!(*event, AgentEvent::Thinking(t) if t == "inner thought"));
            }
            _ => panic!("expected Subagent"),
        }
    }

    #[test]
    fn agent_event_round_trip_error() {
        let ev = AgentEvent::Error("something broke".into());
        let json = serde_json::to_value(&ev).unwrap();
        let back: AgentEvent = serde_json::from_value(json).unwrap();
        assert!(matches!(back, AgentEvent::Error(e) if e == "something broke"));
    }

    #[test]
    fn agent_event_round_trip_steering_injected() {
        let ev = AgentEvent::SteeringInjected { text: "steer".into() };
        let json = serde_json::to_value(&ev).unwrap();
        let back: AgentEvent = serde_json::from_value(json).unwrap();
        assert!(matches!(back, AgentEvent::SteeringInjected { text } if text == "steer"));
    }

    #[test]
    fn agent_event_round_trip_ask_user() {
        let ev = AgentEvent::AskUser {
            session_id: "s1".into(),
            tool_id: "t1".into(),
            questions: json!([{"question": "q?", "options": ["a", "b"]}]),
        };
        let json = serde_json::to_value(&ev).unwrap();
        let back: AgentEvent = serde_json::from_value(json).unwrap();
        match back {
            AgentEvent::AskUser { session_id, tool_id, .. } => {
                assert_eq!(session_id, "s1");
                assert_eq!(tool_id, "t1");
            }
            _ => panic!("expected AskUser"),
        }
    }

    #[test]
    fn agent_event_round_trip_session_stats() {
        let ev = AgentEvent::SessionStats {
            input_tokens: 500,
            output_tokens: 200,
            cumulative_cost: Some(0.003),
            context_tokens: 42_000,
            max_context_tokens: MAX_CONTEXT_TOKENS,
            compact_threshold: COMPACT_THRESHOLD,
        };
        let json = serde_json::to_value(&ev).unwrap();
        assert_eq!(json["data"]["contextTokens"], 42_000);
        assert_eq!(json["data"]["maxContextTokens"], 256_000);
        let back: AgentEvent = serde_json::from_value(json).unwrap();
        match back {
            AgentEvent::SessionStats {
                input_tokens,
                output_tokens,
                cumulative_cost,
                context_tokens,
                ..
            } => {
                assert_eq!(input_tokens, 500);
                assert_eq!(output_tokens, 200);
                assert_eq!(cumulative_cost, Some(0.003));
                assert_eq!(context_tokens, 42_000);
            }
            _ => panic!("expected SessionStats"),
        }
    }

    #[test]
    fn session_stats_without_cost() {
        let ev = AgentEvent::SessionStats {
            input_tokens: 100,
            output_tokens: 50,
            cumulative_cost: None,
            context_tokens: 0,
            max_context_tokens: MAX_CONTEXT_TOKENS,
            compact_threshold: COMPACT_THRESHOLD,
        };
        let json = serde_json::to_value(&ev).unwrap();
        let back: AgentEvent = serde_json::from_value(json).unwrap();
        match back {
            AgentEvent::SessionStats { cumulative_cost, .. } => {
                assert_eq!(cumulative_cost, None);
            }
            _ => panic!("expected SessionStats"),
        }
    }

    #[test]
    fn estimate_tokens_returns_reasonable_value() {
        let msg = Message {
            role: "user".into(),
            content: vec![ContentBlock::text("hello world")],
        };
        let history = vec![msg];
        let system = "You are a helpful assistant.";
        let tools = vec![];
        let estimated = estimate_tokens(&history, system, &tools);
        assert!(estimated > 0, "should estimate some tokens");
        assert!(estimated < 1000, "short message should be < 1k tokens");
    }

    #[test]
    fn estimate_tokens_increases_with_history() {
        let msg1 = Message {
            role: "user".into(),
            content: vec![ContentBlock::text("a".repeat(1000))],
        };
        let msg2 = Message {
            role: "assistant".into(),
            content: vec![ContentBlock::text("b".repeat(1000))],
        };
        let small = estimate_tokens(&[msg1.clone()], "", &[]);
        let large = estimate_tokens(&[msg1, msg2], "", &[]);
        assert!(large > small, "more history should mean more tokens");
    }

    #[test]
    fn compute_tail_turns_covers_last_two_exchanges() {
        let user = |t: &str| SessionRecord::Turn {
            message: Message { role: "user".into(), content: vec![ContentBlock::text(t)] },
            ts: 0,
        };
        let asst = |t: &str| SessionRecord::Turn {
            message: Message { role: "assistant".into(), content: vec![ContentBlock::text(t)] },
            ts: 0,
        };
        let recs = vec![
            user("q1"), asst("a1"),
            user("q2"), asst("a2"),
            user("q3"), asst("a3"),
        ];
        // Last 2 exchanges = q2..a3 = 4 Turn records
        assert_eq!(compute_tail_turns(&recs), 4);
    }

    #[test]
    fn compute_tail_turns_shrinks_when_over_budget() {
        let big = "x".repeat((TAIL_MAX_TOKENS as usize) * 4); // way over budget alone
        let recs = vec![
            SessionRecord::Turn {
                message: Message { role: "user".into(), content: vec![ContentBlock::text(big)] },
                ts: 0,
            },
            SessionRecord::Turn {
                message: Message { role: "assistant".into(), content: vec![ContentBlock::text("a")] },
                ts: 0,
            },
        ];
        assert_eq!(compute_tail_turns(&recs), 0, "oversized tail must be dropped");
    }

    #[test]
    fn cost_claudinio_rates() {
        // claudinio: $0.50/M input, $0.15/M cache read, $2.00/M output
        let cost = cost_for("claudinio", 1000, 0, 500);
        assert!((cost - 0.0015).abs() < 0.0001, "expected ~$0.0015, got {cost}");
    }

    #[test]
    fn cost_claudius_rates() {
        // claudius: $3.00/M input, $0.90/M cache read, $8.00/M output
        let cost = cost_for("claudius", 1000, 0, 500);
        assert!((cost - 0.007).abs() < 0.0001, "expected ~$0.007, got {cost}");
    }

    #[test]
    fn cost_includes_cache_read() {
        // 1M cache-read tokens at claudinio rates = $0.15
        let cost = cost_for("claudinio", 0, 1_000_000, 0);
        assert!((cost - 0.15).abs() < 0.0001, "expected ~$0.15, got {cost}");
    }

    #[test]
    fn cost_unknown_model_falls_back_to_claudinio() {
        assert_eq!(
            cost_for("some-other-model", 1000, 100, 500),
            cost_for("claudinio", 1000, 100, 500)
        );
    }

    #[test]
    fn compact_threshold_is_75_percent_of_window() {
        assert_eq!(MAX_CONTEXT_TOKENS, 256_000);
        assert_eq!(COMPACT_THRESHOLD, 192_000);
    }
}
