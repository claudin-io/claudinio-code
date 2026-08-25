//! What a hook reads on stdin.
//!
//! This is the wire contract, and it is the part of the feature a hook author
//! actually programs against. Every field here is Claude Code's, spelled the way
//! Claude Code spells it, because the acceptance case is somebody else's script
//! doing `jq -r '.prompt'` against a payload it has never seen.
//!
//! Where Claudinio has something Claude Code does not, the extra field is
//! *additive* and prefixed `claudinio_`. Nothing standard is renamed, dropped or
//! given a different meaning — a hook that reads only the documented fields must
//! never be able to tell which harness it is talking to, and a hook that wants
//! to know must be able to find out.

use super::config::HookEvent;
use super::matcher;
use serde_json::{Map, Value, json};
use std::path::Path;

/// The transcript is real and complete, but it is Claudinio's `SessionRecord`
/// JSONL rather than Claude Code's format. A hook that only *locates* the
/// transcript is served correctly; one that parses it can read this marker and
/// bail instead of misreading. Shipping a translator was rejected: it would be a
/// second serializer of an undocumented format, written per prompt, for a field
/// almost nothing reads.
pub const TRANSCRIPT_FORMAT: &str = "claudinio-jsonl/1";

/// Everything every event carries.
#[derive(Debug, Clone)]
pub struct HookBase {
    pub session_id: String,
    pub transcript_path: String,
    pub cwd: String,
}

impl HookBase {
    pub fn new(session_id: &str, transcript_path: &Path, cwd: &Path) -> Self {
        Self {
            session_id: session_id.to_string(),
            transcript_path: transcript_path.to_string_lossy().to_string(),
            cwd: cwd.to_string_lossy().to_string(),
        }
    }

    fn map(&self, event: HookEvent) -> Map<String, Value> {
        let mut m = Map::new();
        m.insert("session_id".into(), json!(self.session_id));
        m.insert("transcript_path".into(), json!(self.transcript_path));
        m.insert("transcript_format".into(), json!(TRANSCRIPT_FORMAT));
        m.insert("cwd".into(), json!(self.cwd));
        m.insert("hook_event_name".into(), json!(event.as_str()));
        m
    }
}

fn finish(m: Map<String, Value>) -> Value {
    Value::Object(m)
}

/// `tool_input`, plus the key spellings a Claude Code hook expects.
///
/// Purely additive: `jq '.tool_input.file_path'` is what published hooks do, and
/// Claudinio calls that key `path`. Renaming would break a hook written against
/// Claudinio; adding breaks nobody, and the duplication is two strings.
fn aliased_tool_input(native_tool: &str, tool_input: &Value) -> Value {
    let Some(obj) = tool_input.as_object() else {
        return tool_input.clone();
    };
    let mut m = obj.clone();
    if matches!(native_tool, "read_file" | "edit_file" | "list_dir") && !m.contains_key("file_path")
    {
        if let Some(p) = obj.get("path") {
            m.insert("file_path".into(), p.clone());
        }
    }
    if native_tool == "edit_file" {
        if let Some(v) = obj.get("old_string") {
            m.entry("old_string").or_insert(v.clone());
        }
        if let Some(v) = obj.get("new_string") {
            m.entry("new_string").or_insert(v.clone());
        }
    }
    Value::Object(m)
}

fn insert_tool(m: &mut Map<String, Value>, native_tool: &str, tool_input: &Value) {
    // The canonical Claude Code name goes in `tool_name`, because that is what a
    // published matcher and a published `case` statement both compare against.
    // The native name is never hidden — it rides alongside.
    m.insert(
        "tool_name".into(),
        json!(matcher::canonical_alias(native_tool)),
    );
    m.insert("claudinio_tool_name".into(), json!(native_tool));
    m.insert(
        "tool_input".into(),
        aliased_tool_input(native_tool, tool_input),
    );
}

pub fn pre_tool_use(base: &HookBase, native_tool: &str, tool_input: &Value) -> Value {
    let mut m = base.map(HookEvent::PreToolUse);
    insert_tool(&mut m, native_tool, tool_input);
    finish(m)
}

pub fn post_tool_use(
    base: &HookBase,
    native_tool: &str,
    tool_input: &Value,
    tool_response: &Value,
) -> Value {
    let mut m = base.map(HookEvent::PostToolUse);
    insert_tool(&mut m, native_tool, tool_input);
    m.insert("tool_response".into(), tool_response.clone());
    finish(m)
}

pub fn user_prompt_submit(base: &HookBase, prompt: &str) -> Value {
    let mut m = base.map(HookEvent::UserPromptSubmit);
    m.insert("prompt".into(), json!(prompt));
    finish(m)
}

pub fn notification(base: &HookBase, message: &str) -> Value {
    let mut m = base.map(HookEvent::Notification);
    m.insert("message".into(), json!(message));
    finish(m)
}

pub fn stop(base: &HookBase, stop_hook_active: bool) -> Value {
    let mut m = base.map(HookEvent::Stop);
    m.insert("stop_hook_active".into(), json!(stop_hook_active));
    finish(m)
}

pub fn subagent_stop(base: &HookBase, stop_hook_active: bool, subagent_id: &str) -> Value {
    let mut m = base.map(HookEvent::SubagentStop);
    m.insert("stop_hook_active".into(), json!(stop_hook_active));
    m.insert("claudinio_subagent_id".into(), json!(subagent_id));
    finish(m)
}

/// `manual` or `auto` — the two words Claude Code uses, and the two a matcher is
/// written against.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum CompactTrigger {
    Manual,
    Auto,
    /// Claudinio's own context handoff. It reports as `auto` on the wire,
    /// because a flush hook that fires on compaction and not on the mechanism
    /// that destroys context *harder* would miss the case it exists for. The
    /// distinction is still available, under a namespaced key.
    Handoff,
}

impl CompactTrigger {
    pub fn wire(&self) -> &'static str {
        match self {
            CompactTrigger::Manual => "manual",
            CompactTrigger::Auto | CompactTrigger::Handoff => "auto",
        }
    }
    pub fn native(&self) -> &'static str {
        match self {
            CompactTrigger::Manual => "compact_manual",
            CompactTrigger::Auto => "compact_auto",
            CompactTrigger::Handoff => "handoff",
        }
    }
}

pub fn pre_compact(base: &HookBase, trigger: CompactTrigger, custom_instructions: &str) -> Value {
    let mut m = base.map(HookEvent::PreCompact);
    m.insert("trigger".into(), json!(trigger.wire()));
    m.insert("custom_instructions".into(), json!(custom_instructions));
    m.insert("claudinio_trigger".into(), json!(trigger.native()));
    finish(m)
}

/// Why this session is starting.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum SessionStartSource {
    Startup,
    Resume,
    Clear,
    Compact,
}

impl SessionStartSource {
    pub fn wire(&self) -> &'static str {
        match self {
            SessionStartSource::Startup => "startup",
            SessionStartSource::Resume => "resume",
            SessionStartSource::Clear => "clear",
            SessionStartSource::Compact => "compact",
        }
    }
}

pub fn session_start(base: &HookBase, source: SessionStartSource) -> Value {
    let mut m = base.map(HookEvent::SessionStart);
    m.insert("source".into(), json!(source.wire()));
    finish(m)
}

/// Why this session is ending. `prompt_input_exit` is deliberately absent — it
/// names a terminal interaction Claudinio does not have, and emitting it would
/// be describing something that did not happen.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum SessionEndReason {
    Clear,
    Logout,
    Other,
}

impl SessionEndReason {
    pub fn wire(&self) -> &'static str {
        match self {
            SessionEndReason::Clear => "clear",
            SessionEndReason::Logout => "logout",
            SessionEndReason::Other => "other",
        }
    }
}

pub fn session_end(base: &HookBase, reason: SessionEndReason) -> Value {
    let mut m = base.map(HookEvent::SessionEnd);
    m.insert("reason".into(), json!(reason.wire()));
    finish(m)
}

/// A tool's result, as a hook sees it.
///
/// Capped, because a hook is handed this on stdin and `read_file` on a large
/// file would otherwise put megabytes through a pipe on every call.
pub fn tool_response(success: bool, output: &str, limit: usize) -> Value {
    let (text, truncated) = if output.len() > limit {
        let mut cut = limit;
        while cut > 0 && !output.is_char_boundary(cut) {
            cut -= 1;
        }
        (&output[..cut], true)
    } else {
        (output, false)
    };
    json!({ "success": success, "output": text, "truncated": truncated })
}

#[cfg(test)]
mod tests {
    use super::*;

    fn base() -> HookBase {
        HookBase::new(
            "s-1",
            Path::new("/ws/.claudinio/sessions/s-1.jsonl"),
            Path::new("/ws"),
        )
    }

    fn keys(v: &Value) -> Vec<String> {
        let mut k: Vec<String> = v.as_object().unwrap().keys().cloned().collect();
        k.sort();
        k
    }

    const COMMON: [&str; 5] = [
        "cwd",
        "hook_event_name",
        "session_id",
        "transcript_format",
        "transcript_path",
    ];

    fn with(extra: &[&str]) -> Vec<String> {
        let mut k: Vec<String> = COMMON
            .iter()
            .chain(extra.iter())
            .map(|s| s.to_string())
            .collect();
        k.sort();
        k
    }

    #[test]
    fn every_event_carries_the_common_fields() {
        let b = base();
        let all = [
            pre_tool_use(&b, "bash", &json!({"command": "ls"})),
            user_prompt_submit(&b, "hi"),
            notification(&b, "m"),
            stop(&b, false),
            pre_compact(&b, CompactTrigger::Auto, ""),
            session_start(&b, SessionStartSource::Startup),
            session_end(&b, SessionEndReason::Clear),
        ];
        for v in all {
            for k in COMMON {
                assert!(v.get(k).is_some(), "{k} missing from {v}");
            }
            assert_eq!(v["session_id"], "s-1");
            assert_eq!(v["cwd"], "/ws");
            assert_eq!(v["transcript_format"], TRANSCRIPT_FORMAT);
        }
    }

    #[test]
    fn pre_tool_use_pins_its_field_set() {
        let v = pre_tool_use(&base(), "bash", &json!({"command": "ls"}));
        assert_eq!(
            keys(&v),
            with(&["tool_name", "claudinio_tool_name", "tool_input"])
        );
        assert_eq!(v["hook_event_name"], "PreToolUse");
        assert_eq!(v["tool_name"], "Bash");
        assert_eq!(v["claudinio_tool_name"], "bash");
        assert_eq!(v["tool_input"]["command"], "ls");
    }

    #[test]
    fn post_tool_use_pins_its_field_set() {
        let v = post_tool_use(
            &base(),
            "bash",
            &json!({"command": "ls"}),
            &tool_response(true, "a\nb", 100),
        );
        assert_eq!(
            keys(&v),
            with(&[
                "tool_name",
                "claudinio_tool_name",
                "tool_input",
                "tool_response"
            ])
        );
        assert_eq!(v["tool_response"]["success"], true);
        assert_eq!(v["tool_response"]["output"], "a\nb");
    }

    #[test]
    fn an_edit_carries_file_path_as_well_as_path() {
        let v = pre_tool_use(
            &base(),
            "edit_file",
            &json!({"path": "/ws/a.rs", "old_string": "x", "new_string": "y"}),
        );
        assert_eq!(v["tool_name"], "Edit");
        assert_eq!(v["tool_input"]["file_path"], "/ws/a.rs");
        assert_eq!(v["tool_input"]["path"], "/ws/a.rs");
        assert_eq!(v["tool_input"]["new_string"], "y");
    }

    #[test]
    fn a_tool_with_no_claude_code_name_keeps_its_own() {
        let v = pre_tool_use(&base(), "run_quality", &json!({}));
        assert_eq!(v["tool_name"], "run_quality");
        assert_eq!(v["claudinio_tool_name"], "run_quality");
    }

    #[test]
    fn the_remaining_events_pin_their_field_sets() {
        let b = base();
        assert_eq!(keys(&user_prompt_submit(&b, "hi")), with(&["prompt"]));
        assert_eq!(keys(&notification(&b, "m")), with(&["message"]));
        assert_eq!(keys(&stop(&b, true)), with(&["stop_hook_active"]));
        assert_eq!(
            keys(&subagent_stop(&b, false, "s-1:sub:0")),
            with(&["stop_hook_active", "claudinio_subagent_id"])
        );
        assert_eq!(
            keys(&pre_compact(&b, CompactTrigger::Manual, "")),
            with(&["trigger", "custom_instructions", "claudinio_trigger"])
        );
        assert_eq!(
            keys(&session_start(&b, SessionStartSource::Resume)),
            with(&["source"])
        );
        assert_eq!(
            keys(&session_end(&b, SessionEndReason::Logout)),
            with(&["reason"])
        );
    }

    #[test]
    fn a_handoff_reports_as_an_auto_compaction_and_says_so_natively() {
        let v = pre_compact(&base(), CompactTrigger::Handoff, "");
        assert_eq!(v["trigger"], "auto");
        assert_eq!(v["claudinio_trigger"], "handoff");
    }

    #[test]
    fn a_tool_response_is_capped_on_a_character_boundary() {
        let v = tool_response(true, "áéíóú", 3);
        assert_eq!(v["truncated"], true);
        assert_eq!(v["output"], "á");
    }
}
