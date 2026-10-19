//! The compaction note on disk: `<workspace>/.claudinio/notes/<session>.md`.
//!
//! A summarizing compaction replaces the conversation with a note the
//! summarizer wrote (`subagent::run_summary_agent`). Until lean-harness Phase 6
//! that note lived only in the session's `compacted` record, which the model
//! sees once and the user never opens. Appending it to a Markdown file per
//! session makes it the file system's memory (Manus: "use the file system as
//! context"): the successor can read an earlier compaction's note after a
//! second one replaced it, the user can read what the agent carried forward,
//! and a `PreCompact` hook's output lands there with it.

use std::io::Write;
use std::path::{Path, PathBuf};

/// The folder under the workspace the notes live in.
pub const NOTES_DIR: &str = ".claudinio/notes";

/// `<workspace>/.claudinio/notes/<session_id>.md`, or `None` for an id that is
/// not a plain file name (the id names a file; `..` or a separator must not
/// make it name another).
pub fn note_path(workspace: &Path, session_id: &str) -> Option<PathBuf> {
    let plain = !session_id.is_empty()
        && session_id
            .chars()
            .all(|c| c.is_ascii_alphanumeric() || c == '-' || c == '_');
    plain.then(|| workspace.join(NOTES_DIR).join(format!("{session_id}.md")))
}

/// Append `note` under a timestamp header and return the compacted text: the
/// note followed by a line naming the file, so the successor knows where the
/// history of its notes is. When there is no workspace or the write fails the
/// note is returned as it is — a compaction never fails over its copy on disk.
pub fn record(workspace: Option<&str>, session_id: &str, note: &str, ts_ms: u64) -> String {
    let Some(ws) = workspace.map(Path::new) else {
        return note.to_string();
    };
    let Some(path) = note_path(ws, session_id) else {
        return note.to_string();
    };
    let when = i64::try_from(ts_ms)
        .ok()
        .and_then(chrono::DateTime::from_timestamp_millis)
        .map(|t| t.format("%Y-%m-%d %H:%M:%S UTC").to_string())
        .unwrap_or_default();
    if append(
        ws,
        &path,
        &format!("## Compaction — {when}\n\n{}\n\n", note.trim_end()),
    )
    .is_err()
    {
        return note.to_string();
    }
    format!(
        "{}\n\n(This note is also appended to `{NOTES_DIR}/{session_id}.md`, one entry per \
         compaction, oldest first: read it for what an earlier compaction carried.)",
        note.trim_end()
    )
}

/// Append to `path`, making the notes folder, and never through a link: a
/// link planted at the note's path would make every compaction append to
/// whatever it points at, and one at `.claudinio` or `notes` (a cloned
/// repository can commit either) would put the folder itself outside the
/// workspace. Each folder under `workspace` is checked before it is used, so
/// nothing is created through a link either.
fn append(workspace: &Path, path: &Path, text: &str) -> std::io::Result<()> {
    let mut dir = workspace.to_path_buf();
    for part in Path::new(NOTES_DIR).components() {
        dir.push(part);
        match std::fs::symlink_metadata(&dir) {
            Ok(meta) if meta.is_dir() => {}
            Ok(_) => return Err(std::io::Error::other("a notes folder is a link or a file")),
            Err(e) if e.kind() == std::io::ErrorKind::NotFound => std::fs::create_dir(&dir)?,
            Err(e) => return Err(e),
        }
    }
    if let Ok(meta) = std::fs::symlink_metadata(path)
        && !meta.file_type().is_file()
    {
        return Err(std::io::Error::other("the note path is not a plain file"));
    }
    let mut file = std::fs::OpenOptions::new()
        .create(true)
        .append(true)
        .open(path)?;
    file.write_all(text.as_bytes())
}

#[cfg(test)]
mod tests {
    use super::*;

    const ID: &str = "0a1b2c3d-4e5f-6789-abcd-ef0123456789";
    // 2026-10-19 12:00:00 UTC
    const TS: u64 = 1_792_411_200_000;

    #[test]
    fn the_note_goes_under_the_workspace_by_session_id() {
        let ws = Path::new("/ws");
        assert_eq!(
            note_path(ws, ID),
            Some(PathBuf::from(format!("/ws/.claudinio/notes/{ID}.md")))
        );
        for bad in ["", "..", "../x", "a/b", "a\\b", "a.md"] {
            assert_eq!(note_path(ws, bad), None, "{bad:?}");
        }
    }

    #[test]
    fn each_compaction_appends_its_note_under_a_timestamp_header() {
        let dir = tempfile::tempdir().unwrap();
        let ws = dir.path().to_str().unwrap();
        let first = record(Some(ws), ID, "## Goal\nFix the login test.", TS);
        let second = record(Some(ws), ID, "## Goal\nShip it.", TS + 60_000);
        let file = dir.path().join(".claudinio/notes").join(format!("{ID}.md"));
        let text = std::fs::read_to_string(&file).expect("the note file");
        assert!(
            text.contains(
                "## Compaction — 2026-10-19 12:00:00 UTC\n\n## Goal\nFix the login test."
            )
        );
        assert!(text.contains("## Compaction — 2026-10-19 12:01:00 UTC\n\n## Goal\nShip it."));
        assert!(text.find("Fix the login").unwrap() < text.find("Ship it").unwrap());
        // The compacted context names the file, relative to the workspace.
        let rel = format!(".claudinio/notes/{ID}.md");
        assert!(first.starts_with("## Goal\nFix the login test."));
        assert!(first.contains(&rel), "{first}");
        assert!(second.contains(&rel), "{second}");
    }

    #[test]
    fn without_a_workspace_or_a_writable_folder_the_note_is_returned_as_it_is() {
        assert_eq!(record(None, ID, "note", TS), "note");
        let dir = tempfile::tempdir().unwrap();
        // `.claudinio` is a file: the notes folder cannot be made.
        std::fs::write(dir.path().join(".claudinio"), "x").unwrap();
        let ws = dir.path().to_str().unwrap();
        assert_eq!(record(Some(ws), ID, "note", TS), "note");
        assert_eq!(record(Some(ws), "../escape", "note", TS), "note");
    }

    // A link planted at the note's path must not make the harness append to
    // whatever it points at.
    #[cfg(unix)]
    #[test]
    fn a_link_at_the_note_path_is_not_followed() {
        let dir = tempfile::tempdir().unwrap();
        let target = dir.path().join("Cargo.toml");
        std::fs::write(&target, "[package]\n").unwrap();
        let notes = dir.path().join(".claudinio/notes");
        std::fs::create_dir_all(&notes).unwrap();
        std::os::unix::fs::symlink(&target, notes.join(format!("{ID}.md"))).unwrap();
        let ws = dir.path().to_str().unwrap();
        assert_eq!(record(Some(ws), ID, "note", TS), "note");
        assert_eq!(std::fs::read_to_string(&target).unwrap(), "[package]\n");
    }

    // A cloned repository can carry `.claudinio` itself as a committed link:
    // following it would make the notes folder, and every note, land outside
    // the workspace.
    #[cfg(unix)]
    #[test]
    fn a_link_at_the_claudinio_folder_is_not_followed() {
        let dir = tempfile::tempdir().unwrap();
        let outside = tempfile::tempdir().unwrap();
        let ws_dir = dir.path().join("ws");
        std::fs::create_dir_all(&ws_dir).unwrap();
        std::os::unix::fs::symlink(outside.path(), ws_dir.join(".claudinio")).unwrap();
        let ws = ws_dir.to_str().unwrap();
        assert_eq!(record(Some(ws), ID, "note", TS), "note");
        assert!(
            !outside.path().join("notes").exists(),
            "nothing is written through the link"
        );
    }
}
