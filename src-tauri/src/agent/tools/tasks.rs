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
    check_brain_lld_gate(ctx, "tasks_set")?;
    let path = ctx
        .session_store_path
        .as_ref()
        .ok_or("session_store_path not set")?;
    let _writing = lock_task_list();
    let prev = crate::agent::persist::load_last_tasks(Path::new(path)).unwrap_or_default();
    let (incoming, renamed) = strip_forged_golden_ids(&prev, args.tasks);
    let incoming = keep_stored_details(&prev, incoming);
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

/// Change one task: its status, its journal, or both.
#[derive(Debug, Deserialize)]
pub struct UpdateTaskArgs {
    pub id: String,
    #[serde(default)]
    pub status: Option<String>,
    /// Entries to append to the task's journal.
    #[serde(default)]
    pub journal: Vec<String>,
}

const TASK_STATUSES: [&str; 3] = ["todo", "doing", "done"];

/// Held from reading the task list to writing it back. Subagents run in
/// parallel and are offered these tools: two of them each closing its own
/// task would otherwise both start from the same list, and the second write
/// would put the first one's task back to `doing`.
static TASK_LIST: std::sync::Mutex<()> = std::sync::Mutex::new(());

fn lock_task_list() -> std::sync::MutexGuard<'static, ()> {
    // A panic while holding it leaves nothing half-written: the list is one
    // appended record. The next writer just goes ahead.
    TASK_LIST.lock().unwrap_or_else(|e| e.into_inner())
}

/// Update a single task in place.
///
/// `tasks_set` is a full replacement, so moving one task from `doing` to
/// `done` meant writing the whole list out again — every title, description
/// and journal — twice per task. Measured on 509 sessions that was 24% of
/// everything the main session wrote, more than its code edits. A delta says
/// the same thing in one line.
///
/// The gates are the ones `tasks_set` goes through, on the list as it would be
/// after the change: a status update is exactly how a goal gets closed, so it
/// must not be a way around them.
///
/// The one-task shorthand of [`execute_updates`]; the dispatcher parses
/// either form (`parse_update_args`) and calls that directly.
#[cfg(test)]
pub fn execute_update(
    args: UpdateTaskArgs,
    ctx: &crate::agent::tools::ToolContext,
) -> Result<String, String> {
    execute_updates(vec![args], ctx)
}

/// The arguments of a `tasks_update` call, in either form: one task (`id`,
/// `status`, `journal`) or several (`updates: [{id, status?, journal?}]`).
/// A call in neither form, or in both at once, is refused with both shapes
/// spelled out, so the retry is the right one.
pub fn parse_update_args(mut args: serde_json::Value) -> Result<Vec<UpdateTaskArgs>, String> {
    const SHAPES: &str = "pass either one task as {\"id\", \"status\"?, \"journal\"?} \
                          or several as {\"updates\": [{\"id\", \"status\"?, \"journal\"?}, ...]}";
    // A model that fills every optional property sends null for the form it
    // does not use; a null is an absent value, not a second form.
    if let Some(fields) = args.as_object_mut() {
        fields.retain(|_, v| !v.is_null());
    }
    // A top-level status or journal beside `updates` names no task of the
    // batch: refused, not silently dropped. An empty journal says nothing.
    let has_id = args.get("id").is_some();
    let single_form = has_id
        || args.get("status").is_some()
        || args
            .get("journal")
            .is_some_and(|j| j.as_array().is_none_or(|a| !a.is_empty()));
    match args.get("updates") {
        Some(_) if single_form => Err(format!("tasks_update: {SHAPES}, not both.")),
        Some(updates) => {
            let items: Vec<UpdateTaskArgs> = serde_json::from_value(updates.clone())
                .map_err(|e| format!("tasks_update: invalid updates ({e}); {SHAPES}."))?;
            if items.is_empty() {
                return Err(format!("tasks_update: updates is empty; {SHAPES}."));
            }
            Ok(items)
        }
        None if has_id => serde_json::from_value(args)
            .map(|one| vec![one])
            .map_err(|e| format!("tasks_update: invalid args ({e}); {SHAPES}.")),
        None => Err(format!("tasks_update: no task named; {SHAPES}.")),
    }
}

/// Apply one or more updates as one change of the list: a model that closes
/// a task and starts the next says so in one call (lean-harness Phase 4: the
/// prompt used to say "never batch", which cost a round per state change).
/// Every item is checked first — the id exists, the status is one of the
/// three — and the quality gate then sees the list as the whole batch leaves
/// it, so a goal cannot be closed on a claim by tucking it beside an ordinary
/// task. If anything fails, nothing is written and the error names the task
/// and the reason. Items apply in the order given; one list record is written
/// for the whole call.
pub fn execute_updates(
    items: Vec<UpdateTaskArgs>,
    ctx: &crate::agent::tools::ToolContext,
) -> Result<String, String> {
    check_brain_lld_gate(ctx, "tasks_update")?;
    let path = ctx
        .session_store_path
        .as_ref()
        .ok_or("session_store_path not set")?;
    let batch = items.len() > 1;
    let none_applied = if batch {
        " The batch was refused: nothing was applied."
    } else {
        ""
    };
    let _writing = lock_task_list();
    let prev = crate::agent::persist::load_last_tasks(Path::new(path)).unwrap_or_default();

    let mut merged = prev.clone();
    // Per task, in the order first named: how many journal entries it got.
    let mut touched: Vec<(String, usize)> = Vec::new();
    for args in items {
        let id = args.id.trim().to_string();
        let Some(index) = merged.iter().position(|t| t.id == id) else {
            // The ids are what the model needs to recover, so they come back in
            // the error instead of costing it a round to read the list.
            let ids: Vec<&str> = prev.iter().map(|t| t.id.as_str()).collect();
            return Err(if ids.is_empty() {
                format!(
                    "tasks_update: there is no task '{id}' — the task list is empty. Create it with tasks_set.{none_applied}"
                )
            } else {
                format!(
                    "tasks_update: there is no task '{id}'. Task ids: {}. Use tasks_set to add a task.{none_applied}",
                    ids.join(", ")
                )
            });
        };
        if let Some(status) = args.status.as_deref()
            && !TASK_STATUSES.contains(&status)
        {
            return Err(if batch {
                format!(
                    "tasks_update: '{id}': status must be one of todo, doing, done — got '{status}'.{none_applied}"
                )
            } else {
                format!("tasks_update: status must be one of todo, doing, done — got '{status}'.")
            });
        }
        let journal: Vec<String> = args
            .journal
            .into_iter()
            .filter(|entry| !entry.trim().is_empty())
            .collect();
        if let Some(status) = args.status {
            merged[index].status = status;
        }
        let added = journal.len();
        merged[index].journal.extend(journal);
        match touched.iter_mut().find(|(t, _)| *t == id) {
            Some((_, n)) => *n += added,
            None => touched.push((id, added)),
        }
    }

    let status_of = |tasks: &[TaskItem], id: &str| {
        tasks
            .iter()
            .find(|t| t.id == id)
            .map(|t| t.status.clone())
            .unwrap_or_default()
    };
    if merged == prev {
        return Ok(if batch {
            format!(
                "No change: every task is already as asked. {}",
                board(&merged)
            )
        } else {
            let id = &touched[0].0;
            format!(
                "No change: '{id}' is already {}. {}",
                status_of(&merged, id),
                board(&merged)
            )
        });
    }
    check_quality_gate(&prev, &merged, ctx)?;
    crate::agent::persist::append_tasks(Path::new(path), &merged)?;

    let whats: Vec<String> = touched
        .iter()
        .map(|(id, added)| {
            let (before, after) = (status_of(&prev, id), status_of(&merged, id));
            let mut what = if before == after {
                format!("'{id}' stays {after}")
            } else {
                format!("'{id}': {before} -> {after}")
            };
            if *added > 0 {
                what.push_str(&format!(
                    ", {added} journal entr{} added",
                    if *added == 1 { "y" } else { "ies" }
                ));
            }
            what
        })
        .collect();
    Ok(if batch {
        format!("Tasks updated — {}. {}", whats.join("; "), board(&merged))
    } else {
        format!("Task updated — {}. {}", whats[0], board(&merged))
    })
}

/// Where the whole list stands, in one line: enough for the model to pick its
/// next step without reading the list back.
fn board(tasks: &[TaskItem]) -> String {
    let count = |status: &str| tasks.iter().filter(|t| t.status == status).count();
    let (done, doing, todo) = (count("done"), count("doing"), count("todo"));
    if done == tasks.len() {
        return format!("All {done} tasks are done.");
    }
    let mut out = format!("Now {done} done, {doing} doing, {todo} todo.");
    let name = |t: &TaskItem| format!("'{}' ({})", t.id, t.title);
    if let Some(t) = tasks.iter().find(|t| t.status == "doing") {
        out.push_str(&format!(" In progress: {}.", name(t)));
    } else if let Some(t) = tasks.iter().find(|t| t.status == "todo") {
        out.push_str(&format!(" Next: {}.", name(t)));
    }
    out
}

/// Brain-mode gate: tasks may only be created once the most recent plan file
/// carries a non-empty `## Low-Level Design` section, so every task can
/// reference concrete technical detail instead of guesses. No-op in Builder
/// mode or when no mode handle / workspace is attached (tests, aux workflows).
fn check_brain_lld_gate(ctx: &crate::agent::tools::ToolContext, tool: &str) -> Result<(), String> {
    use crate::agent::tools::write_plan::{LLD_HEADING, has_nonempty_section, latest_plan_path};
    if !ctx.is_brain() {
        return Ok(());
    }
    let root = match ctx.workspace_root.as_deref() {
        Some(r) => r,
        None => return Ok(()),
    };
    match latest_plan_path(root, ctx.plan_save_path.as_deref()) {
        None => Err(format!(
            "{tool} rejected: no plan file exists yet. In Brain mode tasks may only \
             be created after the plan is complete: write the Solution Design via \
             write_plan, then call write_plan again with the full content plus a \
             '## Low-Level Design' section, then retry {tool}."
        )),
        Some(path) => {
            let content = std::fs::read_to_string(&path)
                .map_err(|e| format!("{tool}: cannot read plan {}: {e}", path.display()))?;
            if has_nonempty_section(&content, LLD_HEADING) {
                Ok(())
            } else {
                Err(format!(
                    "{tool} rejected: the most recent plan ({}) has no non-empty \
                     '## Low-Level Design' section. Research the codebase first (spawn_agents \
                     'explore' mode, semantic_search, file reading), then call write_plan again \
                     with the FULL plan content including a '## Low-Level Design' section \
                     (files/symbols to touch, data flow, APIs/schemas, patterns to reuse) - \
                     then retry {tool}.",
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

/// For a task that already exists, an empty description or an empty journal
/// in a `tasks_set` call keeps the stored one.
///
/// Since Phase 4 the model sees the list each run as a `<tasks>` block —
/// `id · status · title`, nothing more — and `tasks_get` is deferred. A
/// model that adds or rewords one task resends every other one from that
/// block, with nothing to put in their descriptions and journals; read
/// literally, the full replacement would erase what Brain wrote and every
/// finding recorded since (Phase 4 review). What the call does send — a
/// title, a status, a non-empty description or journal — still replaces the
/// stored value, so rewording stays what `tasks_set` is for.
fn keep_stored_details(prev: &[TaskItem], incoming: Vec<TaskItem>) -> Vec<TaskItem> {
    incoming
        .into_iter()
        .map(|mut t| {
            if let Some(stored) = prev.iter().find(|p| p.id == t.id) {
                if t.description.trim().is_empty() {
                    t.description = stored.description.clone();
                }
                if t.journal.iter().all(|j| j.trim().is_empty()) {
                    t.journal = stored.journal.clone();
                }
            }
            t
        })
        .collect()
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
        // By characters, not bytes: letters with accents are kept by the map
        // above and are more than one byte each, so byte 40 can fall inside
        // one — and slicing there panics, for any goal written in a language
        // that has them.
        match result.char_indices().nth(40) {
            Some((end, _)) => result[..end].to_string(),
            None => result,
        }
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
            doc_access: Default::default(),
            hooks: None,
            db_path: None,
            browser: None,
            lsp_manager: None,
            workspace_root: Some(root.to_string_lossy().to_string()),
            embedding_model: Arc::new(tokio::sync::Mutex::new(None)),
            session_store_path: Some(store.to_string_lossy().to_string()),
            limits: Default::default(),
            read_tracker: Arc::new(tokio::sync::Mutex::new(
                crate::agent::tools::ReadTracker::default(),
            )),
            interrupt: None,
            agent_config: None,
            plan_save_path: None,
            base_commit: None,
            auto_approve_git: false,
            shell_gate: None,
            mcp: None,
            mode_ctl: mode.map(|m| Arc::new(ModeCtl::new(m, ModeOrigin::Human))),
            index_progress: None,
            records_cache: Arc::new(std::sync::Mutex::new(LruCache::new(
                std::num::NonZeroUsize::new(1).unwrap(),
            ))),
        };
        (ctx, root)
    }

    // A goal in Portuguese: "ç" and "ã" are two bytes each, and the cut at
    // byte 40 used to land inside one.
    #[test]
    fn a_goal_with_accents_gets_a_slug_instead_of_a_panic() {
        // 39 one-byte letters, then "ç": byte 40 is its second byte.
        let goal = format!("{}ção do cadastro", "x".repeat(39));
        assert!(!goal.is_char_boundary(40));
        let slug = slugify(&goal);
        assert_eq!(slug, format!("{}ç", "x".repeat(39)));
        assert_eq!(slug.chars().count(), 40);
        // A real one, cut between two letters.
        assert_eq!(
            slugify("a migração da autenticação não pode quebrar sessões antigas"),
            "a-migração-da-autenticação-não-pode-queb"
        );
        // Every offset the cut can fall on, inside a character or between two.
        for lead in 0..4 {
            let _ = slugify(&format!("{}{}", "x".repeat(lead), "çã".repeat(40)));
        }
        // Plain ASCII is cut where it always was.
        assert_eq!(slugify(&"a".repeat(60)).len(), 40);
    }

    // Subagents run in parallel and each closes its own task. Without one
    // writer at a time, two of them start from the same list and the later
    // write undoes the earlier one.
    #[test]
    fn updates_from_parallel_agents_do_not_undo_each_other() {
        let (ctx, root) = ctx_for("parallel-updates", None);
        let tasks: Vec<TaskItem> = (0..24)
            .map(|i| TaskItem {
                id: format!("t{i}"),
                title: "t".into(),
                description: "d".into(),
                journal: vec![],
                status: "todo".into(),
            })
            .collect();
        execute_set(SetTasksArgs { tasks }, &ctx).unwrap();

        std::thread::scope(|scope| {
            for i in 0..24 {
                let ctx = &ctx;
                scope.spawn(move || {
                    execute_update(
                        UpdateTaskArgs {
                            id: format!("t{i}"),
                            status: Some("done".into()),
                            journal: vec![format!("closed by agent {i}")],
                        },
                        ctx,
                    )
                    .unwrap();
                });
            }
        });

        let store = ctx.session_store_path.as_deref().unwrap();
        let after = crate::agent::persist::load_last_tasks(Path::new(store)).unwrap();
        assert_eq!(after.len(), 24);
        for (i, task) in after.iter().enumerate() {
            assert_eq!(task.status, "done", "{}", task.id);
            assert_eq!(task.journal, vec![format!("closed by agent {i}")]);
        }
        let _ = std::fs::remove_dir_all(&root);
    }

    // Each run shows the model the list as `id · status · title` only, and
    // `tasks_get` is deferred, so a model that adds or rewords one task with
    // `tasks_set` resends the others from that block: an empty description
    // and an empty journal. A full replacement must not turn that into the
    // loss of the descriptions Brain wrote and the findings recorded since —
    // for a task that already exists, an empty description or journal keeps
    // the stored one, golden tasks included (Phase 4 review).
    #[test]
    fn a_rewrite_from_the_tasks_block_alone_keeps_descriptions_and_journals() {
        let (ctx, root) = ctx_for("rewrite-from-block", None);
        let stored = |id: &str, title: &str, description: &str, journal: &[&str]| TaskItem {
            id: id.into(),
            title: title.into(),
            description: description.into(),
            journal: journal.iter().map(|j| j.to_string()).collect(),
            status: "todo".into(),
        };
        let store = ctx.session_store_path.clone().unwrap();
        crate::agent::persist::append_tasks(
            Path::new(&store),
            &[
                stored("golden-g-0", "the goal", "Plan the goal", &["scoped it"]),
                stored("t1", "Parse it", "Brain's parser design", &["uses nom"]),
            ],
        )
        .unwrap();

        let from_block = |id: &str, title: &str, status: &str| TaskItem {
            id: id.into(),
            title: title.into(),
            description: String::new(),
            journal: vec![],
            status: status.into(),
        };
        execute_set(
            SetTasksArgs {
                tasks: vec![
                    from_block("golden-g-0", "the goal", "doing"),
                    from_block("t1", "Parse the config", "done"),
                    TaskItem {
                        description: "Write the docs".into(),
                        ..from_block("t2", "Docs", "todo")
                    },
                ],
            },
            &ctx,
        )
        .unwrap();

        let after = crate::agent::persist::load_last_tasks(Path::new(&store)).unwrap();
        let by_id = |id: &str| after.iter().find(|t| t.id == id).unwrap().clone();
        let golden = by_id("golden-g-0");
        assert_eq!(golden.description, "Plan the goal");
        assert_eq!(golden.journal, ["scoped it"]);
        assert_eq!(golden.status, "doing");
        let t1 = by_id("t1");
        // What the call does say still wins: the new title and status.
        assert_eq!(t1.title, "Parse the config");
        assert_eq!(t1.status, "done");
        assert_eq!(t1.description, "Brain's parser design");
        assert_eq!(t1.journal, ["uses nom"]);
        assert_eq!(by_id("t2").description, "Write the docs");
        let _ = std::fs::remove_dir_all(&root);
    }

    // A description or journal the call does send replaces the stored one:
    // rewording a task is what `tasks_set` is for.
    #[test]
    fn a_rewrite_that_sends_a_description_or_journal_replaces_it() {
        let (ctx, root) = ctx_for("rewrite-with-details", None);
        let store = ctx.session_store_path.clone().unwrap();
        let t = |description: &str, journal: &[&str]| TaskItem {
            id: "t1".into(),
            title: "t".into(),
            description: description.into(),
            journal: journal.iter().map(|j| j.to_string()).collect(),
            status: "todo".into(),
        };
        crate::agent::persist::append_tasks(Path::new(&store), &[t("old", &["a"])]).unwrap();
        execute_set(
            SetTasksArgs {
                tasks: vec![t("new", &["b"])],
            },
            &ctx,
        )
        .unwrap();
        let after = crate::agent::persist::load_last_tasks(Path::new(&store)).unwrap();
        assert_eq!(after[0].description, "new");
        assert_eq!(after[0].journal, ["b"]);
        let _ = std::fs::remove_dir_all(&root);
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
    fn brain_update_before_the_lld_is_rejected_and_names_itself() {
        let (ctx, root) = ctx_for("update-nold", Some(SessionMode::Brain));
        let err = execute_update(
            UpdateTaskArgs {
                id: "t1".into(),
                status: Some("done".into()),
                journal: vec![],
            },
            &ctx,
        )
        .unwrap_err();
        // The retry instruction names the tool that was actually called.
        assert!(err.contains("tasks_update rejected"), "{err}");
        assert!(err.contains("retry tasks_update"), "{err}");
        std::fs::remove_dir_all(&root).ok();
    }

    // The LLD gate holds for the batch form as for the single one.
    #[test]
    fn brain_batch_update_before_the_lld_is_rejected() {
        let (ctx, root) = ctx_for("batch-nold", Some(SessionMode::Brain));
        let err = execute_updates(
            vec![UpdateTaskArgs {
                id: "t1".into(),
                status: Some("done".into()),
                journal: vec![],
            }],
            &ctx,
        )
        .unwrap_err();
        assert!(err.contains("tasks_update rejected"), "{err}");
        assert!(err.contains("Low-Level Design"), "{err}");
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

    // ── tasks_update: the same gates, one task at a time ─────────────────────

    fn update(
        ctx: &crate::agent::tools::ToolContext,
        id: &str,
        status: Option<&str>,
        journal: &[&str],
    ) -> Result<String, String> {
        execute_update(
            UpdateTaskArgs {
                id: id.into(),
                status: status.map(str::to_string),
                journal: journal.iter().map(|j| j.to_string()).collect(),
            },
            ctx,
        )
    }

    fn stored(ctx: &crate::agent::tools::ToolContext) -> Vec<TaskItem> {
        let store = ctx.session_store_path.clone().unwrap();
        crate::agent::persist::load_last_tasks(std::path::Path::new(&store)).unwrap()
    }

    #[test]
    fn an_update_changes_one_task_and_leaves_the_rest_exactly_as_they_were() {
        let mut second = task("task-2", "todo");
        second.title = "wire the UI".into();
        second.description = "a long description nobody should have to resend".into();
        second.journal = vec!["found the entry point".into()];
        let (ctx, root) = setup("delta", "{}", &[task("task-1", "todo"), second.clone()]);

        let out = update(&ctx, "task-1", Some("doing"), &[]).unwrap();
        assert!(out.contains("'task-1': todo -> doing"), "{out}");
        assert!(out.contains("0 done, 1 doing, 1 todo"), "{out}");
        assert!(out.contains("In progress: 'task-1'"), "{out}");

        let out = update(
            &ctx,
            "task-1",
            Some("done"),
            &["it was the cache", "added a test"],
        )
        .unwrap();
        assert!(out.contains("'task-1': doing -> done"), "{out}");
        assert!(out.contains("2 journal entries added"), "{out}");
        // The reply names what is next, so no tasks_get is needed to go on.
        assert!(out.contains("Next: 'task-2' (wire the UI)"), "{out}");

        let tasks = stored(&ctx);
        assert_eq!(tasks.len(), 2);
        assert_eq!(tasks[0].status, "done");
        assert_eq!(tasks[0].journal, vec!["it was the cache", "added a test"]);
        // The task that was not named is byte-for-byte what it was.
        assert_eq!(tasks[1].title, second.title);
        assert_eq!(tasks[1].description, second.description);
        assert_eq!(tasks[1].journal, second.journal);
        assert_eq!(tasks[1].status, "todo");
        std::fs::remove_dir_all(&root).ok();
    }

    #[test]
    fn journal_entries_can_be_appended_without_touching_the_status() {
        let (ctx, root) = setup("journal", "{}", &[task("task-1", "doing")]);
        let out = update(&ctx, "task-1", None, &["half way", "  "]).unwrap();
        assert!(out.contains("'task-1' stays doing"), "{out}");
        assert!(out.contains("1 journal entry added"), "{out}");
        assert_eq!(stored(&ctx)[0].journal, vec!["half way"]);
        std::fs::remove_dir_all(&root).ok();
    }

    #[test]
    fn the_last_task_closing_says_so() {
        let (ctx, root) = setup(
            "alldone",
            "{}",
            &[task("task-1", "done"), task("task-2", "doing")],
        );
        let out = update(&ctx, "task-2", Some("done"), &[]).unwrap();
        assert!(out.contains("All 2 tasks are done."), "{out}");
        std::fs::remove_dir_all(&root).ok();
    }

    #[test]
    fn an_unknown_id_comes_back_with_the_ids_that_exist() {
        let (ctx, root) = setup(
            "unknown",
            "{}",
            &[task("task-1", "todo"), task("task-2", "todo")],
        );
        let err = update(&ctx, "task-9", Some("done"), &[]).unwrap_err();
        assert!(err.contains("no task 'task-9'"), "{err}");
        assert!(err.contains("task-1, task-2"), "{err}");
        // Nothing was written for a call that changed nothing.
        assert_eq!(stored(&ctx)[0].status, "todo");

        let (empty, root2) = setup("unknown-empty", "{}", &[]);
        let err = update(&empty, "task-1", Some("done"), &[]).unwrap_err();
        assert!(err.contains("tasks_set"), "{err}");
        std::fs::remove_dir_all(&root).ok();
        std::fs::remove_dir_all(&root2).ok();
    }

    #[test]
    fn a_status_that_does_not_exist_is_refused() {
        let (ctx, root) = setup("badstatus", "{}", &[task("task-1", "todo")]);
        let err = update(&ctx, "task-1", Some("finished"), &[]).unwrap_err();
        assert!(err.contains("todo, doing, done"), "{err}");
        assert_eq!(stored(&ctx)[0].status, "todo");
        std::fs::remove_dir_all(&root).ok();
    }

    #[test]
    fn a_call_that_changes_nothing_writes_nothing() {
        let (ctx, root) = setup("noop", "{}", &[task("task-1", "done")]);
        let store = ctx.session_store_path.clone().unwrap();
        let before = std::fs::read_to_string(&store).unwrap();
        let out = update(&ctx, "task-1", Some("done"), &[]).unwrap();
        assert!(out.contains("No change"), "{out}");
        assert_eq!(std::fs::read_to_string(&store).unwrap(), before);
        std::fs::remove_dir_all(&root).ok();
    }

    /// A status update is exactly how a goal gets closed, so the quality gate
    /// has to stand in front of it too — or the delta would be a way around.
    #[test]
    fn closing_an_execution_goal_by_update_goes_through_the_quality_gate() {
        let (ctx, root) = setup(
            "delta-gate",
            "{}",
            &[task("golden-g-0", "done"), task("golden-g-1", "doing")],
        );
        let err = update(&ctx, "golden-g-1", Some("done"), &[]).unwrap_err();
        assert!(err.contains("run_quality"), "{err}");
        assert!(err.contains("golden-g-1"), "{err}");
        assert_eq!(
            stored(&ctx)[1].status,
            "doing",
            "a refused update persists nothing"
        );

        // Journal notes on the goal are not a close and are not gated.
        assert!(update(&ctx, "golden-g-1", None, &["tests still red"]).is_ok());

        record_run(&ctx, &root, true);
        // The journal append above did not touch the workspace, so the green
        // run is still current.
        let out = update(&ctx, "golden-g-1", Some("done"), &[]).unwrap();
        assert!(out.contains("'golden-g-1': doing -> done"), "{out}");
        assert_eq!(stored(&ctx)[1].status, "done");
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

    // ── tasks_update, batched: several tasks, one write, all or nothing ──────

    fn item(id: &str, status: Option<&str>, journal: &[&str]) -> UpdateTaskArgs {
        UpdateTaskArgs {
            id: id.into(),
            status: status.map(str::to_string),
            journal: journal.iter().map(|j| j.to_string()).collect(),
        }
    }

    fn task_records(ctx: &crate::agent::tools::ToolContext) -> usize {
        let store = ctx.session_store_path.clone().unwrap();
        crate::agent::persist::load_records(std::path::Path::new(&store))
            .unwrap()
            .iter()
            .filter(|r| matches!(r, crate::agent::persist::SessionRecord::Tasks { .. }))
            .count()
    }

    #[test]
    fn a_batch_moves_several_tasks_in_one_write() {
        let (ctx, root) = setup(
            "batch",
            "{}",
            &[
                task("task-1", "doing"),
                task("task-2", "todo"),
                task("task-3", "todo"),
            ],
        );
        let before = task_records(&ctx);
        let out = execute_updates(
            vec![
                item("task-1", Some("done"), &["it was the cache"]),
                item("task-2", Some("doing"), &[]),
            ],
            &ctx,
        )
        .unwrap();
        assert_eq!(task_records(&ctx), before + 1, "one list written, not two");
        assert!(out.contains("'task-1': doing -> done"), "{out}");
        assert!(out.contains("'task-2': todo -> doing"), "{out}");
        assert!(out.contains("1 done, 1 doing, 1 todo"), "{out}");
        let tasks = stored(&ctx);
        assert_eq!(tasks[0].status, "done");
        assert_eq!(tasks[0].journal, ["it was the cache"]);
        assert_eq!(tasks[1].status, "doing");
        assert_eq!(tasks[2].status, "todo");
        std::fs::remove_dir_all(&root).ok();
    }

    #[test]
    fn a_batch_with_an_unknown_id_applies_nothing_and_names_it() {
        let (ctx, root) = setup(
            "batch-unknown",
            "{}",
            &[task("task-1", "doing"), task("task-2", "todo")],
        );
        let before = task_records(&ctx);
        let err = execute_updates(
            vec![
                item("task-1", Some("done"), &[]),
                item("task-9", Some("doing"), &[]),
            ],
            &ctx,
        )
        .unwrap_err();
        assert!(err.contains("'task-9'"), "{err}");
        assert!(err.contains("task-1, task-2"), "the ids that exist: {err}");
        assert!(err.contains("nothing was applied"), "{err}");
        assert_eq!(task_records(&ctx), before);
        assert_eq!(stored(&ctx)[0].status, "doing", "task-1 did not move");
        std::fs::remove_dir_all(&root).ok();
    }

    #[test]
    fn a_batch_with_a_status_that_does_not_exist_applies_nothing_and_names_the_task() {
        let (ctx, root) = setup(
            "batch-status",
            "{}",
            &[task("task-1", "doing"), task("task-2", "todo")],
        );
        let err = execute_updates(
            vec![
                item("task-1", Some("done"), &[]),
                item("task-2", Some("finished"), &[]),
            ],
            &ctx,
        )
        .unwrap_err();
        assert!(
            err.contains("'task-2'") && err.contains("'finished'"),
            "{err}"
        );
        assert!(err.contains("nothing was applied"), "{err}");
        assert_eq!(stored(&ctx)[0].status, "doing");
        std::fs::remove_dir_all(&root).ok();
    }

    // The quality gate sees the list as the whole batch would leave it: a
    // batch is not a way to close a goal on a claim, and the task beside the
    // goal does not move either.
    #[test]
    fn a_batch_that_closes_a_goal_without_evidence_applies_nothing() {
        let (ctx, root) = setup(
            "batch-gate",
            "{}",
            &[
                task("task-1", "doing"),
                task("golden-g-0", "done"),
                task("golden-g-1", "doing"),
            ],
        );
        let err = execute_updates(
            vec![
                item("task-1", Some("done"), &[]),
                item("golden-g-1", Some("done"), &[]),
            ],
            &ctx,
        )
        .unwrap_err();
        assert!(
            err.contains("golden-g-1") && err.contains("run_quality"),
            "{err}"
        );
        let tasks = stored(&ctx);
        assert_eq!(tasks[0].status, "doing");
        assert_eq!(tasks[2].status, "doing");

        record_run(&ctx, &root, true);
        let out = execute_updates(
            vec![
                item("task-1", Some("done"), &[]),
                item("golden-g-1", Some("done"), &[]),
            ],
            &ctx,
        )
        .unwrap();
        assert!(out.contains("All 3 tasks are done."), "{out}");
        std::fs::remove_dir_all(&root).ok();
    }

    // Updates apply in the order given, so one task can go todo -> doing ->
    // done in one call, and a batch that changes nothing writes nothing.
    #[test]
    fn a_batch_applies_in_order_and_one_that_changes_nothing_writes_nothing() {
        let (ctx, root) = setup("batch-order", "{}", &[task("task-1", "todo")]);
        let out = execute_updates(
            vec![
                item("task-1", Some("doing"), &[]),
                item("task-1", Some("done"), &["verified"]),
            ],
            &ctx,
        )
        .unwrap();
        assert!(out.contains("'task-1': todo -> done"), "{out}");
        let before = task_records(&ctx);
        let out = execute_updates(vec![item("task-1", Some("done"), &[])], &ctx).unwrap();
        assert!(out.starts_with("No change"), "{out}");
        assert_eq!(task_records(&ctx), before);
        std::fs::remove_dir_all(&root).ok();
    }

    #[test]
    fn both_forms_parse_and_a_call_in_neither_or_both_is_refused() {
        use serde_json::json;
        let one = parse_update_args(json!({"id": "t1", "status": "doing"})).unwrap();
        assert_eq!(one.len(), 1);
        assert_eq!(one[0].id, "t1");
        assert_eq!(one[0].status.as_deref(), Some("doing"));
        let many = parse_update_args(json!({"updates": [
            {"id": "t1", "status": "done", "journal": ["why"]},
            {"id": "t2", "status": "doing"}
        ]}))
        .unwrap();
        assert_eq!(many.len(), 2);
        assert_eq!(many[0].journal, ["why"]);
        assert_eq!(many[1].id, "t2");

        for bad in [
            json!({}),
            json!({"updates": []}),
            json!({"id": "t1", "updates": [{"id": "t2"}]}),
            json!({"updates": [{"status": "done"}]}),
        ] {
            let err = parse_update_args(bad.clone()).unwrap_err();
            assert!(
                err.contains("updates") && err.contains("id"),
                "{bad}: {err}"
            );
        }
    }

    // Some models fill every optional schema property, sending null for the
    // form they do not use. A null is an absent value, not a second form: the
    // call means what its non-null fields say.
    #[test]
    fn a_null_for_the_unused_form_is_read_as_absent() {
        use serde_json::json;
        let one = parse_update_args(
            json!({"id": "t1", "status": "done", "updates": null, "journal": null}),
        )
        .unwrap();
        assert_eq!(one.len(), 1);
        assert_eq!(one[0].id, "t1");
        assert_eq!(one[0].status.as_deref(), Some("done"));
        assert!(one[0].journal.is_empty());
        let many = parse_update_args(json!({
            "updates": [{"id": "t1", "status": "done"}, {"id": "t2", "status": "doing"}],
            "id": null,
            "status": null
        }))
        .unwrap();
        assert_eq!(many.len(), 2);
        // Both null names no task at all.
        let err = parse_update_args(json!({"id": null, "updates": null})).unwrap_err();
        assert!(err.contains("no task named"), "{err}");
    }

    // A top-level status or journal beside `updates` belongs to no task in the
    // batch: the call's intent is clear but its target is not, so it is
    // refused with both shapes instead of being dropped and answered with
    // "No change" (Phase 4 review). An empty journal says nothing and is
    // read as absent, like a null.
    #[test]
    fn a_top_level_status_or_journal_beside_updates_is_refused() {
        use serde_json::json;
        for bad in [
            json!({"updates": [{"id": "t1"}], "status": "done"}),
            json!({"updates": [{"id": "t1"}], "journal": ["found it"]}),
        ] {
            let err = parse_update_args(bad.clone()).unwrap_err();
            assert!(
                err.contains("updates") && err.contains("\"id\""),
                "{bad}: {err}"
            );
        }
        let many =
            parse_update_args(json!({"updates": [{"id": "t1", "status": "done"}], "journal": []}))
                .unwrap();
        assert_eq!(many.len(), 1);
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
