//! What each gate tells the model when it refuses.
//!
//! The gates are policy in code: the Low-Level Design gate on `tasks_set` in
//! Brain, the quality gate on closing a golden execution goal, golden-task
//! protection, read-before-edit, the write gates (Brain, the git operator) and
//! the spec guard. Prompt v1 also described each of them in prose; prompt v2
//! (lean-harness Phase 5, design rule 4: "the prompt explains, it does not
//! enforce") says only that tools refuse what the session may not do yet and
//! that the error says what to do next. So these texts are now the
//! instruction, and each one names the next step — `tests` pins that, gate by
//! gate. They live together so that one test can read all of them, the way
//! `nudges` keeps the loop's own texts.

/// `edit_file` called in Brain, where it is not offered.
pub const EDIT_IN_BRAIN: &str = "edit_file is not available in Brain mode — it is read-only. \
     Record the intended change in the plan (write_plan) instead.";

/// `edit_file` called in a session that does not edit (the git operator).
pub const EDIT_NOT_IN_SESSION: &str = "edit_file is not available in this session.";

/// A `bash` command that writes files, in a session that does not edit.
pub const BASH_WRITES_NO_EDIT: &str = "This bash command writes files, and this session does not edit \
     files. Bash here is for git, builds, tests and read-only inspection.";

/// A `bash` command that writes files, in Brain.
pub const BASH_WRITES_IN_BRAIN: &str = "This bash command writes files. In Brain mode you cannot mutate \
     — record the change in the plan (write_plan) instead.";

/// A `bash` command outside Brain's read-only allowlist.
pub const BASH_NOT_ALLOWED_IN_BRAIN: &str = "Command not allowed in Brain mode — only read-only allowlisted \
     commands (git status/diff/log, ls, cat, cargo check, ...) run here.";

/// The Low-Level Design gate, before any plan file exists.
pub fn lld_no_plan(tool: &str) -> String {
    format!(
        "{tool} rejected: no plan file exists yet. In Brain mode tasks may only \
         be created after the plan is complete: write the Solution Design via \
         write_plan, then call write_plan again with the full content plus a \
         '## Low-Level Design' section, then retry {tool}."
    )
}

/// The Low-Level Design gate, when the latest plan has no such section.
pub fn lld_missing(tool: &str, plan: &str) -> String {
    format!(
        "{tool} rejected: the most recent plan ({plan}) has no non-empty \
         '## Low-Level Design' section. Research the codebase first (spawn_agents \
         'explore' mode, semantic_search, file reading), then call write_plan again \
         with the FULL plan content including a '## Low-Level Design' section \
         (files/symbols to touch, data flow, APIs/schemas, patterns to reuse) - \
         then retry {tool}."
    )
}

/// Read-before-edit, for a file never read in this session.
pub fn not_read(path: &str) -> String {
    format!("read_file must be called on {path} before editing it")
}

/// The quality gate, with no `run_quality` result in the session.
pub fn quality_missing(goals: &str) -> String {
    format!(
        "Task update rejected: golden task(s) {goals} cannot be marked done without verified \
         quality evidence. Call run_quality first — the harness checks the result \
         mechanically, so claiming the tests pass is not enough."
    )
}

/// The quality gate, when files changed after the last run.
pub fn quality_stale(goals: &str) -> String {
    format!(
        "Task update rejected: golden task(s) {goals} cannot be marked done — the last \
         run_quality result is stale because files changed after it ran. Call run_quality \
         again so the evidence matches the code as it is now."
    )
}

/// The quality gate, when the last run on the current files failed.
pub fn quality_failed(goals: &str, summary: &str, detail: &str) -> String {
    format!(
        "Task update rejected: golden task(s) {goals} cannot be marked done — the last \
         quality run FAILED.\n\n{summary}\n{detail}\nFix the failures, then call \
         run_quality again before marking the goal done."
    )
}

/// Golden protection: ids the model gave the reserved prefix were renamed.
pub fn golden_renamed(count: usize) -> String {
    format!(
        " {count} task(s) used the reserved 'golden-' id prefix and were renamed — \
         golden tasks can only be created by the user via <goal> tags."
    )
}

/// Golden protection: golden tasks left out of a `tasks_set` were put back.
pub fn golden_preserved(count: usize) -> String {
    format!(
        " {count} golden task(s) are mandatory and were preserved — golden tasks cannot be \
         deleted, only completed."
    )
}
