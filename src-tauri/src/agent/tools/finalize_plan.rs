//! finalize_plan — feed a plan `.md` its Implementation Log when a build finishes.
//!
//! Once the Builder has implemented a plan, this appends an
//! `## Implementation Log — <ts>` section to the plan file recording the
//! **changed files**, the **commit(s)** landed since work began, and a
//! **journal of findings** written by the agent — so plans accumulate data for
//! future reference. Git data is *read only*; the agent still commits itself.
//!
//! Two entry points share the same core (`run_finalize`):
//! * `execute` — the Builder-only tool (agent supplies the journal).
//! * `auto_finalize` — the harness fallback in the golden-completion loop, used
//!   when the model finished without calling the tool (journal is composed from
//!   the done-task journals) so the plan is *always* fed.

use serde::Deserialize;
use std::path::{Path, PathBuf};
use std::process::Command;

use crate::agent::persist::{self, SessionRecord, SessionStore};
use crate::agent::tools::{ToolContext, write_plan};
use crate::procutil::no_window;

#[derive(Deserialize)]
pub struct FinalizePlanArgs {
    /// Findings/decisions/gotchas from the implementation. The changed files
    /// and commits are recorded automatically, so this should focus on *why*
    /// and *what was learned*, not a file list.
    pub journal: String,
    /// Optional plan file to target (basename or path). Defaults to the most
    /// recently modified `*.md` in the plans directory.
    #[serde(default)]
    pub plan_file: Option<String>,
    /// Optional one-line summary of the implementation.
    #[serde(default)]
    pub summary: Option<String>,
}

/// What a finalize produced — returned so the caller can persist / report it.
pub struct FinalizeOutcome {
    pub plan_file: String,
    pub commits: Vec<String>,
    pub files: Vec<String>,
}

/// The current git HEAD sha for `root`, or None when not a git repo / git
/// unavailable. Best-effort; used to anchor the diff window at run start.
pub fn git_head(root: &str) -> Option<String> {
    let mut cmd = Command::new("git");
    cmd.arg("-C").arg(root).args(["rev-parse", "HEAD"]);
    no_window(&mut cmd);
    let out = cmd.output().ok()?;
    if !out.status.success() {
        return None;
    }
    let sha = String::from_utf8_lossy(&out.stdout).trim().to_string();
    if sha.is_empty() { None } else { Some(sha) }
}

/// `<status>\t<path>` lines for what changed since `base`. Falls back to the
/// working-tree porcelain status when no base is known or the range fails.
fn changed_files(root: &str, base: Option<&str>) -> Vec<String> {
    if let Some(base) = base
        && let Some(lines) =
            run_git_lines(root, &["diff", "--name-status", &format!("{base}..HEAD")])
        && !lines.is_empty()
    {
        return lines;
    }
    // Fallback: uncommitted working-tree changes.
    run_git_lines(root, &["status", "--porcelain"]).unwrap_or_default()
}

/// Short `<sha> <subject>` lines for commits landed since `base`. Empty when
/// no base is known (we cannot bound the range) or git is unavailable.
fn commits(root: &str, base: Option<&str>) -> Vec<String> {
    let base = match base {
        Some(b) => b,
        None => return Vec::new(),
    };
    run_git_lines(
        root,
        &["log", &format!("{base}..HEAD"), "--pretty=format:%h %s"],
    )
    .unwrap_or_default()
}

/// Run a git subcommand and split stdout into non-empty trimmed lines.
/// None when git errors (not a repo, bad range, git missing).
fn run_git_lines(root: &str, args: &[&str]) -> Option<Vec<String>> {
    let mut cmd = Command::new("git");
    cmd.arg("-C").arg(root).args(args);
    no_window(&mut cmd);
    let out = cmd.output().ok()?;
    if !out.status.success() {
        return None;
    }
    Some(
        String::from_utf8_lossy(&out.stdout)
            .lines()
            .map(|l| l.trim().to_string())
            .filter(|l| !l.is_empty())
            .collect(),
    )
}

/// Build the markdown section appended to the plan. Pure/testable.
///
/// `task_notes` are the rolled-up per-task journals (what was actually done
/// during the plan) — the plan is fed both the agent's findings journal and
/// this task-by-task record.
pub fn build_implementation_log(
    now: &str,
    summary: Option<&str>,
    commits: &[String],
    files: &[String],
    journal: &str,
    task_notes: &[String],
) -> String {
    let files_line = if files.is_empty() {
        "_(none detected)_".to_string()
    } else {
        files.join(", ")
    };
    let commits_line = if commits.is_empty() {
        "_(git unavailable or none)_".to_string()
    } else {
        commits.join(", ")
    };
    let mut s = format!("\n\n## Implementation Log — {now}\n");
    if let Some(sum) = summary.filter(|s| !s.trim().is_empty()) {
        s.push_str(&format!("**Summary:** {sum}\n"));
    }
    s.push_str(&format!("**Changed files:** {files_line}\n"));
    s.push_str(&format!("**Commits:** {commits_line}\n"));
    s.push_str(&format!("**Journal:** {}\n", journal.trim()));
    if !task_notes.is_empty() {
        s.push_str("\n**Task journal:**\n");
        for note in task_notes {
            s.push_str(&format!("- {note}\n"));
        }
    }
    s
}

/// Resolve the plans directory for this context.
fn plans_dir(ctx: &ToolContext) -> Result<PathBuf, String> {
    let root = ctx
        .workspace_root
        .as_ref()
        .ok_or("finalize_plan requires an open workspace")?;
    Ok(write_plan::plans_dir(root, ctx.plan_save_path.as_deref()))
}

/// The most recently modified `*.md` in the plans directory, if any.
pub fn latest_plan_file(ctx: &ToolContext) -> Option<PathBuf> {
    let dir = plans_dir(ctx).ok()?;
    let mut newest: Option<(std::time::SystemTime, PathBuf)> = None;
    for entry in std::fs::read_dir(&dir).ok()?.flatten() {
        let path = entry.path();
        if path.extension().and_then(|e| e.to_str()) != Some("md") {
            continue;
        }
        let mtime = entry
            .metadata()
            .and_then(|m| m.modified())
            .unwrap_or(std::time::UNIX_EPOCH);
        if newest.as_ref().map(|(t, _)| mtime > *t).unwrap_or(true) {
            newest = Some((mtime, path));
        }
    }
    newest.map(|(_, p)| p)
}

/// Resolve the target plan file from an optional user-supplied name/path.
fn resolve_plan_file(ctx: &ToolContext, plan_file: Option<&str>) -> Result<PathBuf, String> {
    match plan_file {
        Some(name) if !name.is_empty() => {
            let candidate = PathBuf::from(name);
            let path = if candidate.is_absolute() {
                candidate
            } else if name.contains('/') {
                // A relative path from the workspace root.
                let root = ctx.workspace_root.as_deref().unwrap_or("");
                PathBuf::from(root).join(name)
            } else {
                // A bare basename resolved inside the plans directory.
                plans_dir(ctx)?.join(name)
            };
            if path.exists() {
                Ok(path)
            } else {
                Err(format!("plan file not found: {}", path.to_string_lossy()))
            }
        }
        _ => latest_plan_file(ctx)
            .ok_or_else(|| "no plan file found in the plans directory".to_string()),
    }
}

/// The base commit for the diff window: the earliest one recorded in the
/// session (anchored at the true start of the plan's work), falling back to the
/// commit captured at this run's start.
fn resolve_base(ctx: &ToolContext) -> Option<String> {
    if let Some(path) = ctx.session_store_path.as_deref()
        && let Ok(records) = persist::load_records(Path::new(path))
        && let Some(sha) = persist::earliest_base_commit(&records)
    {
        return Some(sha);
    }
    ctx.base_commit.clone()
}

/// Core: append the Implementation Log to `plan_path`, persist a
/// `PlanFinalized` record, and return the outcome. Shared by the tool and the
/// harness fallback.
fn run_finalize(
    ctx: &ToolContext,
    plan_path: &Path,
    summary: Option<&str>,
    journal: &str,
) -> Result<FinalizeOutcome, String> {
    let root = ctx
        .workspace_root
        .as_deref()
        .ok_or("finalize_plan requires an open workspace")?;
    let base = resolve_base(ctx);
    let commits = commits(root, base.as_deref());
    let files = changed_files(root, base.as_deref());
    let task_notes = collect_task_notes(ctx);
    let now = chrono::Local::now().format("%Y-%m-%d %H:%M").to_string();

    let section = build_implementation_log(&now, summary, &commits, &files, journal, &task_notes);

    let mut existing = std::fs::read_to_string(plan_path).unwrap_or_default();
    existing.push_str(&section);
    std::fs::write(plan_path, &existing).map_err(|e| format!("append plan log: {e}"))?;

    // Persist an observability record (best-effort; the log is already on disk).
    if let Some(path) = ctx.session_store_path.as_deref() {
        let store = SessionStore {
            path: PathBuf::from(path),
        };
        store.try_append(&SessionRecord::PlanFinalized {
            plan_file: plan_path.to_string_lossy().to_string(),
            commits: commits.clone(),
            files_changed: files.clone(),
            ts: persist::now_ms(),
        });
    }

    Ok(FinalizeOutcome {
        plan_file: plan_path.to_string_lossy().to_string(),
        commits,
        files,
    })
}

/// The Builder-only tool entry point.
pub fn execute(args: FinalizePlanArgs, ctx: &ToolContext) -> Result<String, String> {
    let plan_path = resolve_plan_file(ctx, args.plan_file.as_deref())?;
    let outcome = run_finalize(ctx, &plan_path, args.summary.as_deref(), &args.journal)?;
    Ok(format!(
        "Implementation Log appended to {} ({} commit(s), {} changed file(s) recorded).",
        outcome.plan_file,
        outcome.commits.len(),
        outcome.files.len()
    ))
}

/// Harness fallback: finalize the most recent plan when the model finished
/// without calling the tool. Composes the journal from done-task journals so
/// the plan is still fed. Returns None when there is no plan to finalize.
pub fn auto_finalize(ctx: &ToolContext) -> Option<FinalizeOutcome> {
    let plan_path = latest_plan_file(ctx)?;
    // The detailed record lives in the Task journal section (built inside
    // run_finalize); the top-level journal is a short note for the fallback.
    let journal = "Auto-recorded by the harness (finalize_plan was not called). \
                   See the Task journal below for what was done.";
    run_finalize(
        ctx,
        &plan_path,
        Some("Auto-recorded by the harness."),
        journal,
    )
    .ok()
}

/// Roll up the journals of completed tasks into "<title>: <notes>" lines — the
/// task-by-task record of what was done during the plan. Shared by the tool
/// and the fallback so the plan always carries the task journal.
fn collect_task_notes(ctx: &ToolContext) -> Vec<String> {
    let Some(path) = ctx.session_store_path.as_deref() else {
        return Vec::new();
    };
    let tasks = crate::agent::persist::load_last_tasks(Path::new(path)).unwrap_or_default();
    tasks
        .iter()
        .filter(|t| t.status == "done" && !t.journal.is_empty())
        .map(|t| format!("{}: {}", t.title, t.journal.join("; ")))
        .collect()
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::sync::Arc;
    use tokio::sync::Mutex;

    fn ctx_for(root: &str, plan_save_path: Option<&str>) -> ToolContext {
        ToolContext {
            db_path: None,
            lsp_manager: None,
            workspace_root: Some(root.to_string()),
            embedding_model: Arc::new(Mutex::new(None)),
            session_store_path: None,
            read_tracker: Arc::new(Mutex::new(crate::agent::tools::ReadTracker::default())),
            interrupt: None,
            agent_config: None,
            plan_save_path: plan_save_path.map(|s| s.to_string()),
            base_commit: None,
            auto_approve_git: false,
            mcp: None,
            mode_ctl: None,
            index_progress: None,
            records_cache: Arc::new(std::sync::Mutex::new(lru::LruCache::new(
                std::num::NonZeroUsize::new(1).unwrap(),
            ))),
        }
    }

    fn tmp_workspace(name: &str) -> PathBuf {
        let p =
            std::env::temp_dir().join(format!("claudinio_finalize_{name}_{}", std::process::id()));
        let _ = std::fs::remove_dir_all(&p);
        std::fs::create_dir_all(p.join(".claudinio").join("plans")).unwrap();
        p
    }

    fn write_plan_md(root: &Path, name: &str, body: &str) -> PathBuf {
        let path = root.join(".claudinio").join("plans").join(name);
        std::fs::write(&path, body).unwrap();
        path
    }

    #[test]
    fn build_log_formats_sections() {
        let log = build_implementation_log(
            "2026-07-09 14:32",
            Some("did the thing"),
            &["abc123 feat: x".to_string()],
            &["M\tsrc/foo.rs".to_string()],
            "learned that y",
            &["Task A: did x; verified y".to_string()],
        );
        assert!(log.contains("## Implementation Log — 2026-07-09 14:32"));
        assert!(log.contains("**Summary:** did the thing"));
        assert!(log.contains("**Changed files:** M\tsrc/foo.rs"));
        assert!(log.contains("**Commits:** abc123 feat: x"));
        assert!(log.contains("**Journal:** learned that y"));
        assert!(log.contains("**Task journal:**"));
        assert!(log.contains("- Task A: did x; verified y"));
    }

    #[test]
    fn build_log_handles_empty_git() {
        let log = build_implementation_log("t", None, &[], &[], "j", &[]);
        assert!(log.contains("_(none detected)_"));
        assert!(log.contains("_(git unavailable or none)_"));
        assert!(
            !log.contains("**Task journal:**"),
            "no section when no notes"
        );
    }

    #[test]
    fn execute_appends_not_overwrites() {
        let root = tmp_workspace("append");
        let plan = write_plan_md(&root, "2026-07-09_x.md", "# Original plan\nbody\n");
        let ctx = ctx_for(root.to_str().unwrap(), None);
        let args = FinalizePlanArgs {
            journal: "found a gotcha".into(),
            plan_file: None,
            summary: None,
        };
        let msg = execute(args, &ctx).expect("finalize should succeed");
        assert!(msg.contains("Implementation Log appended"));
        let content = std::fs::read_to_string(&plan).unwrap();
        assert!(content.starts_with("# Original plan"), "original preserved");
        assert!(content.contains("## Implementation Log"));
        assert!(content.contains("found a gotcha"));
        let _ = std::fs::remove_dir_all(&root);
    }

    #[test]
    fn resolves_newest_plan_by_default() {
        let root = tmp_workspace("newest");
        write_plan_md(&root, "2026-07-01_old.md", "old\n");
        // Ensure a later mtime for the newer file.
        std::thread::sleep(std::time::Duration::from_millis(20));
        let newer = write_plan_md(&root, "2026-07-09_new.md", "new\n");
        let ctx = ctx_for(root.to_str().unwrap(), None);
        let picked = latest_plan_file(&ctx).unwrap();
        assert_eq!(picked, newer);
        let _ = std::fs::remove_dir_all(&root);
    }

    #[test]
    fn explicit_plan_file_basename() {
        let root = tmp_workspace("explicit");
        write_plan_md(&root, "2026-07-01_old.md", "old\n");
        let target = write_plan_md(&root, "2026-07-05_target.md", "target\n");
        let ctx = ctx_for(root.to_str().unwrap(), None);
        let args = FinalizePlanArgs {
            journal: "j".into(),
            plan_file: Some("2026-07-05_target.md".into()),
            summary: None,
        };
        execute(args, &ctx).expect("finalize should succeed");
        let content = std::fs::read_to_string(&target).unwrap();
        assert!(content.contains("## Implementation Log"));
        let _ = std::fs::remove_dir_all(&root);
    }

    #[test]
    fn errors_when_no_plan_exists() {
        let root = tmp_workspace("noplan");
        let ctx = ctx_for(root.to_str().unwrap(), None);
        let args = FinalizePlanArgs {
            journal: "j".into(),
            plan_file: None,
            summary: None,
        };
        let err = execute(args, &ctx).unwrap_err();
        assert!(err.contains("no plan file found"), "got: {err}");
        let _ = std::fs::remove_dir_all(&root);
    }

    #[test]
    fn git_log_and_diff_captured_in_repo() {
        let root = tmp_workspace("gitrepo");
        let r = root.to_str().unwrap();
        // init a repo with one base commit
        let git = |args: &[&str]| {
            Command::new("git")
                .arg("-C")
                .arg(r)
                .args(args)
                .output()
                .unwrap()
        };
        if !git(&["init", "-q"]).status.success() {
            eprintln!("git not available; skipping");
            let _ = std::fs::remove_dir_all(&root);
            return;
        }
        git(&["config", "user.email", "t@t.t"]);
        git(&["config", "user.name", "t"]);
        std::fs::write(root.join("a.txt"), "1").unwrap();
        git(&["add", "-A"]);
        git(&["commit", "-qm", "base"]);
        let base = git_head(r).unwrap();
        // make a change + commit after base
        std::fs::write(root.join("b.txt"), "2").unwrap();
        git(&["add", "-A"]);
        git(&["commit", "-qm", "add b"]);

        let files = changed_files(r, Some(&base));
        assert!(
            files.iter().any(|f| f.contains("b.txt")),
            "files: {files:?}"
        );
        let cs = commits(r, Some(&base));
        assert!(cs.iter().any(|c| c.contains("add b")), "commits: {cs:?}");
        let _ = std::fs::remove_dir_all(&root);
    }
}
