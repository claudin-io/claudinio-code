//! Tool-loop watch: notice when the agent keeps making the same call and
//! getting the same answer, and break the cycle before it burns the session.
//!
//! The case that motivates it is real: one session issued the same tool call
//! 103 times in a row, the tool answered the same 23 tokens every time, and the
//! prompt grew by exactly the same amount per round — a dollar of tokens for
//! zero work. Nothing in the harness had a cycle breaker.
//!
//! Two layers, because arithmetic alone cannot tell a cycle from legitimate
//! repetition (re-running the tests after each edit, polling a CI run):
//!   * the **gate** is exact and free — identical (tool, args, result) or
//!     identical (tool, args) inside a short window. It decides *when to look*.
//!   * **Jev** decides *whether it is a cycle*, reading the window with a
//!     single `noul`. Without Jev only the strongest gate acts.
//!
//! The intervention is bounded and visible: a nudge telling the model what it
//! is repeating, at most [`MAX_NUDGES`] times; after that the run stops with
//! `tool_loop` and says why. A false positive costs one extra message, never
//! silently killed work.

use serde_json::{Value, json};
use std::collections::VecDeque;
use std::collections::hash_map::DefaultHasher;
use std::hash::{Hash, Hasher};

/// How many recent tool calls the watch remembers.
pub const WINDOW: usize = 10;
/// Identical (tool, args, result) occurrences that make the gate suspicious.
pub const SUSPECT_EXACT: usize = 3;
/// Identical (tool, args) occurrences — results differing — that make it suspicious.
pub const SUSPECT_SAME_CALL: usize = 3;
/// Identical (tool, args, result) occurrences treated as a cycle with no
/// judge at all. High on purpose: polling loops repeat too.
pub const CERTAIN_EXACT: usize = 5;
/// P(stuck) at or above which Jev's answer counts as a cycle. Measured live
/// (2026-10-05): four real cycle shapes scored 0.89–0.96, healthy TDD /
/// exploration / re-run-after-edit 0.03–0.05, and legitimate CI polling 0.72 —
/// the threshold sits above the polling case.
pub const STUCK_THRESHOLD: f64 = 0.85;
/// Nudges before the run is stopped.
pub const MAX_NUDGES: u32 = 2;
/// After asking Jev and hearing "not stuck", wait for this many new calls
/// before asking again, so a long legitimate poll is not judged every round.
pub const REASK_AFTER: usize = 3;
const EXCERPT_CHARS: usize = 300;

#[derive(Debug, Clone)]
struct Step {
    name: String,
    call_sig: u64,
    result_sig: u64,
    args_excerpt: String,
    result_excerpt: String,
}

/// What the gate says about the window right now.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Gate {
    Clear,
    /// Looks like repetition; needs a judge.
    Suspect,
    /// Repetition so exact that no judge is needed.
    Certain,
}

/// What the harness should do this round.
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum LoopAction {
    None,
    Nudge(String),
    Stop(String),
}

#[derive(Debug, Default, Clone)]
pub struct LoopWatch {
    recent: VecDeque<Step>,
    /// Total calls recorded, ever (monotonic; the window is a suffix of these).
    seen: usize,
    /// `seen` when Jev last answered "not stuck".
    cleared_at: Option<usize>,
    pub nudges: u32,
}

fn sig<T: Hash>(t: &T) -> u64 {
    let mut h = DefaultHasher::new();
    t.hash(&mut h);
    h.finish()
}

fn excerpt(s: &str) -> String {
    let s = s.trim();
    if s.chars().count() <= EXCERPT_CHARS {
        return s.replace('\n', " ⏎ ");
    }
    let head: String = s.chars().take(EXCERPT_CHARS).collect();
    format!("{} …", head.replace('\n', " ⏎ "))
}

impl LoopWatch {
    /// Remember one executed tool call and the text it returned.
    pub fn record(&mut self, name: &str, args: &Value, result: &str) {
        let args_text = args.to_string();
        self.recent.push_back(Step {
            name: name.to_string(),
            call_sig: sig(&(name, &args_text)),
            result_sig: sig(&result),
            args_excerpt: excerpt(&args_text),
            result_excerpt: excerpt(result),
        });
        while self.recent.len() > WINDOW {
            self.recent.pop_front();
        }
        self.seen += 1;
    }

    /// The most repeated (call, result) pair and the most repeated call in the
    /// window, by count.
    fn repeats(&self) -> (usize, usize) {
        let mut exact = 0;
        let mut same_call = 0;
        for s in &self.recent {
            let calls = self.recent.iter().filter(|o| o.call_sig == s.call_sig);
            let c = calls.clone().count();
            let e = calls.filter(|o| o.result_sig == s.result_sig).count();
            same_call = same_call.max(c);
            exact = exact.max(e);
        }
        (exact, same_call)
    }

    /// The gate's verdict on the current window.
    pub fn gate(&self) -> Gate {
        let (exact, same_call) = self.repeats();
        if exact >= CERTAIN_EXACT {
            Gate::Certain
        } else if exact >= SUSPECT_EXACT || same_call >= SUSPECT_SAME_CALL {
            Gate::Suspect
        } else {
            Gate::Clear
        }
    }

    /// True when the gate is suspicious and Jev has not recently cleared it.
    pub fn should_ask(&self) -> bool {
        if self.gate() == Gate::Clear {
            return false;
        }
        match self.cleared_at {
            Some(at) => self.seen >= at + REASK_AFTER,
            None => true,
        }
    }

    /// Jev looked and said "not stuck": stay quiet for a few calls.
    pub fn mark_cleared(&mut self) {
        self.cleared_at = Some(self.seen);
    }

    /// The window as Jev's state: oldest first, one line per call.
    pub fn state(&self) -> String {
        let mut out = String::from("Recent tool calls, oldest first:\n");
        for (i, s) in self.recent.iter().enumerate() {
            out.push_str(&format!(
                "#{} {}({})\n   -> {}\n",
                i + 1,
                s.name,
                s.args_excerpt,
                s.result_excerpt
            ));
        }
        out
    }

    /// The call repeated most often in the window, for the nudge text.
    fn most_repeated(&self) -> Option<&Step> {
        self.recent.iter().max_by_key(|s| {
            self.recent
                .iter()
                .filter(|o| o.call_sig == s.call_sig)
                .count()
        })
    }

    /// Decide the intervention from the gate and Jev's P(stuck), if any.
    /// Advances the nudge counter and resets the window on action, so the
    /// next intervention needs fresh evidence.
    pub fn act(&mut self, gate: Gate, p_stuck: Option<f64>) -> LoopAction {
        let stuck = match gate {
            // Jev's answer, when there is one, outranks even the exact gate:
            // a CI poll repeats exactly too.
            Gate::Certain => p_stuck.is_none_or(|p| p >= STUCK_THRESHOLD),
            Gate::Suspect => p_stuck.is_some_and(|p| p >= STUCK_THRESHOLD),
            Gate::Clear => false,
        };
        if !stuck {
            return LoopAction::None;
        }
        let what = match self.most_repeated() {
            Some(s) => {
                let n = self
                    .recent
                    .iter()
                    .filter(|o| o.call_sig == s.call_sig)
                    .count();
                format!(
                    "`{}` with the same arguments ({}) {n} times",
                    s.name, s.args_excerpt
                )
            }
            None => "the same tool call over and over".to_string(),
        };
        self.recent.clear();
        self.cleared_at = None;
        if self.nudges >= MAX_NUDGES {
            return LoopAction::Stop(format!(
                "Stopped: the agent kept calling {what} without making progress, even after \
                 being told to change approach. Review the last steps and tell it how to proceed."
            ));
        }
        self.nudges += 1;
        LoopAction::Nudge(format!(
            "[system] You have called {what} in the last few steps and are not \
             making progress — you are repeating yourself. Stop repeating it. Re-read the last \
             result carefully and change approach: try a genuinely different action, look for \
             the root cause somewhere else, or, if you need information or a decision only the \
             user has, call ask_user."
        ))
    }
}

/// The question asked about [`LoopWatch::state`].
pub fn question() -> Value {
    json!({
        "stuck": crate::agent::jev::noul(
            "Below are the most recent tool calls of an AI coding agent, oldest first, \
             each with a short excerpt of its result. Is the agent stuck in a loop: \
             repeating the same or equivalent actions without making progress toward \
             its goal (same calls, same results, no new information gained, no changes \
             in between)?",
            "It is repeating itself without progress",
            "It is making progress: the calls differ meaningfully, results change, or it \
             is legitimately re-running a check after making changes or while waiting \
             for something",
        )
    })
}

#[cfg(test)]
mod tests {
    use super::*;

    fn feed(w: &mut LoopWatch, steps: &[(&str, Value, &str)]) {
        for (n, a, r) in steps {
            w.record(n, a, r);
        }
    }

    fn gradle() -> (&'static str, Value, &'static str) {
        (
            "bash",
            json!({"command": "./gradlew assembleDebug"}),
            "FAILURE: Build failed with an exception. What went wrong: 25.0.4.1",
        )
    }

    #[test]
    fn varied_work_is_clear() {
        let mut w = LoopWatch::default();
        feed(
            &mut w,
            &[
                ("read_file", json!({"path": "a.rs"}), "fn a"),
                ("read_file", json!({"path": "b.rs"}), "fn b"),
                ("grep", json!({"pattern": "Config"}), "src/config.rs:12"),
                ("read_file", json!({"path": "config.rs"}), "struct Config"),
            ],
        );
        assert_eq!(w.gate(), Gate::Clear);
        assert!(!w.should_ask());
    }

    #[test]
    fn three_identical_calls_with_identical_results_are_suspect() {
        let mut w = LoopWatch::default();
        let g = gradle();
        feed(&mut w, &[g.clone(), g.clone(), g.clone()]);
        assert_eq!(w.gate(), Gate::Suspect);
        assert!(w.should_ask());
    }

    #[test]
    fn rerunning_tests_after_edits_is_suspect_but_only_jev_can_call_it() {
        // Same call, different results: exactly the shape of healthy TDD. The
        // gate may look, but without a judge nothing happens.
        let mut w = LoopWatch::default();
        let t = json!({"command": "cargo test"});
        feed(
            &mut w,
            &[
                ("bash", t.clone(), "3 failed"),
                ("edit_file", json!({"path": "a.rs"}), "ok"),
                ("bash", t.clone(), "2 failed"),
                ("edit_file", json!({"path": "b.rs"}), "ok"),
                ("bash", t.clone(), "ok. 12 passed"),
            ],
        );
        assert_eq!(w.gate(), Gate::Suspect);
        assert_eq!(w.act(Gate::Suspect, None), LoopAction::None);
    }

    #[test]
    fn edit_and_revert_with_the_same_failure_is_suspect() {
        // The 2026-09-21 ticket: flip compileSdk 36→35→36, same Gradle error.
        let mut w = LoopWatch::default();
        let g = gradle();
        feed(
            &mut w,
            &[
                ("edit_file", json!({"old": "36", "new": "35"}), "ok"),
                g.clone(),
                ("edit_file", json!({"old": "35", "new": "36"}), "ok"),
                g.clone(),
                ("edit_file", json!({"old": "36", "new": "35"}), "ok"),
                g.clone(),
            ],
        );
        assert_eq!(w.gate(), Gate::Suspect);
    }

    #[test]
    fn five_identical_calls_are_certain_without_a_judge() {
        let mut w = LoopWatch::default();
        let g = gradle();
        for _ in 0..CERTAIN_EXACT {
            w.record(g.0, &g.1, g.2);
        }
        assert_eq!(w.gate(), Gate::Certain);
        assert!(matches!(w.act(Gate::Certain, None), LoopAction::Nudge(_)));
    }

    #[test]
    fn a_jev_no_outranks_even_the_certain_gate() {
        // Five identical polls of a CI run are exactly what the certain gate
        // sees; Jev scored that shape 0.72 live. Its answer wins when it has one.
        let mut w = LoopWatch::default();
        assert_eq!(w.act(Gate::Certain, Some(0.72)), LoopAction::None);
        assert!(matches!(
            w.act(Gate::Certain, Some(0.9)),
            LoopAction::Nudge(_)
        ));
    }

    #[test]
    fn jev_above_the_threshold_nudges_and_below_does_nothing() {
        let mut w = LoopWatch::default();
        assert_eq!(w.act(Gate::Suspect, Some(0.72)), LoopAction::None);
        assert!(matches!(
            w.act(Gate::Suspect, Some(0.96)),
            LoopAction::Nudge(_)
        ));
    }

    #[test]
    fn the_nudge_names_the_repeated_call() {
        let mut w = LoopWatch::default();
        let g = gradle();
        feed(&mut w, &[g.clone(), g.clone(), g.clone()]);
        match w.act(Gate::Suspect, Some(0.95)) {
            LoopAction::Nudge(msg) => {
                assert!(msg.contains("bash"), "{msg}");
                assert!(msg.contains("ask_user"), "{msg}");
            }
            other => panic!("expected a nudge, got {other:?}"),
        }
    }

    #[test]
    fn after_the_nudges_run_out_the_run_stops() {
        let mut w = LoopWatch::default();
        for _ in 0..MAX_NUDGES {
            assert!(matches!(w.act(Gate::Certain, None), LoopAction::Nudge(_)));
        }
        assert!(matches!(w.act(Gate::Certain, None), LoopAction::Stop(_)));
    }

    #[test]
    fn acting_resets_the_window_so_the_next_intervention_needs_fresh_evidence() {
        let mut w = LoopWatch::default();
        let g = gradle();
        feed(&mut w, &[g.clone(), g.clone(), g.clone()]);
        let _ = w.act(Gate::Suspect, Some(0.95));
        assert_eq!(w.gate(), Gate::Clear);
        w.record(g.0, &g.1, g.2);
        assert_eq!(w.gate(), Gate::Clear);
    }

    #[test]
    fn a_cleared_window_is_not_asked_again_until_new_calls_arrive() {
        let mut w = LoopWatch::default();
        let poll = (
            "bash",
            json!({"command": "sleep 30; gh run view 1 --json status"}),
            r#"{"status":"in_progress"}"#,
        );
        feed(&mut w, &[poll.clone(), poll.clone(), poll.clone()]);
        assert!(w.should_ask());
        w.mark_cleared();
        assert!(!w.should_ask());
        for _ in 0..REASK_AFTER {
            w.record(poll.0, &poll.1, poll.2);
        }
        assert!(w.should_ask());
    }

    #[test]
    fn the_window_only_remembers_recent_calls() {
        let mut w = LoopWatch::default();
        let g = gradle();
        feed(&mut w, &[g.clone(), g.clone()]);
        for i in 0..WINDOW {
            w.record("read_file", &json!({"path": format!("f{i}")}), "x");
        }
        w.record(g.0, &g.1, g.2);
        assert_eq!(w.gate(), Gate::Clear);
    }

    #[test]
    fn the_state_lists_calls_oldest_first_with_result_excerpts() {
        let mut w = LoopWatch::default();
        w.record("read_file", &json!({"path": "a.rs"}), "first");
        w.record("bash", &json!({"command": "ls"}), &"y".repeat(5000));
        let s = w.state();
        let a = s.find("read_file").unwrap();
        let b = s.find("bash").unwrap();
        assert!(a < b);
        assert!(s.contains("first"));
        assert!(s.len() < 2000, "results must be excerpted, got {}", s.len());
    }
}
