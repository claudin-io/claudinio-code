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

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct QualityConfig {
    /// Master switch. False disables both the tool and the gate.
    #[serde(default = "default_true")]
    pub enabled: bool,
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
    #[serde(default = "default_test_timeout")]
    pub test_timeout_secs: u64,
    #[serde(default = "default_coverage_timeout")]
    pub coverage_timeout_secs: u64,
}

fn default_true() -> bool {
    true
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

impl Default for QualityConfig {
    fn default() -> Self {
        Self {
            enabled: true,
            enforced_layers: default_enforced(),
            test_cmd: None,
            coverage_cmd: None,
            diff_coverage_threshold: DEFAULT_DIFF_COVERAGE_THRESHOLD,
            test_timeout_secs: DEFAULT_TEST_TIMEOUT_SECS,
            coverage_timeout_secs: DEFAULT_COVERAGE_TIMEOUT_SECS,
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

    /// The layers worth running by default: everything enforced, plus tests,
    /// which are cheap and are what the model most often actually wants.
    pub fn default_layers(&self) -> Vec<Layer> {
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
    fn default_layers_include_every_enforced_layer() {
        let cfg =
            QualityConfig::from_workspace_json(r#"{"quality":{"enforced_layers":["coverage"]}}"#);
        let layers = cfg.default_layers();
        assert!(layers.contains(&Layer::Tests));
        assert!(layers.contains(&Layer::Coverage));
    }

    #[test]
    fn load_from_missing_file_is_defaults() {
        let cfg = QualityConfig::load(Path::new("/nonexistent-workspace-xyz"));
        assert!(cfg.gate_active());
    }
}
