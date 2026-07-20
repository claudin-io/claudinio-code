//! Persistência pura de tarefas do agente (JSONL da sessão).
//!
//! Os wrappers `#[tauri::command]` (get_tasks/set_tasks/dismiss_golden_tasks)
//! ficam no desktop em `commands/tasks.rs` e delegam para estas funções.

use crate::agent::persist::{load_records, SessionRecord};
use std::path::Path;

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
    let last = records.into_iter().rev().find(|r| matches!(r, SessionRecord::Tasks { .. }));
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
