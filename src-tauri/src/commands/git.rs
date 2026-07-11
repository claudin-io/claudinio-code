use serde::Serialize;
use std::path::Path;
use std::process::Command;

#[derive(Serialize)]
#[serde(rename_all = "camelCase")]
pub struct ChangedFile {
    pub path: String,
    pub status: String,
    pub additions: u32,
    pub deletions: u32,
    #[serde(default)]
    pub staged: bool,
}

#[derive(Serialize)]
#[serde(rename_all = "camelCase")]
pub struct GitStatus {
    pub has_changes: bool,
    pub files: Vec<ChangedFile>,
    pub total_additions: u32,
    pub total_deletions: u32,
}

fn run_git(workspace: &str, args: &[&str]) -> Result<String, String> {
    let output = Command::new("git")
        .arg("-C")
        .arg(workspace)
        .args(args)
        .output()
        .map_err(|e| format!("Failed to run git: {e}"))?;
    if !output.status.success() {
        let stderr = String::from_utf8_lossy(&output.stderr);
        return Err(format!("git error: {stderr}"));
    }
    Ok(String::from_utf8_lossy(&output.stdout).to_string())
}

#[tauri::command]
pub fn git_status(workspace: String) -> Result<GitStatus, String> {
    // Get porcelain status
    let porcelain = run_git(&workspace, &["status", "--porcelain"])?;
    
    if porcelain.is_empty() {
        return Ok(GitStatus {
            has_changes: false,
            files: Vec::new(),
            total_additions: 0,
            total_deletions: 0,
        });
    }

    // Get numstat for tracked modified/added/deleted files
    let numstat = run_git(&workspace, &["diff", "--numstat"]).unwrap_or_default();
    let mut numstat_map: std::collections::HashMap<String, (u32, u32)> = std::collections::HashMap::new();
    for line in numstat.lines() {
        let parts: Vec<&str> = line.split('\t').collect();
        if parts.len() >= 3 {
            let add = parts[0].parse::<u32>().unwrap_or(0);
            let del = parts[1].parse::<u32>().unwrap_or(0);
            numstat_map.insert(parts[2].to_string(), (add, del));
        }
    }

    // Get numstat for staged files
    let staged_numstat = run_git(&workspace, &["diff", "--numstat", "--cached"]).unwrap_or_default();
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
            path_str.split(" -> ").last().unwrap_or(&path_str).to_string()
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
            if let Ok(content) = std::fs::read_to_string(&full_path) {
                let lines = content.lines().count() as u32;
                (lines, 0u32)
            } else {
                (0, 0)
            }
        } else {
            (add, del)
        };

        total_additions += add;
        total_deletions += del;

        files.push(ChangedFile {
            path: file_path.clone(),
            status: status.to_string(),
            additions: add,
            deletions: del,
            staged: false,
        });
    }

    // Get staged file paths from --cached diff
    let staged_output = run_git(&workspace, &["diff", "--cached", "--name-only"]).unwrap_or_default();
    let staged_paths: std::collections::HashSet<String> = staged_output
        .lines()
        .map(|l| l.trim().to_string())
        .collect();

    for file in &mut files {
        if staged_paths.contains(&file.path) {
            file.staged = true;
        }
    }

    Ok(GitStatus {
        has_changes: true,
        files,
        total_additions,
        total_deletions,
    })
}

#[tauri::command]
pub fn git_file_diff(workspace: String, path: String, staged: Option<bool>) -> Result<String, String> {
    // If staged is true, return the cached diff only
    if staged.unwrap_or(false) {
        let cached = run_git(&workspace, &["diff", "--cached", "--", &path]).unwrap_or_default();
        if !cached.is_empty() {
            return Ok(cached);
        }
        // If nothing staged, fall through to the fallback logic below
    }

    // Try unstaged diff first
    let unstaged = run_git(&workspace, &["diff", "--", &path]).unwrap_or_default();
    if !unstaged.is_empty() {
        return Ok(unstaged);
    }

    // Try staged diff
    let staged = run_git(&workspace, &["diff", "--cached", "--", &path]).unwrap_or_default();
    if !staged.is_empty() {
        return Ok(staged);
    }

    // Check if the file exists on disk (untracked or new)
    let full_path = Path::new(&workspace).join(&path);
    if full_path.exists() {
        // Read the file and format as a unified diff (all lines added)
        match std::fs::read_to_string(&full_path) {
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
    let show_result = run_git(&workspace, &["show", &format!("HEAD:{}", path)]).unwrap_or_default();
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
pub fn git_branch(workspace: String) -> Result<String, String> {
    let branch = run_git(&workspace, &["rev-parse", "--abbrev-ref", "HEAD"])?;
    Ok(branch.trim().to_string())
}

#[tauri::command]
pub fn git_stage_file(workspace: String, path: String) -> Result<(), String> {
    run_git(&workspace, &["add", "--", &path])?;
    Ok(())
}

#[tauri::command]
pub fn git_unstage_file(workspace: String, path: String) -> Result<(), String> {
    run_git(&workspace, &["reset", "HEAD", "--", &path])?;
    Ok(())
}

#[tauri::command]
pub fn git_discard_file(workspace: String, path: String) -> Result<(), String> {
    // Check if file is staged. If so, run git reset HEAD -- <path> first.
    let staged = run_git(&workspace, &["diff", "--cached", "--name-only", "--", &path]).unwrap_or_default();
    if !staged.trim().is_empty() {
        run_git(&workspace, &["reset", "HEAD", "--", &path])?;
    }
    // Also discard working tree changes (git checkout) OR delete if untracked
    let porcelain = run_git(&workspace, &["status", "--porcelain", "--", &path]).unwrap_or_default();
    if porcelain.starts_with("??") {
        // Untracked file — delete from disk
        let full_path = std::path::Path::new(&workspace).join(&path);
        let _ = std::fs::remove_file(&full_path);
    } else {
        let _ = run_git(&workspace, &["checkout", "--", &path]);
    }
    Ok(())
}

#[tauri::command]
pub fn git_stage_hunk(workspace: String, path: String, hunk_text: String) -> Result<(), String> {
    // Build a proper patch and apply to index
    // Get the diff header from git diff
    let header = run_git(&workspace, &["diff", "--", &path]).unwrap_or_default();
    let diff_header: String = header.lines()
        .take_while(|l| !l.starts_with("@@"))
        .chain(std::iter::once(""))
        .collect::<Vec<_>>()
        .join("\n");
    
    let patch = format!("{}\n{}", diff_header, hunk_text);
    
    let temp = std::env::temp_dir().join(format!("hunk_{}.patch", std::process::id()));
    std::fs::write(&temp, &patch).map_err(|e| format!("Failed to write patch: {e}"))?;
    let result = run_git(&workspace, &["apply", "--cached", temp.to_str().unwrap()]);
    let _ = std::fs::remove_file(&temp);
    result.map(|_| ())
}

#[tauri::command]
pub fn git_unstage_hunk(workspace: String, path: String, hunk_text: String) -> Result<(), String> {
    // Same as stage but with --reverse
    let header = run_git(&workspace, &["diff", "--cached", "--", &path]).unwrap_or_default();
    let diff_header: String = header.lines()
        .take_while(|l| !l.starts_with("@@"))
        .chain(std::iter::once(""))
        .collect::<Vec<_>>()
        .join("\n");
    
    let patch = format!("{}\n{}", diff_header, hunk_text);
    
    let temp = std::env::temp_dir().join(format!("hunk_{}.patch", std::process::id()));
    std::fs::write(&temp, &patch).map_err(|e| format!("Failed to write patch: {e}"))?;
    let result = run_git(&workspace, &["apply", "--cached", "--reverse", temp.to_str().unwrap()]);
    let _ = std::fs::remove_file(&temp);
    result.map(|_| ())
}

#[tauri::command]
pub fn git_stage_all(workspace: String) -> Result<(), String> {
    run_git(&workspace, &["add", "-A"])?;
    Ok(())
}

#[tauri::command]
pub fn git_unstage_all(workspace: String) -> Result<(), String> {
    run_git(&workspace, &["reset", "HEAD"])?;
    Ok(())
}
