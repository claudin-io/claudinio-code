//! Project memory: `<workspace>/.claudinio/MEMORY.md` (lean-harness Phase 9,
//! decision D6).
//!
//! A session starts knowing nothing its predecessors in the same repository
//! learned, and spends rounds re-discovering it. The memory is the smallest
//! thing that fixes that: a `## Learned` list of one-line entries, each a
//! lesson and the plan file that holds its detail — pointers, not prose (the
//! "AGENTS.md as a table of contents" shape of OpenAI's harness-engineering
//! post).
//!
//! **The harness writes it, never the model.** `finalize_plan` takes an
//! optional `memory` list (at most `MAX_LINES` lines of at most
//! `MAX_LINE_CHARS` characters) and this module appends those lines; with no
//! list nothing is appended — no journal is dumped here automatically. The
//! file is append-only with a cap of `MEMORY_CAP` characters: past it the
//! oldest entries are dropped, whole, and stay in the plans they point to.
//! Nothing is ever summarized or rewritten — the ACE paper's (arXiv
//! 2510.04618) two failure modes of an evolving context are brevity bias (a
//! summary keeps the generic and loses the specific) and context collapse (a
//! rewrite loses what the rewriter did not think mattered).

use std::path::{Path, PathBuf};

/// The memory file, relative to the workspace root.
pub const MEMORY_FILE: &str = ".claudinio/MEMORY.md";
/// The heading of the list the harness appends to.
pub const LEARNED_HEADING: &str = "## Learned";
/// At most this many lines per `finalize_plan` call.
pub const MAX_LINES: usize = 5;
/// At most this many characters per line.
pub const MAX_LINE_CHARS: usize = 200;
/// The file is kept at or under this many characters (about 1k tokens, the
/// plan's budget for it) by dropping its oldest entries.
pub const MEMORY_CAP: usize = 4_000;

/// What a new file starts with, above its list.
const PREAMBLE: &str = "# Project memory\n\n\
Written by Claudinio Code from the `memory` lines of `finalize_plan`. Append-only: \
when the file passes about 4,000 characters the oldest entries are dropped, and \
each stays in the plan it points to.\n\n";

/// The lines a `finalize_plan` call proposed, made one-line and checked:
/// whitespace runs (a newline included) become one space, blank lines are
/// dropped, and more than `MAX_LINES` lines or a line over `MAX_LINE_CHARS`
/// characters is refused with what to change. The schema says the same; the
/// check is here because a schema is a request, not a gate.
pub fn proposed_lines(lines: &[String]) -> Result<Vec<String>, String> {
    let lines: Vec<String> = lines
        .iter()
        .map(|l| l.split_whitespace().collect::<Vec<_>>().join(" "))
        .filter(|l| !l.is_empty())
        .collect();
    if lines.len() > MAX_LINES {
        return Err(format!(
            "memory takes at most {MAX_LINES} lines per call, and this one has {}: keep the \
             ones a later session would most need and call finalize_plan again.",
            lines.len()
        ));
    }
    if let Some((i, n)) = lines
        .iter()
        .map(|l| l.chars().count())
        .enumerate()
        .find(|(_, n)| *n > MAX_LINE_CHARS)
    {
        return Err(format!(
            "memory line {} has {n} characters, over the {MAX_LINE_CHARS} allowed: shorten it \
             (a lesson and where it is written down) and call finalize_plan again.",
            i + 1
        ));
    }
    Ok(lines)
}

/// One entry: `- YYYY-MM-DD — <line> (plan: <plan path>)`.
pub fn entry(date: &str, line: &str, plan: &str) -> String {
    format!("- {date} \u{2014} {line} (plan: {plan})")
}

/// `existing` with `entries` appended to the end of its `## Learned` list
/// (the list and, for an empty file, the preamble are created when missing),
/// then the oldest entries dropped until the text is at most `cap`
/// characters. Only entries that were already there are dropped: what this
/// call adds stays even when the rest of the file alone is over the cap.
/// Text outside the list — a person's own notes — is never touched.
pub fn appended(existing: &str, entries: &[String], cap: usize) -> String {
    let mut lines: Vec<String> = existing.lines().map(str::to_string).collect();
    let Some(heading) = lines.iter().position(|l| l.trim_end() == LEARNED_HEADING) else {
        let mut text = if existing.trim().is_empty() {
            PREAMBLE.to_string()
        } else {
            format!("{}\n\n", existing.trim_end())
        };
        text.push_str(LEARNED_HEADING);
        text.push('\n');
        for e in entries {
            text.push_str(e);
            text.push('\n');
        }
        return text;
    };
    // The list runs to the next heading; new entries go after its last entry.
    let end = (heading + 1..lines.len())
        .find(|&i| lines[i].starts_with('#'))
        .unwrap_or(lines.len());
    let mut at = (heading + 1..end)
        .rev()
        .find(|&i| lines[i].starts_with("- "))
        .map_or(heading + 1, |i| i + 1);
    lines.splice(at..at, entries.iter().cloned());

    // Every line ends with a newline in the result.
    let mut total: usize = lines.iter().map(|l| l.chars().count() + 1).sum();
    let mut i = heading + 1;
    while total > cap && i < at {
        if lines[i].starts_with("- ") {
            total -= lines[i].chars().count() + 1;
            lines.remove(i);
            at -= 1;
        } else {
            i += 1;
        }
    }
    let mut text = lines.join("\n");
    text.push('\n');
    text
}

/// The plan's path as an entry names it: relative to the workspace when it
/// is inside it, with `/` separators, so the entry reads the same on every
/// platform; otherwise the path as it is.
pub fn plan_ref(root: &Path, plan: &Path) -> String {
    let canon = |p: &Path| p.canonicalize().unwrap_or_else(|_| p.to_path_buf());
    let relative = plan
        .strip_prefix(root)
        .ok()
        .map(Path::to_path_buf)
        .or_else(|| {
            canon(plan)
                .strip_prefix(canon(root))
                .ok()
                .map(Path::to_path_buf)
        });
    match relative {
        Some(rel) => rel
            .components()
            .map(|c| c.as_os_str().to_string_lossy())
            .collect::<Vec<_>>()
            .join("/"),
        None => plan.to_string_lossy().into_owned(),
    }
}

/// Append `lines` (already checked by `proposed_lines`) as entries dated
/// `date` pointing at `plan`, and return the file's path. Never through a
/// link: a cloned repository can commit `.claudinio` or `MEMORY.md` as a link,
/// and following it would write wherever it points. The new text replaces
/// the file through a rename, so a reader never sees half of it.
pub fn append(root: &Path, plan: &Path, date: &str, lines: &[String]) -> Result<PathBuf, String> {
    let fail = |e: &dyn std::fmt::Display| format!("could not write {MEMORY_FILE}: {e}");
    let dir = root.join(".claudinio");
    match std::fs::symlink_metadata(&dir) {
        Ok(meta) if meta.is_dir() => {}
        Ok(_) => return Err(fail(&".claudinio is a link or a file")),
        Err(e) if e.kind() == std::io::ErrorKind::NotFound => {
            std::fs::create_dir(&dir).map_err(|e| fail(&e))?
        }
        Err(e) => return Err(fail(&e)),
    }
    let path = root.join(MEMORY_FILE);
    let existing = match std::fs::symlink_metadata(&path) {
        Ok(meta) if meta.file_type().is_file() => {
            std::fs::read_to_string(&path).map_err(|e| fail(&e))?
        }
        Ok(_) => return Err(fail(&"it is a link or not a plain file")),
        Err(e) if e.kind() == std::io::ErrorKind::NotFound => String::new(),
        Err(e) => return Err(fail(&e)),
    };
    let plan = plan_ref(root, plan);
    let entries: Vec<String> = lines.iter().map(|l| entry(date, l, &plan)).collect();
    let text = appended(&existing, &entries, MEMORY_CAP);
    let tmp = dir.join(format!(".MEMORY.md.{}.tmp", std::process::id()));
    // A leftover (or planted) file under the temporary name goes first;
    // removing a link removes the link, never what it points at.
    let _ = std::fs::remove_file(&tmp);
    let written = std::fs::OpenOptions::new()
        .write(true)
        .create_new(true)
        .open(&tmp)
        .and_then(|mut f| std::io::Write::write_all(&mut f, text.as_bytes()))
        .and_then(|()| std::fs::rename(&tmp, &path));
    if let Err(e) = written {
        let _ = std::fs::remove_file(&tmp);
        return Err(fail(&e));
    }
    Ok(path)
}

#[cfg(test)]
mod tests {
    use super::*;

    fn lines(v: &[&str]) -> Vec<String> {
        v.iter().map(|s| s.to_string()).collect()
    }

    #[test]
    fn a_proposed_line_is_made_one_line_and_blank_ones_are_dropped() {
        let got = proposed_lines(&lines(&[
            "  The index lives in\n  .claudinio/index.db  ",
            "   ",
            "Tests need RUST_LOG unset",
        ]))
        .unwrap();
        assert_eq!(
            got,
            [
                "The index lives in .claudinio/index.db",
                "Tests need RUST_LOG unset"
            ]
        );
    }

    #[test]
    fn more_than_five_lines_are_refused_with_what_to_change() {
        let err = proposed_lines(&lines(&["a", "b", "c", "d", "e", "f"])).unwrap_err();
        assert!(err.contains("at most 5"), "{err}");
        assert!(err.contains("6"), "{err}");
    }

    // The limit is in characters, as the schema says, not bytes: 200
    // accented letters are 400 bytes and still one allowed line.
    #[test]
    fn a_line_over_two_hundred_characters_is_refused_and_named() {
        assert!(proposed_lines(&["é".repeat(200)]).is_ok());
        let err = proposed_lines(&lines(&["short", &"é".repeat(201)])).unwrap_err();
        assert!(err.contains("line 2"), "{err}");
        assert!(err.contains("201"), "{err}");
    }

    #[test]
    fn an_entry_is_dated_and_names_its_plan() {
        assert_eq!(
            entry(
                "2026-10-19",
                "Builds need -j 2 here",
                ".claudinio/plans/p.md"
            ),
            "- 2026-10-19 — Builds need -j 2 here (plan: .claudinio/plans/p.md)"
        );
    }

    #[test]
    fn the_first_entries_create_the_preamble_and_the_list() {
        let text = appended("", &lines(&["- one", "- two"]), MEMORY_CAP);
        assert!(text.starts_with("# Project memory\n"), "{text}");
        assert!(text.ends_with("## Learned\n- one\n- two\n"), "{text}");
    }

    #[test]
    fn new_entries_go_to_the_end_of_the_list_and_text_after_it_stays() {
        let existing = "# Project memory\n\n## Learned\n- old\n\n## Mine\nmy own notes\n";
        let text = appended(existing, &lines(&["- new"]), MEMORY_CAP);
        assert_eq!(
            text,
            "# Project memory\n\n## Learned\n- old\n- new\n\n## Mine\nmy own notes\n"
        );
    }

    #[test]
    fn a_file_without_the_list_gains_one_at_its_end() {
        let text = appended("# Notes\nsomething\n", &lines(&["- new"]), MEMORY_CAP);
        assert_eq!(text, "# Notes\nsomething\n\n## Learned\n- new\n");
    }

    // Over the cap the oldest entries go, whole and in order, and nothing
    // else changes: no entry is shortened, merged or reworded.
    #[test]
    fn over_the_cap_the_oldest_entries_are_dropped_whole() {
        let old: Vec<String> = (0..10)
            .map(|i| format!("- old {i} {}", "x".repeat(40)))
            .collect();
        let existing = appended("", &old, usize::MAX);
        let cap = existing.chars().count() + 10;
        let text = appended(
            &existing,
            &lines(&["- brand new entry, 32 chars long"]),
            cap,
        );
        assert!(
            text.chars().count() <= cap,
            "{} > {cap}",
            text.chars().count()
        );
        assert!(!text.contains("- old 0 "), "{text}");
        assert!(text.contains("- old 9 "), "{text}");
        assert!(
            text.ends_with("- brand new entry, 32 chars long\n"),
            "{text}"
        );
        for kept in text.lines().filter(|l| l.starts_with("- old")) {
            assert!(old.iter().any(|o| o == kept), "rewritten: {kept}");
        }
    }

    // The cap counts characters; entries of accented, CJK and emoji text are
    // dropped by whole lines, so no cut ever lands inside a character.
    #[test]
    fn the_cap_counts_characters_of_multi_byte_text_and_drops_whole_lines() {
        let old: Vec<String> = (0..30)
            .map(|i| format!("- {i} café — 日本語 🚀 {}", "é".repeat(60)))
            .collect();
        let existing = appended("", &old[..29], usize::MAX);
        let text = appended(&existing, &old[29..], 1_000);
        assert!(text.chars().count() <= 1_000);
        assert!(text.len() > 1_000, "the text is multi-byte");
        assert!(text.ends_with(&format!("{}\n", old[29])), "{text}");
        for kept in text.lines().filter(|l| l.starts_with("- ")) {
            assert!(old.iter().any(|o| o == kept), "cut: {kept}");
        }
    }

    #[test]
    fn the_entries_just_added_stay_even_when_the_rest_is_over_the_cap() {
        let mine = format!("# Mine\n{}\n", "n".repeat(500));
        let text = appended(&mine, &lines(&["- new"]), 100);
        assert!(text.ends_with("## Learned\n- new\n"), "{text}");
        assert!(text.starts_with(&mine), "a person's text is never dropped");
    }

    #[test]
    fn a_plan_inside_the_workspace_is_named_by_its_relative_path() {
        let root = Path::new("/w");
        assert_eq!(
            plan_ref(root, Path::new("/w/.claudinio/plans/2026-10-19_x.md")),
            ".claudinio/plans/2026-10-19_x.md"
        );
        assert_eq!(
            plan_ref(root, Path::new("/elsewhere/p.md")),
            "/elsewhere/p.md"
        );
    }

    #[test]
    fn append_writes_the_entries_under_the_workspace() {
        let dir = tempfile::tempdir().unwrap();
        let plan = dir.path().join(".claudinio/plans/2026-10-19_x.md");
        let path = append(dir.path(), &plan, "2026-10-19", &lines(&["one", "two"])).unwrap();
        assert_eq!(path, dir.path().join(MEMORY_FILE));
        append(dir.path(), &plan, "2026-10-20", &lines(&["three"])).unwrap();
        let text = std::fs::read_to_string(&path).unwrap();
        assert!(
            text.ends_with(
                "## Learned\n\
                 - 2026-10-19 — one (plan: .claudinio/plans/2026-10-19_x.md)\n\
                 - 2026-10-19 — two (plan: .claudinio/plans/2026-10-19_x.md)\n\
                 - 2026-10-20 — three (plan: .claudinio/plans/2026-10-19_x.md)\n"
            ),
            "{text}"
        );
    }

    #[cfg(unix)]
    #[test]
    fn append_never_writes_through_a_planted_link() {
        let dir = tempfile::tempdir().unwrap();
        let outside = tempfile::tempdir().unwrap();
        let target = outside.path().join("victim.md");
        std::fs::write(&target, "untouched\n").unwrap();
        std::fs::create_dir(dir.path().join(".claudinio")).unwrap();
        std::os::unix::fs::symlink(&target, dir.path().join(MEMORY_FILE)).unwrap();
        let plan = dir.path().join("p.md");
        assert!(append(dir.path(), &plan, "2026-10-19", &lines(&["x"])).is_err());
        assert_eq!(std::fs::read_to_string(&target).unwrap(), "untouched\n");

        // A link at `.claudinio` itself would put the whole folder outside.
        let dir2 = tempfile::tempdir().unwrap();
        std::os::unix::fs::symlink(outside.path(), dir2.path().join(".claudinio")).unwrap();
        assert!(append(dir2.path(), &plan, "2026-10-19", &lines(&["x"])).is_err());
        assert!(!outside.path().join("MEMORY.md").exists());
    }
}
