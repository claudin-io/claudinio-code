use crate::procutil;
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
    // Inject the user's login PATH so `rg` (typically under /opt/homebrew/bin,
    // ~/.cargo/bin, etc.) resolves even when the app is launched from Finder
    // with a minimal GUI PATH. Without this the dedicated grep tool fails with
    // "rg failed: No such file or directory" and the agent is forced to fall
    // back to bash-grep, defeating the smart-tool ordering.
    cmd.env("PATH", crate::agent::tools::bash::login_path());
    cmd.arg("--line-number").arg("--no-heading")
        .arg("--with-filename").arg("--color").arg("never")
        .arg("-g").arg("!.git")
        .arg(&args.pattern);

    if let Some(ref path) = args.path {
        cmd.arg(path);
    }

    procutil::no_window(&mut cmd);

    let output = cmd.output().map_err(|e| {
        if e.kind() == std::io::ErrorKind::NotFound {
            "grep tool needs ripgrep (rg), which was not found on PATH. Install it \
             (macOS: `brew install ripgrep`) or use the bash tool with grep instead."
                .to_string()
        } else {
            format!("rg failed: {e}")
        }
    })?;
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

#[cfg(test)]
mod tests {
    use super::*;

    /// Proves the grep tool actually runs `rg` — with the login PATH injected,
    /// `rg` resolves even from a minimal (GUI-style) process PATH. If this were
    /// still `Command::new("rg")` with no PATH, a Finder-launched app would get
    /// "No such file or directory" here. Also guards the parsing of matches.
    #[test]
    fn grep_finds_a_match_via_login_path() {
        let dir = std::env::temp_dir().join(format!("claudinio_grep_{}", std::process::id()));
        std::fs::create_dir_all(&dir).unwrap();
        let file = dir.join("sample.txt");
        std::fs::write(&file, "alpha\nNEEDLE here\nbeta\n").unwrap();

        let out = execute(GrepArgs {
            pattern: "NEEDLE".into(),
            path: Some(dir.to_string_lossy().to_string()),
        });
        let _ = std::fs::remove_dir_all(&dir);

        let matches = out.expect("rg must run and return Ok (login PATH resolves rg)");
        assert_eq!(matches.len(), 1, "expected exactly one match");
        assert_eq!(matches[0].line, 2);
        assert!(matches[0].content.contains("NEEDLE"));
    }
}
