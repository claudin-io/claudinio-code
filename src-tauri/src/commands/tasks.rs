use crate::agent::persist::{load_records, SessionRecord};
use crate::state::AppState;
use std::path::Path;
use tauri::State;

/// A single task item managed by the agent.
#[derive(Debug, Clone, serde::Serialize, serde::Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct TaskItem {
    pub id: String,
    pub title: String,
    pub description: String,
    pub journal: Vec<String>,
    pub status: String, // "todo" | "doing" | "done"
}

/// Read all records from a JSONL file and find the LAST SessionRecord::Tasks,
/// returning its deserialized tasks (or empty vec if none found).
pub fn load_last_tasks(path: &Path) -> Result<Vec<TaskItem>, String> {
    let records = load_records(path)?;
    let last = records
        .into_iter()
        .rev()
        .find(|r| matches!(r, SessionRecord::Tasks { .. }));
    match last {
        Some(SessionRecord::Tasks { tasks_json, .. }) => {
            serde_json::from_str(&tasks_json).map_err(|e| format!("parse tasks from session: {e}"))
        }
        _ => Ok(Vec::new()),
    }
}

/// Serialize tasks and append a SessionRecord::Tasks line to the JSONL.
pub fn append_tasks(path: &Path, tasks: &[TaskItem]) -> Result<(), String> {
    let tasks_json = serde_json::to_string(tasks).map_err(|e| format!("serialize tasks: {e}"))?;
    let record = SessionRecord::Tasks {
        tasks_json,
        ts: std::time::SystemTime::now()
            .duration_since(std::time::UNIX_EPOCH)
            .map(|d| d.as_millis() as u64)
            .unwrap_or(0),
    };
    let line = serde_json::to_string(&record).map_err(|e| format!("serialize record: {e}"))?;
    let mut file = std::fs::OpenOptions::new()
        .create(true)
        .append(true)
        .open(path)
        .map_err(|e| format!("open session file: {e}"))?;
    use std::io::Write;
    writeln!(file, "{line}").map_err(|e| format!("write session file: {e}"))?;
    Ok(())
}

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
