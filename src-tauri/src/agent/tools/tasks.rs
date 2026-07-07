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

/// Create golden tasks from parsed <goal> tags.
/// Each goal generates two golden tasks: one for planning, one for execution.
pub fn create_golden_tasks(goals: &[String]) -> Vec<TaskItem> {
    let mut tasks = Vec::new();
    for goal in goals {
        let slug = slugify(goal);
        tasks.push(TaskItem {
            id: format!("golden-{}-0", slug),
            title: format!("Plan: {}", goal),
            description: format!("Create a detailed plan to achieve the goal: {}", goal),
            journal: Vec::new(),
            status: "todo".to_string(),
        });
        tasks.push(TaskItem {
            id: format!("golden-{}-1", slug),
            title: format!("Execute: {}", goal),
            description: format!("Execute the plan and verify the goal is met: {}", goal),
            journal: Vec::new(),
            status: "todo".to_string(),
        });
    }
    tasks
}

/// Check if a task is a golden task (id starts with the golden prefix).
pub fn is_golden(task: &TaskItem) -> bool {
    task.id.starts_with(crate::agent::session::GOLDEN_TASK_PREFIX)
}

/// Get all golden tasks that are not yet 'done'.
pub fn golden_tasks_remaining(tasks: &[TaskItem]) -> Vec<&TaskItem> {
    tasks.iter().filter(|t| is_golden(t) && t.status != "done").collect()
}

/// Get IDs of golden tasks that are not yet 'done'.
pub fn golden_pending_ids(tasks: &[TaskItem]) -> Vec<String> {
    golden_tasks_remaining(tasks).into_iter().map(|t| t.id.clone()).collect()
}

/// Create a simple slug from a string: lowercase, replace non-alphanumeric with hyphens, truncate to 40 chars.
pub fn slugify(s: &str) -> String {
    let slug: String = s.to_lowercase()
        .chars()
        .map(|c| {
            if c.is_alphanumeric() || c == '-' || c == '_' {
                c
            } else if c.is_whitespace() {
                '-'
            } else {
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
mod golden_tests {
    use super::*;
    use crate::commands::tasks::TaskItem;

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
        let golden = TaskItem { id: "golden-test-0".into(), title: "test".into(), description: "".into(), journal: vec![], status: "todo".into() };
        let normal = TaskItem { id: "normal-task".into(), title: "test".into(), description: "".into(), journal: vec![], status: "todo".into() };
        assert!(is_golden(&golden));
        assert!(!is_golden(&normal));
    }

    #[test]
    fn test_golden_tasks_remaining() {
        let tasks = vec![
            TaskItem { id: "golden-a-0".into(), title: "test".into(), description: "".into(), journal: vec![], status: "todo".into() },
            TaskItem { id: "golden-a-1".into(), title: "test".into(), description: "".into(), journal: vec![], status: "done".into() },
            TaskItem { id: "normal-task".into(), title: "test".into(), description: "".into(), journal: vec![], status: "todo".into() },
        ];
        let remaining = golden_tasks_remaining(&tasks);
        assert_eq!(remaining.len(), 1);
        assert_eq!(remaining[0].id, "golden-a-0");
    }

    #[test]
    fn test_slugify() {
        assert_eq!(slugify("Code Coverage in 80%"), "code-coverage-in-80");
        assert_eq!(slugify("Hello World"), "hello-world");
        assert!(slugify("a very long string with many many characters that exceeds forty chars total").len() <= 40);
    }

    #[test]
    fn test_golden_pending_ids() {
        let tasks = vec![
            TaskItem { id: "golden-x-0".into(), title: "t".into(), description: "".into(), journal: vec![], status: "todo".into() },
            TaskItem { id: "golden-x-1".into(), title: "t".into(), description: "".into(), journal: vec![], status: "done".into() },
        ];
        let ids = golden_pending_ids(&tasks);
        assert_eq!(ids, vec!["golden-x-0"]);
    }
}
