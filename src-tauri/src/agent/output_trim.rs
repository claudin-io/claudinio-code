//! Fit a long command output into the context by keeping the parts that
//! matter, instead of the first N characters.
//!
//! The history copy of a tool result is capped (`MAX_TOOL_RESULT_CHARS`), and
//! a plain head cut is exactly wrong for command output: `cargo test`, `npm
//! run build` and friends print thousands of lines of progress and put the
//! failure and the summary at the **end**. A head cut kept the noise and
//! dropped the answer.
//!
//! The output is split into blocks of lines. The first and last blocks always
//! stay (the command's opening and its verdict); the rest are ranked — by Jev
//! when it is available (one `noul` per block: "does this block carry errors,
//! failures, warnings or a result?"), by closeness to the end otherwise — and
//! kept greedily up to the budget, in their original order, with explicit
//! "lines a–b omitted" markers. The full output is written to a temp file and
//! its path given to the model, so nothing is lost: it can `read_file` or
//! `grep` the rest.
//!
//! Only the tail of the conversation changes (the result that just arrived),
//! so the cached prefix is untouched.

use serde_json::{Value, json};

/// What the trimmed output may occupy. Below the history cap so the cap's
/// head cut never fires on top of this.
pub const BUDGET_CHARS: usize = 20_000;
/// Target size of one block.
pub const BLOCK_CHARS: usize = 2_000;
/// Blocks Jev is asked about, at most; larger outputs rank the rest by position.
pub const MAX_JUDGED_BLOCKS: usize = 60;
/// P(keep) from which a block counts as signal. Measured live (2026-10-05):
/// failure lines, panics, summaries, compiler errors and warnings 0.87–0.99;
/// passing tests, compile progress and package-fetch noise 0.04–0.12.
pub const KEEP_THRESHOLD: f64 = 0.5;
const CONCURRENCY: usize = 8;

/// A run of whole lines from the output, 1-based inclusive line numbers.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Block {
    pub first_line: usize,
    pub last_line: usize,
    pub text: String,
}

/// Split into blocks of whole lines of about [`BLOCK_CHARS`]. A single line
/// longer than that becomes its own block, cut to the block size.
pub fn split_blocks(text: &str) -> Vec<Block> {
    let mut blocks = Vec::new();
    let mut cur: Vec<&str> = Vec::new();
    let mut cur_len = 0usize;
    let mut first = 1usize;
    let flush = |blocks: &mut Vec<Block>, cur: &mut Vec<&str>, first: usize| {
        if !cur.is_empty() {
            blocks.push(Block {
                first_line: first,
                last_line: first + cur.len() - 1,
                text: cur.join("\n"),
            });
            cur.clear();
        }
    };
    for (i, line) in text.lines().enumerate() {
        let n = i + 1;
        if line.len() > BLOCK_CHARS {
            flush(&mut blocks, &mut cur, first);
            let mut end = BLOCK_CHARS;
            while !line.is_char_boundary(end) {
                end -= 1;
            }
            blocks.push(Block {
                first_line: n,
                last_line: n,
                text: format!("{}… (line cut, {} chars)", &line[..end], line.len()),
            });
            first = n + 1;
            cur_len = 0;
            continue;
        }
        if !cur.is_empty() && cur_len + line.len() + 1 > BLOCK_CHARS {
            flush(&mut blocks, &mut cur, first);
            first = n;
            cur_len = 0;
        }
        cur.push(line);
        cur_len += line.len() + 1;
    }
    flush(&mut blocks, &mut cur, first);
    blocks
}

/// Indices of the blocks to keep, ascending. `scores[i]` is P(signal) for
/// block `i` when a judge ranked them; `None` ranks by closeness to the end.
pub fn select(blocks: &[Block], scores: Option<&[f64]>, budget: usize) -> Vec<usize> {
    let n = blocks.len();
    if n == 0 {
        return Vec::new();
    }
    let mut kept: Vec<usize> = Vec::new();
    let mut used = 0usize;
    let take = |i: usize, kept: &mut Vec<usize>, used: &mut usize| {
        let len = blocks[i].text.len();
        if !kept.contains(&i) && *used + len <= budget {
            kept.push(i);
            *used += len;
        }
    };
    // The command's opening and its verdict always stay.
    take(n - 1, &mut kept, &mut used);
    take(0, &mut kept, &mut used);
    // Then the middle: signal first (when judged), closest to the end first.
    let mut rest: Vec<usize> = (1..n.saturating_sub(1)).collect();
    rest.sort_by(|&a, &b| {
        let signal = |i: usize| {
            scores
                .and_then(|s| s.get(i))
                .is_some_and(|&p| p >= KEEP_THRESHOLD)
        };
        signal(b).cmp(&signal(a)).then(b.cmp(&a))
    });
    for i in rest {
        take(i, &mut kept, &mut used);
    }
    kept.sort_unstable();
    kept
}

/// The kept blocks in order, gaps marked, with a header saying what happened
/// and where the full output is.
pub fn render(
    blocks: &[Block],
    kept: &[usize],
    total_chars: usize,
    full_path: Option<&str>,
) -> String {
    let total_lines = blocks.last().map_or(0, |b| b.last_line);
    let saved = match full_path {
        Some(p) => format!("Full output saved to {p} — read_file it (with an offset) or grep it if you need the omitted lines."),
        None => "The full output could not be saved; re-run the command with a narrower filter if you need the omitted lines.".to_string(),
    };
    let mut out = format!(
        "[Long output ({total_chars} chars, {total_lines} lines) trimmed to the parts that matter. {saved}]\n"
    );
    let mut next_line = 1usize;
    for &i in kept {
        let b = &blocks[i];
        if b.first_line > next_line {
            out.push_str(&format!(
                "[… lines {}–{} omitted …]\n",
                next_line,
                b.first_line - 1
            ));
        }
        out.push_str(&b.text);
        out.push('\n');
        next_line = b.last_line + 1;
    }
    if next_line <= total_lines {
        out.push_str(&format!("[… lines {next_line}–{total_lines} omitted …]\n"));
    }
    out
}

/// The question asked about one block.
pub fn question() -> Value {
    json!({
        "keep": crate::agent::jev::noul(
            "Below is one block from the long output of a shell command an AI coding agent \
             ran. Does this block contain information the agent needs to act on: errors, \
             failures, panics, warnings, stack traces, failing test names, assertion diffs, \
             or a final status/summary line?",
            "It contains errors, failures, warnings, or result/summary information",
            "It is routine progress noise (passing items, download/compile progress, \
             repeated boilerplate)",
        )
    })
}

fn block_state(command: &str, b: &Block, total_lines: usize) -> String {
    format!(
        "Command: {command}\n\nOutput block (lines {}–{} of {total_lines}):\n{}",
        b.first_line, b.last_line, b.text
    )
}

/// How long a saved output is kept: long enough for any run, and a resumed
/// session older than this re-runs the command instead.
const SAVED_MAX_AGE: std::time::Duration = std::time::Duration::from_secs(7 * 24 * 3600);

/// Remove the files in `dir` last written more than `max_age` ago. Since
/// lean-harness Phase 6 every capped result that is not a read is saved here
/// (a grep, an MCP answer, a subagent report), not just `bash` output, so the
/// folder no longer stays small on its own. Best effort.
fn sweep(dir: &std::path::Path, max_age: std::time::Duration) {
    let Ok(entries) = std::fs::read_dir(dir) else {
        return;
    };
    for entry in entries.flatten() {
        let Ok(meta) = entry.metadata() else {
            continue;
        };
        let old = meta
            .modified()
            .ok()
            .and_then(|t| t.elapsed().ok())
            .is_some_and(|age| age > max_age);
        if meta.is_file() && old {
            std::fs::remove_file(entry.path()).ok();
        }
    }
}

/// The folder long outputs are saved to.
pub(crate) fn saved_dir() -> std::path::PathBuf {
    std::env::temp_dir().join("claudinio-code").join("outputs")
}

/// Whether `path` is one of the outputs [`save_full`] wrote: a file directly in
/// [`saved_dir`] under one of its names, once every link is resolved. The
/// markers that name these files tell the model to `read_file` or `grep` them,
/// and the folder is outside any workspace, so `tools::validate_read_path`
/// lets reads (never writes) through here. Resolving links before comparing
/// keeps the opening to this one folder: the temp folder is shared, and a link
/// planted in it, or a `..` out of it, must not lead anywhere else.
pub(crate) fn is_saved_output(path: &std::path::Path) -> bool {
    let (Ok(file), Ok(tmp)) = (path.canonicalize(), std::env::temp_dir().canonicalize()) else {
        return false;
    };
    let ours = tmp.join("claudinio-code").join("outputs");
    let named = file.file_name().and_then(|n| n.to_str()).is_some_and(|n| {
        n.ends_with(".log") && (n.starts_with("bash-") || n.starts_with("result-"))
    });
    file.parent() == Some(ours.as_path()) && named && file.is_file()
}

/// Write the full output where the model can read it back, in a file named
/// after `kind` (`bash`, or `result` for a capped tool result). Best effort.
///
/// The name is the content's hash, so the same output is one file under one
/// name however often it is saved. The path goes into the result the model
/// and `loop_watch` see: with a fresh name per save, five identical greps over
/// the cap read as five different answers and never reached the loop watch's
/// judge-free gate (`CERTAIN_EXACT`), and each repeat left another copy on
/// disk. A file already under that name with other bytes in it (a hash
/// collision, a half-written save of another thread) is left alone and the
/// output goes to a name of its own.
///
/// The folder is the user's alone (0700) and so is each file (0600): what is
/// saved can be private code or a tool's answer, and the temp folder is
/// shared. The first save of a process sweeps outputs past [`SAVED_MAX_AGE`].
pub(crate) fn save_full(kind: &str, text: &str) -> Option<String> {
    let dir = saved_dir();
    std::fs::create_dir_all(&dir).ok()?;
    #[cfg(unix)]
    {
        use std::os::unix::fs::PermissionsExt;
        std::fs::set_permissions(&dir, std::fs::Permissions::from_mode(0o700)).ok();
    }
    static SWEPT: std::sync::Once = std::sync::Once::new();
    SWEPT.call_once(|| sweep(&dir, SAVED_MAX_AGE));
    let hash = xxhash_rust::xxh3::xxh3_128(text.as_bytes());
    let path = dir.join(format!("{kind}-{hash:032x}.log"));
    if let Some(saved) = write_new(&path, text) {
        return Some(saved);
    }
    if std::fs::read(&path).is_ok_and(|bytes| bytes == text.as_bytes()) {
        // Saved before: keep it from the sweep for as long as it is named.
        if let Ok(file) = std::fs::File::options().append(true).open(&path) {
            file.set_modified(std::time::SystemTime::now()).ok();
        }
        return Some(path.to_string_lossy().to_string());
    }
    static SEQ: std::sync::atomic::AtomicU64 = std::sync::atomic::AtomicU64::new(0);
    let seq = SEQ.fetch_add(1, std::sync::atomic::Ordering::Relaxed);
    let unique = format!(
        "{kind}-{hash:032x}-{}-{}-{seq}.log",
        crate::agent::persist::now_ms(),
        std::process::id()
    );
    write_new(&dir.join(unique), text)
}

/// `text` in a file created at `path`, readable by its owner only; `None`
/// when something is already there or the write fails.
fn write_new(path: &std::path::Path, text: &str) -> Option<String> {
    let mut options = std::fs::OpenOptions::new();
    options.write(true).create_new(true);
    #[cfg(unix)]
    {
        use std::os::unix::fs::OpenOptionsExt;
        options.mode(0o600);
    }
    let mut file = options.open(path).ok()?;
    std::io::Write::write_all(&mut file, text.as_bytes()).ok()?;
    Some(path.to_string_lossy().to_string())
}

/// Fit `text` (the output of `command`) into `budget` chars — [`BUDGET_CHARS`]
/// unless the model's window calls for less (`ContextBudget::trim_chars`).
/// Unchanged when it already fits. Returns the text and what Jev cost, in USD.
pub async fn fit(
    command: &str,
    text: String,
    jev: Option<&crate::agent::jev::JevBackend>,
    budget: usize,
) -> (String, f64) {
    if text.len() <= budget {
        return (text, 0.0);
    }
    let blocks = split_blocks(&text);
    let mut cost = 0.0;
    let scores = match jev {
        Some(backend) if blocks.len() > 2 => {
            use futures::StreamExt;
            let total_lines = blocks.last().map_or(0, |b| b.last_line);
            let q = question();
            let judged: Vec<usize> = (1..blocks.len() - 1).take(MAX_JUDGED_BLOCKS).collect();
            let answers: Vec<(usize, Option<crate::agent::jev::Decision>)> =
                futures::stream::iter(judged.into_iter().map(|i| {
                    let state = block_state(command, &blocks[i], total_lines);
                    let q = &q;
                    async move { (i, crate::agent::jev::decide(backend, &state, q).await) }
                }))
                .buffer_unordered(CONCURRENCY)
                .collect()
                .await;
            let mut scores = vec![0.0; blocks.len()];
            let mut any = false;
            for (i, d) in answers {
                if let Some(d) = d {
                    cost += d.cost;
                    if let Some(p) = d.noul("keep") {
                        scores[i] = p;
                        any = true;
                    }
                }
            }
            any.then_some(scores)
        }
        _ => None,
    };
    let kept = select(&blocks, scores.as_deref(), budget);
    let path = save_full("bash", &text);
    (render(&blocks, &kept, text.len(), path.as_deref()), cost)
}

#[cfg(test)]
mod tests {
    use super::*;

    // What a capped result saves can be a grep of private code or an MCP
    // answer: only the user may read it.
    #[cfg(unix)]
    #[test]
    fn a_saved_output_is_readable_by_its_owner_only() {
        use std::os::unix::fs::PermissionsExt;
        let path = save_full("result", "secret").expect("saved");
        let mode = |p: &std::path::Path| std::fs::metadata(p).unwrap().permissions().mode() & 0o777;
        let file = std::path::Path::new(&path);
        assert_eq!(mode(file), 0o600, "{path}");
        assert_eq!(mode(file.parent().unwrap()), 0o700);
        std::fs::remove_file(file).ok();
    }

    // The path is in the result the model and the loop watch see: the same
    // output saved twice is one file under one name, a different output
    // another.
    #[test]
    fn the_same_output_saved_twice_is_one_file_under_one_name() {
        let text = format!("same output {}", std::process::id());
        let first = save_full("result", &text).expect("saved");
        let again = save_full("result", &text).expect("saved");
        let other = save_full("result", &format!("{text}!")).expect("saved");
        assert_eq!(first, again);
        assert_ne!(first, other);
        assert_eq!(std::fs::read_to_string(&first).unwrap(), text);
        std::fs::remove_file(&first).ok();
        std::fs::remove_file(&other).ok();
    }

    #[test]
    fn the_sweep_removes_saved_outputs_older_than_their_age_and_keeps_the_rest() {
        let dir = tempfile::tempdir().unwrap();
        let old = dir.path().join("result-1.log");
        let fresh = dir.path().join("bash-2.log");
        std::fs::write(&old, "old").unwrap();
        std::fs::write(&fresh, "fresh").unwrap();
        let week_ago =
            std::time::SystemTime::now() - SAVED_MAX_AGE - std::time::Duration::from_secs(60);
        std::fs::File::options()
            .write(true)
            .open(&old)
            .unwrap()
            .set_modified(week_ago)
            .unwrap();
        sweep(dir.path(), SAVED_MAX_AGE);
        assert!(!old.exists(), "an output past its age is removed");
        assert!(fresh.exists(), "a recent output stays");
    }

    fn cargo_log(passing: usize) -> String {
        let mut s = String::from(
            "   Compiling app v0.1.0\n    Finished test profile\n     Running unittests\n",
        );
        for i in 0..passing {
            s.push_str(&format!("test module_{i}::test_case_{i} ... ok\n"));
            if i == passing / 2 {
                s.push_str("test parser::handles_empty ... FAILED\n");
            }
        }
        s.push_str("\nfailures:\n\n---- parser::handles_empty stdout ----\n");
        s.push_str("thread 'parser::handles_empty' panicked at src/parser.rs:88:14:\n");
        s.push_str("index out of bounds\n\nfailures:\n    parser::handles_empty\n\n");
        s.push_str("test result: FAILED. 4000 passed; 1 failed; 0 ignored\n");
        s
    }

    #[test]
    fn blocks_cover_every_line_once_in_order() {
        let text = cargo_log(2000);
        let blocks = split_blocks(&text);
        assert!(blocks.len() > 10);
        assert_eq!(blocks[0].first_line, 1);
        for w in blocks.windows(2) {
            assert_eq!(w[1].first_line, w[0].last_line + 1);
        }
        assert_eq!(blocks.last().unwrap().last_line, text.lines().count());
        let rejoined: Vec<&str> = blocks.iter().flat_map(|b| b.text.lines()).collect();
        assert_eq!(rejoined, text.lines().collect::<Vec<_>>());
        assert!(blocks.iter().all(|b| b.text.len() <= BLOCK_CHARS + 200));
    }

    #[test]
    fn a_giant_single_line_is_cut_to_a_block() {
        let text = "x".repeat(BLOCK_CHARS * 5);
        let blocks = split_blocks(&text);
        assert_eq!(blocks.len(), 1);
        assert!(blocks[0].text.len() <= BLOCK_CHARS + 50);
    }

    #[test]
    fn without_a_judge_the_head_and_the_tail_survive() {
        let text = cargo_log(2000);
        let blocks = split_blocks(&text);
        let kept = select(&blocks, None, BUDGET_CHARS);
        assert!(kept.contains(&0));
        assert!(kept.contains(&(blocks.len() - 1)));
        // the verdict is at the end — a head cut lost exactly this
        let out = render(&blocks, &kept, text.len(), None);
        assert!(out.contains("test result: FAILED"));
        assert!(out.contains("panicked at src/parser.rs:88"));
        assert!(out.len() <= BUDGET_CHARS + 1_000);
    }

    #[test]
    fn with_a_judge_the_signal_in_the_middle_survives() {
        let text = cargo_log(2000);
        let blocks = split_blocks(&text);
        let scores: Vec<f64> = blocks
            .iter()
            .map(|b| {
                if b.text.contains("FAILED") || b.text.contains("panicked") {
                    0.97
                } else {
                    0.08
                }
            })
            .collect();
        let kept = select(&blocks, Some(&scores), BUDGET_CHARS);
        let out = render(&blocks, &kept, text.len(), None);
        assert!(
            out.contains("test parser::handles_empty ... FAILED"),
            "the mid-log failure line must survive"
        );
        assert!(out.contains("test result: FAILED"));
    }

    #[test]
    fn kept_blocks_fit_the_budget_and_stay_in_order() {
        let text = cargo_log(3000);
        let blocks = split_blocks(&text);
        let kept = select(&blocks, None, BUDGET_CHARS);
        assert!(kept.windows(2).all(|w| w[0] < w[1]));
        let size: usize = kept.iter().map(|&i| blocks[i].text.len()).sum();
        assert!(size <= BUDGET_CHARS);
    }

    #[test]
    fn gaps_are_marked_with_line_ranges_and_the_full_path_is_given() {
        let text = cargo_log(2000);
        let blocks = split_blocks(&text);
        let kept = select(&blocks, None, BUDGET_CHARS);
        let out = render(&blocks, &kept, text.len(), Some("/tmp/full.log"));
        assert!(out.contains("omitted"), "{}", &out[..300]);
        assert!(out.contains("/tmp/full.log"));
    }

    #[tokio::test]
    async fn output_that_fits_is_returned_untouched() {
        let (out, cost) = fit("ls", "a\nb\n".into(), None, BUDGET_CHARS).await;
        assert_eq!(out, "a\nb\n");
        assert_eq!(cost, 0.0);
    }

    #[tokio::test]
    async fn long_output_without_jev_keeps_the_verdict_and_saves_the_rest() {
        let text = cargo_log(3000);
        let (out, cost) = fit("cargo test", text.clone(), None, BUDGET_CHARS).await;
        assert!(out.len() < text.len());
        assert!(out.len() <= BUDGET_CHARS + 1_000);
        assert!(out.contains("test result: FAILED"));
        assert_eq!(cost, 0.0);
        let path = out
            .lines()
            .find_map(|l| l.split("saved to ").nth(1))
            .map(|p| {
                p.split_whitespace()
                    .next()
                    .unwrap()
                    .trim_end_matches(['.', ';', ')'])
                    .to_string()
            })
            .expect("the full output path is named");
        assert_eq!(std::fs::read_to_string(&path).unwrap(), text);
        let _ = std::fs::remove_file(path);
    }

    /// A small-window model gets a smaller budget, and the same output is cut
    /// to it — still verdict-last, still with the full text on disk.
    #[tokio::test]
    async fn a_smaller_budget_trims_harder_and_keeps_the_verdict() {
        let text = cargo_log(3000);
        let (wide, _) = fit("cargo test", text.clone(), None, BUDGET_CHARS).await;
        let (narrow, _) = fit("cargo test", text.clone(), None, 8_000).await;
        assert!(narrow.len() < wide.len());
        assert!(narrow.len() <= 8_000 + 1_000);
        assert!(narrow.contains("test result: FAILED"));
        // Output that fits the default but not the smaller budget is trimmed
        // only under the smaller one.
        let medium = cargo_log(300);
        assert!(medium.len() > 8_000 && medium.len() <= BUDGET_CHARS);
        let (same, _) = fit("cargo test", medium.clone(), None, BUDGET_CHARS).await;
        assert_eq!(same, medium);
        let (cut, _) = fit("cargo test", medium.clone(), None, 8_000).await;
        assert!(cut.len() < medium.len());
        for l in wide.lines().chain(narrow.lines()).chain(cut.lines()) {
            if let Some(p) = l.split("saved to ").nth(1) {
                let p = p.split_whitespace().next().unwrap();
                let _ = std::fs::remove_file(p.trim_end_matches(['.', ';', ')']));
            }
        }
    }
}
