//! What a hook *is*, on disk.
//!
//! The schema is Claude Code's, field for field, because the whole point of this
//! module is that a config somebody already wrote for another harness works here
//! without being rewritten. That constraint is what decides every judgement call
//! below: where the two shapes disagree, Claude Code wins; where Claude Code is
//! silent, we accept and ignore rather than reject.
//!
//! Parsing never fails. A malformed block yields an empty set and a diagnostic,
//! the same posture `crate::quality::config` takes and for the same reason: a
//! typo must not be able to take a feature down silently, and it must not be
//! able to take the *session* down loudly either.

use serde::{Deserialize, Serialize};
use serde_json::Value;
use std::collections::HashMap;

/// The nine lifecycle events, named exactly as a config file names them.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash, PartialOrd, Ord, Serialize, Deserialize)]
pub enum HookEvent {
    PreToolUse,
    PostToolUse,
    UserPromptSubmit,
    Notification,
    Stop,
    SubagentStop,
    PreCompact,
    SessionStart,
    SessionEnd,
}

impl HookEvent {
    pub const ALL: &'static [HookEvent] = &[
        HookEvent::PreToolUse,
        HookEvent::PostToolUse,
        HookEvent::UserPromptSubmit,
        HookEvent::Notification,
        HookEvent::Stop,
        HookEvent::SubagentStop,
        HookEvent::PreCompact,
        HookEvent::SessionStart,
        HookEvent::SessionEnd,
    ];

    pub fn as_str(&self) -> &'static str {
        match self {
            HookEvent::PreToolUse => "PreToolUse",
            HookEvent::PostToolUse => "PostToolUse",
            HookEvent::UserPromptSubmit => "UserPromptSubmit",
            HookEvent::Notification => "Notification",
            HookEvent::Stop => "Stop",
            HookEvent::SubagentStop => "SubagentStop",
            HookEvent::PreCompact => "PreCompact",
            HookEvent::SessionStart => "SessionStart",
            HookEvent::SessionEnd => "SessionEnd",
        }
    }

    pub fn parse(s: &str) -> Option<HookEvent> {
        HookEvent::ALL.iter().copied().find(|e| e.as_str() == s)
    }

    /// Whether stdout on exit 0 becomes context the model reads.
    ///
    /// Claude Code splices plain stdout into the conversation for exactly these
    /// two events; everywhere else stdout is transcript and UI only. Widening
    /// this would let a `PostToolUse` hook that prints a debug line quietly
    /// become a prompt injection.
    pub fn plain_stdout_is_context(&self) -> bool {
        matches!(self, HookEvent::UserPromptSubmit | HookEvent::SessionStart)
    }

    /// Whether exit code 2 blocks anything. For the four purely observational
    /// events it does not — Claude Code treats a failing SessionStart hook as a
    /// warning, not as a refusal to start the session.
    pub fn exit_two_blocks(&self) -> bool {
        matches!(
            self,
            HookEvent::PreToolUse
                | HookEvent::PostToolUse
                | HookEvent::UserPromptSubmit
                | HookEvent::Stop
                | HookEvent::SubagentStop
        )
    }
}

/// The default a hook gets when it does not name its own deadline. Claude Code's
/// number, in seconds.
pub const DEFAULT_TIMEOUT_SECS: u64 = 60;
const MIN_TIMEOUT_SECS: u64 = 1;
const MAX_TIMEOUT_SECS: u64 = 600;

/// One command to run.
///
/// `args` and `statusMessage` are not in Claude Code's published settings schema
/// but are in the plugin hook configs it ships and reads — see
/// `claudinio-brain/hooks/hooks.json`, the acceptance artifact for this module.
/// Supporting them is the difference between that file working and that file
/// running `brain-hook.sh` with no argument.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(tag = "type", rename_all = "lowercase")]
pub enum HookAction {
    Command {
        command: String,
        #[serde(default, skip_serializing_if = "Vec::is_empty")]
        args: Vec<String>,
        #[serde(default, skip_serializing_if = "Option::is_none")]
        timeout: Option<u64>,
        #[serde(
            default,
            rename = "statusMessage",
            skip_serializing_if = "Option::is_none"
        )]
        status_message: Option<String>,
    },
}

impl HookAction {
    pub fn command(&self) -> &str {
        match self {
            HookAction::Command { command, .. } => command,
        }
    }
    pub fn args(&self) -> &[String] {
        match self {
            HookAction::Command { args, .. } => args,
        }
    }
    pub fn timeout_secs(&self) -> u64 {
        match self {
            HookAction::Command { timeout, .. } => timeout
                .unwrap_or(DEFAULT_TIMEOUT_SECS)
                .clamp(MIN_TIMEOUT_SECS, MAX_TIMEOUT_SECS),
        }
    }
    pub fn status_message(&self) -> Option<&str> {
        match self {
            HookAction::Command { status_message, .. } => status_message.as_deref(),
        }
    }
}

/// A matcher and the commands it selects.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct HookGroup {
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub matcher: Option<String>,
    pub hooks: Vec<HookAction>,
}

/// A whole `hooks` block: events to groups.
pub type HooksBlock = HashMap<HookEvent, Vec<HookGroup>>;

/// Parse `{ "PreToolUse": [ { "matcher": ..., "hooks": [...] } ] }`.
///
/// Everything it cannot understand becomes a line in `diags` and is dropped.
/// Nothing here returns `Err`: the caller is a session that must start whether
/// or not somebody's settings file is well formed.
pub fn parse_block(v: &Value, diags: &mut Vec<String>) -> HooksBlock {
    let mut out: HooksBlock = HashMap::new();
    let Some(obj) = v.as_object() else {
        diags.push("`hooks` is not an object — ignored".into());
        return out;
    };
    for (key, groups_v) in obj {
        let Some(event) = HookEvent::parse(key) else {
            diags.push(format!("unknown hook event `{key}` — ignored"));
            continue;
        };
        let Some(groups) = groups_v.as_array() else {
            diags.push(format!("`hooks.{key}` is not an array — ignored"));
            continue;
        };
        let mut parsed: Vec<HookGroup> = Vec::new();
        for (gi, g) in groups.iter().enumerate() {
            let Some(gobj) = g.as_object() else {
                diags.push(format!("`hooks.{key}[{gi}]` is not an object — ignored"));
                continue;
            };
            let matcher = gobj
                .get("matcher")
                .and_then(Value::as_str)
                .map(str::to_string);

            // Two accepted shapes. The nested one is Claude Code's; the flat one
            // (a bare command object where a group is expected) is a mistake
            // people make often enough that guessing is kinder than dropping.
            let actions_src: Vec<&Value> = match gobj.get("hooks") {
                Some(Value::Array(a)) => a.iter().collect(),
                Some(_) => {
                    diags.push(format!(
                        "`hooks.{key}[{gi}].hooks` is not an array — ignored"
                    ));
                    continue;
                }
                None if gobj.contains_key("command") => vec![g],
                None => {
                    diags.push(format!("`hooks.{key}[{gi}]` has no `hooks` — ignored"));
                    continue;
                }
            };

            let mut actions: Vec<HookAction> = Vec::new();
            for (hi, h) in actions_src.iter().enumerate() {
                match parse_action(h) {
                    Ok(a) => actions.push(a),
                    Err(why) => diags.push(format!("`hooks.{key}[{gi}].hooks[{hi}]`: {why}")),
                }
            }
            if actions.is_empty() {
                continue;
            }
            parsed.push(HookGroup {
                matcher,
                hooks: actions,
            });
        }
        if !parsed.is_empty() {
            out.entry(event).or_default().extend(parsed);
        }
    }
    out
}

fn parse_action(v: &Value) -> Result<HookAction, String> {
    let obj = v.as_object().ok_or("not an object")?;
    // `type` defaults to "command": it is the only type that exists, and a
    // config that omits it means the obvious thing.
    let ty = obj.get("type").and_then(Value::as_str).unwrap_or("command");
    if ty != "command" {
        return Err(format!("unsupported hook type `{ty}`"));
    }
    let command = obj
        .get("command")
        .and_then(Value::as_str)
        .filter(|s| !s.trim().is_empty())
        .ok_or("missing `command`")?
        .to_string();
    let args = match obj.get("args") {
        None | Some(Value::Null) => Vec::new(),
        Some(Value::Array(a)) => {
            let mut v = Vec::with_capacity(a.len());
            for x in a {
                match x.as_str() {
                    Some(s) => v.push(s.to_string()),
                    None => return Err("`args` must be an array of strings".into()),
                }
            }
            v
        }
        Some(_) => return Err("`args` must be an array of strings".into()),
    };
    let timeout = obj.get("timeout").and_then(|t| {
        t.as_u64()
            .or_else(|| t.as_f64().map(|f| f.max(0.0).round() as u64))
    });
    let status_message = obj
        .get("statusMessage")
        .and_then(Value::as_str)
        .map(str::to_string);
    Ok(HookAction::Command {
        command,
        args,
        timeout,
        status_message,
    })
}

/// Parse a whole file that may or may not wrap its event map in `"hooks"`.
///
/// `settings.json` nests it. Plugin `hooks/hooks.json` files nest it too, and
/// some of them carry a sibling `"description"`. A bare event map appears in the
/// wild as well. Disambiguated by looking at the keys rather than by guessing
/// from the filename.
pub fn parse_file(text: &str, diags: &mut Vec<String>) -> HooksBlock {
    let v: Value = match serde_json::from_str(text) {
        Ok(v) => v,
        Err(e) => {
            diags.push(format!("not valid JSON: {e}"));
            return HooksBlock::new();
        }
    };
    match v.get("hooks") {
        Some(inner) => parse_block(inner, diags),
        None => parse_block(&v, diags),
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    /// The exact bytes of `claudinio-brain/hooks/hooks.json`. If this stops
    /// parsing, the feature has stopped meaning what it was built to mean.
    const BRAIN_HOOKS: &str = r#"{
      "hooks": {
        "SessionStart": [{"hooks": [{"type": "command",
          "command": "${CLAUDE_PLUGIN_ROOT}/hooks/brain-hook.sh",
          "args": ["context"], "timeout": 15,
          "statusMessage": "Reading this project's brain"}]}],
        "UserPromptSubmit": [{"hooks": [{"type": "command",
          "command": "${CLAUDE_PLUGIN_ROOT}/hooks/brain-hook.sh",
          "args": ["recall"], "timeout": 10,
          "statusMessage": "Asking the brain"}]}],
        "PreCompact": [{"hooks": [{"type": "command",
          "command": "${CLAUDE_PLUGIN_ROOT}/hooks/brain-hook.sh",
          "args": ["flush"], "timeout": 10,
          "statusMessage": "Asking for anything worth keeping"}]}]
      }
    }"#;

    #[test]
    fn parses_the_published_brain_hooks_json() {
        let mut d = Vec::new();
        let b = parse_file(BRAIN_HOOKS, &mut d);
        assert!(d.is_empty(), "{d:?}");
        assert_eq!(b.len(), 3);
        let g = &b[&HookEvent::UserPromptSubmit][0];
        assert!(g.matcher.is_none());
        assert_eq!(g.hooks[0].args(), ["recall"]);
        assert_eq!(g.hooks[0].timeout_secs(), 10);
        assert_eq!(g.hooks[0].status_message(), Some("Asking the brain"));
        assert!(g.hooks[0].command().contains("${CLAUDE_PLUGIN_ROOT}"));
    }

    #[test]
    fn a_bare_event_map_and_a_hooks_wrapper_parse_the_same() {
        let mut d = Vec::new();
        let wrapped = parse_file(
            r#"{"description":"x","hooks":{"Stop":[{"hooks":[{"command":"a"}]}]}}"#,
            &mut d,
        );
        let bare = parse_file(r#"{"Stop":[{"hooks":[{"command":"a"}]}]}"#, &mut d);
        assert_eq!(wrapped, bare);
        assert!(d.is_empty(), "{d:?}");
    }

    #[test]
    fn an_unknown_event_name_is_dropped_with_a_diagnostic() {
        let mut d = Vec::new();
        let b = parse_file(
            r#"{"hooks":{"OnTuesday":[{"hooks":[{"command":"a"}]}]}}"#,
            &mut d,
        );
        assert!(b.is_empty());
        assert_eq!(d.len(), 1);
        assert!(d[0].contains("OnTuesday"));
    }

    #[test]
    fn a_malformed_hooks_block_yields_an_empty_set_not_an_error() {
        let mut d = Vec::new();
        assert!(parse_file("{ not json", &mut d).is_empty());
        assert!(parse_file(r#"{"hooks":42}"#, &mut d).is_empty());
        assert!(parse_file(r#"{"hooks":{"Stop":"nope"}}"#, &mut d).is_empty());
        assert!(parse_file(r#"{"hooks":{"Stop":[{"hooks":[{}]}]}}"#, &mut d).is_empty());
        assert_eq!(d.len(), 4);
    }

    #[test]
    fn a_flat_command_where_a_group_belongs_is_accepted() {
        let mut d = Vec::new();
        let b = parse_file(
            r#"{"hooks":{"Stop":[{"command":"a","timeout":5}]}}"#,
            &mut d,
        );
        assert!(d.is_empty(), "{d:?}");
        assert_eq!(b[&HookEvent::Stop][0].hooks[0].timeout_secs(), 5);
    }

    #[test]
    fn timeout_is_clamped_and_defaults_to_sixty_seconds() {
        let mut d = Vec::new();
        let b = parse_file(
            r#"{"hooks":{"Stop":[{"hooks":[{"command":"a"},{"command":"b","timeout":0},{"command":"c","timeout":99999}]}]}}"#,
            &mut d,
        );
        let h = &b[&HookEvent::Stop][0].hooks;
        assert_eq!(h[0].timeout_secs(), DEFAULT_TIMEOUT_SECS);
        assert_eq!(h[1].timeout_secs(), MIN_TIMEOUT_SECS);
        assert_eq!(h[2].timeout_secs(), MAX_TIMEOUT_SECS);
    }

    #[test]
    fn every_event_round_trips_through_its_name() {
        for e in HookEvent::ALL {
            assert_eq!(HookEvent::parse(e.as_str()), Some(*e));
        }
        assert_eq!(HookEvent::ALL.len(), 9);
    }

    #[test]
    fn only_prompt_and_session_start_treat_plain_stdout_as_context() {
        let ctx: Vec<_> = HookEvent::ALL
            .iter()
            .filter(|e| e.plain_stdout_is_context())
            .collect();
        assert_eq!(
            ctx,
            vec![&HookEvent::UserPromptSubmit, &HookEvent::SessionStart]
        );
    }
}
