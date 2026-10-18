//! `validate`: prove each task can be failed and can be passed, with no model.
//!
//! A task whose oracle already passes on the start tree measures nothing (every
//! run "solves" it), and one whose oracle fails on the reference measures the
//! oracle. Both have happened in every benchmark that skipped this step, so the
//! runner refuses to count a task until it has been seen to fail and to pass.

use super::materialize::{CommandOutcome, Tree, materialize, run_oracle};
use super::task::Task;
use std::path::Path;

pub(crate) struct Validation {
    pub id: String,
    pub start: Option<CommandOutcome>,
    pub reference: Option<CommandOutcome>,
    /// Why the task could not even be built, when it could not.
    pub error: Option<String>,
}

impl Validation {
    pub fn is_valid(&self) -> bool {
        self.error.is_none()
            && self.start.as_ref().is_some_and(|s| !s.passed)
            && self.reference.as_ref().is_some_and(|r| r.passed)
    }

    fn verdict(&self) -> String {
        if let Some(e) = &self.error {
            return format!("INVALID: {e}");
        }
        match (&self.start, &self.reference) {
            (Some(s), _) if s.passed => "INVALID: oracle passes on the start tree".into(),
            (_, Some(r)) if !r.passed => format!(
                "INVALID: oracle fails on the reference{}",
                if r.timed_out { " (timeout)" } else { "" }
            ),
            _ => "ok".into(),
        }
    }
}

/// Build both trees of `task` under `work` and run the oracle on each.
pub(crate) fn validate_task(task: &Task, work: &Path) -> Validation {
    let mut v = Validation {
        id: task.id.clone(),
        start: None,
        reference: None,
        error: None,
    };
    let start = work.join("start");
    if let Err(e) = materialize(task, &start, Tree::Start) {
        v.error = Some(format!("start tree: {e}"));
        return v;
    }
    v.start = Some(run_oracle(task, &start));
    let reference = work.join("reference");
    if let Err(e) = materialize(task, &reference, Tree::Reference) {
        v.error = Some(format!("reference tree: {e}"));
        return v;
    }
    v.reference = Some(run_oracle(task, &reference));
    v
}

/// One Markdown table row per task, plus a totals line.
pub(crate) fn table(results: &[Validation]) -> String {
    let mut out = String::from(
        "| task | start (must fail) | reference (must pass) | seconds | verdict |\n\
         |---|---|---|---:|---|\n",
    );
    let shown = |o: &Option<CommandOutcome>| match o {
        None => "-".to_string(),
        Some(o) if o.timed_out => "timeout".to_string(),
        Some(o) if o.passed => "pass".to_string(),
        Some(o) => format!(
            "fail ({})",
            o.exit_code.map_or("signal".into(), |c| c.to_string())
        ),
    };
    let mut total = 0.0;
    for v in results {
        let secs =
            v.start.as_ref().map_or(0.0, |o| o.secs) + v.reference.as_ref().map_or(0.0, |o| o.secs);
        total += secs;
        out.push_str(&format!(
            "| {} | {} | {} | {:.1} | {} |\n",
            v.id,
            shown(&v.start),
            shown(&v.reference),
            secs,
            v.verdict()
        ));
    }
    let valid = results.iter().filter(|v| v.is_valid()).count();
    out.push_str(&format!(
        "\n{valid}/{} tasks valid; oracle time {total:.1}s\n",
        results.len()
    ));
    out
}

#[cfg(test)]
mod tests {
    use super::*;

    fn task(oracle: &str) -> Task {
        Task::parse(
            &format!(
                "id: tiny\ntitle: tiny\nprompt: fix it\nsetup:\n  files:\n    a.py: 'X = 1'\n\
                 oracle:\n  command: \"{oracle}\"\n  timeout_s: 20\n\
                 reference:\n  files:\n    a.py: 'X = 2'\n"
            ),
            "tiny",
        )
        .unwrap()
    }

    #[test]
    fn a_task_that_fails_then_passes_is_valid() {
        let work = tempfile::tempdir().unwrap();
        let v = validate_task(&task("python3 -c 'import a; assert a.X == 2'"), work.path());
        assert!(v.is_valid(), "{}", table(&[v]));
    }

    #[test]
    fn an_oracle_that_passes_on_the_start_tree_makes_the_task_invalid() {
        let work = tempfile::tempdir().unwrap();
        let v = validate_task(&task("true"), work.path());
        assert!(!v.is_valid());
        let t = table(&[v]);
        assert!(t.contains("oracle passes on the start tree"), "{t}");
        assert!(t.contains("0/1 tasks valid"), "{t}");
    }

    #[test]
    fn an_oracle_that_never_passes_makes_the_task_invalid() {
        let work = tempfile::tempdir().unwrap();
        let v = validate_task(&task("false"), work.path());
        assert!(!v.is_valid());
        assert!(table(&[v]).contains("oracle fails on the reference"));
    }
}
