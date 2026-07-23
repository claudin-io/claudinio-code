//! Workspace containment: the one definition of "this path is inside that root".
//!
//! Two callers need it and must not drift apart:
//!
//! * `agent::tools::validate_path` — the guard on what the model can touch.
//! * `commands::fs` — the guard on what the webview can touch. The chat renders
//!   untrusted markdown, so a script injected there can call any registered
//!   Tauri command; without this the IPC surface is a way around the tool guard.

use std::path::{Component, Path};

/// True when `requested` resolves inside `root`.
///
/// Relative paths resolve against `root`, never the process CWD — under a Tauri
/// GUI the CWD is arbitrary (`/` on macOS when launched from Finder).
///
/// Canonicalization is the primary check: it follows symlinks, so a link inside
/// the workspace pointing at `/etc` is correctly rejected. It only works on
/// paths that exist, which a file about to be created does not, so the fallback
/// is a lexical check that refuses any `..` component — without resolution there
/// is no safe way to tell whether `a/../../b` escapes.
pub fn is_within_root(requested: &Path, root: &Path) -> bool {
    let effective = if requested.is_relative() {
        root.join(requested)
    } else {
        requested.to_path_buf()
    };

    if let (Ok(canon_req), Ok(canon_root)) = (effective.canonicalize(), root.canonicalize()) {
        return canon_req.starts_with(&canon_root);
    }

    // The target (or the root) does not exist yet — e.g. a file being created.
    // Canonicalize the deepest existing ancestor so symlinked parents are still
    // resolved, then apply the lexical rule to what is left.
    if effective.components().any(|c| c == Component::ParentDir) {
        return false;
    }
    match (nearest_existing_ancestor(&effective), root.canonicalize()) {
        (Some(anchor), Ok(canon_root)) => anchor.starts_with(&canon_root),
        _ => effective.starts_with(root),
    }
}

/// Canonicalize the closest ancestor of `path` that exists on disk. Returns
/// `None` when no ancestor resolves (a path on a missing drive, say).
fn nearest_existing_ancestor(path: &Path) -> Option<std::path::PathBuf> {
    let mut cur = path;
    loop {
        if let Ok(canon) = cur.canonicalize() {
            return Some(canon);
        }
        cur = cur.parent()?;
    }
}

/// `is_within_root` with the error message both call sites share.
pub fn ensure_within_root(requested: &str, root: &Path) -> Result<(), String> {
    if is_within_root(Path::new(requested), root) {
        return Ok(());
    }
    Err(format!(
        "path '{}' is outside the workspace '{}'. All file operations are restricted to the project workspace.",
        requested,
        root.display()
    ))
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::fs;

    fn root() -> tempfile::TempDir {
        tempfile::tempdir().unwrap()
    }

    #[test]
    fn accepts_a_file_inside_the_root() {
        let dir = root();
        let f = dir.path().join("a.txt");
        fs::write(&f, "x").unwrap();
        assert!(is_within_root(&f, dir.path()));
    }

    #[test]
    fn accepts_a_relative_path_resolved_against_the_root() {
        let dir = root();
        fs::write(dir.path().join("a.txt"), "x").unwrap();
        assert!(is_within_root(Path::new("a.txt"), dir.path()));
    }

    #[test]
    fn accepts_a_file_that_does_not_exist_yet() {
        let dir = root();
        assert!(is_within_root(&dir.path().join("new.txt"), dir.path()));
        assert!(is_within_root(&dir.path().join("sub/new.txt"), dir.path()));
    }

    #[test]
    fn rejects_a_sibling_directory() {
        let dir = root();
        let other = root();
        fs::write(other.path().join("a.txt"), "x").unwrap();
        assert!(!is_within_root(&other.path().join("a.txt"), dir.path()));
    }

    #[test]
    fn rejects_parent_traversal() {
        let dir = root();
        assert!(!is_within_root(Path::new("../escape.txt"), dir.path()));
        assert!(!is_within_root(Path::new("a/../../escape.txt"), dir.path()));
        assert!(!is_within_root(
            &dir.path().join("../escape.txt"),
            dir.path()
        ));
    }

    #[test]
    fn rejects_traversal_to_a_nonexistent_target() {
        // The canonicalize path cannot help here — the file does not exist —
        // so this is the case the lexical fallback has to catch.
        let dir = root();
        let escape = dir.path().join("..").join("nope.txt");
        assert!(!is_within_root(&escape, dir.path()));
    }

    #[cfg(unix)]
    #[test]
    fn rejects_a_symlink_pointing_out_of_the_root() {
        let dir = root();
        let outside = root();
        let target = outside.path().join("secret.txt");
        fs::write(&target, "x").unwrap();
        let link = dir.path().join("link.txt");
        std::os::unix::fs::symlink(&target, &link).unwrap();
        assert!(!is_within_root(&link, dir.path()));
    }

    #[cfg(unix)]
    #[test]
    fn rejects_a_new_file_under_a_symlinked_directory() {
        // The file does not exist, so only the ancestor-resolution branch can
        // notice that its parent escapes the workspace.
        let dir = root();
        let outside = root();
        let link = dir.path().join("out");
        std::os::unix::fs::symlink(outside.path(), &link).unwrap();
        assert!(!is_within_root(&link.join("planted.txt"), dir.path()));
    }

    #[test]
    fn ensure_within_root_names_the_offending_path() {
        let dir = root();
        let err = ensure_within_root("/etc/passwd", dir.path()).unwrap_err();
        assert!(err.contains("/etc/passwd"));
        assert!(err.contains("outside the workspace"));
    }
}
