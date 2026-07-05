use serde::Deserialize;
use tokio::process::Command;

const MAX_OUTPUT_BYTES: u64 = 100 * 1024;
const DEFAULT_TIMEOUT_SECS: u64 = 30;

#[derive(Deserialize)]
pub struct BashArgs {
    pub command: String,
    pub workdir: Option<String>,
    pub stdin: Option<String>,
    pub timeout_seconds: Option<u64>,
}

pub async fn execute(args: BashArgs) -> Result<String, String> {
    let shell = if cfg!(target_os = "windows") { "cmd" } else { "sh" };
    let shell_flag = if cfg!(target_os = "windows") { "/c" } else { "-c" };

    let timeout_secs = args.timeout_seconds.unwrap_or(DEFAULT_TIMEOUT_SECS);

    let mut child = Command::new(shell)
        .arg(shell_flag)
        .arg(&args.command)
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

    let result = tokio::time::timeout(
        std::time::Duration::from_secs(timeout_secs),
        child.wait_with_output(),
    )
    .await;

    let output = match result {
        Ok(Ok(out)) => out,
        Ok(Err(e)) => return Err(format!("command failed: {e}")),
        Err(_) => {
            return Err(format!(
                "command timed out after {timeout_secs}s and was killed"
            ));
        }
    };

    let mut text = String::new();
    if !output.stdout.is_empty() {
        let stdout_str = String::from_utf8_lossy(&output.stdout);
        text.push_str(stdout_str.as_ref());
    }
    if !output.stderr.is_empty() {
        if !text.is_empty() {
            text.push('\n');
        }
        let stderr_str = String::from_utf8_lossy(&output.stderr);
        text.push_str(stderr_str.as_ref());
    }

    if !output.status.success() {
        let exit_info = match output.status.code() {
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

#[cfg(test)]
mod tests {
    use super::*;
    use std::time::Instant;

    fn run(cmd: &str) -> Result<String, String> {
        let rt = tokio::runtime::Runtime::new().unwrap();
        rt.block_on(execute(BashArgs {
            command: cmd.to_string(),
            workdir: None,
            stdin: None,
            timeout_seconds: None,
        }))
    }

    #[test]
    fn echo_hello() {
        let out = run("echo hello").unwrap();
        assert_eq!(out.trim(), "hello");
    }

    #[test]
    fn echo_with_stdin() {
        let rt = tokio::runtime::Runtime::new().unwrap();
        let out = rt.block_on(execute(BashArgs {
            command: "cat".to_string(),
            workdir: None,
            stdin: Some("hello from stdin".to_string()),
            timeout_seconds: Some(5),
        }));
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
        let out = rt.block_on(execute(BashArgs {
            command: "pwd".to_string(),
            workdir: Some(canonical_tmp.to_string_lossy().to_string()),
            stdin: None,
            timeout_seconds: Some(5),
        }));
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
        let result = rt.block_on(execute(BashArgs {
            command: "sleep 60".to_string(),
            workdir: None,
            stdin: None,
            timeout_seconds: Some(1),
        }));
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
