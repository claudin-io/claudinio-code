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

use serde_json::{json, Value};

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
        let signal = |i: usize| scores.and_then(|s| s.get(i)).is_some_and(|&p| p >= KEEP_THRESHOLD);
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
pub fn render(blocks: &[Block], kept: &[usize], total_chars: usize, full_path: Option<&str>) -> String {
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
            out.push_str(&format!("[… lines {}–{} omitted …]\n", next_line, b.first_line - 1));
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

/// Write the full output where the model can read it back. Best effort.
fn save_full(text: &str) -> Option<String> {
    let dir = std::env::temp_dir().join("claudinio-code").join("outputs");
    std::fs::create_dir_all(&dir).ok()?;
    static SEQ: std::sync::atomic::AtomicU64 = std::sync::atomic::AtomicU64::new(0);
    let seq = SEQ.fetch_add(1, std::sync::atomic::Ordering::Relaxed);
    let name = format!(
        "bash-{}-{}-{seq}.log",
        crate::agent::persist::now_ms(),
        std::process::id()
    );
    let path = dir.join(name);
    std::fs::write(&path, text).ok()?;
    Some(path.to_string_lossy().to_string())
}

/// Fit `text` (the output of `command`) into [`BUDGET_CHARS`]. Unchanged when
/// it already fits. Returns the text and what Jev cost, in USD.
pub async fn fit(
    command: &str,
    text: String,
    jev: Option<&crate::agent::jev::JevBackend>,
) -> (String, f64) {
    if text.len() <= BUDGET_CHARS {
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
    let kept = select(&blocks, scores.as_deref(), BUDGET_CHARS);
    let path = save_full(&text);
    (render(&blocks, &kept, text.len(), path.as_deref()), cost)
}

#[cfg(test)]
mod tests {
    use super::*;

    fn cargo_log(passing: usize) -> String {
        let mut s = String::from("   Compiling app v0.1.0\n    Finished test profile\n     Running unittests\n");
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
            .map(|b| if b.text.contains("FAILED") || b.text.contains("panicked") { 0.97 } else { 0.08 })
            .collect();
        let kept = select(&blocks, Some(&scores), BUDGET_CHARS);
        let out = render(&blocks, &kept, text.len(), None);
        assert!(out.contains("test parser::handles_empty ... FAILED"), "the mid-log failure line must survive");
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
        let (out, cost) = fit("ls", "a\nb\n".into(), None).await;
        assert_eq!(out, "a\nb\n");
        assert_eq!(cost, 0.0);
    }

    #[tokio::test]
    async fn long_output_without_jev_keeps_the_verdict_and_saves_the_rest() {
        let text = cargo_log(3000);
        let (out, cost) = fit("cargo test", text.clone(), None).await;
        assert!(out.len() < text.len());
        assert!(out.len() <= BUDGET_CHARS + 1_000);
        assert!(out.contains("test result: FAILED"));
        assert_eq!(cost, 0.0);
        let path = out
            .lines()
            .find_map(|l| l.split("saved to ").nth(1))
            .map(|p| p.split_whitespace().next().unwrap().trim_end_matches(['.', ';', ')']).to_string())
            .expect("the full output path is named");
        assert_eq!(std::fs::read_to_string(&path).unwrap(), text);
        let _ = std::fs::remove_file(path);
    }
}
