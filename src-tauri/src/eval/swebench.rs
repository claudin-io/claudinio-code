//! `import-swebench`: SWE-bench instance records → task files.
//!
//! No dataset is downloaded: the input is a JSONL file the user already has
//! (one instance per line, the fields of the published datasets). Each record
//! becomes a task whose start tree is the repository at `base_commit`, whose
//! oracle applies `test_patch` and runs the `FAIL_TO_PASS` tests, and whose
//! reference is the gold `patch`.
//!
//! The oracle runs on the host. SWE-bench's own harness runs each instance in
//! a prepared Docker image, because most repositories need a specific Python
//! and pinned dependencies; here that environment is the user's to provide
//! (run the eval inside the instance image, or a matching virtualenv).

use super::task::{GitSource, Oracle, Reference, Setup, Task};
use serde_json::Value;

pub(crate) const DEFAULT_TEST_CMD_TEMPLATE: &str = "python -m pytest {tests}";
/// SWE-bench test runs are slow: whole Django suites, C extensions to build.
const ORACLE_TIMEOUT_S: u64 = 1800;

/// `FAIL_TO_PASS` / `PASS_TO_PASS`: a JSON array, or (as the Hugging Face
/// exports ship them) a string holding one.
fn test_list(record: &Value, key: &str) -> Result<Vec<String>, String> {
    let list = match record.get(key) {
        None | Some(Value::Null) => return Ok(Vec::new()),
        Some(Value::String(s)) => {
            serde_json::from_str::<Value>(s).map_err(|e| format!("{key}: {e}"))?
        }
        Some(other) => other.clone(),
    };
    list.as_array()
        .ok_or_else(|| format!("{key} is not a list"))?
        .iter()
        .map(|t| {
            t.as_str()
                .map(str::to_string)
                .ok_or_else(|| format!("{key} holds a non-string"))
        })
        .collect()
}

/// Single-quote `s` for bash.
fn shell_quote(s: &str) -> String {
    format!("'{}'", s.replace('\'', r"'\''"))
}

fn fill(template: &str, tests: &[String]) -> String {
    let joined: Vec<String> = tests.iter().map(|t| shell_quote(t)).collect();
    template
        .replace("{tests}", &joined.join(" "))
        .trim()
        .to_string()
}

/// A task id from an instance id: lowercase, anything outside `[a-z0-9-]`
/// becomes `-` (`django__django-11099` → `django--django-11099`).
fn task_id(instance_id: &str) -> String {
    instance_id
        .to_ascii_lowercase()
        .chars()
        .map(|c| {
            if c.is_ascii_lowercase() || c.is_ascii_digit() || c == '-' {
                c
            } else {
                '-'
            }
        })
        .collect()
}

/// Convert one instance record.
pub(crate) fn convert(record: &Value, template: &str) -> Result<Task, String> {
    let field = |key: &str| -> Result<String, String> {
        record
            .get(key)
            .and_then(Value::as_str)
            .map(str::to_string)
            .ok_or_else(|| format!("missing '{key}'"))
    };
    let instance_id = field("instance_id")?;
    let repo = field("repo")?;
    let fail_to_pass = test_list(record, "FAIL_TO_PASS")?;
    if fail_to_pass.is_empty() {
        return Err(format!("{instance_id}: FAIL_TO_PASS is empty"));
    }
    let pass_to_pass = test_list(record, "PASS_TO_PASS")?;
    let task = Task {
        id: task_id(&instance_id),
        title: instance_id.clone(),
        tags: vec!["swebench".into(), repo.clone()],
        prompt: field("problem_statement")?,
        setup: Setup {
            git: Some(GitSource {
                repo: format!("https://github.com/{repo}"),
                commit: field("base_commit")?,
            }),
            ..Setup::default()
        },
        // What the agent may run while working: the tests that already pass
        // (the failing ones arrive with the oracle's patch), or the suite.
        check: Some(fill(template, &pass_to_pass)),
        oracle: Oracle {
            command: fill(template, &fail_to_pass),
            timeout_s: ORACLE_TIMEOUT_S,
            files: Default::default(),
            patch: Some(field("test_patch")?),
            reference_files: Vec::new(),
        },
        reference: Reference {
            patch: Some(field("patch")?),
            ..Reference::default()
        },
        answers: Vec::new(),
        max_rounds: None,
    };
    // Round-trip through the parser: what is written must load.
    let yaml = serde_yaml::to_string(&task).map_err(|e| format!("{instance_id}: {e}"))?;
    Task::parse(&yaml, &task.id)
}

/// Convert every record of `jsonl` and write `<out_dir>/<id>.yaml`. Returns
/// the ids written.
pub(crate) fn import(
    jsonl: &str,
    out_dir: &std::path::Path,
    template: &str,
) -> Result<Vec<String>, String> {
    std::fs::create_dir_all(out_dir).map_err(|e| format!("create {}: {e}", out_dir.display()))?;
    let mut ids = Vec::new();
    for (n, line) in jsonl.lines().enumerate() {
        if line.trim().is_empty() {
            continue;
        }
        let record: Value =
            serde_json::from_str(line).map_err(|e| format!("line {}: {e}", n + 1))?;
        let task = convert(&record, template).map_err(|e| format!("line {}: {e}", n + 1))?;
        let yaml = serde_yaml::to_string(&task).map_err(|e| format!("{}: {e}", task.id))?;
        std::fs::write(out_dir.join(format!("{}.yaml", task.id)), yaml)
            .map_err(|e| format!("write {}: {e}", task.id))?;
        ids.push(task.id);
    }
    Ok(ids)
}

#[cfg(test)]
mod tests {
    use super::*;

    fn sample() -> Value {
        serde_json::json!({
            "instance_id": "astropy__astropy-12907",
            "repo": "astropy/astropy",
            "base_commit": "d16bfe05a744909de4b27f5875fe0d4ed41ce607",
            "problem_statement": "Modeling's `separability_matrix` does not compute separability correctly for nested CompoundModels",
            "patch": "diff --git a/astropy/modeling/separable.py b/astropy/modeling/separable.py\n",
            "test_patch": "diff --git a/astropy/modeling/tests/test_separable.py b/astropy/modeling/tests/test_separable.py\n",
            "FAIL_TO_PASS": "[\"astropy/modeling/tests/test_separable.py::test_separable[compound_model6-result6]\", \"astropy/modeling/tests/test_separable.py::test_separable[compound_model9-result9]\"]",
            "PASS_TO_PASS": ["astropy/modeling/tests/test_separable.py::test_coord_matrix"],
            "version": "4.3"
        })
    }

    #[test]
    fn a_swebench_record_becomes_a_task_file() {
        let task = convert(&sample(), DEFAULT_TEST_CMD_TEMPLATE).unwrap();
        assert_eq!(task.id, "astropy--astropy-12907");
        assert_eq!(task.title, "astropy__astropy-12907");
        assert_eq!(task.tags, vec!["swebench", "astropy/astropy"]);
        assert!(task.prompt.starts_with("Modeling's `separability_matrix`"));
        let git = task.setup.git.as_ref().unwrap();
        assert_eq!(git.repo, "https://github.com/astropy/astropy");
        assert_eq!(git.commit, "d16bfe05a744909de4b27f5875fe0d4ed41ce607");
        assert_eq!(
            task.oracle.command,
            "python -m pytest 'astropy/modeling/tests/test_separable.py::test_separable[compound_model6-result6]' \
             'astropy/modeling/tests/test_separable.py::test_separable[compound_model9-result9]'"
        );
        assert!(
            task.oracle
                .patch
                .as_deref()
                .unwrap()
                .contains("test_separable.py")
        );
        assert!(
            task.reference
                .patch
                .as_deref()
                .unwrap()
                .contains("separable.py")
        );
        assert_eq!(
            task.check_command(),
            "python -m pytest 'astropy/modeling/tests/test_separable.py::test_coord_matrix'"
        );
    }

    #[test]
    fn the_template_and_missing_fields_are_honoured() {
        let task = convert(&sample(), "tox -e py -- {tests}").unwrap();
        assert!(task.oracle.command.starts_with("tox -e py -- 'astropy/"));
        let mut no_tests = sample();
        no_tests["FAIL_TO_PASS"] = serde_json::json!([]);
        assert!(convert(&no_tests, DEFAULT_TEST_CMD_TEMPLATE).is_err());
        let mut no_commit = sample();
        no_commit.as_object_mut().unwrap().remove("base_commit");
        let err = convert(&no_commit, DEFAULT_TEST_CMD_TEMPLATE).unwrap_err();
        assert!(err.contains("base_commit"), "{err}");
    }

    #[test]
    fn import_writes_one_loadable_file_per_record() {
        let out = tempfile::tempdir().unwrap();
        let mut second = sample();
        second["instance_id"] = "django__django-11099".into();
        second["repo"] = "django/django".into();
        let jsonl = format!("{}\n\n{}\n", sample(), second);
        let ids = import(&jsonl, out.path(), DEFAULT_TEST_CMD_TEMPLATE).unwrap();
        assert_eq!(ids, vec!["astropy--astropy-12907", "django--django-11099"]);
        let loaded = super::super::task::load_taskset(out.path(), None).unwrap();
        assert_eq!(loaded.len(), 2);
        assert_eq!(
            loaded[1].setup.git.as_ref().unwrap().repo,
            "https://github.com/django/django"
        );
    }
}
