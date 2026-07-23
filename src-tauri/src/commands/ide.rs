use std::process::Command;

/// Detect which supported IDEs are installed on this machine.
#[tauri::command]
pub async fn detect_ides() -> Result<Vec<String>, String> {
    let mut found: Vec<String> = Vec::new();

    #[cfg(target_os = "macos")]
    {
        // VS Code – check both the CLI and the bundle
        if Command::new("code").arg("--version").output().is_ok()
            || std::path::Path::new("/Applications/Visual Studio Code.app").exists()
        {
            found.push("vscode".into());
        }
        if Command::new("cursor").arg("--version").output().is_ok()
            || std::path::Path::new("/Applications/Cursor.app").exists()
        {
            found.push("cursor".into());
        }
    }

    #[cfg(target_os = "windows")]
    {
        if Command::new("code").arg("--version").output().is_ok() {
            found.push("vscode".into());
        }
        if Command::new("cursor").arg("--version").output().is_ok() {
            found.push("cursor".into());
        }
    }

    #[cfg(target_os = "linux")]
    {
        let login_path = crate::agent::tools::bash::login_path();
        if which_in_path("code", &login_path).is_some() {
            found.push("vscode".into());
        }
        if which_in_path("cursor", &login_path).is_some() {
            found.push("cursor".into());
        }
    }

    Ok(found)
}

/// Open a path (file or folder) in the specified IDE, optionally at a line number.
#[tauri::command]
pub async fn open_in_ide(path: String, ide: String, goto_line: Option<u32>) -> Result<(), String> {
    match ide.as_str() {
        "vscode" => open_ide("code", "Visual Studio Code", &path, goto_line),
        "cursor" => open_ide("cursor", "Cursor", &path, goto_line),
        other => Err(format!("Unknown IDE: {other}")),
    }
}

fn open_ide(cli: &str, app_name: &str, path: &str, goto_line: Option<u32>) -> Result<(), String> {
    if let Some(line) = goto_line {
        // --goto requires the CLI on all platforms
        let mut cmd = Command::new(cli);
        cmd.arg("--goto").arg(format!("{path}:{line}:1"));
        crate::procutil::no_window(&mut cmd);
        cmd.spawn()
            .map_err(|e| format!("Failed to open {app_name} with goto: {e}"))?;
    } else {
        #[cfg(target_os = "macos")]
        {
            Command::new("open")
                .args(["-a", app_name, path])
                .spawn()
                .map_err(|e| format!("Failed to open {app_name}: {e}"))?;
        }
        #[cfg(target_os = "windows")]
        {
            let mut cmd = Command::new("cmd");
            cmd.args(["/c", cli, path]);
            crate::procutil::no_window(&mut cmd);
            cmd.spawn()
                .map_err(|e| format!("Failed to open {app_name}: {e}"))?;
        }
        #[cfg(target_os = "linux")]
        {
            let login_path = crate::agent::tools::bash::login_path();
            let mut cmd = Command::new(cli);
            cmd.env("PATH", &login_path).arg(path);
            crate::procutil::no_window(&mut cmd);
            cmd.spawn()
                .map_err(|e| format!("Failed to open {app_name}: {e}"))?;
        }
    }
    Ok(())
}

#[cfg(target_os = "linux")]
fn which_in_path(binary: &str, login_path: &str) -> Option<String> {
    let full_path = format!("{login_path}:/usr/local/bin:/usr/bin:/bin");
    std::env::split_paths(&full_path)
        .find(|dir| dir.join(binary).exists())
        .map(|d| d.join(binary).to_string_lossy().to_string())
}
