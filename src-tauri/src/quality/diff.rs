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

/// Extensions whose contents no test can execute. Used to decide whether a
/// session's changes are worth running the suite over.
///
/// A denylist rather than an allowlist of source extensions, deliberately: an
/// unfamiliar extension then counts as code and gets verified. Over-verifying
/// an unknown file type costs minutes; under-verifying it costs the guarantee.
const NON_EXECUTABLE_EXTENSIONS: &[&str] = &[
    "md", "mdx", "txt", "rst", "adoc", "png", "jpg", "jpeg", "gif", "svg", "webp", "ico", "bmp",
    "mp4", "webm", "mov", "mp3", "wav", "flac", "pdf", "woff", "woff2", "ttf", "otf",
];

/// Whether this session changed anything a test could execute.
///
/// Note that lockfiles and config are NOT excluded: a dependency bump or a
/// changed build setting can break a suite as surely as an edited function.
pub fn touches_source(root: &Path, base_commit: Option<&str>) -> bool {
    changed_files(root, base_commit)
        .iter()
        .any(|p| is_source(p))
}

/// Could a test conceivably execute this file's contents?
fn is_source(path: &Path) -> bool {
    path.extension()
        .and_then(|e| e.to_str())
        .map(|e| !NON_EXECUTABLE_EXTENSIONS.contains(&e.to_ascii_lowercase().as_str()))
        // No extension at all (Makefile, a shell script, a binary entry
        // point) — treat as code.
        .unwrap_or(true)
}

/// Paths changed since `base_commit`, including untracked files. Cheaper than
/// [`changed_lines`]: it never reads or parses a diff body.
pub fn changed_files(root: &Path, base_commit: Option<&str>) -> Vec<PathBuf> {
    let base = base_commit
        .map(|s| s.to_string())
        .or_else(|| git(root, &["rev-parse", "HEAD"]));
    let Some(base) = base else {
        return Vec::new();
    };
    let mut out: Vec<PathBuf> = git(root, &["diff", "--name-only", &base, "--"])
        .map(|s| s.lines().map(|l| root.join(l.trim())).collect())
        .unwrap_or_default();
    if let Some(status) = git(root, &["status", "--porcelain=v1", "--untracked-files=all"]) {
        for rel in status.lines().filter_map(|l| l.strip_prefix("?? ")) {
            out.push(root.join(rel.trim().trim_matches('"')));
        }
    }
    out.sort();
    out.dedup();
    out
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
    fn documentation_only_changes_are_not_source() {
        assert!(!is_source(Path::new("/r/README.md")));
        assert!(!is_source(Path::new("/r/docs/plan.MDX")));
        assert!(!is_source(Path::new("/r/assets/logo.svg")));
    }

    #[test]
    fn code_and_config_changes_are_source() {
        assert!(is_source(Path::new("/r/src/lib.rs")));
        assert!(is_source(Path::new("/r/src/App.tsx")));
        // A dependency bump can break a suite as surely as an edited function.
        assert!(is_source(Path::new("/r/pnpm-lock.yaml")));
        assert!(is_source(Path::new("/r/Cargo.toml")));
    }

    #[test]
    fn an_unknown_or_extensionless_file_counts_as_source() {
        // The denylist errs toward verifying: over-checking an unfamiliar file
        // type costs minutes, under-checking it costs the guarantee.
        assert!(is_source(Path::new("/r/Makefile")));
        assert!(is_source(Path::new("/r/build.zig")));
        assert!(is_source(Path::new("/r/src/thing.somelang")));
    }

    #[test]
    fn touching_only_docs_does_not_demand_a_test_run() {
        let root = std::env::temp_dir().join(format!("cq-touch-{}", std::process::id()));
        std::fs::remove_dir_all(&root).ok();
        std::fs::create_dir_all(&root).unwrap();
        let git_ok = |args: &[&str]| {
            let mut c = std::process::Command::new("git");
            c.args(args).current_dir(&root);
            crate::procutil::no_window(&mut c);
            c.output().map(|o| o.status.success()).unwrap_or(false)
        };
        if !git_ok(&["init", "-q"]) {
            return;
        }
        git_ok(&["config", "user.email", "t@t"]);
        git_ok(&["config", "user.name", "t"]);
        std::fs::write(root.join("a.rs"), "fn a() {}\n").unwrap();
        git_ok(&["add", "-A"]);
        assert!(git_ok(&["commit", "-q", "-m", "base"]));
        let base = super::super::evidence::git_head(&root);

        assert!(!touches_source(&root, base.as_deref()), "clean tree");

        std::fs::write(root.join("NOTES.md"), "just notes\n").unwrap();
        assert!(
            !touches_source(&root, base.as_deref()),
            "a markdown-only change must not cost a test run"
        );

        std::fs::write(root.join("a.rs"), "fn a() { todo!() }\n").unwrap();
        assert!(touches_source(&root, base.as_deref()), "source changed");

        std::fs::remove_dir_all(&root).ok();
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
