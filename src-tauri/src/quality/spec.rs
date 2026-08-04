//! Finding and protecting the project's specs.
//!
//! Everything else in this harness measures the code against itself. Specs are
//! the only input that comes from outside the model — which is why they get two
//! properties nothing else here has:
//!
//! 1. **They are human-owned.** The agent cannot edit them. A spec the
//!    implementer may rewrite is not a specification, it is a comment: any
//!    disagreement between code and spec would be resolved by editing the spec.
//! 2. **They drive planning.** The scenario index is injected into the planner's
//!    prompt, so the plan is written against what was actually asked for.

use std::path::{Path, PathBuf};

use super::parsers::gherkin::{self, Feature};

/// Where specs live when the project says nothing else. Matches the layout
/// every BDD runner already assumes.
pub const DEFAULT_FEATURES_DIR: &str = "features";

/// Absolute path of the workspace's spec directory.
pub fn features_root(workspace_root: &Path, configured: Option<&str>) -> PathBuf {
    let rel = configured
        .map(str::trim)
        .filter(|s| !s.is_empty())
        .unwrap_or(DEFAULT_FEATURES_DIR);
    workspace_root.join(rel)
}

/// Every `.feature` file under the spec directory, parsed. Empty when the
/// project has no specs, which is not an error — most projects do not.
pub fn load_features(workspace_root: &Path, configured: Option<&str>) -> Vec<Feature> {
    let root = features_root(workspace_root, configured);
    if !root.is_dir() {
        return Vec::new();
    }
    let mut paths: Vec<PathBuf> = ignore::WalkBuilder::new(&root)
        .hidden(false)
        .build()
        .flatten()
        .map(|e| e.into_path())
        .filter(|p| {
            p.extension()
                .is_some_and(|e| e.eq_ignore_ascii_case("feature"))
        })
        .collect();
    // Stable order so the prompt block and reports do not shuffle between runs.
    paths.sort();

    paths
        .iter()
        .filter_map(|path| {
            let text = std::fs::read_to_string(path).ok()?;
            let display = path
                .strip_prefix(workspace_root)
                .unwrap_or(path)
                .to_string_lossy()
                .to_string();
            Some(gherkin::parse_feature(&display, &text))
        })
        .collect()
}

/// Is this path inside the workspace's spec directory?
///
/// Used to refuse edits. Compared after normalising separators so a Windows
/// path cannot slip past a check written with forward slashes.
pub fn is_spec_path(workspace_root: &Path, configured: Option<&str>, candidate: &Path) -> bool {
    let root = features_root(workspace_root, configured);
    let normalise = |p: &Path| p.to_string_lossy().replace('\\', "/");
    let (root, candidate) = (normalise(&root), normalise(candidate));
    candidate == root || candidate.starts_with(&format!("{root}/"))
}

/// The message the edit tool returns when the agent tries to rewrite a spec.
pub fn edit_refusal(path: &str) -> String {
    format!(
        "edit_file rejected: {path} is a specification, and specifications are owned by the \
         user, not by you. They are the one input to this project that did not come from a \
         model, which is what makes them worth checking against. If the code cannot satisfy a \
         scenario, or a scenario looks wrong, say so and ask the user to change it — do not \
         edit the spec to match the implementation."
    )
}

#[cfg(test)]
mod tests {
    use super::*;

    fn workspace(name: &str) -> PathBuf {
        let root = std::env::temp_dir().join(format!("cq-spec-{name}-{}", std::process::id()));
        std::fs::remove_dir_all(&root).ok();
        std::fs::create_dir_all(root.join("features/nested")).unwrap();
        std::fs::write(
            root.join("features/discount.feature"),
            "Feature: Discounts\n  Scenario: Member discount\n    Then it applies\n",
        )
        .unwrap();
        std::fs::write(
            root.join("features/nested/refund.feature"),
            "Feature: Refunds\n  Scenario: Late refund\n    Then it is refused\n",
        )
        .unwrap();
        std::fs::write(root.join("features/README.md"), "not a feature\n").unwrap();
        root
    }

    #[test]
    fn loads_every_feature_file_including_nested_ones() {
        let root = workspace("load");
        let features = load_features(&root, None);
        assert_eq!(features.len(), 2);
        let names: Vec<&str> = features.iter().map(|f| f.name.as_str()).collect();
        assert!(names.contains(&"Discounts"), "{names:?}");
        assert!(names.contains(&"Refunds"), "{names:?}");
        std::fs::remove_dir_all(&root).ok();
    }

    #[test]
    fn non_feature_files_are_ignored() {
        let root = workspace("filter");
        let features = load_features(&root, None);
        assert!(features.iter().all(|f| f.path.ends_with(".feature")));
        std::fs::remove_dir_all(&root).ok();
    }

    #[test]
    fn feature_paths_are_reported_relative_to_the_workspace() {
        let root = workspace("relative");
        let features = load_features(&root, None);
        assert!(
            features.iter().all(|f| !f.path.starts_with('/')),
            "{:?}",
            features.iter().map(|f| &f.path).collect::<Vec<_>>()
        );
        std::fs::remove_dir_all(&root).ok();
    }

    #[test]
    fn a_project_without_specs_loads_nothing_and_that_is_fine() {
        let root = std::env::temp_dir().join(format!("cq-spec-none-{}", std::process::id()));
        std::fs::create_dir_all(&root).unwrap();
        assert!(load_features(&root, None).is_empty());
        std::fs::remove_dir_all(&root).ok();
    }

    #[test]
    fn a_configured_directory_is_used_instead_of_the_default() {
        let root = std::env::temp_dir().join(format!("cq-spec-cfg-{}", std::process::id()));
        std::fs::remove_dir_all(&root).ok();
        std::fs::create_dir_all(root.join("docs/specs")).unwrap();
        std::fs::write(
            root.join("docs/specs/a.feature"),
            "Feature: Configured\n  Scenario: Found\n",
        )
        .unwrap();
        assert!(
            load_features(&root, None).is_empty(),
            "default dir is empty"
        );
        let features = load_features(&root, Some("docs/specs"));
        assert_eq!(features.len(), 1);
        assert_eq!(features[0].name, "Configured");
        std::fs::remove_dir_all(&root).ok();
    }

    #[test]
    fn spec_paths_are_recognised_for_the_edit_guard() {
        let root = Path::new("/repo");
        assert!(is_spec_path(
            root,
            None,
            Path::new("/repo/features/a.feature")
        ));
        assert!(is_spec_path(
            root,
            None,
            Path::new("/repo/features/nested/a.feature")
        ));
        assert!(is_spec_path(root, None, Path::new("/repo/features")));
        // A sibling directory whose name merely starts the same must not match.
        assert!(!is_spec_path(
            root,
            None,
            Path::new("/repo/features-old/a.feature")
        ));
        assert!(!is_spec_path(root, None, Path::new("/repo/src/main.rs")));
    }

    #[test]
    fn the_configured_directory_is_what_gets_protected() {
        let root = Path::new("/repo");
        assert!(is_spec_path(
            root,
            Some("docs/specs"),
            Path::new("/repo/docs/specs/a.feature")
        ));
        // Once specs move, the old default is ordinary code again.
        assert!(!is_spec_path(
            root,
            Some("docs/specs"),
            Path::new("/repo/features/a.feature")
        ));
    }

    #[test]
    fn the_refusal_says_who_owns_the_spec_and_what_to_do() {
        let msg = edit_refusal("features/a.feature");
        assert!(msg.contains("features/a.feature"));
        assert!(msg.contains("ask the user"));
        assert!(msg.contains("do not"));
    }
}
