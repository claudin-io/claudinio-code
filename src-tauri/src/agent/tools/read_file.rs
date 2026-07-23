use serde::Deserialize;
use std::sync::OnceLock;
use tiktoken_rs::{CoreBPE, cl100k_base};

const MAX_TOKENS: usize = 25000;

#[derive(Deserialize)]
pub struct ReadFileArgs {
    #[serde(alias = "file_path")]
    pub path: String,
    /// Optional 1-based start line (inclusive). If None, reads from line 1.
    pub start_line: Option<usize>,
    /// Optional 1-based end line (inclusive). If None, reads to the end.
    pub end_line: Option<usize>,
}

/// Lazy singleton tokenizer for counting tokens.
fn tokenizer() -> &'static CoreBPE {
    static TK: OnceLock<CoreBPE> = OnceLock::new();
    TK.get_or_init(|| cl100k_base().expect("failed to load cl100k_base tokenizer"))
}

/// Truncate content by token count if it exceeds max_tokens.
///
/// Returns (content_or_truncated, tokens_shown, lines_shown, total_tokens).
/// When no truncation is needed, the full content is returned unchanged.
fn truncate_by_tokens(content: &str, max_tokens: usize) -> (String, usize, usize, usize) {
    let bpe = tokenizer();
    let total_tokens = bpe.encode_with_special_tokens(content).len();
    let total_lines = content.lines().count();

    if total_tokens <= max_tokens || total_lines == 0 {
        return (content.to_string(), total_tokens, total_lines, total_tokens);
    }

    // Walk line by line accumulating token counts
    let mut accumulated = 0usize;
    let mut lines_shown = 0usize;

    for line in content.lines() {
        let line_tokens = bpe.encode_with_special_tokens(line).len();
        if accumulated + line_tokens > max_tokens {
            break;
        }
        accumulated += line_tokens;
        lines_shown += 1;
    }

    let truncated: String = content
        .lines()
        .take(lines_shown)
        .collect::<Vec<_>>()
        .join("\n");

    (truncated, accumulated, lines_shown, total_tokens)
}

/// Build the warning header for truncated content.
fn truncation_warning(
    tokens_shown: usize,
    total_tokens: usize,
    lines_shown: usize,
    total_lines: usize,
) -> String {
    format!(
        "⚠⚠⚠⚠⚠⚠⚠⚠⚠⚠⚠⚠⚠⚠⚠⚠⚠⚠⚠⚠⚠⚠⚠⚠⚠⚠⚠⚠⚠⚠⚠⚠⚠⚠⚠⚠⚠⚠⚠⚠⚠⚠⚠⚠⚠⚠⚠⚠⚠⚠⚠⚠\n\
         ⚠  FILE SIZE WARNING\n\
         ⚠  This file has {total_lines} lines (~{total_tokens} tokens total).\n\
         ⚠  Only the first {lines_shown} lines (~{tokens_shown} tokens) are shown\n\
         ⚠  to protect the model's context window.\n\
         ⚠  Use start_line/end_line to read specific sections.\n\
         ⚠⚠⚠⚠⚠⚠⚠⚠⚠⚠⚠⚠⚠⚠⚠⚠⚠⚠⚠⚠⚠⚠⚠⚠⚠⚠⚠⚠⚠⚠⚠⚠⚠⚠⚠⚠⚠⚠⚠⚠⚠⚠⚠⚠⚠⚠⚠⚠⚠⚠⚠⚠\n\n"
    )
}

pub fn execute(args: ReadFileArgs) -> Result<String, String> {
    let p = std::path::Path::new(&args.path);
    let meta = p.metadata().map_err(|e| format!("cannot access: {e}"))?;
    if !meta.is_file() {
        return Err(format!("not a file: {}", args.path));
    }
    if meta.len() > 2 * 1024 * 1024 {
        return Err("file too large (>2MB)".into());
    }
    let content = std::fs::read_to_string(p).map_err(|e| format!("cannot read as text: {e}"))?;

    match (args.start_line, args.end_line) {
        (Some(s), Some(e)) => {
            // User provided an explicit range — no truncation
            if s < 1 {
                return Err("start_line must be >= 1 (1-based)".into());
            }
            if e < s {
                return Err("end_line must be >= start_line".into());
            }
            let lines: Vec<&str> = content.lines().collect();
            if s > lines.len() {
                return Err(format!(
                    "start_line ({s}) exceeds file length ({})",
                    lines.len()
                ));
            }
            let end = e.min(lines.len());
            Ok(lines[s - 1..end].join("\n"))
        }
        _ => {
            // Full file read — check token budget
            let total_lines = content.lines().count();
            let (truncated, tokens_shown, lines_shown, total_tokens) =
                truncate_by_tokens(&content, MAX_TOKENS);

            if lines_shown < total_lines {
                let warning =
                    truncation_warning(tokens_shown, total_tokens, lines_shown, total_lines);
                Ok(format!("{warning}{truncated}"))
            } else {
                Ok(truncated)
            }
        }
    }
}
