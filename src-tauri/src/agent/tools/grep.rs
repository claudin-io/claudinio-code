use serde::Deserialize;
use serde::Serialize;
use std::process::Command;

#[derive(Deserialize)]
pub struct GrepArgs {
    pub pattern: String,
    #[serde(alias = "file_path")]
    pub path: Option<String>,
}

#[derive(Serialize)]
#[serde(rename_all = "camelCase")]
pub struct GrepMatch {
    pub file: String,
    pub line: usize,
    pub content: String,
}

pub fn execute(args: GrepArgs) -> Result<Vec<GrepMatch>, String> {
    let mut cmd = Command::new("rg");
    cmd.arg("--line-number").arg("--no-heading")
        .arg("--with-filename").arg("--color").arg("never")
        .arg("-g").arg("!.git")
        .arg(&args.pattern);

    if let Some(ref path) = args.path {
        cmd.arg(path);
    }

    let output = cmd.output().map_err(|e| format!("rg failed: {e}"))?;
    if !output.status.success() && !output.stderr.is_empty() {
        let stderr = String::from_utf8_lossy(&output.stderr);
        if !stderr.trim().is_empty() {
            return Err(format!("rg error: {stderr}"));
        }
    }

    let mut results = Vec::new();
    let stdout = String::from_utf8_lossy(&output.stdout);
    for line in stdout.lines() {
        let parts: Vec<&str> = line.splitn(3, ':').collect();
        if parts.len() < 3 {
            continue;
        }
        results.push(GrepMatch {
            file: parts[0].to_string(),
            line: parts[1].parse().unwrap_or(0),
            content: parts[2].to_string(),
        });
    }

    Ok(results)
}
