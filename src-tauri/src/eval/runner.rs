//! `run`: one fresh Builder session per task, against a real provider, then
//! the oracle.
//!
//! The session goes through the same entry the app uses — `run_workflow`, a
//! fresh session file, handoffs followed into linked sessions — with three
//! things the app does not have: a provider entry built from the command line,
//! a channel whose "frontend" is a collector, and `policy` standing in for the
//! person who answers prompts. Hooks, the browser, MCP servers and plugins are
//! off, so the run measures the harness and not whatever is installed on the
//! machine. Jev is on only when a key is given for it.

use super::materialize::{CommandOutcome, Tree, exclude_from_git, materialize, run_oracle};
use super::metrics::{Prices, RunMetrics, collect};
use super::policy;
use super::task::Task;
use crate::agent::events::AgentEvent;
use crate::agent::persist::{SessionRecord, SessionStore, now_ms};
use crate::agent::provider::{AgentConfig, PluginPrefs, ProviderEntry};
use crate::agent::session::{
    AnswerMap, ApprovalMap, ModeCtl, ModeOrigin, RunOutcome, SessionMode, SteeringCtl, UserAnswer,
};
use crate::agent::tools::{ReadTracker, ShellGate, ToolContext};
use serde::Serialize;
use std::collections::HashMap;
use std::path::{Path, PathBuf};
use std::sync::atomic::Ordering;
use std::sync::{Arc, Mutex};
use std::time::{Duration, Instant};
use tauri::ipc::{Channel, InvokeResponseBody};

/// A run that hands off more often than this is not converging.
const MAX_HANDOFFS: usize = 5;

/// How long a timed-out session gets to wind down after the interrupt before
/// it is abandoned. A provider request stops at once; a tool already running
/// (a test suite under `run_quality`) may take its own time to notice.
const STOP_GRACE: Duration = Duration::from_secs(60);

/// The id the eval's provider entry is registered under; models are
/// addressed as `eval/<model>`.
const PROVIDER_ID: &str = "eval";

#[derive(Debug, Clone)]
pub(crate) struct RunOptions {
    pub model: String,
    pub base_url: String,
    pub api_key: String,
    /// `anthropic` or `openai`.
    pub protocol: String,
    pub max_rounds: Option<usize>,
    pub context_window: Option<u32>,
    pub prices: Prices,
    pub jev_api_key: Option<String>,
    /// Each task is materialized in `<work_root>/<task id>/ws`.
    pub work_root: PathBuf,
    pub task_timeout: Duration,
}

#[derive(Debug, Clone, Serialize)]
pub(crate) struct TaskResult {
    pub id: String,
    pub title: String,
    pub tags: Vec<String>,
    pub pass: bool,
    /// `completed`, `timeout`, or `error: …` (`timeout (… abandoned)` when
    /// the session did not stop within `STOP_GRACE` of the interrupt).
    pub outcome: String,
    pub wall_s: f64,
    pub oracle_exit: Option<i32>,
    pub oracle_tail: String,
    #[serde(flatten)]
    pub metrics: RunMetrics,
}

/// The agent config for one task: everything the app reads from the user's
/// settings, set to what an unattended, reproducible run needs.
pub(crate) fn eval_config(opts: &RunOptions, workspace: &Path) -> AgentConfig {
    let mut config = AgentConfig {
        // No claudin.io account: web search and the plan's Jev stay off.
        api_key: String::new(),
        builder_model: format!("{PROVIDER_ID}/{}", opts.model),
        brain_model: format!("{PROVIDER_ID}/{}", opts.model),
        max_rounds: opts.max_rounds,
        yolo_mode: false,
        hooks: None,
        hooks_enabled: false,
        auto_commit_plan: false,
        keep_awake: false,
        ..AgentConfig::default()
    };
    config.browser.enabled = false;
    config.mcp.clear();
    config.jev.api_key = opts.jev_api_key.clone();
    config.jev.enabled = opts.jev_api_key.is_some();
    // Plugins are found on disk (the workspace and the home directory); every
    // one found is switched off, so none contributes skills or MCP servers.
    for record in crate::agent::plugins::discover(Some(workspace)) {
        if let crate::agent::plugins::PluginRecord::Loaded(plugin) = record {
            config
                .plugins
                .insert(plugin.manifest.name.clone(), PluginPrefs { enabled: false });
        }
    }
    config.providers.insert(
        PROVIDER_ID.into(),
        ProviderEntry {
            api_key: opts.api_key.clone(),
            base_url: opts.base_url.clone(),
            protocol: opts.protocol.clone(),
            enabled_models: vec![],
            label: Some("eval".into()),
            // Prices are applied when the metrics are read (`Prices`, which
            // prices cache reads and writes). Here they would make the
            // OpenAI-protocol client bill cached tokens at the full input
            // price and record that as the provider's cost.
            model_pricing: HashMap::new(),
            model_output_limits: HashMap::new(),
            custom: true,
            custom_models: vec![opts.model.clone()],
            model_context_limits: HashMap::new(),
            context_window: opts.context_window,
        },
    );
    config
}

/// Who answers the run's prompts: decisions are taken as the events arrive
/// and handed over once the matching oneshot is registered.
#[derive(Default)]
struct Responder {
    approvals: Mutex<HashMap<String, bool>>,
    answers: Mutex<HashMap<String, Vec<UserAnswer>>>,
    scripted: Mutex<Vec<String>>,
}

impl Responder {
    fn on_event(&self, event: AgentEvent) {
        match event {
            AgentEvent::ToolCall {
                session_id,
                tool_id,
                tool_name,
                args,
                permission,
                ..
            } if permission == "requires_approval" => {
                let approve = policy::approves(&tool_name, &args);
                if let Ok(mut map) = self.approvals.lock() {
                    map.insert(format!("{session_id}:{tool_id}"), approve);
                }
            }
            AgentEvent::AskUser {
                session_id,
                tool_id,
                questions,
            } => {
                let reply = match self.scripted.lock() {
                    Ok(mut scripted) => policy::answer_questions(&questions, &mut scripted),
                    Err(_) => Vec::new(),
                };
                if let Ok(mut map) = self.answers.lock() {
                    map.insert(format!("{session_id}:{tool_id}"), reply);
                }
            }
            AgentEvent::Subagent { event, .. } => self.on_event(*event),
            _ => {}
        }
    }

    /// Hand every decided prompt to its waiting oneshot. The app registers a
    /// prompt's oneshot before it emits the event for approvals, and after it
    /// for `ask_user`, so either side may arrive first; whatever is not
    /// matched yet stays for the next pass.
    async fn deliver(&self, approvals: &ApprovalMap, answers: &AnswerMap) {
        {
            let mut waiting = approvals.lock().await;
            let keys: Vec<String> = waiting.keys().cloned().collect();
            for key in keys {
                let decided = self.approvals.lock().ok().and_then(|mut m| m.remove(&key));
                if let Some(approve) = decided
                    && let Some(tx) = waiting.remove(&key)
                {
                    let _ = tx.send(approve);
                }
            }
        }
        let mut waiting = answers.lock().await;
        let keys: Vec<String> = waiting.keys().cloned().collect();
        for key in keys {
            let decided = self.answers.lock().ok().and_then(|mut m| m.remove(&key));
            if let Some(reply) = decided
                && let Some(tx) = waiting.remove(&key)
            {
                let _ = tx.send(reply);
            }
        }
    }
}

/// Materialize `task`, run one session on it, run its oracle, and read the
/// session files back into metrics.
pub(crate) async fn run_task(task: &Task, opts: &RunOptions) -> TaskResult {
    let started = Instant::now();
    let root = opts.work_root.join(&task.id);
    let ws = root.join("ws");
    let mut result = TaskResult {
        id: task.id.clone(),
        title: task.title.clone(),
        tags: task.tags.clone(),
        pass: false,
        outcome: String::new(),
        wall_s: 0.0,
        oracle_exit: None,
        oracle_tail: String::new(),
        metrics: RunMetrics::default(),
    };
    if let Err(e) = prepare_workspace(task, &ws) {
        result.outcome = format!("error: {e}");
        return result;
    }

    let mut opts = opts.clone();
    if task.max_rounds.is_some() {
        opts.max_rounds = task.max_rounds;
    }
    let steering = Arc::new(SteeringCtl::new());
    let index_db = root.join("index.db");
    let session = run_session(task, &opts, &ws, &index_db, steering.clone());
    tokio::pin!(session);
    result.outcome = match tokio::time::timeout(opts.task_timeout, &mut session).await {
        Ok(Ok(())) => "completed".into(),
        Ok(Err(e)) => format!("error: {e}"),
        // Dropping the session here would leave its subagents running — they
        // are spawned tasks, and a dropped JoinHandle does not stop one — and
        // they would go on editing the workspace while the oracle graded it.
        // So time running out is the Stop button: raise the interrupt, which
        // every provider request, subagent round and tool batch watches, and
        // let the session wind down (its approvals still answered) before the
        // oracle runs.
        Err(_) => {
            steering.interrupt.store(true, Ordering::SeqCst);
            match tokio::time::timeout(STOP_GRACE, &mut session).await {
                Ok(_) => "timeout".into(),
                Err(_) => format!(
                    "timeout (still running {}s after the stop; abandoned)",
                    STOP_GRACE.as_secs()
                ),
            }
        }
    };

    let oracle: CommandOutcome = run_oracle(task, &ws);
    result.pass = oracle.passed;
    result.oracle_exit = oracle.exit_code;
    result.oracle_tail = oracle.tail;
    result.metrics = collect(&ws.join(".claudinio").join("sessions"), &opts.prices);
    result.wall_s = started.elapsed().as_secs_f64();
    result
}

/// The start tree, plus the quality config that makes `run_quality` run the
/// task's own check command (kept out of git so the diff stays the agent's).
fn prepare_workspace(task: &Task, ws: &Path) -> Result<(), String> {
    if ws.exists() {
        std::fs::remove_dir_all(ws).map_err(|e| format!("clear {}: {e}", ws.display()))?;
    }
    materialize(task, ws, Tree::Start)?;
    let path = ws.join(".claudinio.json");
    let mut cfg: serde_json::Value = std::fs::read_to_string(&path)
        .ok()
        .and_then(|t| serde_json::from_str(&t).ok())
        .unwrap_or_else(|| serde_json::json!({}));
    cfg["quality"] = serde_json::json!({
        "test_cmd": task.check_command(),
        "enforce_on": "code_change",
        "enforced_layers": ["tests"],
    });
    std::fs::write(
        &path,
        serde_json::to_string_pretty(&cfg).unwrap_or_default(),
    )
    .map_err(|e| format!("write quality config: {e}"))?;
    exclude_from_git(ws, ".claudinio.json")?;
    exclude_from_git(ws, ".claudinio/")
}

/// A lexical index of the workspace, so the code-intel tools answer as they
/// do in the app. No embedding model is loaded: semantic search falls back to
/// lexical matching, as it does in the app while embeddings are still running.
fn build_index(ws: &Path, db_path: &Path) -> Option<String> {
    let db = crate::code_intel::db::IndexDb::open(db_path).ok()?;
    crate::code_intel::indexer::scan_workspace(&db, &ws.to_string_lossy(), None, None, None)
        .ok()?;
    Some(db_path.to_string_lossy().to_string())
}

async fn run_session(
    task: &Task,
    opts: &RunOptions,
    ws: &Path,
    index_db: &Path,
    steering: Arc<SteeringCtl>,
) -> Result<(), String> {
    let workspace = ws.to_string_lossy().to_string();
    let config = eval_config(opts, ws);
    let session_id = uuid::Uuid::new_v4().to_string();
    let mut store = SessionStore::create(&session_id, Some(&workspace))?;
    let mut mode_ctl = Arc::new(ModeCtl::new(SessionMode::Builder, ModeOrigin::Human));
    store.try_append(&SessionRecord::Mode {
        mode: SessionMode::Builder.as_str().into(),
        origin: ModeOrigin::Human.as_str().into(),
        ts: now_ms(),
    });
    let db_path = {
        let (ws, db) = (ws.to_path_buf(), index_db.to_path_buf());
        tokio::task::spawn_blocking(move || build_index(&ws, &db))
            .await
            .ok()
            .flatten()
    };
    let mut ctx = ToolContext {
        hooks: None,
        db_path,
        lsp_manager: None,
        workspace_root: Some(workspace.clone()),
        embedding_model: Default::default(),
        session_store_path: Some(store.path.to_string_lossy().to_string()),
        limits: Default::default(),
        read_tracker: Arc::new(tokio::sync::Mutex::new(ReadTracker::default())),
        browser: None,
        interrupt: Some(steering.interrupt.clone()),
        agent_config: Some(config.clone()),
        plan_save_path: None,
        base_commit: crate::agent::tools::git_head(&workspace),
        auto_approve_git: false,
        shell_gate: Some(ShellGate {
            rejection: policy::SHELL_REJECTION.into(),
        }),
        mcp: None,
        mode_ctl: Some(mode_ctl.clone()),
        index_progress: None,
        records_cache: Arc::new(std::sync::Mutex::new(lru::LruCache::new(
            std::num::NonZeroUsize::new(8).expect("non-zero"),
        ))),
    };

    let responder = Arc::new(Responder::default());
    if let Ok(mut scripted) = responder.scripted.lock() {
        *scripted = task.answers.clone();
    }
    let events: Channel<AgentEvent> = {
        let responder = responder.clone();
        Channel::new(move |body: InvokeResponseBody| -> tauri::Result<()> {
            if let InvokeResponseBody::Json(json) = &body
                && let Ok(event) = serde_json::from_str::<AgentEvent>(json)
            {
                responder.on_event(event);
            }
            Ok(())
        })
    };
    let approvals = ApprovalMap::default();
    let answers = AnswerMap::default();
    // The delivery loop lives exactly as long as this session: aborted when
    // the session returns or, if it is ever abandoned, when it is dropped.
    let _delivery = AbortOnDrop({
        let (responder, approvals, answers) =
            (responder.clone(), approvals.clone(), answers.clone());
        tokio::spawn(async move {
            loop {
                responder.deliver(&approvals, &answers).await;
                tokio::time::sleep(Duration::from_millis(10)).await;
            }
        })
    });

    let mut history = Vec::new();
    let mut message = task.prompt.clone();
    let mut handoffs = 0;
    let result = loop {
        let outcome = crate::agent::session::run_workflow(
            &config,
            &mut history,
            std::mem::take(&mut message),
            Vec::new(),
            &events,
            &approvals,
            &answers,
            &session_id_of(&store),
            &ctx,
            &store,
            &steering,
            &mode_ctl,
        )
        .await;
        match outcome {
            Ok(RunOutcome::Completed) => break Ok(()),
            Ok(RunOutcome::Handoff(_)) if handoffs >= MAX_HANDOFFS => {
                break Err(format!("stopped after {MAX_HANDOFFS} handoffs"));
            }
            Ok(RunOutcome::Handoff(spec)) => {
                handoffs += 1;
                let mut spec = *spec;
                spec.first_message =
                    crate::agent::transition::resolve_first_message(&spec, Some(&workspace), None);
                let next = link_session(&store, &workspace, &spec)?;
                mode_ctl = Arc::new(ModeCtl::new(spec.next_mode, spec.next_origin));
                ctx = crate::agent::transition::rebuild_tool_context(
                    &ctx,
                    &session_id_of(&next),
                    &next.path,
                    mode_ctl.clone(),
                    config.clone(),
                );
                store = next;
                history = Vec::new();
                message = spec.first_message;
            }
            Err(e) => {
                store.try_append(&SessionRecord::Error {
                    message: e.clone(),
                    ts: now_ms(),
                });
                break Err(e);
            }
        }
    };
    result
}

/// Aborts a spawned task when dropped.
struct AbortOnDrop(tokio::task::JoinHandle<()>);

impl Drop for AbortOnDrop {
    fn drop(&mut self) {
        self.0.abort();
    }
}

fn session_id_of(store: &SessionStore) -> String {
    store
        .path
        .file_stem()
        .and_then(|s| s.to_str())
        .unwrap_or_default()
        .to_string()
}

/// The app's `transition::link_session` without the workspace state it keeps
/// for the UI: forward and back pointers, the base commit and the task list
/// carried over, and the successor's mode.
fn link_session(
    old: &SessionStore,
    workspace: &str,
    spec: &crate::agent::session::HandoffSpec,
) -> Result<SessionStore, String> {
    let new_id = uuid::Uuid::new_v4().to_string();
    let old_records = crate::agent::persist::load_records(&old.path).unwrap_or_default();
    old.try_append(&SessionRecord::HandoffTo {
        next_session_id: new_id.clone(),
        reason: spec.reason.as_str().into(),
        ts: now_ms(),
    });
    let new = SessionStore::create(&new_id, Some(workspace))?;
    new.try_append(&SessionRecord::LinkedFrom {
        prev_session_id: session_id_of(old),
        reason: spec.reason.as_str().into(),
        golden_cycle: spec.golden_cycle,
        golden_stalls: spec.golden_stalls,
        golden_last_pending: spec.golden_last_pending.clone(),
        ts: now_ms(),
    });
    if let Some((sha, ts)) = crate::agent::persist::earliest_base_commit_at(&old_records) {
        new.try_append(&SessionRecord::BaseCommit { sha, ts });
    }
    if let Ok(tasks) = crate::agent::persist::load_last_tasks(&old.path)
        && !tasks.is_empty()
    {
        let _ = crate::agent::persist::append_tasks(&new.path, &tasks);
    }
    new.try_append(&SessionRecord::Mode {
        mode: spec.next_mode.as_str().into(),
        origin: spec.next_origin.as_str().into(),
        ts: now_ms(),
    });
    Ok(new)
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::agent::subagent::scripted_model::{Reply, spawn_with_usage};

    const TASK: &str = r#"
id: greet
title: Say goodbye
tags: [bugfix]
prompt: Make greet.txt say goodbye instead of hello.
setup:
  files:
    greet.txt: "hello world\n"
oracle:
  command: grep -q '^goodbye world$' greet.txt
  timeout_s: 20
reference:
  files:
    greet.txt: "goodbye world\n"
"#;

    fn options(base_url: &str, work_root: &Path) -> RunOptions {
        RunOptions {
            model: "m".into(),
            base_url: base_url.into(),
            api_key: "k".into(),
            protocol: "anthropic".into(),
            max_rounds: Some(20),
            // 32k runs the Compact profile, where the session edits itself.
            context_window: Some(32_768),
            prices: Prices::default(),
            jev_api_key: None,
            work_root: work_root.to_path_buf(),
            task_timeout: Duration::from_secs(120),
        }
    }

    /// The whole runner against a scripted model: it reads the file, edits
    /// it (approved by the policy, not by `yolo_mode`), says it is done — and
    /// the oracle, run afterwards in the same directory, passes.
    #[tokio::test]
    async fn a_scripted_model_solves_a_fixture_task_end_to_end() {
        let work = tempfile::tempdir().unwrap();
        let task = Task::parse(TASK, "greet").unwrap();
        let file = work.path().join("greet").join("ws").join("greet.txt");
        let path = file.to_string_lossy().to_string();
        let (base, _, bodies) = spawn_with_usage(
            vec![
                Reply::Tool {
                    name: "read_file",
                    input: serde_json::json!({ "path": path }),
                },
                Reply::Tool {
                    name: "edit_file",
                    input: serde_json::json!({
                        "path": path, "old_string": "hello", "new_string": "goodbye",
                    }),
                },
                Reply::Text("Changed the greeting to goodbye."),
            ],
            Some(serde_json::json!({
                "input_tokens": 100,
                "cache_read_input_tokens": 1_000,
                "cache_creation_input_tokens": 50,
            })),
        )
        .await;
        let result = run_task(&task, &options(&base, work.path())).await;
        assert_eq!(result.outcome, "completed");
        assert!(result.pass, "oracle: {}", result.oracle_tail);
        let requests = bodies.lock().unwrap().len() as u32;
        assert_eq!(requests, 4, "three rounds and the completion judge");
        assert_eq!(
            result.metrics.rounds, requests,
            "one usage record per request"
        );
        assert_eq!(result.metrics.main_rounds, requests);
        assert_eq!(result.metrics.subagents, 0);
        assert_eq!(result.metrics.input_tokens, 100 * u64::from(requests));
        assert_eq!(
            result.metrics.cache_read_tokens,
            1_000 * u64::from(requests)
        );
        assert_eq!(
            result.metrics.cache_creation_tokens,
            50 * u64::from(requests)
        );
        assert_eq!(result.metrics.tool_calls.get("read_file"), Some(&1));
        assert_eq!(result.metrics.tool_calls.get("edit_file"), Some(&1));
        assert!(
            !result.metrics.cost_known,
            "the stub reports no cost and no price was given"
        );
        // The runner's own files stay out of the agent's diff.
        let status = std::process::Command::new("git")
            .args(["status", "--porcelain"])
            .current_dir(file.parent().unwrap())
            .output()
            .unwrap();
        assert_eq!(
            String::from_utf8_lossy(&status.stdout).trim(),
            "M greet.txt"
        );
    }

    /// A task that runs out of time is stopped the way the app's Stop button
    /// stops a run, and wound down before its oracle runs: a subagent still
    /// waiting on the provider when the clock runs out must not edit the
    /// workspace after the oracle graded it, and nothing may keep talking to
    /// the provider once `run_task` has returned.
    #[tokio::test]
    async fn a_timed_out_task_is_stopped_before_its_oracle_runs() {
        let work = tempfile::tempdir().unwrap();
        let task = Task::parse(TASK, "greet").unwrap();
        let file = work.path().join("greet").join("ws").join("greet.txt");
        let path = file.to_string_lossy().to_string();
        let (base, _, bodies) = spawn_with_usage(
            vec![
                Reply::Tool {
                    name: "spawn_agents",
                    input: serde_json::json!({"agents": [{
                        "name": "fixer", "mode": "code",
                        "goal": format!("Change hello to goodbye in {path}"),
                    }]}),
                },
                Reply::Tool {
                    name: "read_file",
                    input: serde_json::json!({ "path": path }),
                },
                Reply::Delayed(
                    3_000,
                    Box::new(Reply::Tool {
                        name: "edit_file",
                        input: serde_json::json!({
                            "path": path, "old_string": "hello", "new_string": "goodbye",
                        }),
                    }),
                ),
                Reply::Text("done"),
            ],
            None,
        )
        .await;
        let mut opts = options(&base, work.path());
        // A window large enough for the full profile, which offers subagents.
        opts.context_window = Some(200_000);
        opts.task_timeout = Duration::from_millis(1_000);
        let result = run_task(&task, &opts).await;
        assert_eq!(result.outcome, "timeout");
        assert!(!result.pass, "the edit had not happened when time ran out");
        let requests = bodies.lock().unwrap().len();
        tokio::time::sleep(Duration::from_millis(4_000)).await;
        assert_eq!(
            std::fs::read_to_string(&file).unwrap(),
            "hello world\n",
            "nothing edits the workspace after the oracle ran"
        );
        assert_eq!(
            bodies.lock().unwrap().len(),
            requests,
            "no request after run_task returned"
        );
    }

    /// A model that tries to run the tests through the shell is refused and
    /// told to use run_quality instead; a model that only looks is not.
    #[tokio::test]
    async fn shell_commands_go_through_the_policy() {
        let work = tempfile::tempdir().unwrap();
        let task = Task::parse(TASK, "greet").unwrap();
        let (base, _, bodies) = spawn_with_usage(
            vec![
                Reply::Tool {
                    name: "bash",
                    input: serde_json::json!({ "command": "echo goodbye world > greet.txt" }),
                },
                Reply::Tool {
                    name: "bash",
                    input: serde_json::json!({ "command": "cat greet.txt" }),
                },
                Reply::Text("I could not change it."),
            ],
            None,
        )
        .await;
        let result = run_task(&task, &options(&base, work.path())).await;
        assert!(!result.pass, "the write was refused, so the oracle fails");
        let bodies = bodies.lock().unwrap().clone();
        assert!(
            bodies[1].contains("run_quality"),
            "the refusal names the way out"
        );
        assert!(
            bodies[2].contains("hello world"),
            "the read-only command ran"
        );
    }

    /// With `--price-in/--price-out` the cost of a request that reports none
    /// is estimated by `metrics::Prices`, which prices cache reads and writes.
    /// Handing the prices to the provider entry instead would make the
    /// OpenAI-protocol client bill `prompt_tokens` — cached tokens included —
    /// at the full input price and record that as the reported cost.
    #[test]
    fn prices_are_applied_by_the_metrics_not_by_the_provider() {
        let work = tempfile::tempdir().unwrap();
        let mut opts = options("http://127.0.0.1:1", work.path());
        opts.protocol = "openai".into();
        opts.prices = Prices {
            input: Some(3.0),
            output: Some(15.0),
            ..Prices::default()
        };
        let config = eval_config(&opts, work.path());
        assert_eq!(config.resolve_provider("eval/m").pricing, None);
    }

    #[test]
    fn the_eval_config_turns_off_everything_installed_on_the_machine() {
        let work = tempfile::tempdir().unwrap();
        let config = eval_config(&options("http://127.0.0.1:1", work.path()), work.path());
        assert_eq!(config.builder_model, "eval/m");
        assert!(!config.hooks_enabled && !config.browser.enabled && !config.yolo_mode);
        assert!(config.mcp.is_empty());
        assert!(!config.is_claudinio_account());
        assert!(crate::agent::jev::backend(&config).is_none());
        assert!(config.plugins.values().all(|p| !p.enabled));
        assert_eq!(config.known_context_window("eval/m"), Some(32_768));
    }
}
