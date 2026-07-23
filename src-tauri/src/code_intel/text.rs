//! Identifier-splitting helpers shared by the embedding text builder and the
//! chunk FTS index, so both retrieval legs see the same word forms.

/// Cap on the total split-word text appended per chunk to the FTS body — a
/// chunk is ~800 chars of code, so unbounded splitting could double it.
pub const FTS_BODY_SPLIT_CAP_CHARS: usize = 400;

/// Split an identifier into lowercase words. Handles camelCase, PascalCase,
/// snake_case, kebab-case, dotted.keys, digit boundaries and acronym runs
/// (`HTTPServer` -> ["http", "server"], `v2Handler` -> ["v2", "handler"]).
/// Words shorter than 2 chars are dropped; order preserved; deduped.
pub fn split_identifier_words(identifier: &str) -> Vec<String> {
    let mut words: Vec<String> = Vec::new();
    for run in identifier.split(|c: char| !c.is_alphanumeric()) {
        if run.is_empty() {
            continue;
        }
        for word in split_camel_run(run) {
            let lower = word.to_lowercase();
            if lower.chars().count() >= 2 && !words.contains(&lower) {
                words.push(lower);
            }
        }
    }
    words
}

/// Split one alphanumeric run at camel boundaries. A boundary sits before an
/// uppercase char that follows a lowercase char or digit (`getUser`, `v2H`),
/// and before the last uppercase of an acronym run followed by lowercase
/// (`HTTPServer` -> HTTP|Server). Trailing digits stay attached (`base64`,
/// `utf8`) — only case changes cut, so all-lower/all-upper runs pass through.
fn split_camel_run(run: &str) -> Vec<&str> {
    let chars: Vec<char> = run.chars().collect();
    let mut parts: Vec<&str> = Vec::new();
    let mut start_byte = 0usize;
    let mut byte_pos = 0usize;
    for i in 0..chars.len() {
        let boundary = i > 0
            && chars[i].is_uppercase()
            && (!chars[i - 1].is_uppercase()
                || (i + 1 < chars.len() && chars[i + 1].is_lowercase()));
        if boundary && byte_pos > start_byte {
            parts.push(&run[start_byte..byte_pos]);
            start_byte = byte_pos;
        }
        byte_pos += chars[i].len_utf8();
    }
    if start_byte < run.len() {
        parts.push(&run[start_byte..]);
    }
    parts
}

/// " "-joined split words of an identifier, or `None` when splitting adds
/// nothing beyond the lowercased identifier itself (single-word names).
pub fn identifier_word_string(identifier: &str) -> Option<String> {
    let words = split_identifier_words(identifier);
    match words.as_slice() {
        [] => None,
        [only] if *only == identifier.to_lowercase() => None,
        _ => Some(words.join(" ")),
    }
}

/// Distinct camelCase/PascalCase identifiers found in a code slice, split
/// into words and joined, capped at `max_chars` total. Snake/kebab/dotted
/// identifiers are excluded — the FTS `unicode61` tokenizer already splits
/// those at `_`/`-`/`.`, so only case-joined words need explicit help.
pub fn body_split_words(slice: &str, max_chars: usize) -> String {
    let mut out = String::new();
    let mut seen: Vec<String> = Vec::new();
    for run in slice.split(|c: char| !c.is_alphanumeric()) {
        if !has_camel_boundary(run) {
            continue;
        }
        for word in split_identifier_words(run) {
            if seen.contains(&word) {
                continue;
            }
            let added = word.len() + if out.is_empty() { 0 } else { 1 };
            if out.len() + added > max_chars {
                return out;
            }
            if !out.is_empty() {
                out.push(' ');
            }
            out.push_str(&word);
            seen.push(word);
        }
    }
    out
}

/// True when a run mixes cases in a way that produces more than one word —
/// i.e. it contains an uppercase char after position 0 next to a lowercase
/// or digit neighbour. All-lower, all-upper and single-word runs are false.
fn has_camel_boundary(run: &str) -> bool {
    let chars: Vec<char> = run.chars().collect();
    for i in 1..chars.len() {
        if chars[i].is_uppercase()
            && (!chars[i - 1].is_uppercase()
                || (i + 1 < chars.len() && chars[i + 1].is_lowercase()))
        {
            return true;
        }
    }
    false
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn split_identifier_words_camel_snake_kebab_acronym_digits() {
        assert_eq!(
            split_identifier_words("getUserSettings"),
            vec!["get", "user", "settings"]
        );
        assert_eq!(split_identifier_words("HTTPServer"), vec!["http", "server"]);
        assert_eq!(split_identifier_words("v2Handler"), vec!["v2", "handler"]);
        assert_eq!(
            split_identifier_words("delete_symbols_for_file"),
            vec!["delete", "symbols", "for", "file"]
        );
        assert_eq!(
            split_identifier_words("kebab-case-name"),
            vec!["kebab", "case", "name"]
        );
        assert_eq!(
            split_identifier_words("dotted.key.path"),
            vec!["dotted", "key", "path"]
        );
        assert_eq!(
            split_identifier_words("base64Encode"),
            vec!["base64", "encode"]
        );
        assert_eq!(split_identifier_words("parseJSON"), vec!["parse", "json"]);
        // 1-char words dropped, duplicates removed.
        assert_eq!(split_identifier_words("x_y_data_data"), vec!["data"]);
    }

    #[test]
    fn identifier_word_string_none_for_single_word() {
        assert_eq!(identifier_word_string("Icon"), None);
        assert_eq!(identifier_word_string("db"), None);
        assert_eq!(identifier_word_string(""), None);
        assert_eq!(identifier_word_string("_"), None);
        assert_eq!(
            identifier_word_string("buildEmbeddingChunks").as_deref(),
            Some("build embedding chunks")
        );
        assert_eq!(
            identifier_word_string("delete_symbols_for_file").as_deref(),
            Some("delete symbols for file")
        );
    }

    #[test]
    fn body_split_words_dedupes_and_caps() {
        let slice = "let userSettings = getUserSettings(); applyUserSettings(userSettings);";
        // Words appear once each despite repeated identifiers; snake_case-only
        // and single-word identifiers contribute nothing.
        assert_eq!(body_split_words(slice, 400), "user settings get apply");
        assert_eq!(body_split_words("snake_case_only plain", 400), "");
        // Cap cuts cleanly at a word boundary.
        assert_eq!(body_split_words(slice, 13), "user settings");
    }
}
