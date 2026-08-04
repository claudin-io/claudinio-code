//! The `run_quality` tool, and the evidence rules the golden gate enforces.
//!
//! Two callers share everything here: the tool (the agent asking for a check)
//! and the harness (the loop refusing to let a goal close without one). They
//! must agree exactly on what counts as evidence, so the rule lives in one
//! place — [`current_evidence`] — rather than being written twice.

use serde::Deserialize;
use std::path::Path;

use crate::agent::tools::ToolContext;
use crate::quality::config::EnforceOn;
use crate::quality::{self, Layer, QualityConfig, QualityReport};

/// Longest failure detail pushed back into the conversation when the gate
/// blocks a finish. Enough to act on, small enough not to evict the context.
const MAX_GATE_DETAIL_CHARS: usize = 4_000;

#[derive(Deserialize, Default)]
pub struct RunQualityArgs {
    /// Layer names to run. Omitted = the layers this project enforces.
    #[serde(default)]
    pub layers: Option<Vec<String>>,
}

/// What the session currently knows about the state of the code.
#[derive(Debug)]
pub enum Evidence {
    /// The workspace does not enforce any layer, so nothing is required.
    NotRequired,
    /// No quality run in this session yet.
    Missing,
    /// A run exists but the worktree changed since — it describes different
    /// code, so it proves nothing about the code now on disk.
    Stale,
    /// A run matching the current worktree. `pass` is its verdict.
    Current { pass: bool, summary: String },
}

/// Resolve the workspace root and its quality config, or `None` when the
/// harness cannot apply (no workspace attached — tests, aux workflows).
fn workspace_and_config(ctx: &ToolContext) -> Option<(std::path::PathBuf, QualityConfig)> {
    let root = std::path::PathBuf::from(ctx.workspace_root.as_deref()?);
    let cfg = QualityConfig::load(&root);
    Some((root, cfg))
}

/// The gate's single source of truth about whether the code has been verified.
pub fn current_evidence(ctx: &ToolContext) -> Evidence {
    let Some((root, cfg)) = workspace_and_config(ctx) else {
        return Evidence::NotRequired;
    };
    if !cfg.gate_active() {
        return Evidence::NotRequired;
    }
    let Some(store) = ctx.session_store_path.as_deref() else {
        return Evidence::NotRequired;
    };
    let Some(run) = crate::agent::persist::load_last_quality_run(Path::new(store)) else {
        return Evidence::Missing;
    };
    if run.digest != quality::evidence::workspace_digest(&root) {
        return Evidence::Stale;
    }
    Evidence::Current {
        pass: run.pass,
        summary: run.summary,
    }
}

/// Whether this run has to be verified before it is allowed to finish.
///
/// A tagged `<goal>` always demands proof. But keying *only* off goals made
/// the harness invisible to anyone who did not know the tag exists — the
/// common case — so `enforce_on: "code_change"` widens it to any session that
/// touched source. The check stays at the finish line, once per run, rather
/// than per task: a session with ten tasks must not mean ten test runs.
pub fn verification_required(ctx: &ToolContext) -> bool {
    let Some((root, cfg)) = workspace_and_config(ctx) else {
        return false;
    };
    if !cfg.gate_active() {
        return false;
    }
    let has_goal = ctx
        .session_store_path
        .as_deref()
        .and_then(|p| crate::agent::persist::load_last_tasks(Path::new(p)).ok())
        .unwrap_or_default()
        .iter()
        .any(crate::agent::tools::tasks::is_golden_execution);
    if has_goal {
        return true;
    }
    match cfg.enforce_on {
        EnforceOn::Goals => false,
        EnforceOn::CodeChange => quality::diff::touches_source(&root, ctx.base_commit.as_deref()),
    }
}

/// The most recent report, parsed — used to quote concrete failures back at
/// the model instead of a bare "quality failed".
pub fn last_report(ctx: &ToolContext) -> Option<QualityReport> {
    let store = ctx.session_store_path.as_deref()?;
    let run = crate::agent::persist::load_last_quality_run(Path::new(store))?;
    serde_json::from_str(&run.report).ok()
}

/// Run the requested layers and persist the result as session evidence.
///
/// `trigger` records who asked: "tool" (the agent) or "harness" (the loop at
/// the finish line). Both produce identical evidence — the harness does not
/// get a weaker check than the agent, which is the point.
pub async fn run_and_record(
    ctx: &ToolContext,
    layers: &[Layer],
    trigger: &str,
) -> Result<QualityReport, String> {
    let (root, cfg) = workspace_and_config(ctx)
        .ok_or("run_quality needs an open workspace: none is attached to this session")?;
    if !cfg.enabled {
        return Err(
            "the quality harness is disabled for this workspace (quality.enabled = false in \
             .claudinio.json)"
                .into(),
        );
    }

    let report = quality::run_layers(
        &root,
        &cfg,
        layers,
        ctx.base_commit.as_deref(),
        ctx.interrupt.as_ref(),
    )
    .await?;

    if let Some(store) = ctx.session_store_path.as_deref() {
        let info = crate::agent::persist::QualityRunInfo {
            digest: report.digest.clone(),
            pass: report.verdict.pass,
            summary: report.summary_text(),
            report: serde_json::to_string(&report).unwrap_or_default(),
            trigger: trigger.to_string(),
            ts: report.ts,
        };
        crate::agent::persist::append_quality_run(Path::new(store), &info)?;
        crate::agent::persist::invalidate_cache(Path::new(store), &ctx.records_cache);
    }
    Ok(report)
}

/// Run exactly the layers this workspace enforces.
///
/// The harness's own call at the finish line. It runs the same checks the
/// agent's `run_quality` would, so the loop never applies a weaker standard to
/// itself than it demands of the model.
pub async fn run_enforced(ctx: &ToolContext, trigger: &str) -> Result<QualityReport, String> {
    let (_, cfg) = workspace_and_config(ctx)
        .ok_or("the quality harness needs an open workspace: none is attached")?;
    run_and_record(ctx, &cfg.finish_line_layers(), trigger).await
}

/// Tool entry point.
pub async fn execute(args: RunQualityArgs, ctx: &ToolContext) -> Result<String, String> {
    let (_, cfg) = workspace_and_config(ctx)
        .ok_or("run_quality needs an open workspace: none is attached to this session")?;

    let layers = match &args.layers {
        Some(names) if !names.is_empty() => {
            let mut parsed = Vec::new();
            for name in names {
                let layer = Layer::parse(name).ok_or_else(|| {
                    format!("unknown quality layer '{name}'; available: tests, coverage")
                })?;
                if !parsed.contains(&layer) {
                    parsed.push(layer);
                }
            }
            parsed
        }
        _ => cfg.tool_default_layers(),
    };

    let report = run_and_record(ctx, &layers, "tool").await?;
    let mut out = report.summary_text();
    if !report.verdict.pass {
        out.push('\n');
        out.push_str(&report.failure_detail(MAX_GATE_DETAIL_CHARS));
        out.push_str(
            "\nFix the failures and call run_quality again. A golden task cannot be marked \
             done while this is red.",
        );
    } else if cfg.gate_active() {
        out.push_str(
            "\nThis evidence is valid only for the current state of the files — editing \
             anything invalidates it and the gate will ask for a fresh run.",
        );
    }
    Ok(out)
}

/// The message the gate hands back when it refuses a golden completion.
/// Written to be directly actionable: it always says what to call next.
pub fn rejection_message(ctx: &ToolContext, evidence: &Evidence, task_ids: &[String]) -> String {
    let goals = task_ids.join(", ");
    match evidence {
        Evidence::Missing => format!(
            "tasks_set rejected: golden task(s) {goals} cannot be marked done without verified \
             quality evidence. Call run_quality first — the harness checks the result \
             mechanically, so claiming the tests pass is not enough."
        ),
        Evidence::Stale => format!(
            "tasks_set rejected: golden task(s) {goals} cannot be marked done — the last \
             run_quality result is stale because files changed after it ran. Call run_quality \
             again so the evidence matches the code as it is now."
        ),
        Evidence::Current { summary, .. } => {
            let detail = last_report(ctx)
                .map(|r| r.failure_detail(MAX_GATE_DETAIL_CHARS))
                .unwrap_or_default();
            format!(
                "tasks_set rejected: golden task(s) {goals} cannot be marked done — the last \
                 quality run FAILED.\n\n{summary}\n{detail}\nFix the failures, then call \
                 run_quality again before marking the goal done."
            )
        }
        Evidence::NotRequired => String::new(),
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::agent::persist::QualityRunInfo;

    /// A workspace with a session file, a `.claudinio.json`, and one committed
    /// file so the digest has something real to fingerprint.
    fn workspace(name: &str, quality_json: &str) -> (ToolContext, std::path::PathBuf) {
        let root = std::env::temp_dir().join(format!("cq-gate-{name}-{}", std::process::id()));
        std::fs::remove_dir_all(&root).ok();
        std::fs::create_dir_all(&root).unwrap();
        std::fs::write(root.join(".claudinio.json"), quality_json).unwrap();
        std::fs::write(root.join("a.txt"), "one").unwrap();
        // Where sessions actually live, so the digest's bookkeeping exclusion
        // is exercised rather than bypassed.
        std::fs::create_dir_all(root.join(".claudinio/sessions")).unwrap();
        let store = root.join(".claudinio/sessions/s.jsonl");
        std::fs::write(&store, "").unwrap();
        let mut ctx = crate::agent::tools::tests_support::ctx();
        ctx.workspace_root = Some(root.to_string_lossy().to_string());
        ctx.session_store_path = Some(store.to_string_lossy().to_string());
        (ctx, root)
    }

    fn record(ctx: &ToolContext, root: &Path, pass: bool) {
        let store = ctx.session_store_path.clone().unwrap();
        let info = QualityRunInfo {
            digest: quality::evidence::workspace_digest(root),
            pass,
            summary: "1 passed, 0 failed".into(),
            report: String::new(),
            trigger: "tool".into(),
            ts: 1,
        };
        crate::agent::persist::append_quality_run(Path::new(&store), &info).unwrap();
    }

    #[test]
    fn no_run_yet_is_missing_evidence() {
        let (ctx, root) = workspace("missing", "{}");
        assert!(matches!(current_evidence(&ctx), Evidence::Missing));
        std::fs::remove_dir_all(&root).ok();
    }

    #[test]
    fn a_matching_green_run_is_current_evidence() {
        let (ctx, root) = workspace("fresh", "{}");
        record(&ctx, &root, true);
        assert!(matches!(
            current_evidence(&ctx),
            Evidence::Current { pass: true, .. }
        ));
        std::fs::remove_dir_all(&root).ok();
    }

    #[test]
    fn editing_a_file_after_the_run_makes_the_evidence_stale() {
        // The central attack this closes: run the tests, then change the code,
        // then claim the goal is done on the strength of the old green run.
        let (ctx, root) = workspace("stale", "{}");
        record(&ctx, &root, true);
        std::fs::write(root.join("a.txt"), "two — changed after the run").unwrap();
        assert!(matches!(current_evidence(&ctx), Evidence::Stale));
        std::fs::remove_dir_all(&root).ok();
    }

    #[test]
    fn a_red_run_is_current_but_failing() {
        let (ctx, root) = workspace("red", "{}");
        record(&ctx, &root, false);
        assert!(matches!(
            current_evidence(&ctx),
            Evidence::Current { pass: false, .. }
        ));
        std::fs::remove_dir_all(&root).ok();
    }

    #[test]
    fn a_workspace_that_enforces_nothing_never_requires_evidence() {
        let (ctx, root) = workspace("off", r#"{"quality":{"enforced_layers":[]}}"#);
        assert!(matches!(current_evidence(&ctx), Evidence::NotRequired));
        std::fs::remove_dir_all(&root).ok();
    }

    #[test]
    fn a_context_without_a_workspace_never_requires_evidence() {
        // Tests and one-off workflows must not be gated.
        let ctx = crate::agent::tools::tests_support::ctx();
        assert!(matches!(current_evidence(&ctx), Evidence::NotRequired));
    }

    /// A git repository with one commit, so `touches_source` has a real base
    /// to diff against.
    fn git_workspace(name: &str, quality_json: &str) -> (ToolContext, std::path::PathBuf, String) {
        let (mut ctx, root) = workspace(name, quality_json);
        let git = |args: &[&str]| {
            let mut c = std::process::Command::new("git");
            c.args(args).current_dir(&root);
            crate::procutil::no_window(&mut c);
            c.output().map(|o| o.status.success()).unwrap_or(false)
        };
        assert!(git(&["init", "-q"]));
        git(&["config", "user.email", "t@t"]);
        git(&["config", "user.name", "t"]);
        std::fs::write(root.join(".gitignore"), ".claudinio/\n").unwrap();
        git(&["add", "-A"]);
        assert!(git(&["commit", "-q", "-m", "base"]));
        let head = quality::evidence::git_head(&root).expect("HEAD");
        ctx.base_commit = Some(head.clone());
        (ctx, root, head)
    }

    fn goal_task(status: &str) -> crate::agent::persist::TaskItem {
        crate::agent::persist::TaskItem {
            id: "golden-g-1".into(),
            title: "goal".into(),
            description: String::new(),
            journal: vec![],
            status: status.into(),
        }
    }

    #[test]
    fn a_tagged_goal_always_demands_verification() {
        let (ctx, root, _) = git_workspace("req-goal", "{}");
        let store = ctx.session_store_path.clone().unwrap();
        crate::agent::persist::append_tasks(Path::new(&store), &[goal_task("doing")]).unwrap();
        assert!(verification_required(&ctx));
        std::fs::remove_dir_all(&root).ok();
    }

    #[test]
    fn by_default_changing_code_demands_verification_without_any_goal() {
        // The point of the default: someone who never learned the <goal> tag
        // is still protected.
        let (ctx, root, _) = git_workspace("req-default", "{}");
        std::fs::write(root.join("code.rs"), "fn a() {}\n").unwrap();
        assert!(verification_required(&ctx));
        std::fs::remove_dir_all(&root).ok();
    }

    #[test]
    fn goals_mode_narrows_it_back_to_tagged_goals() {
        // The escape hatch for a suite too slow to sit through on every change.
        let (ctx, root, _) = git_workspace("req-goals", r#"{"quality":{"enforce_on":"goals"}}"#);
        std::fs::write(root.join("code.rs"), "fn a() {}\n").unwrap();
        assert!(!verification_required(&ctx));
        std::fs::remove_dir_all(&root).ok();
    }

    #[test]
    fn a_docs_only_session_is_left_alone() {
        let (ctx, root, _) = git_workspace("req-docs", "{}");
        std::fs::write(root.join("NOTES.md"), "notes\n").unwrap();
        assert!(
            !verification_required(&ctx),
            "editing prose must not cost a test run"
        );
        std::fs::remove_dir_all(&root).ok();
    }

    #[test]
    fn a_read_only_session_is_left_alone() {
        let (ctx, root, _) = git_workspace("req-readonly", "{}");
        assert!(
            !verification_required(&ctx),
            "a conversation that changed nothing has nothing to verify"
        );
        std::fs::remove_dir_all(&root).ok();
    }

    #[test]
    fn a_workspace_enforcing_nothing_demands_nothing_in_either_mode() {
        let (ctx, root, _) = git_workspace("req-off", r#"{"quality":{"enforced_layers":[]}}"#);
        std::fs::write(root.join("code.rs"), "fn a() {}\n").unwrap();
        assert!(!verification_required(&ctx));
        std::fs::remove_dir_all(&root).ok();
    }

    #[test]
    fn rejection_messages_name_the_goal_and_the_next_call() {
        let (ctx, root) = workspace("msg", "{}");
        let goals = vec!["golden-x-1".to_string()];
        for evidence in [Evidence::Missing, Evidence::Stale] {
            let msg = rejection_message(&ctx, &evidence, &goals);
            assert!(msg.contains("golden-x-1"), "{msg}");
            assert!(msg.contains("run_quality"), "{msg}");
        }
        std::fs::remove_dir_all(&root).ok();
    }

    #[tokio::test]
    async fn an_unknown_layer_name_is_rejected_before_anything_runs() {
        let (ctx, root) = workspace("badlayer", "{}");
        let err = execute(
            RunQualityArgs {
                layers: Some(vec!["not-a-real-layer".into()]),
            },
            &ctx,
        )
        .await
        .unwrap_err();
        assert!(err.contains("unknown quality layer"), "{err}");
        std::fs::remove_dir_all(&root).ok();
    }

    #[tokio::test]
    async fn a_workspace_with_no_recognizable_project_explains_the_override() {
        let (ctx, root) = workspace("noproject", "{}");
        let err = execute(RunQualityArgs::default(), &ctx).await.unwrap_err();
        assert!(err.contains("test_cmd"), "{err}");
        std::fs::remove_dir_all(&root).ok();
    }

    #[tokio::test]
    async fn a_real_run_records_reusable_evidence() {
        let (ctx, root) = workspace("record", r#"{"quality":{"test_cmd":"exit 0"}}"#);
        let report = execute(RunQualityArgs::default(), &ctx).await.unwrap();
        assert!(report.contains("VERDICT: pass"), "{report}");
        assert!(matches!(
            current_evidence(&ctx),
            Evidence::Current { pass: true, .. }
        ));
        std::fs::remove_dir_all(&root).ok();
    }

    #[tokio::test]
    async fn a_failing_suite_records_failing_evidence_and_says_what_to_do() {
        let (ctx, root) = workspace("recordred", r#"{"quality":{"test_cmd":"exit 1"}}"#);
        let out = execute(RunQualityArgs::default(), &ctx).await.unwrap();
        assert!(out.contains("VERDICT: fail"), "{out}");
        assert!(out.contains("cannot be marked done"), "{out}");
        assert!(matches!(
            current_evidence(&ctx),
            Evidence::Current { pass: false, .. }
        ));
        std::fs::remove_dir_all(&root).ok();
    }
}
