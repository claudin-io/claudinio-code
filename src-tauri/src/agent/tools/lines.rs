//! List results as lines of text.
//!
//! A search or a listing used to come back as pretty-printed JSON: one field
//! per line, the key repeated for every row, the absolute path repeated for
//! every match in a file. A tool result is read once by the model and then
//! carried — re-read on every later request of the session — so its format is
//! paid for many times over. Re-encoding the list results stored in 24 real
//! sessions this way took them from 962k characters to about 282k (-70%),
//! with nothing the model acted on left out: database ids and column numbers
//! go, and a path is written once per file instead of once per row.
//!
//! The formats are the ones a shell would print — `path:line`, ripgrep's
//! grouped output — so a model needs no schema to read them, and the UI parses
//! them back into rows (`parseLineList` in `ToolBody.tsx`). Change one side and
//! the other has to follow.

use super::grep::GrepMatch;
use super::list_dir::DirEntryInfo;
use crate::code_intel::db::{SearchResult, SemanticSearchResult, SymbolRecord};

/// A matched or declared line longer than this is cut. One minified file would
/// otherwise spend the whole result on a single row.
const MAX_LINE_CHARS: usize = 240;

/// `text` cut to [`MAX_LINE_CHARS`] characters.
fn clipped(mut text: String) -> String {
    if let Some((cut, _)) = text.char_indices().nth(MAX_LINE_CHARS) {
        text.truncate(cut);
        text.push('…');
    }
    text
}

/// A declaration on one line: a signature can span several, and its line
/// breaks and indentation are layout, not content.
fn one_line(text: &str) -> String {
    clipped(text.split_whitespace().collect::<Vec<_>>().join(" "))
}

/// True when `text` holds `name` as a name of its own, not as part of a longer
/// identifier: `id` is not in `fn valid(x: u8)`.
fn names(text: &str, name: &str) -> bool {
    let part_of_word = |c: char| c.is_alphanumeric() || c == '_';
    if !name.chars().any(part_of_word) {
        // `{` or `*`, as an import is indexed: nothing to delimit.
        return text.contains(name);
    }
    text.match_indices(name).any(|(at, _)| {
        let before = text[..at].chars().next_back();
        let after = text[at + name.len()..].chars().next();
        !before.is_some_and(part_of_word) && !after.is_some_and(part_of_word)
    })
}

/// What a symbol's row says after its location: the kind, then its signature —
/// or its name, when the index holds no signature or one that does not carry
/// the name (the index keeps only the start of a long declaration).
fn symbol_text(kind: &str, name: &str, signature: Option<&str>) -> String {
    let signature = one_line(signature.unwrap_or(""));
    if signature.is_empty() {
        format!("{kind} {name}")
    } else if names(&signature, name) {
        format!("{kind} {signature}")
    } else {
        format!("{kind} {name}: {signature}")
    }
}

/// `start` or `start-end`.
fn line_range(start: i64, end: i64) -> String {
    if end > start {
        format!("{start}-{end}")
    } else {
        start.to_string()
    }
}

/// The directory, then one name per line; a directory's name ends in `/`.
/// Every entry's path is the first line plus its own.
pub fn list_dir(dir: &str, entries: &[DirEntryInfo]) -> String {
    let mut out = format!("{}/", dir.trim_end_matches('/'));
    if entries.is_empty() {
        out.push_str("\n(empty)");
    }
    for e in entries {
        out.push('\n');
        out.push_str(&e.name);
        if e.is_dir {
            out.push('/');
        }
    }
    out
}

/// A count, then ripgrep's grouped form: each file once, its matches under it
/// as `line: text`, a blank line between files. The text is the line as it is
/// in the file, less the indentation around it.
pub fn grep(matches: &[GrepMatch]) -> String {
    if matches.is_empty() {
        return "No matches.".into();
    }
    let mut files = 0usize;
    let mut body = String::new();
    let mut current: Option<&str> = None;
    for m in matches {
        if current != Some(m.file.as_str()) {
            current = Some(m.file.as_str());
            files += 1;
            body.push_str("\n\n");
            body.push_str(&m.file);
        }
        body.push_str(&format!(
            "\n{}: {}",
            m.line,
            clipped(m.content.trim().to_string())
        ));
    }
    let matches_word = if matches.len() == 1 {
        "match"
    } else {
        "matches"
    };
    let files_word = if files == 1 { "file" } else { "files" };
    format!(
        "{} {matches_word} in {files} {files_word}{body}",
        matches.len()
    )
}

/// `code_search` and `symbol_lookup`: one `path:line kind signature` per hit,
/// in rank order.
pub fn symbols(results: &[SearchResult]) -> String {
    if results.is_empty() {
        return "No symbols found.".into();
    }
    results
        .iter()
        .map(|r| {
            format!(
                "{}:{} {}",
                r.file_path,
                r.start_line,
                symbol_text(&r.kind, &r.name, r.signature.as_deref())
            )
        })
        .collect::<Vec<_>>()
        .join("\n")
}

/// The file and how many symbols it declares, then one `lines kind signature`
/// per symbol in file order.
pub fn outline(file: &str, symbols: &[SymbolRecord]) -> String {
    if symbols.is_empty() {
        return format!("{file} (no symbols indexed)");
    }
    let noun = if symbols.len() == 1 {
        "symbol"
    } else {
        "symbols"
    };
    let mut out = format!("{file} ({} {noun})", symbols.len());
    for s in symbols {
        out.push_str(&format!(
            "\n{} {}",
            line_range(s.start_line, s.end_line),
            symbol_text(&s.kind, &s.name, s.signature.as_deref())
        ));
    }
    out
}

/// How the ranking was made (and why it may be partial), then one
/// `path:lines kind signature [match score]` per hit; the top hits carry their
/// source in a fence right under them. The tool's description tells the model
/// how to read the bracket, so the two change together.
pub fn semantic(mode: &str, note: Option<&str>, results: &[SemanticSearchResult]) -> String {
    let mut out = match note {
        Some(note) => format!("mode: {mode} ({note})"),
        None => format!("mode: {mode}"),
    };
    if results.is_empty() {
        out.push_str("\nNo results.");
    }
    for r in results {
        out.push_str(&format!(
            "\n{}:{} {} [{} {:.2}]",
            r.file_path,
            line_range(r.start_line, r.end_line),
            symbol_text(&r.kind, &r.name, r.signature.as_deref()),
            r.match_type,
            r.score
        ));
        if let Some(snippet) = r.snippet.as_deref().filter(|s| !s.trim().is_empty()) {
            // A fence the snippet cannot close by accident: tildes when it
            // holds backticks, and more tildes than any run it holds.
            let mut fence = String::from("```");
            if snippet.contains("```") {
                fence = "~~~~".into();
                while snippet.contains(&fence) {
                    fence.push('~');
                }
            }
            out.push_str(&format!("\n{fence}\n{}\n{fence}", snippet.trim_end()));
        }
    }
    out
}

#[cfg(test)]
mod tests {
    use super::*;

    fn entry(name: &str, is_dir: bool) -> DirEntryInfo {
        DirEntryInfo {
            name: name.into(),
            path: format!("/ws/src/{name}"),
            is_dir,
        }
    }

    fn hit(file: &str, line: usize, content: &str) -> GrepMatch {
        GrepMatch {
            file: file.into(),
            line,
            content: content.into(),
        }
    }

    fn symbol(name: &str, kind: &str, signature: Option<&str>, lines: (i64, i64)) -> SymbolRecord {
        SymbolRecord {
            id: 162_863,
            file_id: 62,
            name: name.into(),
            kind: kind.into(),
            signature: signature.map(str::to_string),
            start_line: lines.0,
            start_col: 1,
            end_line: lines.1,
            end_col: 80,
            file_path: Some("/ws/src/app.rs".into()),
        }
    }

    #[test]
    fn a_directory_is_its_path_and_then_one_name_per_line() {
        let text = list_dir("/ws/src/", &[entry("agent", true), entry("main.rs", false)]);
        assert_eq!(text, "/ws/src/\nagent/\nmain.rs");
        assert_eq!(list_dir("/ws/empty", &[]), "/ws/empty/\n(empty)");
    }

    #[test]
    fn matches_are_grouped_under_the_file_they_are_in() {
        let text = grep(&[
            hit("/ws/a.rs", 12, "    let x = 1;"),
            hit("/ws/a.rs", 40, "\tlet y  =  x;  "),
            hit("/ws/b.rs", 3, "use x;"),
        ]);
        assert_eq!(
            text,
            // Indentation goes; what is between the first and last character
            // of the line is the file's own text.
            "3 matches in 2 files\n\n/ws/a.rs\n12: let x = 1;\n40: let y  =  x;\n\n/ws/b.rs\n3: use x;"
        );
        assert_eq!(
            grep(&[hit("/ws/a.rs", 1, "x")]),
            "1 match in 1 file\n\n/ws/a.rs\n1: x"
        );
        assert_eq!(grep(&[]), "No matches.");
    }

    #[test]
    fn a_minified_line_does_not_take_the_whole_result() {
        let text = grep(&[hit("/ws/app.min.js", 1, &"é".repeat(5_000))]);
        let row = text.lines().last().unwrap();
        assert_eq!(row.chars().count(), "1: ".len() + MAX_LINE_CHARS + 1);
        assert!(row.ends_with('…'));
    }

    #[test]
    fn a_symbol_row_is_its_place_its_kind_and_its_signature() {
        let results = [
            SearchResult {
                symbol_id: 9,
                name: "run_workflow".into(),
                kind: "function".into(),
                file_path: "/ws/src/session.rs".into(),
                start_line: 2_128,
                signature: Some("pub async fn run_workflow(\n    config: &AgentConfig,\n)".into()),
            },
            SearchResult {
                symbol_id: 10,
                name: "MAX".into(),
                kind: "const".into(),
                file_path: "/ws/src/budget.rs".into(),
                start_line: 7,
                signature: None,
            },
            SearchResult {
                symbol_id: 11,
                name: "Props".into(),
                kind: "interface".into(),
                file_path: "/ws/src/App.tsx".into(),
                start_line: 3,
                signature: Some("{ open: boolean }".into()),
            },
        ];
        assert_eq!(
            symbols(&results),
            "/ws/src/session.rs:2128 function pub async fn run_workflow( config: &AgentConfig, )\n\
             /ws/src/budget.rs:7 const MAX\n\
             /ws/src/App.tsx:3 interface Props: { open: boolean }"
        );
        assert_eq!(symbols(&[]), "No symbols found.");
    }

    // The index keeps the start of a declaration. A short name can be cut off
    // the end of it and still "appear" inside a longer word.
    #[test]
    fn a_name_is_given_when_the_signature_only_holds_it_inside_another_word() {
        assert_eq!(
            symbol_text("field", "id", Some("pub valid: bool, pub width: u32, pub")),
            "field id: pub valid: bool, pub width: u32, pub"
        );
        assert_eq!(
            symbol_text("field", "id", Some("pub id: u64")),
            "field pub id: u64"
        );
        assert_eq!(
            symbol_text("function", "run", Some("fn run_all()")),
            "function run: fn run_all()"
        );
        // An import is indexed under its punctuation.
        assert_eq!(
            symbol_text("import", "{", Some("import { a } from \"b\";")),
            "import import { a } from \"b\";"
        );
        assert_eq!(
            symbol_text("fn", "größe", Some("fn größe()")),
            "fn fn größe()"
        );
    }

    #[test]
    fn an_outline_names_the_file_once_and_gives_each_symbol_its_lines() {
        let text = outline(
            "/ws/src/app.rs",
            &[
                symbol("std", "import", Some("use std::fmt;"), (1, 1)),
                symbol("App", "struct", Some("pub struct App"), (3, 9)),
                symbol("run", "function", None, (11, 40)),
            ],
        );
        assert_eq!(
            text,
            "/ws/src/app.rs (3 symbols)\n\
             1 import use std::fmt;\n\
             3-9 struct pub struct App\n\
             11-40 function run"
        );
        assert_eq!(outline("/ws/x.rs", &[]), "/ws/x.rs (no symbols indexed)");
        // Nothing the database knows the row by reaches the model.
        assert!(!text.contains("162863") && !text.contains("fileId"));
    }

    fn semantic_hit(name: &str, snippet: Option<&str>) -> SemanticSearchResult {
        SemanticSearchResult {
            symbol_id: 1,
            name: name.into(),
            kind: "function".into(),
            file_path: "/ws/src/queue.rs".into(),
            start_line: 10,
            end_line: 14,
            signature: Some(format!("fn {name}()")),
            score: 0.78,
            match_type: "hybrid".into(),
            snippet: snippet.map(str::to_string),
        }
    }

    #[test]
    fn a_semantic_hit_carries_its_source_as_source_not_as_an_escaped_string() {
        let text = semantic(
            "hybrid",
            None,
            &[
                semantic_hit("drain", Some("fn drain() {\n    let s = \"x\";\n}\n")),
                semantic_hit("push", None),
            ],
        );
        assert_eq!(
            text,
            "mode: hybrid\n\
             /ws/src/queue.rs:10-14 function fn drain() [hybrid 0.78]\n\
             ```\nfn drain() {\n    let s = \"x\";\n}\n```\n\
             /ws/src/queue.rs:10-14 function fn push() [hybrid 0.78]"
        );
    }

    #[test]
    fn a_partial_ranking_says_so_on_the_first_line() {
        let text = semantic("lexical-only", Some("embedding model unavailable"), &[]);
        assert_eq!(
            text,
            "mode: lexical-only (embedding model unavailable)\nNo results."
        );
    }

    #[test]
    fn a_snippet_holding_a_fence_cannot_close_its_own() {
        let text = semantic(
            "hybrid",
            None,
            &[semantic_hit("doc", Some("/// ```\n/// run()\n/// ```"))],
        );
        assert!(text.contains("\n~~~~\n/// ```\n/// run()\n/// ```\n~~~~"));

        // A document that shows both kinds of fence.
        let both = "```\ncode\n```\n~~~~\nmore\n~~~~";
        let text = semantic("hybrid", None, &[semantic_hit("doc", Some(both))]);
        assert!(text.contains(&format!("\n~~~~~\n{both}\n~~~~~")), "{text}");
    }

    // The saving is the reason for the format: measured against what the same
    // rows cost as pretty-printed JSON.
    #[test]
    fn the_line_formats_cost_a_fraction_of_the_json_they_replace() {
        let entries: Vec<DirEntryInfo> = (0..40)
            .map(|i| DirEntryInfo {
                name: format!("module_{i}.rs"),
                path: format!("/Users/someone/project/src-tauri/src/agent/module_{i}.rs"),
                is_dir: false,
            })
            .collect();
        let json = serde_json::to_string_pretty(&entries).unwrap();
        let lines = list_dir("/Users/someone/project/src-tauri/src/agent", &entries);
        assert!(
            lines.len() * 4 < json.len(),
            "{} vs {}",
            lines.len(),
            json.len()
        );

        let outline_rows: Vec<SymbolRecord> = (0..40)
            .map(|i| {
                symbol(
                    &format!("handler_{i}"),
                    "function",
                    Some(&format!(
                        "pub fn handler_{i}(ctx: &Context) -> Result<(), String>"
                    )),
                    (i * 10, i * 10 + 8),
                )
            })
            .collect();
        let json = serde_json::to_string_pretty(&outline_rows).unwrap();
        let lines = outline("/ws/src/app.rs", &outline_rows);
        assert!(
            lines.len() * 3 < json.len(),
            "{} vs {}",
            lines.len(),
            json.len()
        );
    }
}
