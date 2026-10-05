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
/// `None` means the scope could not be determined at all — no git, or no
/// commit to diff against. That is emphatically NOT the same as an empty map,
/// which means "git looked and nothing changed": treating the two alike would
/// let a non-git project pass coverage and mutation forever without either
/// layer ever running.
pub fn changed_lines(root: &Path, base_commit: Option<&str>) -> Option<ChangedLines> {
    let mut out: ChangedLines = HashMap::new();
    let base = base_commit
        .map(|s| s.to_string())
        .or_else(|| git(root, &["rev-parse", "HEAD"]))?;

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

    Some(out)
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

/// Write the diff since `base_commit` to a scratch file, for tools that scope
/// themselves from a patch rather than a line list (cargo-mutants `--in-diff`).
///
/// `git diff` already emits the `b/` prefix those tools expect. Returns `None`
/// when there is no diff to write, which the caller reads as "mutate
/// everything or nothing", never as an error.
pub fn write_patch(root: &Path, base_commit: Option<&str>) -> Option<PathBuf> {
    let base = base_commit
        .map(|s| s.to_string())
        .or_else(|| git(root, &["rev-parse", "HEAD"]))?;
    let patch = git(root, &["diff", &base, "--"])?;
    if patch.trim().is_empty() {
        return None;
    }
    let dir = crate::quality::runner::artifact_dir(root, "diff");
    let path = dir.join("changes.patch");
    // The trailing newline matters: a patch without one is rejected as
    // malformed by some parsers.
    std::fs::write(&path, format!("{patch}\n")).ok()?;
    Some(path)
}

/// Total number of changed lines across all files.
pub fn total_lines(changed: &ChangedLines) -> usize {
    changed.values().map(|s| s.len()).sum()
}

/// The commit this run's *own* changes are measured from.
///
/// `base_commit` is where HEAD stood when the run began. That is the right
/// base only while HEAD moves by commits the run itself makes. A fast-forward,
/// a pull or a branch switch moves HEAD across commits nobody in this session
/// wrote, and diffing against the old base then reads somebody else's release
/// as "code this session touched": a session asked to explain a version, which
/// fast-forwarded `main` to read it, was held at the finish line while the
/// whole suite ran over thirty files it had never edited.
///
/// So the base moves forward past commits that already existed. A commit on
/// HEAD's first-parent chain is pre-existing when it is reachable from
/// `base_commit`, or when it was committed before `since_secs` (when the
/// session's work began — nothing the session commits can be older than that).
/// The newest such commit is the base. Everything after it, plus the working
/// tree, is the session's.
///
/// Errs toward verifying: without `since_secs`, or whenever git cannot answer,
/// this is `base_commit` unchanged. A commit somebody else made *during* the
/// session and pulled in counts as the session's — an extra test run, never a
/// missed one.
pub fn session_base(
    root: &Path,
    base_commit: Option<&str>,
    since_secs: Option<u64>,
) -> Option<String> {
    let base = base_commit?;
    let Some(head) = git(root, &["rev-parse", "HEAD"]) else {
        return Some(base.to_string());
    };
    // Newest first, and only what `base` cannot reach.
    let Some(log) = git(
        root,
        &[
            "log",
            "--first-parent",
            "--format=%H %ct",
            &format!("{base}..HEAD"),
        ],
    ) else {
        return Some(base.to_string());
    };

    let mut oldest_own: Option<&str> = None;
    for line in log.lines() {
        let mut parts = line.split_whitespace();
        let (Some(sha), Some(committed)) = (
            parts.next(),
            parts.next().and_then(|t| t.parse::<u64>().ok()),
        ) else {
            return Some(base.to_string());
        };
        if since_secs.is_some_and(|since| committed < since) {
            return Some(sha.to_string());
        }
        oldest_own = Some(sha);
    }
    match oldest_own {
        // HEAD is `base` itself or behind it (a reset, a checkout of an older
        // commit): the session committed nothing, only the worktree is its.
        None => Some(head),
        // Every commit in range is the session's; its work starts at the
        // parent of the oldest one — `base` itself on a linear history.
        Some(oldest) => git(root, &["rev-parse", &format!("{oldest}^")]).or(Some(base.to_string())),
    }
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

    /// A repository whose history is under the test's control: every commit
    /// gets an explicit committer date, since "did this exist before the
    /// session began" is exactly what `session_base` reads.
    struct Repo {
        root: PathBuf,
    }

    impl Repo {
        fn new(name: &str) -> Option<Repo> {
            let root = std::env::temp_dir().join(format!("cq-sbase-{name}-{}", std::process::id()));
            std::fs::remove_dir_all(&root).ok();
            std::fs::create_dir_all(&root).unwrap();
            let repo = Repo { root };
            if !repo.git(&["init", "-q"], None) {
                return None; // no git on this machine: nothing to test
            }
            // Whatever `init.defaultBranch` says, the tests call it `main`.
            repo.git(&["symbolic-ref", "HEAD", "refs/heads/main"], None);
            repo.git(&["config", "user.email", "t@t"], None);
            repo.git(&["config", "user.name", "t"], None);
            Some(repo)
        }

        fn git(&self, args: &[&str], committed_at: Option<u64>) -> bool {
            let mut c = std::process::Command::new("git");
            c.args(args).current_dir(&self.root);
            if let Some(at) = committed_at {
                c.env("GIT_COMMITTER_DATE", format!("@{at} +0000"));
                c.env("GIT_AUTHOR_DATE", format!("@{at} +0000"));
            }
            crate::procutil::no_window(&mut c);
            c.output().map(|o| o.status.success()).unwrap_or(false)
        }

        /// Write `file`, commit it at `at`, return the new HEAD.
        fn commit(&self, file: &str, at: u64) -> String {
            std::fs::write(self.root.join(file), format!("// {file} @ {at}\n")).unwrap();
            assert!(self.git(&["add", "-A"], None));
            assert!(self.git(&["commit", "-q", "-m", file], Some(at)));
            super::super::evidence::git_head(&self.root).expect("HEAD")
        }
    }

    impl Drop for Repo {
        fn drop(&mut self) {
            std::fs::remove_dir_all(&self.root).ok();
        }
    }

    /// The session in these tests begins at t=1000.
    const SESSION_START: u64 = 1_000;

    #[test]
    fn a_fast_forward_over_existing_commits_is_not_the_sessions_work() {
        // The run that prompted this: `git merge --ff-only origin/main` moved
        // HEAD across a release the session had only been asked to explain.
        let Some(repo) = Repo::new("ff") else { return };
        let start = repo.commit("a.rs", 100);
        assert!(repo.git(&["checkout", "-q", "-b", "upstream"], None));
        repo.commit("b.rs", 200);
        let release = repo.commit("c.rs", 300);
        assert!(repo.git(&["checkout", "-q", "main"], None));
        assert!(repo.git(&["merge", "-q", "--ff-only", "upstream"], None));

        assert!(
            touches_source(&repo.root, Some(&start)),
            "against the run's starting commit, the release looks like the session's edits"
        );
        let base = session_base(&repo.root, Some(&start), Some(SESSION_START));
        assert_eq!(base.as_deref(), Some(release.as_str()));
        assert!(!touches_source(&repo.root, base.as_deref()));
        assert_eq!(
            changed_lines(&repo.root, base.as_deref()).map(|c| c.len()),
            Some(0)
        );
    }

    #[test]
    fn work_on_top_of_a_fast_forward_is_still_the_sessions() {
        let Some(repo) = Repo::new("ff-then-work") else {
            return;
        };
        let start = repo.commit("a.rs", 100);
        assert!(repo.git(&["checkout", "-q", "-b", "upstream"], None));
        let release = repo.commit("b.rs", 300);
        assert!(repo.git(&["checkout", "-q", "main"], None));
        assert!(repo.git(&["merge", "-q", "--ff-only", "upstream"], None));
        repo.commit("mine.rs", SESSION_START + 60);
        std::fs::write(repo.root.join("wip.rs"), "fn wip() {}\n").unwrap();

        let base = session_base(&repo.root, Some(&start), Some(SESSION_START));
        assert_eq!(base.as_deref(), Some(release.as_str()));
        let mut changed: Vec<String> = changed_files(&repo.root, base.as_deref())
            .iter()
            .map(|p| p.file_name().unwrap().to_string_lossy().to_string())
            .collect();
        changed.sort();
        assert_eq!(
            changed,
            ["mine.rs", "wip.rs"],
            "the release's b.rs is not ours"
        );
    }

    #[test]
    fn the_sessions_own_commits_keep_the_starting_base() {
        let Some(repo) = Repo::new("own") else { return };
        let start = repo.commit("a.rs", 100);
        repo.commit("one.rs", SESSION_START + 10);
        repo.commit("two.rs", SESSION_START + 20);

        let base = session_base(&repo.root, Some(&start), Some(SESSION_START));
        assert_eq!(base.as_deref(), Some(start.as_str()));
        assert!(touches_source(&repo.root, base.as_deref()));
    }

    #[test]
    fn without_a_session_start_nothing_is_assumed_to_be_foreign() {
        // No timestamp to judge by: keep the starting commit and verify.
        let Some(repo) = Repo::new("nosince") else {
            return;
        };
        let start = repo.commit("a.rs", 100);
        repo.commit("b.rs", 200);

        assert_eq!(
            session_base(&repo.root, Some(&start), None).as_deref(),
            Some(start.as_str())
        );
        assert_eq!(session_base(&repo.root, None, Some(SESSION_START)), None);
    }

    #[test]
    fn a_head_moved_behind_the_base_leaves_only_the_worktree() {
        // `git reset --hard HEAD~1`, or checking out an older commit: diffing
        // against the old base would report the dropped commit, reversed.
        let Some(repo) = Repo::new("behind") else {
            return;
        };
        let older = repo.commit("a.rs", 100);
        let start = repo.commit("b.rs", 200);
        assert!(repo.git(&["reset", "-q", "--hard", &older], None));

        let base = session_base(&repo.root, Some(&start), Some(SESSION_START));
        assert_eq!(base.as_deref(), Some(older.as_str()));
        assert!(!touches_source(&repo.root, base.as_deref()));
    }

    #[test]
    fn no_git_repository_yields_no_changed_lines() {
        let root = std::env::temp_dir().join(format!("cq-diff-nogit-{}", std::process::id()));
        std::fs::create_dir_all(&root).unwrap();
        // Deliberately passing a bogus base: the point is that the harness
        // degrades to "nothing to require" instead of erroring.
        // Some() with an empty map would claim "git looked and nothing
        // changed", which is a different and much more dangerous statement.
        let changed = changed_lines(&root, None);
        assert!(
            changed.is_none(),
            "no git means unknown scope, not empty scope"
        );
        std::fs::remove_dir_all(&root).ok();
    }
}
