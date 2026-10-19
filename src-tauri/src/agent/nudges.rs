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

/// The one reminder before the harness writes the Implementation Log itself.
pub const FINALIZE_PLAN: &str = "[system] All goals are met. Before finishing, call \
     finalize_plan with a journal of your findings — it records the \
     changed files and commits into the plan for future reference. \
     This is the required last step.";

/// Brain mode explored for several rounds without a deliverable.
/// Since lean-harness Phase 5 it no longer orders an interview (decision D5):
/// a decision that is the user's goes to `ask_user`; otherwise the plan is due.
pub const BRAIN_EXPLORING: &str = "[system] You have been exploring for several consecutive rounds \
     in Brain mode without making progress on the deliverables. Brain mode is for \
     planning: if a decision only the user can make is still open, ask it with \
     ask_user (independent questions in one call); otherwise write the Solution \
     Design with write_plan, then write_plan again to add the '## Low-Level Design' \
     section, then tasks_set to create executable tasks. Explore further only for \
     a fact the plan is missing.";

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

/// The first message of the session a golden flip hands over to.
pub fn golden_flip(pending: &[String], mode: &str, cycle: u32, max_cycles: usize) -> String {
    format!(
        "[system] Golden tasks are still pending: {}. You are now in {} mode (golden cycle {}/{max_cycles}). The task list, pending goals included, is the `<tasks>` block at the end of this message; read the linked session's plan file, then resume work on the pending goals — plan or execute what is missing, and only mark a golden task 'done' after verifying the goal is truly met.",
        pending.join(", "),
        mode,
        cycle,
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

/// The first message of the Builder session a plan approval opens
/// (`exit_plan_mode`, `HandoffReason::PlanExecution`).
pub const PLAN_APPROVED: &str = "Plan approved. Read the plan and execute the tasks.";

/// `exit_plan_mode`'s result when the plan is approved: this session ends
/// and the Builder session above takes over.
pub const PLAN_APPROVED_RESULT: &str =
    "Plan approved. Switching to Builder mode via new session... End your turn.";

/// Every fixed text above, by name, for the test that checks them all
/// (`prompt::tests::no_harness_text_orders_a_deferred_tool`). Kept complete
/// by [`tests::every_text_constant_is_in_all`], which reads this file: a new
/// `pub const` here that is left out fails the suite.
#[cfg(test)]
pub const ALL: &[(&str, &str)] = &[
    ("CUT_OFF", CUT_OFF),
    ("EMPTY_REPLY", EMPTY_REPLY),
    ("ANNOUNCED_NOT_TAKEN", ANNOUNCED_NOT_TAKEN),
    ("FINALIZE_PLAN", FINALIZE_PLAN),
    ("BRAIN_EXPLORING", BRAIN_EXPLORING),
    ("HANDOFF_REQUEST", HANDOFF_REQUEST),
    ("HANDOFF_RETRY", HANDOFF_RETRY),
    ("PLAN_APPROVED", PLAN_APPROVED),
    ("PLAN_APPROVED_RESULT", PLAN_APPROVED_RESULT),
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

    // Decision D5 (lean-harness Phase 5): the Brain interview is adaptive.
    // The nudge Brain gets after exploring too long must not bring back the
    // mandatory interview the prompt dropped: it asks only when a decision
    // that is the user's is open, and otherwise moves on to the plan.
    #[test]
    fn the_brain_exploring_nudge_keeps_the_interview_adaptive() {
        assert!(!BRAIN_EXPLORING.contains("MUST"), "{BRAIN_EXPLORING}");
        assert!(
            !BRAIN_EXPLORING.contains("interview the user"),
            "{BRAIN_EXPLORING}"
        );
        for step in ["ask_user", "write_plan", "## Low-Level Design", "tasks_set"] {
            assert!(BRAIN_EXPLORING.contains(step), "{step}: {BRAIN_EXPLORING}");
        }
    }

    // A golden flip's successor always has a list (the pending goals are
    // tasks), and its first run ends the kickoff with it as a `<tasks>` block.
    // The kickoff points there rather than asking the model to "read the
    // tasks" — the only tool that reads them is `tasks_get`, which is
    // deferred, so an order to read them is a load and a prefix rewrite
    // (Phase 4 review).
    #[test]
    fn the_golden_flip_points_at_the_tasks_block_instead_of_a_read() {
        let msg = golden_flip(&["golden-ship-1".into()], "builder", 1, 3);
        assert!(msg.contains("`<tasks>` block"), "{msg}");
        assert!(!msg.contains("plan file and tasks"), "{msg}");
        assert!(
            msg.contains("golden-ship-1") && msg.contains("1/3"),
            "{msg}"
        );
    }
}
