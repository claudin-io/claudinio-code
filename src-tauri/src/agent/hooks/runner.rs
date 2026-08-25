//! Running a hook.
//!
//! Mechanically this is `agent::tools::bash`, minus the parts a hook must not
//! have. Two omissions are deliberate and both are security decisions:
//!
//! - **No askpass environment.** `bash` injects it so `git push` can ask for a
//!   passphrase through the app's modal. A hook is not git, and letting a
//!   config-declared program raise the credential dialog is a phishing surface
//!   with no compensating use.
//! - **No timeout extension.** `bash` resets its deadline while a credential
//!   prompt is open. A hook must never be able to extend its own deadline;
//!   `UserPromptSubmit` blocks the first API call, and the user is waiting.

use super::config::HookEvent;
use super::discovery::ResolvedHook;
use super::outcome::{HookEffect, HookStatus, RunResult, interpret};
use crate::agent::tools::bash::login_path;
use crate::procutil::no_window_tokio;
use serde_json::Value;
use std::path::Path;
use std::sync::Arc;
use std::sync::atomic::{AtomicBool, Ordering};
use std::time::Duration;
use tokio::process::Command;

/// Output cap per stream. The same number `bash` uses, for the same reason: this
/// is handed to a model or written to a file that is re-read on every message.
pub const MAX_OUTPUT_BYTES: usize = 100 * 1024;

/// How many hooks may run at once. A batch is usually one or two; the bound
/// exists so a pathological config cannot fork-bomb the machine.
const MAX_CONCURRENT: usize = 8;

/// One hook, run.
#[derive(Debug, Clone)]
pub struct HookRun {
    pub hook: ResolvedHook,
    pub effect: HookEffect,
    pub exit_code: Option<i32>,
    pub stdout: String,
    pub stderr: String,
    pub duration_ms: u64,
}

impl HookRun {
    pub fn status(&self) -> HookStatus {
        self.effect.status()
    }
}

/// Everything a spawned hook needs that is not the hook itself.
#[derive(Debug, Clone)]
pub struct RunEnv {
    pub project_dir: String,
    pub session_id: String,
    pub interrupt: Option<Arc<AtomicBool>>,
}

/// Run one hook to completion.
pub async fn run_one(hook: &ResolvedHook, input: &Value, env: &RunEnv) -> HookRun {
    let started = std::time::Instant::now();
    let payload = serde_json::to_string(input).unwrap_or_else(|_| "{}".into());
    let result = spawn_and_wait(hook, &payload, env).await;
    let (exit_code, stdout, stderr) = match &result {
        RunResult::Exited {
            code,
            stdout,
            stderr,
        } => (Some(*code), stdout.clone(), stderr.clone()),
        _ => (None, String::new(), String::new()),
    };
    HookRun {
        effect: interpret(hook.event, &result),
        hook: hook.clone(),
        exit_code,
        stdout,
        stderr,
        duration_ms: started.elapsed().as_millis() as u64,
    }
}

async fn spawn_and_wait(hook: &ResolvedHook, payload: &str, env: &RunEnv) -> RunResult {
    let mut cmd = build_command(hook, env);
    cmd.stdin(std::process::Stdio::piped())
        .stdout(std::process::Stdio::piped())
        .stderr(std::process::Stdio::piped())
        .kill_on_drop(true);
    no_window_tokio(&mut cmd);

    let mut child = match cmd.spawn() {
        Ok(c) => c,
        Err(e) => {
            // The ordinary case, not an exception: a hook is a line in a config
            // file and the program it names may simply not be installed.
            return RunResult::SpawnFailed {
                message: format!("could not run `{}`: {e}", hook.command),
            };
        }
    };

    if let Some(stdin) = child.stdin.take() {
        use tokio::io::AsyncWriteExt;
        let mut w = stdin;
        let _ = w.write_all(payload.as_bytes()).await;
        // Closed explicitly: a hook that reads stdin to EOF (every `jq`-based
        // one does) hangs forever otherwise, and "hangs forever" here means
        // "the user's prompt never reaches the model".
        let _ = w.shutdown().await;
    }

    // Drained concurrently with the wait, not after it. A hook that writes more
    // than a pipe buffer holds (64 KB on most systems) blocks on the write, and
    // reading only after `wait()` returns means it never returns — the hook
    // would look like it timed out rather than like it printed a lot.
    let out_task = child.stdout.take().map(|p| tokio::spawn(drain(p)));
    let err_task = child.stderr.take().map(|p| tokio::spawn(drain(p)));
    let sleep = tokio::time::sleep(Duration::from_secs(hook.timeout_secs));
    tokio::pin!(sleep);

    let status = loop {
        tokio::select! {
            done = child.wait() => break done,
            _ = &mut sleep => {
                let _ = child.kill().await;
                let _ = child.wait().await;
                return RunResult::Timeout { secs: hook.timeout_secs };
            }
            _ = poll_interrupt(&env.interrupt) => {
                let _ = child.kill().await;
                let _ = child.wait().await;
                return RunResult::Interrupted;
            }
        }
    };

    let stdout = collect(out_task).await;
    let stderr = collect(err_task).await;
    match status {
        Ok(s) => RunResult::Exited {
            // A signal death has no exit code. Reporting -1 keeps it out of the
            // `0` and `2` paths, which is the only thing that matters here.
            code: s.code().unwrap_or(-1),
            stdout,
            stderr,
        },
        Err(e) => RunResult::SpawnFailed {
            message: format!("hook failed: {e}"),
        },
    }
}

fn build_command(hook: &ResolvedHook, env: &RunEnv) -> Command {
    // `args` present means argv: the config author split the arguments
    // themselves, and re-joining them into a shell string would re-introduce
    // every quoting bug they avoided by splitting.
    let mut cmd = if hook.args.is_empty() {
        let (shell, flag) = if cfg!(target_os = "windows") {
            ("cmd", "/c")
        } else {
            ("sh", "-c")
        };
        let mut c = Command::new(shell);
        c.arg(flag).arg(&hook.command);
        c
    } else {
        let mut c = Command::new(&hook.command);
        c.args(&hook.args);
        c
    };
    cmd.env("PATH", login_path())
        .env("CLAUDE_PROJECT_DIR", &env.project_dir)
        .env("CLAUDINIO_PROJECT_DIR", &env.project_dir)
        .env("CLAUDINIO_HOOK_EVENT", hook.event.as_str())
        .env("CLAUDINIO_SESSION_ID", &env.session_id)
        .current_dir(&env.project_dir);
    if let Some(root) = &hook.plugin_root {
        cmd.env("CLAUDE_PLUGIN_ROOT", root)
            .env("CLAUDINIO_PLUGIN_ROOT", root);
    }
    cmd
}

async fn drain<R: tokio::io::AsyncRead + Unpin>(mut pipe: R) -> String {
    use tokio::io::AsyncReadExt;
    let mut buf = Vec::new();
    let _ = pipe.read_to_end(&mut buf).await;
    cap(&buf)
}

async fn collect(task: Option<tokio::task::JoinHandle<String>>) -> String {
    match task {
        Some(t) => t.await.unwrap_or_default(),
        None => String::new(),
    }
}

fn cap(buf: &[u8]) -> String {
    let text = String::from_utf8_lossy(buf);
    if text.len() <= MAX_OUTPUT_BYTES {
        return text.into_owned();
    }
    let mut cut = MAX_OUTPUT_BYTES;
    while cut > 0 && !text.is_char_boundary(cut) {
        cut -= 1;
    }
    format!("{}\n… [truncated]", &text[..cut])
}

async fn poll_interrupt(flag: &Option<Arc<AtomicBool>>) {
    let Some(f) = flag else {
        std::future::pending::<()>().await;
        return;
    };
    loop {
        if f.load(Ordering::SeqCst) {
            return;
        }
        tokio::time::sleep(Duration::from_millis(200)).await;
    }
}

/// Run a whole batch concurrently, preserving the resolved order in the result.
///
/// Order matters after the fact, not during: injected context is concatenated in
/// it, and a stable order is what keeps the prompt prefix cacheable between runs.
pub async fn run_batch(hooks: &[&ResolvedHook], input: &Value, env: &RunEnv) -> Vec<HookRun> {
    if hooks.is_empty() {
        return Vec::new();
    }
    let sem = Arc::new(tokio::sync::Semaphore::new(MAX_CONCURRENT));
    let mut tasks = Vec::with_capacity(hooks.len());
    for hook in hooks {
        let hook = (*hook).clone();
        let input = input.clone();
        let env = env.clone();
        let sem = sem.clone();
        tasks.push(tokio::spawn(async move {
            let _permit = sem.acquire().await;
            run_one(&hook, &input, &env).await
        }));
    }
    let mut out = Vec::with_capacity(tasks.len());
    for (ix, t) in tasks.into_iter().enumerate() {
        match t.await {
            Ok(run) => out.push(run),
            // A panicking task must not take the session with it.
            Err(e) => out.push(HookRun {
                hook: hooks[ix].clone(),
                effect: interpret(
                    hooks[ix].event,
                    &RunResult::SpawnFailed {
                        message: format!("hook task failed: {e}"),
                    },
                ),
                exit_code: None,
                stdout: String::new(),
                stderr: String::new(),
                duration_ms: 0,
            }),
        }
    }
    out
}

/// A synthetic payload for the settings panel's "Run now" button.
///
/// A wrong hook config "installs cleanly, runs on every prompt, and does
/// nothing". A button that prints the exit code is the cure, and it needs an
/// input that looks real enough for the hook to take its normal path.
pub fn probe_input(event: HookEvent, session_id: &str, cwd: &Path) -> Value {
    let base = super::payload::HookBase::new(session_id, &cwd.join("transcript.jsonl"), cwd);
    match event {
        HookEvent::PreToolUse => {
            super::payload::pre_tool_use(&base, "bash", &serde_json::json!({"command": "echo hi"}))
        }
        HookEvent::PostToolUse => super::payload::post_tool_use(
            &base,
            "bash",
            &serde_json::json!({"command": "echo hi"}),
            &super::payload::tool_response(true, "hi", MAX_OUTPUT_BYTES),
        ),
        HookEvent::UserPromptSubmit => {
            super::payload::user_prompt_submit(&base, "a test prompt from the settings panel")
        }
        HookEvent::Notification => {
            super::payload::notification(&base, "a test notification from the settings panel")
        }
        HookEvent::Stop => super::payload::stop(&base, false),
        HookEvent::SubagentStop => super::payload::subagent_stop(&base, false, "probe"),
        HookEvent::PreCompact => {
            super::payload::pre_compact(&base, super::payload::CompactTrigger::Manual, "")
        }
        HookEvent::SessionStart => {
            super::payload::session_start(&base, super::payload::SessionStartSource::Startup)
        }
        HookEvent::SessionEnd => {
            super::payload::session_end(&base, super::payload::SessionEndReason::Other)
        }
    }
}

#[cfg(test)]
mod tests {
    use super::super::discovery::HookSource;
    use super::*;
    use std::path::PathBuf;

    fn tmp(name: &str) -> PathBuf {
        let p = std::env::temp_dir().join(format!("cc-hook-run-{name}-{}", std::process::id()));
        std::fs::remove_dir_all(&p).ok();
        std::fs::create_dir_all(&p).unwrap();
        p
    }

    #[cfg(unix)]
    fn script(dir: &Path, name: &str, body: &str) -> String {
        use std::os::unix::fs::PermissionsExt;
        let p = dir.join(name);
        std::fs::write(&p, body).unwrap();
        std::fs::set_permissions(&p, std::fs::Permissions::from_mode(0o755)).unwrap();
        p.display().to_string()
    }

    fn hook(event: HookEvent, command: &str, args: &[&str]) -> ResolvedHook {
        ResolvedHook {
            event,
            matcher: None,
            command: command.into(),
            args: args.iter().map(|s| s.to_string()).collect(),
            timeout_secs: 10,
            status_message: None,
            source: HookSource::AppConfig,
            plugin_root: None,
            also_from: Vec::new(),
            order: (0, 0, 0),
        }
    }

    fn env(dir: &Path) -> RunEnv {
        RunEnv {
            project_dir: dir.display().to_string(),
            session_id: "s-1".into(),
            interrupt: None,
        }
    }

    #[cfg(unix)]
    #[tokio::test]
    async fn a_hook_that_prints_json_has_its_context_captured() {
        let d = tmp("json");
        let s = script(
            &d,
            "h.sh",
            "#!/bin/sh\necho '{\"hookSpecificOutput\":{\"additionalContext\":\"hello\"}}'\n",
        );
        let run = run_one(
            &hook(HookEvent::UserPromptSubmit, &s, &[]),
            &serde_json::json!({}),
            &env(&d),
        )
        .await;
        assert_eq!(run.exit_code, Some(0));
        assert_eq!(run.effect.additional_context.as_deref(), Some("hello"));
        std::fs::remove_dir_all(&d).ok();
    }

    #[cfg(unix)]
    #[tokio::test]
    async fn args_are_passed_as_argv_without_a_shell() {
        let d = tmp("argv");
        let s = script(&d, "h.sh", "#!/bin/sh\nprintf 'got:%s' \"$1\"\n");
        let run = run_one(
            &hook(HookEvent::UserPromptSubmit, &s, &["recall"]),
            &serde_json::json!({}),
            &env(&d),
        )
        .await;
        assert_eq!(run.stdout, "got:recall");
        std::fs::remove_dir_all(&d).ok();
    }

    #[cfg(unix)]
    #[tokio::test]
    async fn without_args_the_command_goes_through_the_shell() {
        let d = tmp("shell");
        let run = run_one(
            &hook(HookEvent::UserPromptSubmit, "echo one && echo two", &[]),
            &serde_json::json!({}),
            &env(&d),
        )
        .await;
        assert_eq!(run.stdout.trim(), "one\ntwo");
        std::fs::remove_dir_all(&d).ok();
    }

    #[cfg(unix)]
    #[tokio::test]
    async fn the_env_and_stdin_carry_the_contract() {
        let d = tmp("env");
        let s = script(
            &d,
            "h.sh",
            "#!/bin/sh\ncat\nprintf '|%s|%s|%s' \"$CLAUDE_PROJECT_DIR\" \"$CLAUDE_PLUGIN_ROOT\" \"$CLAUDINIO_HOOK_EVENT\"\n",
        );
        let mut h = hook(HookEvent::UserPromptSubmit, &s, &[]);
        h.plugin_root = Some(d.join("plug"));
        let run = run_one(&h, &serde_json::json!({"prompt": "hi"}), &env(&d)).await;
        assert!(run.stdout.contains(r#""prompt":"hi""#), "{}", run.stdout);
        assert!(run.stdout.contains(&format!("|{}|", d.display())));
        assert!(run.stdout.contains("plug"));
        assert!(run.stdout.ends_with("|UserPromptSubmit"));
        std::fs::remove_dir_all(&d).ok();
    }

    #[cfg(unix)]
    #[tokio::test]
    async fn a_hook_that_exceeds_its_timeout_is_killed_and_reported_non_blocking() {
        let d = tmp("timeout");
        let s = script(&d, "h.sh", "#!/bin/sh\nsleep 30\n");
        let mut h = hook(HookEvent::PreToolUse, &s, &[]);
        h.timeout_secs = 1;
        let run = run_one(&h, &serde_json::json!({}), &env(&d)).await;
        assert_eq!(run.status(), HookStatus::Timeout);
        assert!(run.effect.blocking_reason.is_none());
        std::fs::remove_dir_all(&d).ok();
    }

    #[tokio::test]
    async fn a_missing_binary_is_a_non_blocking_error() {
        let d = tmp("missing");
        let run = run_one(
            &hook(
                HookEvent::PreToolUse,
                &d.join("nope-does-not-exist").display().to_string(),
                &["x"],
            ),
            &serde_json::json!({}),
            &env(&d),
        )
        .await;
        assert_eq!(run.status(), HookStatus::Error);
        assert!(run.effect.blocking_reason.is_none());
        std::fs::remove_dir_all(&d).ok();
    }

    #[cfg(unix)]
    #[tokio::test]
    async fn hooks_run_in_parallel_and_keep_their_order() {
        let d = tmp("parallel");
        let s = script(&d, "h.sh", "#!/bin/sh\nsleep 0.4\nprintf '%s' \"$1\"\n");
        let hooks: Vec<ResolvedHook> = ["a", "b", "c"]
            .iter()
            .map(|n| hook(HookEvent::Stop, &s, &[n]))
            .collect();
        let refs: Vec<&ResolvedHook> = hooks.iter().collect();
        let started = std::time::Instant::now();
        let runs = run_batch(&refs, &serde_json::json!({}), &env(&d)).await;
        assert!(
            started.elapsed() < Duration::from_millis(1000),
            "three 0.4s hooks ran sequentially"
        );
        assert_eq!(
            runs.iter().map(|r| r.stdout.as_str()).collect::<Vec<_>>(),
            vec!["a", "b", "c"]
        );
        std::fs::remove_dir_all(&d).ok();
    }

    #[cfg(unix)]
    #[tokio::test]
    async fn output_is_capped() {
        let d = tmp("cap");
        let s = script(&d, "h.sh", "#!/bin/sh\nyes x | head -c 300000\n");
        let run = run_one(
            &hook(HookEvent::Stop, &s, &[]),
            &serde_json::json!({}),
            &env(&d),
        )
        .await;
        assert!(run.stdout.len() < MAX_OUTPUT_BYTES + 100);
        assert!(run.stdout.ends_with("[truncated]"));
        std::fs::remove_dir_all(&d).ok();
    }

    #[cfg(unix)]
    #[tokio::test]
    async fn an_interrupt_kills_a_running_hook_without_blocking_the_tool() {
        let d = tmp("interrupt");
        let s = script(&d, "h.sh", "#!/bin/sh\nsleep 30\n");
        let flag = Arc::new(AtomicBool::new(true));
        let mut e = env(&d);
        e.interrupt = Some(flag);
        let run = run_one(
            &hook(HookEvent::PreToolUse, &s, &[]),
            &serde_json::json!({}),
            &e,
        )
        .await;
        assert_eq!(run.status(), HookStatus::Error);
        assert!(run.effect.blocking_reason.is_none());
        std::fs::remove_dir_all(&d).ok();
    }

    #[test]
    fn every_event_has_a_probe_payload_naming_itself() {
        for e in HookEvent::ALL {
            let v = probe_input(*e, "s", Path::new("/ws"));
            assert_eq!(v["hook_event_name"], e.as_str());
        }
    }
}
