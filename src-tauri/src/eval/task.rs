//! The task file: one YAML document per task under a task-set directory.
//!
//! ```yaml
//! id: off-by-one-pager          # must equal the file name without .yaml
//! title: Fix the pager's last page
//! tags: [bugfix]
//! prompt: |                     # the user's words, sent as the first message
//!   The last page of results is always missing. Fix it.
//! setup:                        # the start state: files, or a git tree
//!   files: { "pager.py": "...", "test_pager.py": "..." }
//!   # git: { repo: /path/or/url, commit: <sha> }   relative paths: from this file
//!   # symlinks: { node_modules: ../../node_modules }  likewise
//! check: python3 -m pytest -q   # what run_quality runs (default: oracle.command)
//! oracle:                       # decides pass/fail after the run
//!   command: python3 -m pytest -q
//!   timeout_s: 120
//!   # files: { path: content }  hidden tests written just before the command
//!   # patch: <unified diff>      applied with `git apply` before the command
//!   # reference_files: [path]    taken from reference.git_commit before the command
//! reference:                    # a known solution, used only by `validate`
//!   files: { "pager.py": "..." }
//!   # git_commit: <sha>           the tree at that commit (needs setup.git)
//!   # patch: <unified diff>       applied to the start tree
//! answers: ["Use the existing helper."]  # scripted replies to ask_user
//! max_rounds: 40
//! ```
//!
//! Unknown keys are refused rather than ignored: a misspelled `oracle.fiels`
//! would otherwise silently produce a task whose hidden tests never run.

use serde::{Deserialize, Serialize};
use std::collections::BTreeMap;
use std::path::{Path, PathBuf};

pub(crate) const DEFAULT_ORACLE_TIMEOUT_S: u64 = 300;

#[derive(Debug, Clone, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub(crate) struct Task {
    pub id: String,
    pub title: String,
    #[serde(default, skip_serializing_if = "Vec::is_empty")]
    pub tags: Vec<String>,
    pub prompt: String,
    pub setup: Setup,
    /// The command `run_quality` runs during the session. Defaults to the
    /// oracle's command; a task whose oracle relies on hidden tests sets it to
    /// what the agent may legitimately run.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub check: Option<String>,
    pub oracle: Oracle,
    pub reference: Reference,
    #[serde(default, skip_serializing_if = "Vec::is_empty")]
    pub answers: Vec<String>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub max_rounds: Option<usize>,
}

#[derive(Debug, Clone, Default, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub(crate) struct Setup {
    #[serde(default, skip_serializing_if = "BTreeMap::is_empty")]
    pub files: BTreeMap<String, String>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub git: Option<GitSource>,
    /// Links created after the tree is materialized (e.g. a shared
    /// `node_modules`), and kept out of the start commit.
    #[serde(default, skip_serializing_if = "BTreeMap::is_empty")]
    pub symlinks: BTreeMap<String, String>,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub(crate) struct GitSource {
    /// A local repository path or a clone URL.
    pub repo: String,
    pub commit: String,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub(crate) struct Oracle {
    pub command: String,
    #[serde(default = "default_timeout")]
    pub timeout_s: u64,
    #[serde(default, skip_serializing_if = "BTreeMap::is_empty")]
    pub files: BTreeMap<String, String>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub patch: Option<String>,
    #[serde(default, skip_serializing_if = "Vec::is_empty")]
    pub reference_files: Vec<String>,
}

fn default_timeout() -> u64 {
    DEFAULT_ORACLE_TIMEOUT_S
}

#[derive(Debug, Clone, Default, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub(crate) struct Reference {
    #[serde(default, skip_serializing_if = "BTreeMap::is_empty")]
    pub files: BTreeMap<String, String>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub git_commit: Option<String>,
    /// A unified diff applied to the start tree with `git apply` (how
    /// SWE-bench ships its gold solutions).
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub patch: Option<String>,
}

impl Task {
    /// The command `run_quality` runs during an eval session.
    pub fn check_command(&self) -> &str {
        self.check.as_deref().unwrap_or(&self.oracle.command)
    }

    /// Parse one task document. `stem` is the file name without extension.
    pub fn parse(text: &str, stem: &str) -> Result<Task, String> {
        let task: Task = serde_yaml::from_str(text).map_err(|e| format!("{stem}: {e}"))?;
        task.check_shape(stem)?;
        Ok(task)
    }

    fn check_shape(&self, stem: &str) -> Result<(), String> {
        let fail = |msg: &str| Err(format!("{stem}: {msg}"));
        if self.id != stem {
            return fail(&format!("id '{}' must equal the file name", self.id));
        }
        if self.id.is_empty()
            || !self
                .id
                .chars()
                .all(|c| c.is_ascii_lowercase() || c.is_ascii_digit() || c == '-')
        {
            return fail("id must be lowercase letters, digits and '-'");
        }
        if self.prompt.trim().is_empty() {
            return fail("prompt is empty");
        }
        if self.oracle.command.trim().is_empty() {
            return fail("oracle.command is empty");
        }
        if self.setup.git.is_some() && !self.setup.files.is_empty() {
            return fail("setup takes files or git, not both");
        }
        if self.setup.git.is_none() && self.setup.files.is_empty() {
            return fail("setup needs files or git");
        }
        let kinds = usize::from(self.reference.git_commit.is_some())
            + usize::from(!self.reference.files.is_empty())
            + usize::from(self.reference.patch.is_some());
        if kinds == 0 {
            return fail("reference needs files or git_commit or patch");
        }
        if kinds > 1 {
            return fail("reference takes one of files, git_commit or patch");
        }
        if self.reference.git_commit.is_some() && self.setup.git.is_none() {
            return fail("reference.git_commit needs setup.git");
        }
        if !self.oracle.reference_files.is_empty() && self.reference.git_commit.is_none() {
            return fail("oracle.reference_files needs reference.git_commit");
        }
        for path in self
            .setup
            .files
            .keys()
            .chain(self.reference.files.keys())
            .chain(self.oracle.files.keys())
            .chain(self.setup.symlinks.keys())
            .chain(self.oracle.reference_files.iter())
        {
            if !is_relative_inside(path) {
                return fail(&format!(
                    "path '{path}' must be relative and stay inside the task"
                ));
            }
        }
        Ok(())
    }
}

impl Task {
    /// Read a relative `setup.git.repo` and relative symlink targets from
    /// `dir` (the task file's directory), so a task set can point at its own
    /// repository (`repo: ../..`) wherever the runner is started from.
    fn resolve_relative_paths(&mut self, dir: &Path) {
        let base = dir.canonicalize().unwrap_or_else(|_| dir.to_path_buf());
        let resolve = |p: &str| -> String {
            let joined = base.join(p);
            joined
                .canonicalize()
                .unwrap_or(joined)
                .to_string_lossy()
                .into_owned()
        };
        if let Some(git) = &mut self.setup.git
            && !git.repo.contains("://")
            && !git.repo.starts_with("git@")
            && Path::new(&git.repo).is_relative()
        {
            git.repo = resolve(&git.repo);
        }
        for target in self.setup.symlinks.values_mut() {
            if Path::new(target.as_str()).is_relative() {
                *target = resolve(target);
            }
        }
    }
}

/// A relative path with no `..` component: everything a task writes stays in
/// its own directory.
pub(crate) fn is_relative_inside(path: &str) -> bool {
    let p = Path::new(path);
    !path.is_empty()
        && p.is_relative()
        && p.components().all(|c| {
            matches!(
                c,
                std::path::Component::Normal(_) | std::path::Component::CurDir
            )
        })
}

/// Every task in `dir` (`*.yaml` / `*.yml`, in file-name order), narrowed to
/// `only` when given. An id in `only` that matches no file is an error, so a
/// typo does not quietly run an empty set.
pub(crate) fn load_taskset(dir: &Path, only: Option<&[String]>) -> Result<Vec<Task>, String> {
    let mut files: Vec<PathBuf> = std::fs::read_dir(dir)
        .map_err(|e| format!("read {}: {e}", dir.display()))?
        .flatten()
        .map(|e| e.path())
        .filter(|p| matches!(p.extension().and_then(|e| e.to_str()), Some("yaml" | "yml")))
        .collect();
    files.sort();
    let mut tasks = Vec::new();
    for file in files {
        let stem = file
            .file_stem()
            .and_then(|s| s.to_str())
            .unwrap_or_default()
            .to_string();
        if let Some(only) = only
            && !only.contains(&stem)
        {
            continue;
        }
        let text =
            std::fs::read_to_string(&file).map_err(|e| format!("read {}: {e}", file.display()))?;
        let mut task = Task::parse(&text, &stem)?;
        task.resolve_relative_paths(dir);
        tasks.push(task);
    }
    if let Some(only) = only {
        for id in only {
            if !tasks.iter().any(|t| t.id == *id) {
                return Err(format!("no task '{id}' in {}", dir.display()));
            }
        }
    }
    if tasks.is_empty() {
        return Err(format!("no tasks in {}", dir.display()));
    }
    Ok(tasks)
}

#[cfg(test)]
mod tests {
    use super::*;

    const TASK: &str = r#"
id: greet
title: Say goodbye
tags: [bugfix]
prompt: Make greet.txt say goodbye.
setup:
  files:
    greet.txt: "hello world\n"
oracle:
  command: grep -q goodbye greet.txt
  timeout_s: 10
reference:
  files:
    greet.txt: "goodbye world\n"
answers: [yes]
max_rounds: 12
"#;

    #[test]
    fn a_task_file_parses_with_defaults_filled_in() {
        let task = Task::parse(TASK, "greet").unwrap();
        assert_eq!(task.title, "Say goodbye");
        assert_eq!(task.tags, vec!["bugfix"]);
        assert_eq!(task.setup.files["greet.txt"], "hello world\n");
        assert_eq!(task.oracle.timeout_s, 10);
        assert_eq!(task.check_command(), "grep -q goodbye greet.txt");
        assert_eq!(task.answers, vec!["yes"]);
        assert_eq!(task.max_rounds, Some(12));
        let no_timeout = TASK.replace("  timeout_s: 10\n", "");
        assert_eq!(
            Task::parse(&no_timeout, "greet").unwrap().oracle.timeout_s,
            DEFAULT_ORACLE_TIMEOUT_S
        );
    }

    #[test]
    fn a_malformed_task_is_refused_with_its_reason() {
        let cases = [
            (TASK.to_string(), "other", "must equal the file name"),
            (
                TASK.replace("timeout_s", "timeout_secs"),
                "greet",
                "unknown field",
            ),
            (
                TASK.replace(
                    "reference:\n  files:\n    greet.txt: \"goodbye world\\n\"\n",
                    "reference: {}\n",
                ),
                "greet",
                "reference needs files or git_commit or patch",
            ),
            (
                TASK.replace("greet.txt: \"hello", "../x: \"hello"),
                "greet",
                "inside the task",
            ),
            (
                TASK.replace(
                    "  files:\n    greet.txt: \"goodbye world\\n\"",
                    "  git_commit: abc",
                ),
                "greet",
                "needs setup.git",
            ),
        ];
        for (text, stem, want) in cases {
            let err = Task::parse(&text, stem).unwrap_err();
            assert!(err.contains(want), "{want}: {err}");
        }
    }

    #[test]
    fn a_taskset_loads_in_file_order_and_can_be_narrowed() {
        let dir = tempfile::tempdir().unwrap();
        for id in ["b-task", "a-task"] {
            std::fs::write(
                dir.path().join(format!("{id}.yaml")),
                TASK.replace("id: greet", &format!("id: {id}")),
            )
            .unwrap();
        }
        std::fs::write(dir.path().join("README.md"), "not a task").unwrap();
        let all = load_taskset(dir.path(), None).unwrap();
        assert_eq!(
            all.iter().map(|t| t.id.as_str()).collect::<Vec<_>>(),
            ["a-task", "b-task"]
        );
        let one = load_taskset(dir.path(), Some(&["b-task".to_string()])).unwrap();
        assert_eq!(one.len(), 1);
        assert!(load_taskset(dir.path(), Some(&["c-task".to_string()])).is_err());
    }

    /// A task set travels with its repository: a relative `setup.git.repo` or
    /// symlink target is read from the task file's directory, not from
    /// wherever the runner was started.
    #[test]
    fn relative_repo_and_link_paths_resolve_against_the_task_file() {
        let root = tempfile::tempdir().unwrap();
        let tasks = root.path().join("eval").join("tasks");
        std::fs::create_dir_all(&tasks).unwrap();
        std::fs::create_dir_all(root.path().join("node_modules")).unwrap();
        std::fs::write(
            tasks.join("hist.yaml"),
            "id: hist\ntitle: t\nprompt: p\nsetup:\n  git: {repo: ../.., commit: abc}\n\
             \x20 symlinks: {node_modules: ../../node_modules}\n\
             oracle: {command: 'true'}\nreference: {git_commit: def}\n",
        )
        .unwrap();
        std::fs::write(
            tasks.join("remote.yaml"),
            "id: remote\ntitle: t\nprompt: p\n\
             setup: {git: {repo: 'https://github.com/a/b', commit: abc}}\n\
             oracle: {command: 'true'}\nreference: {git_commit: def}\n",
        )
        .unwrap();
        let loaded = load_taskset(&tasks, None).unwrap();
        let root = root.path().canonicalize().unwrap();
        assert_eq!(
            loaded[0].setup.git.as_ref().unwrap().repo,
            root.to_string_lossy()
        );
        assert_eq!(
            loaded[0].setup.symlinks["node_modules"],
            root.join("node_modules").to_string_lossy()
        );
        assert_eq!(
            loaded[1].setup.git.as_ref().unwrap().repo,
            "https://github.com/a/b"
        );
    }
}
