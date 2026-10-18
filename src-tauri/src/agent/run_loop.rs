//! The provider → tool loop for one user message (`run_workflow_with_profile`)
//! and the finish-line checks it runs before a goal may close. What happens
//! inside one tool call is `tool_exec`; what happens at a context line is
//! `handoff`.

use crate::agent::budget::ContextBudget;
use crate::agent::events::{AgentEvent, QualityLayerView};
use crate::agent::handoff::{
    HandoffReason, HandoffSpec, RunOutcome, compact_history, has_something_to_compress,
    maybe_context_handoff, try_verbatim_compaction,
};
use crate::agent::judge::{
    TurnVerdict, judge_terminal_turn_metered, judge_with_jev, loop_verdict, should_nudge_terminal,
};
use crate::agent::permissions;
use crate::agent::permissions::PermissionLevel;
use crate::agent::persist::{SessionRecord, SessionStore, now_ms};
use crate::agent::prompt::{build_spec_prompt_section, deferred_tools, run_prefix};
use crate::agent::provider::{AgentConfig, ContentBlock, Message};
use crate::agent::retry::stream_message_with_retry;
use crate::agent::route::route_first_prompt;
use crate::agent::run_state::{CostLedger, GoldenLoopState, GuardState};
use crate::agent::session::{
    AnswerMap, ApprovalMap, ModeCtl, ModeOrigin, PromptProfile, SessionMode, SteeringCtl,
    cost_or_estimate, estimate_tokens, inject_steering, prefix_breakdown, push_turn,
    push_user_blocks, write_status,
};
use crate::agent::subagent;
use crate::agent::tool_batch;
use crate::agent::tool_exec::{
    MAX_STOP_HOOK_BLOCKS, deny_tool, fit_round, force_explore_mode, handle_mode_switch, run_tool,
    tool_response_for_hook,
};
use crate::agent::tools::ToolContext;
use serde_json::Value;
use std::sync::Arc;
use std::sync::atomic::{AtomicBool, Ordering};
use tauri::ipc::Channel;

/// Golden-loop safety caps used when the config leaves them unset.
const DEFAULT_MAX_GOLDEN_CYCLES: usize = 5;
const DEFAULT_MAX_GOLDEN_STALLS: usize = 2;
/// How many times the harness sends the model back over a failing quality
/// gate before stopping honestly. A suite it cannot fix in three tries is a
/// suite it will not fix in ten, and looping burns the user's tokens.
const MAX_QUALITY_RETRIES: u32 = 3;
/// Run the checks the workspace enforces, at the finish line — but give way
/// the moment the user speaks or pauses.
///
/// A suite takes a minute on a good day. Left to run, it holds a message the
/// user just typed behind it; so a queued steering message (or the pause flag)
/// stops the checks, and the caller goes on to answer. `None` means exactly
/// that: cut short, nothing verified, nothing recorded as evidence (see
/// `run_and_record`), so the next finish runs them again.
async fn run_finish_line_checks(
    ctx: &ToolContext,
    steering: &Arc<SteeringCtl>,
) -> Option<Result<crate::quality::QualityReport, String>> {
    let stop = Arc::new(AtomicBool::new(false));
    // The runner only knows one way to be stopped, the context's interrupt
    // flag — so it gets one that also trips on a queued message.
    let mut gate_ctx = ctx.clone();
    gate_ctx.interrupt = Some(stop.clone());
    let watcher = {
        let (steering, stop) = (steering.clone(), stop.clone());
        tokio::spawn(async move {
            loop {
                if steering.interrupt.load(Ordering::SeqCst) || steering.has_pending() {
                    stop.store(true, Ordering::SeqCst);
                    return;
                }
                tokio::time::sleep(std::time::Duration::from_millis(200)).await;
            }
        })
    };
    let result = crate::agent::tools::quality::run_enforced(&gate_ctx, "harness").await;
    watcher.abort();
    if stop.load(Ordering::SeqCst) {
        None
    } else {
        Some(result)
    }
}

/// Run a single continuous provider→tool loop for one user input, until the
/// model produces a turn with no tool calls. Shares one conversation history
/// (append-only, cache-friendly) and persists every step to the session JSONL
/// store. The model decides at each round whether it still needs a tool call
/// or can answer directly — there are no forced phases.
#[allow(clippy::too_many_arguments)]
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
    mode_ctl: &Arc<ModeCtl>,
) -> Result<RunOutcome, String> {
    run_workflow_with_profile(
        config,
        history,
        user_message,
        attachment_blocks,
        event_tx,
        approvals,
        answers,
        session_id,
        ctx,
        store,
        steering,
        mode_ctl,
        PromptProfile::Standard,
    )
    .await
}

/// Same loop as `run_workflow`, with an explicit prompt/toolset profile.
/// `run_workflow` is the `Standard`-profile shorthand used by normal chat
/// sessions; dedicated flows (e.g. commit & push) call this directly.
#[allow(clippy::too_many_arguments)]
pub async fn run_workflow_with_profile(
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
    mode_ctl: &Arc<ModeCtl>,
    profile: PromptProfile,
) -> Result<RunOutcome, String> {
    // ── Hooks: SessionStart, then UserPromptSubmit ───────────────────────────
    //
    // Both fire here, before the user's text becomes a turn, because both may
    // add context to it and one may refuse it outright. SessionStart is fired
    // by the run rather than by whatever created the session: a session that is
    // merely *loaded* has no run to inject into, so its context would be
    // computed and dropped. The command layer decides which source this is and
    // leaves it here to be taken.
    let mut hook_context: Vec<String> = Vec::new();
    if let Some(hooks) = &ctx.hooks {
        let pending = hooks
            .pending_session_start
            .lock()
            .ok()
            .and_then(|mut p| p.take());
        if let Some(source) = pending {
            let out = crate::agent::hooks::fire_session_start(hooks, source, Some(event_tx)).await;
            if let Some(text) = out.context() {
                hook_context.push(text);
            }
        }

        // Not for GitSync: commit & push has no user prompt to submit, and a
        // hook that reads `.prompt` would be handed a git instruction the user
        // never typed.
        let out = if profile.is_chat() {
            crate::agent::hooks::fire_user_prompt_submit(hooks, &user_message, Some(event_tx)).await
        } else {
            crate::agent::hooks::BatchOutcome::default()
        };
        // A blocked prompt takes the same path a rejected one does: persisted
        // as `Rejected` so it never vanishes from the JSONL, and returned as an
        // error the existing UI already renders. The model never sees it.
        if let Some(reason) = out.blocking_message().or(out.stop.clone()) {
            store.try_append(&SessionRecord::Rejected {
                text: user_message.clone(),
                reason: reason.clone(),
                ts: now_ms(),
            });
            crate::agent::persist::invalidate_cache(&store.path, &ctx.records_cache);
            let _ = event_tx.send(AgentEvent::Done {
                stop_reason: "hook_blocked".into(),
                text_output: reason.clone(),
                input_tokens: 0,
                output_tokens: 0,
            });
            return Err(reason);
        }
        if let Some(text) = out.context() {
            hook_context.push(text);
        }
    }

    let route_cost = route_first_prompt(
        config,
        profile,
        history.is_empty(),
        &user_message,
        store,
        ctx,
        mode_ctl,
        event_tx,
    )
    .await;

    store.try_append(&SessionRecord::User {
        text: user_message.clone(),
        ts: now_ms(),
    });
    crate::agent::persist::invalidate_cache(&store.path, &ctx.records_cache);
    let mut blocks = vec![ContentBlock::text(&user_message)];
    blocks.extend(attachment_blocks);
    // Injected context rides inside the same turn as the user's text, which is
    // what makes it replay correctly from the JSONL on reload — no second
    // replay mechanism, no chance of the context and the prompt drifting apart.
    for text in &hook_context {
        blocks.push(ContentBlock::text(format!(
            "<hook-context>\n{text}\n</hook-context>"
        )));
    }
    push_user_blocks(history, store, ctx, blocks);

    let skill_mgr = crate::agent::skills::SkillManager::with_plugin_prefs(
        ctx.workspace_root.as_ref().map(std::path::PathBuf::from),
        &config.plugins,
    );
    let skills_section = crate::agent::skills::build_skills_system_prompt_section(&skill_mgr);
    // The specs are the requirement. Putting them in the prompt is what makes
    // them an input to planning rather than a document nobody opens.
    let spec_section = build_spec_prompt_section(ctx);
    let (mut cur_mode, _) = mode_ctl.get();
    // MCP tool discovery already happened before `run_workflow` was called
    // (the caller awaits `ensure_mcp_connected`), so this is a cheap sync
    // snapshot read, not a fresh connection attempt.
    let mcp_defs = ctx
        .mcp
        .as_ref()
        .map(|m| m.cached_defs())
        .unwrap_or_default();
    // Read once: goals are written before the run starts and never minted
    // inside it, and a profile that moved mid-run would move the cached prefix.
    let has_golden_goals = ctx
        .session_store_path
        .as_deref()
        .and_then(|p| crate::agent::persist::load_last_tasks(std::path::Path::new(p)).ok())
        .is_some_and(|t| t.iter().any(crate::agent::tools::tasks::is_golden));
    // From here on `profile` is the one the run actually uses; what the caller
    // asked for is kept to derive it again when the mode — and with it the
    // model and its window — changes.
    let requested_profile = profile;
    // Deferred tools this session loaded in earlier runs, in load order: the
    // tool array starts where the last request left it (`agent::deferred`).
    let mut loaded = crate::agent::deferred::loaded_names(
        &crate::agent::persist::load_records_cached(&store.path, &ctx.records_cache)
            .unwrap_or_default(),
    );
    let mut prefix_loaded = loaded.len();
    let (mut profile, mut system, mut tools) = run_prefix(
        config,
        ctx,
        requested_profile,
        cur_mode,
        skills_section.as_deref(),
        spec_section.as_deref(),
        &mcp_defs,
        has_golden_goals,
        &loaded,
    );
    ctx.limits.set_writes_files(profile.edits_files(cur_mode));
    // Every line the run is held to comes from the model it is about to talk
    // to, and is re-derived whenever a mode switch changes that model.
    let mut prefix_tokens = estimate_tokens(&[], &system, &tools);
    let mut budget = ContextBudget::for_model(
        config,
        config.model_for_mode(cur_mode.as_str()),
        prefix_tokens,
    );
    ctx.limits.apply(&budget);
    // Before anything else is tried: no handoff and no summary can shrink
    // what is sent ahead of the conversation, and each would cost a request —
    // and the user's message, replaced by a summary of it — to find that out.
    if budget.prefix_overflows(prefix_tokens) {
        return Err(budget.refusal(prefix_tokens));
    }

    {
        let model = config.model_for_mode(cur_mode.as_str());
        store.try_append(&SessionRecord::RunConfig {
            model: model.to_string(),
            mode: cur_mode.as_str().into(),
            context_window: budget.window,
            regime: budget.regime.as_str().into(),
            soft: budget.soft,
            hard: budget.hard,
            ceiling: budget.ceiling,
            prefix: prefix_breakdown(
                &system,
                skills_section.as_deref(),
                spec_section.as_deref(),
                &tools,
            ),
            profile: profile.as_str().into(),
            // The surface is a Standard-profile setting; elsewhere it was not
            // applied, and saying "lean" would describe a prefix that is not.
            surface: if profile == PromptProfile::Standard {
                config.tool_surface_for(model).as_str().into()
            } else {
                String::new()
            },
            ts: now_ms(),
        });
        crate::agent::persist::invalidate_cache(&store.path, &ctx.records_cache);
    }

    // Auto-compact when the context exceeds the threshold. Prefer the real
    // input_tokens the API reported for the last request; the char-based
    // estimate is the fallback (take the max of the two for safety).
    let records = crate::agent::persist::load_records_cached(&store.path, &ctx.records_cache)
        .unwrap_or_default();
    let estimated = estimate_tokens(history, &system, &tools)
        .max(crate::agent::persist::last_context_tokens(&records).unwrap_or(0));
    // Lossless first: Jev drops old tool traffic, every word stays. Only when
    // that is unavailable or not enough does the handoff / summary run.
    let mut estimated = estimated;
    let mut pre_run_jev_cost = route_cost;
    if let Some((new_estimate, cost)) = try_verbatim_compaction(
        config, &budget, profile, estimated, history, &system, &tools, store, ctx, event_tx,
    )
    .await
    {
        estimated = new_estimate;
        pre_run_jev_cost += cost;
    }
    // Context-handoff next (Standard sessions): the model compresses its own
    // context and the run continues in a fresh linked session. Compaction
    // below stays as the safety net when generation fails or doesn't apply.
    if let Some(outcome) = maybe_context_handoff(
        config, &budget, profile, estimated, history, &system, store, ctx, event_tx, session_id,
        steering, mode_ctl, 0, 0, 0,
    )
    .await
    {
        return Ok(outcome);
    }
    if estimated >= budget.hard && has_something_to_compress(history) {
        let _ = event_tx.send(AgentEvent::TextStep {
            text: format!(
                "__compact_start__:{}/{}",
                estimated / 1000,
                budget.ceiling / 1000
            ),
        });
        match compact_history(
            config,
            store,
            ctx,
            event_tx,
            approvals,
            answers,
            session_id,
            steering,
            crate::agent::hooks::CompactTrigger::Auto,
            budget.tail_tokens,
        )
        .await
        {
            Ok(_) => {
                // Rebuild the history exactly as a session reload would:
                // summary + kept-verbatim tail (which already contains the
                // just-persisted user message) + nothing else.
                *history = crate::agent::persist::history_from_records(
                    &crate::agent::persist::load_records_cached(&store.path, &ctx.records_cache)
                        .unwrap_or_default(),
                );
                let new_context = estimate_tokens(history, &system, &tools);
                let (ci, co, cc, cci, cco, ccc) = crate::agent::persist::cumulative_stats(
                    &crate::agent::persist::load_records_cached(&store.path, &ctx.records_cache)
                        .unwrap_or_default(),
                );
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
                let _ = event_tx.send(AgentEvent::SessionStats {
                    input_tokens: ci as u32,
                    output_tokens: co as u32,
                    cumulative_cost: cc,
                    cost_input: cci,
                    cost_output: cco,
                    cost_cache_read: ccc,
                    context_tokens: new_context,
                    max_context_tokens: budget.ceiling,
                    compact_threshold: budget.hard,
                });
                let _ = event_tx.send(AgentEvent::TextStep {
                    text: format!(
                        "__compact_done__:{}/{}",
                        estimated / 1000,
                        new_context / 1000
                    ),
                });
            }
            Err(e) => {
                let _ = event_tx.send(AgentEvent::TextStep {
                    text: format!("__compact_fail__:{e}"),
                });
            }
        }
    }

    // Pre-flight guard: if after (attempted) compaction the context still
    // exceeds the real model limit, return a friendly error instead of
    // calling the API and wasting tokens on a guaranteed failure.
    {
        let post_compact = estimate_tokens(history, &system, &tools).max(
            crate::agent::persist::last_context_tokens(
                &crate::agent::persist::load_records_cached(&store.path, &ctx.records_cache)
                    .unwrap_or_default(),
            )
            .unwrap_or(0),
        );
        if post_compact >= budget.ceiling {
            return Err(budget.refusal(prefix_tokens));
        }
    }

    // Load cumulative totals from the last Status record
    let cumul = crate::agent::persist::cumulative_stats(
        &crate::agent::persist::load_records_cached(&store.path, &ctx.records_cache)
            .unwrap_or_default(),
    );
    let mut ledger = CostLedger::resuming(cumul);
    ledger.jev_cost += pre_run_jev_cost;
    let emit_final_stats = |ledger: &CostLedger, last_context: u64, budget: &ContextBudget| {
        let _ = event_tx.send(AgentEvent::SessionStats {
            input_tokens: ledger.cumul_in as u32,
            output_tokens: ledger.cumul_out as u32,
            cumulative_cost: ledger.cumul_cost,
            cost_input: ledger.cumul_cost_input,
            cost_output: ledger.cumul_cost_output,
            cost_cache_read: ledger.cumul_cost_cache,
            context_tokens: last_context,
            max_context_tokens: budget.ceiling,
            compact_threshold: budget.hard,
        });
    };
    let mut last_text = String::new();
    // Size of the context for the next request: the real number reported by
    // the API when available, the char-based estimate otherwise.
    let mut last_context: u64 = estimate_tokens(history, &system, &tools);
    let mut guards = GuardState::default();

    // Golden-goals loop state. The cycle counter resumes from the session's
    // records so a restart doesn't reset the cap mid-loop; a linked session
    // additionally inherits the predecessor's counters (LinkedFrom record) so
    // the cycle/stall caps never reset across handoffs — without this every
    // golden flip would mint a fresh budget and the loop could run forever.
    let (linked_cycle, linked_stalls, linked_pending) = crate::agent::persist::linked_golden_state(
        &crate::agent::persist::load_records_cached(&store.path, &ctx.records_cache)
            .unwrap_or_default(),
    );
    let mut golden = GoldenLoopState {
        cycle: crate::agent::persist::golden_cycle_count(
            &crate::agent::persist::load_records_cached(&store.path, &ctx.records_cache)
                .unwrap_or_default(),
        )
        .max(linked_cycle),
        last_pending: linked_pending,
        stalls: linked_stalls as usize,
    };

    // Plan-finalization state. `guards.plan_finalized` flips when the agent calls the
    // finalize_plan tool this run; `guards.finalize_nudged` bounds the enforcement to a
    // single reminder before the harness falls back to auto-appending the log.

    // Brain progress guard: track consecutive rounds where the agent only uses
    // explore tools without interviewing (ask_user), writing a plan (write_plan),
    // or creating tasks (tasks_set). After BRAIN_EXPLORE_LIMIT rounds, inject
    // a system reminder redirecting the agent to the required deliverables.
    const BRAIN_EXPLORE_LIMIT: u32 = 4;

    // Anchor the diff window at the true start of the plan's work: record the
    // git HEAD once per session (guarded), so finalize_plan can report every
    // changed file / commit since planning began, even across resumed runs.
    if let Some(sha) = ctx.base_commit.as_deref() {
        let already = crate::agent::persist::has_base_commit(
            &crate::agent::persist::load_records_cached(&store.path, &ctx.records_cache)
                .unwrap_or_default(),
        );
        if !already {
            store.try_append(&SessionRecord::BaseCommit {
                sha: sha.to_string(),
                ts: now_ms(),
            });
            crate::agent::persist::invalidate_cache(&store.path, &ctx.records_cache);
        }
    }

    let max_rounds = config.max_rounds.unwrap_or(usize::MAX);
    // Provider requests made by this run, for the per-request `Usage` record.
    let mut requests: u32 = 0;
    for _ in 0..max_rounds {
        // The mode can change mid-run (human toggle, or the agent's own
        // enter/exit_plan_mode in the previous round) — refresh the prompt
        // and tool list before each request.
        // A deferred-tool load in the last round grows the list the same way.
        let (mode_now, _) = mode_ctl.get();
        if mode_now != cur_mode || loaded.len() != prefix_loaded {
            cur_mode = mode_now;
            prefix_loaded = loaded.len();
            (profile, system, tools) = run_prefix(
                config,
                ctx,
                requested_profile,
                cur_mode,
                skills_section.as_deref(),
                spec_section.as_deref(),
                &mcp_defs,
                has_golden_goals,
                &loaded,
            );
            ctx.limits.set_writes_files(profile.edits_files(cur_mode));
            // A mode switch is also a model switch (brain_model vs
            // builder_model), and the two need not share a window.
            prefix_tokens = estimate_tokens(&[], &system, &tools);
            budget = ContextBudget::for_model(
                config,
                config.model_for_mode(cur_mode.as_str()),
                prefix_tokens,
            );
            ctx.limits.apply(&budget);
            if budget.prefix_overflows(prefix_tokens) {
                return Err(budget.refusal(prefix_tokens));
            }
        }

        // Per-round context re-check: tool_results from the previous round may
        // the next LLM call so we never feed an oversized context.
        let mut pre_tokens = estimate_tokens(history, &system, &tools);
        if let Some((new_estimate, cost)) = try_verbatim_compaction(
            config, &budget, profile, pre_tokens, history, &system, &tools, store, ctx, event_tx,
        )
        .await
        {
            pre_tokens = new_estimate;
            ledger.jev_cost += cost;
        }
        if let Some(outcome) = maybe_context_handoff(
            config,
            &budget,
            profile,
            pre_tokens,
            history,
            &system,
            store,
            ctx,
            event_tx,
            session_id,
            steering,
            mode_ctl,
            ledger.total_in,
            ledger.total_out,
            ledger.total_cache,
        )
        .await
        {
            return Ok(outcome);
        }
        if pre_tokens >= budget.hard && has_something_to_compress(history) {
            let _ = event_tx.send(AgentEvent::TextStep {
                text: format!(
                    "__compact_start__:{}/{}",
                    pre_tokens / 1000,
                    budget.ceiling / 1000
                ),
            });
            match compact_history(
                config,
                store,
                ctx,
                event_tx,
                approvals,
                answers,
                session_id,
                steering,
                crate::agent::hooks::CompactTrigger::Auto,
                budget.tail_tokens,
            )
            .await
            {
                Ok(_) => {
                    *history = crate::agent::persist::history_from_records(
                        &crate::agent::persist::load_records_cached(
                            &store.path,
                            &ctx.records_cache,
                        )
                        .unwrap_or_default(),
                    );
                    // Mode/system/tools may have changed mid-compact, refresh.
                    let (mode_now2, _) = mode_ctl.get();
                    if mode_now2 != cur_mode {
                        cur_mode = mode_now2;
                        (profile, system, tools) = run_prefix(
                            config,
                            ctx,
                            requested_profile,
                            cur_mode,
                            skills_section.as_deref(),
                            spec_section.as_deref(),
                            &mcp_defs,
                            has_golden_goals,
                            &loaded,
                        );
                        ctx.limits.set_writes_files(profile.edits_files(cur_mode));
                        prefix_tokens = estimate_tokens(&[], &system, &tools);
                        budget = ContextBudget::for_model(
                            config,
                            config.model_for_mode(cur_mode.as_str()),
                            prefix_tokens,
                        );
                        ctx.limits.apply(&budget);
                        if budget.prefix_overflows(prefix_tokens) {
                            return Err(budget.refusal(prefix_tokens));
                        }
                    }
                    let new_ctx = estimate_tokens(history, &system, &tools);
                    let (ci, co, cc, cci, cco, ccc) = crate::agent::persist::cumulative_stats(
                        &crate::agent::persist::load_records_cached(
                            &store.path,
                            &ctx.records_cache,
                        )
                        .unwrap_or_default(),
                    );
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
                        Some(new_ctx),
                    );
                    let _ = event_tx.send(AgentEvent::SessionStats {
                        input_tokens: ci as u32,
                        output_tokens: co as u32,
                        cumulative_cost: cc,
                        cost_input: cci,
                        cost_output: cco,
                        cost_cache_read: ccc,
                        context_tokens: new_ctx,
                        max_context_tokens: budget.ceiling,
                        compact_threshold: budget.hard,
                    });
                    let _ = event_tx.send(AgentEvent::TextStep {
                        text: format!("__compact_done__:{}/{}", pre_tokens / 1000, new_ctx / 1000),
                    });
                }
                Err(e) => {
                    let _ = event_tx.send(AgentEvent::TextStep {
                        text: format!("__compact_fail__:{e}"),
                    });
                }
            }
        }

        // Pre-flight guard (per-round): if compaction failed or context still
        // exceeds the limit, return friendly error.
        {
            let cur_ctx = estimate_tokens(history, &system, &tools).max(
                crate::agent::persist::last_context_tokens(
                    &crate::agent::persist::load_records_cached(&store.path, &ctx.records_cache)
                        .unwrap_or_default(),
                )
                .unwrap_or(0),
            );
            if cur_ctx >= budget.ceiling {
                return Err(budget.refusal(prefix_tokens));
            }
        }

        let mut assistant_text = String::new();
        let resolved_model = config.model_for_mode(cur_mode.as_str());
        let net_detail = format!("{resolved_model} · {}", cur_mode.as_str());
        let stream_output = stream_message_with_retry(
            config,
            resolved_model,
            history,
            &tools,
            Some(system.as_str()),
            event_tx,
            session_id,
            &mut assistant_text,
            &steering.interrupt,
            &net_detail,
        )
        .await?;
        requests += 1;
        store.try_append(&SessionRecord::usage(
            requests,
            resolved_model,
            stream_output.usage.as_ref(),
            None,
        ));
        crate::agent::persist::invalidate_cache(&store.path, &ctx.records_cache);

        let text_output = assistant_text;
        let tool_uses = stream_output.tool_uses;
        let was_interrupted = stream_output.interrupted;
        if let Some(u) = &stream_output.usage {
            ledger.total_in += u.input_tokens;
            ledger.total_out += u.output_tokens;
            ledger.total_cache += u.cache_read_input_tokens;
            // Use provider-reported cost if available, otherwise estimate
            if let Some(c) = u.cost {
                ledger.run_cost = Some(ledger.run_cost.unwrap_or(0.0) + c);
            }
            if let Some(c) = u.cost_input {
                ledger.run_cost_input = Some(ledger.run_cost_input.unwrap_or(0.0) + c);
            }
            if let Some(c) = u.cost_output {
                ledger.run_cost_output = Some(ledger.run_cost_output.unwrap_or(0.0) + c);
            }
            if let Some(c) = u.cost_cache_read {
                ledger.run_cost_cache = Some(ledger.run_cost_cache.unwrap_or(0.0) + c);
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
        let (round_ci, round_co, round_cc) = cost_or_estimate(
            resolved_model,
            ledger.total_in,
            ledger.total_cache,
            ledger.total_out,
            ledger.run_cost_input,
            ledger.run_cost_output,
            ledger.run_cost_cache,
        );
        let live_cost_input = ledger.cumul_cost_input.unwrap_or(0.0) + round_ci;
        let live_cost_output = ledger.cumul_cost_output.unwrap_or(0.0) + round_co;
        let live_cost_cache = ledger.cumul_cost_cache.unwrap_or(0.0) + round_cc;
        let _ = event_tx.send(AgentEvent::SessionStats {
            input_tokens: ledger.total_in + ledger.cumul_in as u32,
            output_tokens: ledger.total_out + ledger.cumul_out as u32,
            cumulative_cost: Some(
                ledger.cumul_cost.unwrap_or(0.0)
                    + round_ci
                    + round_co
                    + round_cc
                    + ledger.subagent_cost,
            ),
            cost_input: Some(live_cost_input),
            cost_output: Some(live_cost_output),
            cost_cache_read: Some(live_cost_cache),
            context_tokens: last_context,
            max_context_tokens: budget.ceiling,
            compact_threshold: budget.hard,
        });

        // Interrupted mid-stream: persist any partial text, reset the flag,
        // then either apply queued steering or pause.
        if was_interrupted {
            if !text_output.is_empty() {
                push_turn(
                    history,
                    store,
                    ctx,
                    Message {
                        role: "assistant".into(),
                        content: vec![ContentBlock::text(&text_output)],
                    },
                );
                let _ = event_tx.send(AgentEvent::FinalText {
                    text: text_output.clone(),
                });
                last_text = text_output;
            }
            steering.interrupt.store(false, Ordering::SeqCst);
            if inject_steering(history, store, ctx, steering, event_tx) {
                continue;
            }
            if last_text.is_empty() {
                last_text = "Paused by the user.".into();
            }
            store.try_append(&SessionRecord::Done {
                input_tokens: ledger.total_in,
                output_tokens: ledger.total_out,
                cache_read_tokens: Some(ledger.total_cache),
                ts: now_ms(),
            });
            crate::agent::persist::invalidate_cache(&store.path, &ctx.records_cache);
            ledger.roll(resolved_model);
            ledger.write_status(store, ctx, session_id, Some(last_context));
            emit_final_stats(&ledger, last_context, &budget);
            let _ = event_tx.send(AgentEvent::Done {
                stop_reason: "interrupted".into(),
                text_output: last_text,
                input_tokens: ledger.total_in,
                output_tokens: ledger.total_out,
            });
            return Ok(RunOutcome::Completed);
        }

        // Truncated at the output-token cap (stop_reason "max_tokens"): the
        // model was cut off mid-generation, so an empty tool list here does
        // NOT mean the turn is done. Persist any partial text and nudge the
        // model to continue instead of silently abandoning the task. When
        // complete tool calls did come through, fall through and run them —
        // the loop continues naturally.
        let truncated = stream_output.stop_reason.as_deref() == Some("max_tokens");
        if truncated && tool_uses.is_empty() {
            guards.truncation_streak += 1;
            if !text_output.is_empty() {
                push_turn(
                    history,
                    store,
                    ctx,
                    Message {
                        role: "assistant".into(),
                        content: vec![ContentBlock::text(&text_output)],
                    },
                );
                let _ = event_tx.send(AgentEvent::FinalText {
                    text: text_output.clone(),
                });
                last_text = text_output;
            }
            if guards.truncation_streak < 3 {
                push_user_blocks(
                    history,
                    store,
                    ctx,
                    vec![ContentBlock::text(crate::agent::nudges::CUT_OFF)],
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
                last_text = format!(
                    "{last_text}\n\n(Response truncated at the token limit — it may be incomplete.)"
                );
            }
            store.try_append(&SessionRecord::Done {
                input_tokens: ledger.total_in,
                output_tokens: ledger.total_out,
                cache_read_tokens: Some(ledger.total_cache),
                ts: now_ms(),
            });
            crate::agent::persist::invalidate_cache(&store.path, &ctx.records_cache);
            ledger.roll(resolved_model);
            ledger.write_status(store, ctx, session_id, Some(last_context));
            emit_final_stats(&ledger, last_context, &budget);
            let _ = event_tx.send(AgentEvent::Done {
                stop_reason: "max_tokens".into(),
                text_output: last_text,
                input_tokens: ledger.total_in,
                output_tokens: ledger.total_out,
            });
            return Ok(RunOutcome::Completed);
        }
        if !truncated {
            guards.truncation_streak = 0;
        }

        // Empty response: no text AND no tool calls, without truncation. A real
        // final turn always carries text, so mid-task this is a model glitch —
        // nudge it to continue instead of silently ending the run (same pattern
        // as the truncation nudge above). Give up after repeated empties.
        if tool_uses.is_empty() && text_output.is_empty() {
            guards.empty_streak += 1;
            if guards.empty_streak < 3 {
                push_user_blocks(
                    history,
                    store,
                    ctx,
                    vec![ContentBlock::text(crate::agent::nudges::EMPTY_REPLY)],
                );
                continue;
            }
        } else {
            guards.empty_streak = 0;
        }

        // Terminal turn: no tool calls — the loop is done, this text is the reply.
        if tool_uses.is_empty() {
            if !text_output.is_empty() {
                push_turn(
                    history,
                    store,
                    ctx,
                    Message {
                        role: "assistant".into(),
                        content: vec![ContentBlock::text(&text_output)],
                    },
                );
                let _ = event_tx.send(AgentEvent::FinalText {
                    text: text_output.clone(),
                });
                last_text = text_output;
            }
            // B — Antes de encerrar, verificar steering. Se houver, continuar.
            if inject_steering(history, store, ctx, steering, event_tx) {
                continue;
            }
            // A terminal end_turn whose text only *announces* a next step
            // ("Primeiro vou confirmar…:", "Vou explorar com subagentes:") but
            // carries no tool call is a model glitch, not a finish — the agent
            // narrated its intent and stopped instead of acting, leaving the run
            // dangling mid-task. Ask the model itself (language-agnostic, no
            // hardcoded phrases) whether the turn is really done; if not, nudge
            // it to actually take the action. Bounded so a genuinely-final reply
            // the judge misreads can't loop forever.
            if !last_text.is_empty() && guards.unfinished_streak < 2 {
                // Always judge with the Brain model (planning/reasoning), never
                // the Builder model, regardless of the session's current mode.
                let judge_model = config.model_for_mode(SessionMode::Brain.as_str());
                // Jev first when the user has a credential for it: a calibrated
                // yes/no at ~$0.00002 and ~0.2s instead of a cold call to the
                // Brain model. Its unclear answers, and any outage, fall through
                // to the LLM judge exactly as before.
                let (jev_verdict, jev_cost) = match crate::agent::jev::backend(config) {
                    Some(b) => judge_with_jev(&b, &last_text).await,
                    None => (None, 0.0),
                };
                ledger.jev_cost += jev_cost;
                let (verdict, judge) = match jev_verdict {
                    Some(v) => (v, "jev"),
                    None => {
                        let (verdict, request) =
                            judge_terminal_turn_metered(config, judge_model, &last_text).await;
                        // An answered judge request is a round of this run,
                        // with or without reported usage; one that failed
                        // never reached a model and is not counted.
                        if let Some(usage) = request {
                            requests += 1;
                            store.try_append(&SessionRecord::usage(
                                requests,
                                judge_model,
                                usage.as_ref(),
                                None,
                            ));
                        }
                        (verdict, "llm")
                    }
                };
                // Structural backstop on top of the judge's verdict: a message
                // ending in a question mark or a trailing colon ("the core
                // design question:") is a dangling question, not a final reply.
                let will_nudge =
                    verdict == TurnVerdict::Continue || should_nudge_terminal(&last_text);
                // Transparent to the user (no event emitted, UI renders nothing)
                // but auditable: persist the judge's decision to the JSONL.
                store.try_append(&SessionRecord::ContinuationJudge {
                    verdict: match verdict {
                        TurnVerdict::Continue => "continue".into(),
                        TurnVerdict::Done => "done".into(),
                    },
                    nudged: will_nudge,
                    streak: guards.unfinished_streak + if will_nudge { 1 } else { 0 },
                    judge: Some(judge.into()),
                    ts: now_ms(),
                });
                crate::agent::persist::invalidate_cache(&store.path, &ctx.records_cache);
                if will_nudge {
                    guards.unfinished_streak += 1;
                    push_user_blocks(
                        history,
                        store,
                        ctx,
                        vec![ContentBlock::text(
                            crate::agent::nudges::ANNOUNCED_NOT_TAKEN,
                        )],
                    );
                    continue;
                }
            }
            // Golden verification: an end_turn with golden tasks still pending
            // is not a real finish — flip Brain↔Builder and send the model
            // back to work, bounded by the cycle and stall caps.
            let mut stop_reason = "end_turn";
            let golden_pending: Vec<String> = ctx
                .session_store_path
                .as_deref()
                .and_then(|p| crate::agent::persist::load_last_tasks(std::path::Path::new(p)).ok())
                .map(|t| crate::agent::tools::tasks::golden_pending_ids(&t))
                .unwrap_or_default();
            if !golden_pending.is_empty() {
                let max_cycles = config
                    .max_golden_cycles
                    .unwrap_or(DEFAULT_MAX_GOLDEN_CYCLES);
                let max_stalls = config
                    .max_golden_stalls
                    .unwrap_or(DEFAULT_MAX_GOLDEN_STALLS);
                if golden_pending == golden.last_pending {
                    golden.stalls += 1;
                } else {
                    golden.stalls = 0;
                }
                golden.last_pending = golden_pending.clone();
                if (golden.cycle as usize) < max_cycles && golden.stalls < max_stalls {
                    golden.cycle += 1;
                    let next = match cur_mode {
                        SessionMode::Brain => SessionMode::Builder,
                        SessionMode::Builder => SessionMode::Brain,
                    };
                    // The Mode record for `next` belongs to the successor
                    // session (link_session writes it); this session only logs
                    // the cycle for audit and closes its books.
                    store.try_append(&SessionRecord::GoldenCycle {
                        cycle: golden.cycle,
                        mode: next.as_str().into(),
                        goals: golden_pending.clone(),
                        ts: now_ms(),
                    });
                    store.try_append(&SessionRecord::Done {
                        input_tokens: ledger.total_in,
                        output_tokens: ledger.total_out,
                        cache_read_tokens: Some(ledger.total_cache),
                        ts: now_ms(),
                    });
                    crate::agent::persist::invalidate_cache(&store.path, &ctx.records_cache);
                    ledger.roll(resolved_model);
                    ledger.write_status(store, ctx, session_id, Some(last_context));
                    let _ = event_tx.send(AgentEvent::GoldenLoop {
                        cycle: golden.cycle,
                        max_cycles: max_cycles as u32,
                        pending: golden_pending.clone(),
                        mode: next.as_str().into(),
                    });
                    let handoff_spec = HandoffSpec {
                        reason: HandoffReason::GoldenFlip,
                        next_mode: next,
                        next_origin: ModeOrigin::Agent,
                        first_message: crate::agent::nudges::golden_flip(
                            &golden_pending,
                            next.as_str(),
                            golden.cycle,
                            max_cycles,
                        ),
                        golden_cycle: golden.cycle,
                        golden_stalls: golden.stalls as u32,
                        golden_last_pending: golden_pending.clone(),
                    };
                    return Ok(RunOutcome::Handoff(Box::new(handoff_spec)));
                }
                // Cap hit: stop honestly with a specific reason so the user
                // sees WHY the loop gave up with goals unmet.
                stop_reason = if golden.stalls >= max_stalls {
                    "golden_stalled"
                } else {
                    "max_golden_cycles"
                };
                last_text = format!(
                    "{last_text}\n\n⚠️ Golden goals not achieved ({}): {}",
                    if stop_reason == "golden_stalled" {
                        "no progress across consecutive cycles"
                    } else {
                        "cycle limit reached"
                    },
                    golden_pending.join(", "),
                );
            }

            // Quality verification: the mechanical half of "the goal is met".
            // `tasks_set` already refuses to close an execution goal without
            // fresh green evidence, but the model can also simply stop talking
            // — so before this run is allowed to finish, the harness checks
            // the evidence itself and runs the checks when there are none.
            // Same pattern as auto_finalize below: never rely on the model
            // having done the required last step.
            // A tagged goal always demands proof; a workspace set to
            // `enforce_on: "code_change"` also demands it from any run that
            // touched source, so the harness protects people who never learned
            // the <goal> syntax.
            if stop_reason == "end_turn" && crate::agent::tools::quality::verification_required(ctx)
            {
                {
                    use crate::agent::tools::quality as qtool;
                    let evidence = qtool::current_evidence(ctx);
                    // A failing run whose digest still matches describes
                    // exactly the files on disk right now — nothing changed
                    // since, so re-running would spend minutes to learn the
                    // same thing. This is also the common case while the model
                    // is being sent back over a red gate.
                    let mut report = match &evidence {
                        qtool::Evidence::Current { pass: false, .. } => qtool::last_report(ctx),
                        _ => None,
                    };
                    // Nothing to do when no layer is enforced or the evidence
                    // is already green against these exact files. Otherwise we
                    // genuinely do not know, so we check for ourselves.
                    let must_run = report.is_none()
                        && !matches!(
                            evidence,
                            qtool::Evidence::NotRequired
                                | qtool::Evidence::Current { pass: true, .. }
                        );
                    if must_run {
                        let _ = event_tx.send(AgentEvent::TextStep {
                            text: "🧪 Verifying the goal: running the project's checks…".into(),
                        });
                        match run_finish_line_checks(ctx, steering).await {
                            // The user spoke or paused while the suite ran.
                            // Nothing was verified and nothing was recorded,
                            // so the next finish checks again from scratch.
                            None => {
                                let _ = event_tx.send(AgentEvent::TextStep {
                                    text: "⏸ Checks stopped before they finished — nothing was \
                                           verified."
                                        .into(),
                                });
                            }
                            Some(Ok(r)) => {
                                let _ = event_tx.send(AgentEvent::QualityVerdict {
                                    pass: r.verdict.pass,
                                    summary: r.summary_text(),
                                    layers: QualityLayerView::from_report(&r),
                                    trigger: "harness".into(),
                                });
                                report = Some(r);
                            }
                            Some(Err(e)) => {
                                // The harness itself could not run (no
                                // recognizable project, workspace gone). Fail
                                // open and say so: blocking a finish on OUR
                                // inability to check would strand the user
                                // with no way forward.
                                let _ = event_tx.send(AgentEvent::TextStep {
                                    text: format!("⚠️ Quality checks could not run: {e}"),
                                });
                            }
                        }
                    }

                    if let Some(report) = report.filter(|r| !r.verdict.pass) {
                        if guards.quality_retries < MAX_QUALITY_RETRIES {
                            guards.quality_retries += 1;
                            push_user_blocks(
                                history,
                                store,
                                ctx,
                                vec![ContentBlock::text(crate::agent::nudges::quality_failed(
                                    &report.summary_text(),
                                    &report.failure_detail(4_000),
                                    guards.quality_retries,
                                    MAX_QUALITY_RETRIES,
                                ))],
                            );
                            continue;
                        }
                        // Out of attempts: stop honestly with a reason the
                        // user can see, exactly like the golden cycle cap.
                        stop_reason = "quality_failed";
                        last_text = format!(
                            "{last_text}\n\n⚠️ Quality gate not met after {MAX_QUALITY_RETRIES} \
                             attempts:\n{}",
                            report.summary_text()
                        );
                    }
                }
            }

            // Feed the plan its Implementation Log when a goal-driven build
            // truly finishes: golden tasks existed and are all done (so this is
            // an honest end_turn, not a cap give-up), a plan file exists, and
            // finalize_plan wasn't called yet. One reminder, then the harness
            // auto-appends the log so the plan is ALWAYS fed. Fail-open: this
            // never blocks the finish.
            if !guards.plan_finalized && stop_reason == "end_turn" {
                let tasks = ctx
                    .session_store_path
                    .as_deref()
                    .and_then(|p| {
                        crate::agent::persist::load_last_tasks(std::path::Path::new(p)).ok()
                    })
                    .unwrap_or_default();
                let had_golden = tasks.iter().any(crate::agent::tools::tasks::is_golden);
                let plan_exists =
                    crate::agent::tools::finalize_plan::latest_plan_file(ctx).is_some();
                if had_golden && plan_exists {
                    if !guards.finalize_nudged {
                        guards.finalize_nudged = true;
                        push_user_blocks(
                            history,
                            store,
                            ctx,
                            vec![ContentBlock::text(crate::agent::nudges::FINALIZE_PLAN)],
                        );
                        continue;
                    }
                    // The model still skipped it — record the log ourselves.
                    if let Some(outcome) = crate::agent::tools::finalize_plan::auto_finalize(ctx) {
                        let _ = event_tx.send(AgentEvent::TextStep {
                            text: format!(
                                "📝 Implementation Log recorded to {}",
                                outcome.plan_file
                            ),
                        });
                    }
                }
            }

            // The checks above can run for minutes, and a message the user
            // sends meanwhile is only queued. The check at the top of this
            // branch ran before them, so without this one the run would end
            // with the message still in the queue — and the queue is dropped
            // when a run ends. Answer it instead. The user's message outranks
            // an earlier pause, as it does when a stream is interrupted.
            if inject_steering(history, store, ctx, steering, event_tx) {
                steering.interrupt.store(false, Ordering::SeqCst);
                continue;
            }

            // ── Hooks: Stop ──────────────────────────────────────────────
            //
            // The last possible moment: after the continuation judge, after the
            // golden loop, after the quality gate. Firing earlier would put a
            // hook in a fight with the harness's own continuation logic and
            // produce two nudges for one unfinished turn.
            if let Some(h) = &ctx.hooks
                && profile.is_chat()
                && guards.stop_hook_blocks < MAX_STOP_HOOK_BLOCKS
            {
                let out =
                    crate::agent::hooks::fire_stop(h, guards.stop_hook_active, Some(event_tx))
                        .await;
                if let Some(reason) = out.blocking_message() {
                    guards.stop_hook_active = true;
                    guards.stop_hook_blocks += 1;
                    push_user_blocks(
                        history,
                        store,
                        ctx,
                        vec![ContentBlock::text(format!(
                            "<hook-feedback>\n{reason}\n</hook-feedback>"
                        ))],
                    );
                    continue;
                }
                if let Some(reason) = out.stop {
                    last_text = reason;
                    stop_reason = "hook_stop";
                }
            }

            // A Stop hook is a user script and can be slow too: same rule as
            // above, unless the hook itself asked for the run to end.
            if stop_reason != "hook_stop"
                && inject_steering(history, store, ctx, steering, event_tx)
            {
                steering.interrupt.store(false, Ordering::SeqCst);
                continue;
            }

            // If the model didn't produce a final text response, provide a
            // generic closing so the user doesn't see a blank answer.
            if last_text.is_empty() {
                last_text = "Pronto. Como posso ajudar mais?".into();
            }
            store.try_append(&SessionRecord::Done {
                input_tokens: ledger.total_in,
                output_tokens: ledger.total_out,
                cache_read_tokens: Some(ledger.total_cache),
                ts: now_ms(),
            });
            crate::agent::persist::invalidate_cache(&store.path, &ctx.records_cache);
            ledger.roll(resolved_model);
            ledger.write_status(store, ctx, session_id, Some(last_context));
            emit_final_stats(&ledger, last_context, &budget);
            let _ = event_tx.send(AgentEvent::Done {
                stop_reason: stop_reason.into(),
                text_output: last_text,
                input_tokens: ledger.total_in,
                output_tokens: ledger.total_out,
            });
            return Ok(RunOutcome::Completed);
        }

        // The model recovered and is taking an action — reset the dangling-promise
        // guard so the cap only counts *consecutive* announce-but-don't-act turns.
        guards.unfinished_streak = 0;

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
        let mut pending_handoff: Option<HandoffSpec> = None;
        // Corrections and context a PostToolUse hook produced, appended to the
        // same user turn the tool results go into.
        let mut hook_tool_feedback: Vec<String> = Vec::new();
        // A hook asked for the run to end (`continue: false`).
        let mut hook_stop_request: Option<String> = None;

        // Results of read-only calls run ahead of the loop, by call index.
        let mut ready: Vec<Option<ContentBlock>> = vec![None; tool_uses.len()];
        // Deferred tools loaded before this round; what the round adds is
        // recorded once, after it (`agent::deferred::record`).
        let loaded_before_round = loaded.len();
        for (ti, tool_use) in tool_uses.iter().enumerate() {
            // At the head of a run of read-only, unprompted, unhooked calls,
            // run the whole run at once (`tool_batch`). Each result is then
            // taken below in the model's order, as if the call ran here.
            if ready[ti].is_none() && !steering.interrupt.load(Ordering::SeqCst) {
                let compact = profile == PromptProfile::Compact
                    && !matches!(mode_ctl.get().0, SessionMode::Brain);
                tool_batch::run_ahead(
                    &tool_uses,
                    ti,
                    &mut ready,
                    |name, input| tool_batch::runs_in_parallel(name, input, config, ctx, compact),
                    tool_batch::executor(event_tx, session_id, ctx),
                    &steering.interrupt,
                    event_tx,
                )
                .await;
            }
            // C — Entre tools: checar interrupt. Se setado, sintetizar
            // tool_result "interrompido" para este e todos os tool_uses restantes
            // (a call a parallel run already finished keeps its result).
            if steering.interrupt.load(Ordering::SeqCst) {
                tool_batch::answer_unrun(
                    &tool_uses[ti..],
                    &mut ready[ti..],
                    tool_batch::INTERRUPTED,
                    event_tx,
                    &mut tool_assistant_blocks,
                    &mut tool_result_blocks,
                    |name, input, block| {
                        if let ContentBlock::ToolResult { content, .. } = &block {
                            guards.loop_watch.record(name, input, &content.as_text());
                        }
                        crate::agent::prune::history_copy(history, name, input, block)
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

            tool_assistant_blocks.push(ContentBlock::tool_use(
                &tool_use_id,
                &tool_name,
                tool_input.clone(),
            ));

            // ── Hooks: PreToolUse ────────────────────────────────────────
            //
            // Fired here, in the orchestrator, rather than inside `run_tool`.
            // `run_tool` is never reached for `spawn_agents`, the plan-mode
            // switches or any of the Brain/Builder denials, so a hook matching
            // `Task` — one of the most common matchers there is — would never
            // fire from in there.
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
            let hook_verdict = hook_pre.verdict.clone();
            // `continue: false` can arrive from either side of a tool call.
            // Honoured after the current batch, not mid-loop, so the model
            // still gets a result for every tool_use it asked for — an
            // unanswered tool_use is a malformed conversation.
            if let Some(reason) = hook_pre.stop.clone() {
                hook_stop_request = Some(reason);
            }

            let in_brain = matches!(mode_ctl.get().0, SessionMode::Brain);
            // A Builder session changes files itself (`edits_files`: Compact,
            // and Standard since decision D1). Each change is asked about — an
            // `edit_file` as always, and a shell command that writes a file
            // even when its prefix is allowlisted (`run_tool`, through
            // `RunLimits::writes_files`). Brain's read-only gates are
            // untouched — a toggle mid-round is read here, per call.
            let direct_edit = profile.edits_files(mode_ctl.get().0);
            // Compact also refuses whatever lies outside its ten tools.
            let compact = profile == PromptProfile::Compact && !in_brain;
            // A denial short-circuits everything downstream, including the mode
            // gates — there is nothing left to decide once the tool will not
            // run. An `allow` or `ask` does NOT short-circuit: the mode gates
            // still apply. Hooks may relax a prompt; they may never relax a
            // policy.
            // `load_tools` is answered here; a deferred tool called before it
            // was loaded is loaded now and then runs like any other call.
            // Only Standard defers anything.
            let load_answer = if profile != PromptProfile::Standard
                || matches!(
                    hook_verdict,
                    crate::agent::hooks::PreToolVerdict::Deny { .. }
                ) {
                None
            } else {
                crate::agent::deferred::before_call(
                    &tool_name,
                    &tool_use_id,
                    &tool_input,
                    || deferred_tools(cur_mode, profile, &mcp_defs, config),
                    &mut loaded,
                    event_tx,
                    session_id,
                )
            };
            let block = if let Some(block) = ready[ti].take() {
                block
            } else if let Some(block) = load_answer {
                block
            } else if let crate::agent::hooks::PreToolVerdict::Deny { reason } = &hook_verdict {
                deny_tool(
                    &tool_name,
                    &tool_use_id,
                    &tool_input,
                    reason,
                    event_tx,
                    session_id,
                )
            } else if compact && !crate::agent::surface::COMPACT_TOOLS.contains(&tool_name.as_str())
            {
                // The profile exists because this model's window cannot hold
                // more than these: a call to anything else — a subagent, a
                // plan, an MCP server — is one it was never offered, and would
                // spend the window the profile was protecting.
                deny_tool(
                    &tool_name,
                    &tool_use_id,
                    &tool_input,
                    &format!(
                        "{tool_name} is not available in this session. Your tools are: {}.",
                        crate::agent::surface::COMPACT_TOOLS.join(", ")
                    ),
                    event_tx,
                    session_id,
                )
            } else if tool_name == "enter_plan_mode" || tool_name == "exit_plan_mode" {
                handle_mode_switch(
                    &tool_name,
                    &tool_use_id,
                    &tool_input,
                    mode_ctl,
                    store,
                    ctx,
                    event_tx,
                    session_id,
                    &mut pending_handoff,
                )
            } else if tool_name == "spawn_agents" {
                // Brain may only spawn read-only subagents.
                let tool_input = if in_brain {
                    force_explore_mode(tool_input)
                } else {
                    tool_input
                };
                let (block, sub_in, sub_out, sub_cost) = subagent::run_spawn_agents(
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
                ledger.total_in += sub_in;
                ledger.total_out += sub_out;
                ledger.subagent_cost += sub_cost;
                block
            } else if tool_name == "edit_file" && !direct_edit {
                // Not offered where the session does not edit (Brain, the git
                // operator); deny defensively in case the model hallucinates it.
                deny_tool(
                    &tool_name,
                    &tool_use_id,
                    &tool_input,
                    if in_brain {
                        "edit_file is not available in Brain mode — it is read-only. \
                         Record the intended change in the plan (write_plan) instead."
                    } else {
                        "edit_file is not available in this session."
                    },
                    event_tx,
                    session_id,
                )
            } else if !in_brain
                && !direct_edit
                && tool_name == "bash"
                && permissions::bash_writes_files(
                    tool_input
                        .get("command")
                        .and_then(|v| v.as_str())
                        .unwrap_or(""),
                )
            {
                deny_tool(
                    &tool_name,
                    &tool_use_id,
                    &tool_input,
                    "This bash command writes files, and this session does not edit \
                     files. Bash here is for git, builds, tests and read-only inspection.",
                    event_tx,
                    session_id,
                )
            } else if in_brain
                && tool_name == "bash"
                && permissions::bash_writes_files(
                    tool_input
                        .get("command")
                        .and_then(|v| v.as_str())
                        .unwrap_or(""),
                )
            {
                deny_tool(
                    &tool_name,
                    &tool_use_id,
                    &tool_input,
                    "This bash command writes files. In Brain mode you cannot mutate \
                     — record the change in the plan (write_plan) instead.",
                    event_tx,
                    session_id,
                )
            } else if in_brain
                && tool_name == "bash"
                && !matches!(
                    permissions::bash_permission(
                        tool_input
                            .get("command")
                            .and_then(|v| v.as_str())
                            .unwrap_or(""),
                        false
                    ),
                    PermissionLevel::Auto
                )
            {
                deny_tool(
                    &tool_name,
                    &tool_use_id,
                    &tool_input,
                    "Command not allowed in Brain mode — only read-only allowlisted \
                     commands (git status/diff/log, ls, cat, cargo check, ...) run here.",
                    event_tx,
                    session_id,
                )
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
                    config,
                    &hook_verdict,
                )
                .await
            };
            // Note a successful finalize_plan so the golden-completion gate
            // knows the plan was fed and skips the reminder / fallback.
            if tool_name == "finalize_plan"
                && let ContentBlock::ToolResult { content, .. } = &block
                && !content.as_text().starts_with("Error")
            {
                guards.plan_finalized = true;
            }

            // ── Hooks: PostToolUse ───────────────────────────────────────
            //
            // Skipped when the call was denied: a tool that never ran has no
            // result to inspect, and firing here would make every guarded tool
            // look like it executed.
            if let Some(h) = &ctx.hooks
                && !matches!(
                    hook_verdict,
                    crate::agent::hooks::PreToolVerdict::Deny { .. }
                )
            {
                let response = tool_response_for_hook(&block);
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
                if let Some(reason) = out.stop {
                    hook_stop_request = Some(reason);
                }
            }

            if let ContentBlock::ToolResult { content, .. } = &block {
                guards
                    .loop_watch
                    .record(&tool_name, &watched_input, &content.as_text());
            }
            // After the hook and the loop watch, which are about what the tool
            // returned; this is only about what is worth carrying twice.
            tool_result_blocks.push(crate::agent::prune::history_copy(
                history,
                &tool_name,
                &watched_input,
                block,
            ));
        }
        // However many calls loaded a deferred tool, the next request rewrites
        // the prefix once: one `tools_loaded` record, one cache break.
        crate::agent::deferred::record(store, ctx, &loaded[loaded_before_round..]);

        push_turn(
            history,
            store,
            ctx,
            Message {
                role: "assistant".into(),
                content: tool_assistant_blocks,
            },
        );
        // `history` now ends with the assistant turn these results answer.
        let calls = history.last().map_or(&[][..], |m| m.content.as_slice());
        fit_round(&mut tool_result_blocks, calls, budget.round_chars);
        push_turn(
            history,
            store,
            ctx,
            Message {
                role: "user".into(),
                content: tool_result_blocks,
            },
        );

        // A PostToolUse hook's correction is merged into that same user turn —
        // `push_user_blocks` exists for exactly this — so the model reads it as
        // part of the tool results rather than as a stray turn.
        if !hook_tool_feedback.is_empty() {
            let text = hook_tool_feedback.join("\n\n");
            push_user_blocks(
                history,
                store,
                ctx,
                vec![ContentBlock::text(format!(
                    "<hook-feedback>\n{text}\n</hook-feedback>"
                ))],
            );
        }

        // `continue: false` outranks every other decision a hook can make.
        if let Some(reason) = hook_stop_request {
            let _ = event_tx.send(AgentEvent::Done {
                stop_reason: "hook_stop".into(),
                text_output: reason,
                input_tokens: ledger.total_in,
                output_tokens: ledger.total_out,
            });
            store.try_append(&SessionRecord::Done {
                input_tokens: ledger.total_in,
                output_tokens: ledger.total_out,
                cache_read_tokens: Some(ledger.total_cache),
                ts: now_ms(),
            });
            crate::agent::persist::invalidate_cache(&store.path, &ctx.records_cache);
            return Ok(RunOutcome::Completed);
        }

        // If exit_plan_mode requested a handoff, persist and return it now.
        if let Some(handoff) = pending_handoff.take() {
            store.try_append(&SessionRecord::Done {
                input_tokens: ledger.total_in,
                output_tokens: ledger.total_out,
                cache_read_tokens: Some(ledger.total_cache),
                ts: now_ms(),
            });
            crate::agent::persist::invalidate_cache(&store.path, &ctx.records_cache);
            ledger.roll(resolved_model);
            ledger.write_status(store, ctx, session_id, Some(last_context));
            emit_final_stats(&ledger, last_context, &budget);
            // No AgentEvent::Done: the conversation continues in the linked
            // successor session — SessionLinked (emitted by link_session) is
            // what the UI reacts to.
            return Ok(RunOutcome::Handoff(Box::new(handoff)));
        }

        // Tool-loop watch: the same call with the same result, round after
        // round, is a cycle the model will not leave on its own. Nudge it
        // (bounded), then stop the run and say why — never silently.
        let (loop_action, loop_cost) = loop_verdict(
            &mut guards.loop_watch,
            crate::agent::jev::backend(config).as_ref(),
        )
        .await;
        ledger.jev_cost += loop_cost;
        match loop_action {
            crate::agent::loop_watch::LoopAction::None => {}
            crate::agent::loop_watch::LoopAction::Nudge(msg) => {
                push_user_blocks(history, store, ctx, vec![ContentBlock::text(msg)]);
            }
            crate::agent::loop_watch::LoopAction::Stop(msg) => {
                store.try_append(&SessionRecord::Done {
                    input_tokens: ledger.total_in,
                    output_tokens: ledger.total_out,
                    cache_read_tokens: Some(ledger.total_cache),
                    ts: now_ms(),
                });
                crate::agent::persist::invalidate_cache(&store.path, &ctx.records_cache);
                ledger.roll(resolved_model);
                ledger.write_status(store, ctx, session_id, Some(last_context));
                emit_final_stats(&ledger, last_context, &budget);
                let _ = event_tx.send(AgentEvent::Done {
                    stop_reason: "tool_loop".into(),
                    text_output: msg,
                    input_tokens: ledger.total_in,
                    output_tokens: ledger.total_out,
                });
                return Ok(RunOutcome::Completed);
            }
        }

        // Brain progress guard: if the agent is in Brain mode and only using
        // explore tools without interviewing/planning/tasking, redirect it.
        let is_brain = matches!(mode_ctl.get().0, SessionMode::Brain);
        if is_brain {
            let explore_tools = [
                "spawn_agents",
                "semantic_search",
                "code_search",
                "grep",
                "read_file",
                "file_outline",
                "list_dir",
                "symbol_lookup",
                "go_to_definition",
                "find_references",
            ];
            let is_explore_only = tool_uses.iter().all(|t| {
                let name = t.get("name").and_then(|v| v.as_str()).unwrap_or("");
                explore_tools.contains(&name)
            });
            let has_progress_tool = tool_uses.iter().any(|t| {
                let name = t.get("name").and_then(|v| v.as_str()).unwrap_or("");
                name == "ask_user" || name == "write_plan" || name == "tasks_set"
            });

            if has_progress_tool {
                guards.brain_explore_streak = 0;
            } else if is_explore_only {
                guards.brain_explore_streak += 1;
                if guards.brain_explore_streak >= BRAIN_EXPLORE_LIMIT {
                    guards.brain_explore_streak = 0; // Reset so we don't loop-spam
                    push_user_blocks(
                        history,
                        store,
                        ctx,
                        vec![ContentBlock::text(crate::agent::nudges::BRAIN_EXPLORING)],
                    );
                    continue;
                }
            } else {
                // Some mixed tools that aren't purely explore — don't count.
            }
        }

        // Interrupt check after tool results.
        if steering.interrupt.swap(false, Ordering::SeqCst) {
            if inject_steering(history, store, ctx, steering, event_tx) {
                continue;
            }
            if last_text.is_empty() {
                last_text = "Paused by the user.".into();
            }
            store.try_append(&SessionRecord::Done {
                input_tokens: ledger.total_in,
                output_tokens: ledger.total_out,
                cache_read_tokens: Some(ledger.total_cache),
                ts: now_ms(),
            });
            crate::agent::persist::invalidate_cache(&store.path, &ctx.records_cache);
            ledger.roll(resolved_model);
            ledger.write_status(store, ctx, session_id, Some(last_context));
            emit_final_stats(&ledger, last_context, &budget);
            let _ = event_tx.send(AgentEvent::Done {
                stop_reason: "interrupted".into(),
                text_output: last_text,
                input_tokens: ledger.total_in,
                output_tokens: ledger.total_out,
            });
            return Ok(RunOutcome::Completed);
        }
        inject_steering(history, store, ctx, steering, event_tx);
    }

    // Safety cap hit: stop looping and report what we have so far rather than
    // running forever. Only reachable when config.max_rounds is set.
    let capped_text = if last_text.is_empty() {
        format!(
            "Stopped after {max_rounds} tool rounds without finishing. Try breaking the request into smaller parts."
        )
    } else {
        format!("{last_text}\n\n(Stopped after {max_rounds} tool rounds — this may be incomplete.)")
    };
    store.try_append(&SessionRecord::Done {
        input_tokens: ledger.total_in,
        output_tokens: ledger.total_out,
        cache_read_tokens: Some(ledger.total_cache),
        ts: now_ms(),
    });
    crate::agent::persist::invalidate_cache(&store.path, &ctx.records_cache);
    ledger.roll(config.model_for_mode(cur_mode.as_str()));
    ledger.write_status(store, ctx, session_id, Some(last_context));
    emit_final_stats(&ledger, last_context, &budget);
    let _ = event_tx.send(AgentEvent::Done {
        stop_reason: "max_rounds".into(),
        text_output: capped_text,
        input_tokens: ledger.total_in,
        output_tokens: ledger.total_out,
    });
    Ok(RunOutcome::Completed)
}

#[cfg(test)]
mod finish_line_tests {
    use super::*;
    use crate::agent::session::SteeringEntry;

    /// A workspace whose "test suite" is `test_cmd`, with a session store for
    /// the evidence to land in.
    fn workspace(name: &str, test_cmd: &str) -> (ToolContext, std::path::PathBuf) {
        let root = std::env::temp_dir().join(format!("cq-finish-{name}-{}", std::process::id()));
        std::fs::remove_dir_all(&root).ok();
        std::fs::create_dir_all(root.join(".claudinio/sessions")).unwrap();
        std::fs::write(
            root.join(".claudinio.json"),
            serde_json::json!({ "quality": { "test_cmd": test_cmd } }).to_string(),
        )
        .unwrap();
        let store = root.join(".claudinio/sessions/s.jsonl");
        std::fs::write(&store, "").unwrap();
        let mut ctx = crate::agent::tools::tests_support::ctx();
        ctx.workspace_root = Some(root.to_string_lossy().to_string());
        ctx.session_store_path = Some(store.to_string_lossy().to_string());
        (ctx, root)
    }

    fn recorded_runs(ctx: &ToolContext) -> usize {
        let store = ctx.session_store_path.as_deref().unwrap();
        crate::agent::persist::load_records(std::path::Path::new(store))
            .unwrap()
            .iter()
            .filter(|r| matches!(r, SessionRecord::QualityRun { .. }))
            .count()
    }

    #[tokio::test]
    async fn a_message_sent_while_the_checks_run_stops_them() {
        // The checks used to hold the user's message behind them for as long
        // as the suite took, then the run ended and the queue was dropped.
        let (ctx, root) = workspace("steer", "sleep 30");
        let steering = Arc::new(SteeringCtl::new());
        steering.push(SteeringEntry {
            text: "is main clean?".into(),
            attachments: Vec::new(),
        });

        let started = std::time::Instant::now();
        let outcome = run_finish_line_checks(&ctx, &steering).await;
        assert!(outcome.is_none(), "cut short, not a verdict");
        assert!(
            started.elapsed() < std::time::Duration::from_secs(10),
            "took {:?}",
            started.elapsed()
        );
        assert!(
            steering.has_pending(),
            "the message is still there to answer"
        );
        assert_eq!(recorded_runs(&ctx), 0, "an unfinished run is not evidence");
        std::fs::remove_dir_all(&root).ok();
    }

    #[tokio::test]
    async fn a_pause_stops_the_checks_too() {
        let (ctx, root) = workspace("pause", "sleep 30");
        let steering = Arc::new(SteeringCtl::new());
        steering.interrupt.store(true, Ordering::SeqCst);

        assert!(run_finish_line_checks(&ctx, &steering).await.is_none());
        assert_eq!(recorded_runs(&ctx), 0);
        std::fs::remove_dir_all(&root).ok();
    }

    #[tokio::test]
    async fn undisturbed_checks_run_to_a_verdict_and_are_recorded() {
        let (ctx, root) = workspace("quiet", "exit 0");
        let steering = Arc::new(SteeringCtl::new());

        let report = run_finish_line_checks(&ctx, &steering)
            .await
            .expect("not cut short")
            .expect("the harness ran");
        assert!(report.verdict.pass);
        assert_eq!(recorded_runs(&ctx), 1);
        std::fs::remove_dir_all(&root).ok();
    }
}

#[cfg(test)]
pub(crate) mod surface_tests {
    use super::*;
    use crate::agent::handoff::shed_line;
    use crate::agent::prompt::{api_tools, system_prompt};
    use crate::agent::provider::ProviderEntry;
    use crate::agent::provider::ToolDescription;
    use crate::agent::subagent::scripted_model::{Reply, spawn_recording};
    use crate::agent::surface::{COMPACT_TOOLS, LEAN_DROPPED, ToolSurface, prompt_for};
    use crate::agent::tools;

    const SKILLS: &str = "## SKILLS\n- deploy: ship the thing";
    const SPECS: &str = "## SPECIFICATION\n- Scenario: it works";

    /// A provider "stub" serving model "m" as the builder, with the window the
    /// user typed for it.
    pub(crate) fn config_for(base_url: &str, context_window: Option<u32>) -> AgentConfig {
        let mut config = AgentConfig {
            builder_model: "stub/m".into(),
            // Edits are approved without a person, so a run can finish.
            yolo_mode: true,
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
                model_output_limits: [("m".to_string(), 4_096u32)].into_iter().collect(),
                custom: true,
                custom_models: vec!["m".into()],
                model_context_limits: Default::default(),
                context_window,
            },
        );
        config
    }

    fn names(tools: &[ToolDescription]) -> Vec<&str> {
        tools.iter().map(|t| t.name.as_str()).collect()
    }

    fn standard(mode: SessionMode) -> String {
        system_prompt(
            Some("/ws"),
            Some(SKILLS),
            Some(SPECS),
            None,
            mode,
            PromptProfile::Standard,
            4,
            false,
        )
    }

    // A prompt that still steered toward a tool the model was not given would
    // cost a failed call every time the model believed it.
    #[test]
    fn a_lean_prompt_names_none_of_the_tools_lean_drops() {
        for mode in [SessionMode::Brain, SessionMode::Builder] {
            let full = standard(mode);
            assert!(
                LEAN_DROPPED.iter().any(|t| full.contains(t)) || full.contains("LSP"),
                "{mode:?}: the full prompt steers toward the LSP tools"
            );
            let lean = prompt_for(ToolSurface::Lean, full.clone());
            for tool in LEAN_DROPPED {
                assert!(!lean.contains(tool), "{mode:?} still names {tool}");
            }
            assert!(!lean.contains("LSP"), "{mode:?} still ranks LSP first");
            // Everything else is the same prompt.
            assert!(lean.contains("`semantic_search`") && lean.contains(SKILLS));
            assert!(lean.len() < full.len() && lean.len() > full.len() - 400);
        }
    }

    #[test]
    fn the_surface_is_set_per_model_so_brain_and_builder_can_differ() {
        let mut cfg = AgentConfig::default();
        cfg.tool_surface
            .insert(cfg.builder_model.clone(), "lean".into());

        let builder = api_tools(SessionMode::Builder, PromptProfile::Standard, &[], &cfg);
        let brain = api_tools(SessionMode::Brain, PromptProfile::Standard, &[], &cfg);
        for tool in LEAN_DROPPED {
            assert!(!names(&builder).contains(&tool), "builder is lean: {tool}");
            assert!(names(&brain).contains(&tool), "brain was not set: {tool}");
        }
        // Each mode adds two tools of its own; Builder also has `edit_file`,
        // which Brain never gets.
        assert_eq!(builder.len() + LEAN_DROPPED.len(), brain.len() + 1);

        let ctx = crate::agent::tools::tests_support::ctx();
        let prefix = |mode| {
            run_prefix(
                &cfg,
                &ctx,
                PromptProfile::Standard,
                mode,
                None,
                None,
                &[],
                false,
                &[],
            )
        };
        // symbol_lookup is deferred (Phase 2): offered means listed in the
        // catalog and loadable, not sent.
        let deferred =
            |mode| crate::agent::prompt::deferred_tools(mode, PromptProfile::Standard, &[], &cfg);
        let (profile, system, tools) = prefix(SessionMode::Builder);
        assert_eq!(profile, PromptProfile::Standard);
        assert!(!system.contains("symbol_lookup") && !names(&tools).contains(&"symbol_lookup"));
        assert!(!names(&deferred(SessionMode::Builder)).contains(&"symbol_lookup"));
        let (_, system, _) = prefix(SessionMode::Brain);
        assert!(system.contains("symbol_lookup"));
        assert!(names(&deferred(SessionMode::Brain)).contains(&"symbol_lookup"));
    }

    // With nothing set, nothing moves: the prefix every existing session
    // caches is the one it had.
    #[test]
    fn with_no_surface_set_the_prefix_is_what_it_was() {
        let cfg = AgentConfig::default();
        let ctx = ToolContext {
            workspace_root: Some("/ws".into()),
            ..crate::agent::tools::tests_support::ctx()
        };
        for mode in [SessionMode::Brain, SessionMode::Builder] {
            let (profile, system, tools) = run_prefix(
                &cfg,
                &ctx,
                PromptProfile::Standard,
                mode,
                Some(SKILLS),
                Some(SPECS),
                &[],
                false,
                &[],
            );
            assert_eq!(profile, PromptProfile::Standard);
            // Phase 2 appends the deferred catalog to the prompt and sends the
            // core of the surface plus `load_tools`; with no surface set the
            // surface itself is the full one.
            let catalog = system
                .strip_prefix(standard(mode).as_str())
                .expect("the prompt, then the catalog");
            assert!(catalog.starts_with("\n\n## DEFERRED TOOLS"));
            assert!(catalog.contains("- go_to_definition "));
            let mut core: Vec<&str> = Vec::new();
            let surface = api_tools(mode, PromptProfile::Standard, &[], &cfg);
            core.extend(
                names(&surface)
                    .into_iter()
                    .filter(|n| !crate::agent::deferred::is_deferred(n)),
            );
            core.push("load_tools");
            assert_eq!(names(&tools), core);
            assert!(names(&surface).contains(&"go_to_definition"));
        }
    }

    #[test]
    fn the_compact_prefix_is_one_prompt_and_ten_tools() {
        let cfg = config_for("http://127.0.0.1:1", Some(32_768));
        let mcp = vec![tools::ToolDef {
            name: "mcp__db__query".into(),
            description: "Run a query".into(),
            input_schema: serde_json::json!({"type": "object"}),
        }];
        let ctx = ToolContext {
            workspace_root: Some("/ws".into()),
            ..crate::agent::tools::tests_support::ctx()
        };
        let (profile, system, tools) = run_prefix(
            &cfg,
            &ctx,
            PromptProfile::Standard,
            SessionMode::Builder,
            Some(SKILLS),
            Some(SPECS),
            &mcp,
            false,
            &[],
        );
        assert_eq!(profile, PromptProfile::Compact);
        let mut offered = names(&tools);
        offered.sort_unstable();
        let mut expected = COMPACT_TOOLS.to_vec();
        expected.sort_unstable();
        assert_eq!(offered, expected);

        assert!(system.starts_with(crate::agent::surface::COMPACT_PROMPT));
        assert!(system.contains("Project workspace root: /ws."));
        for absent in [SKILLS, SPECS, "CURRENT MODE", "spawn_agents", "golden"] {
            assert!(!system.contains(absent), "{absent}");
        }
        assert!(!system.contains("{max_parallel}") && !system.contains("{plans_subdir}"));

        // The point of the profile: what a 32k model is sent before the first
        // word of the conversation. 12% of the window, where the Standard
        // prompt and tools — before any skill or MCP schema — took 30%.
        let compact = estimate_tokens(&[], &system, &tools);
        let full_tools = api_tools(SessionMode::Builder, PromptProfile::Standard, &mcp, &cfg);
        let full = estimate_tokens(&[], &standard(SessionMode::Builder), &full_tools);
        assert!(compact < 4_000, "compact prefix: {compact} tokens");
        assert!(full > 2 * compact, "standard {full} vs compact {compact}");
    }

    // The same session in Brain keeps the Standard profile: a toggle must hand
    // the model Brain's read-only protocol, not Compact's edit tools.
    #[test]
    fn switching_a_compact_session_to_brain_restores_the_standard_prefix() {
        let mut cfg = config_for("http://127.0.0.1:1", Some(32_768));
        cfg.brain_model = "stub/m".into();
        let ctx = crate::agent::tools::tests_support::ctx();
        let (profile, system, tools) = run_prefix(
            &cfg,
            &ctx,
            PromptProfile::Standard,
            SessionMode::Brain,
            None,
            None,
            &[],
            false,
            &[],
        );
        assert_eq!(profile, PromptProfile::Standard);
        assert!(system.contains("CURRENT MODE: BRAIN"));
        assert!(!names(&tools).contains(&"edit_file"));
        assert!(names(&tools).contains(&"write_plan"));
    }

    #[test]
    fn a_compact_session_is_still_a_chat_session() {
        // Its hooks fire and its long conversations hand off, like Standard's.
        assert!(PromptProfile::Compact.is_chat() && PromptProfile::Standard.is_chat());
        assert!(!PromptProfile::GitSync.is_chat());
        let cfg = config_for("http://127.0.0.1:1", Some(32_768));
        let budget = ContextBudget::for_model(&cfg, &cfg.builder_model, 3_600);
        assert_eq!(shed_line(&budget, PromptProfile::Compact), budget.soft);
    }

    struct Run {
        outcome: Result<RunOutcome, String>,
        file: std::path::PathBuf,
        store: SessionStore,
        bodies: Vec<String>,
        root: std::path::PathBuf,
        /// Every event the run streamed, in the order it was sent.
        events: Vec<AgentEvent>,
    }

    /// One user message against a model that reads a file, edits it, and says
    /// it is done.
    async fn read_edit_answer(tag: &str, context_window: Option<u32>) -> Run {
        run_script(tag, context_window, None, |path| {
            vec![
                Reply::Tool {
                    name: "read_file",
                    input: serde_json::json!({ "path": path }),
                },
                Reply::Tool {
                    name: "edit_file",
                    input: serde_json::json!({
                        "path": path, "old_string": "hello", "new_string": "goodbye",
                    }),
                },
                Reply::Text("changed the greeting"),
            ]
        })
        .await
    }

    /// One user message against a model that follows `script` (built from the
    /// path of `greet.txt`, the one file in its workspace). With `answer`,
    /// approvals are switched on and every prompt gets that answer; without
    /// it they are off (`yolo_mode`).
    async fn run_script(
        tag: &str,
        context_window: Option<u32>,
        answer: Option<bool>,
        script: impl FnOnce(&str) -> Vec<Reply>,
    ) -> Run {
        let steering = Arc::new(SteeringCtl::new());
        run_script_steered(tag, context_window, answer, script, steering, |_, _| {}).await
    }

    /// `run_script`, with the session in `mode` from its first round (the
    /// brain model is the same stub as the builder).
    async fn run_script_in(
        mode: SessionMode,
        tag: &str,
        answer: Option<bool>,
        script: impl FnOnce(&str) -> Vec<Reply>,
    ) -> Run {
        let steering = Arc::new(SteeringCtl::new());
        run_script_with(mode, tag, None, answer, script, steering, |_, _| {}).await
    }

    /// `run_script`, with the run's steering in the test's hands and
    /// `on_event` called with each event (and that steering) as it is sent.
    async fn run_script_steered(
        tag: &str,
        context_window: Option<u32>,
        answer: Option<bool>,
        script: impl FnOnce(&str) -> Vec<Reply>,
        steering: Arc<SteeringCtl>,
        on_event: impl Fn(&AgentEvent, &SteeringCtl) + Send + Sync + 'static,
    ) -> Run {
        run_script_with(
            SessionMode::Builder,
            tag,
            context_window,
            answer,
            script,
            steering,
            on_event,
        )
        .await
    }

    async fn run_script_with(
        mode: SessionMode,
        tag: &str,
        context_window: Option<u32>,
        answer: Option<bool>,
        script: impl FnOnce(&str) -> Vec<Reply>,
        steering: Arc<SteeringCtl>,
        on_event: impl Fn(&AgentEvent, &SteeringCtl) + Send + Sync + 'static,
    ) -> Run {
        let root = std::env::temp_dir().join(format!(
            "claudinio-surface-{tag}-{}-{}",
            std::process::id(),
            now_ms()
        ));
        std::fs::create_dir_all(&root).unwrap();
        let file = root.join("greet.txt");
        std::fs::write(&file, "hello world\n").unwrap();
        let path = file.to_string_lossy().to_string();
        let store = SessionStore {
            path: root.join("session.jsonl"),
        };
        std::fs::write(&store.path, "").unwrap();

        let (base, _, bodies) = spawn_recording(script(&path)).await;
        let mut config = config_for(&base, context_window);
        if mode == SessionMode::Brain {
            config.brain_model = config.builder_model.clone();
        }
        config.yolo_mode = answer.is_none();
        let ctx = ToolContext {
            workspace_root: Some(root.to_string_lossy().to_string()),
            session_store_path: Some(store.path.to_string_lossy().to_string()),
            ..crate::agent::tools::tests_support::ctx()
        };
        let seen = Arc::new(std::sync::Mutex::new(Vec::new()));
        let events: Channel<AgentEvent> = {
            let (seen, steering) = (seen.clone(), steering.clone());
            Channel::new(
                move |body: tauri::ipc::InvokeResponseBody| -> tauri::Result<()> {
                    if let tauri::ipc::InvokeResponseBody::Json(json) = &body
                        && let Ok(event) = serde_json::from_str::<AgentEvent>(json)
                    {
                        on_event(&event, &steering);
                        seen.lock().unwrap().push(event);
                    }
                    Ok(())
                },
            )
        };
        let approvals = ApprovalMap::default();
        // The person at the other end of every approval prompt.
        let user = answer.map(|answer| {
            let approvals = approvals.clone();
            tokio::spawn(async move {
                loop {
                    let pending: Vec<_> = approvals.lock().await.drain().collect();
                    for (_, reply) in pending {
                        let _ = reply.send(answer);
                    }
                    tokio::time::sleep(std::time::Duration::from_millis(10)).await;
                }
            })
        });
        let mut history = Vec::new();
        let outcome = run_workflow(
            &config,
            &mut history,
            "say goodbye instead".into(),
            Vec::new(),
            &events,
            &approvals,
            &AnswerMap::default(),
            "surface-test",
            &ctx,
            &store,
            &steering,
            &Arc::new(ModeCtl::new(mode, ModeOrigin::Human)),
        )
        .await;
        if let Some(user) = user {
            user.abort();
        }
        let bodies = bodies.lock().unwrap().clone();
        let events = seen.lock().unwrap().clone();
        Run {
            outcome,
            file,
            store,
            bodies,
            root,
            events,
        }
    }

    /// Where in `run.events` the first `ToolCall` (`call`) or the `ToolResult`
    /// of `tool_id` was sent. A call is answered by exactly one `ToolResult`;
    /// it may be announced twice, since an edit sends a second `ToolCall`
    /// carrying its proposal.
    fn event_at(run: &Run, call: bool, tool_id: &str) -> usize {
        let at: Vec<usize> = run
            .events
            .iter()
            .enumerate()
            .filter(|(_, e)| match e {
                AgentEvent::ToolCall { tool_id: id, .. } => call && id == tool_id,
                AgentEvent::ToolResult { tool_id: id, .. } => !call && id == tool_id,
                _ => false,
            })
            .map(|(i, _)| i)
            .collect();
        assert!(!at.is_empty(), "no event for {tool_id}");
        assert!(call || at.len() == 1, "{} results for {tool_id}", at.len());
        at[0]
    }

    /// The `tool_use_id`s of the tool results the request `body` ends with.
    fn answered_ids(body: &str) -> Vec<String> {
        let body: Value = serde_json::from_str(body).unwrap();
        let last = body["messages"].as_array().unwrap().last().unwrap().clone();
        last["content"]
            .as_array()
            .unwrap()
            .iter()
            .filter_map(|b| b["tool_use_id"].as_str().map(str::to_string))
            .collect()
    }

    // Read-only calls next to each other are taken as one parallel run: all
    // of them are announced before any answers. A call that is not one of
    // them waits for the run before it and holds back the run after it, and
    // the model gets every result back in the order it asked. That the calls
    // of a run overlap in time is measured in `tool_batch`'s tests, with an
    // executor slow enough to see it; real reads of a tiny file are not.
    #[tokio::test]
    async fn a_round_announces_its_reads_together_and_runs_everything_else_in_order() {
        let run = run_script("batch", None, None, |path| {
            let root = std::path::Path::new(path)
                .parent()
                .unwrap()
                .to_string_lossy()
                .to_string();
            vec![
                Reply::Tools(vec![
                    ("read_file", serde_json::json!({ "path": path })),
                    ("list_dir", serde_json::json!({ "path": root })),
                    ("bash", serde_json::json!({ "command": "echo between" })),
                    (
                        "grep",
                        serde_json::json!({ "pattern": "hello", "path": root }),
                    ),
                    ("read_file", serde_json::json!({ "path": path })),
                ]),
                Reply::Text("done"),
            ]
        })
        .await;
        assert!(
            matches!(run.outcome, Ok(RunOutcome::Completed)),
            "{:?}",
            run.outcome.err()
        );
        let (call, result) = (
            |id: &str| event_at(&run, true, id),
            |id: &str| event_at(&run, false, id),
        );
        // Both reads of the first run were announced before either answered.
        assert!(call("tu_0_1") < result("tu_0_0"), "{:#?}", run.events);
        // The shell command started only once that run was over…
        assert!(result("tu_0_0") < call("tu_0_2") && result("tu_0_1") < call("tu_0_2"));
        // …and the second run only once the command was.
        assert!(result("tu_0_2") < call("tu_0_3"));
        assert!(call("tu_0_4") < result("tu_0_3"));
        assert_eq!(
            answered_ids(&run.bodies[1]),
            ["tu_0_0", "tu_0_1", "tu_0_2", "tu_0_3", "tu_0_4"]
        );
        assert!(
            run.bodies[1].contains("between"),
            "the command's output came back"
        );
        std::fs::remove_dir_all(&run.root).ok();
    }

    // Stop pressed while a run of reads is under way: whatever had started
    // finishes and keeps its result, the rest are reported as not run, and the
    // model's turn is answered call for call, in order, once each.
    //
    // Six reads, four at a time (`MAX_CONCURRENT_TOOLS`), and the pause comes
    // with the first result: the four already started finish and keep what
    // they read, the two still waiting for a slot never start. The test
    // runtime is single-threaded, so the four jobs are all started before any
    // of them can report — the split is the same on every run.
    #[tokio::test]
    async fn a_pause_during_parallel_reads_keeps_what_ran_and_answers_the_rest_as_not_run() {
        assert_eq!(crate::agent::tool_batch::MAX_CONCURRENT_TOOLS, 4);
        let steering = Arc::new(SteeringCtl::new());
        let run = run_script_steered(
            "batch-pause",
            None,
            None,
            |path| {
                let read = || ("read_file", serde_json::json!({ "path": path }));
                let grep = || {
                    (
                        "grep",
                        serde_json::json!({ "pattern": "hello", "path": path }),
                    )
                };
                vec![
                    Reply::Tools(vec![read(), grep(), read(), grep(), read(), grep()]),
                    Reply::Text("should not be asked for"),
                ]
            },
            steering,
            |event, steering| {
                if matches!(event, AgentEvent::ToolResult { .. }) {
                    steering.interrupt.store(true, Ordering::SeqCst);
                }
            },
        )
        .await;
        assert!(
            matches!(run.outcome, Ok(RunOutcome::Completed)),
            "{:?}",
            run.outcome.err()
        );
        assert_eq!(
            run.bodies.len(),
            1,
            "the run paused instead of asking again"
        );
        let turns: Vec<Message> = crate::agent::persist::load_records(&run.store.path)
            .unwrap()
            .into_iter()
            .filter_map(|r| match r {
                SessionRecord::Turn { message, .. } => Some(message),
                _ => None,
            })
            .collect();
        let [.., asked, answered] = &turns[..] else {
            panic!("no tool round recorded: {turns:?}");
        };
        let all: Vec<String> = (0..6).map(|i| format!("tu_0_{i}")).collect();
        let asked_ids: Vec<String> = asked
            .content
            .iter()
            .filter_map(|b| match b {
                ContentBlock::ToolUse { id, .. } => Some(id.clone()),
                _ => None,
            })
            .collect();
        assert_eq!(asked_ids, all);
        let answers: Vec<(String, String)> = answered
            .content
            .iter()
            .filter_map(|b| match b {
                ContentBlock::ToolResult {
                    tool_use_id,
                    content,
                    ..
                } => Some((tool_use_id.clone(), content.as_text().into_owned())),
                _ => None,
            })
            .collect();
        let answered_ids: Vec<&String> = answers.iter().map(|(id, _)| id).collect();
        assert_eq!(
            answered_ids,
            all.iter().collect::<Vec<_>>(),
            "once each, in order"
        );
        for (id, text) in &answers[..4] {
            assert!(
                text.contains("hello world"),
                "{id} had started, so it keeps what it read: {text}"
            );
        }
        for (id, text) in &answers[4..] {
            assert_eq!(
                text,
                crate::agent::tool_batch::INTERRUPTED,
                "{id} was still waiting for a slot"
            );
        }
        // One ToolResult event per call: a kept call is not answered again.
        for id in &all {
            let n = run
                .events
                .iter()
                .filter(|e| matches!(e, AgentEvent::ToolResult { tool_id, .. } if tool_id == id))
                .count();
            assert_eq!(n, 1, "{id}: {n} results");
        }
        std::fs::remove_dir_all(&run.root).ok();
    }

    // A read a pause let finish is carried like any other: a repeat of what
    // the conversation already holds goes in as the pointer, not a second
    // copy of the file.
    #[tokio::test]
    async fn a_read_kept_after_a_pause_still_becomes_the_repeated_read_pointer() {
        let steering = Arc::new(SteeringCtl::new());
        let run = run_script_steered(
            "batch-pause-repeat",
            None,
            None,
            |path| {
                // Long enough to be worth a pointer.
                std::fs::write(path, "a line worth carrying once\n".repeat(200)).unwrap();
                let root = std::path::Path::new(path)
                    .parent()
                    .unwrap()
                    .to_string_lossy()
                    .to_string();
                vec![
                    Reply::Tools(vec![("read_file", serde_json::json!({ "path": path }))]),
                    Reply::Tools(vec![
                        ("read_file", serde_json::json!({ "path": path })),
                        ("list_dir", serde_json::json!({ "path": root })),
                    ]),
                    Reply::Text("should not be asked for"),
                ]
            },
            steering,
            |event, steering| {
                if matches!(event, AgentEvent::ToolResult { tool_id, .. } if tool_id == "tu_1_0") {
                    steering.interrupt.store(true, Ordering::SeqCst);
                }
            },
        )
        .await;
        assert!(
            matches!(run.outcome, Ok(RunOutcome::Completed)),
            "{:?}",
            run.outcome.err()
        );
        let answered = crate::agent::persist::load_records(&run.store.path)
            .unwrap()
            .into_iter()
            .rev()
            .find_map(|r| match r {
                SessionRecord::Turn { message, .. } if message.role == "user" => Some(message),
                _ => None,
            })
            .expect("the paused round was recorded");
        let repeat = answered
            .content
            .iter()
            .find_map(|b| match b {
                ContentBlock::ToolResult {
                    tool_use_id,
                    content,
                    ..
                } if tool_use_id == "tu_1_0" => Some(content.as_text().into_owned()),
                _ => None,
            })
            .expect("the repeated read was answered");
        assert!(
            repeat.starts_with(crate::agent::prune::UNCHANGED_MARK),
            "{}",
            &repeat[..repeat.len().min(200)]
        );
        std::fs::remove_dir_all(&run.root).ok();
    }

    // read-before-edit holds inside one round: the reads finish (and are on
    // record) before the edit that depends on them is tried.
    #[tokio::test]
    async fn an_edit_after_parallel_reads_in_the_same_round_sees_them() {
        let run = run_script("batch-edit", Some(32_768), None, |path| {
            let root = std::path::Path::new(path)
                .parent()
                .unwrap()
                .to_string_lossy()
                .to_string();
            vec![
                Reply::Tools(vec![
                    (
                        "read_file",
                        serde_json::json!({ "path": path, "start_line": 1, "end_line": 1 }),
                    ),
                    ("list_dir", serde_json::json!({ "path": root })),
                    (
                        "edit_file",
                        serde_json::json!({
                            "path": path, "old_string": "hello", "new_string": "goodbye",
                        }),
                    ),
                ]),
                Reply::Text("changed the greeting"),
            ]
        })
        .await;
        assert!(
            matches!(run.outcome, Ok(RunOutcome::Completed)),
            "{:?}",
            run.outcome.err()
        );
        assert_eq!(
            std::fs::read_to_string(&run.file).unwrap(),
            "goodbye world\n"
        );
        assert!(event_at(&run, true, "tu_0_1") < event_at(&run, false, "tu_0_0"));
        assert!(event_at(&run, false, "tu_0_1") < event_at(&run, true, "tu_0_2"));
        assert_eq!(answered_ids(&run.bodies[1]), ["tu_0_0", "tu_0_1", "tu_0_2"]);
        std::fs::remove_dir_all(&run.root).ok();
    }

    // In every other profile a shell command that writes a file never reaches
    // the tool, so the allowlist could afford to be a list of prefixes. In
    // Compact it does reach it — and `echo …` being harmless to run is no
    // reason for `echo … > file` to be written without anyone being asked.
    #[tokio::test]
    async fn a_compact_session_asks_before_a_shell_command_writes_a_file() {
        let write = |path: &str| {
            vec![
                Reply::Tool {
                    name: "bash",
                    input: serde_json::json!({ "command": format!("echo goodbye > {path}") }),
                },
                Reply::Text("done"),
            ]
        };

        let refused = run_script("bash-no", Some(32_768), Some(false), write).await;
        assert!(matches!(refused.outcome, Ok(RunOutcome::Completed)));
        assert_eq!(
            std::fs::read_to_string(&refused.file).unwrap(),
            "hello world\n"
        );
        assert!(
            refused.bodies[1].contains("rejected by user"),
            "the model is told"
        );

        let approved = run_script("bash-yes", Some(32_768), Some(true), write).await;
        assert_eq!(
            std::fs::read_to_string(&approved.file).unwrap(),
            "goodbye\n"
        );

        // A command that only reads is still not worth a prompt: with every
        // prompt answered "no", it runs.
        let reads = run_script("bash-read", Some(32_768), Some(false), |path| {
            vec![
                Reply::Tool {
                    name: "bash",
                    input: serde_json::json!({ "command": format!("cat {path}") }),
                },
                Reply::Text("done"),
            ]
        })
        .await;
        assert!(reads.bodies[1].contains("hello world"));
        for run in [refused, approved, reads] {
            std::fs::remove_dir_all(&run.root).ok();
        }
    }

    // What the model is not offered it cannot run, whatever it asks for.
    #[tokio::test]
    async fn a_compact_session_runs_only_the_tools_it_was_offered() {
        let run = run_script("offered", Some(32_768), None, |_| {
            vec![
                Reply::Tool {
                    name: "spawn_agents",
                    input: serde_json::json!({
                        "agents": [{"name": "a", "goal": "look around", "mode": "explore"}],
                    }),
                },
                Reply::Text("done"),
            ]
        })
        .await;
        assert!(matches!(run.outcome, Ok(RunOutcome::Completed)));
        // Two requests: the call, and the answer to its refusal. A subagent
        // would have made more against the same stub.
        assert_eq!(run.bodies.len(), 2);
        assert!(run.bodies[1].contains("spawn_agents is not available in this session"));
        std::fs::remove_dir_all(&run.root).ok();
    }

    fn run_config(store: &SessionStore) -> (String, u64) {
        crate::agent::persist::load_records(&store.path)
            .unwrap()
            .into_iter()
            .find_map(|r| match r {
                SessionRecord::RunConfig {
                    profile, prefix, ..
                } => Some((
                    profile,
                    prefix.system + prefix.skills + prefix.specs + prefix.tools + prefix.mcp,
                )),
                _ => None,
            })
            .expect("a run_config record")
    }

    /// The whole reason the profile exists, end to end: a 32k model is sent a
    /// request it can hold, and gets the change made without a second agent.
    #[tokio::test]
    async fn on_a_small_window_the_session_edits_the_file_itself() {
        let run = read_edit_answer("compact", Some(32_768)).await;
        assert!(
            matches!(run.outcome, Ok(RunOutcome::Completed)),
            "{:?}",
            run.outcome.err()
        );
        assert_eq!(
            std::fs::read_to_string(&run.file).unwrap(),
            "goodbye world\n"
        );

        let (profile, prefix) = run_config(&run.store);
        assert_eq!(profile, "compact");
        assert!(prefix < 4_000, "prefix: {prefix} tokens");

        let first: serde_json::Value = serde_json::from_str(&run.bodies[0]).unwrap();
        let sent: Vec<&str> = first["tools"]
            .as_array()
            .unwrap()
            .iter()
            .filter_map(|t| t["name"].as_str())
            .collect();
        assert_eq!(sent.len(), COMPACT_TOOLS.len(), "{sent:?}");
        assert!(sent.contains(&"edit_file") && !sent.contains(&"spawn_agents"));
        // Every request of the run stays well inside the window.
        let largest = run.bodies.iter().map(String::len).max().unwrap() / 3;
        assert!(largest < 8_000, "largest request: {largest} tokens");
        std::fs::remove_dir_all(&run.root).ok();
    }

    /// The counterpart on a window that fits the Standard profile (decision
    /// D1, lean-harness Phase 3): the Builder session is offered `edit_file`
    /// beside `spawn_agents` and makes the change itself, without a second
    /// prefix and a second context for a subagent.
    #[tokio::test]
    async fn on_a_large_window_the_builder_session_edits_the_file_itself() {
        let run = read_edit_answer("standard", None).await;
        assert!(
            matches!(run.outcome, Ok(RunOutcome::Completed)),
            "{:?}",
            run.outcome.err()
        );
        assert_eq!(
            std::fs::read_to_string(&run.file).unwrap(),
            "goodbye world\n"
        );
        assert_eq!(run_config(&run.store).0, "standard");

        let first: serde_json::Value = serde_json::from_str(&run.bodies[0]).unwrap();
        let sent: Vec<&str> = first["tools"]
            .as_array()
            .unwrap()
            .iter()
            .filter_map(|t| t["name"].as_str())
            .collect();
        assert!(sent.contains(&"spawn_agents") && sent.contains(&"edit_file"));
        std::fs::remove_dir_all(&run.root).ok();
    }

    /// An edit the Standard Builder makes is asked about like Compact's: the
    /// person sees the proposal (the diff) and a "no" leaves the file alone.
    #[tokio::test]
    async fn a_standard_builder_edit_is_proposed_as_a_diff_and_waits_for_approval() {
        let script = |path: &str| {
            vec![
                Reply::Tool {
                    name: "read_file",
                    input: serde_json::json!({ "path": path }),
                },
                Reply::Tool {
                    name: "edit_file",
                    input: serde_json::json!({
                        "path": path, "old_string": "hello", "new_string": "goodbye",
                    }),
                },
                Reply::Text("done"),
            ]
        };
        let refused = run_script("std-edit-no", None, Some(false), script).await;
        assert!(matches!(refused.outcome, Ok(RunOutcome::Completed)));
        assert_eq!(
            std::fs::read_to_string(&refused.file).unwrap(),
            "hello world\n"
        );
        assert!(
            refused.events.iter().any(|e| matches!(
                e,
                AgentEvent::ToolCall { tool_name, permission, edit_proposal: Some(_), .. }
                    if tool_name == "edit_file" && permission == "requires_approval"
            )),
            "the edit is shown as a proposal awaiting approval"
        );
        assert!(refused.bodies[2].contains("rejected"), "the model is told");

        let approved = run_script("std-edit-yes", None, Some(true), script).await;
        assert!(matches!(approved.outcome, Ok(RunOutcome::Completed)));
        assert_eq!(
            std::fs::read_to_string(&approved.file).unwrap(),
            "goodbye world\n"
        );
        for run in [refused, approved] {
            std::fs::remove_dir_all(&run.root).ok();
        }
    }

    /// A shell command that writes a file is an edit in the Standard Builder
    /// too: asked about even when its prefix (`echo `) is allowlisted, never
    /// refused outright as it was while the session had to delegate edits.
    #[tokio::test]
    async fn a_standard_builder_asks_before_a_shell_command_writes_a_file() {
        let write = |path: &str| {
            vec![
                Reply::Tool {
                    name: "bash",
                    input: serde_json::json!({ "command": format!("echo goodbye > {path}") }),
                },
                Reply::Text("done"),
            ]
        };

        let refused = run_script("std-bash-no", None, Some(false), write).await;
        assert!(matches!(refused.outcome, Ok(RunOutcome::Completed)));
        assert_eq!(
            std::fs::read_to_string(&refused.file).unwrap(),
            "hello world\n"
        );
        assert!(
            refused.bodies[1].contains("rejected by user"),
            "the person was asked and said no"
        );

        let approved = run_script("std-bash-yes", None, Some(true), write).await;
        assert_eq!(
            std::fs::read_to_string(&approved.file).unwrap(),
            "goodbye\n"
        );

        // A command that only reads is still not worth a prompt.
        let reads = run_script("std-bash-read", None, Some(false), |path| {
            vec![
                Reply::Tool {
                    name: "bash",
                    input: serde_json::json!({ "command": format!("cat {path}") }),
                },
                Reply::Text("done"),
            ]
        })
        .await;
        assert!(reads.bodies[1].contains("hello world"));
        for run in [refused, approved, reads] {
            std::fs::remove_dir_all(&run.root).ok();
        }
    }

    /// Brain stays strictly read-only whatever Builder may now do: it is not
    /// offered `edit_file`, a call to it is refused, and a shell command that
    /// writes a file is refused even when the person would have said yes.
    #[tokio::test]
    async fn a_brain_session_refuses_every_write_even_with_approval() {
        let run = run_script_in(SessionMode::Brain, "brain-writes", Some(true), |path| {
            vec![
                Reply::Tool {
                    name: "read_file",
                    input: serde_json::json!({ "path": path }),
                },
                Reply::Tool {
                    name: "edit_file",
                    input: serde_json::json!({
                        "path": path, "old_string": "hello", "new_string": "goodbye",
                    }),
                },
                Reply::Tool {
                    name: "bash",
                    input: serde_json::json!({ "command": format!("echo goodbye > {path}") }),
                },
                Reply::Text("done"),
            ]
        })
        .await;
        assert!(
            matches!(run.outcome, Ok(RunOutcome::Completed)),
            "{:?}",
            run.outcome.err()
        );
        assert_eq!(std::fs::read_to_string(&run.file).unwrap(), "hello world\n");
        let first: serde_json::Value = serde_json::from_str(&run.bodies[0]).unwrap();
        assert!(first["system"].to_string().contains("CURRENT MODE: BRAIN"));
        let sent: Vec<&str> = first["tools"]
            .as_array()
            .unwrap()
            .iter()
            .filter_map(|t| t["name"].as_str())
            .collect();
        assert!(!sent.contains(&"edit_file"), "{sent:?}");
        assert!(run.bodies[2].contains("not available in Brain mode"));
        assert!(run.bodies[3].contains("In Brain mode you cannot mutate"));
        std::fs::remove_dir_all(&run.root).ok();
    }
}

/// What a run does at its context lines when there is nothing to shed.
#[cfg(test)]
mod context_line_tests {
    use super::surface_tests::config_for;
    use super::*;
    use crate::agent::subagent::scripted_model::{Reply, spawn_recording};
    use crate::agent::tool_exec::{tool_result_block, uncapped};

    fn store(tag: &str) -> SessionStore {
        let path = std::env::temp_dir().join(format!(
            "claudinio_lines_{tag}_{}_{}.jsonl",
            std::process::id(),
            now_ms()
        ));
        std::fs::write(&path, "").unwrap();
        SessionStore { path }
    }

    /// One opening message of `message_chars` against a model that just answers.
    async fn open_with(
        tag: &str,
        window: u32,
        message_chars: usize,
    ) -> (Result<RunOutcome, String>, usize, Vec<SessionRecord>) {
        let (base, _, bodies) = spawn_recording(vec![Reply::Text("on it")]).await;
        let config = config_for(&base, Some(window));
        let store = store(tag);
        let ctx = ToolContext {
            session_store_path: Some(store.path.to_string_lossy().to_string()),
            ..crate::agent::tools::tests_support::ctx()
        };
        let events: Channel<AgentEvent> = Channel::new(|_| Ok(()));
        let mut history = Vec::new();
        let outcome = run_workflow(
            &config,
            &mut history,
            "x ".repeat(message_chars / 2),
            Vec::new(),
            &events,
            &ApprovalMap::default(),
            &AnswerMap::default(),
            "lines-test",
            &ctx,
            &store,
            &Arc::new(SteeringCtl::new()),
            &Arc::new(ModeCtl::new(SessionMode::Builder, ModeOrigin::Human)),
        )
        .await;
        let requests = bodies.lock().unwrap().len();
        let records = crate::agent::persist::load_records(&store.path).unwrap();
        let _ = std::fs::remove_file(&store.path);
        (outcome, requests, records)
    }

    /// The lines a Compact run on a `window`-sized stub model is held to.
    fn lines(window: u32) -> (ContextBudget, u64) {
        let config = config_for("http://127.0.0.1:1", Some(window));
        let ctx = crate::agent::tools::tests_support::ctx();
        let (_, system, tools) = run_prefix(
            &config,
            &ctx,
            PromptProfile::Standard,
            SessionMode::Builder,
            None,
            None,
            &[],
            false,
            &[],
        );
        let prefix = estimate_tokens(&[], &system, &tools);
        (
            ContextBudget::for_model(&config, &config.builder_model, prefix),
            prefix,
        )
    }

    #[test]
    fn a_round_of_results_is_held_to_its_share_together() {
        let text = |b: &ContentBlock| match b {
            ContentBlock::ToolResult { content, .. } => content.as_text().into_owned(),
            _ => panic!("a tool result"),
        };
        let round = |sizes: &[usize]| -> Vec<ContentBlock> {
            sizes
                .iter()
                .enumerate()
                .map(|(i, n)| ContentBlock::tool_result(format!("t{i}"), "x".repeat(*n)))
                .collect()
        };

        // Under the cap, or where there is none: untouched.
        let mut fits = round(&[10_000, 10_000]);
        fit_round(&mut fits, &[], 30_000);
        assert!(fits.iter().all(|b| text(b).len() == 10_000));
        let mut unbounded = round(&[24_000; 6]);
        fit_round(&mut unbounded, &[], usize::MAX);
        assert!(unbounded.iter().all(|b| text(b).len() == 24_000));

        // Over it: the small result keeps all of itself, and the room it does
        // not use goes to the others — (30_000 - 400) / 3 each.
        let mut over = round(&[15_000, 15_000, 15_000, 400]);
        fit_round(&mut over, &[], 30_000);
        for b in &over[..3] {
            assert!(text(b).starts_with(&"x".repeat(9_866)));
            assert!(text(b).ends_with("...(truncated, 15000 chars total)"));
        }
        assert_eq!(text(&over[3]).len(), 400);
        let bodies: usize = over.iter().map(|b| uncapped(&text(b)).0.len()).sum();
        assert!(bodies <= 30_000, "{bodies}");
    }

    // The cap per result runs first and leaves its note. A round of two such
    // results is exactly the round's room plus two notes — not "over" — and a
    // result cut twice must still say how large the file was, not how large
    // the first cut left it.
    #[test]
    fn a_result_cut_twice_still_reports_the_size_it_really_had() {
        let text = |b: &ContentBlock| match b {
            ContentBlock::ToolResult { content, .. } => content.as_text().into_owned(),
            _ => panic!("a tool result"),
        };
        let capped = |id: &str, total: usize| tool_result_block(id, &"x".repeat(total), 15_000);

        let mut two = vec![capped("a", 60_000), capped("b", 48_213)];
        let before: Vec<String> = two.iter().map(&text).collect();
        fit_round(&mut two, &[], 30_000);
        assert_eq!(two.iter().map(&text).collect::<Vec<_>>(), before);

        let mut four = vec![
            capped("a", 60_000),
            capped("b", 48_213),
            capped("c", 20_000),
            capped("d", 16_000),
        ];
        fit_round(&mut four, &[], 30_000);
        for (b, total) in four.iter().zip([60_000, 48_213, 20_000, 16_000]) {
            let t = text(b);
            assert!(t.starts_with(&"x".repeat(7_500)) && !t.starts_with(&"x".repeat(7_501)));
            assert!(
                t.ends_with(&format!("...(truncated, {total} chars total)")),
                "{}",
                &t[7_490..]
            );
        }
    }

    // What the user answered is not a file that can be read again.
    #[test]
    fn a_round_never_cuts_what_the_user_answered() {
        let text = |b: &ContentBlock| match b {
            ContentBlock::ToolResult { content, .. } => content.as_text().into_owned(),
            _ => panic!("a tool result"),
        };
        let calls = vec![
            ContentBlock::tool_use("ask", "ask_user", serde_json::json!({})),
            ContentBlock::tool_use("r1", "read_file", serde_json::json!({})),
            ContentBlock::tool_use("r2", "read_file", serde_json::json!({})),
        ];
        let answer = format!(
            "Question: which?\nAnswer: {}",
            "the second one. ".repeat(90)
        );
        let mut round = vec![
            ContentBlock::tool_result("ask", answer.clone()),
            ContentBlock::tool_result("r1", "x".repeat(2_000)),
            ContentBlock::tool_result("r2", "y".repeat(2_000)),
        ];
        fit_round(&mut round, &calls, 3_864);
        assert_eq!(text(&round[0]), answer);
        // The reads share what the answer left.
        let room = (3_864 - answer.len()) / 2;
        assert!(text(&round[1]).starts_with(&"x".repeat(room)));
        assert!(text(&round[1]).ends_with("...(truncated, 2000 chars total)"));
        assert!(uncapped(&text(&round[2])).0.len() <= room);
    }

    fn rewrites(records: &[SessionRecord]) -> usize {
        records
            .iter()
            .filter(|r| {
                matches!(
                    r,
                    SessionRecord::Compacted { .. }
                        | SessionRecord::Pruned { .. }
                        | SessionRecord::HandoffTo { .. }
                )
            })
            .count()
    }

    // The prompt and tools alone are more than this model can be sent. That
    // used to be found out after a handoff request and a summarizing run, the
    // second of which replaced the user's message with a summary of it.
    #[tokio::test]
    async fn a_prefix_the_window_cannot_hold_is_refused_before_anything_is_tried() {
        // 2,048 since Phase 2's description diet: the Compact prefix now fits
        // the 4,096 window this test used to overflow.
        let (budget, prefix) = lines(2_048);
        assert!(budget.prefix_overflows(prefix), "{budget:?} / {prefix}");

        let (outcome, requests, records) = open_with("overflow", 2_048, 200).await;
        let err = outcome.err().expect("refused");
        assert!(err.contains("too small for the"), "{err}");
        assert_eq!(requests, 0, "nothing was sent to find that out");
        assert_eq!(rewrites(&records), 0);
    }

    // A session that opens over its handoff line — a successor whose handoff
    // document is large, or simply a long first prompt — has nothing to hand
    // off: the only thing in it is the message. Handing that off produced a
    // successor in the same state, with no round ever run in between.
    #[tokio::test]
    async fn an_opening_message_over_the_line_is_sent_not_handed_off_or_summarized() {
        let (budget, prefix) = lines(32_768);
        assert!(budget.soft < budget.hard && budget.hard < budget.ceiling);
        for (tag, total) in [
            ("soft", (budget.soft + budget.hard) / 2),
            ("hard", (budget.hard + budget.ceiling) / 2),
        ] {
            let chars = usize::try_from((total - prefix) * 3).unwrap();
            let (outcome, requests, records) = open_with(tag, 32_768, chars).await;
            assert!(
                matches!(outcome, Ok(RunOutcome::Completed)),
                "{tag}: {:?}",
                outcome.err()
            );
            assert_eq!(requests, 1, "{tag}: the message itself, and nothing else");
            assert_eq!(rewrites(&records), 0, "{tag}");
        }

        // Past the ceiling there is still nothing to try: it is refused as
        // what it is, a message too long for the model.
        let chars = usize::try_from((budget.ceiling + 500 - prefix) * 3).unwrap();
        let (outcome, requests, records) = open_with("ceiling", 32_768, chars).await;
        assert!(
            outcome
                .err()
                .unwrap()
                .contains("exceeds the model's context limit")
        );
        assert_eq!((requests, rewrites(&records)), (0, 0));
    }

    // `push_user_blocks` merges into the last user turn without writing a
    // record. A prune that rebuilt the history from the file dropped whatever
    // had been merged — the prompt typed after a run that ended on tool
    // results, a hook's correction — just before the model was to read it.
    #[tokio::test]
    async fn what_was_merged_into_the_last_turn_survives_a_prune() {
        let config = AgentConfig::default();
        let store = store("merge");
        let ctx = crate::agent::tools::tests_support::ctx();
        let events: Channel<AgentEvent> = Channel::new(|_| Ok(()));
        let system = "You are a test.".to_string();
        let budget = ContextBudget::for_model(&config, &config.builder_model, 0);

        let mut history = vec![Message {
            role: "user".into(),
            content: vec![ContentBlock::text("Refactor the parser.")],
        }];
        for i in 0..30 {
            let id = format!("r{i}");
            history.push(Message {
                role: "assistant".into(),
                content: vec![ContentBlock::tool_use(
                    &id,
                    "read_file",
                    serde_json::json!({ "path": format!("/ws/src/f{i}.rs") }),
                )],
            });
            history.push(Message {
                role: "user".into(),
                content: vec![ContentBlock::tool_result(
                    &id,
                    format!("// {i}\n").repeat(3_000),
                )],
            });
        }
        // In memory only: no record of it exists in the store.
        history
            .last_mut()
            .unwrap()
            .content
            .push(ContentBlock::text("ONLY TOUCH THE LEXER"));

        let estimated = estimate_tokens(&history, &system, &[]);
        assert!(estimated >= budget.soft, "{estimated}");
        let (new_estimate, _) = try_verbatim_compaction(
            &config,
            &budget,
            PromptProfile::Standard,
            estimated,
            &mut history,
            &system,
            &[],
            &store,
            &ctx,
            &events,
        )
        .await
        .expect("thirty old reads are worth cutting");
        assert!(new_estimate < estimated * 8 / 10);

        let last = history.last().unwrap();
        assert_eq!(last.role, "user");
        assert!(
            last.content
                .iter()
                .any(|b| b.get_text() == Some("ONLY TOUCH THE LEXER")),
            "the instruction is still the last thing the model reads"
        );
        assert_eq!(
            history[0].content[0].get_text(),
            Some("Refactor the parser.")
        );
        // And the cut is on record, so a reload makes the same one.
        let records = crate::agent::persist::load_records(&store.path).unwrap();
        assert_eq!(rewrites(&records), 1);
        let _ = std::fs::remove_file(&store.path);
    }
}

/// Per-request accounting: what the eval reads cost per task and rounds from.
#[cfg(test)]
mod usage_record_tests {
    use super::surface_tests::config_for;
    use super::*;
    use crate::agent::subagent::scripted_model::{Reply, spawn_with_usage};

    #[tokio::test]
    async fn every_provider_request_of_a_run_leaves_one_usage_record() {
        let root = std::env::temp_dir().join(format!(
            "claudinio-usage-run-{}-{}",
            std::process::id(),
            now_ms()
        ));
        std::fs::create_dir_all(&root).unwrap();
        let file = root.join("greet.txt");
        std::fs::write(&file, "hello world\n").unwrap();
        let path = file.to_string_lossy().to_string();
        let store = SessionStore {
            path: root.join("session.jsonl"),
        };
        std::fs::write(&store.path, "").unwrap();
        let (base, _, bodies) = spawn_with_usage(
            vec![
                Reply::Tool {
                    name: "read_file",
                    input: serde_json::json!({ "path": path }),
                },
                Reply::Tool {
                    name: "edit_file",
                    input: serde_json::json!({
                        "path": path, "old_string": "hello", "new_string": "goodbye",
                    }),
                },
                Reply::Text("changed the greeting"),
            ],
            Some(serde_json::json!({
                "input_tokens": 100,
                "cache_read_input_tokens": 2_000,
                "cache_creation_input_tokens": 300,
            })),
        )
        .await;
        // A 32k window runs the Compact profile, which edits without a subagent,
        // so every request in this run is the main session's own. The judge
        // (the Brain model, asked whether the final turn is finished) is
        // pointed at the same stub, so its request is counted too.
        let mut config = config_for(&base, Some(32_768));
        config.brain_model = "stub/m".into();
        let ctx = ToolContext {
            workspace_root: Some(root.to_string_lossy().to_string()),
            session_store_path: Some(store.path.to_string_lossy().to_string()),
            ..crate::agent::tools::tests_support::ctx()
        };
        let events: Channel<AgentEvent> = Channel::new(|_| Ok(()));
        let outcome = run_workflow(
            &config,
            &mut Vec::new(),
            "say goodbye instead".into(),
            Vec::new(),
            &events,
            &ApprovalMap::default(),
            &AnswerMap::default(),
            "usage-test",
            &ctx,
            &store,
            &Arc::new(SteeringCtl::new()),
            &Arc::new(ModeCtl::new(SessionMode::Builder, ModeOrigin::Human)),
        )
        .await;
        assert!(matches!(outcome, Ok(RunOutcome::Completed)));

        let usage: Vec<_> = crate::agent::persist::load_records(&store.path)
            .unwrap()
            .into_iter()
            .filter_map(|r| match r {
                SessionRecord::Usage {
                    round,
                    model,
                    input_tokens,
                    cache_read_tokens,
                    cache_creation_tokens,
                    output_tokens,
                    subagent,
                    ..
                } => Some((
                    round,
                    model,
                    input_tokens,
                    cache_read_tokens,
                    cache_creation_tokens,
                    output_tokens,
                    subagent,
                )),
                _ => None,
            })
            .collect();
        let requests = bodies.lock().unwrap().len();
        assert_eq!(requests, 4, "three rounds and the completion judge");
        assert_eq!(usage.len(), requests, "one record per provider request");
        let rounds: Vec<u32> = usage.iter().map(|u| u.0).collect();
        assert_eq!(rounds, vec![1, 2, 3, 4]);
        for u in &usage {
            assert_eq!(u.1, "stub/m");
            assert_eq!((u.2, u.3, u.4), (100, 2_000, 300));
            assert_eq!(u.6, None, "the main session's requests carry no subagent");
        }
        let outputs: Vec<u32> = usage.iter().map(|u| u.5).collect();
        assert_eq!(
            outputs,
            vec![20, 20, 5, 0],
            "the judge's stub reply reports no output"
        );
        std::fs::remove_dir_all(&root).ok();
    }
}

/// Deferred tools through the real loop (lean-harness Phase 2): what
/// `load_tools` and a direct call to a deferred tool do to the next request,
/// and that a later run of the same session sends the same tool array.
#[cfg(test)]
mod deferred_loop_tests {
    use super::*;
    use crate::agent::session::{AnswerMap, ApprovalMap, ModeCtl, ModeOrigin, SteeringCtl};
    use crate::agent::subagent::scripted_model::{Reply, spawn_recording};

    struct Workspace {
        root: std::path::PathBuf,
        store: SessionStore,
    }

    fn workspace(tag: &str) -> Workspace {
        let root = std::env::temp_dir().join(format!(
            "claudinio-deferred-{tag}-{}-{}",
            std::process::id(),
            now_ms()
        ));
        std::fs::create_dir_all(&root).unwrap();
        std::fs::write(root.join("greet.txt"), "hello world\n").unwrap();
        let store = SessionStore {
            path: root.join("session.jsonl"),
        };
        std::fs::write(&store.path, "").unwrap();
        Workspace { root, store }
    }

    /// One user message against a model following `script`, on `ws`'s
    /// session file. Returns the run's outcome and every request body sent.
    async fn run(ws: &Workspace, script: Vec<Reply>) -> (Result<RunOutcome, String>, Vec<String>) {
        run_on(ws, None, script).await
    }

    /// `run`, on a model with this context window.
    async fn run_on(
        ws: &Workspace,
        window: Option<u32>,
        script: Vec<Reply>,
    ) -> (Result<RunOutcome, String>, Vec<String>) {
        let (base, _, bodies) = spawn_recording(script).await;
        let config = super::surface_tests::config_for(&base, window);
        let ctx = ToolContext {
            workspace_root: Some(ws.root.to_string_lossy().to_string()),
            session_store_path: Some(ws.store.path.to_string_lossy().to_string()),
            ..crate::agent::tools::tests_support::ctx()
        };
        let events: Channel<AgentEvent> = Channel::new(|_| Ok(()));
        let mut history = crate::agent::persist::history_from_records(
            &crate::agent::persist::load_records(&ws.store.path).unwrap_or_default(),
        );
        let outcome = run_workflow(
            &config,
            &mut history,
            "go".into(),
            Vec::new(),
            &events,
            &ApprovalMap::default(),
            &AnswerMap::default(),
            "deferred-test",
            &ctx,
            &ws.store,
            &Arc::new(SteeringCtl::new()),
            &Arc::new(ModeCtl::new(SessionMode::Builder, ModeOrigin::Human)),
        )
        .await;
        let bodies = bodies.lock().unwrap().clone();
        (outcome, bodies)
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

    fn last_result(body: &str) -> String {
        let body: Value = serde_json::from_str(body).unwrap();
        let last = body["messages"].as_array().unwrap().last().unwrap().clone();
        last["content"][0]["content"].to_string()
    }

    fn records_of(ws: &Workspace) -> Vec<SessionRecord> {
        crate::agent::persist::load_records(&ws.store.path).unwrap()
    }

    #[tokio::test]
    async fn load_tools_appends_the_definitions_to_the_next_request_and_records_the_load() {
        let ws = workspace("load");
        let (outcome, bodies) = run(
            &ws,
            vec![
                Reply::Tool {
                    name: "load_tools",
                    input: serde_json::json!({"names": ["semantic_search", "enter_plan_mode"]}),
                },
                Reply::Text("done"),
            ],
        )
        .await;
        assert!(
            matches!(outcome, Ok(RunOutcome::Completed)),
            "{:?}",
            outcome.err()
        );
        let first = tool_names(&bodies[0]);
        let second = tool_names(&bodies[1]);
        assert!(!first.contains(&"semantic_search".to_string()));
        assert_eq!(second[..first.len()], first[..], "nothing before moves");
        assert_eq!(
            second[first.len()..],
            ["semantic_search", "enter_plan_mode"]
        );
        // The system prompt is the same bytes on both requests.
        let system = |b: &str| serde_json::from_str::<Value>(b).unwrap()["system"].clone();
        assert_eq!(system(&bodies[0]), system(&bodies[1]));
        assert!(last_result(&bodies[1]).contains("Loaded: semantic_search, enter_plan_mode"));
        let records = records_of(&ws);
        assert_eq!(
            crate::agent::deferred::loaded_names(&records),
            ["semantic_search", "enter_plan_mode"]
        );
        assert!(records.iter().any(|r| matches!(
            r,
            SessionRecord::CacheBreak { reason, .. } if reason == "tools_loaded"
        )));
        std::fs::remove_dir_all(&ws.root).ok();
    }

    /// A model that calls a deferred tool it never loaded gets the call run —
    /// refusing it would cost a round to learn the name it already knew — and
    /// the definition from then on.
    #[tokio::test]
    async fn a_deferred_tool_called_without_loading_runs_and_joins_the_list() {
        let ws = workspace("direct");
        let (outcome, bodies) = run(
            &ws,
            vec![
                Reply::Tool {
                    name: "code_search",
                    input: serde_json::json!({"query": "greet"}),
                },
                Reply::Text("done"),
            ],
        )
        .await;
        assert!(
            matches!(outcome, Ok(RunOutcome::Completed)),
            "{:?}",
            outcome.err()
        );
        // The call reaches code_search itself: its own answer (a temp dir has
        // no index), not a refusal for a tool the request did not carry.
        let result = last_result(&bodies[1]);
        assert!(
            result.contains("index not available"),
            "the call ran: {result}"
        );
        assert!(!result.contains("unknown tool") && !result.contains("not available in this"));
        assert_eq!(tool_names(&bodies[1]).last().unwrap(), "code_search");
        assert!(!tool_names(&bodies[0]).contains(&"code_search".to_string()));
        assert_eq!(
            crate::agent::deferred::loaded_names(&records_of(&ws)),
            ["code_search"]
        );
        std::fs::remove_dir_all(&ws.root).ok();
    }

    /// The prefix is rewritten once per round however many deferred tools the
    /// round loaded, so a round that calls two of them side by side — or
    /// `load_tools` and a deferred tool directly — must count as one cache
    /// break, not one per call: the plan's gate counts these records.
    #[tokio::test]
    async fn two_deferred_tools_loaded_in_one_round_are_one_cache_break() {
        let ws = workspace("one-break");
        let (outcome, bodies) = run(
            &ws,
            vec![
                Reply::Tools(vec![
                    ("code_search", serde_json::json!({"query": "greet"})),
                    ("symbol_lookup", serde_json::json!({"name": "greet"})),
                    ("load_tools", serde_json::json!({"names": ["file_outline"]})),
                ]),
                Reply::Text("done"),
            ],
        )
        .await;
        assert!(
            matches!(outcome, Ok(RunOutcome::Completed)),
            "{:?}",
            outcome.err()
        );
        let second = tool_names(&bodies[1]);
        assert_eq!(
            second[second.len() - 3..],
            ["code_search", "symbol_lookup", "file_outline"],
            "loaded in the order the calls came"
        );
        let records = records_of(&ws);
        let breaks = records
            .iter()
            .filter(|r| {
                matches!(
                    r,
                    SessionRecord::CacheBreak { reason, .. } if reason == "tools_loaded"
                )
            })
            .count();
        assert_eq!(breaks, 1, "one prefix rewrite, one cache break");
        let loads = records
            .iter()
            .filter(|r| matches!(r, SessionRecord::ToolsLoaded { .. }))
            .count();
        assert_eq!(loads, 1, "one record names everything the round loaded");
        assert_eq!(
            crate::agent::deferred::loaded_names(&records),
            ["code_search", "symbol_lookup", "file_outline"]
        );
        std::fs::remove_dir_all(&ws.root).ok();
    }

    /// The next message of the session — or the session reopened after a
    /// restart, which reads the same file — starts from the tool array the
    /// last request ended with, so the provider's cached prefix still matches.
    #[tokio::test]
    async fn a_later_run_of_the_session_sends_the_tools_it_loaded_in_the_same_order() {
        let ws = workspace("resume");
        let (_, first_run) = run(
            &ws,
            vec![
                Reply::Tool {
                    name: "load_tools",
                    input: serde_json::json!({"names": ["file_outline"]}),
                },
                Reply::Tool {
                    name: "code_search",
                    input: serde_json::json!({"query": "greet"}),
                },
                Reply::Text("done"),
            ],
        )
        .await;
        let (outcome, second_run) = run(&ws, vec![Reply::Text("again")]).await;
        assert!(
            matches!(outcome, Ok(RunOutcome::Completed)),
            "{:?}",
            outcome.err()
        );
        let ended_with = tool_names(first_run.last().unwrap());
        assert_eq!(
            &ended_with[ended_with.len() - 2..],
            ["file_outline", "code_search"]
        );
        assert_eq!(tool_names(&second_run[0]), ended_with);
        std::fs::remove_dir_all(&ws.root).ok();
    }

    /// The Compact profile defers nothing: `load_tools` is a tool it was
    /// never offered, refused the way any other one is, and nothing is
    /// recorded.
    #[tokio::test]
    async fn a_compact_session_refuses_load_tools_like_any_tool_it_lacks() {
        let ws = workspace("compact");
        let (_, bodies) = run_on(
            &ws,
            Some(32_768),
            vec![
                Reply::Tool {
                    name: "load_tools",
                    input: serde_json::json!({"names": ["semantic_search"]}),
                },
                Reply::Text("done"),
            ],
        )
        .await;
        let result = last_result(&bodies[1]);
        assert!(
            result.contains("is not available in this session"),
            "{result}"
        );
        assert!(crate::agent::deferred::loaded_names(&records_of(&ws)).is_empty());
        std::fs::remove_dir_all(&ws.root).ok();
    }

    #[tokio::test]
    async fn loading_a_tool_that_is_not_deferred_here_is_an_error_that_lists_what_is() {
        let ws = workspace("unknown");
        let (_, bodies) = run(
            &ws,
            vec![
                Reply::Tool {
                    name: "load_tools",
                    input: serde_json::json!({"names": ["write_plan"]}),
                },
                Reply::Text("done"),
            ],
        )
        .await;
        let result = last_result(&bodies[1]);
        assert!(
            result.contains("Error") && result.contains("semantic_search"),
            "{result}"
        );
        assert_eq!(tool_names(&bodies[1]), tool_names(&bodies[0]));
        assert!(crate::agent::deferred::loaded_names(&records_of(&ws)).is_empty());
        std::fs::remove_dir_all(&ws.root).ok();
    }
}
