//! The edit guardrail: after `edit_file` writes a file, parse it with the
//! tree-sitter grammar the indexer already carries, and when the edit made a
//! file that parsed stop parsing, say so in the tool result — with the line
//! and the text of the first error — so the model fixes it in the next round
//! instead of discovering it at the next build, or never.
//!
//! SWE-agent (Yang et al., 2024) guards its edit command with a linter and
//! measured the guard in its ablation: a model rarely notices a broken edit
//! from a bare "applied", and recovers well when told where it broke. No LSP is needed for this: the
//! grammars are in the binary for indexing (lean-harness Phase 6).
//!
//! The edit is never reverted — the model may be midway through a change that
//! takes two edits — and a file that did not parse before is not warned about:
//! the warning is for what this edit broke, not for what was already there
//! (a template, a file the grammar does not fully cover).

use crate::code_intel::parser;

/// A file larger than this is not parsed twice per edit.
const MAX_CHECKED_BYTES: usize = 2_000_000;
/// How much of the offending line the warning quotes.
const SNIPPET_CHARS: usize = 120;

/// The warning to append to an applied edit of `path` from `old` to `new`, or
/// `None` when there is no grammar for the file, it did not parse before, or
/// it still parses.
pub fn new_syntax_error(path: &str, old: &str, new: &str) -> Option<String> {
    if old.len().max(new.len()) > MAX_CHECKED_BYTES {
        return None;
    }
    let lang = parser::detect_language(path)?;
    let language = parser::get_language(lang).ok()?;
    let mut ts = tree_sitter::Parser::new();
    ts.set_language(&language).ok()?;
    if ts.parse(old, None)?.root_node().has_error() {
        return None;
    }
    let tree = ts.parse(new, None)?;
    let root = tree.root_node();
    if !root.has_error() {
        return None;
    }
    let row = first_error_row(root)?;
    let line = new.lines().nth(row).unwrap_or("").trim();
    let snippet: String = line.chars().take(SNIPPET_CHARS).collect();
    Some(format!(
        "Warning: after this edit {path} no longer parses as {lang} — first syntax error at \
         line {}: `{snippet}`. The edit was applied; fix it unless that is intended.",
        row + 1
    ))
}

/// The 0-based row of the first error or missing node in document order.
fn first_error_row(root: tree_sitter::Node) -> Option<usize> {
    let mut cursor = root.walk();
    let mut stack = vec![root];
    while let Some(node) = stack.pop() {
        if node.is_error() || node.is_missing() {
            return Some(node.start_position().row);
        }
        if !node.has_error() {
            continue;
        }
        let children: Vec<_> = node.children(&mut cursor).collect();
        stack.extend(children.into_iter().rev());
    }
    None
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn a_rust_edit_that_breaks_the_syntax_is_named_with_its_line() {
        let old = "fn main() {\n    let x = 1;\n    println!(\"{x}\");\n}\n";
        let new = "fn main() {\n    let x = (1;\n    println!(\"{x}\");\n}\n";
        let warning = new_syntax_error("/ws/src/main.rs", old, new).expect("a warning");
        assert!(warning.contains("syntax error"), "{warning}");
        assert!(warning.contains("line 2"), "{warning}");
        assert!(warning.contains("let x = (1;"), "{warning}");
        assert!(
            warning.contains("applied"),
            "the edit is never reverted: {warning}"
        );
    }

    #[test]
    fn a_javascript_edit_that_drops_a_brace_is_named() {
        let old = "function f(a) {\n  if (a) {\n    return 1;\n  }\n  return 2;\n}\n";
        let new = "function f(a) {\n  if (a) {\n    return 1;\n\n  return 2;\n}\n";
        let warning = new_syntax_error("/ws/app.js", old, new).expect("a warning");
        assert!(warning.contains("syntax error"), "{warning}");
    }

    #[test]
    fn an_edit_of_a_file_that_was_already_broken_says_nothing() {
        let old = "fn main( {\n    let x = 1;\n}\n";
        let new = "fn main( {\n    let x = 2;\n}\n";
        assert_eq!(new_syntax_error("/ws/src/main.rs", old, new), None);
    }

    #[test]
    fn a_clean_edit_or_a_file_with_no_grammar_says_nothing() {
        let old = "fn main() {\n    let x = 1;\n}\n";
        let new = "fn main() {\n    let x = 2;\n}\n";
        assert_eq!(new_syntax_error("/ws/src/main.rs", old, new), None);
        assert_eq!(new_syntax_error("/ws/notes.unknownext", "a", "(("), None);
    }

    // A new file has no old tree to compare with: an empty one parses.
    #[test]
    fn a_created_file_that_does_not_parse_is_named() {
        let warning = new_syntax_error("/ws/src/new.rs", "", "fn broken( {\n").expect("a warning");
        assert!(warning.contains("line 1"), "{warning}");
    }

    #[test]
    fn edit_file_applies_a_breaking_edit_and_appends_the_warning() {
        let dir = tempfile::tempdir().unwrap();
        let path = dir.path().join("lib.rs");
        std::fs::write(&path, "pub fn one() -> u32 {\n    1\n}\n").unwrap();
        let path = path.to_string_lossy().to_string();
        let diff = super::super::edit_file::preview(&super::super::edit_file::EditFileArgs {
            path: path.clone(),
            old_string: "    1\n}".into(),
            new_string: "    1\n".into(),
            replace_all: false,
        })
        .unwrap();
        let out = super::super::edit_file::apply(&diff).unwrap();
        assert!(out.starts_with(&format!("Applied edit to {path}")), "{out}");
        assert!(out.contains("syntax error"), "{out}");
        assert_eq!(
            std::fs::read_to_string(&path).unwrap(),
            "pub fn one() -> u32 {\n    1\n\n",
            "never reverted"
        );
        // A clean edit answers as it always did.
        let diff = super::super::edit_file::preview(&super::super::edit_file::EditFileArgs {
            path: path.clone(),
            old_string: "    1\n\n".into(),
            new_string: "    1\n}\n".into(),
            replace_all: false,
        })
        .unwrap();
        assert_eq!(
            super::super::edit_file::apply(&diff).unwrap(),
            format!("Applied edit to {path}")
        );
    }
}
