//! Turning a task file into a directory, and running its oracle there.
//!
//! Every tree is materialized fresh into a temporary directory outside any git
//! repository, then made a repository of its own with one commit, `start`. The
//! commit is what makes the harness behave as it does for a user: `git diff`
//! shows the session's changes, the quality digest sees an edit, and the
//! finish-line check knows code changed. A task whose start state comes from
//! git is exported with `git archive` rather than cloned, so the model cannot
//! read the reference solution out of the history it was given.

use super::task::{Task, is_relative_inside};
use std::io::Read;
use std::path::{Path, PathBuf};
use std::process::{Command, Stdio};
use std::time::{Duration, Instant};

/// Which tree of a task to build.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub(crate) enum Tree {
    /// What the agent starts from.
    Start,
    /// The start state with the known solution applied.
    Reference,
}

/// Build `which` tree of `task` in `dest` (created if missing, expected empty).
pub(crate) fn materialize(task: &Task, dest: &Path, which: Tree) -> Result<(), String> {
    std::fs::create_dir_all(dest).map_err(|e| format!("create {}: {e}", dest.display()))?;
    if let Some(enclosing) = enclosing_repo(dest) {
        return Err(format!(
            "{} is inside the git repository at {}; tasks are materialized outside any repository",
            dest.display(),
            enclosing.display()
        ));
    }
    match (&task.setup.git, which, &task.reference.git_commit) {
        (Some(git), Tree::Reference, Some(sha)) => export_git_tree(&git.repo, sha, dest)?,
        (Some(git), _, _) => export_git_tree(&git.repo, &git.commit, dest)?,
        (None, _, _) => write_files(dest, &task.setup.files)?,
    }
    if which == Tree::Reference {
        write_files(dest, &task.reference.files)?;
        if let Some(patch) = &task.reference.patch {
            apply_patch(dest, patch, "reference patch")?;
        }
    }
    for (path, target) in &task.setup.symlinks {
        let link = dest.join(path);
        if let Some(parent) = link.parent() {
            std::fs::create_dir_all(parent).map_err(|e| format!("create {path}: {e}"))?;
        }
        symlink(Path::new(target), &link).map_err(|e| format!("link {path} -> {target}: {e}"))?;
    }
    init_repo(dest, task.setup.symlinks.keys())
}

/// The nearest ancestor of `dir` that holds a `.git`, if any.
fn enclosing_repo(dir: &Path) -> Option<PathBuf> {
    let dir = dir.canonicalize().ok()?;
    dir.ancestors()
        .find(|a| a.join(".git").exists())
        .map(Path::to_path_buf)
}

pub(crate) fn write_files(
    dest: &Path,
    files: &std::collections::BTreeMap<String, String>,
) -> Result<(), String> {
    for (path, content) in files {
        if !is_relative_inside(path) {
            return Err(format!("path '{path}' leaves the task directory"));
        }
        let file = dest.join(path);
        if let Some(parent) = file.parent() {
            std::fs::create_dir_all(parent).map_err(|e| format!("create {path}: {e}"))?;
        }
        std::fs::write(&file, content).map_err(|e| format!("write {path}: {e}"))?;
    }
    Ok(())
}

#[cfg(unix)]
fn symlink(target: &Path, link: &Path) -> std::io::Result<()> {
    std::os::unix::fs::symlink(target, link)
}

#[cfg(windows)]
fn symlink(target: &Path, link: &Path) -> std::io::Result<()> {
    if target.is_dir() {
        std::os::windows::fs::symlink_dir(target, link)
    } else {
        std::os::windows::fs::symlink_file(target, link)
    }
}

/// `git` with an identity and no user config in the way: the commit has to
/// succeed on a machine with no `user.name`, and must not be signed.
fn git(dir: &Path) -> Command {
    let mut cmd = Command::new("git");
    cmd.current_dir(dir)
        .args(["-c", "user.name=claudinio-eval"])
        .args(["-c", "user.email=eval@claudinio.invalid"])
        .args(["-c", "commit.gpgsign=false"])
        .args(["-c", "init.defaultBranch=main"])
        .stdin(Stdio::null());
    cmd
}

fn run_git(cmd: &mut Command, what: &str) -> Result<Vec<u8>, String> {
    let out = cmd.output().map_err(|e| format!("{what}: {e}"))?;
    if !out.status.success() {
        return Err(format!(
            "{what}: {}",
            String::from_utf8_lossy(&out.stderr).trim()
        ));
    }
    Ok(out.stdout)
}

fn init_repo<'a>(dest: &Path, excluded: impl Iterator<Item = &'a String>) -> Result<(), String> {
    run_git(git(dest).args(["init", "-q"]), "git init")?;
    let exclude: String = excluded.map(|p| format!("/{p}\n")).collect();
    if !exclude.is_empty() {
        let info = dest.join(".git").join("info");
        std::fs::create_dir_all(&info).map_err(|e| format!("git info: {e}"))?;
        let path = info.join("exclude");
        let mut current = std::fs::read_to_string(&path).unwrap_or_default();
        current.push_str(&exclude);
        std::fs::write(&path, current).map_err(|e| format!("git exclude: {e}"))?;
    }
    run_git(git(dest).args(["add", "-A"]), "git add")?;
    run_git(
        git(dest).args([
            "commit",
            "-q",
            "--no-verify",
            "--allow-empty",
            "-m",
            "start",
        ]),
        "git commit",
    )?;
    Ok(())
}

/// Keep `path` out of the start commit and out of `git status` (a file the
/// runner itself writes, like the quality config).
pub(crate) fn exclude_from_git(dest: &Path, path: &str) -> Result<(), String> {
    let file = dest.join(".git").join("info").join("exclude");
    let mut current = std::fs::read_to_string(&file).unwrap_or_default();
    current.push_str(&format!("/{path}\n"));
    std::fs::write(&file, current).map_err(|e| format!("git exclude: {e}"))
}

/// Unpack the tree of `commit` from `repo` (a local path or a clone URL) into
/// `dest`, without its history.
fn export_git_tree(repo: &str, commit: &str, dest: &Path) -> Result<(), String> {
    let local = Path::new(repo);
    let clone_dir;
    let source = if local.is_dir() {
        local.to_path_buf()
    } else {
        clone_dir = tempfile::Builder::new()
            .prefix("claudinio-eval-clone-")
            .tempdir()
            .map_err(|e| format!("clone dir: {e}"))?;
        run_git(
            Command::new("git")
                .args(["clone", "-q", "--no-checkout", repo])
                .arg(clone_dir.path())
                .stdin(Stdio::null()),
            &format!("git clone {repo}"),
        )?;
        clone_dir.path().to_path_buf()
    };
    let tar = run_git(
        Command::new("git")
            .arg("-C")
            .arg(&source)
            .args(["archive", "--format=tar", commit])
            .stdin(Stdio::null()),
        &format!("git archive {commit}"),
    )?;
    tar::Archive::new(std::io::Cursor::new(tar))
        .unpack(dest)
        .map_err(|e| format!("unpack {commit}: {e}"))
}

/// The bytes of `path` at `commit` in `repo` (a local repository).
fn file_at_commit(repo: &str, commit: &str, path: &str) -> Result<Vec<u8>, String> {
    run_git(
        Command::new("git")
            .arg("-C")
            .arg(repo)
            .args(["show", &format!("{commit}:{path}")])
            .stdin(Stdio::null()),
        &format!("git show {commit}:{path}"),
    )
}

/// What one shell command did.
#[derive(Debug, Clone)]
pub(crate) struct CommandOutcome {
    pub passed: bool,
    pub exit_code: Option<i32>,
    pub timed_out: bool,
    /// The last ~2,000 characters of combined stdout and stderr.
    pub tail: String,
    pub secs: f64,
}

/// Run the task's oracle in `dir`: write its hidden files, apply its patch,
/// copy the reference files it names, then run its command.
pub(crate) fn run_oracle(task: &Task, dir: &Path) -> CommandOutcome {
    if let Err(e) = prepare_oracle(task, dir) {
        return CommandOutcome {
            passed: false,
            exit_code: None,
            timed_out: false,
            tail: format!("oracle setup failed: {e}"),
            secs: 0.0,
        };
    }
    run_shell(
        &task.oracle.command,
        dir,
        Duration::from_secs(task.oracle.timeout_s),
    )
}

fn prepare_oracle(task: &Task, dir: &Path) -> Result<(), String> {
    write_files(dir, &task.oracle.files)?;
    if let (Some(git), Some(sha)) = (&task.setup.git, &task.reference.git_commit) {
        for path in &task.oracle.reference_files {
            let bytes = file_at_commit(&git.repo, sha, path)?;
            let file = dir.join(path);
            if let Some(parent) = file.parent() {
                std::fs::create_dir_all(parent).map_err(|e| format!("create {path}: {e}"))?;
            }
            std::fs::write(&file, bytes).map_err(|e| format!("write {path}: {e}"))?;
        }
    }
    if let Some(patch) = &task.oracle.patch {
        apply_patch(dir, patch, "oracle patch")?;
    }
    Ok(())
}

/// `git apply` a unified diff in `dir`, which need not be a repository yet.
fn apply_patch(dir: &Path, patch: &str, what: &str) -> Result<(), String> {
    let mut file = tempfile::NamedTempFile::new().map_err(|e| format!("{what}: {e}"))?;
    std::io::Write::write_all(&mut file, patch.as_bytes()).map_err(|e| format!("{what}: {e}"))?;
    run_git(
        git(dir)
            .args(["apply", "--whitespace=nowarn"])
            .arg(file.path()),
        &format!("git apply ({what})"),
    )
    .map(|_| ())
}

/// `bash -c command` in `cwd`, killed (with everything it started) after
/// `timeout`. Output goes to a file rather than a pipe, so a chatty test suite
/// cannot fill a pipe and hang the runner.
pub(crate) fn run_shell(command: &str, cwd: &Path, timeout: Duration) -> CommandOutcome {
    let started = Instant::now();
    let fail = |tail: String| CommandOutcome {
        passed: false,
        exit_code: None,
        timed_out: false,
        tail,
        secs: started.elapsed().as_secs_f64(),
    };
    let log = match tempfile::NamedTempFile::new() {
        Ok(f) => f,
        Err(e) => return fail(format!("log file: {e}")),
    };
    let (out, err) = match (log.reopen(), log.reopen()) {
        (Ok(o), Ok(e)) => (o, e),
        _ => return fail("log file: reopen failed".into()),
    };
    let mut cmd = Command::new("bash");
    cmd.arg("-c")
        .arg(command)
        .current_dir(cwd)
        .env("CI", "1")
        .stdin(Stdio::null())
        .stdout(out)
        .stderr(err);
    #[cfg(unix)]
    {
        use std::os::unix::process::CommandExt;
        cmd.process_group(0);
    }
    let mut child = match cmd.spawn() {
        Ok(c) => c,
        Err(e) => return fail(format!("spawn bash: {e}")),
    };
    let mut timed_out = false;
    let status = loop {
        match child.try_wait() {
            Ok(Some(status)) => break Some(status),
            Ok(None) if started.elapsed() >= timeout => {
                timed_out = true;
                kill_tree(&mut child);
                break child.wait().ok();
            }
            Ok(None) => std::thread::sleep(Duration::from_millis(25)),
            Err(_) => break None,
        }
    };
    let mut text = String::new();
    if let Ok(mut f) = log.reopen() {
        let mut bytes = Vec::new();
        let _ = f.read_to_end(&mut bytes);
        text = String::from_utf8_lossy(&bytes).into_owned();
    }
    let exit_code = status.and_then(|s| s.code());
    CommandOutcome {
        passed: !timed_out && exit_code == Some(0),
        exit_code,
        timed_out,
        tail: tail(&text, 2_000),
        secs: started.elapsed().as_secs_f64(),
    }
}

#[cfg(unix)]
fn kill_tree(child: &mut std::process::Child) {
    // The child leads its own process group, so the group id is its pid.
    let _ = Command::new("kill")
        .args(["-KILL", "--", &format!("-{}", child.id())])
        .stdin(Stdio::null())
        .stdout(Stdio::null())
        .stderr(Stdio::null())
        .status();
    let _ = child.kill();
}

#[cfg(not(unix))]
fn kill_tree(child: &mut std::process::Child) {
    let _ = child.kill();
}

fn tail(text: &str, max: usize) -> String {
    let count = text.chars().count();
    if count <= max {
        return text.to_string();
    }
    text.chars().skip(count - max).collect()
}

#[cfg(test)]
mod tests {
    use super::*;

    fn task(yaml: &str) -> Task {
        Task::parse(yaml, "t").unwrap()
    }

    const FILES_TASK: &str = r#"
id: t
title: t
prompt: p
setup:
  files:
    src/greet.txt: "hello\n"
  symlinks:
    shared: /tmp
oracle:
  command: grep -q goodbye src/greet.txt && test -f hidden.txt
  timeout_s: 10
  files:
    hidden.txt: "x"
reference:
  files:
    src/greet.txt: "goodbye\n"
"#;

    #[test]
    fn a_files_task_materializes_as_a_fresh_repository_with_one_commit() {
        let tmp = tempfile::tempdir().unwrap();
        let t = task(FILES_TASK);
        let start = tmp.path().join("start");
        materialize(&t, &start, Tree::Start).unwrap();
        assert_eq!(
            std::fs::read_to_string(start.join("src/greet.txt")).unwrap(),
            "hello\n"
        );
        assert!(start.join("shared").is_symlink());
        let log = run_git(git(&start).args(["log", "--oneline"]), "log").unwrap();
        assert_eq!(String::from_utf8_lossy(&log).lines().count(), 1);
        // The link is not part of the start commit, and nothing is dirty.
        let status = run_git(git(&start).args(["status", "--porcelain"]), "status").unwrap();
        assert!(status.is_empty(), "{}", String::from_utf8_lossy(&status));

        let reference = tmp.path().join("reference");
        materialize(&t, &reference, Tree::Reference).unwrap();
        assert_eq!(
            std::fs::read_to_string(reference.join("src/greet.txt")).unwrap(),
            "goodbye\n"
        );
    }

    #[test]
    fn the_oracle_fails_on_the_start_tree_and_passes_on_the_reference() {
        let tmp = tempfile::tempdir().unwrap();
        let t = task(FILES_TASK);
        let (start, reference) = (tmp.path().join("s"), tmp.path().join("r"));
        materialize(&t, &start, Tree::Start).unwrap();
        materialize(&t, &reference, Tree::Reference).unwrap();
        assert!(!run_oracle(&t, &start).passed);
        let ok = run_oracle(&t, &reference);
        assert!(ok.passed, "{ok:?}");
        // The hidden file was written just before the command ran.
        assert!(reference.join("hidden.txt").exists());
    }

    #[test]
    fn a_command_over_its_timeout_is_killed_and_fails() {
        let tmp = tempfile::tempdir().unwrap();
        let out = run_shell("sleep 30", tmp.path(), Duration::from_millis(300));
        assert!(out.timed_out && !out.passed);
        assert!(out.secs < 10.0, "{}", out.secs);
        let out = run_shell("echo visible; exit 3", tmp.path(), Duration::from_secs(10));
        assert_eq!(out.exit_code, Some(3));
        assert!(out.tail.contains("visible"));
    }

    #[test]
    fn a_git_task_is_exported_without_its_history() {
        let tmp = tempfile::tempdir().unwrap();
        let repo = tmp.path().join("repo");
        std::fs::create_dir_all(&repo).unwrap();
        run_git(git(&repo).args(["init", "-q"]), "init").unwrap();
        std::fs::write(repo.join("a.txt"), "one\n").unwrap();
        run_git(git(&repo).args(["add", "-A"]), "add").unwrap();
        run_git(git(&repo).args(["commit", "-q", "-m", "one"]), "c1").unwrap();
        let first =
            String::from_utf8(run_git(git(&repo).args(["rev-parse", "HEAD"]), "rev").unwrap())
                .unwrap();
        std::fs::write(repo.join("a.txt"), "two\n").unwrap();
        std::fs::write(repo.join("a_test.txt"), "two\n").unwrap();
        run_git(git(&repo).args(["add", "-A"]), "add").unwrap();
        run_git(git(&repo).args(["commit", "-q", "-m", "two"]), "c2").unwrap();
        let second =
            String::from_utf8(run_git(git(&repo).args(["rev-parse", "HEAD"]), "rev").unwrap())
                .unwrap();
        let yaml = format!(
            "id: t\ntitle: t\nprompt: p\nsetup:\n  git: {{repo: '{}', commit: '{}'}}\n\
             oracle:\n  command: diff a.txt a_test.txt\n  reference_files: [a_test.txt]\n\
             reference:\n  git_commit: '{}'\n",
            repo.display(),
            first.trim(),
            second.trim()
        );
        let t = task(&yaml);
        let start = tmp.path().join("start");
        materialize(&t, &start, Tree::Start).unwrap();
        assert_eq!(
            std::fs::read_to_string(start.join("a.txt")).unwrap(),
            "one\n"
        );
        let log = run_git(git(&start).args(["log", "--all", "--oneline"]), "log").unwrap();
        assert_eq!(
            String::from_utf8_lossy(&log).lines().count(),
            1,
            "only the start commit, never the solution's"
        );
        // The oracle's test file comes from the reference commit.
        assert!(!run_oracle(&t, &start).passed);
        let reference = tmp.path().join("reference");
        materialize(&t, &reference, Tree::Reference).unwrap();
        assert!(run_oracle(&t, &reference).passed);
    }
}
