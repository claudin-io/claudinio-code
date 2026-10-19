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
pub const EDIT_NOT_IN_SESSION: &str = "edit_file is not available in this session: it commits \
     and pushes the changes already in the workspace. If something must change first, say so \
     with ask_user.";

/// A `bash` command that writes files, in a session that does not edit.
pub const BASH_WRITES_NO_EDIT: &str = "This bash command writes files, and this session does not edit \
     files. Bash here is for git, builds, tests and read-only inspection; if a file must \
     change first, say so with ask_user.";

/// A `bash` command that writes files, in Brain.
pub const BASH_WRITES_IN_BRAIN: &str = "This bash command writes files. In Brain mode you cannot mutate \
     — record the change in the plan (write_plan) instead.";

/// A `bash` command outside Brain's read-only allowlist.
pub const BASH_NOT_ALLOWED_IN_BRAIN: &str = "Command not allowed in Brain mode — only read-only allowlisted \
     commands (git status/diff/log, ls, cat, cargo check, ...) run here. Read files with \
     read_file, search with grep or the indexed search tools, and put what needs running in \
     the plan.";

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
         '## Low-Level Design' section. Research the codebase first ('explore' \
         subagents, indexed search, file reading), then call write_plan again \
         with the FULL plan content including a '## Low-Level Design' section \
         (files/symbols to touch, data flow, APIs/schemas, patterns to reuse) - \
         then retry {tool}."
    )
}

/// Read-before-edit, for a file never read in this session.
pub fn not_read(path: &str) -> String {
    format!(
        "read_file must be called on {path} before editing it: read the whole file, or the \
         lines you will change, then retry edit_file."
    )
}

/// Read-before-edit, when the file was read but not the text being replaced,
/// and the text is in the file at `line`.
pub fn partial_read_at_line(path: &str, line: usize) -> String {
    format!(
        "{} The text is at line {line}. Call read_file with start_line/end_line \
         covering it, then retry edit_file.",
        partial_read(path)
    )
}

/// Read-before-edit, when the text being replaced is not in the file at all.
pub fn partial_read_not_found(path: &str) -> String {
    format!(
        "{} That exact text was not found in the current file — call read_file on it \
         again, copy old_string verbatim (including indentation), then retry edit_file.",
        partial_read(path)
    )
}

/// Read-before-edit, when the file could not be re-read to locate the text.
pub fn partial_read_unreadable(path: &str, error: &str) -> String {
    format!(
        "{} The file could not be re-read to find it ({error}): check the path, call \
         read_file on the lines you will change, then retry edit_file.",
        partial_read(path)
    )
}

/// `edit_file` with text to replace in a file that does not exist. Not the
/// read gate: `read_file` cannot read it either, and that loop is what every
/// failed run of the clean baseline eval (2026-10-18) was stuck in.
pub fn edit_target_missing(path: &str) -> String {
    format!(
        "{path} does not exist. To create it, retry edit_file with old_string \"\" and the \
         whole file as new_string (missing folders are created)."
    )
}

/// `edit_file` with an empty `old_string` on a file that exists: creating
/// never overwrites.
pub fn create_over_existing(path: &str) -> String {
    format!(
        "{path} already exists, and an empty old_string only creates a new file. Call \
         read_file on it, then retry edit_file with the exact text to replace as old_string."
    )
}

/// A new file named by a relative path, which the containment check resolves
/// against the workspace and the write would resolve against the process's
/// own directory.
pub fn create_needs_absolute_path(path: &str, absolute: &str) -> String {
    format!(
        "{path} is relative; a new file needs an absolute path inside the workspace. Retry \
         edit_file with path {absolute}."
    )
}

/// `edit_file` with an `old_string` that occurs `count` times.
pub fn ambiguous_match(path: &str, count: usize) -> String {
    format!(
        "old_string occurs {count} times in {path}; include more surrounding lines so it \
         matches once, or pass replace_all: true, then retry edit_file."
    )
}

fn partial_read(path: &str) -> String {
    format!(
        "read_file was called on {path}, but you have not read the exact text you're trying \
         to edit."
    )
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
        " {count} task(s) used the reserved 'golden-' id prefix and were renamed to \
         'task-…' — golden tasks can only be created by the user via <goal> tags. \
         Carry on with the renamed ids; if a goal must be golden, ask the user to add a <goal>."
    )
}

/// Golden protection: golden tasks left out of a `tasks_set` were put back.
pub fn golden_preserved(count: usize) -> String {
    format!(
        " {count} golden task(s) are mandatory and were preserved — golden tasks cannot be \
         deleted, only completed: keep them in the list and update their status."
    )
}

/// Every refusal above, by name, rendered with sample arguments: for the
/// actionability test below and for
/// `prompt::tests::no_harness_text_orders_a_deferred_tool`. Kept complete by
/// `tests::every_gate_text_is_in_all`, which reads this file.
#[cfg(test)]
pub fn all() -> Vec<(&'static str, String)> {
    vec![
        ("EDIT_IN_BRAIN", EDIT_IN_BRAIN.to_string()),
        ("EDIT_NOT_IN_SESSION", EDIT_NOT_IN_SESSION.to_string()),
        ("BASH_WRITES_NO_EDIT", BASH_WRITES_NO_EDIT.to_string()),
        ("BASH_WRITES_IN_BRAIN", BASH_WRITES_IN_BRAIN.to_string()),
        (
            "BASH_NOT_ALLOWED_IN_BRAIN",
            BASH_NOT_ALLOWED_IN_BRAIN.to_string(),
        ),
        ("lld_no_plan", lld_no_plan("tasks_set")),
        (
            "lld_missing",
            lld_missing("tasks_set", ".claudinio/plans/p.md"),
        ),
        ("not_read", not_read("/w/src/a.rs")),
        (
            "partial_read_at_line",
            partial_read_at_line("/w/src/a.rs", 10),
        ),
        (
            "partial_read_not_found",
            partial_read_not_found("/w/src/a.rs"),
        ),
        (
            "partial_read_unreadable",
            partial_read_unreadable("/w/src/a.rs", "permission denied"),
        ),
        ("edit_target_missing", edit_target_missing("/w/src/new.rs")),
        ("create_over_existing", create_over_existing("/w/src/a.rs")),
        (
            "create_needs_absolute_path",
            create_needs_absolute_path("src/new.rs", "/w/src/new.rs"),
        ),
        ("ambiguous_match", ambiguous_match("/w/src/a.rs", 2)),
        ("quality_missing", quality_missing("golden-g-1")),
        ("quality_stale", quality_stale("golden-g-1")),
        (
            "quality_failed",
            quality_failed("golden-g-1", "1 failing", "detail"),
        ),
        ("golden_renamed", golden_renamed(1)),
        ("golden_preserved", golden_preserved(1)),
    ]
}

#[cfg(test)]
mod tests {
    use super::*;

    /// The step each refusal must name, because since prompt v2 nothing else
    /// tells the model what to do when it hits that gate. Case-blind.
    const NEXT_STEP: &[(&str, &[&str])] = &[
        // Brain plans; the edit belongs in the plan.
        ("EDIT_IN_BRAIN", &["write_plan"]),
        // The git operator commits what is there; a needed change is the user's call.
        ("EDIT_NOT_IN_SESSION", &["ask_user"]),
        ("BASH_WRITES_NO_EDIT", &["ask_user"]),
        ("BASH_WRITES_IN_BRAIN", &["write_plan"]),
        // What to use instead of the refused command.
        ("BASH_NOT_ALLOWED_IN_BRAIN", &["read_file", "git status"]),
        (
            "lld_no_plan",
            &["write_plan", "## low-level design", "then retry tasks_set"],
        ),
        (
            "lld_missing",
            &["write_plan", "## low-level design", "then retry tasks_set"],
        ),
        ("not_read", &["read_file", "then retry edit_file"]),
        // Read the lines that hold the text, or the file again, then edit.
        (
            "partial_read_at_line",
            &[
                "line 10",
                "read_file with start_line/end_line",
                "then retry edit_file",
            ],
        ),
        (
            "partial_read_not_found",
            &[
                "read_file",
                "copy old_string verbatim",
                "then retry edit_file",
            ],
        ),
        (
            "partial_read_unreadable",
            &["permission denied", "read_file", "then retry edit_file"],
        ),
        // Creating a file is edit_file with an empty old_string, never a shell write.
        (
            "edit_target_missing",
            &["does not exist", "retry edit_file with old_string \"\""],
        ),
        (
            "create_over_existing",
            &["already exists", "read_file", "then retry edit_file"],
        ),
        (
            "create_needs_absolute_path",
            &["absolute", "retry edit_file with path /w/src/new.rs"],
        ),
        (
            "ambiguous_match",
            &[
                "occurs 2 times",
                "include more surrounding lines",
                "replace_all: true",
                "then retry edit_file",
            ],
        ),
        ("quality_missing", &["call run_quality"]),
        ("quality_stale", &["call run_quality"]),
        ("quality_failed", &["fix the failures", "call run_quality"]),
        // v1's golden block said "keep them in the list and update their
        // status"; v2 leaves that to the note `tasks_set` returns.
        (
            "golden_renamed",
            &[
                "Carry on with the renamed ids",
                "ask the user to add a <goal>",
            ],
        ),
        (
            "golden_preserved",
            &["keep them in the list", "update their status"],
        ),
    ];

    #[test]
    fn every_gate_refusal_says_what_to_do_next() {
        let all = all();
        for (name, steps) in NEXT_STEP {
            let text = &all
                .iter()
                .find(|(n, _)| n == name)
                .unwrap_or_else(|| panic!("{name} is not in gate_text::all"))
                .1
                .to_lowercase();
            for step in *steps {
                assert!(text.contains(step), "{name} does not say {step:?}: {text}");
            }
        }
        assert_eq!(
            all.len(),
            NEXT_STEP.len(),
            "every refusal needs its next step pinned"
        );
    }

    /// The finish-line check and the spec guard keep their texts beside
    /// their logic (`nudges`, `quality::spec`); they are instructions too.
    #[test]
    fn the_finish_line_and_the_spec_guard_say_what_to_do_next() {
        let finish = crate::agent::nudges::quality_failed("1 failing", "d", 1, 3).to_lowercase();
        assert!(finish.contains("fix the cause") && finish.contains("call run_quality again"));
        let spec = crate::quality::spec::edit_refusal("features/a.feature").to_lowercase();
        assert!(spec.contains("ask the user"), "{spec}");
    }

    // The guard is only as wide as `all()`; a refusal added here and left
    // out of it would never be checked, so the list is compared with the
    // file itself.
    #[test]
    fn every_gate_text_is_in_all() {
        let source = include_str!("gate_text.rs");
        let production = source.split("#[cfg(test)]").next().unwrap();
        let declared: Vec<&str> = production
            .lines()
            .filter_map(|l| {
                l.strip_prefix("pub const ")
                    .and_then(|r| r.split_once(':'))
                    .or_else(|| l.strip_prefix("pub fn ").and_then(|r| r.split_once('(')))
            })
            .map(|(name, _)| name)
            .collect();
        let listed: Vec<&str> = all().iter().map(|(n, _)| *n).collect();
        assert_eq!(declared, listed);
    }
}
