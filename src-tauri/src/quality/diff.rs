//! Which lines this run actually changed.
//!
//! Coverage is measured against these lines rather than the whole repo. A
//! project-wide percentage barely moves when an agent adds forty lines, so it
//! is useless as a gate; the share of *new* lines that no test executes is
//! exactly the "code the model wrote and nothing checks" signal we want.
//!
//! It also keeps the gate fair: nobody has to pay off a legacy repo's coverage
//! debt to land a two-line fix.

use std::collections::{HashMap, HashSet};
use std::path::{Path, PathBuf};

use super::evidence::git;

/// Added or modified lines, keyed by absolute file path.
pub type ChangedLines = HashMap<PathBuf, HashSet<u32>>;

/// Lines changed since `base_commit` (defaults to HEAD), including uncommitted
/// work and untracked files.
///
/// Returns empty when the workspace is not a git repository — the coverage
/// layer then has nothing to require, which is the honest answer rather than a
/// fabricated one.
pub fn changed_lines(root: &Path, base_commit: Option<&str>) -> ChangedLines {
    let mut out: ChangedLines = HashMap::new();
    let base = base_commit
        .map(|s| s.to_string())
        .or_else(|| git(root, &["rev-parse", "HEAD"]));
    let Some(base) = base else {
        return out;
    };

    // `git diff <commit>` spans commits made since the base AND the current
    // working tree, so one call covers everything tracked. -U0 keeps the hunks
    // to their changed lines only.
    if let Some(patch) = git(root, &["diff", "-U0", &base, "--"]) {
        parse_unified_diff(&patch, root, &mut out);
    }

    // A brand-new file is entirely new code, and git shows it in no diff until
    // it is added — count all of its lines.
    if let Some(status) = git(root, &["status", "--porcelain=v1", "--untracked-files=all"]) {
        for rel in status.lines().filter_map(|l| l.strip_prefix("?? ")) {
            let path = root.join(rel.trim_matches('"'));
            let Ok(text) = std::fs::read_to_string(&path) else {
                continue; // binary or unreadable: nothing to cover
            };
            let lines = (1..=text.lines().count() as u32).collect::<HashSet<u32>>();
            if !lines.is_empty() {
                out.entry(path).or_default().extend(lines);
            }
        }
    }

    out
}

/// Pull `+` line numbers out of a unified diff. Only the post-image matters:
/// a deleted line cannot be covered by a test.
fn parse_unified_diff(patch: &str, root: &Path, out: &mut ChangedLines) {
    let mut current: Option<PathBuf> = None;
    for line in patch.lines() {
        if let Some(rest) = line.strip_prefix("+++ ") {
            current = match rest.trim() {
                "/dev/null" => None,
                p => Some(root.join(p.strip_prefix("b/").unwrap_or(p))),
            };
            continue;
        }
        let Some(rest) = line.strip_prefix("@@ ") else {
            continue;
        };
        let Some(ref path) = current else { continue };
        // "@@ -12,0 +13,4 @@ optional context"
        let Some(plus) = rest.split_whitespace().find(|t| t.starts_with('+')) else {
            continue;
        };
        let mut parts = plus[1..].split(',');
        let Some(start) = parts.next().and_then(|s| s.parse::<u32>().ok()) else {
            continue;
        };
        // A missing count means exactly one line, per the unified diff format.
        let count = parts
            .next()
            .and_then(|s| s.parse::<u32>().ok())
            .unwrap_or(1);
        if count == 0 {
            continue; // pure deletion
        }
        out.entry(path.clone())
            .or_default()
            .extend(start..start + count);
    }
}

/// Total number of changed lines across all files.
pub fn total_lines(changed: &ChangedLines) -> usize {
    changed.values().map(|s| s.len()).sum()
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn parses_added_lines_from_a_hunk_header() {
        let patch = "diff --git a/src/a.rs b/src/a.rs\n\
                     --- a/src/a.rs\n\
                     +++ b/src/a.rs\n\
                     @@ -12,0 +13,3 @@ fn thing()\n\
                     +one\n+two\n+three\n";
        let root = Path::new("/repo");
        let mut out = ChangedLines::new();
        parse_unified_diff(patch, root, &mut out);
        let lines = &out[&PathBuf::from("/repo/src/a.rs")];
        assert_eq!(lines, &HashSet::from([13, 14, 15]));
    }

    #[test]
    fn a_hunk_without_a_count_is_one_line() {
        let patch = "+++ b/x.rs\n@@ -3 +3 @@\n-old\n+new\n";
        let mut out = ChangedLines::new();
        parse_unified_diff(patch, Path::new("/r"), &mut out);
        assert_eq!(out[&PathBuf::from("/r/x.rs")], HashSet::from([3]));
    }

    #[test]
    fn pure_deletions_contribute_no_lines() {
        // "+0" means nothing was added — there is no new line to cover.
        let patch = "+++ b/x.rs\n@@ -5,3 +4,0 @@\n-a\n-b\n-c\n";
        let mut out = ChangedLines::new();
        parse_unified_diff(patch, Path::new("/r"), &mut out);
        assert!(out.is_empty(), "{out:?}");
    }

    #[test]
    fn a_deleted_file_is_skipped() {
        let patch = "+++ /dev/null\n@@ -1,3 +0,0 @@\n-a\n";
        let mut out = ChangedLines::new();
        parse_unified_diff(patch, Path::new("/r"), &mut out);
        assert!(out.is_empty());
    }

    #[test]
    fn several_files_in_one_patch() {
        let patch = "+++ b/a.rs\n@@ -0,0 +1,2 @@\n+x\n+y\n\
                     +++ b/b.ts\n@@ -0,0 +9,1 @@\n+z\n";
        let mut out = ChangedLines::new();
        parse_unified_diff(patch, Path::new("/r"), &mut out);
        assert_eq!(out.len(), 2);
        assert_eq!(total_lines(&out), 3);
    }

    #[test]
    fn no_git_repository_yields_no_changed_lines() {
        let root = std::env::temp_dir().join(format!("cq-diff-nogit-{}", std::process::id()));
        std::fs::create_dir_all(&root).unwrap();
        // Deliberately passing a bogus base: the point is that the harness
        // degrades to "nothing to require" instead of erroring.
        let changed = changed_lines(&root, Some("0000000000000000000000000000000000000000"));
        assert!(changed.is_empty());
        std::fs::remove_dir_all(&root).ok();
    }
}
