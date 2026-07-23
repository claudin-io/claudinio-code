use serde::Serialize;
use std::path::Path;
use tokio::process::Command;

use crate::procutil::no_window_tokio;

#[derive(Serialize)]
#[serde(rename_all = "camelCase")]
pub struct ChangedFile {
    pub path: String,
    pub status: String,
    pub additions: u32,
    pub deletions: u32,
}

#[derive(Serialize)]
#[serde(rename_all = "camelCase")]
pub struct GitStatus {
    pub has_changes: bool,
    pub files: Vec<ChangedFile>,
    pub total_additions: u32,
    pub total_deletions: u32,
}

async fn run_git(workspace: &str, args: &[&str]) -> Result<String, String> {
    let mut cmd = Command::new("git");
    cmd.arg("-C").arg(workspace).args(args);
    no_window_tokio(&mut cmd);
    let output = cmd
        .output()
        .await
        .map_err(|e| format!("Failed to run git: {e}"))?;
    if !output.status.success() {
        let stderr = String::from_utf8_lossy(&output.stderr);
        return Err(format!("git error: {stderr}"));
    }
    Ok(String::from_utf8_lossy(&output.stdout).to_string())
}

#[tauri::command]
pub async fn git_status(workspace: String) -> Result<GitStatus, String> {
    // Get porcelain status
    let porcelain = run_git(&workspace, &["status", "--porcelain"]).await?;

    if porcelain.is_empty() {
        return Ok(GitStatus {
            has_changes: false,
            files: Vec::new(),
            total_additions: 0,
            total_deletions: 0,
        });
    }

    // Get numstat for tracked modified/added/deleted files, and for staged
    // files, concurrently.
    let (numstat, staged_numstat) = tokio::join!(
        run_git(&workspace, &["diff", "--numstat"]),
        run_git(&workspace, &["diff", "--numstat", "--cached"]),
    );
    let numstat = numstat.unwrap_or_default();
    let staged_numstat = staged_numstat.unwrap_or_default();

    let mut numstat_map: std::collections::HashMap<String, (u32, u32)> =
        std::collections::HashMap::new();
    for line in numstat.lines() {
        let parts: Vec<&str> = line.split('\t').collect();
        if parts.len() >= 3 {
            let add = parts[0].parse::<u32>().unwrap_or(0);
            let del = parts[1].parse::<u32>().unwrap_or(0);
            numstat_map.insert(parts[2].to_string(), (add, del));
        }
    }

    for line in staged_numstat.lines() {
        let parts: Vec<&str> = line.split('\t').collect();
        if parts.len() >= 3 {
            let add = parts[0].parse::<u32>().unwrap_or(0);
            let del = parts[1].parse::<u32>().unwrap_or(0);
            let entry = numstat_map.entry(parts[2].to_string()).or_insert((0, 0));
            entry.0 += add;
            entry.1 += del;
        }
    }

    let mut files = Vec::new();
    let mut total_additions = 0u32;
    let mut total_deletions = 0u32;

    for line in porcelain.lines() {
        if line.len() < 3 {
            continue;
        }
        let status_chars = &line[..2];
        let path_str = line[3..].trim().to_string();

        // Determine actual file path (handle "R" with "->")
        let file_path = if status_chars.starts_with('R') {
            path_str
                .split(" -> ")
                .last()
                .unwrap_or(&path_str)
                .to_string()
        } else {
            path_str
        };

        let status = match &status_chars[..2] {
            "M " | " M" | "MM" => "M",
            "A " | " A" | "AM" => "A",
            "D " | " D" => "D",
            "??" => "?",
            "R " | " R" => "R",
            _ => "M",
        };

        let (add, del) = numstat_map.get(&file_path).copied().unwrap_or((0, 0));

        // For untracked files, estimate from file content
        let (add, del) = if status == "?" && add == 0 && del == 0 {
            let full_path = Path::new(&workspace).join(&file_path);
            match tokio::fs::read_to_string(&full_path).await {
                Ok(content) => {
                    let lines = content.lines().count() as u32;
                    (lines, 0u32)
                }
                Err(_) => (0, 0),
            }
        } else {
            (add, del)
        };

        total_additions += add;
        total_deletions += del;

        files.push(ChangedFile {
            path: file_path,
            status: status.to_string(),
            additions: add,
            deletions: del,
        });
    }

    Ok(GitStatus {
        has_changes: true,
        files,
        total_additions,
        total_deletions,
    })
}

#[tauri::command]
pub async fn git_file_diff(workspace: String, path: String) -> Result<String, String> {
    // Try unstaged diff first
    let unstaged = run_git(&workspace, &["diff", "--", &path])
        .await
        .unwrap_or_default();
    if !unstaged.is_empty() {
        return Ok(unstaged);
    }

    // Try staged diff
    let staged = run_git(&workspace, &["diff", "--cached", "--", &path])
        .await
        .unwrap_or_default();
    if !staged.is_empty() {
        return Ok(staged);
    }

    // Check if the file exists on disk (untracked or new)
    let full_path = Path::new(&workspace).join(&path);
    if full_path.exists() {
        // Read the file and format as a unified diff (all lines added)
        match tokio::fs::read_to_string(&full_path).await {
            Ok(content) => {
                let lines: Vec<&str> = content.lines().collect();
                let line_count = lines.len();
                let header = format!(
                    "diff --git a/{path} b/{path}\nnew file mode 100644\nindex 0000000..0000000\n--- /dev/null\n+++ b/{path}\n@@ -0,0 +1,{line_count} @@\n"
                );
                let body = lines
                    .iter()
                    .map(|l| format!("+{l}"))
                    .collect::<Vec<_>>()
                    .join("\n");
                return Ok(format!("{header}{body}"));
            }
            Err(_) => {
                return Ok(String::new());
            }
        }
    }

    // File was deleted — show the full deletion from HEAD
    let show_result = run_git(&workspace, &["show", &format!("HEAD:{}", path)])
        .await
        .unwrap_or_default();
    if !show_result.is_empty() {
        let old_lines: Vec<&str> = show_result.lines().collect();
        let line_count = old_lines.len();
        let header = format!(
            "diff --git a/{path} b/{path}\ndeleted file mode 100644\nindex 0000000..0000000\n--- a/{path}\n+++ /dev/null\n@@ -1,{line_count} +0,0 @@\n"
        );
        let body = old_lines
            .iter()
            .map(|l| format!("-{l}"))
            .collect::<Vec<_>>()
            .join("\n");
        return Ok(format!("{header}{body}"));
    }

    Ok(String::new())
}

#[tauri::command]
pub async fn git_branch(workspace: String) -> Result<String, String> {
    let branch = run_git(&workspace, &["rev-parse", "--abbrev-ref", "HEAD"]).await?;
    Ok(branch.trim().to_string())
}

#[tauri::command]
pub async fn check_git_available() -> bool {
    let mut cmd = Command::new("git");
    cmd.arg("--version");
    no_window_tokio(&mut cmd);
    cmd.output()
        .await
        .map(|o| o.status.success())
        .unwrap_or(false)
}
