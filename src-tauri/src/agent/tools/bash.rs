use serde::Deserialize;
use std::path::Path;
use std::sync::atomic::{AtomicBool, Ordering};
use std::sync::Arc;
use std::sync::OnceLock;
use std::time::Duration;
use tokio::process::Command;

use crate::agent::tools::ToolContext;

const MAX_OUTPUT_BYTES: u64 = 100 * 1024;
const DEFAULT_TIMEOUT_SECS: u64 = 30;

/// Cache for the resolved login PATH (with nvm, cargo, etc.).
/// Resolved once on first `execute()` call, reused for all subsequent calls.
static LOGIN_PATH: OnceLock<String> = OnceLock::new();

/// Build a PATH that includes the user's login shell PATH (from .zshrc / .bashrc etc.).
///
/// Platform behavior:
/// - **macOS**: runs `$SHELL -l -c 'echo $PATH'` once to extract the full login PATH
///   (nvm, cargo, homebrew, etc.), avoiding TCC prompts that macOS shows when `sh`
///   inherits a PATH referencing protected directories.
/// - **Linux**: same login-shell extraction — works transparently.
/// - **Windows**: returns the current process PATH directly (no login-shell concept).
///
/// The result is cached in a `OnceLock` after the first call.
fn resolve_login_path() -> String {
    // Windows: no login-shell concept, just use current PATH
    if cfg!(target_os = "windows") {
        return std::env::var("PATH").unwrap_or_default();
    }

    // macOS / Linux: extract PATH from the user's configured login shell
    // so that nvm, cargo, homebrew, and other custom paths are available
    // without the LLM needing to prepend `export PATH=...` manually.
    let shell = std::env::var("SHELL").unwrap_or_else(|_| "/bin/sh".into());

    let output = std::process::Command::new(&shell)
        .arg("-l")
        .arg("-c")
        .arg("echo $PATH")
        .output();

    match output {
        Ok(out) if out.status.success() => {
            let path = String::from_utf8_lossy(&out.stdout).trim().to_string();
            if !path.is_empty() {
                return path;
            }
        }
        _ => { /* fall through to curated fallback */ }
    }

    // Fallback: scan known binary directories for this machine.
    // Used when the login-shell call fails (e.g. macOS TCC blocks it on first launch).
    let home = std::env::var("HOME").unwrap_or_else(|_| "/Users/victortavernari".into());
    let mut paths: Vec<String> = Vec::new();

    // NVM node bins (sorted by version, latest first)
    if cfg!(unix) {
        let nvm_base = format!("{}/.nvm/versions/node", home);
        if let Ok(entries) = std::fs::read_dir(&nvm_base) {
            let mut version_bins: Vec<String> = entries
                .filter_map(|e| e.ok())
                .filter(|e| e.file_type().map(|t| t.is_dir()).unwrap_or(false))
                .map(|e| e.path().join("bin"))
                .filter(|p| p.exists())
                .map(|p| p.to_string_lossy().to_string())
                .collect();
            version_bins.sort_by(|a, b| b.cmp(a));
            paths.extend(version_bins);
        }
    }

    // Cargo bin
    if cfg!(unix) {
        let cargo_bin = format!("{}/.cargo/bin", home);
        if Path::new(&cargo_bin).exists() {
            paths.push(cargo_bin);
        }
    }

    // Homebrew (macOS)
    if cfg!(target_os = "macos") {
        for dir in &["/opt/homebrew/bin", "/opt/homebrew/sbin"] {
            if Path::new(dir).exists() {
                paths.push(dir.to_string());
            }
        }
    }

    // Current process PATH as tail (always — works on all platforms)
    if let Ok(existing) = std::env::var("PATH") {
        paths.push(existing);
    }

    paths.join(":")
}

#[derive(Deserialize)]
pub struct BashArgs {
    pub command: String,
    pub workdir: Option<String>,
    pub stdin: Option<String>,
    pub timeout_seconds: Option<u64>,
}

pub async fn execute(args: BashArgs, ctx: &ToolContext) -> Result<String, String> {
    let shell = if cfg!(target_os = "windows") { "cmd" } else { "sh" };
    let shell_flag = if cfg!(target_os = "windows") { "/c" } else { "-c" };

    let timeout_secs = args.timeout_seconds.unwrap_or(DEFAULT_TIMEOUT_SECS);

    // Resolve and inject the login PATH (cached in OnceLock after first call).
    // This avoids the LLM needing to prepend `export PATH=...` to every command,
    // which on macOS triggers repeated TCC permission prompts.
    let login_path = LOGIN_PATH.get_or_init(resolve_login_path);

    let mut child = Command::new(shell)
        .arg(shell_flag)
        .arg(&args.command)
        .env("PATH", login_path)
        .current_dir(args.workdir.as_deref().unwrap_or("."))
        .stdin(std::process::Stdio::piped())
        .stdout(std::process::Stdio::piped())
        .stderr(std::process::Stdio::piped())
        .kill_on_drop(true)
        .spawn()
        .map_err(|e| format!("failed to spawn command: {e}"))?;

    if let Some(stdin_data) = &args.stdin {
        if let Some(stdin_handle) = child.stdin.take() {
            use tokio::io::AsyncWriteExt;
            let mut writer = stdin_handle;
            let _ = writer.write_all(stdin_data.as_bytes()).await;
            let _ = writer.shutdown().await;
        }
    }

    // Take the stdout/stderr handles before the select loop so we can
    // still read them after child.wait() completes.
    let mut child_stdout = child.stdout.take();
    let mut child_stderr = child.stderr.take();

    let interrupt = &ctx.interrupt;
    let timeout_sleep = tokio::time::sleep(Duration::from_secs(timeout_secs));
    tokio::pin!(timeout_sleep);

    // Outer loop: only the interrupt branch loops (check every 200ms);
    // the timeout and child-completion branches exit it.
    let result = loop {
        tokio::select! {
            status = child.wait() => {
                break status.map_err(|e| format!("command failed: {e}"));
            }
            _ = &mut timeout_sleep => {
                return Err(format!(
                    "command timed out after {timeout_secs}s and was killed"
                ));
            }
            _ = poll_interrupt(interrupt) => {
                // User hit pause/ESC: kill the child process eagerly
                let _ = child.kill().await;
                let _ = child.wait().await; // reap to avoid zombie
                return Err("Interrupted by user".into());
            }
        }
    }?;

    // Read captured stdout/stderr
    use tokio::io::AsyncReadExt;
    let stdout_text = match child_stdout.as_mut() {
        Some(pipe) => {
            let mut buf = Vec::new();
            let _ = pipe.read_to_end(&mut buf).await;
            String::from_utf8_lossy(&buf).to_string()
        }
        None => String::new(),
    };
    let stderr_text = match child_stderr.as_mut() {
        Some(pipe) => {
            let mut buf = Vec::new();
            let _ = pipe.read_to_end(&mut buf).await;
            String::from_utf8_lossy(&buf).to_string()
        }
        None => String::new(),
    };

    let mut text = String::new();
    if !stdout_text.is_empty() {
        text.push_str(&stdout_text);
    }
    if !stderr_text.is_empty() {
        if !text.is_empty() {
            text.push('\n');
        }
        text.push_str(&stderr_text);
    }

    if !result.success() {
        let exit_info = match result.code() {
            Some(code) => format!("exit code {code}"),
            None => "terminated by signal".into(),
        };
        if text.trim().is_empty() {
            return Err(exit_info);
        }
        text = format!("{text}\n({exit_info})");
    }

    if text.len() as u64 > MAX_OUTPUT_BYTES {
        let mut end = MAX_OUTPUT_BYTES as usize;
        while end < text.len() && !text.is_char_boundary(end) {
            end += 1;
        }
        text.truncate(end);
        text.push_str(&format!(
            "\n...(output truncated, {} chars total)",
            text.len()
        ));
    }

    Ok(text)
}

/// Poll the interrupt flag at ~200ms intervals until it becomes true.
async fn poll_interrupt(interrupt: &Option<Arc<AtomicBool>>) {
    let flag = match interrupt {
        Some(f) => f,
        None => {
            // No interrupt available — wait forever (timeout or child end wins)
            std::future::pending::<()>().await;
            unreachable!()
        }
    };
    loop {
        tokio::time::sleep(Duration::from_millis(200)).await;
        if flag.load(Ordering::SeqCst) {
            return;
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::time::Instant;

    fn run(cmd: &str) -> Result<String, String> {
        let rt = tokio::runtime::Runtime::new().unwrap();
        rt.block_on(execute(
            BashArgs {
                command: cmd.to_string(),
                workdir: None,
                stdin: None,
                timeout_seconds: None,
            },
            &ToolContext {
                db_path: None,
                lsp_manager: None,
                workspace_root: None,
                embedding_model: std::sync::Arc::new(tokio::sync::Mutex::new(None)),
                session_store_path: None,
                read_tracker: std::sync::Arc::new(tokio::sync::Mutex::new(
                    crate::agent::tools::ReadTracker::default(),
                )),
                interrupt: None,
                agent_config: None,
            },
        ))
    }

    #[test]
    fn echo_hello() {
        let out = run("echo hello").unwrap();
        assert_eq!(out.trim(), "hello");
    }

    #[test]
    fn echo_with_stdin() {
        let rt = tokio::runtime::Runtime::new().unwrap();
        let ctx = crate::agent::tools::ToolContext {
            db_path: None,
            lsp_manager: None,
            workspace_root: None,
            embedding_model: std::sync::Arc::new(tokio::sync::Mutex::new(None)),
            session_store_path: None,
            read_tracker: std::sync::Arc::new(tokio::sync::Mutex::new(
                crate::agent::tools::ReadTracker::default(),
            )),
            interrupt: None,
            agent_config: None,
        };
        let out = rt.block_on(execute(
            BashArgs {
                command: "cat".to_string(),
                workdir: None,
                stdin: Some("hello from stdin".to_string()),
                timeout_seconds: Some(5),
            },
            &ctx,
        ));
        let out = out.unwrap();
        assert!(out.contains("hello from stdin"), "got: {out}");
    }

    #[test]
    fn exit_code_non_zero_reported() {
        let out = run("false");
        assert!(out.is_err() || out.unwrap().contains("exit code 1"));
    }

    #[test]
    fn workdir_changes_output() {
        let rt = tokio::runtime::Runtime::new().unwrap();
        let tmp = std::env::temp_dir();
        let canonical_tmp = std::fs::canonicalize(&tmp).unwrap_or(tmp.clone());
        let ctx = crate::agent::tools::ToolContext {
            db_path: None,
            lsp_manager: None,
            workspace_root: None,
            embedding_model: std::sync::Arc::new(tokio::sync::Mutex::new(None)),
            session_store_path: None,
            read_tracker: std::sync::Arc::new(tokio::sync::Mutex::new(
                crate::agent::tools::ReadTracker::default(),
            )),
            interrupt: None,
            agent_config: None,
        };
        let out = rt.block_on(execute(
            BashArgs {
                command: "pwd".to_string(),
                workdir: Some(canonical_tmp.to_string_lossy().to_string()),
                stdin: None,
                timeout_seconds: Some(5),
            },
            &ctx,
        ));
        let out = out.unwrap();
        let pwd = out.trim();
        assert!(
            pwd == canonical_tmp.to_string_lossy().as_ref(),
            "expected pwd='{}', got='{}'",
            canonical_tmp.display(),
            pwd
        );
    }

    #[test]
    fn timeout_kills_command() {
        let rt = tokio::runtime::Runtime::new().unwrap();
        let start = Instant::now();
        let ctx = crate::agent::tools::ToolContext {
            db_path: None,
            lsp_manager: None,
            workspace_root: None,
            embedding_model: std::sync::Arc::new(tokio::sync::Mutex::new(None)),
            session_store_path: None,
            read_tracker: std::sync::Arc::new(tokio::sync::Mutex::new(
                crate::agent::tools::ReadTracker::default(),
            )),
            interrupt: None,
            agent_config: None,
        };
        let result = rt.block_on(execute(
            BashArgs {
                command: "sleep 60".to_string(),
                workdir: None,
                stdin: None,
                timeout_seconds: Some(1),
            },
            &ctx,
        ));
        let elapsed = start.elapsed();
        assert!(result.is_err(), "expected timeout error, got: {result:?}");
        let err = result.unwrap_err();
        assert!(err.contains("timed out"), "expected timeout message, got: {err}");
        assert!(elapsed.as_secs() < 10, "timeout took too long: {elapsed:?}");
    }

    #[test]
    fn stderr_included_in_output() {
        let out = run("echo ok && echo err >&2");
        let out = out.unwrap();
        assert!(out.contains("ok"));
        assert!(out.contains("err"));
    }

    #[test]
    fn command_not_found() {
        let result = run("nonexistent_command_xyz123");
        // Should either error or include some failure message
        match result {
            Ok(text) => assert!(
                !text.is_empty(),
                "should report command not found, got empty"
            ),
            Err(e) => assert!(
                e.contains("not found") || e.contains("No such file")
                    || e.contains("failed to spawn"),
                "unexpected error: {e}"
            ),
        }
    }

    #[test]
    fn pwd_default_workdir() {
        let cwd = std::env::current_dir().unwrap();
        let cwd_str = cwd.to_string_lossy().to_string();
        let out = run("pwd").unwrap();
        assert_eq!(out.trim(), cwd_str);
    }

    #[test]
    fn multiline_output() {
        let out = run("echo line1 && echo line2 && echo line3").unwrap();
        let lines: Vec<&str> = out.lines().collect();
        assert!(lines.len() >= 3, "expected 3+ lines, got {}: {out:?}", lines.len());
    }
}
