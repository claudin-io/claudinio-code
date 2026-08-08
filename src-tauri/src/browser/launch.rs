//! Starting and stopping the Chromium process.

use crate::procutil;
use std::path::{Path, PathBuf};
use std::time::Duration;
use tokio::process::{Child, Command};

/// How long to wait for Chromium to write its `DevToolsActivePort` file.
const LAUNCH_TIMEOUT: Duration = Duration::from_secs(20);
const PORT_POLL_INTERVAL: Duration = Duration::from_millis(50);

/// Records the PID of the process we started, so a crash of the app does not
/// leave an orphaned Chromium holding ~300 MB until the machine reboots.
const PID_FILE: &str = "claudinio-browser.pid";

/// Flags every launch gets.
///
/// Two groups are load-bearing rather than cosmetic:
/// - the `--disable-background*` / `--disable-renderer-backgrounding` trio:
///   without them a headed window that is behind another window gets throttled
///   by the compositor and screenshots come back blank or stale;
/// - the network/telemetry group: this browser exists to look at the page under
///   development, and background traffic to Google's update and metrics
///   endpoints would show up in the user's own network panel.
///
/// Deliberately absent: `--no-sandbox`. Running a browser that renders
/// arbitrary web content with the sandbox off, inside the user's workspace, is
/// a privilege escalation, so it is opt-in through an env var instead.
const BASE_FLAGS: &[&str] = &[
    "--remote-debugging-port=0",
    "--no-first-run",
    "--no-default-browser-check",
    "--no-service-autorun",
    "--disable-background-networking",
    "--disable-component-update",
    "--disable-client-side-phishing-detection",
    "--disable-sync",
    "--disable-extensions",
    "--disable-default-apps",
    "--disable-popup-blocking",
    "--disable-prompt-on-repost",
    "--disable-hang-monitor",
    "--metrics-recording-only",
    "--no-pings",
    "--password-store=basic",
    "--hide-crash-restore-bubble",
    "--disable-backgrounding-occluded-windows",
    "--disable-renderer-backgrounding",
    "--disable-background-timer-throttling",
    "--disable-ipc-flooding-protection",
];

pub struct LaunchOptions {
    pub exe: PathBuf,
    pub user_data_dir: PathBuf,
    pub headless: bool,
    pub viewport_width: u32,
    pub viewport_height: u32,
}

pub struct LaunchedBrowser {
    pub child: Child,
    /// The `ws://127.0.0.1:PORT/devtools/browser/<uuid>` endpoint.
    pub ws_url: String,
    pub user_data_dir: PathBuf,
}

/// A profile directory per workspace.
///
/// Two workspaces sharing one `--user-data-dir` is a subtle failure, not a
/// loud one: the second Chromium hands its URL to the first and exits
/// immediately, so `DevToolsActivePort` never appears and the launch dies of an
/// unexplained timeout.
pub fn profile_dir_for(browser_dir: &Path, workspace_root: &Path) -> PathBuf {
    let hash = xxhash_rust::xxh3::xxh3_64(workspace_root.to_string_lossy().as_bytes());
    browser_dir.join(format!("profile-{hash:016x}"))
}

pub async fn launch(opts: LaunchOptions) -> Result<LaunchedBrowser, String> {
    std::fs::create_dir_all(&opts.user_data_dir).map_err(|e| format!("create profile dir: {e}"))?;

    // A previous run may have died without cleaning up; that process still
    // holds this profile and would make our launch time out.
    kill_recorded_orphan(&opts.user_data_dir, &opts.exe);
    let port_file = opts.user_data_dir.join("DevToolsActivePort");
    let _ = std::fs::remove_file(&port_file);

    let mut cmd = Command::new(&opts.exe);
    cmd.args(BASE_FLAGS)
        .arg(format!("--user-data-dir={}", opts.user_data_dir.display()))
        .arg(format!(
            "--window-size={},{}",
            opts.viewport_width, opts.viewport_height
        ));
    if opts.headless {
        cmd.arg("--headless=new");
        // Several Windows GPU drivers hang the new headless mode without this.
        if cfg!(target_os = "windows") {
            cmd.arg("--disable-gpu");
        }
    }
    if cfg!(target_os = "macos") {
        // Otherwise the first launch pops a Keychain prompt at the user.
        cmd.arg("--use-mock-keychain");
    }
    if std::env::var("CLAUDINIO_BROWSER_NO_SANDBOX").as_deref() == Ok("1") {
        cmd.arg("--no-sandbox");
    }
    cmd.arg("about:blank");

    cmd.stdin(std::process::Stdio::null())
        .stdout(std::process::Stdio::null())
        // Chromium reports launch failures here and nowhere else.
        .stderr(std::process::Stdio::piped())
        .kill_on_drop(true);
    procutil::no_window_tokio(&mut cmd);

    let mut child = cmd
        .spawn()
        .map_err(|e| format!("launch {}: {e}", opts.exe.display()))?;

    if let Some(pid) = child.id() {
        let _ = std::fs::write(opts.user_data_dir.join(PID_FILE), pid.to_string());
    }

    match wait_for_ws_url(&port_file, &mut child).await {
        Ok(ws_url) => Ok(LaunchedBrowser {
            child,
            ws_url,
            user_data_dir: opts.user_data_dir,
        }),
        Err(e) => {
            let _ = child.kill().await;
            Err(e)
        }
    }
}

/// Poll for `DevToolsActivePort`, which Chromium writes once the debugging
/// socket is listening: line 1 is the port, line 2 the browser endpoint path.
///
/// Port 0 plus this file is what makes concurrent workspaces safe — no fixed
/// port to collide on.
async fn wait_for_ws_url(port_file: &Path, child: &mut Child) -> Result<String, String> {
    let deadline = tokio::time::Instant::now() + LAUNCH_TIMEOUT;
    loop {
        if let Ok(contents) = std::fs::read_to_string(port_file)
            && let Some(ws) = parse_devtools_port_file(&contents)
        {
            return Ok(ws);
        }
        // Exiting early is the common failure (missing shared library, bad
        // flag); surfacing Chromium's own stderr beats a bare timeout.
        if let Ok(Some(status)) = child.try_wait() {
            return Err(format!(
                "Chromium exited before it was ready (status {status}). {}",
                drain_stderr(child).await
            ));
        }
        if tokio::time::Instant::now() >= deadline {
            return Err(format!(
                "Chromium did not report a debugging port within {}s. {}",
                LAUNCH_TIMEOUT.as_secs(),
                drain_stderr(child).await
            ));
        }
        tokio::time::sleep(PORT_POLL_INTERVAL).await;
    }
}

async fn drain_stderr(child: &mut Child) -> String {
    use tokio::io::AsyncReadExt;
    let Some(mut err) = child.stderr.take() else {
        return String::new();
    };
    let mut buf = Vec::new();
    // The process is already gone or hung, so this returns promptly; the
    // timeout only guards the hung case.
    let _ = tokio::time::timeout(Duration::from_secs(2), err.read_to_end(&mut buf)).await;
    let text = String::from_utf8_lossy(&buf);
    let tail: Vec<&str> = text.lines().rev().take(5).collect();
    if tail.is_empty() {
        String::new()
    } else {
        format!(
            "Chromium said: {}",
            tail.into_iter().rev().collect::<Vec<_>>().join(" / ")
        )
    }
}

/// `DevToolsActivePort` is exactly two lines: the port, then the endpoint path.
pub fn parse_devtools_port_file(contents: &str) -> Option<String> {
    let mut lines = contents.lines();
    let port: u16 = lines.next()?.trim().parse().ok()?;
    let path = lines.next()?.trim();
    if path.is_empty() {
        return None;
    }
    Some(format!("ws://127.0.0.1:{port}{path}"))
}

/// Kill a Chromium recorded by a previous run of this app that is still alive.
///
/// The recorded PID is only acted on when the live process's executable matches
/// the one we launch: PIDs are recycled, and killing whatever inherited the
/// number would be someone else's process.
fn kill_recorded_orphan(user_data_dir: &Path, exe: &Path) {
    let pid_path = user_data_dir.join(PID_FILE);
    let Ok(text) = std::fs::read_to_string(&pid_path) else {
        return;
    };
    let Ok(pid) = text.trim().parse::<u32>() else {
        let _ = std::fs::remove_file(&pid_path);
        return;
    };

    let mut sys = sysinfo::System::new();
    let spid = sysinfo::Pid::from_u32(pid);
    sys.refresh_processes(sysinfo::ProcessesToUpdate::Some(&[spid]), true);
    if let Some(proc) = sys.process(spid)
        && proc.exe() == Some(exe)
    {
        eprintln!("[browser] killing orphaned Chromium from a previous run (pid {pid})");
        proc.kill();
    }
    let _ = std::fs::remove_file(&pid_path);
}

/// Terminate a launched browser and clear its PID record.
///
/// `kill_on_drop` alone is not enough: it kills the parent, while Chromium's
/// renderer children are a separate tree. The caller is expected to have tried
/// a graceful `Browser.close` over CDP first, which makes Chromium tear its own
/// children down; this is the fallback for when that does not land.
pub async fn shutdown(mut launched: LaunchedBrowser) {
    let pid = launched.child.id();
    let _ = launched.child.start_kill();
    let _ = tokio::time::timeout(Duration::from_secs(3), launched.child.wait()).await;

    #[cfg(target_os = "windows")]
    if let Some(pid) = pid {
        // /T takes the whole tree, which is the part start_kill misses.
        let mut cmd = std::process::Command::new("taskkill");
        cmd.args(["/PID", &pid.to_string(), "/T", "/F"]);
        procutil::no_window(&mut cmd);
        let _ = cmd.output();
    }
    #[cfg(not(target_os = "windows"))]
    let _ = pid;

    let _ = std::fs::remove_file(launched.user_data_dir.join(PID_FILE));
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn parses_a_devtools_port_file() {
        let ws = parse_devtools_port_file("54321\n/devtools/browser/abc-123\n").unwrap();
        assert_eq!(ws, "ws://127.0.0.1:54321/devtools/browser/abc-123");
    }

    #[test]
    fn rejects_a_half_written_port_file() {
        // Chromium writes this file in two steps, so a poll can catch it with
        // only the port present. Treating that as ready would build a bogus URL.
        assert!(parse_devtools_port_file("54321\n").is_none());
        assert!(parse_devtools_port_file("54321").is_none());
        assert!(parse_devtools_port_file("").is_none());
        assert!(parse_devtools_port_file("not-a-port\n/devtools/x").is_none());
    }

    #[test]
    fn profile_dirs_differ_per_workspace() {
        let root = Path::new("/tmp/browser");
        let a = profile_dir_for(root, Path::new("/home/u/project-a"));
        let b = profile_dir_for(root, Path::new("/home/u/project-b"));
        assert_ne!(a, b, "two workspaces must not share one Chromium profile");
        assert_eq!(a, profile_dir_for(root, Path::new("/home/u/project-a")));
    }

    #[test]
    fn sandbox_is_not_disabled_by_default() {
        assert!(
            !BASE_FLAGS.contains(&"--no-sandbox"),
            "--no-sandbox must stay opt-in"
        );
    }
}
