//! The text the harness itself puts in front of the model: the loop's
//! nudges, the handoff request, the loop watch's warning.
//!
//! They live in one place so a test can read every one of them
//! (`prompt::tests::no_harness_text_orders_a_deferred_tool`): a nudge that
//! orders a deferred tool makes the model load it, and a load late in a
//! session rewrites the cached prefix in front of the whole conversation
//! (Phase 2 review, 2026-10-18). The handoff kickoffs are composed in
//! `agent::transition` and checked by the same test.

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
pub const BRAIN_EXPLORING: &str = "[system] You have been exploring for several consecutive rounds \
     in Brain mode without making progress on the required deliverables. \
     Brain mode is for planning — you MUST call ask_user to interview \
     the user, then write_plan to create the Solution Design, then \
     write_plan again to add the '## Low-Level Design' section, then \
     tasks_set to create executable tasks. Do not continue exploring \
     until you have gathered the information you need.";

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
        "[system] Golden tasks are still pending: {}. You are now in {} mode (golden cycle {}/{max_cycles}). Read the linked session's plan file and tasks, then resume work on the pending goals — plan or execute what is missing, and only mark a golden task 'done' after verifying the goal is truly met.",
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
