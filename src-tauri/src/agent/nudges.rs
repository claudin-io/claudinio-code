//! The text the harness itself puts in front of the model: the loop's
//! nudges, the handoff request, the loop watch's warning.
//!
//! They live in one place so a test can read every one of them
//! (`prompt::tests::no_harness_text_orders_a_deferred_tool`): a nudge that
//! orders a deferred tool makes the model load it, and a load late in a
//! session rewrites the cached prefix in front of the whole conversation
//! (Phase 2 review, 2026-10-18). The handoff kickoffs are composed in
//! `agent::transition` and checked by the same test. A fixed text added
//! here joins [`ALL`]; a test fails until it does.

/// The response stopped at the output-token limit before a tool call or a
/// final answer was complete.
pub const CUT_OFF: &str = "[system] Your previous response was cut off at the output token \
     limit before completing a tool call or final answer. Continue from \
     where you stopped, working in smaller steps — if you were emitting \
     a large tool call (e.g. a whole-file edit), split it into several \
     smaller edits.";

/// The response had neither text nor a tool call.
pub const EMPTY_REPLY: &str = "[system] Your previous response was empty (no text and no \
     tool calls). If the task is not finished, continue from \
     where you stopped; otherwise reply with a short final \
     summary of what was done.";

/// The response announced a next step and ended without taking it.
pub const ANNOUNCED_NOT_TAKEN: &str = "[system] Your previous message announced a next step \
     but ended without taking it — no tool call followed. \
     Do not stop here. Continue now and actually perform the \
     action you described: call the appropriate tool (e.g. \
     ask the user, spawn the subagents, read the file, make \
     the edit). If the task is genuinely complete, instead \
     reply with a short final summary of what was done.";

/// The context reached its limit: the next reply is the handoff document.
pub const HANDOFF_REQUEST: &str = "[system] This session's context reached its limit. Your next reply must be \
     ONLY a handoff document for a successor session that will continue this \
     exact work with a fresh context. Structure it with these markdown sections: \
     ## Purpose of next session / ## Current state (what is done, and how it was \
     verified) / ## Key decisions (and why) / ## Pointers (plan file path, key \
     file paths, task ids - references only, never duplicate file contents) / \
     ## In-flight work (the task mid-execution and its exact next step) / \
     ## Next actions. Rules: no secrets, API keys, tokens or personal data; be \
     concise; do not call tools; do not address the user.";

/// The reply to `HANDOFF_REQUEST` was not a handoff document.
pub const HANDOFF_RETRY: &str = "[system] That reply was not a valid handoff document. Reply with \
     ONLY the handoff document, using the exact markdown sections \
     requested above.";

/// The message that sends a run back to its pending golden goals when it
/// tried to end with some still open (single-timeline plan: the same
/// session continues; there is no mode to flip to).
pub fn golden_continue(pending: &[String], cycle: u32, max_cycles: usize) -> String {
    format!(
        "[system] Golden tasks are still pending: {} (golden cycle {cycle}/{max_cycles}). Continue the work on them, and only mark a golden task 'done' after verifying its goal is truly met.",
        pending.join(", "),
    )
}

/// The model tried to finish while the project's own checks fail.
pub fn quality_failed(summary: &str, detail: &str, attempt: u32, max_attempts: u32) -> String {
    format!(
        "[system] You tried to finish, but the project's own checks \
         FAIL against the code as it stands. The goal is not met \
         until they pass.\n\n{summary}\n{detail}\nFix the cause (not the test), \
         then call run_quality again. Attempt {attempt}/{max_attempts}."
    )
}

/// The model ended its turn with tasks it set `doing` in this run (or wrote
/// to while `doing`) still marked `doing` — never one an earlier run left
/// there (`task_recital::still_doing_nudge`). Sent once per run
/// (`GuardState::doing_nudged`): the prompt no longer demands a final
/// `tasks_update` before the answer (lean-harness Phase 4), so the harness
/// checks at the finish line instead, and then lets the run end either way.
pub fn still_doing(tasks: &[crate::agent::persist::TaskItem]) -> String {
    let named: Vec<String> = tasks
        .iter()
        .map(|t| format!("'{}' ({})", t.id, t.title))
        .collect();
    format!(
        "[system] You ended your turn with {} still marked 'doing'. If that work is \
         finished and verified, mark it 'done' with `tasks_update` (several tasks go in \
         one call); if it is not, finish it, or set it back to 'todo' and say what is \
         left. Then give your final answer.",
        named.join(", ")
    )
}

/// The loop watch saw the same call repeated without progress.
pub fn repeating(what: &str) -> String {
    format!(
        "[system] You have called {what} in the last few steps and are not \
         making progress — you are repeating yourself. Stop repeating it. Re-read the last \
         result carefully and change approach: try a genuinely different action, look for \
         the root cause somewhere else, or, if you need information or a decision only the \
         user has, call ask_user."
    )
}

/// Every fixed text above, by name, for the test that checks them all
/// (`prompt::tests::no_harness_text_orders_a_deferred_tool`). Kept complete
/// by [`tests::every_text_constant_is_in_all`], which reads this file: a new
/// `pub const` here that is left out fails the suite.
#[cfg(test)]
pub const ALL: &[(&str, &str)] = &[
    ("CUT_OFF", CUT_OFF),
    ("EMPTY_REPLY", EMPTY_REPLY),
    ("ANNOUNCED_NOT_TAKEN", ANNOUNCED_NOT_TAKEN),
    ("HANDOFF_REQUEST", HANDOFF_REQUEST),
    ("HANDOFF_RETRY", HANDOFF_RETRY),
];

#[cfg(test)]
mod tests {
    use super::*;

    // The guard over harness texts is only as wide as `ALL`. A constant
    // added to this file and not to `ALL` would never be checked, so the
    // list is compared with the file itself.
    #[test]
    fn every_text_constant_is_in_all() {
        let declared: Vec<&str> = include_str!("nudges.rs")
            .lines()
            .filter_map(|l| l.strip_prefix("pub const "))
            .filter_map(|l| l.split_once(": &str"))
            .map(|(name, _)| name)
            .collect();
        assert!(declared.contains(&"CUT_OFF"), "{declared:?}");
        let listed: Vec<&str> = ALL.iter().map(|(name, _)| *name).collect();
        assert_eq!(
            declared, listed,
            "nudges::ALL must list every text constant, in order"
        );
    }

    // The golden loop sends the run back to its pending goals in the same
    // session: the message names them and the cycle, and asks for a check
    // before a goal is closed.
    #[test]
    fn the_golden_continue_names_the_pending_goals_and_the_cycle() {
        let msg = golden_continue(&["golden-ship-1".into()], 1, 3);
        assert!(
            msg.contains("golden-ship-1") && msg.contains("1/3"),
            "{msg}"
        );
        assert!(msg.contains("verifying"), "{msg}");
        assert!(!msg.contains("mode"), "{msg}");
    }
}
