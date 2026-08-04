use serde::Deserialize;
use std::path::Path;

use crate::agent::persist::TaskItem;

/// Return the current list of tasks from the session JSONL.
pub fn execute_get(ctx: &crate::agent::tools::ToolContext) -> Result<String, String> {
    let path = ctx
        .session_store_path
        .as_ref()
        .ok_or("session_store_path not set")?;
    let tasks = crate::agent::persist::load_last_tasks(Path::new(path))?;
    Ok(serde_json::to_string_pretty(&tasks).unwrap_or_else(|_| "[]".into()))
}

/// Replace all tasks with a new list (appends to the session JSONL).
#[derive(Deserialize)]
pub struct SetTasksArgs {
    pub tasks: Vec<TaskItem>,
}

pub fn execute_set(
    args: SetTasksArgs,
    ctx: &crate::agent::tools::ToolContext,
) -> Result<String, String> {
    check_brain_lld_gate(ctx)?;
    let path = ctx
        .session_store_path
        .as_ref()
        .ok_or("session_store_path not set")?;
    let prev = crate::agent::persist::load_last_tasks(Path::new(path)).unwrap_or_default();
    let (incoming, renamed) = strip_forged_golden_ids(&prev, args.tasks);
    let (merged, preserved) = merge_preserving_golden(&prev, incoming);
    check_quality_gate(&prev, &merged, ctx)?;
    crate::agent::persist::append_tasks(Path::new(path), &merged)?;
    let mut note = String::new();
    if renamed > 0 {
        note.push_str(&format!(
            " {} task(s) used the reserved 'golden-' id prefix and were renamed — \
             golden tasks can only be created by the user via <goal> tags.",
            renamed
        ));
    }
    if preserved > 0 {
        note.push_str(&format!(
            " {} golden task(s) are mandatory and were preserved — golden tasks cannot be \
             deleted, only completed.",
            preserved
        ));
    }
    Ok(format!(
        "Tasks updated: {} task(s) saved.{}",
        merged.len(),
        note
    ))
}

/// Brain-mode gate: tasks may only be created once the most recent plan file
/// carries a non-empty `## Low-Level Design` section, so every task can
/// reference concrete technical detail instead of guesses. No-op in Builder
/// mode or when no mode handle / workspace is attached (tests, aux workflows).
fn check_brain_lld_gate(ctx: &crate::agent::tools::ToolContext) -> Result<(), String> {
    use crate::agent::tools::write_plan::{LLD_HEADING, has_nonempty_section, latest_plan_path};
    if !ctx.is_brain() {
        return Ok(());
    }
    let root = match ctx.workspace_root.as_deref() {
        Some(r) => r,
        None => return Ok(()),
    };
    match latest_plan_path(root, ctx.plan_save_path.as_deref()) {
        None => Err(
            "tasks_set rejected: no plan file exists yet. In Brain mode tasks may only \
                     be created after the plan is complete: write the Solution Design via \
                     write_plan, then call write_plan again with the full content plus a \
                     '## Low-Level Design' section, then retry tasks_set."
                .into(),
        ),
        Some(path) => {
            let content = std::fs::read_to_string(&path)
                .map_err(|e| format!("tasks_set: cannot read plan {}: {e}", path.display()))?;
            if has_nonempty_section(&content, LLD_HEADING) {
                Ok(())
            } else {
                Err(format!(
                    "tasks_set rejected: the most recent plan ({}) has no non-empty \
                     '## Low-Level Design' section. Research the codebase first (spawn_agents \
                     'explore' mode, semantic_search, file reading), then call write_plan again \
                     with the FULL plan content including a '## Low-Level Design' section \
                     (files/symbols to touch, data flow, APIs/schemas, patterns to reuse) - \
                     then retry tasks_set.",
                    path.display()
                ))
            }
        }
    }
}

/// Quality gate: an execution goal may only be closed with verified evidence.
///
/// This is the mechanical half of the golden-goals system. The prompt has
/// always told the model to "run the checks" before marking a goal done, and
/// nothing ever confirmed that it did — a goal could be closed on the model's
/// word. Here `tasks_set` refuses the transition unless the session holds a
/// passing quality run whose digest still matches the worktree, which also
/// rules out run-then-edit-then-close.
///
/// Only the execution half of each goal is gated (`golden-<slug>-1`): the
/// planning half is closed by Brain, which has not written any code and could
/// not make a test suite green if it wanted to.
fn check_quality_gate(
    prev: &[TaskItem],
    merged: &[TaskItem],
    ctx: &crate::agent::tools::ToolContext,
) -> Result<(), String> {
    use crate::agent::tools::quality::{Evidence, current_evidence, rejection_message};

    let closing = newly_done_execution_goals(prev, merged);
    if closing.is_empty() {
        return Ok(());
    }
    match current_evidence(ctx) {
        Evidence::NotRequired => Ok(()),
        Evidence::Current { pass: true, .. } => Ok(()),
        other => Err(rejection_message(ctx, &other, &closing)),
    }
}

/// Ids of execution goals moving to `done` in this call. A goal that was
/// already done stays done without re-verification — the transition is the
/// event worth gating, and re-gating settled goals would deadlock any later
/// `tasks_set` after an unrelated edit.
fn newly_done_execution_goals(prev: &[TaskItem], merged: &[TaskItem]) -> Vec<String> {
    merged
        .iter()
        .filter(|t| is_golden_execution(t) && t.status == "done")
        .filter(|t| {
            prev.iter()
                .find(|p| p.id == t.id)
                .is_none_or(|p| p.status != "done")
        })
        .map(|t| t.id.clone())
        .collect()
}

/// The execute half of a goal pair. `create_golden_tasks` mints `-0` (plan)
/// and `-1` (execute) for every `<goal>`; the UI already derives its label
/// from that same suffix.
pub fn is_golden_execution(task: &TaskItem) -> bool {
    is_golden(task) && task.id.ends_with("-1")
}

/// Strip the reserved `golden-` prefix from any incoming task id that isn't
/// already a known golden task, so the model can't mint new mandatory goals
/// by imitating the id convention in `tasks_set`. Ids that already exist as
/// golden tasks in `prev` (legitimate status updates) pass through untouched.
/// Returns the sanitized list and how many ids were renamed.
fn strip_forged_golden_ids(prev: &[TaskItem], incoming: Vec<TaskItem>) -> (Vec<TaskItem>, usize) {
    use std::collections::HashSet;
    let known_golden: HashSet<&str> = prev
        .iter()
        .filter(|t| is_golden(t))
        .map(|t| t.id.as_str())
        .collect();
    let mut renamed = 0;
    let sanitized = incoming
        .into_iter()
        .map(|mut t| {
            if is_golden(&t) && !known_golden.contains(t.id.as_str()) {
                t.id = format!(
                    "task-{}",
                    &t.id[crate::agent::session::GOLDEN_TASK_PREFIX.len()..]
                );
                renamed += 1;
            }
            t
        })
        .collect();
    (sanitized, renamed)
}

/// Merge an incoming task list with the previous snapshot, guaranteeing that
/// no golden task is ever dropped. Any golden task present in `prev` whose id
/// is absent from `incoming` is re-injected (preserving its prior state).
/// Golden tasks the model *did* include pass through unchanged, so status
/// updates (todo→doing→done) still work. Returns the merged list and the
/// number of golden tasks that had to be re-injected.
pub fn merge_preserving_golden(
    prev: &[TaskItem],
    incoming: Vec<TaskItem>,
) -> (Vec<TaskItem>, usize) {
    use std::collections::HashSet;
    let incoming_ids: HashSet<&str> = incoming.iter().map(|t| t.id.as_str()).collect();
    let missing: Vec<TaskItem> = prev
        .iter()
        .filter(|t| is_golden(t) && !incoming_ids.contains(t.id.as_str()))
        .cloned()
        .collect();
    let preserved = missing.len();
    // Re-injected golden tasks go first so they stay prominent in the UI.
    let mut result = missing;
    result.extend(incoming);
    (result, preserved)
}

/// Create golden tasks from parsed <goal> tags.
/// Each goal generates two golden tasks: one for planning, one for execution.
pub fn create_golden_tasks(goals: &[String]) -> Vec<TaskItem> {
    let mut tasks = Vec::new();
    for goal in goals {
        let slug = slugify(goal);
        // Title carries the raw goal text only; the phase lives in the id
        // suffix (-0 plan, -1 execute). The UI localizes the visible label
        // from those two facts, so no user-facing language is baked in here.
        // The description is model-facing (tasks_get) and stays English.
        tasks.push(TaskItem {
            id: format!("golden-{}-0", slug),
            title: goal.clone(),
            description: format!("Create a detailed plan to achieve the goal: {}", goal),
            journal: Vec::new(),
            status: "todo".to_string(),
        });
        tasks.push(TaskItem {
            id: format!("golden-{}-1", slug),
            title: goal.clone(),
            description: format!("Execute the plan and verify the goal is met: {}", goal),
            journal: Vec::new(),
            status: "todo".to_string(),
        });
    }
    tasks
}

/// Check if a task is a golden task (id starts with the golden prefix).
pub fn is_golden(task: &TaskItem) -> bool {
    task.id
        .starts_with(crate::agent::session::GOLDEN_TASK_PREFIX)
}

/// Get all golden tasks that are not yet 'done'.
pub fn golden_tasks_remaining(tasks: &[TaskItem]) -> Vec<&TaskItem> {
    tasks
        .iter()
        .filter(|t| is_golden(t) && t.status != "done")
        .collect()
}

/// Get IDs of golden tasks that are not yet 'done'.
pub fn golden_pending_ids(tasks: &[TaskItem]) -> Vec<String> {
    golden_tasks_remaining(tasks)
        .into_iter()
        .map(|t| t.id.clone())
        .collect()
}

/// Create a simple slug from a string: lowercase, replace non-alphanumeric with hyphens, truncate to 40 chars.
pub fn slugify(s: &str) -> String {
    let slug: String = s
        .to_lowercase()
        .chars()
        .map(|c| {
            if c.is_alphanumeric() || c == '-' || c == '_' {
                c
            } else {
                // whitespace and punctuation alike collapse to a hyphen
                '-'
            }
        })
        .collect();
    let mut result = String::new();
    let mut prev_hyphen = false;
    for c in slug.chars() {
        if c == '-' {
            if !prev_hyphen {
                result.push(c);
            }
            prev_hyphen = true;
        } else {
            result.push(c);
            prev_hyphen = false;
        }
    }
    let result = result.trim_matches('-').to_string();
    if result.len() > 40 {
        result[..40].to_string()
    } else {
        result
    }
}

#[cfg(test)]
mod lld_gate_tests {
    use super::*;
    use crate::agent::session::{ModeCtl, ModeOrigin, SessionMode};
    use lru::LruCache;
    use std::sync::Arc;

    /// Tempdir workspace with a session JSONL; `mode` = None disables the gate.
    fn ctx_for(
        name: &str,
        mode: Option<SessionMode>,
    ) -> (crate::agent::tools::ToolContext, std::path::PathBuf) {
        let root = std::env::temp_dir().join(format!("lld-gate-{name}-{}", std::process::id()));
        std::fs::create_dir_all(&root).unwrap();
        let store = root.join("session.jsonl");
        std::fs::write(&store, "").unwrap();
        let ctx = crate::agent::tools::ToolContext {
            db_path: None,
            lsp_manager: None,
            workspace_root: Some(root.to_string_lossy().to_string()),
            embedding_model: Arc::new(tokio::sync::Mutex::new(None)),
            session_store_path: Some(store.to_string_lossy().to_string()),
            read_tracker: Arc::new(tokio::sync::Mutex::new(
                crate::agent::tools::ReadTracker::default(),
            )),
            interrupt: None,
            agent_config: None,
            plan_save_path: None,
            base_commit: None,
            auto_approve_git: false,
            mcp: None,
            mode_ctl: mode.map(|m| Arc::new(ModeCtl::new(m, ModeOrigin::Human))),
            index_progress: None,
            records_cache: Arc::new(std::sync::Mutex::new(LruCache::new(
                std::num::NonZeroUsize::new(1).unwrap(),
            ))),
        };
        (ctx, root)
    }

    fn one_task() -> SetTasksArgs {
        SetTasksArgs {
            tasks: vec![TaskItem {
                id: "t1".into(),
                title: "t".into(),
                description: "d".into(),
                journal: vec![],
                status: "todo".into(),
            }],
        }
    }

    fn write_plan_file(root: &std::path::Path, content: &str) {
        let dir = root.join(".claudinio").join("plans");
        std::fs::create_dir_all(&dir).unwrap();
        std::fs::write(dir.join("2026-07-14_plan.md"), content).unwrap();
    }

    #[test]
    fn brain_without_plan_rejected() {
        let (ctx, root) = ctx_for("no-plan", Some(SessionMode::Brain));
        let err = execute_set(one_task(), &ctx).unwrap_err();
        assert!(err.contains("Low-Level Design"), "unexpected error: {err}");
        std::fs::remove_dir_all(&root).ok();
    }

    #[test]
    fn brain_plan_without_lld_rejected() {
        let (ctx, root) = ctx_for("no-lld", Some(SessionMode::Brain));
        write_plan_file(&root, "# Plan\n## Solution Design\nagreed stuff\n");
        let err = execute_set(one_task(), &ctx).unwrap_err();
        assert!(err.contains("Low-Level Design"), "unexpected error: {err}");
        std::fs::remove_dir_all(&root).ok();
    }

    #[test]
    fn brain_plan_with_lld_accepted() {
        let (ctx, root) = ctx_for("with-lld", Some(SessionMode::Brain));
        write_plan_file(
            &root,
            "# Plan\n## Solution Design\nagreed\n## Low-Level Design\nsrc/foo.rs: add bar()\n",
        );
        let res = execute_set(one_task(), &ctx);
        assert!(res.is_ok(), "unexpected error: {res:?}");
        std::fs::remove_dir_all(&root).ok();
    }

    #[test]
    fn builder_mode_not_gated() {
        let (ctx, root) = ctx_for("builder", Some(SessionMode::Builder));
        let res = execute_set(one_task(), &ctx);
        assert!(res.is_ok(), "unexpected error: {res:?}");
        std::fs::remove_dir_all(&root).ok();
    }

    #[test]
    fn no_mode_handle_not_gated() {
        let (ctx, root) = ctx_for("no-handle", None);
        let res = execute_set(one_task(), &ctx);
        assert!(res.is_ok(), "unexpected error: {res:?}");
        std::fs::remove_dir_all(&root).ok();
    }
}

#[cfg(test)]
mod quality_gate_tests {
    use super::*;
    use crate::agent::persist::QualityRunInfo;

    /// A workspace whose `.claudinio.json` carries `quality_json`, plus a
    /// session JSONL seeded with `prev` tasks.
    fn setup(
        name: &str,
        quality_json: &str,
        prev: &[TaskItem],
    ) -> (crate::agent::tools::ToolContext, std::path::PathBuf) {
        let root = std::env::temp_dir().join(format!("cq-tasks-{name}-{}", std::process::id()));
        std::fs::remove_dir_all(&root).ok();
        std::fs::create_dir_all(&root).unwrap();
        std::fs::write(root.join(".claudinio.json"), quality_json).unwrap();
        std::fs::write(root.join("src.txt"), "code").unwrap();
        // Where sessions actually live: appending records must not itself
        // invalidate the evidence the gate is about to read.
        std::fs::create_dir_all(root.join(".claudinio/sessions")).unwrap();
        let store = root.join(".claudinio/sessions/s.jsonl");
        std::fs::write(&store, "").unwrap();
        if !prev.is_empty() {
            crate::agent::persist::append_tasks(&store, prev).unwrap();
        }
        let mut ctx = crate::agent::tools::tests_support::ctx();
        ctx.workspace_root = Some(root.to_string_lossy().to_string());
        ctx.session_store_path = Some(store.to_string_lossy().to_string());
        (ctx, root)
    }

    fn task(id: &str, status: &str) -> TaskItem {
        TaskItem {
            id: id.into(),
            title: "goal".into(),
            description: "d".into(),
            journal: vec![],
            status: status.into(),
        }
    }

    fn record_run(ctx: &crate::agent::tools::ToolContext, root: &std::path::Path, pass: bool) {
        let store = ctx.session_store_path.clone().unwrap();
        crate::agent::persist::append_quality_run(
            std::path::Path::new(&store),
            &QualityRunInfo {
                digest: crate::quality::evidence::workspace_digest(root),
                pass,
                summary: if pass {
                    "3 passed, 0 failed"
                } else {
                    "2 passed, 1 failed"
                }
                .into(),
                report: String::new(),
                trigger: "tool".into(),
                ts: 1,
            },
        )
        .unwrap();
    }

    fn close_goal(ctx: &crate::agent::tools::ToolContext) -> Result<String, String> {
        execute_set(
            SetTasksArgs {
                tasks: vec![task("golden-g-0", "done"), task("golden-g-1", "done")],
            },
            ctx,
        )
    }

    #[test]
    fn closing_an_execution_goal_without_a_run_is_rejected() {
        let (ctx, root) = setup(
            "noevidence",
            "{}",
            &[task("golden-g-0", "done"), task("golden-g-1", "doing")],
        );
        let err = close_goal(&ctx).unwrap_err();
        assert!(err.contains("run_quality"), "{err}");
        assert!(err.contains("golden-g-1"), "{err}");
        std::fs::remove_dir_all(&root).ok();
    }

    #[test]
    fn closing_an_execution_goal_with_fresh_green_evidence_is_allowed() {
        let (ctx, root) = setup(
            "green",
            "{}",
            &[task("golden-g-0", "done"), task("golden-g-1", "doing")],
        );
        record_run(&ctx, &root, true);
        assert!(close_goal(&ctx).is_ok());
        std::fs::remove_dir_all(&root).ok();
    }

    #[test]
    fn editing_after_a_green_run_makes_the_evidence_stale_and_blocks_the_close() {
        // The run-then-edit-then-close path. Without the digest check this is
        // exactly how a broken change would ship with a green report attached.
        let (ctx, root) = setup(
            "stale",
            "{}",
            &[task("golden-g-0", "done"), task("golden-g-1", "doing")],
        );
        record_run(&ctx, &root, true);
        std::fs::write(root.join("src.txt"), "code, changed after the run").unwrap();
        let err = close_goal(&ctx).unwrap_err();
        assert!(err.contains("stale"), "{err}");
        std::fs::remove_dir_all(&root).ok();
    }

    #[test]
    fn a_failing_run_blocks_the_close() {
        let (ctx, root) = setup(
            "red",
            "{}",
            &[task("golden-g-0", "done"), task("golden-g-1", "doing")],
        );
        record_run(&ctx, &root, false);
        let err = close_goal(&ctx).unwrap_err();
        assert!(err.contains("FAILED"), "{err}");
        std::fs::remove_dir_all(&root).ok();
    }

    #[test]
    fn the_planning_half_of_a_goal_is_not_gated() {
        // Brain closes `-0` without having written a line of code; demanding a
        // green suite from it would deadlock every planning session.
        let (ctx, root) = setup("planhalf", "{}", &[task("golden-g-0", "doing")]);
        let res = execute_set(
            SetTasksArgs {
                tasks: vec![task("golden-g-0", "done")],
            },
            &ctx,
        );
        assert!(res.is_ok(), "{res:?}");
        std::fs::remove_dir_all(&root).ok();
    }

    #[test]
    fn ordinary_tasks_are_not_gated() {
        let (ctx, root) = setup("ordinary", "{}", &[task("task-1", "doing")]);
        let res = execute_set(
            SetTasksArgs {
                tasks: vec![task("task-1", "done")],
            },
            &ctx,
        );
        assert!(res.is_ok(), "{res:?}");
        std::fs::remove_dir_all(&root).ok();
    }

    #[test]
    fn a_workspace_that_enforces_nothing_is_not_gated() {
        let (ctx, root) = setup(
            "observation",
            r#"{"quality":{"enforced_layers":[]}}"#,
            &[task("golden-g-0", "done"), task("golden-g-1", "doing")],
        );
        assert!(close_goal(&ctx).is_ok());
        std::fs::remove_dir_all(&root).ok();
    }

    #[test]
    fn an_already_closed_goal_can_be_resubmitted_after_later_edits() {
        // tasks_set is a full replace, so a settled goal is re-sent on every
        // later call. Re-gating it would make the list unwritable the moment
        // the agent touched another file.
        let (ctx, root) = setup(
            "settled",
            "{}",
            &[task("golden-g-0", "done"), task("golden-g-1", "done")],
        );
        std::fs::write(root.join("src.txt"), "changed since the goal closed").unwrap();
        let res = execute_set(
            SetTasksArgs {
                tasks: vec![
                    task("golden-g-0", "done"),
                    task("golden-g-1", "done"),
                    task("task-new", "doing"),
                ],
            },
            &ctx,
        );
        assert!(res.is_ok(), "{res:?}");
        std::fs::remove_dir_all(&root).ok();
    }

    #[test]
    fn the_rejected_call_does_not_persist_the_task_list() {
        // A rejected tasks_set must leave no trace, or a retry would see the
        // goal already marked done and skip the gate entirely.
        let (ctx, root) = setup(
            "atomic",
            "{}",
            &[task("golden-g-0", "done"), task("golden-g-1", "doing")],
        );
        close_goal(&ctx).unwrap_err();
        let store = ctx.session_store_path.clone().unwrap();
        let saved = crate::agent::persist::load_last_tasks(std::path::Path::new(&store)).unwrap();
        let goal = saved.iter().find(|t| t.id == "golden-g-1").unwrap();
        assert_eq!(goal.status, "doing", "the rejected close must not stick");
        std::fs::remove_dir_all(&root).ok();
    }
}

#[cfg(test)]
mod golden_tests {
    use super::*;
    use crate::agent::persist::TaskItem;

    #[test]
    fn test_create_golden_tasks() {
        let goals = vec!["code coverage in 80%".to_string()];
        let tasks = create_golden_tasks(&goals);
        assert_eq!(tasks.len(), 2);
        assert!(tasks[0].id.starts_with("golden-"));
        assert!(tasks[1].id.starts_with("golden-"));
        assert_eq!(tasks[0].status, "todo");
        assert_eq!(tasks[1].status, "todo");
    }

    #[test]
    fn test_is_golden() {
        let golden = TaskItem {
            id: "golden-test-0".into(),
            title: "test".into(),
            description: "".into(),
            journal: vec![],
            status: "todo".into(),
        };
        let normal = TaskItem {
            id: "normal-task".into(),
            title: "test".into(),
            description: "".into(),
            journal: vec![],
            status: "todo".into(),
        };
        assert!(is_golden(&golden));
        assert!(!is_golden(&normal));
    }

    #[test]
    fn test_golden_tasks_remaining() {
        let tasks = vec![
            TaskItem {
                id: "golden-a-0".into(),
                title: "test".into(),
                description: "".into(),
                journal: vec![],
                status: "todo".into(),
            },
            TaskItem {
                id: "golden-a-1".into(),
                title: "test".into(),
                description: "".into(),
                journal: vec![],
                status: "done".into(),
            },
            TaskItem {
                id: "normal-task".into(),
                title: "test".into(),
                description: "".into(),
                journal: vec![],
                status: "todo".into(),
            },
        ];
        let remaining = golden_tasks_remaining(&tasks);
        assert_eq!(remaining.len(), 1);
        assert_eq!(remaining[0].id, "golden-a-0");
    }

    #[test]
    fn test_slugify() {
        assert_eq!(slugify("Code Coverage in 80%"), "code-coverage-in-80");
        assert_eq!(slugify("Hello World"), "hello-world");
        assert!(
            slugify("a very long string with many many characters that exceeds forty chars total")
                .len()
                <= 40
        );
    }

    fn task(id: &str, status: &str) -> TaskItem {
        TaskItem {
            id: id.into(),
            title: "t".into(),
            description: "".into(),
            journal: vec![],
            status: status.into(),
        }
    }

    #[test]
    fn test_merge_preserving_golden_blocks_drop() {
        let prev = vec![task("golden-a-0", "todo"), task("normal", "doing")];
        // Model tries to replace the list, omitting the golden task.
        let incoming = vec![task("new-task", "todo")];
        let (merged, preserved) = merge_preserving_golden(&prev, incoming);
        assert_eq!(preserved, 1);
        let golden: Vec<_> = merged.iter().filter(|t| t.id == "golden-a-0").collect();
        assert_eq!(golden.len(), 1);
        assert_eq!(golden[0].status, "todo");
        assert!(merged.iter().any(|t| t.id == "new-task"));
    }

    #[test]
    fn test_merge_preserving_golden_status_update_passes_through() {
        let prev = vec![task("golden-a-0", "todo")];
        // Model keeps the golden task and marks it done — allowed, no re-injection.
        let incoming = vec![task("golden-a-0", "done")];
        let (merged, preserved) = merge_preserving_golden(&prev, incoming);
        assert_eq!(preserved, 0);
        let golden: Vec<_> = merged.iter().filter(|t| t.id == "golden-a-0").collect();
        assert_eq!(golden.len(), 1);
        assert_eq!(golden[0].status, "done");
    }

    #[test]
    fn test_merge_preserving_golden_no_golden() {
        let prev = vec![task("normal-1", "todo")];
        let incoming = vec![task("normal-2", "todo")];
        let (merged, preserved) = merge_preserving_golden(&prev, incoming.clone());
        assert_eq!(preserved, 0);
        assert_eq!(merged.len(), 1);
        assert_eq!(merged[0].id, "normal-2");
    }

    #[test]
    fn test_strip_forged_golden_ids_renames_unknown() {
        let prev = vec![task("normal", "todo")];
        let incoming = vec![task("golden-fake-0", "todo")];
        let (sanitized, renamed) = strip_forged_golden_ids(&prev, incoming);
        assert_eq!(renamed, 1);
        assert_eq!(sanitized[0].id, "task-fake-0");
    }

    #[test]
    fn test_strip_forged_golden_ids_allows_known() {
        let prev = vec![task("golden-a-0", "todo")];
        let incoming = vec![task("golden-a-0", "done")];
        let (sanitized, renamed) = strip_forged_golden_ids(&prev, incoming);
        assert_eq!(renamed, 0);
        assert_eq!(sanitized[0].id, "golden-a-0");
        assert_eq!(sanitized[0].status, "done");
    }

    #[test]
    fn test_golden_pending_ids() {
        let tasks = vec![
            TaskItem {
                id: "golden-x-0".into(),
                title: "t".into(),
                description: "".into(),
                journal: vec![],
                status: "todo".into(),
            },
            TaskItem {
                id: "golden-x-1".into(),
                title: "t".into(),
                description: "".into(),
                journal: vec![],
                status: "done".into(),
            },
        ];
        let ids = golden_pending_ids(&tasks);
        assert_eq!(ids, vec!["golden-x-0"]);
    }
}
