use serde::Deserialize;

#[derive(Deserialize)]
pub struct ReadFileArgs {
    pub path: String,
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
    std::fs::read_to_string(p).map_err(|e| format!("cannot read as text: {e}"))
}
