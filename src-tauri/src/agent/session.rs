//! A session's shared state, and the plumbing every part of a run uses: the
//! mode and steering controls the UI and the running workflow share, the
//! approval and answer maps, token estimation, history helpers, status and
//! cost.
//!
//! It is also the facade of the agent loop. The loop lived here whole until
//! this file reached 8,446 lines; it was split by responsibility (lean-harness
//! plan, Phase 8) into `prompt`, `events`, `judge`, `handoff`, `retry`,
//! `run_loop` and `tool_exec`. The names the rest of the crate reaches through
//! `agent::session` are re-exported below, so those call sites did not move.

use crate::agent::persist::{SessionRecord, SessionStore, now_ms};
use crate::agent::provider::{ContentBlock, Message, ToolDescription, ToolResultContent};
use crate::agent::tools::ToolContext;
use serde::{Deserialize, Serialize};
use std::collections::HashMap;
use std::sync::atomic::{AtomicBool, Ordering};
use std::sync::{Arc, Mutex as StdMutex};
use tauri::ipc::Channel;
use tokio::sync::{Mutex, oneshot};

// The names other modules reach through `agent::session`, from where they live
// now (see the module docs).
pub use crate::agent::events::AgentEvent;
pub use crate::agent::handoff::{HandoffReason, HandoffSpec, RunOutcome, compact_history};
pub(crate) use crate::agent::handoff::{accept_prune, shed_options};
pub(crate) use crate::agent::judge::loop_verdict;
pub use crate::agent::run_loop::{run_workflow, run_workflow_with_profile};
pub(crate) use crate::agent::tool_exec::{
    MAX_STOP_HOOK_BLOCKS, deny_tool, fit_round, run_tool, tool_response_for_hook,
};

/// Context window of the supported models (claudinio and claudius: 256K).
pub const MAX_CONTEXT_TOKENS: u64 = 200_000;

/// Threshold for auto-compaction: if the context exceeds this before a
/// request, the history is compacted first (75% of the window).
pub const COMPACT_THRESHOLD: u64 = MAX_CONTEXT_TOKENS * 75 / 100;

/// Prefix that identifies a golden task.
pub const GOLDEN_TASK_PREFIX: &str = "golden-";

/// Parse <goal>...</goal> tags from user input.
/// Returns (cleaned_text, list_of_goals).
pub fn parse_goals(text: &str) -> (String, Vec<String>) {
    let re = regex::Regex::new(r"<goal>(.*?)</goal>").unwrap();
    let mut goals = Vec::new();
    for cap in re.captures_iter(text) {
        let goal_text = cap[1].trim().to_string();
        if !goal_text.is_empty() {
            goals.push(goal_text);
        }
    }
    let cleaned = re.replace_all(text, "").to_string();
    let cleaned = cleaned.trim().to_string();
    (cleaned, goals)
}

/// Rough token estimation per message. For image blocks, estimates based on
/// pixel dimensions (w*h/750) since base64 serialization overestimates ~50x.
/// Other blocks use serialized length / 3.
pub(super) fn estimate_message_tokens(msg: &Message) -> u64 {
    // Per-message overhead for the role field and envelope (~4 tokens)
    let mut total: u64 = 4;
    for block in &msg.content {
        total += estimate_block_tokens(block);
    }
    total
}

fn estimate_block_tokens(block: &ContentBlock) -> u64 {
    match block {
        ContentBlock::Image { source, .. } => {
            if source.width > 0 && source.height > 0 {
                // Anthropic's cost model: ~w*h/750 tokens per image
                (source.width as u64 * source.height as u64) / 750
            } else {
                // Conservative fallback: max cost of a 1568px image
                1_600
            }
        }
        // A tool result carrying images has to recurse: the generic branch
        // below prices a block by its serialized length, and base64 image data
        // measured that way overestimates by ~50x — enough to trip the handoff
        // and compaction thresholds on a single screenshot.
        ContentBlock::ToolResult {
            content: ToolResultContent::Blocks(blocks),
            ..
        } => 4 + blocks.iter().map(estimate_block_tokens).sum::<u64>(),
        _ => {
            // Serialize only this block (not the full message) to estimate tokens
            match serde_json::to_string(block) {
                Ok(json) => json.len() as u64 / 3,
                Err(_) => 0,
            }
        }
    }
}

pub(crate) fn estimate_tokens(history: &[Message], system: &str, tools: &[ToolDescription]) -> u64 {
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

/// Itemize the fixed prefix of a request by the same chars/3 rule as
/// `estimate_tokens`, so the parts add up to what the context meter counts.
/// `system` is the whole prompt; the skills and spec sections are carved out
/// of it because they are the parts a workspace, not the app, decides.
pub(crate) fn prefix_breakdown(
    system: &str,
    skills_section: Option<&str>,
    spec_section: Option<&str>,
    tools: &[ToolDescription],
) -> crate::agent::persist::PrefixBreakdown {
    // A profile that leaves a section out of its prompt (GitSync) must not be
    // charged for it.
    let chars =
        |s: Option<&str>| s.filter(|s| system.contains(*s)).map(str::len).unwrap_or(0) as u64;
    let skills = chars(skills_section);
    let specs = chars(spec_section);
    let schema_chars = |mcp: bool| -> u64 {
        tools
            .iter()
            .filter(|t| t.name.starts_with("mcp__") == mcp)
            .map(|t| serde_json::to_string(t).unwrap_or_default().len() as u64)
            .sum()
    };
    crate::agent::persist::PrefixBreakdown {
        system: (system.len() as u64).saturating_sub(skills + specs) / 3,
        skills: skills / 3,
        specs: specs / 3,
        tools: schema_chars(false) / 3,
        mcp: schema_chars(true) / 3,
    }
}

/// The session's operating mode: Brain plans with read-only tools,
/// Builder executes with the full toolset.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "lowercase")]
pub enum SessionMode {
    Brain,
    Builder,
}

impl SessionMode {
    pub fn as_str(&self) -> &'static str {
        match self {
            SessionMode::Brain => "brain",
            SessionMode::Builder => "builder",
        }
    }

    pub fn parse(s: &str) -> Option<SessionMode> {
        match s {
            // "pensador"/"constructor" are the original names of these modes;
            // JSONL files written before the rename still carry them.
            "brain" | "pensador" => Some(SessionMode::Brain),
            "builder" | "constructor" => Some(SessionMode::Builder),
            _ => None,
        }
    }
}

/// Which situational prompt/toolset a workflow run uses. `Standard` is the
/// full agent (task system, Brain/Builder modes, skills, subagents, golden
/// tasks). Other variants are lean, purpose-built profiles for a single kind
/// of job — no task system, no modes, minimal toolset — so the model isn't
/// paying for ceremony it doesn't need.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum PromptProfile {
    Standard,
    /// Commit & push: a single-purpose git operator. Bash + ask_user only.
    GitSync,
    /// A chat session on a window too small for `Standard`: one short prompt,
    /// ten tools, and the session edits files itself. Never requested by a
    /// caller — `surface::effective_profile` picks it from the model's window.
    Compact,
}

impl PromptProfile {
    /// True for the profiles a person is chatting through. What is theirs —
    /// their prompt hooks, their Stop hook, the handoff that keeps a long
    /// conversation going — applies to these and not to a single-purpose job.
    pub fn is_chat(self) -> bool {
        matches!(self, PromptProfile::Standard | PromptProfile::Compact)
    }

    pub fn as_str(self) -> &'static str {
        match self {
            PromptProfile::Standard => "standard",
            PromptProfile::GitSync => "git_sync",
            PromptProfile::Compact => "compact",
        }
    }
}

/// Who put the session in its current mode. The agent may only exit Brain
/// on its own if it was the one who entered it; a human-initiated Brain
/// can only be exited by the human toggle.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "lowercase")]
pub enum ModeOrigin {
    Human,
    Agent,
}

impl ModeOrigin {
    pub fn as_str(&self) -> &'static str {
        match self {
            ModeOrigin::Human => "human",
            ModeOrigin::Agent => "agent",
        }
    }
}

/// Shared, mutable mode state for a session. Lives in AppState keyed by
/// session id so the UI toggle and the running workflow see the same value.
/// The Mutex is never held across await.
pub struct ModeCtl {
    state: StdMutex<(SessionMode, ModeOrigin)>,
    /// The user left the choice of phase to the harness for the next prompt.
    /// Consumed by the run that routes it, so it applies to one prompt only.
    auto: AtomicBool,
}

impl ModeCtl {
    pub fn new(mode: SessionMode, origin: ModeOrigin) -> Self {
        Self {
            state: StdMutex::new((mode, origin)),
            auto: AtomicBool::new(false),
        }
    }

    /// Set by the command layer from what the mode control sent with a prompt.
    pub fn request_auto(&self, auto: bool) {
        self.auto.store(auto, Ordering::SeqCst);
    }

    /// Whether Auto was requested, clearing it.
    pub fn take_auto(&self) -> bool {
        self.auto.swap(false, Ordering::SeqCst)
    }

    pub fn get(&self) -> (SessionMode, ModeOrigin) {
        *self.state.lock().unwrap()
    }

    pub fn set(&self, mode: SessionMode, origin: ModeOrigin) {
        *self.state.lock().unwrap() = (mode, origin);
    }
}

/// A single steering message: text + pre-processed attachment data.
pub struct SteeringEntry {
    pub text: String,
    pub attachments: Vec<(ContentBlock, crate::agent::persist::AttachmentMeta)>,
}

/// Steering: a queue of mid-run user messages and an interrupt flag.
/// Thread-safe; the Mutex is never held across await.
pub struct SteeringCtl {
    pub queue: StdMutex<Vec<SteeringEntry>>,
    pub interrupt: Arc<AtomicBool>,
}

impl SteeringCtl {
    pub fn new() -> Self {
        Self {
            queue: StdMutex::new(Vec::new()),
            interrupt: Arc::new(AtomicBool::new(false)),
        }
    }

    pub fn drain(&self) -> Vec<SteeringEntry> {
        let mut q = self.queue.lock().unwrap();
        std::mem::take(&mut *q)
    }

    pub fn push(&self, entry: SteeringEntry) {
        let mut q = self.queue.lock().unwrap();
        q.push(entry);
    }

    /// Whether a message is waiting to be injected.
    pub fn has_pending(&self) -> bool {
        !self.queue.lock().unwrap().is_empty()
    }

    pub fn clear(&self) {
        self.queue.lock().unwrap().clear();
        self.interrupt.store(false, Ordering::SeqCst);
    }
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

/// Push a message onto history and persist it as a Turn record.
pub(super) fn push_turn(
    history: &mut Vec<Message>,
    store: &SessionStore,
    ctx: &ToolContext,
    message: Message,
) {
    store.try_append(&SessionRecord::Turn {
        message: message.clone(),
        ts: now_ms(),
    });
    crate::agent::persist::invalidate_cache(&store.path, &ctx.records_cache);
    history.push(message);
}

/// Add user-role content, merging into the previous message when it is already a
/// user turn. The Anthropic API requires strictly alternating roles, so this
/// prevents two consecutive user turns (which can happen when the model returns
/// nothing). Merges are intentionally not persisted as a new Turn record,
/// keeping the JSONL history alternating on reopen as well.
pub(super) fn push_user_blocks(
    history: &mut Vec<Message>,
    store: &SessionStore,
    ctx: &ToolContext,
    blocks: Vec<ContentBlock>,
) {
    if let Some(last) = history.last_mut()
        && last.role == "user"
    {
        last.content.extend(blocks);
        return;
    }
    push_turn(
        history,
        store,
        ctx,
        Message {
            role: "user".into(),
            content: blocks,
        },
    );
}

/// Drain the steering queue, persist each message, merge into the last user turn
/// (or create a new one), and emit SteeringInjected events. Returns true if any
/// steering was injected.
pub(super) fn inject_steering(
    history: &mut Vec<Message>,
    store: &SessionStore,
    ctx: &ToolContext,
    steering: &SteeringCtl,
    event_tx: &Channel<AgentEvent>,
) -> bool {
    let entries = steering.drain();
    if entries.is_empty() {
        return false;
    }
    for entry in &entries {
        // Build content blocks: text first, then attachments
        let mut blocks = vec![ContentBlock::text(&entry.text)];
        let mut attachment_metas: Vec<crate::agent::persist::AttachmentMeta> = Vec::new();
        for (block, meta) in &entry.attachments {
            blocks.push(block.clone());
            attachment_metas.push(meta.clone());
        }
        store.try_append(&SessionRecord::Steering {
            text: entry.text.clone(),
            attachments: Some(attachment_metas.clone()),
            ts: now_ms(),
        });
        crate::agent::persist::invalidate_cache(&store.path, &ctx.records_cache);
        push_user_blocks(history, store, ctx, blocks);
        let _ = event_tx.send(AgentEvent::SteeringInjected {
            text: entry.text.clone(),
            attachments: Some(attachment_metas),
        });
    }
    true
}

/// Write a Status record with cumulative token/cost stats and the size of
/// the context for the next request.
#[allow(clippy::too_many_arguments)]
pub(super) fn write_status(
    store: &SessionStore,
    ctx: &ToolContext,
    session_id: &str,
    cumul_in: u64,
    cumul_out: u64,
    cumul_cost: Option<f64>,
    cumul_cost_input: Option<f64>,
    cumul_cost_output: Option<f64>,
    cumul_cost_cache_read: Option<f64>,
    context_tokens: Option<u64>,
) {
    store.try_append(&SessionRecord::Status {
        session_id: session_id.to_string(),
        total_input_tokens: cumul_in,
        total_output_tokens: cumul_out,
        total_cost: cumul_cost,
        total_cost_input: cumul_cost_input,
        total_cost_output: cumul_cost_output,
        total_cost_cache_read: cumul_cost_cache_read,
        context_tokens,
        ts: now_ms(),
    });
    crate::agent::persist::invalidate_cache(&store.path, &ctx.records_cache);
}

/// Per-million-token rates for a model (claudin.io official pricing).
/// Fallback estimate for when the litellm proxy's cost_injector middleware
/// doesn't report a real breakdown (unpriced model, older proxy deploy).
pub(crate) struct Pricing {
    input: f64,
    cache_read: f64,
    output: f64,
}

pub(crate) fn model_pricing(model: &str) -> Pricing {
    if model.contains("claudius") {
        Pricing {
            input: 3.00,
            cache_read: 0.90,
            output: 8.00,
        }
    } else {
        // claudinio and unknown models: balanced tier
        Pricing {
            input: 0.50,
            cache_read: 0.15,
            output: 2.00,
        }
    }
}

/// Cost broken down by token category, in USD.
pub(crate) struct CostBreakdown {
    pub(crate) input: f64,
    pub(crate) output: f64,
    pub(crate) cache_read: f64,
}

/// Estimate cost breakdown for provider calls when the provider does not
/// report a real cost breakdown.
pub(crate) fn cost_breakdown_for(
    model: &str,
    input: u32,
    cache_read: u32,
    output: u32,
) -> CostBreakdown {
    let p = model_pricing(model);
    CostBreakdown {
        input: input as f64 * p.input / 1_000_000.0,
        output: output as f64 * p.output / 1_000_000.0,
        cache_read: cache_read as f64 * p.cache_read / 1_000_000.0,
    }
}

/// This round's cost breakdown: the provider-reported values when present,
/// otherwise the local per-million-token estimate.
#[allow(clippy::too_many_arguments)]
pub(crate) fn cost_or_estimate(
    model: &str,
    total_in: u32,
    total_cache: u32,
    total_out: u32,
    run_cost_input: Option<f64>,
    run_cost_output: Option<f64>,
    run_cost_cache: Option<f64>,
) -> (f64, f64, f64) {
    if run_cost_input.is_none() && run_cost_output.is_none() && run_cost_cache.is_none() {
        let b = cost_breakdown_for(model, total_in, total_cache, total_out);
        (b.input, b.output, b.cache_read)
    } else {
        (
            run_cost_input.unwrap_or(0.0),
            run_cost_output.unwrap_or(0.0),
            run_cost_cache.unwrap_or(0.0),
        )
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use lru::LruCache;

    use std::num::NonZeroUsize;

    fn tmp_store() -> SessionStore {
        SessionStore {
            path: std::env::temp_dir().join(format!("claudinio_test_{}.jsonl", now_ms())),
        }
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
            mcp: None,
            mode_ctl: None,
            index_progress: None,
        }
    }

    /// A screenshot in a tool result must be priced by its pixels, not by the
    /// length of its base64. The generic arm serializes the block and divides
    /// by 3, which overestimates an image by roughly 50x — enough to trip the
    /// handoff and compaction thresholds on a single capture, and it re-runs
    /// over the whole history every round.
    #[test]
    fn image_in_a_tool_result_is_priced_by_pixels_not_base64_length() {
        // ~180 KB of base64, the ballpark of a real 1280x800 JPEG.
        let fake_b64 = "A".repeat(180_000);
        let msg = Message {
            role: "user".into(),
            content: vec![ContentBlock::tool_result_blocks(
                "toolu_1",
                vec![
                    ContentBlock::text("Screenshot of localhost:5173"),
                    ContentBlock::image("image/jpeg", &fake_b64, 1280, 800),
                ],
            )],
        };

        let estimate = estimate_message_tokens(&msg);
        let by_pixels = 1280 * 800 / 750; // ~1365
        assert!(
            estimate < by_pixels + 200,
            "expected ~{by_pixels} tokens, got {estimate} — the base64 leaked into the estimate"
        );
        assert!(estimate > by_pixels - 200, "estimate {estimate} is too low");
    }

    /// The plain-text path must keep its old behaviour.
    #[test]
    fn text_tool_result_is_still_priced_by_serialized_length() {
        let msg = Message {
            role: "user".into(),
            content: vec![ContentBlock::tool_result("toolu_1", "x".repeat(3_000))],
        };
        let estimate = estimate_message_tokens(&msg);
        assert!(
            (900..1_200).contains(&estimate),
            "expected ~1000 tokens, got {estimate}"
        );
    }

    #[test]
    fn push_user_blocks_merges_consecutive_user_turns() {
        let store = tmp_store();
        let mut history = vec![Message {
            role: "user".into(),
            content: vec![ContentBlock::text("a")],
        }];
        push_user_blocks(
            &mut history,
            &store,
            &dummy_ctx(),
            vec![ContentBlock::text("b")],
        );
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
        push_user_blocks(
            &mut history,
            &store,
            &dummy_ctx(),
            vec![ContentBlock::text("b")],
        );
        assert_eq!(history.len(), 2, "user turn after assistant must append");
        assert_eq!(history[1].role, "user");
        let _ = std::fs::remove_file(&store.path);
    }

    #[test]
    fn user_turn_carries_only_the_raw_message_no_injected_directive() {
        let store = tmp_store();
        let mut history: Vec<Message> = Vec::new();
        push_user_blocks(
            &mut history,
            &store,
            &dummy_ctx(),
            vec![ContentBlock::text("O que este projeto faz?")],
        );
        assert_eq!(history.len(), 1);
        assert_eq!(
            history[0].content.len(),
            1,
            "no phase directive should be folded in"
        );
        let _ = std::fs::remove_file(&store.path);
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
        let small = estimate_tokens(std::slice::from_ref(&msg1), "", &[]);
        let large = estimate_tokens(&[msg1, msg2], "", &[]);
        assert!(large > small, "more history should mean more tokens");
    }

    /// Estimate cost for provider calls when the provider does not report cost.
    fn cost_for(model: &str, input: u32, cache_read: u32, output: u32) -> f64 {
        let p = model_pricing(model);
        (input as f64 * p.input + cache_read as f64 * p.cache_read + output as f64 * p.output)
            / 1_000_000.0
    }

    #[test]
    fn cost_claudinio_rates() {
        // claudinio: $0.50/M input, $0.15/M cache read, $2.00/M output
        let cost = cost_for("claudinio", 1000, 0, 500);
        assert!(
            (cost - 0.0015).abs() < 0.0001,
            "expected ~$0.0015, got {cost}"
        );
    }

    #[test]
    fn cost_claudius_rates() {
        // claudius: $3.00/M input, $0.90/M cache read, $8.00/M output
        let cost = cost_for("claudius", 1000, 0, 500);
        assert!(
            (cost - 0.007).abs() < 0.0001,
            "expected ~$0.007, got {cost}"
        );
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
        assert_eq!(MAX_CONTEXT_TOKENS, 200_000);
        assert_eq!(COMPACT_THRESHOLD, 150_000);
    }
}

#[cfg(test)]
mod golden_goal_tests {
    use super::*;

    #[test]
    fn test_parse_goals_no_goals() {
        let (cleaned, goals) = parse_goals("hello world");
        assert_eq!(cleaned, "hello world");
        assert!(goals.is_empty());
    }

    #[test]
    fn test_parse_goals_single() {
        let (cleaned, goals) = parse_goals("do <goal>code coverage in 80%</goal> please");
        assert_eq!(goals, vec!["code coverage in 80%"]);
        assert!(!cleaned.contains("<goal>"));
    }

    #[test]
    fn test_parse_goals_multiple() {
        let (cleaned, goals) =
            parse_goals("<goal>coverage 80%</goal> and <goal>no lint errors</goal>");
        assert_eq!(goals.len(), 2);
        assert_eq!(goals[0], "coverage 80%");
        assert_eq!(goals[1], "no lint errors");
        assert!(!cleaned.contains("<goal>"));
    }

    #[test]
    fn test_parse_goals_empty_goal_text() {
        let (cleaned, goals) = parse_goals("<goal>  </goal>");
        assert!(goals.is_empty());
        assert!(cleaned.is_empty());
    }
}
