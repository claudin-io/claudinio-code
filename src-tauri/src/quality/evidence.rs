//! Freshness: proving a quality report still describes the code on disk.
//!
//! Without this the gate is trivially defeated — run the tests, edit the code,
//! mark the goal done — and the whole harness would be back to trusting the
//! model. A report carries the digest of the worktree it ran against; the gate
//! recomputes the digest and refuses anything that no longer matches.
//!
//! The digest must be cheap (it runs on every `tasks_set` that closes a golden
//! task) and must change on any edit the agent can make.

use sha2::{Digest, Sha256};
use std::path::Path;
use std::process::Command;

use crate::procutil::no_window;

/// Beyond this many files the non-git walk stops hashing and marks itself
/// partial. Only reachable in a non-git workspace, where we are already in
/// best-effort territory.
const MAX_WALK_FILES: usize = 20_000;

/// A fingerprint of the workspace's current state. Equal digests mean no
/// tracked change, no staged change, and no touched untracked file since the
/// report was produced.
pub fn workspace_digest(root: &Path) -> String {
    match git_digest(root) {
        Some(d) => d,
        None => walk_digest(root),
    }
}

/// The agent's own bookkeeping directory. It must never enter the digest: the
/// session JSONL lives there and grows on every single turn, including the
/// turn that records the quality run itself, so counting it would make every
/// report stale the instant it was written. It is gitignored in this repo but
/// not necessarily in the user's, hence the explicit exclusion.
const BOOKKEEPING_DIR: &str = ".claudinio";

fn is_bookkeeping(rel: &str) -> bool {
    let rel = rel.trim_matches('"').replace('\\', "/");
    rel == BOOKKEEPING_DIR || rel.starts_with(&format!("{BOOKKEEPING_DIR}/"))
}

/// Path portion of a `git status --porcelain=v1` line (`XY path`).
fn porcelain_path(line: &str) -> &str {
    line.get(3..).unwrap_or("").trim()
}

/// Git path: HEAD, plus the full diff against it, plus the identity of every
/// untracked file.
///
/// `git diff HEAD` covers staged and unstaged edits to tracked files. Untracked
/// files are not in that diff, so their path/size/mtime is folded in as well —
/// otherwise the agent could create a new file, run the suite, rewrite the file
/// and still present the old green report as evidence.
fn git_digest(root: &Path) -> Option<String> {
    let head = git(root, &["rev-parse", "HEAD"])?;
    let raw_status = git(root, &["status", "--porcelain=v1"])?;
    let status: String = raw_status
        .lines()
        .filter(|l| !is_bookkeeping(porcelain_path(l)))
        .collect::<Vec<_>>()
        .join("\n");
    let diff = git(
        root,
        &[
            "diff",
            "HEAD",
            "--",
            ".",
            &format!(":(exclude){BOOKKEEPING_DIR}"),
        ],
    )?;

    let mut hasher = Sha256::new();
    hasher.update(b"git\0");
    hasher.update(head.as_bytes());
    hasher.update(b"\0");
    hasher.update(status.as_bytes());
    hasher.update(b"\0");
    hasher.update(diff.as_bytes());

    // Untracked entries are the "??" lines of the porcelain output.
    let mut untracked: Vec<&str> = status
        .lines()
        .filter_map(|l| l.strip_prefix("?? "))
        .collect();
    untracked.sort_unstable();
    for rel in untracked {
        let path = root.join(rel.trim_matches('"'));
        hasher.update(b"\0u\0");
        hasher.update(rel.as_bytes());
        stamp_path(&mut hasher, &path);
    }

    Some(format!("{:x}", hasher.finalize()))
}

/// Fold a path's size and mtime into the digest. Cheaper than reading contents
/// and sufficient: an edit that leaves both identical is not something an agent
/// writing code produces.
fn stamp_path(hasher: &mut Sha256, path: &Path) {
    let Ok(meta) = std::fs::metadata(path) else {
        hasher.update(b"missing");
        return;
    };
    if meta.is_dir() {
        // An untracked directory is reported as one porcelain line; hash the
        // files inside it so edits below it still register.
        let mut children: Vec<_> = walkdir(path).collect();
        children.sort();
        for child in children {
            hasher.update(child.to_string_lossy().as_bytes());
            if let Ok(m) = std::fs::metadata(&child) {
                hasher.update(m.len().to_le_bytes());
                hasher.update(mtime_nanos(&m).to_le_bytes());
            }
        }
        return;
    }
    hasher.update(meta.len().to_le_bytes());
    hasher.update(mtime_nanos(&meta).to_le_bytes());
}

fn mtime_nanos(meta: &std::fs::Metadata) -> u128 {
    meta.modified()
        .ok()
        .and_then(|t| t.duration_since(std::time::UNIX_EPOCH).ok())
        .map(|d| d.as_nanos())
        .unwrap_or(0)
}

/// Non-git fallback: size + mtime of every non-ignored file. Slower and only
/// used when `git` is unavailable or the folder is not a repository.
fn walk_digest(root: &Path) -> String {
    let mut entries: Vec<(String, u64, u128)> = Vec::new();
    for path in walkdir(root).take(MAX_WALK_FILES) {
        let rel = path
            .strip_prefix(root)
            .unwrap_or(&path)
            .to_string_lossy()
            .to_string();
        if is_bookkeeping(&rel) {
            continue;
        }
        let Ok(meta) = std::fs::metadata(&path) else {
            continue;
        };
        entries.push((rel, meta.len(), mtime_nanos(&meta)));
    }
    entries.sort();
    let mut hasher = Sha256::new();
    hasher.update(b"walk\0");
    for (rel, len, mtime) in entries {
        hasher.update(rel.as_bytes());
        hasher.update(len.to_le_bytes());
        hasher.update(mtime.to_le_bytes());
    }
    format!("{:x}", hasher.finalize())
}

/// Files under `root`, honouring .gitignore — the same `ignore` crate the
/// indexer walks with, so the digest and the index agree on what counts as
/// project source.
fn walkdir(root: &Path) -> impl Iterator<Item = std::path::PathBuf> {
    ignore::WalkBuilder::new(root)
        .hidden(false)
        .git_ignore(true)
        .build()
        .flatten()
        .filter(|e| e.file_type().is_some_and(|t| t.is_file()))
        .map(|e| e.into_path())
}

/// The commit the workspace is on, if any. Used to scope diff coverage.
pub fn git_head(root: &Path) -> Option<String> {
    git(root, &["rev-parse", "HEAD"])
}

pub(crate) fn git(root: &Path, args: &[&str]) -> Option<String> {
    let mut cmd = Command::new("git");
    cmd.args(args).current_dir(root);
    no_window(&mut cmd);
    let out = cmd.output().ok()?;
    if !out.status.success() {
        return None;
    }
    Some(String::from_utf8_lossy(&out.stdout).trim().to_string())
}

#[cfg(test)]
mod tests {
    use super::*;

    fn tmp(name: &str) -> std::path::PathBuf {
        let p = std::env::temp_dir().join(format!("cq-evidence-{name}-{}", std::process::id()));
        std::fs::remove_dir_all(&p).ok();
        std::fs::create_dir_all(&p).unwrap();
        p
    }

    fn init_repo(root: &std::path::Path) -> bool {
        let ok = |args: &[&str]| {
            let mut c = Command::new("git");
            c.args(args).current_dir(root);
            no_window(&mut c);
            c.output().map(|o| o.status.success()).unwrap_or(false)
        };
        if !ok(&["init", "-q"]) {
            return false;
        }
        ok(&["config", "user.email", "t@t"]);
        ok(&["config", "user.name", "t"]);
        std::fs::write(root.join("a.txt"), "one\n").unwrap();
        ok(&["add", "-A"]);
        ok(&["commit", "-q", "-m", "init"])
    }

    #[test]
    fn digest_is_stable_when_nothing_changes() {
        let root = tmp("stable");
        std::fs::write(root.join("a.txt"), "x").unwrap();
        assert_eq!(workspace_digest(&root), workspace_digest(&root));
        std::fs::remove_dir_all(&root).ok();
    }

    #[test]
    fn editing_a_tracked_file_changes_the_digest() {
        let root = tmp("tracked");
        if !init_repo(&root) {
            return; // git unavailable in this environment
        }
        let before = workspace_digest(&root);
        std::fs::write(root.join("a.txt"), "two\n").unwrap();
        assert_ne!(
            before,
            workspace_digest(&root),
            "an edit must invalidate prior evidence"
        );
        std::fs::remove_dir_all(&root).ok();
    }

    #[test]
    fn editing_an_untracked_file_changes_the_digest() {
        // The hole this closes: `git diff HEAD` says nothing about untracked
        // files, so without the extra stamping the agent could write a new
        // file, test it, rewrite it, and reuse the green report.
        let root = tmp("untracked");
        if !init_repo(&root) {
            return;
        }
        std::fs::write(root.join("new.rs"), "fn a() {}\n").unwrap();
        let before = workspace_digest(&root);
        // Size differs, so this is caught regardless of mtime granularity.
        std::fs::write(root.join("new.rs"), "fn a() { todo!() }\n").unwrap();
        assert_ne!(before, workspace_digest(&root));
        std::fs::remove_dir_all(&root).ok();
    }

    #[test]
    fn adding_a_file_changes_the_digest_without_git() {
        let root = tmp("nogit");
        std::fs::write(root.join("a.txt"), "x").unwrap();
        let before = workspace_digest(&root);
        std::fs::write(root.join("b.txt"), "y").unwrap();
        assert_ne!(before, workspace_digest(&root));
        std::fs::remove_dir_all(&root).ok();
    }

    #[test]
    fn the_agents_own_bookkeeping_never_moves_the_digest() {
        // The session JSONL lives in .claudinio/ and grows on every turn —
        // including the turn that records the quality run. Counting it would
        // make every report stale the instant it was written.
        let root = tmp("bookkeeping");
        std::fs::write(root.join("src.rs"), "fn a() {}\n").unwrap();
        std::fs::create_dir_all(root.join(".claudinio/sessions")).unwrap();
        std::fs::write(root.join(".claudinio/sessions/s.jsonl"), "{}\n").unwrap();
        let before = workspace_digest(&root);
        std::fs::write(
            root.join(".claudinio/sessions/s.jsonl"),
            "{}\n{\"more\":true}\n",
        )
        .unwrap();
        assert_eq!(before, workspace_digest(&root));
        std::fs::remove_dir_all(&root).ok();
    }

    #[test]
    fn bookkeeping_exclusion_matches_the_directory_not_a_prefix() {
        assert!(is_bookkeeping(".claudinio"));
        assert!(is_bookkeeping(".claudinio/sessions/a.jsonl"));
        // The workspace config file is real input to the gate, not bookkeeping.
        assert!(!is_bookkeeping(".claudinio.json"));
        assert!(!is_bookkeeping("src/.claudinio/x"));
    }

    #[test]
    fn porcelain_paths_are_extracted_past_the_status_columns() {
        assert_eq!(
            porcelain_path("?? .claudinio/sessions/a.jsonl"),
            ".claudinio/sessions/a.jsonl"
        );
        assert_eq!(porcelain_path(" M src/lib.rs"), "src/lib.rs");
        assert_eq!(porcelain_path("A  a.txt"), "a.txt");
    }

    #[test]
    fn digest_is_a_hex_sha256() {
        let root = tmp("shape");
        let d = workspace_digest(&root);
        assert_eq!(d.len(), 64, "{d}");
        assert!(d.chars().all(|c| c.is_ascii_hexdigit()));
        std::fs::remove_dir_all(&root).ok();
    }
}
