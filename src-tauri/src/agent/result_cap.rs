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
//! - a `read_file` result: the file lines left out, in the file's numbering
//!   (a ranged read's text starts at its `start_line`; `read_file`'s own size
//!   warning is not counted), and the `start_line`/`end_line` that read them —
//!   the file is the copy;
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

/// The call a result answers, as far as its cap cares: the tool, which decides
/// what the marker offers for the rest, and the file line a `read_file`
/// result's text starts on, so the marker can name the lines it left out.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct Call<'a> {
    pub tool: &'a str,
    pub first_line: usize,
}

impl<'a> Call<'a> {
    /// The call `tool` made with `input`. `read_file` honours a range only
    /// when both ends are given (`tools::read_file::execute`); otherwise its
    /// text starts on line 1. Its line numbers are numbers (`ReadFileArgs`):
    /// a call that sent strings was refused and has no read to cap.
    pub fn of(tool: &'a str, input: &serde_json::Value) -> Self {
        let line = |key: &str| input.get(key)?.as_u64().map(|n| n as usize);
        let first_line = match (line("start_line"), line("end_line")) {
            (Some(start), Some(_)) if tool == "read_file" => start.max(1),
            _ => 1,
        };
        Call { tool, first_line }
    }

    /// A call whose input does not matter to its cap.
    pub fn tool(tool: &'a str) -> Self {
        Call {
            tool,
            first_line: 1,
        }
    }
}

/// `text` held to `max_chars` of its own characters: unchanged when it fits,
/// otherwise its head and tail around the marker. `call` decides what the
/// marker offers for the rest (see the module docs).
pub fn cap(call: Call<'_>, text: &str, max_chars: usize) -> String {
    if text.len() <= max_chars {
        return text.to_string();
    }
    let (head, tail) = cut(text, text, max_chars);
    let hint = if call.tool == "read_file" {
        ReadRange::of(text, head.len(), text.len() - tail.len(), call.first_line)
            .map_or_else(|| READ_HINT.to_string(), |r| r.render())
    } else {
        saved_copy_hint(text)
    };
    render(head, tail, text.len(), &hint)
}

/// The lines a cut left out of a `read_file` result, in the file's numbering:
/// `from..=to` of the `first..=last` the read returned. The marker says them
/// as `lines 412-1880 of 1-2400; read_file with start_line=412,
/// end_line=1880 to see them` — `read_file` returns bare text, so a count of
/// characters alone does not tell the model where the head stops or the tail
/// starts, least of all in a ranged read.
///
/// A line the cut split counts as left out (reading it again shows all of
/// it). `first` and `last` are in the marker so a second cut ([`recut`]) can
/// widen the range from what the first one kept, without the text in between.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
struct ReadRange {
    from: usize,
    to: usize,
    first: usize,
    last: usize,
}

impl ReadRange {
    /// What a cut keeping `text[..head_end]` and `text[tail_start..]` left out,
    /// for a read whose text starts on file line `first`; `None` when the cut
    /// took no whole or partial line (one line break).
    fn of(text: &str, head_end: usize, tail_start: usize, first: usize) -> Option<Self> {
        let header = crate::agent::tools::read_file::header_lines(text);
        let line = |at: usize| file_line(&text[..at], first, header);
        let skip_break = usize::from(text.as_bytes().get(head_end) == Some(&b'\n'));
        let range = ReadRange {
            from: line(head_end + skip_break),
            to: line(tail_start.checked_sub(1)?),
            first,
            last: line(text.len().checked_sub(1)?),
        };
        (range.from <= range.to).then_some(range)
    }

    /// The range after a second cut keeps `head[..head_len]` of the `head` and
    /// the last `tail_len` bytes of the `tail` the first cut kept: each side
    /// counts back from what it still knows, the head from `first`, the tail
    /// from `last`.
    ///
    /// A second cut keeps less, so its range never starts after the first
    /// one's. That bound is also what stops a head the first cut left inside
    /// `read_file`'s size warning from counting the warning's lines as the
    /// file's: such a head has no blank line closing the warning, so
    /// `header_lines` cannot see it.
    fn recut(self, head: &str, tail: &str, head_len: usize, tail_len: usize) -> Option<Self> {
        let from = if head_len >= head.len() {
            self.from
        } else {
            let skip_break = usize::from(head.as_bytes()[head_len] == b'\n');
            let header = crate::agent::tools::read_file::header_lines(head);
            file_line(&head[..head_len + skip_break], self.first, header).min(self.from)
        };
        let dropped = tail.len() - tail_len.min(tail.len());
        let to = if dropped == 0 {
            self.to
        } else {
            let after = tail.len().saturating_sub(1).max(dropped - 1);
            self.last
                .saturating_sub(tail[dropped - 1..after].matches('\n').count())
                .max(self.first)
        };
        let range = ReadRange { from, to, ..self };
        (range.from <= range.to).then_some(range)
    }

    fn render(self) -> String {
        let ReadRange {
            from,
            to,
            first,
            last,
        } = self;
        format!(
            "lines {from}-{to} of {first}-{last}; read_file with start_line={from}, \
             end_line={to} to see them"
        )
    }

    /// The range a read marker's hint names, if it names one (a marker of
    /// the first Phase 6 builds did not).
    fn parse(hint: &str) -> Option<Self> {
        let (span, _) = hint.strip_prefix("lines ")?.split_once(';')?;
        let (cut, of) = span.split_once(" of ")?;
        let pair = |s: &str| -> Option<(usize, usize)> {
            let (a, b) = s.split_once('-')?;
            Some((a.parse().ok()?, b.parse().ok()?))
        };
        let ((from, to), (first, last)) = (pair(cut)?, pair(of)?);
        Some(ReadRange {
            from,
            to,
            first,
            last,
        })
    }
}

/// The file line of the byte right after `before`, in a read whose text starts
/// on file line `first` behind `header` lines of `read_file`'s own warning
/// (a byte inside the warning counts as the read's first line).
fn file_line(before: &str, first: usize, header: usize) -> usize {
    (first + before.matches('\n').count())
        .saturating_sub(header)
        .max(first)
}

/// The hint for an output about to be cut: the path of a saved copy.
fn saved_copy_hint(text: &str) -> String {
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
fn recut(call: Call<'_>, text: &str, room: usize) -> Option<String> {
    match parse(text) {
        Some(c) if c.head.len() + c.tail.len() > room => {
            let (head, tail) = cut(c.head, c.tail, room);
            let hint = match ReadRange::parse(c.hint) {
                Some(range) => range
                    .recut(c.head, c.tail, head.len(), tail.len())
                    .map_or_else(|| READ_HINT.to_string(), |r| r.render()),
                None => c.hint.to_string(),
            };
            Some(render(head, tail, c.total, &hint))
        }
        Some(_) => None,
        None if text.len() > room => Some(cap(call, text, room)),
        None => None,
    }
}

/// The history copy of a tool result, cut to `max_chars`
/// (`ContextBudget::tool_result_chars`) so one large report, file read or
/// search cannot take the context — a cap that follows the model's window.
pub fn tool_result_block(
    tool_use_id: &str,
    call: Call<'_>,
    content: &str,
    max_chars: usize,
) -> ContentBlock {
    ContentBlock::tool_result(tool_use_id, cap(call, content, max_chars))
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
    let tools: std::collections::HashMap<&str, Call<'_>> = calls
        .iter()
        .filter_map(|b| match b {
            ContentBlock::ToolUse {
                id, name, input, ..
            } => Some((id.as_str(), Call::of(name, input))),
            _ => None,
        })
        .collect();
    let is_answer = |id: &str| tools.get(id).is_some_and(|c| c.tool == "ask_user");
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
            let call = tools
                .get(tool_use_id.as_str())
                .copied()
                .unwrap_or(Call::tool(""));
            if let Some(cut) = recut(call, text, level) {
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
        let out = cap(Call::tool("bash"), &text, 10_000);
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
        let out = cap(Call::tool("read_file"), &text, 10_000);
        let c = parse(&out).expect("a capped result parses");
        assert!(c.hint.contains("read_file with start_line="), "{}", c.hint);
        assert!(!c.hint.contains("full output is in"), "{}", c.hint);
        assert!(c.tail.ends_with("line 5000\n"));
    }

    /// The `start_line`/`end_line` a read marker offers, as numbers.
    fn offered_range(hint: &str) -> (usize, usize) {
        let num = |key: &str| -> usize {
            let rest = hint
                .split(key)
                .nth(1)
                .unwrap_or_else(|| panic!("{key} in {hint}"));
            let digits: String = rest.chars().take_while(char::is_ascii_digit).collect();
            digits
                .parse()
                .unwrap_or_else(|_| panic!("a number after {key} in {hint}"))
        };
        (num("start_line="), num("end_line="))
    }

    /// The number in a `line N` line.
    fn line_no(line: &str) -> usize {
        line.trim()
            .strip_prefix("line ")
            .and_then(|n| n.parse().ok())
            .unwrap_or_else(|| panic!("a numbered line: {line:?}"))
    }

    /// The offered range must be exactly the lines the cut left out: the
    /// head's last line is the one before it, the tail's first the one after.
    fn assert_range_is_what_was_cut(c: &Capped<'_>) {
        let (from, to) = offered_range(c.hint);
        assert_eq!(
            line_no(c.head.lines().last().unwrap()),
            from - 1,
            "{}",
            c.hint
        );
        assert_eq!(
            line_no(c.tail.lines().next().unwrap()),
            to + 1,
            "{}",
            c.hint
        );
    }

    #[test]
    fn a_read_takes_its_first_line_from_a_range_only_when_both_ends_are_given() {
        let call = |input| Call::of("read_file", &input);
        use serde_json::json;
        assert_eq!(
            call(json!({"path": "a", "start_line": 300, "end_line": 900})).first_line,
            300
        );
        // read_file's arguments are numbers: a string range is refused before
        // there is a result to cap, so it names no first line here either.
        let strings = json!({"path": "a", "start_line": "40", "end_line": "90"});
        assert!(
            serde_json::from_value::<crate::agent::tools::read_file::ReadFileArgs>(strings.clone())
                .is_err()
        );
        assert_eq!(call(strings).first_line, 1);
        // read_file reads the whole file unless both ends are given.
        assert_eq!(call(json!({"path": "a", "start_line": 300})).first_line, 1);
        assert_eq!(call(json!({"path": "a"})).first_line, 1);
        let grep = Call::of("grep", &json!({"start_line": 5, "end_line": 9}));
        assert_eq!(grep.first_line, 1);
    }

    #[test]
    fn a_capped_read_names_the_lines_it_left_out() {
        let text = numbered(5_000);
        let out = cap(Call::tool("read_file"), &text, 10_000);
        let c = parse(&out).expect("a capped result parses");
        assert_range_is_what_was_cut(&c);
        assert!(c.hint.contains("of 1-5000"), "{}", c.hint);
    }

    // A ranged read's text starts at its start_line; the lines the marker
    // names are the file's, not the result's.
    #[test]
    fn a_capped_ranged_read_names_the_file_lines_it_left_out() {
        let text = (300..=5_299)
            .map(|i| format!("line {i}"))
            .collect::<Vec<_>>()
            .join("\n");
        let input = serde_json::json!({"path": "a", "start_line": 300, "end_line": 5_299});
        let out = cap(Call::of("read_file", &input), &text, 10_000);
        let c = parse(&out).expect("a capped result parses");
        assert!(c.head.starts_with("line 300\n"));
        assert_range_is_what_was_cut(&c);
        assert!(c.hint.contains("of 300-5299"), "{}", c.hint);
    }

    // A read read_file itself shortened opens with its size warning: the
    // file's line 1 is the first line after it.
    #[test]
    fn a_capped_read_behind_the_size_warning_counts_lines_from_the_file() {
        let dir = tempfile::tempdir().unwrap();
        let path = dir.path().join("big.txt");
        std::fs::write(&path, numbered(30_000)).unwrap();
        let input = serde_json::json!({"path": path.to_str().unwrap()});
        let text =
            crate::agent::tools::read_file::execute(serde_json::from_value(input.clone()).unwrap())
                .unwrap();
        assert!(text.contains("FILE SIZE WARNING"), "the read is shortened");
        let out = cap(Call::of("read_file", &input), &text, 10_000);
        let c = parse(&out).expect("a capped result parses");
        assert_range_is_what_was_cut(&c);
        assert!(c.hint.contains("of 1-"), "{}", c.hint);
    }

    // fit_round cuts a capped read again from what it kept: the range grows
    // to what the second cut left out, still in the file's lines.
    #[test]
    fn a_read_cut_twice_names_the_lines_both_cuts_left_out() {
        let read = (300..=5_299)
            .map(|i| format!("line {i}"))
            .collect::<Vec<_>>()
            .join("\n");
        let input = serde_json::json!({"path": "a", "start_line": 300, "end_line": 5_299});
        let calls = vec![
            ContentBlock::tool_use("r0", "read_file", input.clone()),
            ContentBlock::tool_use("r1", "read_file", input.clone()),
        ];
        let call = Call::of("read_file", &input);
        let mut round = vec![
            tool_result_block("r0", call, &read, 15_000),
            tool_result_block("r1", call, &read, 15_000),
        ];
        fit_round(&mut round, &calls, 12_000);
        for b in &round {
            let t = text(b);
            let c = parse(&t).expect("still a marker");
            assert!(c.head.len() + c.tail.len() <= 6_000);
            assert_eq!(c.total, read.len());
            assert_range_is_what_was_cut(&c);
            assert!(c.hint.contains("of 300-5299"), "{}", c.hint);
        }
    }

    // A cut that keeps only part of read_file's size warning has shown none of
    // the file: cut again, smaller, the range it names still starts at the
    // file's first line, never at a line counted inside the warning.
    #[test]
    fn a_warned_read_cut_twice_inside_its_warning_names_the_range_from_line_one() {
        let dir = tempfile::tempdir().unwrap();
        let path = dir.path().join("big.txt");
        std::fs::write(&path, numbered(30_000)).unwrap();
        let input = serde_json::json!({"path": path.to_str().unwrap()});
        let text =
            crate::agent::tools::read_file::execute(serde_json::from_value(input.clone()).unwrap())
                .unwrap();
        let warning = crate::agent::tools::read_file::header_lines(&text);
        assert!(warning > 0, "the read is shortened");
        let call = Call::of("read_file", &input);
        let once = cap(call, &text, 600);
        let first = parse(&once).expect("capped once");
        assert!(
            !first.head.contains("\n\n"),
            "the head is inside the warning"
        );
        assert_eq!(offered_range(first.hint).0, 1, "{}", first.hint);

        let twice = recut(call, &once, 300).expect("cut again");
        let second = parse(&twice).expect("capped twice");
        assert!(second.head.lines().count() >= 2, "{:?}", second.head);
        assert_eq!(offered_range(second.hint).0, 1, "{}", second.hint);
    }

    #[test]
    fn a_result_within_the_cap_is_untouched() {
        let text = numbered(10);
        assert_eq!(cap(Call::tool("bash"), &text, text.len()), text);
        assert_eq!(parse(&text), None);
    }

    #[test]
    fn a_long_single_line_is_cut_on_characters_without_splitting_one() {
        let text = "é".repeat(20_000);
        let out = cap(Call::tool("grep"), &text, 9_999);
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
        let out = cap(Call::tool("read_file"), &text, 8_000);
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
        let out = cap(Call::tool("bash"), &text, 8_000);
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
        let capped = |id: &str, total: usize| {
            tool_result_block(id, Call::tool("read_file"), &"x".repeat(total), 15_000)
        };

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
            // One long line: what is left out is all on line 1.
            assert!(
                c.hint
                    .contains("lines 1-1 of 1-1; read_file with start_line=1, end_line=1"),
                "the way to the rest is kept: {}",
                c.hint
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
