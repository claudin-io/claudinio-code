//! Hooks: the harness doing what somebody else's shell script says.
//!
//! Claudinio's agent loop is the thing this crate is about, and a hook is a
//! deliberate hole in it — a point where a program the user chose gets to read
//! what is happening and, at five of the nine events, change it. The whole
//! module is built around one constraint: **the protocol is Claude Code's**, not
//! ours. A `hooks` block written for that harness, or for Codex, or for Gemini
//! CLI (all three read the same wire format) works here unedited, which is only
//! true if we resist improving any part of it.
//!
//! The nine events and where they fire:
//!
//! | event | fires |
//! |---|---|
//! | `SessionStart` | a run begins — startup, resume, clear or after compaction |
//! | `UserPromptSubmit` | before the user's text becomes a turn |
//! | `PreToolUse` | before a tool call is dispatched; may allow, ask or deny |
//! | `PostToolUse` | after a tool returns; may feed the model a correction |
//! | `Notification` | the agent is waiting on the user |
//! | `Stop` | the run is about to end; may refuse and continue |
//! | `SubagentStop` | a subagent is about to finish; same |
//! | `PreCompact` | context is about to be compacted or handed off |
//! | `SessionEnd` | the conversation is being cleared, or the app is quitting |
//!
//! Three invariants hold at every one of them, because a hook is code nobody is
//! watching run:
//!
//! - **A failing hook never derails a session.** Only exit code 2 and an
//!   explicit `continue:false` change what the agent does. A missing binary, a
//!   timeout, a crash and a malformed answer are all rows in the timeline and
//!   lines in the JSONL, and the run continues.
//! - **Hooks may relax a prompt; they may never relax a policy.** `allow` skips
//!   an approval dialog. It does not get past the bash deny-list, the browser
//!   scheme check, or Brain mode's read-only rule.
//! - **Nothing runs untrusted.** A workspace's hooks are listed and displayed
//!   from the moment they are found, and spawned only after the user approves
//!   that exact set. See [`trust`].

pub mod config;
pub mod discovery;
pub mod matcher;
pub mod outcome;
pub mod payload;
pub mod runner;
pub mod trust;

pub use config::HookEvent;
pub use discovery::{HookSet, HookSource, ResolvedHook, resolve};
pub use outcome::{BatchOutcome, HookStatus, PreToolVerdict};
pub use payload::{CompactTrigger, SessionEndReason, SessionStartSource};
pub use trust::{TrustStatus, TrustStore};

use crate::agent::persist::{SessionRecord, SessionStore, now_ms};
use crate::agent::session::AgentEvent;
use runner::{HookRun, RunEnv};
use serde_json::Value;
use std::path::PathBuf;
use std::sync::atomic::{AtomicBool, Ordering};
use std::sync::{Arc, Mutex};
use tauri::ipc::Channel;

/// Everything the firing sites need, carried on `ToolContext` so it survives a
/// handoff into a linked session without being rebuilt from scratch.
#[derive(Clone)]
pub struct HookCtx {
    pub set: Arc<HookSet>,
    pub trust: Arc<TrustStore>,
    pub session_id: String,
    pub transcript_path: PathBuf,
    pub cwd: PathBuf,
    pub interrupt: Option<Arc<AtomicBool>>,
    pub store: Option<SessionStore>,
    /// Set by the command layer before a run and taken by the loop at the top of
    /// it, so `SessionStart`'s context lands where it can still reach the model.
    /// A session that is merely *loaded* has no run to inject into; making this
    /// a handoff rather than an immediate call is what stops that context from
    /// being computed and dropped.
    pub pending_session_start: Arc<Mutex<Option<SessionStartSource>>>,
    /// One approval prompt per run, not one per event.
    awaiting_notified: Arc<AtomicBool>,
}

impl HookCtx {
    pub fn new(
        set: Arc<HookSet>,
        trust: Arc<TrustStore>,
        session_id: &str,
        transcript_path: PathBuf,
        cwd: PathBuf,
    ) -> Self {
        Self {
            set,
            trust,
            session_id: session_id.to_string(),
            transcript_path,
            cwd,
            interrupt: None,
            store: None,
            pending_session_start: Arc::new(Mutex::new(None)),
            awaiting_notified: Arc::new(AtomicBool::new(false)),
        }
    }

    /// Re-point at a different session, keeping the resolved set and the
    /// approval. A handoff is the same run continuing under a new id.
    pub fn for_session(&self, session_id: &str, transcript_path: PathBuf) -> Self {
        Self {
            session_id: session_id.to_string(),
            transcript_path,
            store: None,
            ..self.clone()
        }
    }

    pub fn with_store(mut self, store: SessionStore) -> Self {
        self.store = Some(store);
        self
    }

    pub fn with_interrupt(mut self, interrupt: Option<Arc<AtomicBool>>) -> Self {
        self.interrupt = interrupt;
        self
    }

    pub fn trust_status(&self) -> TrustStatus {
        self.trust
            .status(self.set.workspace.as_deref(), &self.set.fingerprint)
    }

    /// Whether any hook is declared for an event at all. Checked before building
    /// a payload so the common case — no hooks — costs one vector scan.
    pub fn has(&self, event: HookEvent) -> bool {
        self.set.hooks.iter().any(|h| h.event == event)
    }

    fn base(&self) -> payload::HookBase {
        payload::HookBase::new(&self.session_id, &self.transcript_path, &self.cwd)
    }

    fn run_env(&self) -> RunEnv {
        RunEnv {
            project_dir: self
                .set
                .workspace
                .clone()
                .unwrap_or_else(|| self.cwd.display().to_string()),
            session_id: self.session_id.clone(),
            interrupt: self.interrupt.clone(),
        }
    }

    fn record(&self, record: &SessionRecord) {
        if let Some(s) = &self.store {
            s.try_append(record);
        }
    }
}

/// The record cap. Deliberately far below the 100 KB the model may see: this
/// file is re-read on every message, so a chatty hook must not be able to grow
/// it without bound.
const RECORD_OUTPUT_CAP: usize = 8 * 1024;

fn clip(s: &str) -> String {
    if s.len() <= RECORD_OUTPUT_CAP {
        return s.to_string();
    }
    let mut cut = RECORD_OUTPUT_CAP;
    while cut > 0 && !s.is_char_boundary(cut) {
        cut -= 1;
    }
    format!("{}…", &s[..cut])
}

/// Run every hook selected for an event and fold the answers.
///
/// This is the only path that spawns anything, which is what makes the trust
/// check here rather than at a dozen call sites a correctness property and not
/// just a convenience.
async fn run(
    ctx: &HookCtx,
    event: HookEvent,
    selected: Vec<&ResolvedHook>,
    input: Value,
    event_tx: Option<&Channel<AgentEvent>>,
) -> BatchOutcome {
    if selected.is_empty() {
        return BatchOutcome::default();
    }
    // Evaluated here, at fire time, rather than snapshotted with the set:
    // approving mid-run must take effect on the very next event, without a
    // restart. That is the difference between opt-in and opt-in eventually.
    if !ctx.trust_status().may_run() {
        announce_awaiting(ctx, event_tx);
        for h in &selected {
            ctx.record(&hook_record(
                h,
                HookStatus::SkippedUntrusted,
                None,
                "",
                "",
                0,
                None,
            ));
        }
        return BatchOutcome::default();
    }

    if let Some(tx) = event_tx {
        for h in &selected {
            let _ = tx.send(AgentEvent::HookStarted {
                session_id: ctx.session_id.clone(),
                hook_id: hook_id(ctx, h),
                event: event.as_str().to_string(),
                command: h.display_command(),
                source: h.source.label(),
                status_message: h.status_message.clone(),
            });
        }
    }

    let runs = runner::run_batch(&selected, &input, &ctx.run_env()).await;

    for r in &runs {
        ctx.record(&hook_record(
            &r.hook,
            r.status(),
            r.exit_code,
            &r.stdout,
            &r.stderr,
            r.duration_ms,
            r.effect.decision.clone(),
        ));
        if let Some(tx) = event_tx {
            let _ = tx.send(finished_event(ctx, r));
        }
    }

    let effects: Vec<outcome::HookEffect> = runs.into_iter().map(|r| r.effect).collect();
    let agg = outcome::aggregate(&effects);

    if let Some(text) = agg.context() {
        ctx.record(&SessionRecord::HookContext {
            event: event.as_str().to_string(),
            source: "hooks".into(),
            text: clip(&text),
            ts: now_ms(),
        });
    }
    agg
}

fn hook_id(ctx: &HookCtx, h: &ResolvedHook) -> String {
    // Stable within a run and unique per (event, command), so the UI can pair a
    // `HookFinished` with the row a `HookStarted` created.
    format!(
        "{}:{}:{:x}",
        ctx.session_id,
        h.event.as_str(),
        seahash(&h.signature())
    )
}

fn seahash(s: &str) -> u64 {
    // Not cryptographic and not meant to be — this only has to pair two UI
    // events. The trust hash, which does have to resist collisions, is SHA-256.
    let mut h: u64 = 0xcbf2_9ce4_8422_2325;
    for b in s.as_bytes() {
        h ^= *b as u64;
        h = h.wrapping_mul(0x0000_0100_0000_01b3);
    }
    h
}

fn finished_event(ctx: &HookCtx, r: &HookRun) -> AgentEvent {
    AgentEvent::HookFinished {
        session_id: ctx.session_id.clone(),
        hook_id: hook_id(ctx, &r.hook),
        event: r.hook.event.as_str().to_string(),
        status: r.status().as_str().to_string(),
        exit_code: r.exit_code,
        duration_ms: r.duration_ms,
        // `suppressOutput` hides the text from the timeline. The JSONL record
        // above still carries it: quiet must not mean unauditable.
        output: if r.effect.suppress_output {
            String::new()
        } else {
            clip(&r.stdout)
        },
        error: r.effect.error.clone(),
        decision: r.effect.decision.clone(),
        system_message: r.effect.system_message.clone(),
    }
}

fn hook_record(
    h: &ResolvedHook,
    status: HookStatus,
    exit_code: Option<i32>,
    stdout: &str,
    stderr: &str,
    duration_ms: u64,
    decision: Option<String>,
) -> SessionRecord {
    SessionRecord::Hook {
        event: h.event.as_str().to_string(),
        matcher: h.matcher.clone(),
        source: h.source.label(),
        command: h.display_command(),
        status: status.as_str().to_string(),
        exit_code,
        duration_ms,
        stdout: clip(stdout),
        stderr: clip(stderr),
        decision,
        ts: now_ms(),
    }
}

fn announce_awaiting(ctx: &HookCtx, event_tx: Option<&Channel<AgentEvent>>) {
    if ctx.awaiting_notified.swap(true, Ordering::SeqCst) {
        return;
    }
    ctx.record(&SessionRecord::HookTrust {
        status: "pending".into(),
        hash: ctx.set.fingerprint.clone(),
        commands: ctx.set.hooks.iter().map(|h| h.display_command()).collect(),
        ts: now_ms(),
    });
    if let Some(tx) = event_tx {
        let _ = tx.send(AgentEvent::HooksAwaitingApproval {
            workspace: ctx.set.workspace.clone().unwrap_or_default(),
            hash: ctx.set.fingerprint.clone(),
            count: ctx.set.hooks.len(),
            commands: ctx.set.hooks.iter().map(|h| h.display_command()).collect(),
        });
    }
}

// ─── The nine entry points ────────────────────────────────────────────────────

pub async fn fire_pre_tool_use(
    ctx: &HookCtx,
    native_tool: &str,
    tool_input: &Value,
    event_tx: Option<&Channel<AgentEvent>>,
) -> BatchOutcome {
    let selected = ctx.set.select_tool(HookEvent::PreToolUse, native_tool);
    if selected.is_empty() {
        return BatchOutcome::default();
    }
    let input = payload::pre_tool_use(&ctx.base(), native_tool, tool_input);
    run(ctx, HookEvent::PreToolUse, selected, input, event_tx).await
}

pub async fn fire_post_tool_use(
    ctx: &HookCtx,
    native_tool: &str,
    tool_input: &Value,
    tool_response: &Value,
    event_tx: Option<&Channel<AgentEvent>>,
) -> BatchOutcome {
    let selected = ctx.set.select_tool(HookEvent::PostToolUse, native_tool);
    if selected.is_empty() {
        return BatchOutcome::default();
    }
    let input = payload::post_tool_use(&ctx.base(), native_tool, tool_input, tool_response);
    run(ctx, HookEvent::PostToolUse, selected, input, event_tx).await
}

pub async fn fire_user_prompt_submit(
    ctx: &HookCtx,
    prompt: &str,
    event_tx: Option<&Channel<AgentEvent>>,
) -> BatchOutcome {
    let selected = ctx.set.select(HookEvent::UserPromptSubmit);
    if selected.is_empty() {
        return BatchOutcome::default();
    }
    let input = payload::user_prompt_submit(&ctx.base(), prompt);
    run(ctx, HookEvent::UserPromptSubmit, selected, input, event_tx).await
}

pub async fn fire_session_start(
    ctx: &HookCtx,
    source: SessionStartSource,
    event_tx: Option<&Channel<AgentEvent>>,
) -> BatchOutcome {
    let selected = ctx
        .set
        .select_literal(HookEvent::SessionStart, source.wire());
    if selected.is_empty() {
        return BatchOutcome::default();
    }
    let input = payload::session_start(&ctx.base(), source);
    run(ctx, HookEvent::SessionStart, selected, input, event_tx).await
}

pub async fn fire_session_end(
    ctx: &HookCtx,
    reason: SessionEndReason,
    event_tx: Option<&Channel<AgentEvent>>,
) -> BatchOutcome {
    let selected = ctx.set.select(HookEvent::SessionEnd);
    if selected.is_empty() {
        return BatchOutcome::default();
    }
    let input = payload::session_end(&ctx.base(), reason);
    run(ctx, HookEvent::SessionEnd, selected, input, event_tx).await
}

pub async fn fire_pre_compact(
    ctx: &HookCtx,
    trigger: CompactTrigger,
    event_tx: Option<&Channel<AgentEvent>>,
) -> BatchOutcome {
    let selected = ctx
        .set
        .select_literal(HookEvent::PreCompact, trigger.wire());
    if selected.is_empty() {
        return BatchOutcome::default();
    }
    let input = payload::pre_compact(&ctx.base(), trigger, "");
    run(ctx, HookEvent::PreCompact, selected, input, event_tx).await
}

pub async fn fire_stop(
    ctx: &HookCtx,
    stop_hook_active: bool,
    event_tx: Option<&Channel<AgentEvent>>,
) -> BatchOutcome {
    let selected = ctx.set.select(HookEvent::Stop);
    if selected.is_empty() {
        return BatchOutcome::default();
    }
    let input = payload::stop(&ctx.base(), stop_hook_active);
    run(ctx, HookEvent::Stop, selected, input, event_tx).await
}

pub async fn fire_subagent_stop(
    ctx: &HookCtx,
    stop_hook_active: bool,
    subagent_id: &str,
    event_tx: Option<&Channel<AgentEvent>>,
) -> BatchOutcome {
    let selected = ctx.set.select(HookEvent::SubagentStop);
    if selected.is_empty() {
        return BatchOutcome::default();
    }
    let input = payload::subagent_stop(&ctx.base(), stop_hook_active, subagent_id);
    run(ctx, HookEvent::SubagentStop, selected, input, event_tx).await
}

/// Notification is spawned and never awaited.
///
/// Its stdout carries no context and no decision, and the two things it fires
/// on — an approval dialog and a question — are the two places the user is
/// already waiting. Making them wait on a notifier as well would be the feature
/// working against its own purpose.
pub fn fire_notification(ctx: &HookCtx, message: &str, event_tx: Option<&Channel<AgentEvent>>) {
    if !ctx.has(HookEvent::Notification) {
        return;
    }
    let ctx = ctx.clone();
    let message = message.to_string();
    let tx = event_tx.cloned();
    tokio::spawn(async move {
        let selected = ctx.set.select(HookEvent::Notification);
        let input = payload::notification(&ctx.base(), &message);
        run(&ctx, HookEvent::Notification, selected, input, tx.as_ref()).await;
    });
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::path::Path;

    fn tmp(name: &str) -> PathBuf {
        let p = std::env::temp_dir().join(format!("cc-hooks-e2e-{name}-{}", std::process::id()));
        std::fs::remove_dir_all(&p).ok();
        std::fs::create_dir_all(&p).unwrap();
        p
    }

    fn write(path: &Path, text: &str) {
        std::fs::create_dir_all(path.parent().unwrap()).unwrap();
        std::fs::write(path, text).unwrap();
    }

    /// The published config from `claudinio-brain/hooks/hooks.json`, byte for
    /// byte. If this test fails, the feature has stopped meaning what it was
    /// built to mean.
    const BRAIN_HOOKS_JSON: &str = r#"{
  "hooks": {
    "SessionStart": [
      {
        "hooks": [
          {
            "type": "command",
            "command": "${CLAUDE_PLUGIN_ROOT}/hooks/brain-hook.sh",
            "args": ["context"],
            "timeout": 15,
            "statusMessage": "Reading this project's brain"
          }
        ]
      }
    ],
    "UserPromptSubmit": [
      {
        "hooks": [
          {
            "type": "command",
            "command": "${CLAUDE_PLUGIN_ROOT}/hooks/brain-hook.sh",
            "args": ["recall"],
            "timeout": 10,
            "statusMessage": "Asking the brain"
          }
        ]
      }
    ],
    "PreCompact": [
      {
        "hooks": [
          {
            "type": "command",
            "command": "${CLAUDE_PLUGIN_ROOT}/hooks/brain-hook.sh",
            "args": ["flush"],
            "timeout": 10,
            "statusMessage": "Asking for anything worth keeping"
          }
        ]
      }
    ]
  }
}"#;

    #[cfg(unix)]
    #[tokio::test]
    async fn the_published_brain_hooks_config_runs_end_to_end() {
        use std::os::unix::fs::PermissionsExt;

        let ws = tmp("brain");
        let plugin = ws.join(".claudinio/plugins/claudinio-brain");
        write(
            &plugin.join("plugin.json"),
            r#"{"$schema":"https://agent-plugins.org/schemas/1.0.0/plugin.schema.json",
                "name":"claudinio-brain","version":"0.1.0"}"#,
        );
        write(&plugin.join("hooks/hooks.json"), BRAIN_HOOKS_JSON);
        let script = plugin.join("hooks/brain-hook.sh");
        write(
            &script,
            "#!/bin/sh\nprintf '{\"hookSpecificOutput\":{\"hookEventName\":\"x\",\"additionalContext\":\"REMEMBERED:%s\"}}' \"$1\"\n",
        );
        std::fs::set_permissions(&script, std::fs::Permissions::from_mode(0o755)).unwrap();

        let mut cfg = crate::agent::provider::AgentConfig {
            hooks_enabled: true,
            ..Default::default()
        };
        cfg.plugins.clear();

        // 1. Discovery finds all three, with ${CLAUDE_PLUGIN_ROOT} expanded.
        let set = discovery::resolve_with_home(Some(&ws), &cfg, Some(&ws.join("_home")));
        assert_eq!(set.hooks.len(), 3, "{:?}", set.diagnostics);
        assert!(set.diagnostics.is_empty(), "{:?}", set.diagnostics);
        for h in &set.hooks {
            assert_eq!(h.command, script.display().to_string());
            assert!(matches!(h.source, HookSource::Plugin { .. }));
        }
        assert_eq!(
            set.select_literal(HookEvent::SessionStart, "startup")[0].timeout_secs,
            15
        );
        assert_eq!(
            set.select(HookEvent::UserPromptSubmit)[0]
                .status_message
                .as_deref(),
            Some("Asking the brain")
        );

        // 2. Untrusted, nothing runs.
        let trust_path = ws.join("trust.json");
        let trust = Arc::new(TrustStore::with_path(trust_path));
        let store = SessionStore {
            path: ws.join("s.jsonl"),
        };
        let ctx = HookCtx::new(
            Arc::new(set),
            trust.clone(),
            "s-1",
            store.path.clone(),
            ws.clone(),
        )
        .with_store(store.clone());
        assert_eq!(ctx.trust_status(), TrustStatus::Pending);
        let blocked = fire_user_prompt_submit(&ctx, "what is the port", None).await;
        assert!(blocked.context().is_none());

        // 3. Approved, the whole envelope round-trips — proving `args` reached
        //    argv rather than being dropped into a shell string.
        trust
            .approve(
                &ws.display().to_string(),
                &ctx.set.fingerprint,
                ctx.set.hooks.iter().map(|h| h.display_command()).collect(),
                1,
            )
            .unwrap();
        assert_eq!(ctx.trust_status(), TrustStatus::Trusted);

        let prompt = fire_user_prompt_submit(&ctx, "what is the port", None).await;
        assert_eq!(prompt.context().as_deref(), Some("REMEMBERED:recall"));

        let start = fire_session_start(&ctx, SessionStartSource::Startup, None).await;
        assert_eq!(start.context().as_deref(), Some("REMEMBERED:context"));

        let compact = fire_pre_compact(&ctx, CompactTrigger::Auto, None).await;
        assert_eq!(compact.context().as_deref(), Some("REMEMBERED:flush"));

        // A handoff is an auto compaction on the wire, so flush fires there too.
        let handoff = fire_pre_compact(&ctx, CompactTrigger::Handoff, None).await;
        assert_eq!(handoff.context().as_deref(), Some("REMEMBERED:flush"));

        // Events the config says nothing about stay silent and cost nothing.
        assert!(fire_stop(&ctx, false, None).await.context().is_none());
        assert!(
            fire_pre_tool_use(&ctx, "bash", &serde_json::json!({}), None)
                .await
                .context()
                .is_none()
        );

        // 4. The JSONL explains what happened, including the skipped run.
        let text = std::fs::read_to_string(&store.path).unwrap();
        assert!(text.contains("skipped_untrusted"));
        assert!(text.contains("\"hook_context\""));
        assert!(text.contains("REMEMBERED:recall"));
        std::fs::remove_dir_all(&ws).ok();
    }

    #[cfg(unix)]
    #[tokio::test]
    async fn a_pre_tool_use_hook_can_deny_a_tool_by_name() {
        use std::os::unix::fs::PermissionsExt;
        let ws = tmp("deny");
        let script = ws.join("guard.sh");
        std::fs::write(&script, "#!/bin/sh\necho 'no edits today' >&2\nexit 2\n").unwrap();
        std::fs::set_permissions(&script, std::fs::Permissions::from_mode(0o755)).unwrap();
        write(
            &ws.join(".claudinio.json"),
            &format!(
                r#"{{"hooks":{{"PreToolUse":[{{"matcher":"Edit|Write","hooks":[{{"type":"command","command":"{}"}}]}}]}}}}"#,
                script.display()
            ),
        );

        let cfg = crate::agent::provider::AgentConfig {
            hooks_enabled: true,
            ..Default::default()
        };
        let set = Arc::new(discovery::resolve_with_home(
            Some(&ws),
            &cfg,
            Some(&ws.join("_home")),
        ));
        let trust = Arc::new(TrustStore::with_path(ws.join("trust.json")));
        trust
            .approve(&ws.display().to_string(), &set.fingerprint, vec![], 1)
            .unwrap();
        let ctx = HookCtx::new(set, trust, "s-1", ws.join("s.jsonl"), ws.clone());

        let denied =
            fire_pre_tool_use(&ctx, "edit_file", &serde_json::json!({"path": "a"}), None).await;
        assert_eq!(
            denied.verdict,
            PreToolVerdict::Deny {
                reason: "no edits today".into()
            }
        );

        // The matcher is what scopes it: bash is untouched.
        let allowed = fire_pre_tool_use(&ctx, "bash", &serde_json::json!({}), None).await;
        assert_eq!(allowed.verdict, PreToolVerdict::None);
        std::fs::remove_dir_all(&ws).ok();
    }

    #[tokio::test]
    async fn an_empty_context_fires_nothing_and_costs_nothing() {
        let ctx = HookCtx::new(
            Arc::new(HookSet::default()),
            Arc::new(TrustStore::with_path(PathBuf::new())),
            "",
            PathBuf::new(),
            PathBuf::new(),
        );
        for e in HookEvent::ALL {
            assert!(!ctx.has(*e));
        }
        assert!(
            fire_user_prompt_submit(&ctx, "hi", None)
                .await
                .context()
                .is_none()
        );
        assert_eq!(ctx.trust_status(), TrustStatus::NoHooks);
    }
}
