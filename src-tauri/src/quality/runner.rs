//! Running the project's own commands.
//!
//! Deliberately not the `bash` tool: that one truncates at 100 KiB and kills
//! at 30 seconds, which is right for an agent poking around and wrong for a
//! coverage run. Here the full output goes to a log file outside the
//! workspace, only a bounded tail reaches the model, and the timeout is
//! per-layer and configurable.
//!
//! The command strings come from detection or from `.claudinio.json` — never
//! from the model's tool arguments. That is what makes `run_quality` safe to
//! auto-approve.

use std::path::{Path, PathBuf};
use std::process::Stdio;
use std::sync::Arc;
use std::sync::atomic::{AtomicBool, Ordering};
use std::time::Duration;
use tokio::process::Command;

use super::config::QualityConfig;
use super::diff::{ChangedLines, total_lines};
use super::parsers::coverage::diff_coverage;
use super::parsers::{
    TestSummary, parse_cargo_test, parse_jest_json, parse_lcov, parse_vitest_json,
};
use super::profile::{StackProfile, TestParser};
use super::{Layer, LayerResult, LayerStatus, truncate_chars};
use crate::procutil::no_window_tokio;

/// How much of a command's output goes into the report. The rest stays in the
/// log file: the model does not need 60k lines of compiler output to learn
/// that three tests failed.
const MAX_INLINE_OUTPUT_CHARS: usize = 8_000;
/// A tooling probe (`cargo llvm-cov --version`) either answers fast or is not
/// installed in a way we can use.
const PROBE_TIMEOUT_SECS: u64 = 20;

pub struct CommandOutcome {
    pub exit_code: Option<i32>,
    pub output: String,
    pub timed_out: bool,
    pub interrupted: bool,
    pub log_path: Option<PathBuf>,
    pub spawn_error: Option<String>,
}

impl CommandOutcome {
    pub fn success(&self) -> bool {
        self.exit_code == Some(0) && !self.timed_out && self.spawn_error.is_none()
    }
}

/// Scratch space for one run's artifacts and logs. Kept outside the workspace
/// so the harness never adds noise to the user's repo (or trips their
/// .gitignore), following the same reasoning that moved the code index into
/// app data.
pub fn artifact_dir(workspace_root: &Path, label: &str) -> PathBuf {
    let hash = xxhash_rust::xxh3::xxh3_64(workspace_root.to_string_lossy().as_bytes());
    let dir = std::env::temp_dir()
        .join("claudinio-quality")
        .join(format!("{hash:016x}"))
        .join(label);
    // A stale artifact from a previous run would be read as this run's result.
    std::fs::remove_dir_all(&dir).ok();
    std::fs::create_dir_all(&dir).ok();
    dir
}

/// Run one shell command to completion, streaming its output to a log file.
pub async fn run_command(
    command: &str,
    cwd: &Path,
    timeout_secs: u64,
    log_path: Option<&Path>,
    interrupt: Option<&Arc<AtomicBool>>,
) -> CommandOutcome {
    let (shell, flag) = if cfg!(target_os = "windows") {
        ("cmd", "/c")
    } else {
        ("sh", "-c")
    };

    let mut cmd = Command::new(shell);
    cmd.arg(flag)
        .arg(command)
        // A GUI app inherits a minimal PATH, so `cargo`/`pnpm` would not
        // resolve without the login shell's PATH (shared with the bash tool
        // so both see exactly the same toolchain).
        .env("PATH", crate::agent::tools::bash::login_path())
        // Test runners colour their output when they think they have a TTY;
        // escape codes make the parsers and the log unreadable.
        .env("NO_COLOR", "1")
        .env("CI", "1")
        .current_dir(cwd)
        .stdin(Stdio::null())
        .stdout(Stdio::piped())
        .stderr(Stdio::piped())
        .kill_on_drop(true);
    no_window_tokio(&mut cmd);

    let mut child = match cmd.spawn() {
        Ok(c) => c,
        Err(e) => {
            return CommandOutcome {
                exit_code: None,
                output: String::new(),
                timed_out: false,
                interrupted: false,
                log_path: None,
                spawn_error: Some(format!("failed to start `{command}`: {e}")),
            };
        }
    };

    let mut stdout = child.stdout.take();
    let mut stderr = child.stderr.take();

    let timeout = tokio::time::sleep(Duration::from_secs(timeout_secs));
    tokio::pin!(timeout);

    let mut timed_out = false;
    let mut interrupted = false;
    let exit_code = tokio::select! {
        status = child.wait() => status.ok().and_then(|s| s.code()),
        _ = &mut timeout => {
            timed_out = true;
            let _ = child.kill().await;
            let _ = child.wait().await;
            None
        }
        _ = poll_interrupt(interrupt) => {
            interrupted = true;
            let _ = child.kill().await;
            let _ = child.wait().await;
            None
        }
    };

    let mut output = read_pipe(&mut stdout).await;
    let err_text = read_pipe(&mut stderr).await;
    if !err_text.is_empty() {
        if !output.is_empty() {
            output.push('\n');
        }
        output.push_str(&err_text);
    }

    let written_log = log_path.and_then(|p| {
        if let Some(dir) = p.parent() {
            let _ = std::fs::create_dir_all(dir);
        }
        std::fs::write(p, &output).ok().map(|_| p.to_path_buf())
    });

    CommandOutcome {
        exit_code,
        output: truncate_chars(&output, MAX_INLINE_OUTPUT_CHARS),
        timed_out,
        interrupted,
        log_path: written_log,
        spawn_error: None,
    }
}

async fn read_pipe<R: tokio::io::AsyncRead + Unpin>(pipe: &mut Option<R>) -> String {
    use tokio::io::AsyncReadExt;
    let Some(p) = pipe.as_mut() else {
        return String::new();
    };
    let mut buf = Vec::new();
    let _ = p.read_to_end(&mut buf).await;
    String::from_utf8_lossy(&buf).to_string()
}

async fn poll_interrupt(interrupt: Option<&Arc<AtomicBool>>) {
    let Some(flag) = interrupt else {
        std::future::pending::<()>().await;
        unreachable!()
    };
    loop {
        tokio::time::sleep(Duration::from_millis(250)).await;
        if flag.load(Ordering::SeqCst) {
            return;
        }
    }
}

/// Run a stack's test suite and score it.
pub async fn run_tests(
    stack: &StackProfile,
    cfg: &QualityConfig,
    interrupt: Option<&Arc<AtomicBool>>,
) -> LayerResult {
    let dir = artifact_dir(&stack.root, &format!("tests-{}", stack.name));
    let command = stack
        .test_cmd
        .replace("{artifact_dir}", &dir.to_string_lossy());
    let log = dir.join("output.log");

    let outcome = run_command(
        &command,
        &stack.root,
        cfg.test_timeout_secs,
        Some(&log),
        interrupt,
    )
    .await;

    if let Some(result) = infrastructure_failure(Layer::Tests, stack, &outcome, &command) {
        return result;
    }

    let summary = parse_test_output(stack, &dir, &outcome.output);

    // Two independent signals, and disagreement resolves toward failure: a
    // suite that exits non-zero has failed even if its report says otherwise
    // (a crashed runner writes a truncated or stale report).
    let green = outcome.success() && summary.failed == 0;
    let mut detail = String::new();
    if !green {
        if !summary.failures.is_empty() {
            detail.push_str("Failing tests:\n");
            for f in &summary.failures {
                detail.push_str(&format!("  - {f}\n"));
            }
            detail.push('\n');
        }
        detail.push_str(&outcome.output);
    }

    LayerResult {
        layer: Layer::Tests,
        status: if green {
            LayerStatus::Pass
        } else {
            LayerStatus::Fail
        },
        stack: stack.name.clone(),
        summary: if green {
            summary.headline()
        } else {
            format!("{} (exit {:?})", summary.headline(), outcome.exit_code)
        },
        detail: truncate_chars(&detail, MAX_INLINE_OUTPUT_CHARS),
        exit_code: outcome.exit_code,
        metrics: serde_json::json!({
            "passed": summary.passed,
            "failed": summary.failed,
            "skipped": summary.skipped,
            "parsed": summary.parsed,
        }),
        log_path: outcome.log_path.map(|p| p.to_string_lossy().to_string()),
    }
}

fn parse_test_output(stack: &StackProfile, dir: &Path, stdout: &str) -> TestSummary {
    let artifact = stack
        .test_artifact
        .as_ref()
        .and_then(|name| std::fs::read_to_string(dir.join(name)).ok());
    match stack.test_parser {
        TestParser::CargoTest => parse_cargo_test(stdout),
        TestParser::VitestJson => artifact
            .as_deref()
            .map(parse_vitest_json)
            .unwrap_or_default(),
        TestParser::JestJson => artifact.as_deref().map(parse_jest_json).unwrap_or_default(),
        TestParser::ExitCodeOnly => TestSummary::default(),
    }
}

/// Run coverage and score it against the lines this run changed.
pub async fn run_coverage(
    stack: &StackProfile,
    cfg: &QualityConfig,
    changed: &ChangedLines,
    interrupt: Option<&Arc<AtomicBool>>,
) -> LayerResult {
    let Some(template) = stack.coverage_cmd.clone() else {
        return unavailable(
            Layer::Coverage,
            stack,
            "no coverage command is known for this stack; set quality.coverage_cmd in \
             .claudinio.json (it must write lcov to {artifact_dir}/lcov.info)",
        );
    };

    // Nothing changed means nothing to require. Skipping the run also saves
    // the user a multi-minute instrumented build for no information.
    if total_lines(changed) == 0 {
        return LayerResult {
            layer: Layer::Coverage,
            status: LayerStatus::Pass,
            stack: stack.name.clone(),
            summary: "no changed lines to cover".into(),
            detail: String::new(),
            exit_code: None,
            metrics: serde_json::json!({"covered": 0, "total": 0, "pct": 100.0}),
            log_path: None,
        };
    }

    if let Some(probe) = stack.coverage_probe.as_deref() {
        let probed = run_command(probe, &stack.root, PROBE_TIMEOUT_SECS, None, interrupt).await;
        if !probed.success() {
            return unavailable(
                Layer::Coverage,
                stack,
                "coverage tooling is not installed (try `cargo install cargo-llvm-cov`); \
                 coverage was not measured",
            );
        }
    }

    let dir = artifact_dir(&stack.root, &format!("coverage-{}", stack.name));
    let command = template.replace("{artifact_dir}", &dir.to_string_lossy());
    let log = dir.join("output.log");
    let outcome = run_command(
        &command,
        &stack.root,
        cfg.coverage_timeout_secs,
        Some(&log),
        interrupt,
    )
    .await;

    if let Some(result) = infrastructure_failure(Layer::Coverage, stack, &outcome, &command) {
        return result;
    }

    let lcov_path = dir.join(&stack.coverage_lcov);
    let Ok(lcov_text) = std::fs::read_to_string(&lcov_path) else {
        return unavailable(
            Layer::Coverage,
            stack,
            &format!(
                "the coverage command produced no lcov report at {}; check quality.coverage_cmd",
                lcov_path.display()
            ),
        );
    };

    let lcov = parse_lcov(&lcov_text, &stack.root);
    let summary = diff_coverage(&lcov, changed);
    let pct = summary.pct();
    let green = pct + f64::EPSILON >= cfg.diff_coverage_threshold;

    let mut detail = String::new();
    if !green {
        detail.push_str(&format!(
            "{} changed line(s) across {} file(s) are not executed by any test:\n",
            summary.total - summary.covered,
            summary.uncovered_files
        ));
        for sample in &summary.uncovered_samples {
            detail.push_str(&format!("  - {sample}\n"));
        }
        detail.push_str(
            "\nEither add tests that exercise these lines, or delete the code they cover if \
             nothing needs it.\n",
        );
    }

    LayerResult {
        layer: Layer::Coverage,
        status: if green {
            LayerStatus::Pass
        } else {
            LayerStatus::Fail
        },
        stack: stack.name.clone(),
        summary: format!(
            "{} (threshold {:.1}%)",
            summary.headline(),
            cfg.diff_coverage_threshold
        ),
        detail,
        exit_code: outcome.exit_code,
        metrics: serde_json::json!({
            "covered": summary.covered,
            "total": summary.total,
            "pct": pct,
            "threshold": cfg.diff_coverage_threshold,
            "uncovered_files": summary.uncovered_files,
        }),
        log_path: outcome.log_path.map(|p| p.to_string_lossy().to_string()),
    }
}

/// Distinguish "the check ran and says no" from "the check never ran".
/// Reporting a spawn failure or a timeout as `Fail` would tell the model to go
/// fix tests that were never executed.
fn infrastructure_failure(
    layer: Layer,
    stack: &StackProfile,
    outcome: &CommandOutcome,
    command: &str,
) -> Option<LayerResult> {
    if let Some(err) = &outcome.spawn_error {
        return Some(unavailable(layer, stack, err));
    }
    if outcome.interrupted {
        return Some(unavailable(layer, stack, "interrupted by the user"));
    }
    if outcome.timed_out {
        return Some(unavailable(
            layer,
            stack,
            &format!(
                "`{command}` timed out; raise quality.{}_timeout_secs in .claudinio.json if the \
                 suite legitimately takes longer",
                layer.as_str()
            ),
        ));
    }
    None
}

fn unavailable(layer: Layer, stack: &StackProfile, why: &str) -> LayerResult {
    LayerResult {
        layer,
        status: LayerStatus::Unavailable,
        stack: stack.name.clone(),
        summary: why.to_string(),
        detail: String::new(),
        exit_code: None,
        metrics: serde_json::Value::Null,
        log_path: None,
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn stack(root: &Path, test_cmd: &str) -> StackProfile {
        StackProfile {
            name: "t".into(),
            root: root.to_path_buf(),
            test_cmd: test_cmd.into(),
            test_parser: TestParser::ExitCodeOnly,
            test_artifact: None,
            coverage_cmd: None,
            coverage_probe: None,
            coverage_lcov: "lcov.info".into(),
        }
    }

    fn tmp(name: &str) -> PathBuf {
        let p = std::env::temp_dir().join(format!("cq-runner-{name}-{}", std::process::id()));
        std::fs::create_dir_all(&p).unwrap();
        p
    }

    #[tokio::test]
    async fn a_passing_command_is_a_green_layer() {
        let root = tmp("green");
        let r = run_tests(&stack(&root, "exit 0"), &QualityConfig::default(), None).await;
        assert_eq!(r.status, LayerStatus::Pass);
        assert_eq!(r.exit_code, Some(0));
        std::fs::remove_dir_all(&root).ok();
    }

    #[tokio::test]
    async fn a_nonzero_exit_fails_the_layer() {
        let root = tmp("red");
        let r = run_tests(
            &stack(&root, "echo boom; exit 3"),
            &QualityConfig::default(),
            None,
        )
        .await;
        assert_eq!(r.status, LayerStatus::Fail);
        assert_eq!(r.exit_code, Some(3));
        assert!(r.detail.contains("boom"), "{}", r.detail);
        std::fs::remove_dir_all(&root).ok();
    }

    #[tokio::test]
    async fn a_timeout_is_unavailable_not_failed() {
        // "the suite never ran" must never be reported as "the suite is red",
        // or the model goes off fixing tests that were never executed.
        let root = tmp("timeout");
        let cfg = QualityConfig {
            test_timeout_secs: 1,
            ..Default::default()
        };
        let r = run_tests(&stack(&root, "sleep 30"), &cfg, None).await;
        assert_eq!(r.status, LayerStatus::Unavailable);
        assert!(r.summary.contains("timed out"), "{}", r.summary);
        std::fs::remove_dir_all(&root).ok();
    }

    #[tokio::test]
    async fn output_is_written_to_a_log_outside_the_workspace() {
        let root = tmp("log");
        let r = run_tests(
            &stack(&root, "echo hello-log"),
            &QualityConfig::default(),
            None,
        )
        .await;
        let log = r.log_path.expect("log recorded");
        assert!(
            !log.starts_with(root.to_string_lossy().as_ref()),
            "logs must not land in the user's workspace: {log}"
        );
        assert!(std::fs::read_to_string(&log).unwrap().contains("hello-log"));
        std::fs::remove_dir_all(&root).ok();
    }

    #[tokio::test]
    async fn cargo_style_output_is_parsed_into_counts() {
        let root = tmp("counts");
        let mut s = stack(
            &root,
            "echo 'test result: ok. 4 passed; 0 failed; 0 ignored'",
        );
        s.test_parser = TestParser::CargoTest;
        let r = run_tests(&s, &QualityConfig::default(), None).await;
        assert_eq!(r.status, LayerStatus::Pass);
        assert_eq!(r.metrics["passed"], 4);
        std::fs::remove_dir_all(&root).ok();
    }

    #[tokio::test]
    async fn a_report_claiming_success_cannot_override_a_nonzero_exit() {
        let root = tmp("disagree");
        let mut s = stack(
            &root,
            "echo 'test result: ok. 1 passed; 0 failed; 0 ignored'; exit 101",
        );
        s.test_parser = TestParser::CargoTest;
        let r = run_tests(&s, &QualityConfig::default(), None).await;
        assert_eq!(r.status, LayerStatus::Fail, "exit code must win");
        std::fs::remove_dir_all(&root).ok();
    }

    #[tokio::test]
    async fn coverage_without_changed_lines_passes_without_running() {
        let root = tmp("nochange");
        let mut s = stack(&root, "true");
        // A command that would fail if it ever ran, proving we skipped it.
        s.coverage_cmd = Some("exit 9".into());
        let r = run_coverage(&s, &QualityConfig::default(), &ChangedLines::new(), None).await;
        assert_eq!(r.status, LayerStatus::Pass);
        assert_eq!(r.exit_code, None);
        std::fs::remove_dir_all(&root).ok();
    }

    #[tokio::test]
    async fn coverage_without_a_command_is_unavailable() {
        let root = tmp("nocov");
        let mut changed = ChangedLines::new();
        changed.insert(root.join("a.rs"), [1u32].into_iter().collect());
        let r = run_coverage(
            &stack(&root, "true"),
            &QualityConfig::default(),
            &changed,
            None,
        )
        .await;
        assert_eq!(r.status, LayerStatus::Unavailable);
        std::fs::remove_dir_all(&root).ok();
    }

    #[tokio::test]
    async fn coverage_scores_changed_lines_against_the_lcov_report() {
        let root = tmp("cov");
        let file = root.join("a.rs");
        std::fs::write(&file, "fn a() {}\n").unwrap();
        let mut s = stack(&root, "true");
        // Emit an lcov report where one changed line ran and one did not.
        s.coverage_cmd = Some(format!(
            "printf 'SF:{}\\nDA:1,1\\nDA:2,0\\nend_of_record\\n' > {{artifact_dir}}/lcov.info",
            file.display()
        ));
        let mut changed = ChangedLines::new();
        changed.insert(file, [1u32, 2].into_iter().collect());

        let cfg = QualityConfig {
            diff_coverage_threshold: 80.0,
            ..Default::default()
        };
        let r = run_coverage(&s, &cfg, &changed, None).await;
        assert_eq!(
            r.status,
            LayerStatus::Fail,
            "50% is under the 80% threshold"
        );
        assert_eq!(r.metrics["covered"], 1);
        assert_eq!(r.metrics["total"], 2);
        assert!(
            r.detail.contains(":2"),
            "must name the uncovered line: {}",
            r.detail
        );

        // Same run, threshold the change actually meets.
        let lenient = QualityConfig {
            diff_coverage_threshold: 50.0,
            ..Default::default()
        };
        let r2 = run_coverage(&s, &lenient, &changed, None).await;
        assert_eq!(r2.status, LayerStatus::Pass);
        std::fs::remove_dir_all(&root).ok();
    }

    #[tokio::test]
    async fn coverage_command_that_writes_no_report_is_unavailable() {
        let root = tmp("noreport");
        let mut s = stack(&root, "true");
        s.coverage_cmd = Some("true".into());
        let mut changed = ChangedLines::new();
        changed.insert(root.join("a.rs"), [1u32].into_iter().collect());
        let r = run_coverage(&s, &QualityConfig::default(), &changed, None).await;
        assert_eq!(r.status, LayerStatus::Unavailable);
        assert!(r.summary.contains("no lcov report"), "{}", r.summary);
        std::fs::remove_dir_all(&root).ok();
    }
}
