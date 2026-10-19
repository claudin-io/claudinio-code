//! The cap a single tool result is held to in the history
//! (`ContextBudget::tool_result_chars`), and the share of a round it gets
//! when the results of one round are held together ([`fit_round`]).
//!
//! The cap used to keep the first N characters and say how many there were in
//! all. A head cut is right for the start of a file and wrong for nearly
//! everything else: a test run, a build, a search and a long listing put the
//! answer — the failure, the summary, the last matches — at the end. Since
//! lean-harness Phase 6 a capped result keeps its head **and** its tail (about
//! 70/30, on line boundaries when a line ends close enough) around one marker
//! line that says how much was left out of how much, and how to get it:
//!
//! - a `read_file` result: read the lines in between with
//!   `start_line`/`end_line` (the file is the copy);
//! - anything else: the whole output is written to a temp file first, as
//!   `output_trim` does for `bash`, and the marker names it; an output that
//!   already names its saved copy (`output_trim`'s first line) is not saved
//!   again.
//!
//! [`parse`] reads the marker back — `fit_round` re-cuts a capped result from
//! what it kept, never from its marker, and still reports the size the result
//! really had — and `prune` uses it to know a capped read is not the whole
//! file. A marker counts only when its numbers add up to the text around it,
//! so text that looks like one inside a result (a log of an earlier session)
//! is never taken for it.

use crate::agent::provider::{ContentBlock, ToolResultContent};

/// Share of a cut result's room given to its head; the rest goes to the tail.
pub const HEAD_PERCENT: usize = 70;
/// How the marker opens: on a line of its own, between head and tail.
const MARK_OPEN: &str = "\n[… ";
const MARK_CLOSE: &str = " …]\n";
/// The size note of a head-only cut, written before Phase 6.
const LEGACY_NOTE: (&str, &str) = ("...(truncated, ", " chars total)");
/// What a `read_file` marker says instead of naming a copy.
const READ_HINT: &str = "read_file with start_line/end_line to see them";
/// `output_trim`'s first line when it saved the whole output.
const TRIMMED_SAVED: &str = "Full output saved to ";

/// What a capped result kept and what it says it lost.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Capped<'a> {
    pub head: &'a str,
    pub tail: &'a str,
    /// The size of the result before any cap.
    pub total: usize,
    /// How to get what was left out; empty when nothing says.
    pub hint: &'a str,
}

/// `text` held to `max_chars` of its own characters: unchanged when it fits,
/// otherwise its head and tail around the marker. `tool` decides what the
/// marker offers for the rest (see the module docs).
pub fn cap(tool: &str, text: &str, max_chars: usize) -> String {
    if text.len() <= max_chars {
        return text.to_string();
    }
    let hint = hint_for(tool, text);
    let (head, tail) = cut(text, text, max_chars);
    render(head, tail, text.len(), &hint)
}

/// The hint for a result of `tool` about to be cut: a line range for a file,
/// otherwise the path of a saved copy.
fn hint_for(tool: &str, text: &str) -> String {
    if tool == "read_file" {
        return READ_HINT.to_string();
    }
    if text
        .lines()
        .next()
        .is_some_and(|l| l.contains(TRIMMED_SAVED))
    {
        return "the full output's path is in the first line".to_string();
    }
    match crate::agent::output_trim::save_full("result", text) {
        Some(path) => format!("the full output is in {path}; read_file or grep it"),
        None => String::new(),
    }
}

/// A head of `from_head` and a tail of `from_tail` that together hold at most
/// `room` characters, about [`HEAD_PERCENT`] to the head; what one side cannot
/// use goes to the other. For a whole text both are the text.
fn cut<'a>(from_head: &'a str, from_tail: &'a str, room: usize) -> (&'a str, &'a str) {
    let mut head_room = room * HEAD_PERCENT / 100;
    let mut tail_room = room - head_room;
    if from_head.len() < head_room {
        tail_room += head_room - from_head.len();
        head_room = from_head.len();
    } else if from_tail.len() < tail_room {
        head_room += tail_room - from_tail.len();
        tail_room = from_tail.len();
    }
    (head_of(from_head, head_room), tail_of(from_tail, tail_room))
}

/// At most `max` bytes from the start of `s`, ending before a line break when
/// one falls in the second half of that room (a whole line reads better than
/// half of one; a cut that loses more than half the room to find one does not
/// pay).
fn head_of(s: &str, max: usize) -> &str {
    if max >= s.len() {
        return s;
    }
    let mut end = max;
    while !s.is_char_boundary(end) {
        end -= 1;
    }
    match s[..end].rfind('\n') {
        Some(nl) if nl >= max / 2 => &s[..nl],
        _ => &s[..end],
    }
}

/// At most `max` bytes from the end of `s`, starting after a line break when
/// one falls in the first half of that room.
fn tail_of(s: &str, max: usize) -> &str {
    if max >= s.len() {
        return s;
    }
    let mut start = s.len() - max;
    while !s.is_char_boundary(start) {
        start += 1;
    }
    match s[start..].find('\n') {
        Some(nl) if nl < max / 2 => &s[start + nl + 1..],
        _ => &s[start..],
    }
}

fn render(head: &str, tail: &str, total: usize, hint: &str) -> String {
    let omitted = total - head.len() - tail.len();
    let hint = if hint.is_empty() {
        String::new()
    } else {
        format!("; {hint}")
    };
    format!("{head}{MARK_OPEN}{omitted} of {total} chars omitted{hint}{MARK_CLOSE}{tail}")
}

/// The cap's marker in `text`, if a cap left one: the first marker line whose
/// counts add up to the text around it, or the head-only note of a cut made
/// before Phase 6.
pub fn parse(text: &str) -> Option<Capped<'_>> {
    let mut from = 0;
    while let Some(found) = text[from..].find(MARK_OPEN) {
        let at = from + found;
        from = at + MARK_OPEN.len();
        let rest = &text[from..];
        let Some(close) = rest.find(MARK_CLOSE) else {
            break;
        };
        let inner = &rest[..close];
        let Some((counts, hint)) = inner
            .split_once(" chars omitted")
            .map(|(c, h)| (c, h.strip_prefix("; ").unwrap_or(h)))
        else {
            continue;
        };
        let Some((omitted, total)) = counts.split_once(" of ") else {
            continue;
        };
        let (Ok(omitted), Ok(total)) = (omitted.parse::<usize>(), total.parse::<usize>()) else {
            continue;
        };
        let head = &text[..at];
        let tail = &rest[close + MARK_CLOSE.len()..];
        if head.len() + tail.len() + omitted == total {
            return Some(Capped {
                head,
                tail,
                total,
                hint,
            });
        }
    }
    let (open, close) = LEGACY_NOTE;
    let at = text.rfind(open)?;
    let total = text[at + open.len()..]
        .strip_suffix(close)?
        .parse::<usize>()
        .ok()
        .filter(|&total| total >= at)?;
    Some(Capped {
        head: &text[..at],
        tail: "",
        total,
        hint: "",
    })
}

/// The characters of a result's own text: what a cap kept of it, not its
/// marker.
pub fn body_len(text: &str) -> usize {
    match parse(text) {
        Some(c) => c.head.len() + c.tail.len(),
        None => text.len(),
    }
}

/// `text` held to `room` characters of its own text, keeping what an earlier
/// cap said: a capped result is cut from what it kept, with the size it
/// really had and the same way to get the rest. `None` when it fits.
fn recut(tool: &str, text: &str, room: usize) -> Option<String> {
    match parse(text) {
        Some(c) if c.head.len() + c.tail.len() > room => {
            let (head, tail) = cut(c.head, c.tail, room);
            Some(render(head, tail, c.total, c.hint))
        }
        Some(_) => None,
        None if text.len() > room => Some(cap(tool, text, room)),
        None => None,
    }
}

/// The history copy of a tool result, cut to `max_chars`
/// (`ContextBudget::tool_result_chars`) so one large report, file read or
/// search cannot take the context — a cap that follows the model's window.
pub fn tool_result_block(
    tool_use_id: &str,
    tool: &str,
    content: &str,
    max_chars: usize,
) -> ContentBlock {
    ContentBlock::tool_result(tool_use_id, cap(tool, content, max_chars))
}

/// The least a result is cut to: below this it says nothing but its own note.
const MIN_ROUND_SHARE: usize = 200;

/// Hold the results of one round to `round_chars` together
/// (`ContextBudget::round_chars`). The cap per result does not bound a round:
/// a model that reads four files in one turn gets four results, and the round
/// just read is the one thing no prune will cut.
///
/// The room is shared the way water fills glasses: a small result keeps all of
/// itself, and what it does not use goes to the larger ones. A result the cap
/// per result already cut is cut from what it kept, not from its marker, and
/// still reports the size it really had — that marker is the only way the
/// model learns how much of a file it has not seen. The user's answers
/// (`ask_user`, looked up in `calls`, the assistant turn these results answer)
/// are never cut.
pub fn fit_round(blocks: &mut [ContentBlock], calls: &[ContentBlock], round_chars: usize) {
    if round_chars == usize::MAX {
        return;
    }
    let tools: std::collections::HashMap<&str, &str> = calls
        .iter()
        .filter_map(|b| match b {
            ContentBlock::ToolUse { id, name, .. } => Some((id.as_str(), name.as_str())),
            _ => None,
        })
        .collect();
    let is_answer = |id: &str| tools.get(id) == Some(&"ask_user");
    let mut kept_whole = 0usize;
    let mut bodies: Vec<usize> = Vec::new();
    for b in blocks.iter() {
        if let ContentBlock::ToolResult {
            tool_use_id,
            content: ToolResultContent::Text(text),
            ..
        } = b
        {
            if is_answer(tool_use_id) {
                kept_whole += text.len();
            } else {
                bodies.push(body_len(text));
            }
        }
    }
    let mut room = round_chars.saturating_sub(kept_whole);
    if bodies.iter().sum::<usize>() <= room {
        return;
    }
    // The level every larger result is cut to.
    bodies.sort_unstable();
    let mut level = 0;
    for (i, len) in bodies.iter().enumerate() {
        let share = room / (bodies.len() - i);
        if *len > share {
            level = share;
            break;
        }
        room -= len;
    }
    let level = level.max(MIN_ROUND_SHARE);
    for b in blocks.iter_mut() {
        if let ContentBlock::ToolResult {
            tool_use_id,
            content: ToolResultContent::Text(text),
            ..
        } = b
            && !is_answer(tool_use_id)
        {
            let tool = tools.get(tool_use_id.as_str()).copied().unwrap_or("");
            if let Some(cut) = recut(tool, text, level) {
                *text = cut;
            }
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn numbered(lines: usize) -> String {
        (1..=lines).map(|i| format!("line {i}\n")).collect()
    }

    #[test]
    fn a_capped_output_keeps_its_head_and_its_tail_and_saves_the_whole() {
        let text = numbered(5_000);
        let out = cap("bash", &text, 10_000);
        let c = parse(&out).expect("a capped result parses");
        assert_eq!(c.total, text.len());
        // About 70/30, on line boundaries: the opening and the verdict.
        assert!(c.head.starts_with("line 1\n"), "{}", &c.head[..20]);
        assert!(c.tail.ends_with("line 5000\n"), "{}", c.tail);
        assert!((3_500..=7_000).contains(&c.head.len()), "{}", c.head.len());
        assert!((1_500..=3_000).contains(&c.tail.len()), "{}", c.tail.len());
        assert!(text.starts_with(c.head) && text.ends_with(c.tail));
        assert!(
            text[c.head.len()..].starts_with('\n'),
            "the head ends on a whole line"
        );
        assert!(
            c.tail.starts_with("line "),
            "the tail starts on a whole line"
        );
        let omitted = text.len() - c.head.len() - c.tail.len();
        assert!(
            out.contains(&format!("[… {omitted} of {} chars omitted", text.len())),
            "{out}"
        );
        // The marker names a file holding all of it.
        let path = c
            .hint
            .split("the full output is in ")
            .nth(1)
            .and_then(|r| r.split(';').next())
            .expect("a saved path");
        assert_eq!(std::fs::read_to_string(path).unwrap(), text);
        std::fs::remove_file(path).ok();
    }

    #[test]
    fn a_capped_read_points_at_a_line_range_instead_of_a_copy() {
        let text = numbered(5_000);
        let out = cap("read_file", &text, 10_000);
        let c = parse(&out).expect("a capped result parses");
        assert!(c.hint.contains("start_line/end_line"), "{}", c.hint);
        assert!(!c.hint.contains("full output is in"), "{}", c.hint);
        assert!(c.tail.ends_with("line 5000\n"));
    }

    #[test]
    fn a_result_within_the_cap_is_untouched() {
        let text = numbered(10);
        assert_eq!(cap("bash", &text, text.len()), text);
        assert_eq!(parse(&text), None);
    }

    #[test]
    fn a_long_single_line_is_cut_on_characters_without_splitting_one() {
        let text = "é".repeat(20_000);
        let out = cap("grep", &text, 9_999);
        let c = parse(&out).expect("parses");
        assert_eq!(c.total, text.len());
        assert!(c.head.len() + c.tail.len() <= 9_999);
        assert!(c.head.chars().all(|ch| ch == 'é') && c.tail.chars().all(|ch| ch == 'é'));
        if let Some(p) = c.hint.split("the full output is in ").nth(1) {
            std::fs::remove_file(p.split(';').next().unwrap()).ok();
        }
    }

    // The tail is the result's own text and may hold anything — a log of an
    // earlier session's results, say. Only the marker whose numbers add up
    // to the text around it is the cap's.
    #[test]
    fn a_lookalike_marker_in_the_kept_text_is_not_taken_for_the_cap() {
        let mut text = numbered(3_000);
        text.push_str("\n[… 5 of 10 chars omitted …]\nthe end\n");
        let out = cap("read_file", &text, 8_000);
        let c = parse(&out).expect("parses");
        assert_eq!(c.total, text.len());
        assert!(c.tail.contains("[… 5 of 10 chars omitted …]"));
        assert_eq!(parse("ab\n[… 5 of 10 chars omitted …]\ncd"), None);
    }

    // Sessions written before Phase 6 hold head-only cuts.
    #[test]
    fn a_head_cut_from_before_phase_6_still_parses() {
        let c = parse("abc...(truncated, 500 chars total)").expect("legacy");
        assert_eq!((c.head, c.tail, c.total), ("abc", "", 500));
    }

    // A file that merely ends with the old note's words is not a cut: its
    // "total" is smaller than the text, and re-cutting it as one would count
    // a negative omission.
    #[test]
    fn an_old_note_that_does_not_add_up_is_not_a_cut() {
        let text_of = text;
        let text = format!("{}...(truncated, 5 chars total)", "x".repeat(400));
        assert_eq!(parse(&text), None);
        let calls = vec![ContentBlock::tool_use(
            "r",
            "read_file",
            serde_json::json!({}),
        )];
        let mut round = vec![ContentBlock::tool_result("r", text.clone())];
        fit_round(&mut round, &calls, 300);
        let cut = text_of(&round[0]);
        let c = parse(&cut).expect("cut as a whole text");
        assert_eq!(c.total, text.len());
    }

    // `output_trim` already saved the whole output and says where in its
    // first line; the cap keeps that line and does not save a copy of the
    // trimmed text.
    #[test]
    fn a_trimmed_output_that_names_its_saved_copy_is_not_saved_again() {
        let text = format!(
            "[Long output (90000 chars, 9000 lines) trimmed to the parts that matter. Full output \
             saved to /tmp/x.log — read_file it (with an offset) or grep it if you need the \
             omitted lines.]\n{}",
            numbered(3_000)
        );
        let out = cap("bash", &text, 8_000);
        let c = parse(&out).expect("parses");
        assert!(c.head.contains("/tmp/x.log"));
        assert!(!c.hint.contains("full output is in"), "{}", c.hint);
        assert!(c.hint.contains("first line"), "{}", c.hint);
    }

    // ── One round held together (moved from run_loop.rs with fit_round) ─────

    fn text(b: &ContentBlock) -> String {
        match b {
            ContentBlock::ToolResult { content, .. } => content.as_text().into_owned(),
            _ => panic!("a tool result"),
        }
    }

    /// One `read_file` call per id, so no cut saves a copy.
    fn reads(n: usize) -> Vec<ContentBlock> {
        (0..n)
            .map(|i| ContentBlock::tool_use(format!("t{i}"), "read_file", serde_json::json!({})))
            .collect()
    }

    // Behaviour deliberately changed by lean-harness Phase 6: a cut keeps a
    // head and a tail around the marker instead of a head and a trailing
    // note; the shares and the reported size are what they were.
    #[test]
    fn a_round_of_results_is_held_to_its_share_together() {
        let round = |sizes: &[usize]| -> Vec<ContentBlock> {
            sizes
                .iter()
                .enumerate()
                .map(|(i, n)| ContentBlock::tool_result(format!("t{i}"), "x".repeat(*n)))
                .collect()
        };

        // Under the cap, or where there is none: untouched.
        let mut fits = round(&[10_000, 10_000]);
        fit_round(&mut fits, &reads(2), 30_000);
        assert!(fits.iter().all(|b| text(b).len() == 10_000));
        let mut unbounded = round(&[24_000; 6]);
        fit_round(&mut unbounded, &reads(6), usize::MAX);
        assert!(unbounded.iter().all(|b| text(b).len() == 24_000));

        // Over it: the small result keeps all of itself, and the room it does
        // not use goes to the others — (30_000 - 400) / 3 each.
        let mut over = round(&[15_000, 15_000, 15_000, 400]);
        fit_round(&mut over, &reads(4), 30_000);
        for b in &over[..3] {
            let t = text(b);
            let c = parse(&t).expect("cut with a marker");
            assert_eq!(c.head.len() + c.tail.len(), 9_866);
            assert_eq!(c.head.len(), 9_866 * HEAD_PERCENT / 100);
            assert_eq!(c.total, 15_000);
        }
        assert_eq!(text(&over[3]).len(), 400);
        let bodies: usize = over.iter().map(|b| body_len(&text(b))).sum();
        assert!(bodies <= 30_000, "{bodies}");
    }

    // The cap per result runs first and leaves its marker. A round of two
    // such results is exactly the round's room plus two markers — not "over"
    // — and a result cut twice must still say how large the file was, not how
    // large the first cut left it.
    #[test]
    fn a_result_cut_twice_still_reports_the_size_it_really_had() {
        let capped =
            |id: &str, total: usize| tool_result_block(id, "read_file", &"x".repeat(total), 15_000);

        let mut two = vec![capped("t0", 60_000), capped("t1", 48_213)];
        let before: Vec<String> = two.iter().map(text).collect();
        fit_round(&mut two, &reads(2), 30_000);
        assert_eq!(two.iter().map(text).collect::<Vec<_>>(), before);

        let mut four = vec![
            capped("t0", 60_000),
            capped("t1", 48_213),
            capped("t2", 20_000),
            capped("t3", 16_000),
        ];
        fit_round(&mut four, &reads(4), 30_000);
        for (b, total) in four.iter().zip([60_000, 48_213, 20_000, 16_000]) {
            let t = text(b);
            let c = parse(&t).expect("still a marker");
            assert_eq!(c.head.len() + c.tail.len(), 7_500, "{total}");
            assert_eq!(c.total, total);
            assert!(
                c.hint.contains("start_line/end_line"),
                "the way to the rest is kept"
            );
        }
    }

    // A result the round cuts that no cap cut before gets the same marker as
    // the cap's, saved copy included.
    #[test]
    fn a_round_cut_of_a_whole_output_saves_it_like_the_cap() {
        let calls = vec![ContentBlock::tool_use("g", "grep", serde_json::json!({}))];
        let mut round = vec![ContentBlock::tool_result("g", "match\n".repeat(2_000))];
        fit_round(&mut round, &calls, 3_000);
        let t = text(&round[0]);
        let c = parse(&t).expect("cut with a marker");
        assert_eq!(c.total, 12_000);
        assert!(c.tail.ends_with("match\n"));
        let path = c
            .hint
            .split("the full output is in ")
            .nth(1)
            .and_then(|r| r.split(';').next())
            .expect("a saved copy");
        assert_eq!(
            std::fs::read_to_string(path).unwrap(),
            "match\n".repeat(2_000)
        );
        std::fs::remove_file(path).ok();
    }

    // What the user answered is not a file that can be read again.
    #[test]
    fn a_round_never_cuts_what_the_user_answered() {
        let calls = vec![
            ContentBlock::tool_use("ask", "ask_user", serde_json::json!({})),
            ContentBlock::tool_use("r1", "read_file", serde_json::json!({})),
            ContentBlock::tool_use("r2", "read_file", serde_json::json!({})),
        ];
        let answer = format!(
            "Question: which?\nAnswer: {}",
            "the second one. ".repeat(90)
        );
        let mut round = vec![
            ContentBlock::tool_result("ask", answer.clone()),
            ContentBlock::tool_result("r1", "x".repeat(2_000)),
            ContentBlock::tool_result("r2", "y".repeat(2_000)),
        ];
        fit_round(&mut round, &calls, 3_864);
        assert_eq!(text(&round[0]), answer);
        // The reads share what the answer left.
        let room = (3_864 - answer.len()) / 2;
        let r1 = text(&round[1]);
        let c = parse(&r1).expect("cut");
        assert_eq!(c.head.len() + c.tail.len(), room);
        assert_eq!(c.total, 2_000);
        assert!(body_len(&text(&round[2])) <= room);
    }
}
