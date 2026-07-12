use std::process::Command;

#[tauri::command]
pub fn open_in_terminal(path: String) -> Result<(), String> {
    #[cfg(target_os = "macos")]
    {
        Command::new("open")
            .args(["-b", "com.apple.Terminal", &path])
            .output()
            .map_err(|e| format!("Failed to open Terminal: {e}"))?;
    }
    #[cfg(target_os = "windows")]
    {
        // `start` opens a new visible terminal window on purpose here (that's
        // the point of "Open in Terminal"), so CREATE_NO_WINDOW is NOT applied
        // to this spawn — unlike the background git/rg spawns elsewhere.
        Command::new("cmd")
            .args(["/c", "start", "cmd", "/k", "cd", "/d", &path])
            .output()
            .map_err(|e| format!("Failed to open Terminal: {e}"))?;
    }
    #[cfg(target_os = "linux")]
    {
        let terminal = std::env::var("TERMINAL")
            .unwrap_or_else(|_| "x-terminal-emulator".to_string());
        Command::new(&terminal)
            .arg(&path)
            .output()
            .map_err(|e| format!("Failed to open Terminal: {e}"))?;
    }

    Ok(())
}
