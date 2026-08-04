//! IPC surface for the quality harness settings.
//!
//! These settings are per-workspace, not global: the commands, thresholds and
//! trigger belong to the project being worked on, so they live in that
//! project's `.claudinio.json` — the same file the agent and the harness read.
//! The Settings panel is a view onto that file, never a second source of truth.

use serde::{Deserialize, Serialize};
use std::path::Path;

use crate::quality::config::EnforceOn;
use crate::quality::{Layer, QualityConfig};

/// The settings, in the shape the UI speaks. A DTO rather than
/// [`QualityConfig`] itself so the wire format can stay camelCase and use
/// plain strings the panel can bind to directly.
#[derive(Debug, Clone, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct QualitySettings {
    pub enabled: bool,
    /// "goals" | "code_change"
    pub enforce_on: String,
    /// Layer names that block a finish: any of "tests", "coverage".
    pub enforced_layers: Vec<String>,
    pub diff_coverage_threshold: f64,
    /// Empty = use the detected command.
    pub test_cmd: String,
    pub coverage_cmd: String,
    pub test_timeout_secs: u64,
    pub coverage_timeout_secs: u64,
}

/// One detected build root, so the panel can show what will actually run
/// instead of asking the user to trust that detection worked.
#[derive(Debug, Clone, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct DetectedStack {
    pub name: String,
    pub root: String,
    pub test_cmd: String,
    pub coverage_cmd: Option<String>,
}

#[derive(Debug, Clone, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct QualityInfo {
    pub settings: QualitySettings,
    pub stacks: Vec<DetectedStack>,
}

impl From<&QualityConfig> for QualitySettings {
    fn from(cfg: &QualityConfig) -> Self {
        Self {
            enabled: cfg.enabled,
            enforce_on: match cfg.enforce_on {
                EnforceOn::Goals => "goals".into(),
                EnforceOn::CodeChange => "code_change".into(),
            },
            enforced_layers: cfg
                .enforced_layers
                .iter()
                .map(|l| l.as_str().to_string())
                .collect(),
            diff_coverage_threshold: cfg.diff_coverage_threshold,
            test_cmd: cfg.test_cmd.clone().unwrap_or_default(),
            coverage_cmd: cfg.coverage_cmd.clone().unwrap_or_default(),
            test_timeout_secs: cfg.test_timeout_secs,
            coverage_timeout_secs: cfg.coverage_timeout_secs,
        }
    }
}

impl QualitySettings {
    /// Back to the internal config. Unknown layer names are dropped rather
    /// than rejected: a panel sending junk must not be able to make the file
    /// unreadable for the harness.
    fn to_config(&self) -> QualityConfig {
        QualityConfig {
            enabled: self.enabled,
            enforce_on: if self.enforce_on == "code_change" {
                EnforceOn::CodeChange
            } else {
                EnforceOn::Goals
            },
            enforced_layers: self
                .enforced_layers
                .iter()
                .filter_map(|l| Layer::parse(l))
                .collect(),
            test_cmd: non_empty(&self.test_cmd),
            coverage_cmd: non_empty(&self.coverage_cmd),
            diff_coverage_threshold: self.diff_coverage_threshold.clamp(0.0, 100.0),
            test_timeout_secs: self.test_timeout_secs.max(1),
            coverage_timeout_secs: self.coverage_timeout_secs.max(1),
        }
    }
}

fn non_empty(s: &str) -> Option<String> {
    let t = s.trim();
    if t.is_empty() {
        None
    } else {
        Some(t.to_string())
    }
}

/// Current settings plus what detection found for this workspace.
#[tauri::command]
pub async fn get_quality_config(workspace_root: String) -> Result<QualityInfo, String> {
    let root = std::path::PathBuf::from(&workspace_root);
    let cfg = QualityConfig::load(&root);
    let profile = crate::quality::profile::detect(&root, &cfg);
    Ok(QualityInfo {
        settings: QualitySettings::from(&cfg),
        stacks: profile
            .stacks
            .into_iter()
            .map(|s| DetectedStack {
                name: s.name,
                root: s.root.to_string_lossy().to_string(),
                test_cmd: s.test_cmd,
                coverage_cmd: s.coverage_cmd,
            })
            .collect(),
    })
}

/// Write the `quality` block, preserving every other key in the file.
#[tauri::command]
pub async fn set_quality_config(
    workspace_root: String,
    settings: QualitySettings,
) -> Result<(), String> {
    let config_path = Path::new(&workspace_root).join(".claudinio.json");
    let mut file: serde_json::Value = if config_path.exists() {
        std::fs::read_to_string(&config_path)
            .ok()
            .and_then(|s| serde_json::from_str(&s).ok())
            .unwrap_or_else(|| serde_json::json!({}))
    } else {
        serde_json::json!({})
    };
    let block = serde_json::to_value(settings.to_config())
        .map_err(|e| format!("serialize quality settings: {e}"))?;
    let Some(obj) = file.as_object_mut() else {
        return Err(".claudinio.json is not a JSON object".into());
    };
    obj.insert("quality".into(), block);
    let json = serde_json::to_string_pretty(&file)
        .map_err(|e| format!("serialize .claudinio.json: {e}"))?;
    std::fs::write(&config_path, json).map_err(|e| format!("write .claudinio.json: {e}"))?;
    Ok(())
}

#[cfg(test)]
mod tests {
    use super::*;

    fn tmp(name: &str) -> std::path::PathBuf {
        let p = std::env::temp_dir().join(format!("cq-cmd-{name}-{}", std::process::id()));
        std::fs::remove_dir_all(&p).ok();
        std::fs::create_dir_all(&p).unwrap();
        p
    }

    fn settings() -> QualitySettings {
        QualitySettings::from(&QualityConfig::default())
    }

    #[test]
    fn defaults_round_trip_through_the_wire_shape() {
        let cfg = settings().to_config();
        assert!(cfg.enabled);
        assert_eq!(cfg.enforce_on, EnforceOn::Goals);
        assert_eq!(cfg.enforced_layers, vec![Layer::Tests]);
    }

    #[test]
    fn an_unknown_layer_name_is_dropped_not_persisted() {
        // The panel must never be able to write a file the harness cannot read.
        let mut s = settings();
        s.enforced_layers = vec!["tests".into(), "mutation".into()];
        assert_eq!(s.to_config().enforced_layers, vec![Layer::Tests]);
    }

    #[test]
    fn threshold_and_timeouts_are_clamped_to_sane_values() {
        let mut s = settings();
        s.diff_coverage_threshold = 250.0;
        s.test_timeout_secs = 0;
        let cfg = s.to_config();
        assert_eq!(cfg.diff_coverage_threshold, 100.0);
        assert_eq!(cfg.test_timeout_secs, 1, "a zero timeout kills every run");
    }

    #[test]
    fn blank_commands_mean_use_detection() {
        let mut s = settings();
        s.test_cmd = "   ".into();
        assert_eq!(s.to_config().test_cmd, None);
    }

    #[tokio::test]
    async fn writing_settings_preserves_the_rest_of_the_file() {
        let root = tmp("preserve");
        std::fs::write(
            root.join(".claudinio.json"),
            r#"{"plan_save_path":"docs/plans","mcp":{"x":{}}}"#,
        )
        .unwrap();

        let mut s = settings();
        s.enforce_on = "code_change".into();
        set_quality_config(root.to_string_lossy().to_string(), s)
            .await
            .unwrap();

        let text = std::fs::read_to_string(root.join(".claudinio.json")).unwrap();
        let v: serde_json::Value = serde_json::from_str(&text).unwrap();
        assert_eq!(v["plan_save_path"], "docs/plans");
        assert!(v["mcp"]["x"].is_object(), "unrelated keys must survive");
        assert_eq!(v["quality"]["enforce_on"], "code_change");

        // And the harness reads back exactly what the panel wrote.
        assert_eq!(QualityConfig::load(&root).enforce_on, EnforceOn::CodeChange);
        std::fs::remove_dir_all(&root).ok();
    }

    #[tokio::test]
    async fn reading_reports_the_detected_stacks() {
        let root = tmp("detect");
        std::fs::write(root.join("Cargo.toml"), "[package]\nname=\"x\"\n").unwrap();
        let info = get_quality_config(root.to_string_lossy().to_string())
            .await
            .unwrap();
        assert_eq!(info.stacks.len(), 1);
        assert_eq!(info.stacks[0].name, "rust");
        assert_eq!(info.stacks[0].test_cmd, "cargo test");
        assert_eq!(info.settings.enforce_on, "goals");
        std::fs::remove_dir_all(&root).ok();
    }

    #[tokio::test]
    async fn a_workspace_with_no_config_file_reads_the_defaults() {
        let root = tmp("nofile");
        let info = get_quality_config(root.to_string_lossy().to_string())
            .await
            .unwrap();
        assert!(info.settings.enabled);
        assert_eq!(info.settings.enforced_layers, vec!["tests".to_string()]);
        assert!(info.stacks.is_empty());
        std::fs::remove_dir_all(&root).ok();
    }
}
