use crate::agent::persist::{TaskItem, append_tasks, load_last_tasks};
use crate::state::AppState;
use tauri::State;

/// Return all current tasks from a workspace's active session JSONL.
#[tauri::command]
pub async fn get_tasks(
    workspace: String,
    state: State<'_, AppState>,
) -> Result<Vec<TaskItem>, String> {
    let ws = match state.workspace(&workspace).await {
        Ok(ws) => ws,
        Err(_) => return Ok(Vec::new()),
    };
    let guard = ws.active_session.lock().await;
    match guard.as_ref() {
        Some(handle) => load_last_tasks(&handle.store_path),
        None => Ok(Vec::new()),
    }
}

/// Append tasks to a workspace's active session JSONL.
#[tauri::command]
pub async fn set_tasks(
    workspace: String,
    tasks: Vec<TaskItem>,
    state: State<'_, AppState>,
) -> Result<(), String> {
    let ws = state.workspace(&workspace).await?;
    let guard = ws.active_session.lock().await;
    match guard.as_ref() {
        Some(handle) => append_tasks(&handle.store_path, &tasks),
        None => Ok(()),
    }
}

/// Drop golden tasks from a workspace's active session so a stale `<goal>`
/// from an earlier turn stops re-triggering the golden loop. If `task_id` is
/// given, only that golden task is dropped; otherwise all golden tasks are
/// dropped. Non-golden tasks are always preserved.
#[tauri::command]
pub async fn dismiss_golden_tasks(
    workspace: String,
    task_id: Option<String>,
    state: State<'_, AppState>,
) -> Result<Vec<TaskItem>, String> {
    let ws = state.workspace(&workspace).await?;
    let guard = ws.active_session.lock().await;
    let Some(handle) = guard.as_ref() else {
        return Ok(Vec::new());
    };
    let tasks = load_last_tasks(&handle.store_path)?;
    let remaining: Vec<TaskItem> = tasks
        .into_iter()
        .filter(|t| {
            let is_golden = crate::agent::tools::tasks::is_golden(t);
            match &task_id {
                Some(id) => !(is_golden && &t.id == id),
                None => !is_golden,
            }
        })
        .collect();
    append_tasks(&handle.store_path, &remaining)?;
    Ok(remaining)
}
