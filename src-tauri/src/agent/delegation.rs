//! What crosses between a session and the subagents it spawns: the report
//! format an explore subagent is asked for, the cap on the report the parent
//! carries, and the shared brief a code subagent starts from.
//!
//! Lean-harness Phase 3 (plan C3). Since decision D1 the Builder session edits
//! files itself, and a subagent is the exception: an explore subagent keeps its
//! fresh context and hands back a condensed report — Anthropic's multi-agent
//! write-up puts the useful size at 1–2k tokens — and the parent sees that
//! report, not the transcript. A code subagent is still offered for parallel
//! changes to disjoint files; two writers that start from different facts
//! write code that disagrees (Cognition, "Don't build multi-agents"), so the
//! harness hands each one the same short brief, the cheapest form of "share
//! the trace".

use crate::agent::persist::TaskItem;
use crate::agent::tools::ToolContext;

/// Asked of every explore subagent in its system prompt. The structure lets
/// the parent read the answer first and the evidence after, and the file list
/// is what lets it go straight to the lines instead of re-exploring.
pub(crate) const EXPLORE_REPORT_FORMAT: &str = "\n\nREPORT: your final message is all the parent \
sees of your work. Keep it under about 1,500 tokens, in three sections:\n\
## Findings\nThe answer to your goal, most important first.\n\
## Files\nOne `path:line` per line, with what is there.\n\
## Open questions\nWhat you could not establish, or `none`.\n\
A longer report is cut in the middle.";

/// Appended to the goal of a subagent spawned with a `section` (decision
/// D9): the section is the deliverable and the parent reads it in the
/// document, so the report that comes back through the parent's context can
/// be two lines instead of the three-section explore report.
pub(crate) fn section_brief(id: &str, topic: &str) -> String {
    format!(
        "\n\nYOUR SECTION: {id} ({topic}) of the session's document. Write it with doc_write \
         {{\"id\": \"{id}\", \"body\": \"...\"}} (status 'done' when finished): it is your \
         deliverable, and the parent reads it there. Your final message can then be one or two \
         lines: what the section covers and what is still open."
    )
}

/// The most of a subagent's report the parent carries, in characters. A
/// report the ~1,500-token request above produces fits with room to spare;
/// past it the middle goes, never the head or the tail.
pub(crate) const REPORT_CHARS: usize = 6_000;

fn omission_marker(omitted: usize) -> String {
    format!("[… {omitted} characters of this report omitted …]")
}

/// `report`, cut to [`REPORT_CHARS`] keeping its head (two thirds of what is
/// kept: the findings come first) and its tail (the files and open
/// questions), each cut on a line boundary when one is near, around a marker
/// that says how many characters were omitted.
pub(crate) fn cap_report(report: &str) -> String {
    let total = report.chars().count();
    if total <= REPORT_CHARS {
        return report.to_string();
    }
    // The marker at its longest (it never omits more than the whole), and the
    // two newlines around it.
    let budget = REPORT_CHARS - omission_marker(total).chars().count() - 2;
    let head_chars = budget * 2 / 3;
    let tail_chars = budget - head_chars;
    let byte_at = |chars: usize| {
        report
            .char_indices()
            .nth(chars)
            .map_or(report.len(), |(i, _)| i)
    };
    let mut head_end = byte_at(head_chars);
    let mut tail_start = byte_at(total - tail_chars);
    // Snap to whole lines when that gives up less than half of either part.
    if let Some(nl) = report[..head_end].rfind('\n')
        && report[..nl].chars().count() >= head_chars / 2
    {
        head_end = nl;
    }
    if let Some(nl) = report[tail_start..].find('\n')
        && report[tail_start..tail_start + nl].chars().count() < tail_chars / 2
    {
        tail_start += nl + 1;
    }
    let omitted = total - report[..head_end].chars().count() - report[tail_start..].chars().count();
    format!(
        "{}\n{}\n{}",
        &report[..head_end],
        omission_marker(omitted),
        &report[tail_start..]
    )
}

/// The most file paths a brief lists.
pub(crate) const BRIEF_PATHS: usize = 40;

/// The most of the current task's description a brief repeats: the subagent's
/// goal carries the spec; this is orientation.
const BRIEF_DESCRIPTION_CHARS: usize = 300;

fn clip(text: &str, max: usize) -> String {
    let text = text.split_whitespace().collect::<Vec<_>>().join(" ");
    if text.chars().count() <= max {
        return text;
    }
    let mut cut: String = text.chars().take(max).collect();
    cut.push('…');
    cut
}

/// `path` relative to the workspace root when it lies inside it.
fn relative<'a>(path: &'a str, workspace_root: Option<&str>) -> &'a str {
    workspace_root
        .map(|root| root.trim_end_matches('/'))
        .filter(|root| !root.is_empty())
        .and_then(|root| path.strip_prefix(root))
        .and_then(|rest| rest.strip_prefix('/'))
        .unwrap_or(path)
}

/// The brief for the code subagents `ctx`'s session is about to spawn: its
/// task in progress, the latest plan in the plans directory, and every file
/// read through `read_file` in this run. The read tracker is the run's, and
/// subagents share it, so the paths include what earlier subagents read —
/// facts the session has, whoever fetched them.
pub(crate) async fn brief_for(ctx: &ToolContext) -> Option<String> {
    let task = ctx
        .session_store_path
        .as_deref()
        .and_then(|p| crate::agent::persist::load_last_tasks(std::path::Path::new(p)).ok())
        .and_then(|tasks| tasks.into_iter().find(|t| t.status == "doing"));
    let plan = ctx.workspace_root.as_deref().and_then(|root| {
        crate::agent::tools::write_plan::latest_plan_path(root, ctx.plan_save_path.as_deref())
    });
    let read_paths: Vec<String> = ctx
        .read_tracker
        .lock()
        .await
        .files
        .keys()
        .cloned()
        .collect();
    compose_brief(
        task.as_ref(),
        plan.as_deref().and_then(|p| p.to_str()),
        &read_paths,
        ctx.workspace_root.as_deref(),
    )
}

/// The `<brief>` block prepended to a code subagent's goal, or `None` when
/// there is nothing to share: the task in progress (title and the start of
/// its description), the latest plan's path, and the files read in this run
/// as paths only — sorted, so the same reads give the same bytes, and at most
/// [`BRIEF_PATHS`] of them with a count of the rest.
pub(crate) fn compose_brief(
    task: Option<&TaskItem>,
    plan: Option<&str>,
    read_paths: &[String],
    workspace_root: Option<&str>,
) -> Option<String> {
    let task = task.filter(|t| t.status == "doing");
    if task.is_none() && plan.is_none() && read_paths.is_empty() {
        return None;
    }
    let mut out = String::from(
        "<brief>\nShared context from the session that spawned you (paths only: read what you need).\n",
    );
    if let Some(t) = task {
        out.push_str(&format!(
            "Current task: {} — {}\n",
            clip(&t.title, 120),
            clip(&t.description, BRIEF_DESCRIPTION_CHARS)
        ));
    }
    if let Some(plan) = plan {
        out.push_str(&format!("Plan: {}\n", relative(plan, workspace_root)));
    }
    if !read_paths.is_empty() {
        let mut paths: Vec<&str> = read_paths
            .iter()
            .map(|p| relative(p, workspace_root))
            .collect();
        paths.sort_unstable();
        paths.dedup();
        out.push_str("Files the session has read:\n");
        for path in paths.iter().take(BRIEF_PATHS) {
            out.push_str(&format!("- {path}\n"));
        }
        if paths.len() > BRIEF_PATHS {
            out.push_str(&format!("({} more)\n", paths.len() - BRIEF_PATHS));
        }
    }
    out.push_str("</brief>");
    Some(out)
}

#[cfg(test)]
mod tests {
    use super::*;

    fn lines(n: usize) -> String {
        (0..n)
            .map(|i| format!("line {i:05}: a finding the subagent wrote down\n"))
            .collect()
    }

    /// The marker a capped report carries, and the head and tail around it.
    fn split_capped(capped: &str) -> (&str, usize, &str) {
        let start = capped.find("[… ").expect("an omission marker");
        let end = capped[start..].find(" …]").expect("the marker's end") + start + " …]".len();
        let marker = &capped[start..end];
        let omitted: usize = marker
            .trim_start_matches("[… ")
            .split(' ')
            .next()
            .unwrap()
            .replace(',', "")
            .parse()
            .expect("the marker counts what it omitted");
        (&capped[..start], omitted, &capped[end..])
    }

    #[test]
    fn a_report_within_the_cap_reaches_the_parent_unchanged() {
        let short = lines(10);
        assert_eq!(cap_report(&short), short);
        let exact = "x".repeat(REPORT_CHARS);
        assert_eq!(cap_report(&exact), exact);
    }

    #[test]
    fn a_long_report_keeps_its_head_and_tail_and_says_what_was_omitted() {
        let report = lines(500);
        let total = report.chars().count();
        assert!(total > 3 * REPORT_CHARS);
        let capped = cap_report(&report);
        assert!(
            capped.chars().count() <= REPORT_CHARS,
            "{} chars",
            capped.chars().count()
        );
        assert!(capped.starts_with("line 00000:"), "the head is kept");
        assert!(
            capped
                .trim_end()
                .ends_with("line 00499: a finding the subagent wrote down")
        );
        let (head, omitted, tail) = split_capped(&capped);
        assert!(capped.contains("characters of this report omitted"));
        assert_eq!(
            head.trim_end_matches('\n').chars().count()
                + tail.trim_start_matches('\n').chars().count()
                + omitted,
            total,
            "the marker counts exactly what is missing"
        );
        // Cut on line boundaries, so no finding is left half-written.
        // `head` ends with the separator `cap_report` adds, so the check is
        // on the text before it: a whole line of the report, ending where
        // that line ended.
        let kept = head
            .strip_suffix('\n')
            .expect("the separator before the marker");
        assert!(
            kept.ends_with("a finding the subagent wrote down")
                && report.starts_with(kept)
                && report[kept.len()..].starts_with('\n'),
            "{:?}",
            &kept[kept.len() - 40..]
        );
        assert!(
            tail.trim_start_matches('\n').starts_with("line "),
            "{:?}",
            &tail[..40]
        );
        // Most of what is kept is the head: the answer comes first.
        assert!(head.len() > tail.len());
    }

    #[test]
    fn a_report_without_newlines_or_in_multibyte_text_is_cut_on_char_boundaries() {
        for report in [
            "é".repeat(10_000),
            "x".repeat(10_000),
            "日本語".repeat(4_000),
        ] {
            let capped = cap_report(&report);
            assert!(capped.chars().count() <= REPORT_CHARS);
            let (head, omitted, tail) = split_capped(&capped);
            assert_eq!(
                head.trim_end_matches('\n').chars().count()
                    + tail.trim_start_matches('\n').chars().count()
                    + omitted,
                report.chars().count()
            );
        }
    }

    fn task(title: &str, description: &str, status: &str) -> TaskItem {
        TaskItem {
            id: "t1".into(),
            title: title.into(),
            description: description.into(),
            journal: vec![],
            status: status.into(),
        }
    }

    #[test]
    fn with_nothing_to_share_there_is_no_brief() {
        assert_eq!(compose_brief(None, None, &[], Some("/ws")), None);
    }

    #[test]
    fn a_brief_carries_the_task_the_plan_and_the_files_read_as_sorted_paths() {
        let doing = task(
            "Add the discount",
            "Members get 10% off at checkout.",
            "doing",
        );
        let paths = vec![
            "/ws/src/b.rs".to_string(),
            "/ws/src/a.rs".to_string(),
            "README.md".to_string(),
            "/elsewhere/notes.md".to_string(),
        ];
        let brief = compose_brief(
            Some(&doing),
            Some("/ws/.claudinio/plans/discount.md"),
            &paths,
            Some("/ws"),
        )
        .expect("a brief");
        assert!(brief.starts_with("<brief>\n"), "{brief}");
        assert!(brief.ends_with("\n</brief>"), "{brief}");
        assert!(brief.contains("Add the discount"), "{brief}");
        assert!(
            brief.contains("Members get 10% off at checkout."),
            "{brief}"
        );
        assert!(brief.contains(".claudinio/plans/discount.md"), "{brief}");
        let listed: Vec<&str> = brief.lines().filter_map(|l| l.strip_prefix("- ")).collect();
        assert_eq!(
            listed,
            ["/elsewhere/notes.md", "README.md", "src/a.rs", "src/b.rs"],
            "workspace paths relative to the root, all in one sorted order"
        );
        // Paths only: nothing the session read is copied in.
        assert!(!brief.contains("fn "), "{brief}");
    }

    #[test]
    fn a_brief_is_the_same_bytes_whatever_order_the_reads_came_in() {
        let mut paths: Vec<String> = (0..12).map(|i| format!("/ws/src/m{i:02}.rs")).collect();
        let a = compose_brief(None, None, &paths, Some("/ws"));
        paths.reverse();
        paths.swap(2, 7);
        assert_eq!(compose_brief(None, None, &paths, Some("/ws")), a);
    }

    #[test]
    fn a_brief_lists_at_most_forty_paths_and_counts_the_rest() {
        let paths: Vec<String> = (0..55).map(|i| format!("/ws/src/m{i:02}.rs")).collect();
        let brief = compose_brief(None, None, &paths, Some("/ws")).unwrap();
        let listed = brief.lines().filter(|l| l.starts_with("- ")).count();
        assert_eq!(listed, BRIEF_PATHS);
        assert!(brief.contains("- src/m39.rs") && !brief.contains("src/m40.rs"));
        assert!(brief.contains("(15 more)"), "{brief}");
    }

    #[test]
    fn a_brief_names_only_a_task_in_progress_and_keeps_its_description_short() {
        let todo = task("Later", "not started", "todo");
        assert_eq!(compose_brief(Some(&todo), None, &[], None), None);
        let long = task("Big one", &"word ".repeat(400), "doing");
        let brief = compose_brief(Some(&long), None, &[], None).unwrap();
        assert!(
            brief.chars().count() < 800,
            "{} chars",
            brief.chars().count()
        );
        assert!(brief.contains("Big one") && brief.contains('…'));
    }
}
