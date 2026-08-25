//! The IPC surface for lifecycle hooks.
//!
//! Thin by design: everything that decides anything lives in
//! `crate::agent::hooks`, and this file only shapes it for the UI and carries
//! the user's approval back. The architecture test in `lib.rs` depends on that
//! direction holding.

use crate::agent::hooks::{
    self, HookEvent, HookSource, ResolvedHook, TrustStatus, discovery::HookDiagnostic,
};
use crate::agent::persist::now_ms;
use crate::state::AppState;
use serde::Serialize;
use std::path::PathBuf;
use tauri::State;

/// One hook, as the settings panel shows it.
#[derive(Debug, Clone, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct HookInfo {
    pub event: String,
    pub matcher: Option<String>,
    /// The command after `${CLAUDE_PLUGIN_ROOT}` and friends were expanded.
    /// Approving a command you cannot trace to a real path is not consent.
    pub command: String,
    pub args: Vec<String>,
    pub display: String,
    pub timeout_secs: u64,
    pub status_message: Option<String>,
    pub source: String,
    pub source_kind: String,
    /// Other places that declare the identical command. It runs once.
    pub also_from: Vec<String>,
    /// Which tools this matcher will actually select. A matcher that hits
    /// nothing is the failure mode of this whole feature, and the only way to
    /// see it is to be told.
    pub hits: Vec<String>,
    pub matcher_valid: bool,
}

#[derive(Debug, Clone, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct HooksInfo {
    pub enabled: bool,
    pub workspace: Option<String>,
    pub trust: TrustStatus,
    pub fingerprint: String,
    /// What was approved last time, so a changed set can say what changed.
    pub approved_commands: Vec<String>,
    pub hooks: Vec<HookInfo>,
    pub diagnostics: Vec<HookDiagnostic>,
}

#[derive(Debug, Clone, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct HookRunView {
    pub status: String,
    pub exit_code: Option<i32>,
    pub duration_ms: u64,
    pub stdout: String,
    pub stderr: String,
    pub decision: Option<String>,
    pub additional_context: Option<String>,
    pub error: Option<String>,
}

fn source_kind(s: &HookSource) -> &'static str {
    match s {
        HookSource::UserSettings { .. } => "userSettings",
        HookSource::AppConfig => "appConfig",
        HookSource::Plugin { .. } => "plugin",
        HookSource::WorkspaceConfig { .. } => "workspaceConfig",
        HookSource::ProjectSettings { .. } => "projectSettings",
        HookSource::LocalSettings { .. } => "localSettings",
    }
}

/// Every tool a matcher could name, so "what will this hit" is answerable.
fn tool_universe() -> Vec<String> {
    crate::agent::tools::get_defs(4)
        .into_iter()
        .map(|d| d.name)
        .collect()
}

fn to_info(h: &ResolvedHook, universe: &[String]) -> HookInfo {
    let hits = match h.event {
        HookEvent::PreToolUse | HookEvent::PostToolUse => {
            let refs: Vec<&str> = universe.iter().map(String::as_str).collect();
            hooks::matcher::hits(h.matcher.as_deref(), &refs)
        }
        HookEvent::PreCompact => {
            hooks::matcher::hits_literal(h.matcher.as_deref(), &["manual", "auto"])
        }
        HookEvent::SessionStart => hooks::matcher::hits_literal(
            h.matcher.as_deref(),
            &["startup", "resume", "clear", "compact"],
        ),
        _ => Vec::new(),
    };
    HookInfo {
        event: h.event.as_str().to_string(),
        matcher: h.matcher.clone(),
        command: h.command.clone(),
        args: h.args.clone(),
        display: h.display_command(),
        timeout_secs: h.timeout_secs,
        status_message: h.status_message.clone(),
        source: h.source.label(),
        source_kind: source_kind(&h.source).to_string(),
        also_from: h.also_from.iter().map(HookSource::label).collect(),
        hits,
        matcher_valid: h
            .matcher
            .as_deref()
            .map(hooks::matcher::matcher_is_valid_regex)
            .unwrap_or(true),
    }
}

async fn info_for(
    workspace: Option<&str>,
    state: &State<'_, AppState>,
) -> Result<HooksInfo, String> {
    let config = state.config.lock().await.clone();
    let root = workspace.map(PathBuf::from);
    // Read from disk rather than from the run snapshot: the panel's job is to
    // show what is declared right now, including an edit the current run has
    // deliberately not picked up.
    let set = hooks::resolve(root.as_deref(), &config);
    let trust = state
        .hook_trust
        .status(set.workspace.as_deref(), &set.fingerprint);
    let approved_commands = workspace
        .and_then(|w| state.hook_trust.entry(w))
        .map(|e| e.commands)
        .unwrap_or_default();
    let universe = tool_universe();
    Ok(HooksInfo {
        enabled: config.hooks_enabled,
        workspace: set.workspace.clone(),
        trust,
        fingerprint: set.fingerprint.clone(),
        approved_commands,
        hooks: set.hooks.iter().map(|h| to_info(h, &universe)).collect(),
        diagnostics: set.diagnostics.clone(),
    })
}

#[tauri::command]
pub async fn hooks_list(
    workspace: Option<String>,
    state: State<'_, AppState>,
) -> Result<HooksInfo, String> {
    info_for(workspace.as_deref(), &state).await
}

/// Record the user's consent for the set they were shown.
///
/// The hash is passed back rather than recomputed here: a set that changed
/// between rendering the list and clicking the button must not be approvable
/// blind.
#[tauri::command]
pub async fn hooks_approve(
    workspace: String,
    hash: String,
    state: State<'_, AppState>,
) -> Result<HooksInfo, String> {
    let info = info_for(Some(&workspace), &state).await?;
    if info.fingerprint != hash {
        return Err("these hooks changed since they were shown to you — review them again".into());
    }
    state.hook_trust.approve(
        &workspace,
        &hash,
        info.hooks.iter().map(|h| h.display.clone()).collect(),
        now_ms(),
    )?;
    info_for(Some(&workspace), &state).await
}

#[tauri::command]
pub async fn hooks_revoke(
    workspace: String,
    state: State<'_, AppState>,
) -> Result<HooksInfo, String> {
    state.hook_trust.revoke(&workspace)?;
    info_for(Some(&workspace), &state).await
}

#[tauri::command]
pub async fn hooks_set_enabled(
    enabled: bool,
    workspace: Option<String>,
    state: State<'_, AppState>,
) -> Result<HooksInfo, String> {
    {
        let mut cfg = state.config.lock().await;
        cfg.hooks_enabled = enabled;
        crate::agent::provider::save_config(&cfg);
    }
    info_for(workspace.as_deref(), &state).await
}

/// Drop the run snapshot so the next message re-reads the config from disk.
///
/// The snapshot exists so a hook cannot change mid-run. This is the escape
/// hatch for the case that makes that hostile: authoring one.
#[tauri::command]
pub async fn hooks_reload(
    workspace: String,
    state: State<'_, AppState>,
) -> Result<HooksInfo, String> {
    let ws = state.workspace(&workspace).await?;
    *ws.hooks.lock().await = None;
    *ws.hooks_fingerprint.lock().await = None;
    info_for(Some(&workspace), &state).await
}

/// Run one hook against a synthetic payload and report what it did.
///
/// The highest-value thing in the panel. A wrong hook config installs cleanly,
/// runs on every prompt and does nothing; a button that prints the exit code is
/// the difference between diagnosing that in a minute and never noticing.
#[tauri::command]
pub async fn hooks_test(
    workspace: String,
    index: usize,
    state: State<'_, AppState>,
) -> Result<HookRunView, String> {
    let config = state.config.lock().await.clone();
    let root = PathBuf::from(&workspace);
    let set = hooks::resolve(Some(&root), &config);
    let hook = set
        .hooks
        .get(index)
        .ok_or_else(|| "that hook no longer exists — reload the list".to_string())?;
    let env = hooks::runner::RunEnv {
        project_dir: workspace.clone(),
        session_id: "hook-test".into(),
        interrupt: None,
    };
    let input = hooks::runner::probe_input(hook.event, "hook-test", &root);
    let run = hooks::runner::run_one(hook, &input, &env).await;
    Ok(HookRunView {
        status: run.status().as_str().to_string(),
        exit_code: run.exit_code,
        duration_ms: run.duration_ms,
        stdout: run.stdout.clone(),
        stderr: run.stderr.clone(),
        decision: run.effect.decision.clone(),
        additional_context: run.effect.additional_context.clone(),
        error: run.effect.error.clone(),
    })
}

/// Fire `SessionEnd` for every workspace's active session.
///
/// Used on logout and on quit. Bounded overall rather than per hook: a flush
/// hook should get to run before the process dies, and a hung one must never be
/// able to hold the window open.
pub async fn fire_session_end_everywhere(
    state: &AppState,
    reason: hooks::SessionEndReason,
    budget: std::time::Duration,
) {
    let config = state.config.lock().await.clone();
    let workspaces: Vec<_> = state.workspaces.lock().await.values().cloned().collect();
    let _ = tokio::time::timeout(budget, async {
        for ws in workspaces {
            let Some(handle) = ws.active_session.lock().await.clone() else {
                continue;
            };
            let ctx = hooks::HookCtx::new(
                ws.resolve_hooks(&config).await,
                state.hook_trust.clone(),
                &handle.id,
                handle.store_path.clone(),
                ws.root.clone(),
            )
            .with_store(crate::agent::persist::SessionStore {
                path: handle.store_path.clone(),
            });
            hooks::fire_session_end(&ctx, reason, None).await;
        }
    })
    .await;
}

#[cfg(test)]
mod tests {
    use super::*;

    fn hook(event: HookEvent, matcher: Option<&str>) -> ResolvedHook {
        ResolvedHook {
            event,
            matcher: matcher.map(str::to_string),
            command: "run.sh".into(),
            args: vec!["a".into()],
            timeout_secs: 10,
            status_message: None,
            source: HookSource::AppConfig,
            plugin_root: None,
            also_from: Vec::new(),
            order: (0, 0, 0),
        }
    }

    #[test]
    fn listing_says_what_each_matcher_will_hit() {
        let universe = tool_universe();
        assert!(universe.contains(&"edit_file".to_string()));
        let info = to_info(
            &hook(HookEvent::PreToolUse, Some("^(Edit|Write)$")),
            &universe,
        );
        assert_eq!(info.hits, vec!["edit_file"]);
        // Unanchored, `Write` also reaches `TodoWrite` -> `tasks_set`, exactly
        // as it does in Claude Code. The panel showing that is the point.
        let loose = to_info(&hook(HookEvent::PreToolUse, Some("Edit|Write")), &universe);
        assert!(loose.hits.contains(&"tasks_set".to_string()));
        assert!(info.matcher_valid);
        assert_eq!(info.display, "run.sh a");
        assert_eq!(info.source_kind, "appConfig");
    }

    #[test]
    fn a_matcher_that_hits_nothing_is_visible_as_such() {
        let universe = tool_universe();
        let info = to_info(&hook(HookEvent::PreToolUse, Some("Nonexistent")), &universe);
        assert!(info.hits.is_empty());
        let broken = to_info(&hook(HookEvent::PreToolUse, Some("Edit(")), &universe);
        assert!(!broken.matcher_valid);
    }

    #[test]
    fn the_literal_events_report_their_triggers() {
        let universe = tool_universe();
        let compact = to_info(&hook(HookEvent::PreCompact, Some("manual")), &universe);
        assert_eq!(compact.hits, vec!["manual"]);
        let start = to_info(&hook(HookEvent::SessionStart, None), &universe);
        assert_eq!(start.hits, vec!["startup", "resume", "clear", "compact"]);
        // An event with no matcher dimension reports none rather than lying.
        assert!(
            to_info(&hook(HookEvent::Stop, None), &universe)
                .hits
                .is_empty()
        );
    }
}
