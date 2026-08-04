//! The quality harness: mechanical verification of the code the agent writes.
//!
//! The coding harness (tasks, golden goals, mode flips) governs *flow*; this
//! module governs *evidence*. Its whole reason to exist is that the golden
//! prompt used to say "run the checks" and nothing ever checked that the model
//! did — a golden goal could be marked done on the model's word alone.
//!
//! Everything here is deterministic: run the project's own commands, parse
//! their machine-readable output, compare against thresholds the user owns.
//! No layer asks a model whether the code is good. A [`QualityReport`] is only
//! usable as evidence while its [`digest`](evidence::workspace_digest) still
//! matches the worktree, so "run the tests, then edit, then mark done" is not
//! a path the model can take.
//!
//! Layering: like `agent/` and `code_intel/`, this module sits below the
//! command/IPC adapter layer and must never reach back up into it — enforced
//! by `architecture_tests` in `lib.rs`, which greps for the import.

pub mod config;
pub mod diff;
pub mod evidence;
pub mod parsers;
pub mod profile;
pub mod runner;
pub mod spec;

use serde::{Deserialize, Serialize};
use std::path::Path;
use std::sync::Arc;
use std::sync::atomic::AtomicBool;

pub use config::QualityConfig;
pub use profile::{ProjectProfile, StackProfile};

/// One verification layer. Each catches a class of defect the others miss:
/// tests catch broken logic, coverage catches code no test ever touched,
/// mutation catches tests that execute code without actually checking it, and
/// Gherkin checks the whole thing against what a human actually asked for —
/// the one input that did not come out of a model. Trend metrics are the
/// remaining phase (see the design doc).
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "lowercase")]
pub enum Layer {
    Tests,
    Coverage,
    Mutation,
    Gherkin,
}

impl Layer {
    pub fn as_str(&self) -> &'static str {
        match self {
            Layer::Tests => "tests",
            Layer::Coverage => "coverage",
            Layer::Mutation => "mutation",
            Layer::Gherkin => "gherkin",
        }
    }

    pub fn parse(s: &str) -> Option<Layer> {
        match s.trim().to_ascii_lowercase().as_str() {
            "tests" | "test" => Some(Layer::Tests),
            "coverage" | "cov" => Some(Layer::Coverage),
            "mutation" | "mutants" => Some(Layer::Mutation),
            "gherkin" | "spec" | "specs" | "bdd" => Some(Layer::Gherkin),
            _ => None,
        }
    }
}

/// How a layer ended.
///
/// `Unavailable` is deliberately distinct from `Fail`: a missing
/// `cargo-llvm-cov` means we learned nothing, which must never read as a green
/// check, but also must not block a user who never installed it. It is
/// reported honestly and excluded from the verdict.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "lowercase")]
pub enum LayerStatus {
    Pass,
    Fail,
    Unavailable,
}

impl LayerStatus {
    pub fn as_str(&self) -> &'static str {
        match self {
            LayerStatus::Pass => "pass",
            LayerStatus::Fail => "fail",
            LayerStatus::Unavailable => "unavailable",
        }
    }
}

/// The outcome of one layer against one stack (a workspace can have several —
/// this repo is a Rust crate under `src-tauri/` plus a pnpm/vitest frontend).
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct LayerResult {
    pub layer: Layer,
    pub status: LayerStatus,
    /// Which stack produced this, e.g. "rust" or "js" — several results per
    /// layer are normal.
    pub stack: String,
    /// One line, safe to show the model and the user.
    pub summary: String,
    /// Failure detail (test names, uncovered files). Empty when green.
    #[serde(default)]
    pub detail: String,
    #[serde(default)]
    pub exit_code: Option<i32>,
    /// Layer-specific numbers, e.g. `{"passed":12,"failed":0}` or
    /// `{"covered":30,"total":40,"pct":75.0}`.
    #[serde(default)]
    pub metrics: serde_json::Value,
    /// Full command log on disk, for the user to open when a summary is not
    /// enough. Outside the workspace — never pollutes the user's repo.
    #[serde(default)]
    pub log_path: Option<String>,
}

/// Why the gate refused. Phrased as expected-vs-actual so the message handed
/// back to the model is actionable rather than just "quality failed".
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct GateFailure {
    pub layer: Layer,
    pub stack: String,
    pub expected: String,
    pub actual: String,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct GateVerdict {
    pub pass: bool,
    #[serde(default)]
    pub failures: Vec<GateFailure>,
}

/// A complete verification run. Valid as gate evidence only while `digest`
/// still equals the worktree's current digest.
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct QualityReport {
    pub ts: u64,
    pub digest: String,
    #[serde(default)]
    pub base_commit: Option<String>,
    pub layers: Vec<LayerResult>,
    pub verdict: GateVerdict,
}

impl QualityReport {
    /// Compact rendering for the model and for the timeline. Deliberately
    /// small: the full logs live on disk, and a 100k-line coverage dump in the
    /// context window helps nobody.
    pub fn summary_text(&self) -> String {
        let mut out = String::new();
        for r in &self.layers {
            let mark = match r.status {
                LayerStatus::Pass => "PASS",
                LayerStatus::Fail => "FAIL",
                LayerStatus::Unavailable => "n/a ",
            };
            out.push_str(&format!(
                "[{mark}] {} ({}): {}\n",
                r.layer.as_str(),
                r.stack,
                r.summary
            ));
        }
        if self.verdict.pass {
            out.push_str("VERDICT: pass\n");
        } else {
            out.push_str("VERDICT: fail\n");
            for f in &self.verdict.failures {
                out.push_str(&format!(
                    "  - {} ({}): expected {}, got {}\n",
                    f.layer.as_str(),
                    f.stack,
                    f.expected,
                    f.actual
                ));
            }
        }
        out
    }

    /// Failure detail for the message pushed back into the loop when the gate
    /// blocks a finish. Bounded so a thousand failing tests cannot flood the
    /// context.
    pub fn failure_detail(&self, max_chars: usize) -> String {
        let mut out = String::new();
        for r in &self.layers {
            if r.status == LayerStatus::Fail && !r.detail.is_empty() {
                out.push_str(&format!("--- {} ({}) ---\n", r.layer.as_str(), r.stack));
                out.push_str(&r.detail);
                out.push('\n');
            }
        }
        truncate_chars(&out, max_chars)
    }
}

/// Truncate on a char boundary, marking that we did.
pub(crate) fn truncate_chars(s: &str, max: usize) -> String {
    if s.chars().count() <= max {
        return s.to_string();
    }
    let cut: String = s.chars().take(max).collect();
    format!("{cut}\n…(truncated)")
}

/// Turn layer results into a verdict, given which layers the user chose to
/// enforce. Layers that ran but are not enforced are reported and ignored;
/// `Unavailable` never fails a gate (we learned nothing, we do not pretend
/// otherwise in either direction).
pub fn evaluate_gate(layers: &[LayerResult], enforced: &[Layer]) -> GateVerdict {
    let mut failures = Vec::new();
    for r in layers {
        if !enforced.contains(&r.layer) || r.status != LayerStatus::Fail {
            continue;
        }
        failures.push(GateFailure {
            layer: r.layer,
            stack: r.stack.clone(),
            expected: match r.layer {
                Layer::Tests => "all tests passing".into(),
                Layer::Coverage => "diff coverage at or above threshold".into(),
                Layer::Mutation => "mutation score at or above threshold".into(),
                Layer::Gherkin => "every scenario in the spec passing".into(),
            },
            actual: r.summary.clone(),
        });
    }
    GateVerdict {
        pass: failures.is_empty(),
        failures,
    }
}

/// Run the requested layers over every detected stack and produce a report.
///
/// Never returns `Err` for a failing check — a red suite is a *result*, not an
/// error. `Err` is reserved for "the harness itself could not run", e.g. the
/// workspace has no recognizable project.
pub async fn run_layers(
    workspace_root: &Path,
    cfg: &QualityConfig,
    requested: &[Layer],
    base_commit: Option<&str>,
    interrupt: Option<&Arc<AtomicBool>>,
) -> Result<QualityReport, String> {
    let profile = profile::detect(workspace_root, cfg);
    if profile.stacks.is_empty() {
        return Err(
            "no test-capable project detected in this workspace (looked for Cargo.toml and \
             package.json with vitest/jest). Set \"quality\": {\"test_cmd\": \"...\"} in \
             .claudinio.json to tell the harness how to run this project's tests."
                .into(),
        );
    }

    // Coverage and mutation are both scoped to the lines this run actually
    // changed, so they report on the agent's work rather than the repo's
    // history — and so mutation stays affordable at all.
    let scoped = requested.contains(&Layer::Coverage) || requested.contains(&Layer::Mutation);
    let changed = if scoped {
        diff::changed_lines(workspace_root, base_commit)
    } else {
        None
    };
    // cargo-mutants scopes itself from a patch file rather than a line list.
    let patch = if requested.contains(&Layer::Mutation) {
        diff::write_patch(workspace_root, base_commit)
    } else {
        None
    };

    // The specs are read once: they are the same for every stack, and reading
    // them is also how the layer reports "you have scenarios nothing runs".
    let features = if requested.contains(&Layer::Gherkin) {
        spec::load_features(workspace_root, cfg.features_dir.as_deref())
    } else {
        Vec::new()
    };

    let mut results: Vec<LayerResult> = Vec::new();
    for stack in &profile.stacks {
        // Mutation depends on this: breaking code on top of a red suite tells
        // you nothing, and costs a rerun per mutant to find out.
        let mut tests_passed = true;
        if requested.contains(&Layer::Tests) {
            let result = runner::run_tests(stack, cfg, interrupt).await;
            tests_passed = result.status == LayerStatus::Pass;
            results.push(result);
        }
        if requested.contains(&Layer::Coverage) {
            results.push(runner::run_coverage(stack, cfg, changed.as_ref(), interrupt).await);
        }
        if requested.contains(&Layer::Gherkin) {
            results.push(runner::run_gherkin(stack, cfg, &features, interrupt).await);
        }
        if requested.contains(&Layer::Mutation) {
            results.push(
                runner::run_mutation(
                    stack,
                    cfg,
                    changed.as_ref(),
                    patch.as_deref(),
                    tests_passed,
                    interrupt,
                )
                .await,
            );
        }
    }

    let verdict = evaluate_gate(&results, &cfg.enforced_layers);
    Ok(QualityReport {
        ts: now_ms(),
        digest: evidence::workspace_digest(workspace_root),
        base_commit: base_commit.map(|s| s.to_string()),
        layers: results,
        verdict,
    })
}

pub(crate) fn now_ms() -> u64 {
    std::time::SystemTime::now()
        .duration_since(std::time::UNIX_EPOCH)
        .map(|d| d.as_millis() as u64)
        .unwrap_or(0)
}

#[cfg(test)]
mod tests {
    use super::*;

    fn result(layer: Layer, status: LayerStatus) -> LayerResult {
        LayerResult {
            layer,
            status,
            stack: "rust".into(),
            summary: "s".into(),
            detail: String::new(),
            exit_code: None,
            metrics: serde_json::Value::Null,
            log_path: None,
        }
    }

    #[test]
    fn failing_enforced_layer_fails_the_gate() {
        let v = evaluate_gate(&[result(Layer::Tests, LayerStatus::Fail)], &[Layer::Tests]);
        assert!(!v.pass);
        assert_eq!(v.failures.len(), 1);
    }

    #[test]
    fn failing_unenforced_layer_is_reported_but_does_not_block() {
        let v = evaluate_gate(
            &[result(Layer::Coverage, LayerStatus::Fail)],
            &[Layer::Tests],
        );
        assert!(v.pass, "coverage is not enforced here, so it cannot block");
    }

    #[test]
    fn unavailable_tooling_never_fails_the_gate() {
        // A missing cargo-llvm-cov means we learned nothing — that must not
        // read as green, but it must not block the user either.
        let v = evaluate_gate(
            &[result(Layer::Coverage, LayerStatus::Unavailable)],
            &[Layer::Coverage],
        );
        assert!(v.pass);
    }

    #[test]
    fn green_run_passes() {
        let v = evaluate_gate(
            &[
                result(Layer::Tests, LayerStatus::Pass),
                result(Layer::Coverage, LayerStatus::Pass),
            ],
            &[Layer::Tests, Layer::Coverage],
        );
        assert!(v.pass);
        assert!(v.failures.is_empty());
    }

    #[test]
    fn summary_text_names_every_layer_and_the_verdict() {
        let report = QualityReport {
            ts: 0,
            digest: "d".into(),
            base_commit: None,
            layers: vec![result(Layer::Tests, LayerStatus::Fail)],
            verdict: evaluate_gate(&[result(Layer::Tests, LayerStatus::Fail)], &[Layer::Tests]),
        };
        let text = report.summary_text();
        assert!(text.contains("tests"), "{text}");
        assert!(text.contains("VERDICT: fail"), "{text}");
    }

    #[test]
    fn layer_parse_round_trips() {
        assert_eq!(Layer::parse("tests"), Some(Layer::Tests));
        assert_eq!(Layer::parse("Coverage"), Some(Layer::Coverage));
        assert_eq!(Layer::parse("mutation"), Some(Layer::Mutation));
        assert_eq!(Layer::parse("gherkin"), Some(Layer::Gherkin));
        // A name chosen so it can never become a real layer and quietly
        // turn this assertion into a no-op.
        assert_eq!(Layer::parse("not-a-real-layer"), None);
    }

    #[test]
    fn truncate_marks_the_cut() {
        assert_eq!(truncate_chars("abc", 10), "abc");
        assert!(truncate_chars("abcdef", 3).starts_with("abc"));
        assert!(truncate_chars("abcdef", 3).contains("truncated"));
    }

    /// The claim the whole layer rests on, against the real tool: a suite with
    /// full line coverage but no assertions lets mutants survive.
    ///
    /// Skipped when cargo-mutants is not installed — this must not turn CI red
    /// on machines without it, and every scoring rule is unit-tested separately.
    #[tokio::test]
    async fn mutation_catches_a_test_that_covers_without_checking() {
        let root = std::env::temp_dir().join(format!("cq-mut-e2e-{}", std::process::id()));
        std::fs::remove_dir_all(&root).ok();
        std::fs::create_dir_all(root.join("src")).unwrap();

        let sh = |cmd: &str| {
            let mut c = std::process::Command::new("sh");
            c.arg("-c").arg(cmd).current_dir(&root);
            crate::procutil::no_window(&mut c);
            c.output().map(|o| o.status.success()).unwrap_or(false)
        };
        if !sh("cargo mutants --version") {
            return; // tool absent; the unit tests still cover the scoring
        }

        std::fs::write(
            root.join("Cargo.toml"),
            "[package]\nname = \"mutdemo\"\nversion = \"0.1.0\"\nedition = \"2021\"\n[workspace]\n",
        )
        .unwrap();
        // Commit the scaffold, then add the code as a NEW file: that is the
        // real shape of an agent's work, and it gives the diff something to
        // scope to.
        if !sh(
            "git init -q && git config user.email t@t && git config user.name t \
                && git add -A && git commit -qm scaffold",
        ) {
            return; // git unavailable
        }
        std::fs::write(
            root.join("src/lib.rs"),
            // The test runs every line of `discount` and asserts almost
            // nothing — 100% coverage, zero verification.
            "pub fn discount(total: f64, is_member: bool) -> f64 {\n    \
             if is_member { total * 0.9 } else { total }\n}\n\n\
             #[cfg(test)]\nmod tests {\n    use super::*;\n    #[test]\n    \
             fn covers_without_checking() {\n        \
             assert!(discount(100.0, true) > 0.0);\n    }\n}\n",
        )
        .unwrap();

        let cfg = QualityConfig {
            enforced_layers: vec![Layer::Tests, Layer::Mutation],
            mutation_score_threshold: 60.0,
            mutation_timeout_secs: 600,
            ..Default::default()
        };
        // No base commit: not a git repo, so mutation runs unscoped over the
        // whole (tiny) crate.
        let report = run_layers(&root, &cfg, &[Layer::Tests, Layer::Mutation], None, None)
            .await
            .unwrap();

        let tests = report
            .layers
            .iter()
            .find(|l| l.layer == Layer::Tests)
            .unwrap();
        assert_eq!(tests.status, LayerStatus::Pass, "the weak suite is green");

        let mutation = report
            .layers
            .iter()
            .find(|l| l.layer == Layer::Mutation)
            .unwrap();
        assert_eq!(
            mutation.status,
            LayerStatus::Fail,
            "mutants must survive a suite that checks nothing: {}",
            mutation.summary
        );
        assert!(!report.verdict.pass);
        assert!(
            mutation.detail.contains("survived"),
            "the survivors must be named: {}",
            mutation.detail
        );

        std::fs::remove_dir_all(&root).ok();
    }

    #[test]
    fn a_workspace_with_no_project_says_how_to_fix_it() {
        let root = std::env::temp_dir().join(format!("cq-mod-empty-{}", std::process::id()));
        std::fs::create_dir_all(&root).unwrap();
        let err = futures::executor::block_on(run_layers(
            &root,
            &QualityConfig::default(),
            &[Layer::Tests],
            None,
            None,
        ))
        .unwrap_err();
        assert!(err.contains("test_cmd"), "{err}");
        std::fs::remove_dir_all(&root).ok();
    }

    /// End-to-end over a real git repository: commit a baseline, change a
    /// file, then run both layers. This is the seam the unit tests cannot
    /// prove on their own — `git diff` really feeding the lcov scorer, and the
    /// digest really pinning the report to this exact state of the files.
    #[tokio::test]
    async fn diff_coverage_scores_the_lines_this_run_changed() {
        let root = std::env::temp_dir().join(format!("cq-e2e-{}", std::process::id()));
        std::fs::remove_dir_all(&root).ok();
        std::fs::create_dir_all(&root).unwrap();

        let git = |args: &[&str]| {
            let mut c = std::process::Command::new("git");
            c.args(args).current_dir(&root);
            crate::procutil::no_window(&mut c);
            c.output().map(|o| o.status.success()).unwrap_or(false)
        };
        if !git(&["init", "-q"]) {
            return; // no git here; the unit tests still cover the parsing
        }
        git(&["config", "user.email", "t@t"]);
        git(&["config", "user.name", "t"]);
        std::fs::write(root.join("lib.rs"), "fn old() {}\n").unwrap();
        git(&["add", "-A"]);
        assert!(git(&["commit", "-q", "-m", "base"]));
        let base = evidence::git_head(&root).expect("HEAD after commit");

        // Two new lines: line 2 will be reported as executed, line 3 as not.
        std::fs::write(
            root.join("lib.rs"),
            "fn old() {}\nfn covered() {}\nfn ghost() {}\n",
        )
        .unwrap();

        let cfg = QualityConfig {
            test_cmd: Some("exit 0".into()),
            coverage_cmd: Some(format!(
                "printf 'SF:{}/lib.rs\\nDA:2,4\\nDA:3,0\\nend_of_record\\n' > {{artifact_dir}}/lcov.info",
                root.display()
            )),
            enforced_layers: vec![Layer::Tests, Layer::Coverage],
            diff_coverage_threshold: 80.0,
            ..Default::default()
        };

        let report = run_layers(
            &root,
            &cfg,
            &[Layer::Tests, Layer::Coverage],
            Some(&base),
            None,
        )
        .await
        .unwrap();

        let coverage = report
            .layers
            .iter()
            .find(|l| l.layer == Layer::Coverage)
            .expect("coverage layer ran");
        assert_eq!(
            coverage.metrics["total"], 2,
            "both new lines are executable"
        );
        assert_eq!(coverage.metrics["covered"], 1);
        assert_eq!(coverage.status, LayerStatus::Fail, "50% is under 80%");
        assert!(!report.verdict.pass);
        assert!(
            report
                .verdict
                .failures
                .iter()
                .any(|f| f.layer == Layer::Coverage),
            "the verdict must name the layer that blocked it"
        );

        // The report is pinned to this state of the files, and only this one.
        assert_eq!(report.digest, evidence::workspace_digest(&root));
        std::fs::write(root.join("lib.rs"), "fn old() {}\n// touched again\n").unwrap();
        assert_ne!(report.digest, evidence::workspace_digest(&root));

        std::fs::remove_dir_all(&root).ok();
    }
}
