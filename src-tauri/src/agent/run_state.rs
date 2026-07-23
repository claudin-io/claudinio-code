//! State carried across the rounds of one agent run.
//!
//! These used to be ~30 loose `let mut` bindings at the top of
//! `run_workflow_with_profile`, threaded by hand into `roll_cost` (12 args) and
//! `write_status` (10 args) at a dozen call sites. Grouping them is what makes
//! the loop body extractable at all, and it removes the class of bug where one
//! of the twelve arguments is passed in the wrong position.

use crate::agent::persist::{SessionRecord, SessionStore};
use crate::agent::tools::ToolContext;

/// Token and cost accounting for a run.
///
/// `cumul_*` is the whole session, reloaded from the last `Status` record so it
/// survives restarts. `total_*` / `run_cost_*` accumulate over the rounds of the
/// current run and are folded in by `roll` on whichever terminal path the run
/// takes (done, error, interrupt, handoff, round cap) — exactly one of them
/// runs, so there is no double counting. `subagent_cost` tracks what spawned
/// subagents spent, which the provider never reports on the parent's usage.
#[derive(Debug, Default, Clone)]
pub struct CostLedger {
    pub cumul_in: u64,
    pub cumul_out: u64,
    pub cumul_cost: Option<f64>,
    pub cumul_cost_input: Option<f64>,
    pub cumul_cost_output: Option<f64>,
    pub cumul_cost_cache: Option<f64>,

    pub total_in: u32,
    pub total_out: u32,
    pub total_cache: u32,
    pub run_cost: Option<f64>,
    pub run_cost_input: Option<f64>,
    pub run_cost_output: Option<f64>,
    pub run_cost_cache: Option<f64>,

    pub subagent_cost: f64,
}

/// The tuple shape `persist::cumulative_totals` returns.
pub type CumulativeTotals = (u64, u64, Option<f64>, Option<f64>, Option<f64>, Option<f64>);

impl CostLedger {
    /// Seed the cumulative side from the session's last `Status` record.
    pub fn resuming(cumul: CumulativeTotals) -> Self {
        Self {
            cumul_in: cumul.0,
            cumul_out: cumul.1,
            cumul_cost: cumul.2,
            cumul_cost_input: cumul.3,
            cumul_cost_output: cumul.4,
            cumul_cost_cache: cumul.5,
            ..Default::default()
        }
    }

    /// Fold this round's tokens and cost into the cumulative totals.
    ///
    /// The blended `cumul_cost` is kept independent of the per-category
    /// breakdown so sessions persisted before the breakdown existed do not lose
    /// their historical total.
    pub fn roll(&mut self, model: &str) {
        self.cumul_in += self.total_in as u64;
        self.cumul_out += self.total_out as u64;
        let (ci, co, cc) = crate::agent::session::cost_or_estimate(
            model,
            self.total_in,
            self.total_cache,
            self.total_out,
            self.run_cost_input,
            self.run_cost_output,
            self.run_cost_cache,
        );
        self.cumul_cost = Some(self.cumul_cost.unwrap_or(0.0) + ci + co + cc + self.subagent_cost);
        self.cumul_cost_input = Some(self.cumul_cost_input.unwrap_or(0.0) + ci);
        self.cumul_cost_output = Some(self.cumul_cost_output.unwrap_or(0.0) + co);
        self.cumul_cost_cache = Some(self.cumul_cost_cache.unwrap_or(0.0) + cc);
    }

    /// Persist a `Status` record with the cumulative stats and the size of the
    /// context the next request will carry.
    pub fn write_status(
        &self,
        store: &SessionStore,
        ctx: &ToolContext,
        session_id: &str,
        context_tokens: Option<u64>,
    ) {
        store.try_append(&SessionRecord::Status {
            session_id: session_id.to_string(),
            total_input_tokens: self.cumul_in,
            total_output_tokens: self.cumul_out,
            total_cost: self.cumul_cost,
            total_cost_input: self.cumul_cost_input,
            total_cost_output: self.cumul_cost_output,
            total_cost_cache_read: self.cumul_cost_cache,
            context_tokens,
            ts: crate::agent::persist::now_ms(),
        });
        crate::agent::persist::invalidate_cache(&store.path, &ctx.records_cache);
    }
}

/// Golden-goals loop bookkeeping.
///
/// The counters resume from the session's records so a restart does not reset
/// the cap mid-loop, and a linked session inherits its predecessor's values —
/// without that, every golden flip would mint a fresh budget and the loop could
/// run forever.
#[derive(Debug, Default, Clone)]
pub struct GoldenLoopState {
    pub cycle: u32,
    pub last_pending: Vec<String>,
    pub stalls: usize,
}

/// Per-run guards against the model looping without making progress. Each is a
/// consecutive-round streak that trips a bounded intervention, never a hard
/// abort on the first occurrence.
#[derive(Debug, Default, Clone)]
pub struct GuardState {
    /// Rounds ending at the output-token cap.
    pub truncation_streak: u32,
    /// Rounds producing neither text nor a tool call.
    pub empty_streak: u32,
    /// Terminal turns the completion judge ruled unfinished.
    pub unfinished_streak: u32,
    /// Brain-mode rounds using only explore tools — no interview, plan or tasks.
    pub brain_explore_streak: u32,
    /// The agent called `finalize_plan` during this run.
    pub plan_finalized: bool,
    /// The single reminder to finalize has been sent; next time the harness
    /// appends the log itself.
    pub finalize_nudged: bool,
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn roll_folds_round_tokens_into_the_cumulative_totals() {
        let mut ledger = CostLedger::resuming((100, 50, Some(1.0), None, None, None));
        ledger.total_in = 10;
        ledger.total_out = 5;
        ledger.roll("claudinio");
        assert_eq!(ledger.cumul_in, 110);
        assert_eq!(ledger.cumul_out, 55);
        // The blended total only ever grows.
        assert!(ledger.cumul_cost.unwrap() > 1.0);
    }

    #[test]
    fn roll_includes_subagent_cost_in_the_blended_total_only() {
        let mut ledger = CostLedger::resuming((0, 0, None, None, None, None));
        ledger.subagent_cost = 2.5;
        ledger.roll("claudinio");
        let blended = ledger.cumul_cost.unwrap();
        let by_category = ledger.cumul_cost_input.unwrap()
            + ledger.cumul_cost_output.unwrap()
            + ledger.cumul_cost_cache.unwrap();
        assert!((blended - by_category - 2.5).abs() < 1e-9);
    }

    #[test]
    fn resuming_carries_every_cumulative_field() {
        let l = CostLedger::resuming((1, 2, Some(3.0), Some(4.0), Some(5.0), Some(6.0)));
        assert_eq!(
            (
                l.cumul_in,
                l.cumul_out,
                l.cumul_cost,
                l.cumul_cost_input,
                l.cumul_cost_output,
                l.cumul_cost_cache
            ),
            (1, 2, Some(3.0), Some(4.0), Some(5.0), Some(6.0))
        );
    }
}
