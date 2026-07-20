/// Windows `CREATE_NO_WINDOW` flag — prevents every spawned child process
/// (git, cmd, etc.) from allocating its own console window when the app
/// runs as a GUI subsystem binary. Without this, each spawn flashes a
/// console (and can spawn `conhost.exe`) which is what users saw behind
/// the frozen window in the v0.1.3/v0.1.4 Windows freeze reports.
#[cfg(target_os = "windows")]
pub const CREATE_NO_WINDOW: u32 = 0x0800_0000;

#[cfg(target_os = "windows")]
pub fn no_window(cmd: &mut std::process::Command) {
    use std::os::windows::process::CommandExt;
    cmd.creation_flags(CREATE_NO_WINDOW);
}

#[cfg(target_os = "windows")]
pub fn no_window_tokio(cmd: &mut tokio::process::Command) {
    cmd.creation_flags(CREATE_NO_WINDOW);
}

#[cfg(not(target_os = "windows"))]
pub fn no_window(_cmd: &mut std::process::Command) {}

#[cfg(not(target_os = "windows"))]
pub fn no_window_tokio(_cmd: &mut tokio::process::Command) {}
