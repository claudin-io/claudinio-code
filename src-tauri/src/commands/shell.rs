use std::process::Command;

#[tauri::command]
pub async fn open_in_terminal(path: String) -> Result<(), String> {
    #[cfg(target_os = "macos")]
    {
        Command::new("open")
            .args(["-b", "com.apple.Terminal", &path])
            .spawn()
            .map_err(|e| format!("Failed to open Terminal: {e}"))?;
    }
    #[cfg(target_os = "windows")]
    {
        use std::os::windows::process::CommandExt;
        // CREATE_NO_WINDOW suppresses the console flash of the wrapper `cmd`
        // process itself; the `start`-ed terminal window it launches is a
        // separate process and remains visible, which is the whole point.
        const CREATE_NO_WINDOW: u32 = 0x08000000;
        Command::new("cmd")
            .args(["/c", "start", "cmd", "/k", "cd", "/d", &path])
            .creation_flags(CREATE_NO_WINDOW)
            .spawn()
            .map_err(|e| format!("Failed to open Terminal: {e}"))?;
    }
    #[cfg(target_os = "linux")]
    {
        let terminal = std::env::var("TERMINAL")
            .unwrap_or_else(|_| "x-terminal-emulator".to_string());
        Command::new(&terminal)
            .arg(&path)
            .spawn()
            .map_err(|e| format!("Failed to open Terminal: {e}"))?;
    }

    Ok(())
}
