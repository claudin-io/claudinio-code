//! The headless eval: a fixed task set, run through the real agent loop, scored
//! by each task's own oracle (lean-harness plan, Phase 0).
//!
//! Every later change to the harness is gated on three numbers this produces:
//! **M1**, billed cost per solved task (cache-aware); **M2**, provider rounds
//! per task; **M3**, the pass rate. Token counts alone are not the metric —
//! a leaner context that costs more turns can cost more money.
//!
//! The entry point is the `claudinio-eval` example (`examples/claudinio-eval.rs`),
//! a thin `main` over [`cli`]. It lives inside the crate because it drives
//! `crate::agent`, which the app keeps private; nothing here is reachable from
//! the app's IPC surface, and the one switch it adds to the core
//! (`ToolContext::shell_gate`) only ever adds a prompt.
//!
//! See `eval/README.md` at the repository root for the task format, the
//! approval policy and how to read the results.

mod materialize;
pub(crate) mod metrics;
mod policy;
mod runner;
mod swebench;
mod task;
mod validate;

use crate::agent::prompt::PromptVersion;
use metrics::{Prices, cache_read_share, median};
use runner::{RunOptions, TaskResult};
use serde::Serialize;
use std::path::PathBuf;
use std::time::Duration;

const USAGE: &str = "\
usage: claudinio-eval <command> ...

  validate <taskset-dir> [--tasks id,id] [--work-dir DIR]
      Check every task without a model: its oracle must fail on the start tree
      and pass on the reference solution.

  run <taskset-dir> --model ID --base-url URL --api-key-env ENV
      [--protocol anthropic|openai] [--tasks id,id] [--out results.jsonl]
      [--max-rounds N] [--context-window TOKENS] [--task-timeout-s SECS]
      [--price-in USD --price-out USD [--price-cache-read USD] [--price-cache-write USD]]
      [--jev-api-key-env ENV] [--work-dir DIR] [--prompt-version v1|v2]
      One fresh Builder session per task against the given provider, then the
      oracle. Prices are USD per million tokens, used only for requests whose
      provider reports no cost. --prompt-version picks the Standard system
      prompt (default v2); each result line records it.

  import-swebench <instances.jsonl> <out-dir> [--test-cmd-template 'python -m pytest {tests}']
      Convert SWE-bench instance records into task files.
";

/// Run the eval command line; returns the process exit code.
pub async fn cli(args: Vec<String>) -> i32 {
    let Some((command, rest)) = args.split_first() else {
        eprint!("{USAGE}");
        return 2;
    };
    let parsed = match Args::parse(rest) {
        Ok(a) => a,
        Err(e) => {
            eprintln!("{e}\n\n{USAGE}");
            return 2;
        }
    };
    let result = match command.as_str() {
        "validate" => cmd_validate(&parsed),
        "run" => cmd_run(&parsed).await,
        "import-swebench" => cmd_import(&parsed),
        "help" | "--help" | "-h" => {
            print!("{USAGE}");
            Ok(0)
        }
        other => Err(format!("unknown command '{other}'")),
    };
    match result {
        Ok(code) => code,
        Err(e) => {
            eprintln!("claudinio-eval: {e}");
            1
        }
    }
}

/// Positional arguments and `--flag value` pairs, in the order given.
struct Args {
    positional: Vec<String>,
    flags: Vec<(String, String)>,
}

impl Args {
    fn parse(args: &[String]) -> Result<Args, String> {
        let mut positional = Vec::new();
        let mut flags = Vec::new();
        let mut it = args.iter();
        while let Some(arg) = it.next() {
            if let Some(name) = arg.strip_prefix("--") {
                let value = it.next().ok_or_else(|| format!("--{name} needs a value"))?;
                flags.push((name.to_string(), value.clone()));
            } else {
                positional.push(arg.clone());
            }
        }
        Ok(Args { positional, flags })
    }

    fn flag(&self, name: &str) -> Option<&str> {
        self.flags
            .iter()
            .rev()
            .find(|(n, _)| n == name)
            .map(|(_, v)| v.as_str())
    }

    fn required(&self, name: &str) -> Result<&str, String> {
        self.flag(name)
            .ok_or_else(|| format!("--{name} is required"))
    }

    fn number<T: std::str::FromStr>(&self, name: &str) -> Result<Option<T>, String> {
        self.flag(name)
            .map(|v| {
                v.parse::<T>()
                    .map_err(|_| format!("--{name}: not a number: {v}"))
            })
            .transpose()
    }

    fn positional(&self, index: usize, what: &str) -> Result<&str, String> {
        self.positional
            .get(index)
            .map(String::as_str)
            .ok_or_else(|| format!("missing {what}"))
    }

    fn tasks(&self) -> Option<Vec<String>> {
        self.flag("tasks").map(|v| {
            v.split(',')
                .map(|s| s.trim().to_string())
                .filter(|s| !s.is_empty())
                .collect()
        })
    }

    /// `--prompt-version v1|v2`; v2 when absent.
    fn prompt_version(&self) -> Result<PromptVersion, String> {
        match self.flag("prompt-version") {
            None => Ok(PromptVersion::default()),
            Some("v1") => Ok(PromptVersion::V1),
            Some("v2") => Ok(PromptVersion::V2),
            Some(other) => Err(format!("--prompt-version must be v1 or v2, not {other}")),
        }
    }

    /// `--work-dir`, or a fresh temporary directory (kept by `run`, and by a
    /// `validate` with an invalid task, so a failure can be inspected; its
    /// path is printed).
    fn work_dir(&self) -> Result<WorkDir, String> {
        match self.flag("work-dir") {
            Some(dir) => Ok(WorkDir {
                path: PathBuf::from(dir),
                temporary: false,
            }),
            None => tempfile::Builder::new()
                .prefix("claudinio-eval-")
                .tempdir()
                .map(|d| WorkDir {
                    path: d.keep(),
                    temporary: true,
                })
                .map_err(|e| format!("work dir: {e}")),
        }
    }
}

/// Where a command materializes its tasks, and whether this run made it.
struct WorkDir {
    path: PathBuf,
    /// Made by this run rather than named with `--work-dir`.
    temporary: bool,
}

/// Where `run` writes its JSONL: `--out`, else `results.jsonl` in the work dir.
fn results_path(args: &Args, work: &WorkDir) -> PathBuf {
    args.flag("out")
        .map(PathBuf::from)
        .unwrap_or_else(|| work.path.join("results.jsonl"))
}

fn cmd_validate(args: &Args) -> Result<i32, String> {
    validate_taskset(args).map(|(code, _)| code)
}

/// `validate`, returning the exit code and the work dir it used.
fn validate_taskset(args: &Args) -> Result<(i32, PathBuf), String> {
    let dir = PathBuf::from(args.positional(0, "<taskset-dir>")?);
    let only = args.tasks();
    let tasks = task::load_taskset(&dir, only.as_deref())?;
    let WorkDir {
        path: work,
        temporary,
    } = args.work_dir()?;
    eprintln!("validating {} tasks in {}", tasks.len(), work.display());
    let mut results = Vec::new();
    for t in &tasks {
        let v = validate::validate_task(t, &work.join(&t.id));
        eprintln!(
            "  {} {}",
            if v.is_valid() { "ok     " } else { "INVALID" },
            t.id
        );
        results.push(v);
    }
    print!("{}", validate::table(&results));
    let code = if results.iter().all(validate::Validation::is_valid) {
        0
    } else {
        1
    };
    // A full validate materializes every start tree and reference, ~235 MB
    // (measured on the shipped set): kept only when there is something to
    // look at — a failing task — or when the user named the directory.
    if code == 0 && temporary {
        std::fs::remove_dir_all(&work).map_err(|e| format!("remove {}: {e}", work.display()))?;
    } else {
        eprintln!("work dir kept: {}", work.display());
    }
    Ok((code, work))
}

async fn cmd_run(args: &Args) -> Result<i32, String> {
    let dir = PathBuf::from(args.positional(0, "<taskset-dir>")?);
    let only = args.tasks();
    let tasks = task::load_taskset(&dir, only.as_deref())?;
    let key_env = args.required("api-key-env")?;
    let api_key = std::env::var(key_env).map_err(|_| format!("${key_env} is not set"))?;
    let jev_api_key = match args.flag("jev-api-key-env") {
        Some(env) => Some(std::env::var(env).map_err(|_| format!("${env} is not set"))?),
        None => None,
    };
    let protocol = args.flag("protocol").unwrap_or("anthropic").to_string();
    if protocol != "anthropic" && protocol != "openai" {
        return Err(format!(
            "--protocol must be anthropic or openai, not {protocol}"
        ));
    }
    let work = args.work_dir()?;
    let opts = RunOptions {
        model: args.required("model")?.to_string(),
        base_url: args.required("base-url")?.to_string(),
        api_key,
        protocol,
        max_rounds: args.number("max-rounds")?,
        context_window: args.number("context-window")?,
        prices: Prices {
            input: args.number("price-in")?,
            output: args.number("price-out")?,
            cache_read: args.number("price-cache-read")?,
            cache_write: args.number("price-cache-write")?,
        },
        jev_api_key,
        work_root: work.path.clone(),
        task_timeout: Duration::from_secs(args.number("task-timeout-s")?.unwrap_or(3_600)),
        prompt_version: args.prompt_version()?,
    };
    let out = results_path(args, &work);
    eprintln!(
        "running {} tasks on {} in {}; results -> {}",
        tasks.len(),
        opts.model,
        opts.work_root.display(),
        out.display()
    );
    let mut lines = String::new();
    let mut results = Vec::new();
    for t in &tasks {
        eprintln!("  {} …", t.id);
        let r = runner::run_task(t, &opts).await;
        eprintln!(
            "  {} {} ({} rounds, {:.0}s, {})",
            if r.pass { "PASS" } else { "fail" },
            r.id,
            r.metrics.rounds,
            r.wall_s,
            r.outcome
        );
        lines.push_str(&serde_json::to_string(&r).map_err(|e| e.to_string())?);
        lines.push('\n');
        std::fs::write(&out, &lines).map_err(|e| format!("write {}: {e}", out.display()))?;
        results.push(r);
    }
    let summary = summarize(&results);
    print!("{}", summary.render());
    Ok(0)
}

fn cmd_import(args: &Args) -> Result<i32, String> {
    let input = args.positional(0, "<instances.jsonl>")?;
    let out = PathBuf::from(args.positional(1, "<out-dir>")?);
    let template = args
        .flag("test-cmd-template")
        .unwrap_or(swebench::DEFAULT_TEST_CMD_TEMPLATE);
    let text = std::fs::read_to_string(input).map_err(|e| format!("read {input}: {e}"))?;
    let ids = swebench::import(&text, &out, template)?;
    println!("wrote {} tasks to {}", ids.len(), out.display());
    Ok(0)
}

/// The numbers a phase is gated on, over one run of the task set.
#[derive(Debug, Serialize, PartialEq)]
pub(crate) struct Summary {
    pub tasks: usize,
    pub passed: usize,
    /// M1: total cost over tasks passed. `None` when nothing passed or some
    /// request's cost is unknown.
    pub cost_per_solved: Option<f64>,
    pub total_cost: f64,
    pub cost_known: bool,
    /// M2: median provider rounds per task (main session and subagents).
    pub median_rounds: Option<f64>,
    /// M3: tasks passed over tasks run.
    pub pass_rate: f64,
    /// Cache reads over all input processed (uncached + cache reads + writes).
    pub cache_read_share: Option<f64>,
    pub subagent_token_share: Option<f64>,
}

pub(crate) fn summarize(results: &[TaskResult]) -> Summary {
    let passed = results.iter().filter(|r| r.pass).count();
    let total_cost: f64 = results.iter().map(|r| r.metrics.cost).sum();
    let cost_known = results.iter().all(|r| r.metrics.cost_known);
    let sum = |f: fn(&TaskResult) -> u64| results.iter().map(f).sum::<u64>();
    let input = sum(|r| r.metrics.input_tokens);
    let read = sum(|r| r.metrics.cache_read_tokens);
    let created = sum(|r| r.metrics.cache_creation_tokens);
    let output = sum(|r| r.metrics.output_tokens);
    let sub = sum(|r| r.metrics.subagent_tokens);
    let all_tokens = input + read + created + output;
    let rounds: Vec<u32> = results.iter().map(|r| r.metrics.rounds).collect();
    Summary {
        tasks: results.len(),
        passed,
        cost_per_solved: (passed > 0 && cost_known).then(|| total_cost / passed as f64),
        total_cost,
        cost_known,
        median_rounds: median(&rounds),
        pass_rate: if results.is_empty() {
            0.0
        } else {
            passed as f64 / results.len() as f64
        },
        cache_read_share: cache_read_share(input, read, created),
        subagent_token_share: (all_tokens > 0).then(|| sub as f64 / all_tokens as f64),
    }
}

impl Summary {
    fn render(&self) -> String {
        let pct = |v: Option<f64>| v.map_or("n/a".into(), |v| format!("{:.1}%", v * 100.0));
        format!(
            "\nM1 cost per solved task: {}\n\
             M2 median rounds per task: {}\n\
             M3 pass rate: {}/{} ({:.1}%)\n\
             total cost: ${:.4}{}\n\
             cache-read share of input: {}\n\
             subagent share of tokens: {}\n",
            self.cost_per_solved
                .map_or("n/a".into(), |c| format!("${c:.4}")),
            self.median_rounds.map_or("n/a".into(), |m| format!("{m}")),
            self.passed,
            self.tasks,
            self.pass_rate * 100.0,
            self.total_cost,
            if self.cost_known {
                ""
            } else {
                " (incomplete: some requests reported no cost; pass --price-in/--price-out)"
            },
            pct(self.cache_read_share),
            pct(self.subagent_token_share),
        )
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use metrics::RunMetrics;

    fn result(pass: bool, rounds: u32, cost: f64, read: u64) -> TaskResult {
        TaskResult {
            id: "t".into(),
            title: "t".into(),
            tags: vec![],
            pass,
            outcome: "completed".into(),
            wall_s: 1.0,
            oracle_exit: Some(0),
            oracle_tail: String::new(),
            prompt_version: PromptVersion::default(),
            metrics: RunMetrics {
                rounds,
                input_tokens: 100,
                cache_read_tokens: read,
                cost,
                cost_known: true,
                ..RunMetrics::default()
            },
        }
    }

    #[test]
    fn the_summary_is_m1_m2_m3_and_the_cache_share() {
        let s = summarize(&[
            result(true, 4, 0.30, 300),
            result(false, 10, 0.50, 100),
            result(true, 6, 0.20, 0),
        ]);
        assert_eq!((s.tasks, s.passed), (3, 2));
        assert!(
            (s.cost_per_solved.unwrap() - 0.5).abs() < 1e-9,
            "1.00 / 2 solved"
        );
        assert_eq!(s.median_rounds, Some(6.0));
        assert!((s.pass_rate - 2.0 / 3.0).abs() < 1e-9);
        assert_eq!(s.cache_read_share, Some(400.0 / 700.0));
        assert!(s.render().contains("M3 pass rate: 2/3"));
    }

    #[test]
    fn m1_is_unknown_when_nothing_passed_or_a_cost_is_missing() {
        assert_eq!(summarize(&[result(false, 3, 0.1, 0)]).cost_per_solved, None);
        let mut unknown = result(true, 3, 0.1, 0);
        unknown.metrics.cost_known = false;
        assert_eq!(summarize(&[unknown]).cost_per_solved, None);
    }

    fn argv(args: &[&str]) -> Args {
        Args::parse(&args.iter().map(|s| s.to_string()).collect::<Vec<_>>()).unwrap()
    }

    /// A task set of one tiny task, valid or not depending on `oracle`.
    fn taskset(oracle: &str) -> tempfile::TempDir {
        let set = tempfile::tempdir().unwrap();
        std::fs::write(
            set.path().join("tiny.yaml"),
            format!(
                "id: tiny\ntitle: tiny\nprompt: fix it\nsetup:\n  files:\n    a.py: 'X = 1'\n\
                 oracle:\n  command: \"{oracle}\"\n  timeout_s: 20\n\
                 reference:\n  files:\n    a.py: 'X = 2'\n"
            ),
        )
        .unwrap();
        set
    }

    // A full validate materializes every start tree and reference: hundreds of
    // megabytes in /tmp per run. When every task is valid nothing in it is
    // worth inspecting.
    #[test]
    fn a_validate_where_every_task_is_valid_leaves_no_temporary_work_dir() {
        let set = taskset("python3 -c 'import a; assert a.X == 2'");
        let (code, work) = validate_taskset(&argv(&[set.path().to_str().unwrap()])).unwrap();
        assert_eq!(code, 0);
        assert!(!work.exists(), "{} was left behind", work.display());
    }

    #[test]
    fn an_invalid_task_keeps_the_work_dir_for_inspection() {
        let set = taskset("false");
        let (code, work) = validate_taskset(&argv(&[set.path().to_str().unwrap()])).unwrap();
        assert_eq!(code, 1);
        assert!(work.join("tiny").exists());
        std::fs::remove_dir_all(&work).ok();
    }

    #[test]
    fn a_work_dir_the_user_named_is_never_removed() {
        let set = taskset("python3 -c 'import a; assert a.X == 2'");
        let mine = tempfile::tempdir().unwrap();
        let (code, work) = validate_taskset(&argv(&[
            set.path().to_str().unwrap(),
            "--work-dir",
            mine.path().to_str().unwrap(),
        ]))
        .unwrap();
        assert_eq!((code, work.as_path()), (0, mine.path()));
        assert!(mine.path().exists());
    }

    // The documented invocation runs from src-tauri/, and a results file
    // written into the cwd lands untracked in the repository.
    #[test]
    fn run_results_default_to_the_work_dir_not_the_current_directory() {
        let work = WorkDir {
            path: PathBuf::from("/w"),
            temporary: true,
        };
        assert_eq!(
            results_path(&argv(&[]), &work),
            PathBuf::from("/w/results.jsonl")
        );
        assert_eq!(
            results_path(&argv(&["--out", "r.jsonl"]), &work),
            PathBuf::from("r.jsonl")
        );
    }

    #[test]
    fn flags_and_positionals_are_read_in_any_order() {
        let argv: Vec<String> = ["dir", "--tasks", "a, b", "--max-rounds", "7"]
            .iter()
            .map(|s| s.to_string())
            .collect();
        let a = Args::parse(&argv).unwrap();
        assert_eq!(a.positional(0, "dir").unwrap(), "dir");
        assert_eq!(a.tasks().unwrap(), vec!["a", "b"]);
        assert_eq!(a.number::<usize>("max-rounds").unwrap(), Some(7));
        assert!(a.number::<usize>("context-window").unwrap().is_none());
        assert!(Args::parse(&["--model".to_string()]).is_err());
    }

    /// A typo must not silently measure v2 under a v1 label: unlike the
    /// config file's lenient reader, the flag accepts exactly v1 or v2.
    #[test]
    fn the_prompt_version_flag_selects_v1_or_v2_and_rejects_anything_else() {
        assert_eq!(argv(&[]).prompt_version().unwrap(), PromptVersion::V2);
        assert_eq!(
            argv(&["--prompt-version", "v1"]).prompt_version().unwrap(),
            PromptVersion::V1
        );
        assert_eq!(
            argv(&["--prompt-version", "v2"]).prompt_version().unwrap(),
            PromptVersion::V2
        );
        let err = argv(&["--prompt-version", "1"])
            .prompt_version()
            .unwrap_err();
        assert!(err.contains("v1 or v2"), "{err}");
    }
}
