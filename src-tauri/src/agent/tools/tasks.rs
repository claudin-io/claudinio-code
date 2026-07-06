use serde::Deserialize;
use std::path::Path;

use crate::commands::tasks::TaskItem;

/// Return the current list of tasks from the session JSONL.
pub fn execute_get(ctx: &crate::agent::tools::ToolContext) -> Result<String, String> {
    let path = ctx.session_store_path.as_ref().ok_or("session_store_path not set")?;
    let tasks = crate::commands::tasks::load_last_tasks(Path::new(path))?;
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
    let path = ctx.session_store_path.as_ref().ok_or("session_store_path not set")?;
    crate::commands::tasks::append_tasks(Path::new(path), &args.tasks)?;
    Ok(format!("Tasks updated: {} task(s) saved.", args.tasks.len()))
}
