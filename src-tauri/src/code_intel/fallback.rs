use crate::code_intel::parser::ParsedSymbol;
use std::path::Path;

#[derive(Clone)]
struct Chunk {
    name: String,
    kind: String,
    start_line: usize,
    end_line: usize,
}

// ---------------------------------------------------------------------------
// Kotlin
// ---------------------------------------------------------------------------
fn scan_kotlin(content: &str) -> Vec<Chunk> {
    let kws = [
        "private suspend fun ", "suspend fun ",
        "private fun ", "public fun ", "internal fun ", "protected fun ", "fun ",
        "private data class ", "data class ",
        "private sealed class ", "sealed class ",
        "private abstract class ", "abstract class ",
        "private class ", "public class ", "internal class ", "class ",
        "private open class ", "open class ",
        "private object ", "public object ", "internal object ", "object ",
        "private companion object ", "companion object ",
        "private interface ", "public interface ", "internal interface ", "interface ",
        "private enum ", "public enum ", "internal enum ", "enum ",
        "typealias ", "private val ", "val ", "private var ", "var ",
    ];
    scan_brace_keywords(content, &kws)
}

// ---------------------------------------------------------------------------
// Gleam
// ---------------------------------------------------------------------------
fn scan_gleam(content: &str) -> Vec<Chunk> {
    let kws = [
        "pub fn ", "fn ", "pub type ", "type ",
        "pub opaque type ", "opaque type ",
        "pub const ", "const ", "pub enum ", "enum ",
        "import ",
    ];
    scan_brace_keywords(content, &kws)
}

// ---------------------------------------------------------------------------
// Fish — function ... end (keyword pair, not brace-delimited)
// ---------------------------------------------------------------------------
fn scan_fish(content: &str) -> Vec<Chunk> {
    let lines: Vec<&str> = content.lines().collect();
    let mut chunks = Vec::new();
    let mut i = 0;
    while i < lines.len() {
        let t = lines[i].trim();
        if t.starts_with("function ") {
            let name = t.strip_prefix("function ").unwrap_or("")
                .split_whitespace().next().unwrap_or("?").to_string();
            let end = find_keyword_pair_end(&lines, i, "function", "end");
            chunks.push(Chunk { name, kind: "function".into(), start_line: i + 1, end_line: end + 1 });
            i = end + 1;
            continue;
        }
        i += 1;
    }
    chunks
}

// ---------------------------------------------------------------------------
// SCSS
// ---------------------------------------------------------------------------
fn scan_scss(content: &str) -> Vec<Chunk> {
    let mut a = scan_brace_keywords(content, &["@mixin ", "@function "]);
    let b = scan_css_rules(content);
    a.extend(b);
    a
}

// ---------------------------------------------------------------------------
// Less — same as SCSS
// ---------------------------------------------------------------------------
fn scan_less(content: &str) -> Vec<Chunk> {
    scan_scss(content)
}

fn scan_css_rules(content: &str) -> Vec<Chunk> {
    let lines: Vec<&str> = content.lines().collect();
    let mut chunks = Vec::new();
    let mut i = 0;
    while i < lines.len() {
        let t = lines[i].trim();
        if let Some(start_ch) = t.chars().next() {
            // .class, #id, @media, %placeholder
            let is_rule = start_ch == '.' || start_ch == '#' || start_ch == '@' || start_ch == '%';
            if is_rule {
                let name = t.split(&[' ', '{', ':', '['][..]).next().unwrap_or("?").trim_end_matches('{').trim_end_matches(':').to_string();
                if name.len() > 1 {
                    let end = if t.contains('{') {
                        let depth = t.matches('{').count().saturating_sub(t.matches('}').count());
                        if depth == 0 { i } else { find_brace_end(&lines, i, depth) }
                    } else if lines.get(i + 1).map(|l| l.trim() == "{").unwrap_or(false) {
                        find_brace_end(&lines, i + 1, 1)
                    } else { i };
                    chunks.push(Chunk { name, kind: "css-rule".into(), start_line: i + 1, end_line: end + 1 });
                    i = end + 1;
                    continue;
                }
            }
        }
        i += 1;
    }
    chunks
}

// ---------------------------------------------------------------------------
// DOT (Graphviz)
// ---------------------------------------------------------------------------
fn scan_dot(content: &str) -> Vec<Chunk> {
    let kws = ["digraph ", "graph ", "subgraph "];
    let mut a = scan_brace_keywords(content, &kws);
    // Also catch top-level node/edge attribute blocks
    let b = scan_brace_keywords(content, &["node ", "edge "]);
    a.extend(b);
    a
}

// ---------------------------------------------------------------------------
// Org mode — heading-based
// ---------------------------------------------------------------------------
fn scan_org(content: &str) -> Vec<Chunk> {
    let lines: Vec<&str> = content.lines().collect();
    let mut chunks = Vec::new();
    let mut i = 0;
    while i < lines.len() {
        let raw = lines[i];
        let ws = raw.len() - raw.trim_start().len();
        let t = &raw[ws..];
        if t.starts_with('*') {
            let level = t.chars().take_while(|c| *c == '*').count();
            let name = t[level..].trim().to_string();
            if !name.is_empty() {
                let start = i;
                let end = lines[start + 1..].iter().position(|l| {
                    let tw = l.len() - l.trim_start().len();
                    let lt = &l[tw..];
                    lt.starts_with('*') && lt.chars().take_while(|c| *c == '*').count() <= level
                }).map(|p| start + p).unwrap_or(lines.len() - 1);
                chunks.push(Chunk { name, kind: format!("h{}", level), start_line: start + 1, end_line: end + 1 });
                i = end + 1;
                continue;
            }
        }
        i += 1;
    }
    chunks
}

// ---------------------------------------------------------------------------
// Generic brace scanner
// ---------------------------------------------------------------------------
fn scan_brace_keywords(content: &str, keywords: &[&str]) -> Vec<Chunk> {
    let lines: Vec<&str> = content.lines().collect();
    let mut chunks = Vec::new();
    let mut i = 0;
    while i < lines.len() {
        let t = lines[i].trim();
        if let Some((name, kw)) = parse_decl_keyword(t, keywords) {
            let start = i;
            let has_brace = t.contains('{');
            let brace_next = !has_brace && lines.get(i + 1).map(|l| l.trim() == "{").unwrap_or(false);
            let end = if has_brace {
                let d = t.matches('{').count().saturating_sub(t.matches('}').count());
                if d == 0 { i } else { find_brace_end(&lines, i, d) }
            } else if brace_next {
                find_brace_end(&lines, i + 1, 1)
            } else if lines.get(i + 1).map(|l| l.starts_with(' ')).unwrap_or(false) {
                find_indent_end(&lines, i)
            } else {
                i
            };
            let kind = kw.split_whitespace().next().unwrap_or("declaration").trim_end().to_string();
            chunks.push(Chunk { name, kind, start_line: start + 1, end_line: end + 1 });
            i = end + 1;
            continue;
        }
        i += 1;
    }
    chunks
}

fn find_brace_end(lines: &[&str], start: usize, initial: usize) -> usize {
    let mut depth = initial;
    for i in start + 1..lines.len() {
        depth += lines[i].matches('{').count();
        depth = depth.saturating_sub(lines[i].matches('}').count());
        if depth == 0 { return i; }
    }
    lines.len() - 1
}

fn find_indent_end(lines: &[&str], start: usize) -> usize {
    let base = lines[start].len() - lines[start].trim_start().len();
    let mut end = start;
    for i in start + 1..lines.len() {
        let l = lines[i];
        if l.trim().is_empty() { continue; }
        if l.len() - l.trim_start().len() <= base && !l.trim().starts_with('.') { break; }
        end = i;
    }
    end
}

fn find_keyword_pair_end(lines: &[&str], start: usize, start_kw: &str, end_kw: &str) -> usize {
    let mut depth = 0usize;
    for i in start..lines.len() {
        let t = lines[i].trim();
        if t.starts_with(start_kw) && i > start { depth += 1; }
        else if t == end_kw || t.starts_with(end_kw) {
            if depth == 0 { return i; }
            depth -= 1;
        }
    }
    lines.len() - 1
}

fn parse_decl_keyword<'a>(line: &'a str, keywords: &[&'a str]) -> Option<(String, &'a str)> {
    for kw in keywords {
        if let Some(after) = line.strip_prefix(kw) {
            if !after.is_empty() && after.starts_with(|c: char| c.is_alphanumeric()) {
                continue;
            }
            let name = after.split(&[' ', '(', '{', ':', '<', '=', '\n', '\t'][..])
                .next().unwrap_or("?")
                .trim_end_matches('(').trim_end_matches('{').to_string();
            return Some((name, kw));
        }
    }
    None
}

// ---------------------------------------------------------------------------
// Public API
// ---------------------------------------------------------------------------

pub fn chunk_fallback(lang: &str, path: &str, content: &str) -> Vec<ParsedSymbol> {
    let chunks = match lang {
        "kotlin" => scan_kotlin(content),
        "gleam"  => scan_gleam(content),
        "fish"   => scan_fish(content),
        "scss"   => scan_scss(content),
        "less"   => scan_less(content),
        "dot"    => scan_dot(content),
        "org"    => scan_org(content),
        _ => vec![],
    };

    let lines: Vec<&str> = content.lines().collect();
    let file_name = Path::new(path).file_name().and_then(|n| n.to_str()).unwrap_or("?").to_string();

    if chunks.is_empty() {
        let lc = lines.len() as i64;
        return vec![ParsedSymbol {
            name: file_name,
            kind: "file".into(),
            parent_context: None,
            signature: None,
            doc_comment: None,
            body_text: Some(content.to_string()),
            start_line: 1, start_col: 1, end_line: lc.max(1), end_col: 1,
        }];
    }

    let mut result = Vec::with_capacity(chunks.len());
    for c in &chunks {
        if c.start_line > c.end_line { continue; }
        let body = if c.start_line == c.end_line {
            None
        } else {
            let s = (c.start_line.saturating_sub(1)).min(lines.len());
            let e = c.end_line.min(lines.len());
            if s < e { Some(lines[s..e].join("\n")) } else { None }
        };
        let parent_ctx = find_parent_ctx(&chunks, c, &file_name);
        let s_col = lines.get(c.start_line.saturating_sub(1)).map(|l| (l.len() - l.trim_start().len() + 1) as i64).unwrap_or(1);
        let e_col = lines.get(c.end_line.saturating_sub(1)).map(|l| l.len() as i64 + 1).unwrap_or(1);
        result.push(ParsedSymbol {
            name: c.name.clone(),
            kind: c.kind.clone(),
            parent_context: parent_ctx,
            signature: None,
            doc_comment: None,
            body_text: body,
            start_line: c.start_line as i64,
            start_col: s_col,
            end_line: c.end_line as i64,
            end_col: e_col,
        });
    }
    result
}

fn find_parent_ctx(chunks: &[Chunk], child: &Chunk, _file_name: &str) -> Option<String> {
    for p in chunks {
        if p.start_line < child.start_line && p.end_line >= child.end_line {
            return Some(format!("{}:{}", p.kind, p.name));
        }
    }
    None
}
