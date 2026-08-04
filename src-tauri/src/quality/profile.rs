//! Working out how to run *this* project's tests.
//!
//! The app had no notion of project type before this (the LSP manager's
//! two-server lookup was the closest thing), so detection lives here. It stays
//! deliberately shallow: root first, then one level down, because the common
//! shape is a repo root plus one or two build roots — this very repo is a pnpm
//! frontend at the root with a Cargo crate in `src-tauri/`. Anything more
//! exotic is what the `.claudinio.json` overrides are for.

use serde::{Deserialize, Serialize};
use std::path::{Path, PathBuf};

use super::config::QualityConfig;

/// Which parser reads this stack's test output. Every option is a
/// machine-readable format — the harness never decides pass/fail by eyeballing
/// prose, except for `cargo test`, whose summary line is stable and is checked
/// alongside the process exit code.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
pub enum TestParser {
    CargoTest,
    VitestJson,
    JestJson,
    /// A user-supplied command: exit code is the only signal available.
    ExitCodeOnly,
}

/// One buildable, testable root inside the workspace.
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct StackProfile {
    /// Short label used in reports: "rust", "js".
    pub name: String,
    /// Directory the commands run in.
    pub root: PathBuf,
    /// Test command. `{artifact_dir}` is substituted with a scratch directory.
    pub test_cmd: String,
    pub test_parser: TestParser,
    /// File inside `{artifact_dir}` the test command writes, if any.
    pub test_artifact: Option<String>,
    /// Coverage command, also taking `{artifact_dir}`. `None` when the stack
    /// has no coverage tooling we know how to drive.
    pub coverage_cmd: Option<String>,
    /// Cheap command that fails when the coverage tool is not installed, so a
    /// missing `cargo-llvm-cov` is reported as `Unavailable` in a few
    /// milliseconds instead of after a long confusing failure.
    pub coverage_probe: Option<String>,
    /// lcov file the coverage command writes inside `{artifact_dir}`.
    pub coverage_lcov: String,
    /// Mutation command. Takes `{artifact_dir}` and `{in_diff}`, the latter
    /// expanding to the diff-scoping flag or to nothing. `None` when the
    /// harness cannot drive this stack natively.
    pub mutation_cmd: Option<String>,
    pub mutation_probe: Option<String>,
    /// Command that executes the project's `.feature` specs, when a real BDD
    /// runner is wired up. `None` means the scenarios exist but nothing here
    /// can execute them — reported honestly rather than guessed at.
    pub gherkin_cmd: Option<String>,
}

#[derive(Debug, Clone, Default, Serialize, Deserialize)]
pub struct ProjectProfile {
    pub stacks: Vec<StackProfile>,
}

/// Detect every testable stack under `workspace_root`, then apply the user's
/// overrides. An explicit `test_cmd` in `.claudinio.json` always wins — it is
/// the escape hatch for anything detection gets wrong.
pub fn detect(workspace_root: &Path, cfg: &QualityConfig) -> ProjectProfile {
    let mut stacks = Vec::new();
    if let Some(rust) = detect_rust(workspace_root) {
        stacks.push(rust);
    }
    if let Some(js) = detect_js(workspace_root) {
        stacks.push(js);
    }

    // A user-supplied command replaces detection entirely: if they told us how
    // to test the project, running our guess too would double the wall clock
    // and could contradict them.
    if let Some(cmd) = cfg.test_cmd.as_deref().filter(|c| !c.trim().is_empty()) {
        stacks = vec![StackProfile {
            name: "custom".into(),
            root: workspace_root.to_path_buf(),
            test_cmd: cmd.to_string(),
            test_parser: TestParser::ExitCodeOnly,
            test_artifact: None,
            coverage_cmd: None,
            coverage_probe: None,
            coverage_lcov: "lcov.info".into(),
            mutation_cmd: None,
            mutation_probe: None,
            gherkin_cmd: None,
        }];
    }
    if let Some(cmd) = cfg.coverage_cmd.as_deref().filter(|c| !c.trim().is_empty()) {
        for stack in &mut stacks {
            stack.coverage_cmd = Some(cmd.to_string());
            stack.coverage_probe = None;
        }
    }
    if let Some(cmd) = cfg.mutation_cmd.as_deref().filter(|c| !c.trim().is_empty()) {
        for stack in &mut stacks {
            stack.mutation_cmd = Some(cmd.to_string());
            stack.mutation_probe = None;
        }
    }
    if let Some(cmd) = cfg.gherkin_cmd.as_deref().filter(|c| !c.trim().is_empty()) {
        for stack in &mut stacks {
            stack.gherkin_cmd = Some(cmd.to_string());
        }
    }

    ProjectProfile { stacks }
}

/// Root first, then one level down, skipping the directories that are always
/// noise. Returns the first hit — several Cargo workspaces under one folder is
/// a case for the config override, not for guessing.
fn find_manifest(workspace_root: &Path, file: &str) -> Option<PathBuf> {
    if workspace_root.join(file).is_file() {
        return Some(workspace_root.to_path_buf());
    }
    let entries = std::fs::read_dir(workspace_root).ok()?;
    let mut candidates: Vec<PathBuf> = entries
        .flatten()
        .map(|e| e.path())
        .filter(|p| p.is_dir() && !is_noise_dir(p))
        .filter(|p| p.join(file).is_file())
        .collect();
    // Deterministic pick: detection must not depend on readdir order, or the
    // same repo could report a different stack between runs.
    candidates.sort();
    candidates.into_iter().next()
}

fn is_noise_dir(path: &Path) -> bool {
    let name = path
        .file_name()
        .map(|n| n.to_string_lossy().to_string())
        .unwrap_or_default();
    name.starts_with('.')
        || matches!(
            name.as_str(),
            "node_modules" | "target" | "dist" | "build" | "vendor" | "coverage" | "__pycache__"
        )
}

fn detect_rust(workspace_root: &Path) -> Option<StackProfile> {
    let root = find_manifest(workspace_root, "Cargo.toml")?;
    Some(StackProfile {
        name: "rust".into(),
        root,
        test_cmd: "cargo test".into(),
        test_parser: TestParser::CargoTest,
        test_artifact: None,
        coverage_cmd: Some("cargo llvm-cov --lcov --output-path {artifact_dir}/lcov.info".into()),
        coverage_probe: Some("cargo llvm-cov --version".into()),
        coverage_lcov: "lcov.info".into(),
        // `-o` makes cargo-mutants create mutants.out INSIDE the given
        // directory instead of the source tree, so a run never leaves
        // artifacts in the user's repo (which would also churn the digest).
        mutation_cmd: Some("cargo mutants -o {artifact_dir} {in_diff}".into()),
        mutation_probe: Some("cargo mutants --version".into()),
        // cucumber-rs runs as an ordinary test target, so `cargo test` already
        // executes it. A separate command would run the scenarios twice.
        gherkin_cmd: None,
    })
}

fn detect_js(workspace_root: &Path) -> Option<StackProfile> {
    let root = find_manifest(workspace_root, "package.json")?;
    let manifest = std::fs::read_to_string(root.join("package.json")).ok()?;
    let pkg: serde_json::Value = serde_json::from_str(&manifest).ok()?;
    let runner = js_test_runner(&pkg)?;
    let exec = package_manager_exec(&root);

    let (test_cmd, test_parser, coverage_cmd) = match runner {
        JsRunner::Vitest => (
            format!("{exec} vitest run --reporter=json --outputFile={{artifact_dir}}/tests.json"),
            TestParser::VitestJson,
            format!(
                "{exec} vitest run --coverage --coverage.reporter=lcovonly \
                 --coverage.reportsDirectory={{artifact_dir}}"
            ),
        ),
        JsRunner::Jest => (
            format!("{exec} jest --json --outputFile={{artifact_dir}}/tests.json"),
            TestParser::JestJson,
            format!(
                "{exec} jest --coverage --coverageReporters=lcovonly \
                 --coverageDirectory={{artifact_dir}}"
            ),
        ),
    };

    Some(StackProfile {
        name: "js".into(),
        root,
        test_cmd,
        test_parser,
        test_artifact: Some("tests.json".into()),
        coverage_cmd: Some(coverage_cmd),
        coverage_probe: None,
        coverage_lcov: "lcov.info".into(),
        // Deliberately absent: Stryker's flags for redirecting its report and
        // scoping to changed files have not been verified end to end here, and
        // shipping an unverified command means the user waits minutes to learn
        // nothing. Set quality.mutation_cmd to opt in — the standard
        // mutation-testing-elements JSON is already parsed.
        mutation_cmd: None,
        mutation_probe: None,
        gherkin_cmd: bdd_runner_cmd(&pkg, exec),
    })
}

/// A JS BDD runner, if the project has one wired up. Only cucumber-js is
/// detected: it is the one with a stable CLI that exits non-zero on a failing
/// scenario, which is all the gate needs.
fn bdd_runner_cmd(pkg: &serde_json::Value, exec: &str) -> Option<String> {
    let has = |name: &str| {
        ["devDependencies", "dependencies"]
            .iter()
            .any(|block| pkg.get(block).and_then(|d| d.get(name)).is_some())
    };
    if has("@cucumber/cucumber") {
        Some(format!("{exec} cucumber-js"))
    } else {
        None
    }
}

enum JsRunner {
    Vitest,
    Jest,
}

/// Look for the runner in either dependency block. Vitest wins when both are
/// present: a repo migrating to vitest keeps jest installed for a while, and
/// the newer runner is the one whose config is current.
fn js_test_runner(pkg: &serde_json::Value) -> Option<JsRunner> {
    let has = |name: &str| {
        ["devDependencies", "dependencies"]
            .iter()
            .any(|block| pkg.get(block).and_then(|d| d.get(name)).is_some())
    };
    if has("vitest") {
        Some(JsRunner::Vitest)
    } else if has("jest") {
        Some(JsRunner::Jest)
    } else {
        None
    }
}

/// How to invoke a local binary, inferred from the lockfile. Getting this
/// wrong is not fatal (`npx` resolves `node_modules/.bin` too) but using the
/// project's own package manager avoids npx's install prompt.
fn package_manager_exec(root: &Path) -> &'static str {
    if root.join("pnpm-lock.yaml").is_file() {
        "pnpm exec"
    } else if root.join("bun.lockb").is_file() || root.join("bun.lock").is_file() {
        "bunx"
    } else if root.join("yarn.lock").is_file() {
        "yarn"
    } else {
        "npx"
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn tmp(name: &str) -> PathBuf {
        let p = std::env::temp_dir().join(format!("cq-profile-{name}-{}", std::process::id()));
        std::fs::remove_dir_all(&p).ok();
        std::fs::create_dir_all(&p).unwrap();
        p
    }

    #[test]
    fn detects_a_rust_crate_at_the_root() {
        let root = tmp("rust-root");
        std::fs::write(root.join("Cargo.toml"), "[package]\nname=\"x\"\n").unwrap();
        let p = detect(&root, &QualityConfig::default());
        assert_eq!(p.stacks.len(), 1);
        assert_eq!(p.stacks[0].name, "rust");
        assert_eq!(p.stacks[0].test_cmd, "cargo test");
        std::fs::remove_dir_all(&root).ok();
    }

    #[test]
    fn finds_a_crate_one_level_down() {
        // This repo's exact shape: frontend at the root, crate in src-tauri/.
        let root = tmp("nested");
        std::fs::create_dir_all(root.join("src-tauri")).unwrap();
        std::fs::write(root.join("src-tauri/Cargo.toml"), "[package]\n").unwrap();
        std::fs::write(
            root.join("package.json"),
            r#"{"devDependencies":{"vitest":"^4"}}"#,
        )
        .unwrap();
        std::fs::write(root.join("pnpm-lock.yaml"), "lockfileVersion: '9'\n").unwrap();

        let p = detect(&root, &QualityConfig::default());
        let names: Vec<&str> = p.stacks.iter().map(|s| s.name.as_str()).collect();
        assert!(names.contains(&"rust"), "{names:?}");
        assert!(names.contains(&"js"), "{names:?}");
        let js = p.stacks.iter().find(|s| s.name == "js").unwrap();
        assert!(
            js.test_cmd.starts_with("pnpm exec vitest"),
            "{}",
            js.test_cmd
        );
        std::fs::remove_dir_all(&root).ok();
    }

    #[test]
    fn ignores_node_modules_when_scanning_one_level_down() {
        let root = tmp("noise");
        std::fs::create_dir_all(root.join("node_modules/some-dep")).unwrap();
        std::fs::write(root.join("node_modules/Cargo.toml"), "[package]\n").unwrap();
        let p = detect(&root, &QualityConfig::default());
        assert!(p.stacks.is_empty(), "node_modules must never be a stack");
        std::fs::remove_dir_all(&root).ok();
    }

    #[test]
    fn package_json_without_a_test_runner_is_not_a_stack() {
        let root = tmp("no-runner");
        std::fs::write(
            root.join("package.json"),
            r#"{"dependencies":{"react":"^19"}}"#,
        )
        .unwrap();
        let p = detect(&root, &QualityConfig::default());
        assert!(p.stacks.is_empty());
        std::fs::remove_dir_all(&root).ok();
    }

    #[test]
    fn jest_is_detected_and_npx_is_the_fallback_exec() {
        let root = tmp("jest");
        std::fs::write(
            root.join("package.json"),
            r#"{"devDependencies":{"jest":"^29"}}"#,
        )
        .unwrap();
        let p = detect(&root, &QualityConfig::default());
        assert_eq!(p.stacks[0].test_parser, TestParser::JestJson);
        assert!(p.stacks[0].test_cmd.starts_with("npx jest"));
        std::fs::remove_dir_all(&root).ok();
    }

    #[test]
    fn a_configured_test_command_replaces_detection() {
        let root = tmp("override");
        std::fs::write(root.join("Cargo.toml"), "[package]\n").unwrap();
        let cfg = QualityConfig {
            test_cmd: Some("make test".into()),
            ..Default::default()
        };
        let p = detect(&root, &cfg);
        assert_eq!(
            p.stacks.len(),
            1,
            "the override must not run alongside cargo"
        );
        assert_eq!(p.stacks[0].test_cmd, "make test");
        assert_eq!(p.stacks[0].test_parser, TestParser::ExitCodeOnly);
        std::fs::remove_dir_all(&root).ok();
    }

    #[test]
    fn detects_this_repository() {
        // The shape detection has to get right in practice: a pnpm/vitest
        // frontend at the root with the Cargo crate one level down. A fixture
        // can drift from reality; the real checkout cannot.
        let repo_root = Path::new(env!("CARGO_MANIFEST_DIR"))
            .parent()
            .expect("src-tauri has a parent");
        let p = detect(repo_root, &QualityConfig::default());
        let rust = p
            .stacks
            .iter()
            .find(|s| s.name == "rust")
            .expect("rust stack");
        assert!(rust.root.ends_with("src-tauri"), "{:?}", rust.root);
        assert_eq!(rust.test_cmd, "cargo test");

        let js = p.stacks.iter().find(|s| s.name == "js").expect("js stack");
        assert_eq!(js.root, repo_root);
        assert_eq!(js.test_parser, TestParser::VitestJson);
        assert!(
            js.test_cmd.starts_with("pnpm exec vitest run"),
            "{}",
            js.test_cmd
        );
    }

    #[test]
    fn empty_workspace_has_no_stacks() {
        let root = tmp("empty");
        assert!(detect(&root, &QualityConfig::default()).stacks.is_empty());
        std::fs::remove_dir_all(&root).ok();
    }
}
