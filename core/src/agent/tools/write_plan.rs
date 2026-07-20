use serde::Deserialize;
use std::path::PathBuf;

/// Write (or overwrite) a plan document at
/// `<workspace>/.claudinio/plans/YYYY-MM-DD_<slug>.md`.
///
/// Called twice per Brain session: first with the Solution Design
/// (requirements), then again with the same name and the FULL content plus
/// the `## Low-Level Design` section (technical spec) — `tasks_set` is gated
/// on that section existing.
#[derive(Deserialize)]
pub struct WritePlanArgs {
    pub name: String,
    pub content: String,
}

/// The heading `tasks_set` requires in the latest plan before Brain mode may
/// create tasks.
pub const LLD_HEADING: &str = "## Low-Level Design";

/// True when `content` contains a heading line equal to `heading` (trimmed,
/// ascii-case-insensitive) followed by at least one non-whitespace character
/// or a sub-heading (deeper `#` level) before the next heading at the same
/// or higher level (`<= target_level`) or EOF.
pub fn has_nonempty_section(content: &str, heading: &str) -> bool {
    let target_level = heading.chars().take_while(|c| *c == '#').count();
    let mut in_section = false;
    for line in content.lines() {
        let trimmed = line.trim();
        if trimmed.starts_with('#') {
            if trimmed.eq_ignore_ascii_case(heading) {
                in_section = true;
                continue;
            }
            if in_section {
                let line_level = trimmed.chars().take_while(|c| *c == '#').count();
                if line_level <= target_level {
                    return false; // end of section at same/higher level, no body
                } else {
                    return true; // sub-heading is body content
                }
            }
            continue;
        }
        if in_section && !trimmed.is_empty() {
            return true;
        }
    }
    false
}

/// Newest `.md` file in the plans dir by modified time (ties broken by name).
/// None when the dir is missing or holds no plan files.
pub fn latest_plan_path(workspace_root: &str, plan_save_path: Option<&str>) -> Option<PathBuf> {
    let dir = plans_dir(workspace_root, plan_save_path);
    let entries = std::fs::read_dir(&dir).ok()?;
    let mut best: Option<(std::time::SystemTime, PathBuf)> = None;
    for entry in entries.flatten() {
        let path = entry.path();
        if path.extension().and_then(|e| e.to_str()) != Some("md") {
            continue;
        }
        let mtime = entry
            .metadata()
            .and_then(|m| m.modified())
            .unwrap_or(std::time::SystemTime::UNIX_EPOCH);
        let candidate = (mtime, path);
        best = match best {
            Some(cur) if cur >= candidate => Some(cur),
            _ => Some(candidate),
        };
    }
    best.map(|(_, path)| path)
}

/// Turn a free-form plan name into a filesystem-safe slug.
pub fn slugify(name: &str) -> String {
    let mut slug = String::new();
    let mut last_dash = true; // suppress a leading dash
    for c in name.chars() {
        let c = c.to_ascii_lowercase();
        if c.is_ascii_alphanumeric() {
            slug.push(c);
            last_dash = false;
        } else if !last_dash {
            slug.push('-');
            last_dash = true;
        }
    }
    while slug.ends_with('-') {
        slug.pop();
    }
    if slug.is_empty() {
        slug.push_str("plan");
    }
    slug
}

/// Resolve the directory where plans are saved.
///
/// * If `plan_save_path` is `Some(path)`, the plans go to
///   `<workspace_root>/<path>` (relative) or `<path>` (absolute).
/// * Otherwise the default is `<workspace_root>/.claudinio/plans`.
pub fn plans_dir(workspace_root: &str, plan_save_path: Option<&str>) -> PathBuf {
    match plan_save_path {
        Some(path) if !path.is_empty() => {
            let candidate = PathBuf::from(path);
            if candidate.is_absolute() {
                candidate
            } else {
                PathBuf::from(workspace_root).join(path)
            }
        }
        _ => PathBuf::from(workspace_root).join(".claudinio").join("plans"),
    }
}

pub fn execute(args: WritePlanArgs, ctx: &crate::agent::tools::ToolContext) -> Result<String, String> {
    let root = ctx
        .workspace_root
        .as_ref()
        .ok_or("write_plan requires an open workspace")?;
    let dir = plans_dir(root, ctx.plan_save_path.as_deref());
    std::fs::create_dir_all(&dir).map_err(|e| format!("create plans dir: {e}"))?;

    let date = chrono::Local::now().format("%Y-%m-%d");
    let path = dir.join(format!("{date}_{}.md", slugify(&args.name)));
    std::fs::write(&path, &args.content).map_err(|e| format!("write plan: {e}"))?;
    let mut msg = format!(
        "Plan written to {} ({} bytes). Call write_plan again with the same name and the \
         full updated content to revise it.",
        path.to_string_lossy(),
        args.content.len()
    );
    // Soft validation: warn, never error — the first (Solution Design) call
    // legitimately has no Low-Level Design yet.
    let missing: Vec<&str> = ["## Context", "## Solution Design", "## Risks"]
        .into_iter()
        .filter(|h| !has_nonempty_section(&args.content, h))
        .collect();
    if !missing.is_empty() {
        msg.push_str(&format!(" Note: expected section(s) missing: {}.", missing.join(", ")));
    }
    if !has_nonempty_section(&args.content, LLD_HEADING) {
        msg.push_str(
            " Note: the plan has no '## Low-Level Design' section yet - tasks_set will reject \
             tasks until you call write_plan again with the full content including it.",
        );
    }
    // Auto-commit the plan file if configured and this is the final version (has LLD)
    let has_lld = has_nonempty_section(&args.content, LLD_HEADING);
    let auto_commit = ctx.agent_config.as_ref()
        .map(|c| c.auto_commit_plan)
        .unwrap_or(true); // default true when config is absent

    if has_lld && auto_commit {
        if let Some(root) = &ctx.workspace_root {
            let slug = slugify(&args.name);
            let commit_msg = format!("docs(plan): {slug}");
            // git add only the plan file (NOT -A)
            let add = std::process::Command::new("git")
                .arg("-C").arg(root)
                .arg("add")
                .arg(path.to_string_lossy().as_ref())
                .output();
            if add.is_ok() {
                let commit = std::process::Command::new("git")
                    .arg("-C").arg(root)
                    .arg("commit")
                    .arg("-m").arg(&commit_msg)
                    .output();
                match commit {
                    Ok(out) if out.status.success() => {
                        msg.push_str(&format!("\nPlan auto-committed: \"{commit_msg}\""));
                    }
                    Ok(out) => {
                        let stderr = String::from_utf8_lossy(&out.stderr);
                        eprintln!("[write_plan] git commit plan (non-zero): {stderr}");
                    }
                    Err(e) => {
                        eprintln!("[write_plan] git commit plan failed: {e}");
                    }
                }
            }
        }
    }
    Ok(msg)
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn slugify_basic() {
        assert_eq!(slugify("Modo Pensador / Constructor"), "modo-pensador-constructor");
        assert_eq!(slugify("  weird__name!! "), "weird-name");
        assert_eq!(slugify("///"), "plan");
    }

    #[test]
    fn plans_dir_default() {
        let got = plans_dir("/home/user/project", None);
        assert_eq!(got, PathBuf::from("/home/user/project/.claudinio/plans"));
    }

    #[test]
    fn plans_dir_custom_relative() {
        let got = plans_dir("/home/user/project", Some("docs/plans"));
        assert_eq!(got, PathBuf::from("/home/user/project/docs/plans"));
    }

    #[test]
    fn plans_dir_custom_absolute() {
        let got = plans_dir("/home/user/project", Some("/tmp/my-plans"));
        assert_eq!(got, PathBuf::from("/tmp/my-plans"));
    }

    #[test]
    fn plans_dir_empty_falls_back() {
        let got = plans_dir("/home/user/project", Some(""));
        assert_eq!(got, PathBuf::from("/home/user/project/.claudinio/plans"));
    }

    #[test]
    fn nonempty_section_present_with_body() {
        let content = "# Plan\n\n## Low-Level Design\nTouch src/foo.rs\n\n## Risks\nnone";
        assert!(has_nonempty_section(content, LLD_HEADING));
    }

    #[test]
    fn nonempty_section_empty_body_fails() {
        let content = "# Plan\n\n## Low-Level Design\n\n## Risks\nnone";
        assert!(!has_nonempty_section(content, LLD_HEADING));
    }

    #[test]
    fn nonempty_section_absent_fails() {
        let content = "# Plan\n\n## Solution Design\nstuff";
        assert!(!has_nonempty_section(content, LLD_HEADING));
    }

    #[test]
    fn nonempty_section_case_insensitive() {
        let content = "## LOW-LEVEL DESIGN\ndetails here";
        assert!(has_nonempty_section(content, LLD_HEADING));
    }

    #[test]
    fn nonempty_section_heading_as_last_line_fails() {
        let content = "# Plan\nintro\n## Low-Level Design";
        assert!(!has_nonempty_section(content, LLD_HEADING));
    }

    /// LLD starts with a sub-heading (###) — should count as body (the bug).
    #[test]
    fn nonempty_section_subheading_counts_as_body() {
        let content = "## Low-Level Design\n### Files to Change\n- src/foo.rs\n";
        assert!(has_nonempty_section(content, LLD_HEADING));
    }

    /// LLD followed by another ## heading before any body — empty section.
    #[test]
    fn nonempty_section_ends_at_same_level() {
        let content = "## Low-Level Design\n## Risks\nnone";
        assert!(!has_nonempty_section(content, LLD_HEADING));
    }

    /// LLD followed by a # heading before any body — ends at higher level, empty.
    #[test]
    fn nonempty_section_ends_at_higher_level() {
        let content = "## Low-Level Design\n# Other\nbody";
        assert!(!has_nonempty_section(content, LLD_HEADING));
    }

    /// LLD with only a sub-heading and no other text — sub-heading itself is body.
    #[test]
    fn nonempty_section_subheading_only_no_text() {
        let content = "## Low-Level Design\n### Files\n";
        assert!(has_nonempty_section(content, LLD_HEADING));
    }

    #[test]
    fn latest_plan_path_picks_newest() {
        let tmp = std::env::temp_dir().join(format!("wp-latest-{}", std::process::id()));
        let plans = tmp.join("docs/plans");
        std::fs::create_dir_all(&plans).unwrap();
        let old = plans.join("2026-07-10_old.md");
        let new = plans.join("2026-07-14_new.md");
        std::fs::write(&old, "old").unwrap();
        std::fs::write(&new, "new").unwrap();
        let past = std::time::SystemTime::now() - std::time::Duration::from_secs(3600);
        let f = std::fs::File::options().write(true).open(&old).unwrap();
        f.set_modified(past).unwrap();
        let got = latest_plan_path(tmp.to_str().unwrap(), Some("docs/plans"));
        assert_eq!(got, Some(new));
        std::fs::remove_dir_all(&tmp).ok();
    }

    #[test]
    fn latest_plan_path_missing_dir() {
        assert_eq!(latest_plan_path("/nonexistent/root", None), None);
    }

    fn ctx_with_root(root: &std::path::Path) -> crate::agent::tools::ToolContext {
        use std::sync::Arc;
        crate::agent::tools::ToolContext {
            db_path: None,
            lsp_manager: None,
            workspace_root: Some(root.to_string_lossy().to_string()),
            embedding_model: Arc::new(tokio::sync::Mutex::new(None)),
            session_store_path: None,
            read_tracker: Arc::new(tokio::sync::Mutex::new(
                crate::agent::tools::ReadTracker::default(),
            )),
            interrupt: None,
            agent_config: None,
            plan_save_path: None,
            base_commit: None,
            auto_approve_git: false,
            mcp: None,
            mode_ctl: None,
            index_progress: None,
            records_cache: Arc::new(std::sync::Mutex::new(lru::LruCache::new(std::num::NonZeroUsize::new(1).unwrap()))),
        }
    }

    #[test]
    fn execute_warns_when_lld_missing_and_not_when_present() {
        let root = std::env::temp_dir().join(format!("wp-warn-{}", std::process::id()));
        std::fs::create_dir_all(&root).unwrap();
        let ctx = ctx_with_root(&root);

        let sd_only = execute(
            WritePlanArgs {
                name: "test plan".into(),
                content: "## Context\nc\n## Solution Design\nsd\n## Risks\nr\n".into(),
            },
            &ctx,
        )
        .unwrap();
        assert!(sd_only.contains("no '## Low-Level Design' section yet"), "got: {sd_only}");

        let full = execute(
            WritePlanArgs {
                name: "test plan".into(),
                content: "## Context\nc\n## Solution Design\nsd\n## Risks\nr\n## Low-Level Design\nsrc/foo.rs\n".into(),
            },
            &ctx,
        )
        .unwrap();
        assert!(!full.contains("no '## Low-Level Design'"), "got: {full}");
        assert!(!full.contains("expected section(s) missing"), "got: {full}");
        std::fs::remove_dir_all(&root).ok();
    }
}
