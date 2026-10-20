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
    if crate::agent::doc_session::has_document(ctx) {
        return Err(crate::agent::gate_text::TASKS_WITH_DOCUMENT.into());
    }
    let path = ctx
        .session_store_path
        .as_ref()
        .ok_or("session_store_path not set")?;
    let _writing = lock_task_list();
    let prev = crate::agent::persist::load_last_tasks(Path::new(path)).unwrap_or_default();
    let incoming = fill_from_stored(&prev, args.tasks)?;
    let (incoming, renamed) = strip_forged_golden_ids(&prev, incoming);
    let (incoming, kept) = keep_stored_details(&prev, incoming);
    let (merged, preserved) = merge_preserving_golden(&prev, incoming);
    check_quality_gate(&prev, &merged, ctx)?;
    crate::agent::persist::append_tasks(Path::new(path), &merged)?;
    let mut note = String::new();
    if renamed > 0 {
        note.push_str(&crate::agent::gate_text::golden_renamed(renamed));
    }
    if preserved > 0 {
        note.push_str(&crate::agent::gate_text::golden_preserved(preserved));
    }
    if !kept.is_empty() {
        note.push_str(&crate::agent::gate_text::kept_stored_details(&kept));
    }
    Ok(format!(
        "Tasks updated: {} task(s) saved.{}",
        merged.len(),
        note
    ))
}

/// A task is an id and a title; the rest is optional (single-timeline plan,
/// Phase 5). A known task resent without its title or status keeps the
/// stored ones; a new task without a status is `todo`, and one without a
/// title is refused — a list of bare ids is not a plan anyone can read.
fn fill_from_stored(prev: &[TaskItem], incoming: Vec<TaskItem>) -> Result<Vec<TaskItem>, String> {
    incoming
        .into_iter()
        .map(|mut t| {
            let stored = prev.iter().find(|p| p.id == t.id);
            if t.title.trim().is_empty() {
                t.title = stored
                    .map(|p| p.title.clone())
                    .ok_or_else(|| format!("task '{}' is new and needs a title", t.id))?;
            }
            if t.status.trim().is_empty() {
                t.status = stored.map_or_else(|| "todo".to_string(), |p| p.status.clone());
            }
            Ok(t)
        })
        .collect()
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
    // Nor does an empty `updates` beside the single form: the same model
    // fills an unused array with `[]` instead of null.
    if single_form
        && args
            .get("updates")
            .and_then(|u| u.as_array())
            .is_some_and(|a| a.is_empty())
        && let Some(fields) = args.as_object_mut()
    {
        fields.remove("updates");
    }
    match args.get("updates") {
        // A top-level status or journal with no id names no task of the batch.
        Some(_) if single_form && !has_id => Err(format!("tasks_update: {SHAPES}, not both.")),
        Some(updates) => {
            let mut items: Vec<UpdateTaskArgs> = serde_json::from_value(updates.clone())
                .map_err(|e| format!("tasks_update: invalid updates ({e}); {SHAPES}."))?;
            if items.is_empty() {
                return Err(format!("tasks_update: updates is empty; {SHAPES}."));
            }
            // A top-level task that names its id joins the batch, first
            // (single-timeline plan, Phase 5: refusing it cost a round).
            if has_id {
                let mut one = args.clone();
                if let Some(fields) = one.as_object_mut() {
                    fields.remove("updates");
                }
                let one: UpdateTaskArgs = serde_json::from_value(one)
                    .map_err(|e| format!("tasks_update: invalid args ({e}); {SHAPES}."))?;
                items.insert(0, one);
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
///
/// "Already exists" is more than a shared id. Ids are the model's and
/// usually sequential, so a fresh list for an unrelated request reuses the
/// ids of a finished one, and matching on the id alone handed the new tasks
/// the old ones' journals (Phase 4 final review). A stored task counts as
/// the same task when it is not done yet (the list is still being worked,
/// a reword included) or when the title is unchanged (a finished task
/// resent from the block). A done task under another title is a different
/// task that happens to reuse the id. One consequence stays: `tasks_set`
/// cannot clear the journal of a task it keeps — an empty journal reads as
/// "not sent".
///
/// An unfinished task sent under a new title is the one case the rule cannot
/// settle: a reword, or a fresh list reusing the ids of one abandoned
/// unfinished (Phase 4f final review). Its details are kept, and the ids of
/// such tasks are returned so `tasks_set` names them and says how to start
/// one clean (a new id).
///
/// Each named task comes with what was kept of it ("description", "journal"
/// or both), so the note never claims a part the stored task did not have.
fn keep_stored_details(
    prev: &[TaskItem],
    incoming: Vec<TaskItem>,
) -> (Vec<TaskItem>, Vec<(String, &'static str)>) {
    let same_task = |stored: &TaskItem, t: &TaskItem| {
        stored.status != "done" || stored.title.trim() == t.title.trim()
    };
    let mut retitled = Vec::new();
    let tasks = incoming
        .into_iter()
        .map(|mut t| {
            if let Some(stored) = prev.iter().find(|p| p.id == t.id && same_task(p, &t)) {
                let (mut description, mut journal) = (false, false);
                if t.description.trim().is_empty() {
                    description = !stored.description.trim().is_empty();
                    t.description = stored.description.clone();
                }
                if t.journal.iter().all(|j| j.trim().is_empty()) {
                    journal = !stored.journal.is_empty();
                    t.journal = stored.journal.clone();
                }
                let kept = match (description, journal) {
                    (true, true) => Some("description and journal"),
                    (true, false) => Some("description"),
                    (false, true) => Some("journal"),
                    (false, false) => None,
                };
                if let Some(kept) = kept
                    && stored.title.trim() != t.title.trim()
                {
                    retitled.push((t.id.clone(), kept));
                }
            }
            t
        })
        .collect();
    (tasks, retitled)
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
    // One task per goal, gated by the quality check (`is_golden_execution`):
    // the `-1` suffix is the gated half sessions from before one timeline
    // already carry, so their goals keep their ids. The title carries the
    // raw goal text; the description is model-facing (tasks_get).
    goals
        .iter()
        .map(|goal| TaskItem {
            id: format!("golden-{}-1", slugify(goal)),
            title: goal.clone(),
            description: format!("Achieve the goal and verify it is met: {goal}"),
            journal: Vec::new(),
            status: "todo".to_string(),
        })
        .collect()
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
        // A `-0` "plan" half is from a session before one timeline: nobody
        // has to close it any more.
        .filter(|t| is_golden(t) && t.status != "done" && !t.id.ends_with("-0"))
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
    fn both_forms_parse_and_a_call_that_names_no_task_is_refused() {
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
            json!({"updates": [{"status": "done"}]}),
            // A top-level status beside `updates` names no task of the batch.
            json!({"status": "done", "updates": [{"id": "t2"}]}),
        ] {
            let err = parse_update_args(bad.clone()).unwrap_err();
            assert!(
                err.contains("updates") && err.contains("id"),
                "{bad}: {err}"
            );
        }
    }

    // Single-timeline plan, Phase 5: session fc2bd5c3a6992b1e named one task
    // at the top level and another in `updates`, and the call was refused
    // ("not both") — a retry round for a call whose meaning was plain. A
    // top-level task that names its id joins the batch, first.
    #[test]
    fn a_named_task_beside_the_batch_joins_it() {
        use serde_json::json;
        let both = parse_update_args(json!({
            "id": "t1", "status": "done",
            "updates": [{"id": "t2", "status": "doing"}]
        }))
        .unwrap();
        assert_eq!(
            both.iter()
                .map(|u| (u.id.as_str(), u.status.as_deref()))
                .collect::<Vec<_>>(),
            [("t1", Some("done")), ("t2", Some("doing"))]
        );
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

    // The same model fills an unused array with its empty default instead of
    // null. An empty `updates` beside a single-form call names no task, as
    // an empty journal does: the call means its single form, not a refusal
    // that costs a retry round. Alone, it still names nothing.
    #[test]
    fn an_empty_updates_beside_the_single_form_is_read_as_absent() {
        use serde_json::json;
        let one = parse_update_args(json!({"id": "t1", "status": "done", "updates": []})).unwrap();
        assert_eq!(one.len(), 1);
        assert_eq!(one[0].id, "t1");
        assert_eq!(one[0].status.as_deref(), Some("done"));
        assert!(parse_update_args(json!({"updates": []})).is_err());
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

/// The checklist's lean shape (single-timeline plan, Phase 5): a task is
/// an id and a title; the rest is optional.
#[cfg(test)]
mod checklist_tests {
    use super::*;

    fn ctx_with(
        name: &str,
        prev: &[TaskItem],
    ) -> (crate::agent::tools::ToolContext, std::path::PathBuf) {
        let root = std::env::temp_dir().join(format!("cq-checklist-{name}-{}", std::process::id()));
        std::fs::remove_dir_all(&root).ok();
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

    fn stored(ctx: &crate::agent::tools::ToolContext) -> Vec<TaskItem> {
        crate::agent::persist::load_last_tasks(std::path::Path::new(
            ctx.session_store_path.as_deref().unwrap(),
        ))
        .unwrap()
    }

    /// The schema asks for what the code needs and no more: every required
    /// field the model must fill is a chance to get one wrong.
    #[test]
    fn the_tasks_set_schema_requires_only_an_id_and_a_title() {
        let def = crate::agent::tools::get_defs(4)
            .into_iter()
            .find(|d| d.name == "tasks_set")
            .unwrap();
        assert_eq!(
            def.input_schema["properties"]["tasks"]["items"]["required"],
            serde_json::json!(["id", "title"])
        );
    }

    #[tokio::test]
    async fn a_task_list_needs_only_ids_and_titles() {
        let (ctx, root) = ctx_with("ids-titles", &[]);
        crate::agent::tools::execute(
            "tasks_set",
            serde_json::json!({"tasks": [{"id": "t1", "title": "Parse the config"}]}),
            &ctx,
        )
        .await
        .expect("an id and a title are a task");
        let tasks = stored(&ctx);
        assert_eq!(tasks.len(), 1);
        assert_eq!(tasks[0].status, "todo");
        assert!(tasks[0].description.is_empty() && tasks[0].journal.is_empty());
        std::fs::remove_dir_all(&root).ok();
    }

    /// Session fb4cb4d615a93162: `tasks_set` was refused for a task sent
    /// without its title ("missing field `title`"). A task the list already
    /// has keeps its stored title; a new one still needs one.
    #[tokio::test]
    async fn a_known_task_sent_without_a_title_keeps_its_title() {
        let prev = TaskItem {
            id: "t1".into(),
            title: "Parse the config".into(),
            description: String::new(),
            journal: vec![],
            status: "todo".into(),
        };
        let (ctx, root) = ctx_with("keep-title", &[prev]);
        crate::agent::tools::execute(
            "tasks_set",
            serde_json::json!({"tasks": [{"id": "t1", "status": "doing"}, {"id": "t2", "title": "Write the parser"}]}),
            &ctx,
        )
        .await
        .expect("a known id keeps its title");
        let tasks = stored(&ctx);
        assert_eq!(tasks[0].title, "Parse the config");
        assert_eq!(tasks[0].status, "doing");
        let err = crate::agent::tools::execute(
            "tasks_set",
            serde_json::json!({"tasks": [{"id": "t3"}]}),
            &ctx,
        )
        .await
        .unwrap_err();
        assert!(err.contains("t3") && err.contains("title"), "{err}");
        std::fs::remove_dir_all(&root).ok();
    }
}

#[cfg(test)]
mod golden_tests {
    use super::*;
    use crate::agent::persist::TaskItem;

    /// One timeline (single-timeline plan, Phase 3): a goal is one task,
    /// gated by the quality check, not a Brain "plan" half and a Builder
    /// "execute" half.
    #[test]
    fn a_goal_is_one_gated_task() {
        let tasks = create_golden_tasks(&["ship it".into()]);
        assert_eq!(tasks.len(), 1, "{tasks:?}");
        assert!(is_golden_execution(&tasks[0]));
        assert_eq!(tasks[0].title, "ship it");
        // A session from before keeps loading: its "-0" plan half is not a
        // goal anyone has to close any more.
        let mut legacy = create_golden_tasks(&["ship it".into()]);
        legacy.push(TaskItem {
            id: "golden-ship-it-0".into(),
            title: "ship it".into(),
            description: String::new(),
            journal: vec![],
            status: "todo".into(),
        });
        legacy[0].status = "done".into();
        assert!(
            golden_pending_ids(&legacy).is_empty(),
            "{:?}",
            golden_pending_ids(&legacy)
        );
    }

    #[test]
    fn test_create_golden_tasks() {
        let goals = vec!["code coverage in 80%".to_string()];
        let tasks = create_golden_tasks(&goals);
        assert_eq!(tasks.len(), 1, "one gated task per goal");
        assert!(tasks[0].id.starts_with("golden-") && tasks[0].id.ends_with("-1"));
        assert_eq!(tasks[0].status, "todo");
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
        let task = |id: &str, status: &str| TaskItem {
            id: id.into(),
            title: "test".into(),
            description: "".into(),
            journal: vec![],
            status: status.into(),
        };
        // A goal's gated task still open counts; a done one and a plain task
        // do not, and neither does a legacy "-0" plan half.
        let tasks = vec![
            task("golden-a-0", "todo"),
            task("golden-a-1", "todo"),
            task("golden-b-1", "done"),
            task("normal-task", "todo"),
        ];
        let remaining = golden_tasks_remaining(&tasks);
        assert_eq!(remaining.len(), 1);
        assert_eq!(remaining[0].id, "golden-a-1");
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
        let task = |id: &str, status: &str| TaskItem {
            id: id.into(),
            title: "t".into(),
            description: "".into(),
            journal: vec![],
            status: status.into(),
        };
        let tasks = vec![
            task("golden-x-0", "todo"),
            task("golden-x-1", "todo"),
            task("golden-y-1", "done"),
        ];
        assert_eq!(golden_pending_ids(&tasks), vec!["golden-x-1"]);
    }
}
