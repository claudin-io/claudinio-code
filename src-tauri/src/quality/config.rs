//! Per-project quality settings, read from the `"quality"` object in
//! `<workspace_root>/.claudinio.json`.
//!
//! The thresholds and the commands belong to the user, not to the model: the
//! agent can propose changing them, but it edits the same file the user reads,
//! and the harness always re-reads from disk. That is also why `run_quality`
//! can be auto-approved — the command strings it executes never come from the
//! model's tool arguments.

use serde::{Deserialize, Serialize};
use std::path::Path;

use super::Layer;

/// Default share of changed lines that must be covered before the coverage
/// layer goes green.
pub const DEFAULT_DIFF_COVERAGE_THRESHOLD: f64 = 80.0;
/// Test suites get 10 minutes, coverage 15 — both far beyond the bash tool's
/// 30s, which is exactly why the quality runner does not go through it.
pub const DEFAULT_TEST_TIMEOUT_SECS: u64 = 600;
pub const DEFAULT_COVERAGE_TIMEOUT_SECS: u64 = 900;
/// Mutation reruns the suite once per mutant, so it is the slow one by an
/// order of magnitude. Half an hour is generous for a diff-scoped run and
/// still bounded enough that a runaway does not hang a session forever.
pub const DEFAULT_MUTATION_TIMEOUT_SECS: u64 = 1800;
/// A deliberately reachable bar. Perfect mutation scores are not the goal;
/// catching suites that verify nothing is.
pub const DEFAULT_MUTATION_SCORE_THRESHOLD: f64 = 60.0;

/// When a run must be verified before it is allowed to finish.
///
/// `CodeChange` is the default: any session that touched source gets verified
/// once, at the finish line. A harness that only acts when the user remembers
/// to tag a `<goal>` protects nobody by default, and the whole point is to earn
/// the right not to read the generated code.
///
/// `Goals` narrows it back to tagged goals only — the escape hatch for a repo
/// whose suite is too slow to sit through on every change.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum EnforceOn {
    Goals,
    CodeChange,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct QualityConfig {
    /// Master switch. False disables both the tool and the gate.
    #[serde(default = "default_true")]
    pub enabled: bool,
    /// What triggers the finish-line check. Tagged goals always trigger it;
    /// this widens the net beyond them.
    #[serde(default = "default_enforce_on")]
    pub enforce_on: EnforceOn,
    /// Which layers block a golden task from being marked done. Empty = the
    /// harness reports but never blocks (observation mode).
    #[serde(default = "default_enforced")]
    pub enforced_layers: Vec<Layer>,
    /// Override the detected test command. Applies to every stack, so it is
    /// the right knob for a project with one canonical `make test`.
    #[serde(default)]
    pub test_cmd: Option<String>,
    /// Override the detected coverage command. It MUST write an lcov file to
    /// the path the harness substitutes for `{lcov}`.
    #[serde(default)]
    pub coverage_cmd: Option<String>,
    #[serde(default = "default_diff_coverage_threshold")]
    pub diff_coverage_threshold: f64,
    /// Override the detected mutation command. Required for stacks the harness
    /// does not drive natively; it must write a `mutants.out` directory into
    /// `{artifact_dir}`, or an lcov-style mutation report — see the design doc.
    #[serde(default)]
    pub mutation_cmd: Option<String>,
    /// Share of viable mutants the tests must catch.
    #[serde(default = "default_mutation_score_threshold")]
    pub mutation_score_threshold: f64,
    /// Where the project's `.feature` specs live, relative to the workspace.
    /// Everything under it is human-owned: the agent cannot edit it.
    #[serde(default)]
    pub features_dir: Option<String>,
    /// Override the detected BDD runner command.
    #[serde(default)]
    pub gherkin_cmd: Option<String>,
    #[serde(default = "default_test_timeout")]
    pub test_timeout_secs: u64,
    #[serde(default = "default_coverage_timeout")]
    pub coverage_timeout_secs: u64,
    #[serde(default = "default_mutation_timeout")]
    pub mutation_timeout_secs: u64,
}

fn default_true() -> bool {
    true
}
fn default_enforce_on() -> EnforceOn {
    // Verify by default. The cost is one test run at the end of a session that
    // changed code — not per task, and never for a read-only or prose-only
    // session. Narrow it with "enforce_on": "goals" when a suite is too slow to
    // sit through on every change.
    EnforceOn::CodeChange
}
fn default_enforced() -> Vec<Layer> {
    // Tests only by default: coverage needs tooling the user may not have
    // installed, and a harness that blocks on day one gets switched off.
    vec![Layer::Tests]
}
fn default_diff_coverage_threshold() -> f64 {
    DEFAULT_DIFF_COVERAGE_THRESHOLD
}
fn default_test_timeout() -> u64 {
    DEFAULT_TEST_TIMEOUT_SECS
}
fn default_coverage_timeout() -> u64 {
    DEFAULT_COVERAGE_TIMEOUT_SECS
}
fn default_mutation_score_threshold() -> f64 {
    DEFAULT_MUTATION_SCORE_THRESHOLD
}
fn default_mutation_timeout() -> u64 {
    DEFAULT_MUTATION_TIMEOUT_SECS
}

impl Default for QualityConfig {
    fn default() -> Self {
        Self {
            enabled: true,
            enforce_on: default_enforce_on(),
            enforced_layers: default_enforced(),
            test_cmd: None,
            coverage_cmd: None,
            diff_coverage_threshold: DEFAULT_DIFF_COVERAGE_THRESHOLD,
            mutation_cmd: None,
            mutation_score_threshold: DEFAULT_MUTATION_SCORE_THRESHOLD,
            features_dir: None,
            gherkin_cmd: None,
            test_timeout_secs: DEFAULT_TEST_TIMEOUT_SECS,
            coverage_timeout_secs: DEFAULT_COVERAGE_TIMEOUT_SECS,
            mutation_timeout_secs: DEFAULT_MUTATION_TIMEOUT_SECS,
        }
    }
}

impl QualityConfig {
    /// Read `<workspace_root>/.claudinio.json` and pull out `"quality"`.
    /// Every failure mode — no file, bad JSON, no `quality` key — yields the
    /// defaults, so a typo in the config can never silently disable the gate.
    pub fn load(workspace_root: &Path) -> Self {
        let Ok(text) = std::fs::read_to_string(workspace_root.join(".claudinio.json")) else {
            return Self::default();
        };
        Self::from_workspace_json(&text)
    }

    pub fn from_workspace_json(text: &str) -> Self {
        let Ok(value) = serde_json::from_str::<serde_json::Value>(text) else {
            return Self::default();
        };
        match value.get("quality") {
            Some(q) => serde_json::from_value(q.clone()).unwrap_or_default(),
            None => Self::default(),
        }
    }

    /// True when a golden task being marked done must carry green evidence.
    pub fn gate_active(&self) -> bool {
        self.enabled && !self.enforced_layers.is_empty()
    }

    /// What `run_quality` runs when the agent names no layers.
    ///
    /// Mutation is excluded even when enforced: it reruns the suite once per
    /// mutant, so an agent checking its work mid-task would burn half an hour
    /// to learn what a plain test run tells it in seconds. It still runs at
    /// the finish line, and the agent can always ask for it by name.
    pub fn tool_default_layers(&self) -> Vec<Layer> {
        self.finish_line_layers()
            .into_iter()
            .filter(|l| *l != Layer::Mutation)
            .collect()
    }

    /// What the harness runs before letting a run finish: everything enforced,
    /// plus tests, which are cheap and gate the rest.
    pub fn finish_line_layers(&self) -> Vec<Layer> {
        let mut layers = vec![Layer::Tests];
        for l in &self.enforced_layers {
            if !layers.contains(l) {
                layers.push(*l);
            }
        }
        layers
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn missing_quality_key_yields_defaults() {
        let cfg = QualityConfig::from_workspace_json(r#"{"plan_save_path":"docs/plans"}"#);
        assert!(cfg.enabled);
        assert_eq!(cfg.enforced_layers, vec![Layer::Tests]);
        assert_eq!(cfg.diff_coverage_threshold, 80.0);
    }

    #[test]
    fn malformed_json_does_not_disable_the_gate() {
        // A broken config must fail safe *toward* enforcement — the opposite
        // would let a stray comma silently turn the harness off.
        let cfg = QualityConfig::from_workspace_json("{not json");
        assert!(cfg.gate_active());
    }

    #[test]
    fn parses_overrides() {
        let cfg = QualityConfig::from_workspace_json(
            r#"{"quality":{"enforced_layers":["tests","coverage"],
                "test_cmd":"make test","diff_coverage_threshold":95.0}}"#,
        );
        assert_eq!(cfg.enforced_layers, vec![Layer::Tests, Layer::Coverage]);
        assert_eq!(cfg.test_cmd.as_deref(), Some("make test"));
        assert_eq!(cfg.diff_coverage_threshold, 95.0);
        // Unspecified fields keep their defaults.
        assert_eq!(cfg.test_timeout_secs, DEFAULT_TEST_TIMEOUT_SECS);
    }

    #[test]
    fn empty_enforced_list_is_observation_mode() {
        let cfg = QualityConfig::from_workspace_json(r#"{"quality":{"enforced_layers":[]}}"#);
        assert!(cfg.enabled);
        assert!(!cfg.gate_active(), "nothing enforced = report, never block");
    }

    #[test]
    fn disabled_turns_the_gate_off() {
        let cfg = QualityConfig::from_workspace_json(r#"{"quality":{"enabled":false}}"#);
        assert!(!cfg.gate_active());
    }

    #[test]
    fn finish_line_layers_include_every_enforced_layer() {
        let cfg =
            QualityConfig::from_workspace_json(r#"{"quality":{"enforced_layers":["coverage"]}}"#);
        let layers = cfg.finish_line_layers();
        assert!(layers.contains(&Layer::Tests));
        assert!(layers.contains(&Layer::Coverage));
    }

    #[test]
    fn enforce_on_defaults_to_any_code_change() {
        // A harness that only acts when the user remembers a <goal> tag
        // protects nobody by default.
        assert_eq!(QualityConfig::default().enforce_on, EnforceOn::CodeChange);
        assert_eq!(
            QualityConfig::from_workspace_json(r#"{"quality":{}}"#).enforce_on,
            EnforceOn::CodeChange
        );
        assert_eq!(
            QualityConfig::from_workspace_json(r#"{}"#).enforce_on,
            EnforceOn::CodeChange
        );
    }

    #[test]
    fn narrowing_to_goals_only_is_explicit() {
        let cfg = QualityConfig::from_workspace_json(r#"{"quality":{"enforce_on":"goals"}}"#);
        assert_eq!(cfg.enforce_on, EnforceOn::Goals);
    }

    #[test]
    fn an_unknown_enforce_on_value_falls_back_to_the_safe_defaults() {
        // serde rejects the whole object, so every field reverts — including
        // the gate staying on, and staying wide.
        let cfg = QualityConfig::from_workspace_json(r#"{"quality":{"enforce_on":"always"}}"#);
        assert_eq!(cfg.enforce_on, EnforceOn::CodeChange);
        assert!(cfg.gate_active());
    }

    #[test]
    fn load_from_missing_file_is_defaults() {
        let cfg = QualityConfig::load(Path::new("/nonexistent-workspace-xyz"));
        assert!(cfg.gate_active());
    }
}
