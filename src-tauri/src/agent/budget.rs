//! How much context a run may use, derived from the model it talks to.
//!
//! The session used to be driven by three constants — hand off at the slider
//! (120k by default), summarize at 150k, refuse at 200k — whatever model was on
//! the other end. That is right for the 200k models the app shipped with and
//! wrong for everything smaller: a 32k local model overflowed long before any
//! of the three lines was reached, which is the regime where context
//! management matters most ("An Empirical Study of Harness Design for Coding
//! Agents", arXiv 2609.20804: at a 32k window, an unmanaged harness loses 78.7%
//! of tasks to overflow and any managed one loses none).
//!
//! Three regimes, chosen by what the window can hold:
//!
//! - **Full** — the window is 200k or more. The numbers are exactly the ones
//!   the app always used.
//! - **Sized** — smaller than 200k, but large enough to honor the handoff
//!   slider. The slider keeps its meaning; only the ceiling moves down to what
//!   the model really accepts. This is every local model served with a window
//!   cut to the slider (`llama::effective_ctx`).
//! - **Scaled** — too small for the slider. The lines are placed
//!   proportionally over the space the conversation can actually use: what is
//!   left of the window after the reply and the fixed prefix (system prompt and
//!   tool schemas) are taken out. 60% and 85% are the paper's thresholds, and
//!   also what 120k / 200k already was.

use crate::agent::provider::AgentConfig;
use crate::agent::session::{COMPACT_THRESHOLD, MAX_CONTEXT_TOKENS};
use std::sync::atomic::{AtomicUsize, Ordering};

/// History cap for one tool result when the window is not the constraint.
pub const TOOL_RESULT_CHARS: usize = 24_000;
/// What a trimmed command output may occupy when the window is not the
/// constraint.
pub const TRIM_CHARS: usize = crate::agent::output_trim::BUDGET_CHARS;
/// Budget for the verbatim tail kept across a summarizing compaction.
pub const TAIL_TOKENS: u64 = 20_000;

/// Share of the conversation space at which a scaled run sheds weight.
const SOFT_PERCENT: u64 = 60;
/// Share at which a scaled run falls back to the summarizing compaction.
const HARD_PERCENT: u64 = 85;
/// Assumed reply size when nothing pins one, and never more than a quarter of
/// the window: a model whose catalog entry says "64k output, 64k context" still
/// has to be sent a prompt.
const DEFAULT_REPLY_RESERVE: u64 = 8_192;
/// The gap kept between the handoff line and the compaction behind it, so the
/// fallback is not the same request as the thing it falls back from.
const HARD_GAP: u64 = 10_000;
/// One tool result may take at most this share of a scaled conversation space.
const RESULT_PERCENT: u64 = 25;
/// Below this a result stops being useful at all; the model would only see
/// truncation notes.
const MIN_TOOL_RESULT_CHARS: usize = 2_000;
/// The verbatim tail's share of a scaled conversation space (the paper keeps
/// 0.3 of the budget verbatim).
const TAIL_PERCENT: u64 = 30;
/// Same chars-per-token rule as `session::estimate_tokens`.
const CHARS_PER_TOKEN: u64 = 3;

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Regime {
    Full,
    Sized,
    Scaled,
}

impl Regime {
    pub fn as_str(&self) -> &'static str {
        match self {
            Regime::Full => "full",
            Regime::Sized => "sized",
            Regime::Scaled => "scaled",
        }
    }
}

/// The lines one run is held to. All token counts are on the session's own
/// meter (`session::estimate_tokens`), which is what they are compared against.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct ContextBudget {
    /// What the model accepts, capped at `MAX_CONTEXT_TOKENS`.
    pub window: u64,
    pub regime: Regime,
    /// Shed weight here: prune, then hand off.
    pub soft: u64,
    /// The summarizing compaction, when shedding was not enough.
    pub hard: u64,
    /// Refuse to send at or above this.
    pub ceiling: u64,
    /// History cap for one tool result.
    pub tool_result_chars: usize,
    /// What a trimmed command output may occupy.
    pub trim_chars: usize,
    /// Verbatim tail kept across a summarizing compaction.
    pub tail_tokens: u64,
    /// The part of the context no prune can touch, when it is large enough to
    /// matter: the fixed prefix of a scaled run. A prune is judged by the room
    /// it leaves above this, not above zero — on a small window the prefix can
    /// be half of everything, and a target measured from zero would be one no
    /// prune could ever meet. Zero where the window is not the constraint,
    /// which keeps those runs on the acceptance rule they always had.
    pub prune_floor: u64,
}

impl ContextBudget {
    /// `slider` is the user's handoff preference; `reply_reserve` the most
    /// output a request will ask for, when known; `prefix_tokens` the fixed
    /// part of every request.
    pub fn new(window: u64, slider: u64, reply_reserve: Option<u64>, prefix_tokens: u64) -> Self {
        if window >= MAX_CONTEXT_TOKENS {
            // The slider goes to 256k but nothing above the ceiling is ever
            // sent, so an unclamped value put the handoff — and the
            // compaction 10k behind it — out of reach, leaving only the
            // refusal. Keep both lines below the ceiling.
            let hard_cap = MAX_CONTEXT_TOKENS * 9 / 10;
            let soft = slider.min(hard_cap - HARD_GAP);
            return Self {
                window: MAX_CONTEXT_TOKENS,
                regime: Regime::Full,
                soft,
                hard: COMPACT_THRESHOLD.max(soft + HARD_GAP),
                ceiling: MAX_CONTEXT_TOKENS,
                tool_result_chars: TOOL_RESULT_CHARS,
                trim_chars: TRIM_CHARS,
                tail_tokens: TAIL_TOKENS,
                prune_floor: 0,
            };
        }
        let reserve = reply_reserve
            .unwrap_or(DEFAULT_REPLY_RESERVE)
            .min(window / 4);
        let usable = window - reserve;
        if usable >= slider {
            return Self {
                window,
                regime: Regime::Sized,
                soft: slider,
                hard: (slider + HARD_GAP).min(usable),
                ceiling: usable,
                tool_result_chars: TOOL_RESULT_CHARS,
                trim_chars: TRIM_CHARS,
                tail_tokens: TAIL_TOKENS,
                prune_floor: 0,
            };
        }
        // What is left for the conversation itself. A prefix as large as the
        // window leaves nothing, and every line collapses onto the ceiling:
        // the run is refused with a message that says so, instead of failing
        // at the provider with a context error nobody can act on.
        let space = usable.saturating_sub(prefix_tokens);
        let floor = usable.min(prefix_tokens);
        let result_chars = usize::try_from(space * CHARS_PER_TOKEN * RESULT_PERCENT / 100)
            .unwrap_or(usize::MAX)
            .clamp(MIN_TOOL_RESULT_CHARS, TOOL_RESULT_CHARS);
        Self {
            window,
            regime: Regime::Scaled,
            soft: floor + space * SOFT_PERCENT / 100,
            hard: floor + space * HARD_PERCENT / 100,
            ceiling: usable,
            tool_result_chars: result_chars,
            trim_chars: result_chars * TRIM_CHARS / TOOL_RESULT_CHARS,
            tail_tokens: TAIL_TOKENS.min(space * TAIL_PERCENT / 100),
            prune_floor: floor,
        }
    }

    /// The budget for a run against `model`, whose requests all start with
    /// `prefix_tokens` of system prompt and tool schemas.
    pub fn for_model(config: &AgentConfig, model: &str, prefix_tokens: u64) -> Self {
        Self::new(
            config.context_window_for(model),
            config.effective_handoff_threshold(),
            config.known_output_limit(model),
            prefix_tokens,
        )
    }

    /// True when the prefix alone leaves no room to work in: the model is too
    /// small for this prompt and these tools, and no amount of pruning helps.
    pub fn prefix_overflows(&self, prefix_tokens: u64) -> bool {
        prefix_tokens >= self.ceiling
    }

    /// What the user is told when a request cannot be sent.
    pub fn refusal(&self, prefix_tokens: u64) -> String {
        let window = self.window / 1000;
        if self.prefix_overflows(prefix_tokens) {
            format!(
                "This model's context window ({window}k tokens) is too small for the \
                 system prompt and tools alone ({}k tokens). Pick a model with a larger \
                 window, or switch off tools you do not need (MCP servers, browser).",
                prefix_tokens / 1000
            )
        } else {
            format!(
                "The message exceeds the model's context limit ({window}k tokens). \
                 Reduce the attachments or start a new session to continue."
            )
        }
    }
}

/// The per-run caps a tool needs while it runs, shared through `ToolContext`.
///
/// Atomics rather than plain fields because the context is built before the
/// run knows its model, and a mode switch mid-run changes the model — and with
/// it the caps — without rebuilding the context.
#[derive(Debug)]
pub struct RunLimits {
    tool_result_chars: AtomicUsize,
    trim_chars: AtomicUsize,
}

impl Default for RunLimits {
    fn default() -> Self {
        Self {
            tool_result_chars: AtomicUsize::new(TOOL_RESULT_CHARS),
            trim_chars: AtomicUsize::new(TRIM_CHARS),
        }
    }
}

impl RunLimits {
    pub fn for_budget(budget: &ContextBudget) -> Self {
        let limits = Self::default();
        limits.apply(budget);
        limits
    }

    pub fn apply(&self, budget: &ContextBudget) {
        self.tool_result_chars
            .store(budget.tool_result_chars, Ordering::Relaxed);
        self.trim_chars.store(budget.trim_chars, Ordering::Relaxed);
    }

    pub fn tool_result_chars(&self) -> usize {
        self.tool_result_chars.load(Ordering::Relaxed)
    }

    pub fn trim_chars(&self) -> usize {
        self.trim_chars.load(Ordering::Relaxed)
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    /// The contract with every existing user: a 200k model, default settings,
    /// behaves exactly as it did before windows were tracked.
    #[test]
    fn a_full_window_keeps_the_numbers_the_app_always_used() {
        for window in [200_000, 256_000, 1_000_000] {
            let b = ContextBudget::new(window, 120_000, None, 33_000);
            assert_eq!(b.regime, Regime::Full);
            assert_eq!((b.soft, b.hard, b.ceiling), (120_000, 150_000, 200_000));
            assert_eq!(b.window, 200_000);
            assert_eq!(b.tool_result_chars, 24_000);
            assert_eq!(b.trim_chars, 20_000);
            assert_eq!(b.tail_tokens, 20_000);
            assert_eq!(b.prune_floor, 0);
        }
    }

    #[test]
    fn the_prefix_and_the_reply_reserve_do_not_move_a_full_window() {
        let a = ContextBudget::new(200_000, 120_000, None, 0);
        let b = ContextBudget::new(200_000, 120_000, Some(64_000), 90_000);
        assert_eq!(a, b);
    }

    /// The slider reaches 256k; the ceiling is 200k. Unclamped, a slider above
    /// the ceiling meant no handoff and no compaction — only the refusal.
    #[test]
    fn a_slider_above_the_ceiling_still_leaves_both_lines_reachable() {
        for slider in [180_000, 200_000, 256_000] {
            let b = ContextBudget::new(200_000, slider, None, 0);
            assert_eq!(b.soft, 170_000);
            assert_eq!(b.hard, 180_000);
            assert!(b.soft < b.hard && b.hard < b.ceiling);
        }
        // Below the clamp the slider is taken as given.
        let b = ContextBudget::new(200_000, 144_000, None, 0);
        assert_eq!((b.soft, b.hard), (144_000, 154_000));
    }

    /// `CLAUDINIO_HANDOFF_TOKENS` exists so the handoff can be exercised
    /// end-to-end with a tiny number; it must stay unclamped from below.
    #[test]
    fn a_tiny_slider_is_honored_as_given() {
        let b = ContextBudget::new(200_000, 5_000, None, 0);
        assert_eq!((b.soft, b.hard, b.ceiling), (5_000, 150_000, 200_000));
    }

    /// A local model served at slider + 8k of reply headroom: the slider keeps
    /// its meaning, and the fallback behind it is now inside the window
    /// instead of at a 150k the server would have rejected.
    #[test]
    fn a_window_cut_to_the_slider_is_sized_not_scaled() {
        let b = ContextBudget::new(128_192, 120_000, Some(8_192), 33_000);
        assert_eq!(b.regime, Regime::Sized);
        assert_eq!((b.soft, b.hard, b.ceiling), (120_000, 120_000, 120_000));
        assert_eq!(b.tool_result_chars, 24_000);

        let roomy = ContextBudget::new(160_000, 120_000, Some(8_000), 33_000);
        assert_eq!(roomy.regime, Regime::Sized);
        assert_eq!(
            (roomy.soft, roomy.hard, roomy.ceiling),
            (120_000, 130_000, 152_000)
        );
    }

    #[test]
    fn a_small_window_scales_every_line_over_what_the_conversation_can_use() {
        // 32k served, 8k reply, 4k of prompt and tools: 20k to work in.
        let b = ContextBudget::new(32_768, 120_000, Some(8_192), 4_576);
        assert_eq!(b.regime, Regime::Scaled);
        assert_eq!(b.ceiling, 24_576);
        assert_eq!(b.soft, 4_576 + 12_000);
        assert_eq!(b.hard, 4_576 + 17_000);
        assert!(b.soft < b.hard && b.hard < b.ceiling);
        // A quarter of the space, in chars: 20k tokens * 3 * 25%.
        assert_eq!(b.tool_result_chars, 15_000);
        assert_eq!(b.trim_chars, 12_500);
        assert_eq!(b.tail_tokens, 6_000);
        // Prunes are judged by the room they leave above the prefix.
        assert_eq!(b.prune_floor, 4_576);
    }

    #[test]
    fn a_64k_catalog_model_is_scaled_too() {
        let b = ContextBudget::new(65_536, 120_000, Some(8_192), 10_000);
        assert_eq!(b.regime, Regime::Scaled);
        assert_eq!(b.ceiling, 57_344);
        let space = 47_344;
        assert_eq!(b.soft, 10_000 + space * 60 / 100);
        assert_eq!(b.hard, 10_000 + space * 85 / 100);
        // Enough room that a result is not cut below the usual cap.
        assert_eq!(b.tool_result_chars, 24_000);
        assert_eq!(b.tail_tokens, 14_203);
    }

    #[test]
    fn the_reply_reserve_never_takes_more_than_a_quarter_of_the_window() {
        // A catalog entry claiming a 64k output on a 64k window.
        let b = ContextBudget::new(64_000, 120_000, Some(64_000), 0);
        assert_eq!(b.ceiling, 48_000);
        // Unknown output limit: the default reserve.
        let b = ContextBudget::new(64_000, 120_000, None, 0);
        assert_eq!(b.ceiling, 64_000 - 8_192);
    }

    #[test]
    fn a_prefix_as_large_as_the_window_collapses_the_lines_and_says_why() {
        let b = ContextBudget::new(32_768, 120_000, Some(8_192), 33_000);
        assert_eq!((b.soft, b.hard, b.ceiling), (24_576, 24_576, 24_576));
        assert!(b.prefix_overflows(33_000));
        let msg = b.refusal(33_000);
        assert!(msg.contains("32k"), "{msg}");
        assert!(msg.contains("33k"), "{msg}");
        assert!(msg.contains("too small"), "{msg}");
        // A result cap that is still a usable size, not zero.
        assert_eq!(b.tool_result_chars, 2_000);

        // The ordinary refusal names the real window, not a hardcoded 200k.
        let full = ContextBudget::new(200_000, 120_000, None, 30_000);
        assert!(!full.prefix_overflows(30_000));
        assert!(full.refusal(30_000).contains("200k"));
    }

    #[test]
    fn run_limits_start_at_the_defaults_and_follow_the_budget() {
        let limits = RunLimits::default();
        assert_eq!(limits.tool_result_chars(), 24_000);
        assert_eq!(limits.trim_chars(), 20_000);
        limits.apply(&ContextBudget::new(32_768, 120_000, Some(8_192), 4_576));
        assert_eq!(limits.tool_result_chars(), 15_000);
        assert_eq!(limits.trim_chars(), 12_500);
        let same = RunLimits::for_budget(&ContextBudget::new(200_000, 120_000, None, 0));
        assert_eq!(same.tool_result_chars(), 24_000);
    }
}
