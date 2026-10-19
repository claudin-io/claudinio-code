//! What a run does when its context crosses a line, cheapest first: drop old
//! tool traffic and keep every word (`try_verbatim_compaction`), hand off to a
//! linked successor session (`maybe_context_handoff`), and summarize as the
//! last resort (`compact_history`). Also the outcome a run reports back —
//! finished, or a handoff to start.

use crate::agent::budget::ContextBudget;
use crate::agent::events::AgentEvent;
use crate::agent::persist::{CacheBreakReason, SessionRecord, SessionStore, now_ms};
use crate::agent::provider::{AgentConfig, ContentBlock, Message, ToolDescription};
use crate::agent::retry::stream_message_with_retry;
use crate::agent::session::{
    AnswerMap, ApprovalMap, ModeCtl, ModeOrigin, PromptProfile, SessionMode, SteeringCtl,
    estimate_message_tokens, estimate_tokens, push_turn, push_user_blocks, write_status,
};
use crate::agent::subagent;
use crate::agent::tools::ToolContext;
use serde::{Deserialize, Serialize};
use std::sync::Arc;
use tauri::ipc::Channel;

/// How many recent user↔agent exchanges stay verbatim after a compaction.
const TAIL_USER_TURNS: usize = 2;

/// Number of Turn records (counted back from the end) to keep verbatim when
/// compacting: the last `TAIL_USER_TURNS` real user exchanges, bounded by
/// `tail_tokens` (`ContextBudget::tail_tokens`) — if the recent exchanges alone
/// exceed it, the tail shrinks, down to zero, so compaction still frees the
/// context. Only looks at records after the previous compaction.
fn compute_tail_turns(records: &[SessionRecord], tail_tokens: u64) -> usize {
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
        let SessionRecord::Turn { message, .. } = rec else {
            continue;
        };
        turns += 1;
        tokens += estimate_message_tokens(message);
        if tokens > tail_tokens {
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
#[allow(clippy::too_many_arguments)]
pub async fn compact_history(
    config: &AgentConfig,
    store: &SessionStore,
    ctx: &ToolContext,
    event_tx: &Channel<AgentEvent>,
    approvals: &ApprovalMap,
    answers: &AnswerMap,
    session_id: &str,
    steering: &Arc<SteeringCtl>,
    trigger: crate::agent::hooks::CompactTrigger,
    tail_tokens: u64,
) -> Result<String, String> {
    // ── Hooks: PreCompact ────────────────────────────────────────────────────
    //
    // The single funnel for both triggers. Whatever a hook returns is prepended
    // to the summary, which is the only text that survives a compaction — a
    // flush hook's "write this down before it is gone" has to land somewhere
    // that is not about to be thrown away.
    let mut hook_note = String::new();
    if let Some(h) = &ctx.hooks {
        let out = crate::agent::hooks::fire_pre_compact(h, trigger, Some(event_tx)).await;
        if let Some(text) = out.context() {
            hook_note = format!("{text}\n\n");
        }
    }

    let jsonl_path = store.path.to_string_lossy().to_string();
    let records = crate::agent::persist::load_records_cached(&store.path, &ctx.records_cache)
        .unwrap_or_default();
    let tail_turns = compute_tail_turns(&records, tail_tokens);

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
    // history_from_records logic picks the LAST one. The note, with the
    // hook's text, is also appended to `.claudinio/notes/<session>.md`
    // (`agent::notes`), and the compacted context names that file.
    let summary = crate::agent::notes::record(
        ctx.workspace_root.as_deref(),
        session_id,
        &format!("{hook_note}{summary}"),
        now_ms(),
    );
    store.append(&SessionRecord::Compacted {
        summary: summary.clone(),
        tail_turns,
        document: crate::agent::doc_session::outline_block(ctx),
        ts: now_ms(),
    })?;
    store.try_append(&SessionRecord::cache_break(CacheBreakReason::Compaction));
    crate::agent::persist::invalidate_cache(&store.path, &ctx.records_cache);

    // Record the post-compaction context size so the UI meter drops even for
    // manual compaction (no run in flight). The estimate excludes the system
    // prompt/tools; the next run's Status corrects it with the real number.
    let new_recs = crate::agent::persist::load_records_cached(&store.path, &ctx.records_cache)
        .unwrap_or_default();
    let new_history = crate::agent::persist::history_from_records(&new_recs);
    let (ci, co, cc, cci, cco, ccc) = crate::agent::persist::cumulative_stats(&new_recs);
    let new_context = estimate_tokens(&new_history, "", &[]);
    write_status(
        store,
        ctx,
        session_id,
        ci,
        co,
        cc,
        cci,
        cco,
        ccc,
        Some(new_context),
    );

    Ok(summary)
}

/// Why this session is handing off to a new linked session.
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
pub enum HandoffReason {
    #[serde(rename = "plan_execution")]
    PlanExecution,
    #[serde(rename = "golden_flip")]
    GoldenFlip,
    #[serde(rename = "context_handoff")]
    ContextHandoff,
    #[serde(rename = "manual_builder")]
    ManualBuilder,
    /// `doc_create` in a session that already has a document: a new
    /// top-level topic is a new session (lean-harness Phase 4b).
    #[serde(rename = "new_document")]
    NewDocument,
}

impl HandoffReason {
    pub fn as_str(&self) -> &'static str {
        match self {
            HandoffReason::PlanExecution => "plan_execution",
            HandoffReason::GoldenFlip => "golden_flip",
            HandoffReason::ContextHandoff => "context_handoff",
            HandoffReason::ManualBuilder => "manual_builder",
            HandoffReason::NewDocument => "new_document",
        }
    }
}

/// The answer to a handoff-producing call (`doc_create` in a session that has
/// a document, `enter_plan_mode`, `exit_plan_mode`) in a round that already
/// queued a handoff: the app follows one successor per run.
pub const HANDOFF_QUEUED: &str = "A handoff to a new session is already queued this turn, \
     so this call was not run. End your turn now; anything else waits for the user's next message.";

/// A PostToolUse hook stopped the run in the round that queued `spec`. A
/// hook's `continue: false` outranks the handoff, so no successor run starts;
/// what the round already did stays, and this says so to the model — which
/// had been told a session opens — as a note for the tool-results turn.
/// `exit_plan_mode` had already recorded Builder and committed the plan, so
/// the session takes the Builder its record says and the next message builds
/// here.
pub(crate) fn not_followed(
    spec: &HandoffSpec,
    mode_ctl: &ModeCtl,
    event_tx: &Channel<AgentEvent>,
) -> String {
    let what = match (&spec.reason, &spec.document) {
        (HandoffReason::NewDocument, Some(doc)) => format!(
            "the linked session for {doc} did not open. {doc} was created and no session \
             owns it; this session keeps its own document."
        ),
        (HandoffReason::PlanExecution, _) => {
            mode_ctl.set(spec.next_mode, spec.next_origin);
            let _ = event_tx.send(AgentEvent::ModeChanged {
                mode: spec.next_mode.as_str().into(),
                origin: spec.next_origin.as_str().into(),
                reason: None,
            });
            "the plan's Builder session did not open. This session is now in Builder mode."
                .to_string()
        }
        _ => "the linked session did not open.".to_string(),
    };
    format!("<handoff-not-followed>A hook stopped the run, so {what}</handoff-not-followed>")
}

/// Data needed to create a linked successor session and resume work.
pub struct HandoffSpec {
    pub reason: HandoffReason,
    pub next_mode: SessionMode,
    pub next_origin: ModeOrigin,
    pub first_message: String,
    pub golden_cycle: u32,
    pub golden_stalls: u32,
    pub golden_last_pending: Vec<String>,
    /// The living document the successor is linked to instead of its
    /// predecessor's (`NewDocument`), relative to the workspace.
    pub document: Option<String>,
}

/// What a workflow run produced: either it finished normally, or it requests a
/// handoff to a new linked session.
pub enum RunOutcome {
    Completed,
    Handoff(Box<HandoffSpec>),
}

/// False while the history is one message: the prompt that opened the session,
/// or the handoff document a successor starts from. A handoff or a summary of
/// that is the same text again at best — and for a successor already over its
/// line, a handoff answered by another handoff, with nothing ever run in
/// between. Such a run is sent as it is, or refused at the ceiling.
pub(super) fn has_something_to_compress(history: &[Message]) -> bool {
    history.len() > 1
}

/// The context size at which a run must shed weight: the handoff line for
/// chat sessions; the compaction line for the single-purpose profiles, which have no
/// handoff to try first.
pub(super) fn shed_line(budget: &ContextBudget, profile: PromptProfile) -> u64 {
    if profile.is_chat() {
        budget.soft
    } else {
        budget.hard
    }
}

/// A verbatim compaction must land at most this share of the limit, or the
/// limit is crossed again a few rounds later and the cold prefix was wasted.
pub(crate) const PRUNE_TARGET: f64 = 0.8;

/// Take a verbatim compaction only when it frees enough and leaves room.
/// "Room" is measured above `floor` (`ContextBudget::prune_floor`): what no
/// prune can touch is not counted against the prune.
pub(crate) fn accept_prune(
    outcome: &crate::agent::prune::Outcome,
    new_estimate: u64,
    limit: u64,
    floor: u64,
) -> bool {
    outcome.reduction() >= crate::agent::prune::MIN_REDUCTION
        && (new_estimate.saturating_sub(floor) as f64)
            < limit.saturating_sub(floor) as f64 * PRUNE_TARGET
}

/// What a prune at `limit` should free, and what it must leave alone.
pub(crate) fn shed_options(
    estimated: u64,
    limit: u64,
    budget: &ContextBudget,
    last_resort: bool,
) -> crate::agent::prune::ShedOptions {
    // Halfway down to the floor, not just under the line: every prune is a
    // cold prefix, so one deep cut is cheaper than several shallow ones.
    let floor = budget.prune_floor.min(limit);
    let target = floor + (limit - floor) / 2;
    crate::agent::prune::ShedOptions {
        free_chars: usize::try_from(estimated.saturating_sub(target) * 3).unwrap_or(usize::MAX),
        recent_chars: usize::try_from(budget.tail_tokens * 3).unwrap_or(usize::MAX),
        last_resort,
    }
}

/// Before handing off or summarizing, try the verbatim way: drop old tool
/// traffic and keep every word of the conversation (`agent::prune::shed` —
/// what is superseded first, then the oldest results masked to their head,
/// with Jev, when there is one, keeping what it says the newest rounds still
/// need). Returns the new context estimate and what Jev cost, or `None` — not
/// enough freed — and the caller falls through to the handoff / compaction
/// exactly as before.
#[allow(clippy::too_many_arguments)]
pub(super) async fn try_verbatim_compaction(
    config: &AgentConfig,
    budget: &ContextBudget,
    profile: PromptProfile,
    estimated: u64,
    history: &mut Vec<Message>,
    system: &str,
    tools: &[ToolDescription],
    store: &SessionStore,
    ctx: &ToolContext,
    event_tx: &Channel<AgentEvent>,
) -> Option<(u64, f64)> {
    let limit = shed_line(budget, profile);
    if estimated < limit {
        return None;
    }
    let backend = crate::agent::jev::backend(config);
    // Not a last resort: no call is dropped outright, a handoff comes next.
    let (outcome, pruned) = crate::agent::prune::shed(
        history,
        backend.as_ref(),
        shed_options(estimated, limit, budget, false),
        |outcome, pruned| {
            accept_prune(
                outcome,
                estimate_tokens(pruned, system, tools),
                limit,
                budget.prune_floor,
            )
        },
    )
    .await
    .accepted?;

    // The transcript is about to lose content: PreCompact's contract.
    let mut hook_note = None;
    if let Some(h) = &ctx.hooks {
        let out = crate::agent::hooks::fire_pre_compact(
            h,
            crate::agent::hooks::CompactTrigger::Auto,
            Some(event_tx),
        )
        .await;
        hook_note = out.context();
    }

    let _ = event_tx.send(AgentEvent::TextStep {
        text: format!(
            "__compact_start__:{}/{}",
            estimated / 1000,
            budget.ceiling / 1000
        ),
    });
    store
        .append(&SessionRecord::Pruned {
            drop_calls: outcome.decision.drop_calls.clone(),
            truncate_results: outcome.decision.truncate_results.clone(),
            head_chars: crate::agent::prune::TRUNCATE_HEAD_CHARS,
            stats: Some(outcome.stats_json()),
            ts: now_ms(),
        })
        .ok()?;
    store.try_append(&SessionRecord::cache_break(CacheBreakReason::Prune));
    crate::agent::persist::invalidate_cache(&store.path, &ctx.records_cache);
    // The decision applied to the history in memory, not a rebuild from the
    // file: `push_user_blocks` merges into the last user turn without writing
    // a record, so the file does not hold everything the model is about to be
    // shown — the prompt just typed after a run that ended on tool results, a
    // hook's feedback, a loop nudge. A reload applies the same decision to the
    // same turns and differs only by those merges, as it already did.
    *history = pruned;
    if let Some(note) = hook_note {
        push_user_blocks(history, store, ctx, vec![ContentBlock::text(note)]);
    }
    let new_estimate = estimate_tokens(history, system, tools);
    let _ = event_tx.send(AgentEvent::TextStep {
        text: format!(
            "__compact_done__:{}/{}",
            estimated / 1000,
            new_estimate / 1000
        ),
    });
    Some((new_estimate, outcome.cost))
}

/// When the context crosses the configured handoff threshold, ask the model to
/// compress its own context into a handoff document (Matt Pocock style) and
/// request a linked-session handoff carrying it as the successor's first
/// message. Returns `None` when the threshold isn't crossed, the profile is
/// not Standard, generation was interrupted, or generation failed — callers
/// fall through to the compaction safety net.
#[allow(clippy::too_many_arguments)]
pub(super) async fn maybe_context_handoff(
    config: &AgentConfig,
    budget: &ContextBudget,
    profile: PromptProfile,
    estimated: u64,
    history: &mut Vec<Message>,
    system: &str,
    store: &SessionStore,
    ctx: &ToolContext,
    event_tx: &Channel<AgentEvent>,
    session_id: &str,
    steering: &Arc<SteeringCtl>,
    mode_ctl: &Arc<ModeCtl>,
    run_in: u32,
    run_out: u32,
    run_cache: u32,
) -> Option<RunOutcome> {
    if !profile.is_chat() {
        return None;
    }
    if estimated < budget.soft || !has_something_to_compress(history) {
        return None;
    }
    let _ = event_tx.send(AgentEvent::TextStep {
        text: format!(
            "__handoff_start__:{}/{}",
            estimated / 1000,
            budget.ceiling / 1000
        ),
    });

    // ── Hooks: PreCompact, on a handoff ──────────────────────────────────────
    //
    // A handoff destroys context harder than a compaction does: the successor
    // starts with an empty history and one document. PreCompact's contract is
    // "the transcript is about to go away, write down anything worth keeping",
    // and a hook that fired on compaction but not here would miss the single
    // most destructive moment in a Claudinio session — in exactly the long runs
    // that have learned something worth keeping. It reports as `trigger: auto`
    // on the wire (no published config binds to a `PreHandoff` that does not
    // exist) with `claudinio_trigger: handoff` alongside for anyone who cares.
    if let Some(h) = &ctx.hooks {
        let out = crate::agent::hooks::fire_pre_compact(
            h,
            crate::agent::hooks::CompactTrigger::Handoff,
            Some(event_tx),
        )
        .await;
        if let Some(text) = out.context() {
            push_user_blocks(
                history,
                store,
                ctx,
                vec![ContentBlock::text(format!(
                    "<hook-context>\n{text}\n</hook-context>"
                ))],
            );
        }
    }

    push_user_blocks(
        history,
        store,
        ctx,
        vec![ContentBlock::text(crate::agent::nudges::HANDOFF_REQUEST)],
    );

    let (cur_mode, cur_origin) = mode_ctl.get();
    let resolved_model = config.model_for_mode(cur_mode.as_str());
    let net_detail = format!("{resolved_model} · handoff");
    let mut gen_in: u32 = 0;
    let mut gen_out: u32 = 0;
    let mut gen_cache: u32 = 0;
    let mut handoff_text = String::new();
    for attempt in 0..2 {
        let mut assistant_text = String::new();
        let out = match stream_message_with_retry(
            config,
            resolved_model,
            history,
            &[], // no tools: text is the only possible output
            Some(system),
            event_tx,
            session_id,
            &mut assistant_text,
            &steering.interrupt,
            &net_detail,
        )
        .await
        {
            Ok(o) => o,
            Err(e) => {
                let _ = event_tx.send(AgentEvent::TextStep {
                    text: format!("__handoff_fail__:{e}"),
                });
                return None;
            }
        };
        store.try_append(&SessionRecord::usage(
            0,
            resolved_model,
            out.usage.as_ref(),
            None,
        ));
        if out.interrupted {
            // The user interrupted mid-generation: abandon the handoff and let
            // the main loop take its normal interrupted path.
            return None;
        }
        if let Some(u) = &out.usage {
            gen_in += u.input_tokens;
            gen_out += u.output_tokens;
            gen_cache += u.cache_read_input_tokens;
        }
        let trimmed = assistant_text.trim();
        // Sanity check: a usable handoff has real substance and the requested
        // section structure — a rambling or empty reply silently degrades the
        // successor session, so re-ask once and then give up to compaction.
        if trimmed.len() >= 200 && trimmed.contains("##") {
            handoff_text = trimmed.to_string();
            push_turn(
                history,
                store,
                ctx,
                Message {
                    role: "assistant".into(),
                    content: vec![ContentBlock::text(trimmed)],
                },
            );
            break;
        }
        if attempt == 0 {
            push_user_blocks(
                history,
                store,
                ctx,
                vec![ContentBlock::text(crate::agent::nudges::HANDOFF_RETRY)],
            );
        }
    }
    if handoff_text.is_empty() {
        let _ = event_tx.send(AgentEvent::TextStep {
            text: "__handoff_fail__:empty or malformed handoff document".into(),
        });
        return None;
    }

    // Close this session's books: audit record, Done, cumulative Status.
    store.try_append(&SessionRecord::Handoff {
        text: handoff_text.clone(),
        ts: now_ms(),
    });
    store.try_append(&SessionRecord::cache_break(CacheBreakReason::Handoff));
    store.try_append(&SessionRecord::Done {
        input_tokens: run_in + gen_in,
        output_tokens: run_out + gen_out,
        cache_read_tokens: Some(run_cache + gen_cache),
        ts: now_ms(),
    });
    crate::agent::persist::invalidate_cache(&store.path, &ctx.records_cache);
    let records = crate::agent::persist::load_records_cached(&store.path, &ctx.records_cache)
        .unwrap_or_default();
    let (ci, co, cc, cci, cco, ccc) = crate::agent::persist::cumulative_stats(&records);
    write_status(
        store,
        ctx,
        session_id,
        ci + (run_in + gen_in) as u64,
        co + (run_out + gen_out) as u64,
        cc,
        cci,
        cco,
        ccc,
        Some(estimated),
    );
    let _ = event_tx.send(AgentEvent::TextStep {
        text: format!(
            "__handoff_done__:{}/{}",
            estimated / 1000,
            budget.ceiling / 1000
        ),
    });

    // Golden state carries over so the loop caps never reset. During rounds
    // the live counters equal what the records say (they only change on the
    // terminal branch, which returns immediately), so recomputing is exact.
    let (linked_cycle, linked_stalls, linked_pending) =
        crate::agent::persist::linked_golden_state(&records);
    let golden_cycle = crate::agent::persist::golden_cycle_count(&records).max(linked_cycle);

    let plan_path = ctx.workspace_root.as_deref().and_then(|root| {
        crate::agent::tools::write_plan::latest_plan_path(root, ctx.plan_save_path.as_deref())
    });
    let first_message = crate::agent::transition::compose_context_handoff_message(
        &handoff_text,
        plan_path.as_deref().map(|p| p.to_str().unwrap_or_default()),
    );

    Some(RunOutcome::Handoff(Box::new(HandoffSpec {
        reason: HandoffReason::ContextHandoff,
        next_mode: cur_mode,
        next_origin: cur_origin,
        first_message,
        golden_cycle,
        golden_stalls: linked_stalls,
        golden_last_pending: linked_pending,
        document: None,
    })))
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn compute_tail_turns_covers_last_two_exchanges() {
        let user = |t: &str| SessionRecord::Turn {
            message: Message {
                role: "user".into(),
                content: vec![ContentBlock::text(t)],
            },
            ts: 0,
        };
        let asst = |t: &str| SessionRecord::Turn {
            message: Message {
                role: "assistant".into(),
                content: vec![ContentBlock::text(t)],
            },
            ts: 0,
        };
        let recs = vec![
            user("q1"),
            asst("a1"),
            user("q2"),
            asst("a2"),
            user("q3"),
            asst("a3"),
        ];
        // Last 2 exchanges = q2..a3 = 4 Turn records
        let tail = crate::agent::budget::TAIL_TOKENS;
        assert_eq!(compute_tail_turns(&recs, tail), 4);
        // A budget too small for even the last exchange keeps nothing verbatim,
        // which is what a small-window model gets instead of an overflow.
        assert_eq!(compute_tail_turns(&recs, 0), 0);
    }

    #[test]
    fn compute_tail_turns_shrinks_when_over_budget() {
        let tail = crate::agent::budget::TAIL_TOKENS;
        let big = "x".repeat((tail as usize) * 4); // way over budget alone
        let recs = vec![
            SessionRecord::Turn {
                message: Message {
                    role: "user".into(),
                    content: vec![ContentBlock::text(big)],
                },
                ts: 0,
            },
            SessionRecord::Turn {
                message: Message {
                    role: "assistant".into(),
                    content: vec![ContentBlock::text("a")],
                },
                ts: 0,
            },
        ];
        assert_eq!(
            compute_tail_turns(&recs, tail),
            0,
            "oversized tail must be dropped"
        );
    }
}

#[cfg(test)]
mod verbatim_compaction_tests {
    use super::*;
    use crate::agent::prune::Outcome;
    use crate::agent::session::{COMPACT_THRESHOLD, MAX_CONTEXT_TOKENS};

    fn outcome(before: usize, after: usize) -> Outcome {
        Outcome {
            chars_before: before,
            chars_after: after,
            ..Default::default()
        }
    }

    #[test]
    fn a_prune_that_frees_enough_and_lands_well_under_the_limit_is_taken() {
        assert!(accept_prune(&outcome(100, 40), 60_000, 120_000, 0));
    }

    #[test]
    fn a_prune_that_frees_too_little_falls_back() {
        // 20% < MIN_REDUCTION: not worth a cold prefix; the handoff decides.
        assert!(!accept_prune(&outcome(100, 80), 60_000, 120_000, 0));
    }

    #[test]
    fn a_prune_that_stays_near_the_limit_falls_back() {
        // Freed half, but would cross the limit again a few rounds later.
        assert!(!accept_prune(&outcome(100, 50), 110_000, 120_000, 0));
    }

    /// On a small window the prefix is most of the context. Measured from
    /// zero, the target would sit below the prefix itself and no prune could
    /// ever be taken; measured above the prefix, a real one is.
    #[test]
    fn room_is_measured_above_what_no_prune_can_touch() {
        // 10k of prefix, a soft line at 18k: 8k of conversation, pruned to 5k.
        assert!(!accept_prune(&outcome(100, 40), 15_000, 18_000, 0));
        assert!(accept_prune(&outcome(100, 40), 15_000, 18_000, 10_000));
        // Still refused when the conversation stays near the line.
        assert!(!accept_prune(&outcome(100, 40), 17_000, 18_000, 10_000));
    }

    #[test]
    fn a_prune_aims_halfway_between_the_floor_and_the_line() {
        let full = ContextBudget::new(200_000, 120_000, None, 30_000);
        let o = shed_options(130_000, full.soft, &full, false);
        // Floor 0: land at 60k, so free 70k tokens of history.
        assert_eq!(o.free_chars, 70_000 * 3);
        assert_eq!(o.recent_chars, 20_000 * 3);
        assert!(!o.last_resort);

        let small = ContextBudget::new(32_768, 120_000, Some(8_192), 4_576);
        let o = shed_options(17_000, small.soft, &small, true);
        // Floor 4_576, line 16_576: land at 10_576.
        assert_eq!(o.free_chars, (17_000 - 10_576) * 3);
        assert_eq!(o.recent_chars, 6_000 * 3);
        assert!(o.last_resort);
    }

    #[test]
    fn the_limit_is_the_handoff_line_for_standard_sessions() {
        let cfg = AgentConfig::default();
        let budget = ContextBudget::for_model(&cfg, &cfg.builder_model, 30_000);
        assert_eq!(
            shed_line(&budget, PromptProfile::Standard),
            cfg.effective_handoff_threshold()
        );
        // The lean profiles have no handoff: their first line is the compaction.
        assert_eq!(
            shed_line(&budget, PromptProfile::GitSync),
            COMPACT_THRESHOLD
        );
    }

    /// The default config against the default models is the contract with
    /// every existing session: nothing about its three lines may move.
    #[test]
    fn the_default_models_run_on_the_lines_they_always_had() {
        let cfg = AgentConfig::default();
        for model in [cfg.brain_model.as_str(), cfg.builder_model.as_str()] {
            let b = ContextBudget::for_model(&cfg, model, 33_000);
            assert_eq!(
                (b.soft, b.hard, b.ceiling),
                (120_000, COMPACT_THRESHOLD, MAX_CONTEXT_TOKENS),
                "{model}"
            );
        }
    }

    /// A catalog model with a 64k window used to be driven by the 200k lines,
    /// so it overflowed at the provider before any of them was reached.
    #[test]
    fn a_small_catalog_model_gets_lines_inside_its_own_window() {
        let mut cfg = AgentConfig::default();
        cfg.providers.insert(
            "deepseek".into(),
            crate::agent::provider::ProviderEntry {
                api_key: "k".into(),
                base_url: "https://api.deepseek.com".into(),
                protocol: "openai".into(),
                enabled_models: vec![],
                label: None,
                model_pricing: Default::default(),
                model_output_limits: [("deepseek-chat".to_string(), 8_192u32)]
                    .into_iter()
                    .collect(),
                custom: false,
                custom_models: vec![],
                model_context_limits: [("deepseek-chat".to_string(), 65_536u32)]
                    .into_iter()
                    .collect(),
                context_window: None,
            },
        );
        let b = ContextBudget::for_model(&cfg, "deepseek/deepseek-chat", 10_000);
        assert_eq!(b.window, 65_536);
        assert!(b.soft < b.hard && b.hard < b.ceiling, "{b:?}");
        assert!(b.ceiling < 65_536);
        assert!(b.soft > 10_000, "the soft line is past the prefix: {b:?}");
    }
}

/// The handoff document is a provider request like any other, and is billed.
#[cfg(test)]
mod handoff_usage_tests {
    use super::*;
    use crate::agent::subagent::scripted_model::{Reply, spawn_recording};

    const DOCUMENT: &str = "## Purpose of next session\nFinish the parser refactor.\n\
        ## Current state\nThe lexer is split; tests green.\n## Key decisions\nKeep the API.\n\
        ## Pointers\nsrc/parser.rs\n## In-flight work\nThe expression module.\n\
        ## Next actions\nMove the expression parser, run the tests.";

    #[tokio::test]
    async fn the_handoff_document_request_leaves_a_usage_record() {
        let (base, _, bodies) = spawn_recording(vec![Reply::Text(DOCUMENT)]).await;
        let config = crate::agent::run_loop::surface_tests::config_for(&base, None);
        let path = std::env::temp_dir().join(format!(
            "claudinio_handoff_usage_{}_{}.jsonl",
            std::process::id(),
            now_ms()
        ));
        std::fs::write(&path, "").unwrap();
        let store = SessionStore { path };
        let ctx = crate::agent::tools::tests_support::ctx();
        let budget = ContextBudget::for_model(&config, &config.builder_model, 10_000);
        let mut history = vec![
            Message {
                role: "user".into(),
                content: vec![ContentBlock::text("Refactor the parser.")],
            },
            Message {
                role: "assistant".into(),
                content: vec![ContentBlock::text("Working on it.")],
            },
        ];
        let events: Channel<AgentEvent> = Channel::new(|_| Ok(()));
        let outcome = maybe_context_handoff(
            &config,
            &budget,
            PromptProfile::Standard,
            budget.soft,
            &mut history,
            "You are a test.",
            &store,
            &ctx,
            &events,
            "handoff-usage",
            &Arc::new(SteeringCtl::new()),
            &Arc::new(ModeCtl::new(SessionMode::Builder, ModeOrigin::Human)),
            0,
            0,
            0,
        )
        .await;
        assert!(matches!(outcome, Some(RunOutcome::Handoff(_))));
        assert_eq!(bodies.lock().unwrap().len(), 1);
        let usage: Vec<u32> = crate::agent::persist::load_records(&store.path)
            .unwrap()
            .into_iter()
            .filter_map(|r| match r {
                SessionRecord::Usage { round, .. } => Some(round),
                _ => None,
            })
            .collect();
        assert_eq!(usage, vec![0], "one request, outside the loop's rounds");
        std::fs::remove_file(&store.path).ok();
    }
}

/// A compaction's summarizer is a subagent making provider requests, and every
/// one of them is billed. Before it had a transcript, a run that compacted
/// reported fewer rounds and a lower cost than the provider charged for it.
#[cfg(test)]
mod compaction_usage_tests {
    use super::*;
    use crate::agent::persist::load_records;
    use crate::agent::subagent::scripted_model::{Reply, spawn_with_usage};

    fn usage_labels(path: &std::path::Path) -> Vec<Option<String>> {
        load_records(path)
            .unwrap_or_default()
            .into_iter()
            .filter_map(|r| match r {
                SessionRecord::Usage { subagent, .. } => Some(subagent),
                _ => None,
            })
            .collect()
    }

    #[tokio::test]
    async fn every_summarizer_request_leaves_a_usage_record_in_a_compaction_transcript() {
        let dir = std::env::temp_dir().join(format!(
            "claudinio_compact_usage_{}_{}",
            std::process::id(),
            now_ms()
        ));
        std::fs::create_dir_all(&dir).unwrap();
        let store = SessionStore {
            path: dir.join("sess.jsonl"),
        };
        store
            .append(&SessionRecord::User {
                text: "Refactor the parser.".into(),
                ts: now_ms(),
            })
            .unwrap();
        store
            .append(&SessionRecord::Turn {
                message: Message {
                    role: "assistant".into(),
                    content: vec![ContentBlock::text("Working on it.")],
                },
                ts: now_ms(),
            })
            .unwrap();
        let (base, sizes, _) = spawn_with_usage(
            vec![
                Reply::Tool {
                    name: "read_file",
                    input: serde_json::json!({ "path": store.path.to_string_lossy() }),
                },
                Reply::Text("## Task & intent\nRefactor the parser."),
            ],
            Some(serde_json::json!({"input_tokens": 40, "cache_read_input_tokens": 300})),
        )
        .await;
        let config = crate::agent::run_loop::surface_tests::config_for(&base, None);
        let ctx = ToolContext {
            workspace_root: Some(dir.to_string_lossy().to_string()),
            session_store_path: Some(store.path.to_string_lossy().to_string()),
            ..crate::agent::tools::tests_support::ctx()
        };
        let events: Channel<AgentEvent> = Channel::new(|_| Ok(()));
        let (approvals, answers) = (ApprovalMap::default(), AnswerMap::default());
        let steering = Arc::new(SteeringCtl::new());
        let compact = || {
            compact_history(
                &config,
                &store,
                &ctx,
                &events,
                &approvals,
                &answers,
                "compact-usage",
                &steering,
                crate::agent::hooks::CompactTrigger::Auto,
                10_000,
            )
        };
        compact().await.expect("the first compaction");
        let requests = sizes.lock().unwrap().len();
        assert_eq!(requests, 2, "a read, then the summary");

        let first = dir.join("sess.sub").join("compact-0.jsonl");
        assert_eq!(
            usage_labels(&first),
            vec![Some("compact-0".to_string()); requests],
            "one usage record per summarizer request, labelled with its transcript"
        );
        assert!(
            usage_labels(&store.path).is_empty(),
            "the summarizer's requests stay out of the model's history file"
        );

        // A second compaction of the same session gets a transcript of its own.
        compact().await.expect("the second compaction");
        let second = dir.join("sess.sub").join("compact-1.jsonl");
        assert_eq!(
            usage_labels(&second).len(),
            sizes.lock().unwrap().len() - requests
        );

        // And the eval reads them back as provider requests of the run.
        let m = crate::eval::metrics::collect(&dir, &Default::default());
        assert_eq!(m.rounds as usize, sizes.lock().unwrap().len());
        assert_eq!(m.subagents, 0, "a summarizer is not a spawned subagent");
        assert_eq!(m.compactions, 2);
        std::fs::remove_dir_all(&dir).ok();
    }
}

/// Every time the harness rewrites the history a provider has cached, the
/// next request pays for the whole context again. The session file says so,
/// one `cache_break` per rewrite, so a phase that adds a rewrite (a deferred
/// tool load, say) can be held to a budget.
#[cfg(test)]
mod cache_break_tests {
    use super::*;
    use crate::agent::persist::load_records;
    use crate::agent::subagent::scripted_model::{Reply, spawn_recording};

    fn breaks(path: &std::path::Path) -> Vec<String> {
        load_records(path)
            .unwrap_or_default()
            .into_iter()
            .filter_map(|r| match r {
                SessionRecord::CacheBreak { reason, .. } => Some(reason),
                _ => None,
            })
            .collect()
    }

    fn temp_store(tag: &str) -> SessionStore {
        let dir = std::env::temp_dir().join(format!(
            "claudinio_cache_break_{tag}_{}_{}",
            std::process::id(),
            now_ms()
        ));
        std::fs::create_dir_all(&dir).unwrap();
        let path = dir.join("s.jsonl");
        std::fs::write(&path, "").unwrap();
        SessionStore { path }
    }

    fn turn(role: &str, content: ContentBlock) -> Message {
        Message {
            role: role.into(),
            content: vec![content],
        }
    }

    #[tokio::test]
    async fn an_accepted_prune_records_a_cache_break() {
        let config = AgentConfig::default();
        let store = temp_store("prune");
        let ctx = crate::agent::tools::tests_support::ctx();
        let events: Channel<AgentEvent> = Channel::new(|_| Ok(()));
        let budget = ContextBudget::for_model(&config, &config.builder_model, 0);
        let mut history = vec![turn("user", ContentBlock::text("Refactor the parser."))];
        for i in 0..30 {
            let id = format!("r{i}");
            history.push(turn(
                "assistant",
                ContentBlock::tool_use(
                    &id,
                    "read_file",
                    serde_json::json!({ "path": format!("/ws/f{i}.rs") }),
                ),
            ));
            history.push(turn(
                "user",
                ContentBlock::tool_result(&id, format!("// {i}\n").repeat(3_000)),
            ));
        }
        let estimated = estimate_tokens(&history, "s", &[]);
        try_verbatim_compaction(
            &config,
            &budget,
            PromptProfile::Standard,
            estimated,
            &mut history,
            "s",
            &[],
            &store,
            &ctx,
            &events,
        )
        .await
        .expect("thirty old reads are worth cutting");
        assert_eq!(breaks(&store.path), ["prune"]);
        std::fs::remove_dir_all(store.path.parent().unwrap()).ok();
    }

    #[tokio::test]
    async fn a_prune_that_is_not_taken_records_nothing() {
        let config = AgentConfig::default();
        let store = temp_store("no-prune");
        let ctx = crate::agent::tools::tests_support::ctx();
        let events: Channel<AgentEvent> = Channel::new(|_| Ok(()));
        let budget = ContextBudget::for_model(&config, &config.builder_model, 0);
        let mut history = vec![turn("user", ContentBlock::text("hi"))];
        let taken = try_verbatim_compaction(
            &config,
            &budget,
            PromptProfile::Standard,
            10,
            &mut history,
            "s",
            &[],
            &store,
            &ctx,
            &events,
        )
        .await;
        assert!(taken.is_none());
        assert!(breaks(&store.path).is_empty());
        std::fs::remove_dir_all(store.path.parent().unwrap()).ok();
    }

    #[tokio::test]
    async fn a_compaction_records_a_cache_break() {
        let store = temp_store("compact");
        store
            .append(&SessionRecord::User {
                text: "Refactor the parser.".into(),
                ts: now_ms(),
            })
            .unwrap();
        let (base, _, _) =
            spawn_recording(vec![Reply::Text("## Task & intent\nRefactor the parser.")]).await;
        let config = crate::agent::run_loop::surface_tests::config_for(&base, None);
        let ctx = ToolContext {
            session_store_path: Some(store.path.to_string_lossy().to_string()),
            ..crate::agent::tools::tests_support::ctx()
        };
        let events: Channel<AgentEvent> = Channel::new(|_| Ok(()));
        compact_history(
            &config,
            &store,
            &ctx,
            &events,
            &ApprovalMap::default(),
            &AnswerMap::default(),
            "cache-break-compact",
            &Arc::new(SteeringCtl::new()),
            crate::agent::hooks::CompactTrigger::Manual,
            10_000,
        )
        .await
        .expect("the compaction");
        assert_eq!(breaks(&store.path), ["compaction"]);
        std::fs::remove_dir_all(store.path.parent().unwrap()).ok();
    }

    /// Lean-harness Phase 6: the compaction note also lands in
    /// `.claudinio/notes/<session>.md`, where the user and a later successor
    /// can read it, and the compacted context says where.
    #[tokio::test]
    async fn a_compaction_appends_its_note_to_the_sessions_notes_file() {
        let store = temp_store("compact-note");
        let root = store.path.parent().unwrap().to_path_buf();
        store
            .append(&SessionRecord::User {
                text: "Refactor the parser.".into(),
                ts: now_ms(),
            })
            .unwrap();
        let note = "## Goal\nRefactor the parser.\n## What failed\nerror[E0308]: mismatched types";
        let (base, _, _) = spawn_recording(vec![Reply::Text(note)]).await;
        let config = crate::agent::run_loop::surface_tests::config_for(&base, None);
        let ctx = ToolContext {
            workspace_root: Some(root.to_string_lossy().to_string()),
            session_store_path: Some(store.path.to_string_lossy().to_string()),
            ..crate::agent::tools::tests_support::ctx()
        };
        let events: Channel<AgentEvent> = Channel::new(|_| Ok(()));
        let summary = compact_history(
            &config,
            &store,
            &ctx,
            &events,
            &ApprovalMap::default(),
            &AnswerMap::default(),
            "compact-note",
            &Arc::new(SteeringCtl::new()),
            crate::agent::hooks::CompactTrigger::Manual,
            10_000,
        )
        .await
        .expect("the compaction");
        let file = root.join(".claudinio/notes/compact-note.md");
        let text = std::fs::read_to_string(&file).expect("the notes file");
        assert!(text.starts_with("## Compaction — "), "{text}");
        assert!(text.contains(note), "{text}");
        assert!(summary.starts_with(note), "{summary}");
        assert!(
            summary.contains(".claudinio/notes/compact-note.md"),
            "{summary}"
        );
        let records = crate::agent::persist::load_records(&store.path).unwrap();
        let history = crate::agent::persist::history_from_records(&records);
        let first = history[0].content[0]
            .get_text()
            .unwrap_or_default()
            .to_string();
        assert!(
            first.contains(".claudinio/notes/compact-note.md"),
            "{first}"
        );
        std::fs::remove_dir_all(&root).ok();
    }

    /// A compaction records the session document's outline as the file
    /// stands at that moment — the freshest one, not the run's start — so
    /// the rebuilt context can restate it after the summary.
    #[tokio::test]
    async fn a_compaction_records_the_documents_outline_as_the_file_stands() {
        let store = temp_store("compact-doc");
        let root = store.path.parent().unwrap().to_path_buf();
        store
            .append(&SessionRecord::User {
                text: "Write the recipe.".into(),
                ts: now_ms(),
            })
            .unwrap();
        let (rel, _) = crate::agent::doc_session::create(
            &root,
            Some(&store.path),
            "Pudim",
            &["origem".to_string(), "servir".to_string()],
        )
        .unwrap();
        store
            .append(&SessionRecord::DocumentLinked {
                path: rel.clone(),
                ts: now_ms(),
            })
            .unwrap();
        let ctx = ToolContext {
            workspace_root: Some(root.to_string_lossy().to_string()),
            session_store_path: Some(store.path.to_string_lossy().to_string()),
            ..crate::agent::tools::tests_support::ctx()
        };
        crate::agent::doc_session::write(
            &ctx,
            "s1",
            Some("Veio de Portugal."),
            crate::agent::document::Status::Done,
        )
        .unwrap();
        let (base, _, _) =
            spawn_recording(vec![Reply::Text("## Task & intent\nWrite the recipe.")]).await;
        let config = crate::agent::run_loop::surface_tests::config_for(&base, None);
        let events: Channel<AgentEvent> = Channel::new(|_| Ok(()));
        compact_history(
            &config,
            &store,
            &ctx,
            &events,
            &ApprovalMap::default(),
            &AnswerMap::default(),
            "compact-doc",
            &Arc::new(SteeringCtl::new()),
            crate::agent::hooks::CompactTrigger::Manual,
            10_000,
        )
        .await
        .expect("the compaction");
        let records = crate::agent::persist::load_records(&store.path).unwrap();
        let document = records.iter().rev().find_map(|r| match r {
            SessionRecord::Compacted { document, .. } => Some(document.clone()),
            _ => None,
        });
        let Some(Some(block)) = document else {
            panic!("no outline on the compaction: {document:?}");
        };
        assert!(
            block.starts_with(&format!("<document path=\"{rel}\">")),
            "{block}"
        );
        assert!(
            block.contains("s1 \u{b7} done \u{b7} main \u{b7} origem"),
            "{block}"
        );
        std::fs::remove_dir_all(&root).ok();
    }

    #[tokio::test]
    async fn a_context_handoff_records_a_cache_break() {
        let (base, _, _) = spawn_recording(vec![Reply::Text(
            "## Purpose of next session\nFinish the parser refactor.\n\
             ## Current state\nThe lexer is split; tests green.\n## Key decisions\nKeep the API.\n\
             ## Pointers\nsrc/parser.rs\n## In-flight work\nThe expression module.\n\
             ## Next actions\nMove the expression parser, run the tests.",
        )])
        .await;
        let config = crate::agent::run_loop::surface_tests::config_for(&base, None);
        let store = temp_store("handoff");
        let ctx = crate::agent::tools::tests_support::ctx();
        let budget = ContextBudget::for_model(&config, &config.builder_model, 10_000);
        let mut history = vec![
            turn("user", ContentBlock::text("Refactor the parser.")),
            turn("assistant", ContentBlock::text("Working on it.")),
        ];
        let events: Channel<AgentEvent> = Channel::new(|_| Ok(()));
        let outcome = maybe_context_handoff(
            &config,
            &budget,
            PromptProfile::Standard,
            budget.soft,
            &mut history,
            "s",
            &store,
            &ctx,
            &events,
            "cache-break-handoff",
            &Arc::new(SteeringCtl::new()),
            &Arc::new(ModeCtl::new(SessionMode::Builder, ModeOrigin::Human)),
            0,
            0,
            0,
        )
        .await;
        assert!(matches!(outcome, Some(RunOutcome::Handoff(_))));
        assert_eq!(breaks(&store.path), ["handoff"]);
        std::fs::remove_dir_all(store.path.parent().unwrap()).ok();
    }
}
