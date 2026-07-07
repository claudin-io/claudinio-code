use serde::Deserialize;

#[derive(Deserialize)]
pub struct ReadFileArgs {
    pub path: String,
    /// Optional 1-based start line (inclusive). If None, reads from line 1.
    pub start_line: Option<usize>,
    /// Optional 1-based end line (inclusive). If None, reads to the end.
    pub end_line: Option<usize>,
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
        _ => Ok(content),
    }
}
