//! What a hook's answer means.
//!
//! A hook says three things and only three: an exit code, whatever it wrote on
//! stdout, and whatever it wrote on stderr. This file turns that into decisions
//! the agent loop can act on, and it is the file where being wrong is expensive:
//! read a `deny` as an `allow` and a guard hook stops guarding.
//!
//! One rule dominates the parsing: **unknown JSON fields are ignored, never
//! rejected**. `claudinio-brain/docs/harnesses.md` documents a live bug in
//! another harness whose PreCompact wire type is `deny_unknown_fields` and which
//! therefore errors on every compaction and injects nothing. That harness's
//! hooks "install cleanly and do nothing", which is the exact outcome this
//! module exists to prevent.

use super::config::HookEvent;
use serde::{Deserialize, Serialize};

/// The JSON a hook may print on stdout.
#[derive(Debug, Clone, Default, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct HookJsonOutput {
    /// `false` stops the whole run once the current batch finishes.
    #[serde(rename = "continue")]
    pub continue_: Option<bool>,
    /// Shown to the user when `continue` is false. Never shown to the model:
    /// it is an explanation of why the run ended, not an instruction to it.
    pub stop_reason: Option<String>,
    /// Hide stdout from the timeline. The transcript record is written anyway —
    /// quiet must not mean unauditable.
    pub suppress_output: Option<bool>,
    /// A warning row for the user.
    pub system_message: Option<String>,
    /// Pre-`hookSpecificOutput` spelling, still the most common in the wild.
    pub decision: Option<String>,
    pub reason: Option<String>,
    pub hook_specific_output: Option<HookSpecificOutput>,
}

#[derive(Debug, Clone, Default, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct HookSpecificOutput {
    pub hook_event_name: Option<String>,
    /// PreToolUse: `allow` | `deny` | `ask`.
    pub permission_decision: Option<String>,
    pub permission_decision_reason: Option<String>,
    /// Text spliced into the model's context.
    pub additional_context: Option<String>,
}

/// How a single hook process ended, before any interpretation.
#[derive(Debug, Clone)]
pub enum RunResult {
    Exited {
        code: i32,
        stdout: String,
        stderr: String,
    },
    Timeout {
        secs: u64,
    },
    SpawnFailed {
        message: String,
    },
    Interrupted,
}

/// What the timeline and the JSONL call this run.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize)]
#[serde(rename_all = "snake_case")]
pub enum HookStatus {
    Ok,
    Blocked,
    Error,
    Timeout,
    SkippedUntrusted,
}

impl HookStatus {
    pub fn as_str(&self) -> &'static str {
        match self {
            HookStatus::Ok => "ok",
            HookStatus::Blocked => "blocked",
            HookStatus::Error => "error",
            HookStatus::Timeout => "timeout",
            HookStatus::SkippedUntrusted => "skipped_untrusted",
        }
    }
}

/// A PreToolUse verdict. Ordered by force: a deny anywhere in the batch wins.
#[derive(Debug, Clone, PartialEq, Eq, Default)]
pub enum PreToolVerdict {
    #[default]
    None,
    Allow {
        reason: Option<String>,
    },
    Ask {
        reason: Option<String>,
    },
    Deny {
        reason: String,
    },
}

impl PreToolVerdict {
    fn rank(&self) -> u8 {
        match self {
            PreToolVerdict::None => 0,
            PreToolVerdict::Allow { .. } => 1,
            PreToolVerdict::Ask { .. } => 2,
            PreToolVerdict::Deny { .. } => 3,
        }
    }
    pub fn reason(&self) -> Option<&str> {
        match self {
            PreToolVerdict::None => None,
            PreToolVerdict::Allow { reason } | PreToolVerdict::Ask { reason } => reason.as_deref(),
            PreToolVerdict::Deny { reason } => Some(reason),
        }
    }
}

/// One hook's contribution.
#[derive(Debug, Clone, Default)]
pub struct HookEffect {
    pub status: Option<HookStatus>,
    pub verdict: PreToolVerdict,
    /// Text fed back to the model as an instruction (exit 2, `decision:"block"`).
    pub blocking_reason: Option<String>,
    /// Text spliced into the model's context.
    pub additional_context: Option<String>,
    /// `continue:false` — stop the run, with this explanation for the user.
    pub stop: Option<String>,
    pub system_message: Option<String>,
    pub suppress_output: bool,
    /// A failure that must be visible but must not change what the agent does.
    pub error: Option<String>,
    /// What to record as this run's decision.
    pub decision: Option<String>,
}

impl HookEffect {
    pub fn status(&self) -> HookStatus {
        self.status.unwrap_or(HookStatus::Ok)
    }
}

/// Turn one process result into one effect.
pub fn interpret(event: HookEvent, result: &RunResult) -> HookEffect {
    match result {
        RunResult::SpawnFailed { message } => HookEffect {
            status: Some(HookStatus::Error),
            error: Some(message.clone()),
            ..Default::default()
        },
        RunResult::Timeout { secs } => HookEffect {
            status: Some(HookStatus::Timeout),
            error: Some(format!("hook timed out after {secs}s")),
            ..Default::default()
        },
        // A user interrupt is the user's decision, not the hook's failure, and
        // must never block the tool it was gating.
        RunResult::Interrupted => HookEffect {
            status: Some(HookStatus::Error),
            error: Some("hook cancelled by the user".into()),
            ..Default::default()
        },
        RunResult::Exited {
            code,
            stdout,
            stderr,
        } => interpret_exit(event, *code, stdout, stderr),
    }
}

fn interpret_exit(event: HookEvent, code: i32, stdout: &str, stderr: &str) -> HookEffect {
    if code == 0 {
        return interpret_success(event, stdout);
    }
    // Exit 2 is the documented "block" channel, and stderr is its message.
    if code == 2 && event.exit_two_blocks() {
        let reason = first_nonempty(stderr)
            .unwrap_or("a hook blocked this")
            .to_string();
        let mut eff = HookEffect {
            status: Some(HookStatus::Blocked),
            blocking_reason: Some(reason.clone()),
            decision: Some("block".into()),
            ..Default::default()
        };
        if event == HookEvent::PreToolUse {
            eff.verdict = PreToolVerdict::Deny { reason };
        }
        return eff;
    }
    // Everything else is a problem the user should see and the agent should
    // ignore. This is what keeps a hook whose binary is not installed from
    // becoming an error on every prompt.
    HookEffect {
        status: Some(HookStatus::Error),
        error: Some(
            first_nonempty(stderr)
                .map(str::to_string)
                .unwrap_or_else(|| format!("hook exited with code {code}")),
        ),
        ..Default::default()
    }
}

fn interpret_success(event: HookEvent, stdout: &str) -> HookEffect {
    let trimmed = stdout.trim();
    if !trimmed.starts_with('{') {
        // Plain text. Context for the two events that splice stdout, and
        // transcript-only everywhere else.
        let mut eff = HookEffect::default();
        if event.plain_stdout_is_context() {
            if let Some(t) = first_nonempty(trimmed) {
                eff.additional_context = Some(t.to_string());
            }
        }
        return eff;
    }
    let parsed: HookJsonOutput = match serde_json::from_str(trimmed) {
        Ok(p) => p,
        Err(e) => {
            return HookEffect {
                status: Some(HookStatus::Error),
                error: Some(format!("hook printed JSON that could not be read: {e}")),
                ..Default::default()
            };
        }
    };
    from_json(event, parsed)
}

/// Map a parsed JSON answer onto an effect.
pub fn from_json(event: HookEvent, p: HookJsonOutput) -> HookEffect {
    let mut eff = HookEffect {
        suppress_output: p.suppress_output.unwrap_or(false),
        system_message: p.system_message.and_then(nonempty),
        ..Default::default()
    };

    if p.continue_ == Some(false) {
        eff.stop = Some(
            p.stop_reason
                .and_then(nonempty)
                .unwrap_or_else(|| "a hook stopped this run".into()),
        );
        eff.decision = Some("stop".into());
    }

    let hso = p.hook_specific_output.unwrap_or_default();
    eff.additional_context = hso.additional_context.and_then(nonempty);

    // Current spelling first; the legacy one only fills what it left empty.
    if event == HookEvent::PreToolUse {
        match hso.permission_decision.as_deref() {
            Some("allow") => {
                eff.verdict = PreToolVerdict::Allow {
                    reason: hso.permission_decision_reason.clone().and_then(nonempty),
                };
                eff.decision = eff.decision.or(Some("allow".into()));
            }
            Some("ask") => {
                eff.verdict = PreToolVerdict::Ask {
                    reason: hso.permission_decision_reason.clone().and_then(nonempty),
                };
                eff.decision = eff.decision.or(Some("ask".into()));
            }
            Some("deny") => {
                let reason = hso
                    .permission_decision_reason
                    .clone()
                    .and_then(nonempty)
                    .unwrap_or_else(|| "a hook denied this tool call".into());
                eff.verdict = PreToolVerdict::Deny {
                    reason: reason.clone(),
                };
                eff.blocking_reason = Some(reason);
                eff.status = Some(HookStatus::Blocked);
                eff.decision = Some("deny".into());
            }
            _ => {}
        }
    }

    match (p.decision.as_deref(), event) {
        (Some("approve"), HookEvent::PreToolUse) if eff.verdict == PreToolVerdict::None => {
            eff.verdict = PreToolVerdict::Allow {
                reason: p.reason.clone().and_then(nonempty),
            };
            eff.decision = eff.decision.or(Some("allow".into()));
        }
        (Some("block"), HookEvent::PreToolUse) if eff.verdict == PreToolVerdict::None => {
            let reason = p
                .reason
                .clone()
                .and_then(nonempty)
                .unwrap_or_else(|| "a hook denied this tool call".into());
            eff.verdict = PreToolVerdict::Deny {
                reason: reason.clone(),
            };
            eff.blocking_reason = Some(reason);
            eff.status = Some(HookStatus::Blocked);
            eff.decision = Some("deny".into());
        }
        (
            Some("block"),
            HookEvent::PostToolUse
            | HookEvent::UserPromptSubmit
            | HookEvent::Stop
            | HookEvent::SubagentStop,
        ) => {
            // `reason` is required here and its absence is a config bug worth
            // naming, but a block with no explanation is still a block.
            eff.blocking_reason = Some(p.reason.clone().and_then(nonempty).unwrap_or_else(|| {
                "a hook blocked this without giving a reason — check the hook's output".into()
            }));
            eff.status = Some(HookStatus::Blocked);
            eff.decision = Some("block".into());
        }
        _ => {}
    }
    eff
}

/// What a whole batch decided.
#[derive(Debug, Clone, Default)]
pub struct BatchOutcome {
    pub verdict: PreToolVerdict,
    pub blocking_reasons: Vec<String>,
    pub additional_context: Vec<String>,
    pub stop: Option<String>,
    pub system_messages: Vec<String>,
    pub errors: Vec<String>,
}

impl BatchOutcome {
    pub fn blocked(&self) -> bool {
        !self.blocking_reasons.is_empty()
    }
    /// The injected context, in the order the hooks were resolved. Stable across
    /// runs so the prompt prefix stays cacheable.
    pub fn context(&self) -> Option<String> {
        if self.additional_context.is_empty() {
            None
        } else {
            Some(self.additional_context.join("\n\n"))
        }
    }
    pub fn blocking_message(&self) -> Option<String> {
        if self.blocking_reasons.is_empty() {
            None
        } else {
            Some(self.blocking_reasons.join("\n\n"))
        }
    }
}

/// Fold a batch. Effects arrive in resolution order and stay in it.
pub fn aggregate(effects: &[HookEffect]) -> BatchOutcome {
    let mut out = BatchOutcome::default();
    for e in effects {
        if e.verdict.rank() > out.verdict.rank() {
            out.verdict = e.verdict.clone();
        }
        if let Some(r) = &e.blocking_reason {
            out.blocking_reasons.push(r.clone());
        }
        if let Some(c) = &e.additional_context {
            out.additional_context.push(c.clone());
        }
        if let Some(s) = &e.stop {
            // First stop wins; the rest are recorded as system messages so a
            // second opinion is not silently dropped.
            if out.stop.is_none() {
                out.stop = Some(s.clone());
            } else {
                out.system_messages.push(s.clone());
            }
        }
        if let Some(m) = &e.system_message {
            out.system_messages.push(m.clone());
        }
        if let Some(err) = &e.error {
            out.errors.push(err.clone());
        }
    }
    out
}

fn nonempty(s: String) -> Option<String> {
    let t = s.trim();
    if t.is_empty() {
        None
    } else {
        Some(t.to_string())
    }
}

fn first_nonempty(s: &str) -> Option<&str> {
    let t = s.trim();
    if t.is_empty() { None } else { Some(t) }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn exited(code: i32, out: &str, err: &str) -> RunResult {
        RunResult::Exited {
            code,
            stdout: out.into(),
            stderr: err.into(),
        }
    }

    #[test]
    fn exit_zero_stdout_is_context_only_for_prompt_and_session_start() {
        for e in [HookEvent::UserPromptSubmit, HookEvent::SessionStart] {
            let eff = interpret(e, &exited(0, "remember: the port is 8080", ""));
            assert_eq!(
                eff.additional_context.as_deref(),
                Some("remember: the port is 8080")
            );
        }
        for e in [
            HookEvent::PostToolUse,
            HookEvent::Stop,
            HookEvent::Notification,
        ] {
            let eff = interpret(e, &exited(0, "some chatter", ""));
            assert!(eff.additional_context.is_none());
            assert_eq!(eff.status(), HookStatus::Ok);
        }
    }

    #[test]
    fn exit_two_blocks_pretooluse_and_feeds_stderr_to_the_model() {
        let eff = interpret(HookEvent::PreToolUse, &exited(2, "", "no rm in this repo"));
        assert_eq!(
            eff.verdict,
            PreToolVerdict::Deny {
                reason: "no rm in this repo".into()
            }
        );
        assert_eq!(eff.blocking_reason.as_deref(), Some("no rm in this repo"));
        assert_eq!(eff.status(), HookStatus::Blocked);
    }

    #[test]
    fn exit_two_is_non_blocking_on_the_observational_events() {
        for e in [
            HookEvent::SessionStart,
            HookEvent::SessionEnd,
            HookEvent::Notification,
            HookEvent::PreCompact,
        ] {
            let eff = interpret(e, &exited(2, "", "something went wrong"));
            assert!(eff.blocking_reason.is_none(), "{e:?}");
            assert_eq!(eff.status(), HookStatus::Error);
            assert_eq!(eff.error.as_deref(), Some("something went wrong"));
        }
    }

    #[test]
    fn exit_one_is_non_blocking_everywhere() {
        for e in HookEvent::ALL {
            let eff = interpret(*e, &exited(1, "", "brain not installed"));
            assert!(eff.blocking_reason.is_none(), "{e:?}");
            assert_eq!(eff.verdict, PreToolVerdict::None);
            assert_eq!(eff.status(), HookStatus::Error);
        }
    }

    #[test]
    fn a_timeout_and_a_missing_binary_are_non_blocking_errors() {
        let t = interpret(HookEvent::PreToolUse, &RunResult::Timeout { secs: 10 });
        assert_eq!(t.status(), HookStatus::Timeout);
        assert_eq!(t.verdict, PreToolVerdict::None);
        let s = interpret(
            HookEvent::PreToolUse,
            &RunResult::SpawnFailed {
                message: "No such file".into(),
            },
        );
        assert_eq!(s.status(), HookStatus::Error);
        assert!(s.blocking_reason.is_none());
    }

    #[test]
    fn the_brain_envelope_becomes_context() {
        let out = r#"{"hookSpecificOutput":{"hookEventName":"UserPromptSubmit","additionalContext":"the port is 8080"}}"#;
        let eff = interpret(HookEvent::UserPromptSubmit, &exited(0, out, ""));
        assert_eq!(eff.additional_context.as_deref(), Some("the port is 8080"));
        assert_eq!(eff.status(), HookStatus::Ok);
    }

    #[test]
    fn an_empty_object_says_nothing_and_is_not_an_error() {
        let eff = interpret(HookEvent::UserPromptSubmit, &exited(0, "{}\n", ""));
        assert!(eff.additional_context.is_none());
        assert!(eff.blocking_reason.is_none());
        assert_eq!(eff.status(), HookStatus::Ok);
    }

    #[test]
    fn unknown_json_fields_are_ignored() {
        // Named for the bug in claudinio-brain/docs/harnesses.md: a harness that
        // rejects unknown fields rejects every hookSpecificOutput it is sent.
        let out = r#"{"cancel":false,"contextModification":"x","additional_context":"y",
                      "hookSpecificOutput":{"hookEventName":"SessionStart","additionalContext":"kept","futureField":1}}"#;
        let eff = interpret(HookEvent::SessionStart, &exited(0, out, ""));
        assert_eq!(eff.additional_context.as_deref(), Some("kept"));
        assert_eq!(eff.status(), HookStatus::Ok);
    }

    #[test]
    fn legacy_decision_block_equals_deny() {
        let eff = interpret(
            HookEvent::PreToolUse,
            &exited(0, r#"{"decision":"block","reason":"nope"}"#, ""),
        );
        assert_eq!(
            eff.verdict,
            PreToolVerdict::Deny {
                reason: "nope".into()
            }
        );
        let ok = interpret(
            HookEvent::PreToolUse,
            &exited(0, r#"{"decision":"approve","reason":"fine"}"#, ""),
        );
        assert_eq!(
            ok.verdict,
            PreToolVerdict::Allow {
                reason: Some("fine".into())
            }
        );
    }

    #[test]
    fn a_stop_hook_blocking_without_a_reason_still_blocks() {
        let eff = interpret(HookEvent::Stop, &exited(0, r#"{"decision":"block"}"#, ""));
        assert!(
            eff.blocking_reason
                .unwrap()
                .contains("without giving a reason")
        );
    }

    #[test]
    fn permission_decision_wins_over_the_legacy_spelling() {
        let out = r#"{"decision":"block","reason":"old",
                      "hookSpecificOutput":{"hookEventName":"PreToolUse","permissionDecision":"allow"}}"#;
        let eff = interpret(HookEvent::PreToolUse, &exited(0, out, ""));
        assert!(matches!(eff.verdict, PreToolVerdict::Allow { .. }));
    }

    #[test]
    fn deny_beats_ask_beats_allow() {
        let allow = HookEffect {
            verdict: PreToolVerdict::Allow { reason: None },
            ..Default::default()
        };
        let ask = HookEffect {
            verdict: PreToolVerdict::Ask { reason: None },
            ..Default::default()
        };
        let deny = HookEffect {
            verdict: PreToolVerdict::Deny {
                reason: "no".into(),
            },
            ..Default::default()
        };
        assert!(matches!(
            aggregate(&[allow.clone(), ask.clone()]).verdict,
            PreToolVerdict::Ask { .. }
        ));
        assert!(matches!(
            aggregate(&[deny.clone(), allow.clone()]).verdict,
            PreToolVerdict::Deny { .. }
        ));
        assert!(matches!(
            aggregate(&[ask, deny]).verdict,
            PreToolVerdict::Deny { .. }
        ));
        assert!(matches!(
            aggregate(&[allow]).verdict,
            PreToolVerdict::Allow { .. }
        ));
    }

    #[test]
    fn continue_false_stops_the_run_and_keeps_its_reason() {
        let eff = interpret(
            HookEvent::PostToolUse,
            &exited(0, r#"{"continue":false,"stopReason":"budget spent"}"#, ""),
        );
        assert_eq!(eff.stop.as_deref(), Some("budget spent"));
        let agg = aggregate(&[eff]);
        assert_eq!(agg.stop.as_deref(), Some("budget spent"));
    }

    #[test]
    fn context_is_concatenated_in_order() {
        let a = HookEffect {
            additional_context: Some("first".into()),
            ..Default::default()
        };
        let b = HookEffect {
            additional_context: Some("second".into()),
            ..Default::default()
        };
        assert_eq!(
            aggregate(&[a, b]).context().as_deref(),
            Some("first\n\nsecond")
        );
    }

    #[test]
    fn suppress_output_hides_stdout_but_the_effect_survives() {
        let eff = interpret(
            HookEvent::UserPromptSubmit,
            &exited(
                0,
                r#"{"suppressOutput":true,"hookSpecificOutput":{"additionalContext":"quiet"}}"#,
                "",
            ),
        );
        assert!(eff.suppress_output);
        assert_eq!(eff.additional_context.as_deref(), Some("quiet"));
    }

    #[test]
    fn malformed_json_on_stdout_is_an_error_not_a_block() {
        let eff = interpret(HookEvent::PreToolUse, &exited(0, "{oops", ""));
        assert_eq!(eff.status(), HookStatus::Error);
        assert_eq!(eff.verdict, PreToolVerdict::None);
    }
}
