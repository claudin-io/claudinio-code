//! Deciding whether a hook applies.
//!
//! A matcher is a string somebody wrote against *Claude Code's* tool names —
//! `Bash`, `Edit|Write`, `Task` — and the tools it has to select here are called
//! `bash`, `edit_file` and `spawn_agents`. Translating between the two is the
//! whole job of this file, and it is why a config written for another harness
//! selects anything at all instead of silently selecting nothing.
//!
//! The failure mode being designed against is not a crash. It is a hook that
//! installs cleanly, runs on every prompt and matches nothing, which looks
//! exactly like a hook that is working and has nothing to say.

use regex::Regex;

/// Native tool name to the Claude Code names that should select it.
///
/// The first alias is canonical: it is what goes on the wire as `tool_name`, so
/// a hook that does `case "$tool_name" in Edit)` sees what it expects. The
/// native name always matches too, so a matcher written against Claudinio's own
/// vocabulary is never second-class.
///
/// Tools with no Claude Code counterpart are absent on purpose — `browser`,
/// `run_quality`, `code_search`, `semantic_search`, `symbol_lookup`,
/// `file_outline`, `go_to_definition`, `find_references`, `write_plan`,
/// `finalize_plan`, `enter_plan_mode`. Inventing an alias for them would be
/// inventing a name no config in the world is written against.
const TOOL_ALIASES: &[(&str, &[&str])] = &[
    ("bash", &["Bash"]),
    ("read_file", &["Read"]),
    // One native tool answers four Claude Code names. `Write` and `MultiEdit`
    // map here because `edit_file` is what performs those effects; a `Write`
    // guard that did not fire on `edit_file` would be a guard that does nothing.
    ("edit_file", &["Edit", "Write", "MultiEdit", "NotebookEdit"]),
    ("list_dir", &["Glob", "LS"]),
    ("grep", &["Grep"]),
    ("spawn_agents", &["Task"]),
    ("web_search", &["WebSearch"]),
    ("ask_user", &["AskUserQuestion"]),
    ("tasks_set", &["TodoWrite"]),
    ("tasks_get", &["TodoRead"]),
    ("exit_plan_mode", &["ExitPlanMode"]),
];

/// Every name a matcher may use to select this tool, native first.
pub fn names_for(native: &str) -> Vec<String> {
    let mut out = vec![native.to_string()];
    if let Some((_, aliases)) = TOOL_ALIASES.iter().find(|(n, _)| *n == native) {
        out.extend(aliases.iter().map(|s| s.to_string()));
    }
    out
}

/// What this tool calls itself on the wire. Claude Code's name when it has one,
/// because that is the string published hooks compare against.
pub fn canonical_alias(native: &str) -> &str {
    TOOL_ALIASES
        .iter()
        .find(|(n, _)| *n == native)
        .and_then(|(_, a)| a.first().copied())
        .unwrap_or(native)
}

/// Every native tool a matcher selects, out of the names it could see.
pub fn hits(matcher: Option<&str>, candidates: &[&str]) -> Vec<String> {
    candidates
        .iter()
        .filter(|c| matches(matcher, c))
        .map(|c| c.to_string())
        .collect()
}

/// Every literal trigger a matcher selects, out of the ones the event defines.
pub fn hits_literal(matcher: Option<&str>, candidates: &[&str]) -> Vec<String> {
    candidates
        .iter()
        .filter(|c| matches_literal(matcher, c))
        .map(|c| c.to_string())
        .collect()
}

/// Does `matcher` select `native_tool`?
///
/// Absent, empty and `*` all mean everything — Claude Code accepts all three and
/// configs use all three. Otherwise the matcher is tried as an exact string and
/// then as an unanchored regex, against the native name and every alias.
///
/// Unanchored is deliberate rather than sloppy: `"Edit"` selecting
/// `"NotebookEdit"` is Claude Code's own behaviour, and configs are written
/// expecting it. Case-sensitive for the same reason.
///
/// It has a sharp edge, which is inherited rather than introduced: `"Write"`
/// also selects `"TodoWrite"`, so an edit guard also guards the task list. That
/// is exactly what the same matcher does in Claude Code, against the same two
/// tool names, and a config that behaved differently here would be worse than
/// one that behaves surprisingly in both. Anchor the matcher (`^Write$`) to opt
/// out.
pub fn matches(matcher: Option<&str>, native_tool: &str) -> bool {
    let Some(m) = matcher.map(str::trim).filter(|m| !m.is_empty()) else {
        return true;
    };
    if m == "*" {
        return true;
    }
    let names = names_for(native_tool);
    if names.iter().any(|n| n == m) {
        return true;
    }
    // An invalid regex degrades to the exact comparison above rather than
    // matching everything. Matching everything on a typo would hand a guard
    // hook veto power over tools nobody meant to guard.
    match Regex::new(m) {
        Ok(re) => names.iter().any(|n| re.is_match(n)),
        Err(_) => false,
    }
}

/// Is this matcher a valid regex? A matcher that is not still works as an exact
/// comparison; the panel says so rather than leaving the user to wonder.
pub fn matcher_is_valid_regex(matcher: &str) -> bool {
    Regex::new(matcher).is_ok()
}

/// PreCompact (`manual`/`auto`) and SessionStart (`startup`/`resume`/`clear`/
/// `compact`) match on the event's own trigger word, with no tool vocabulary in
/// play, so aliasing must not apply.
pub fn matches_literal(matcher: Option<&str>, value: &str) -> bool {
    let Some(m) = matcher.map(str::trim).filter(|m| !m.is_empty()) else {
        return true;
    };
    if m == "*" || m == value {
        return true;
    }
    Regex::new(m).map(|re| re.is_match(value)).unwrap_or(false)
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn edit_write_matcher_matches_the_native_edit_tool() {
        assert!(matches(Some("Edit|Write"), "edit_file"));
        assert!(matches(Some("Write"), "edit_file"));
        assert!(!matches(Some("Edit|Write"), "bash"));
    }

    #[test]
    fn an_unanchored_matcher_reaches_substrings_as_it_does_in_claude_code() {
        // `Write` selecting `TodoWrite` is Claude Code's behaviour against the
        // same two names. Pinned so nobody "fixes" it into a difference.
        assert!(matches(Some("Write"), "tasks_set"));
        assert!(matches(Some("Edit"), "edit_file"));
        // And anchoring is how a config opts out of the substring reach —
        // without losing the tool it actually meant.
        assert!(!matches(Some("^Write$"), "tasks_set"));
        assert!(matches(Some("^Write$"), "edit_file"));
        assert!(matches(Some("^Edit$"), "edit_file"));
    }

    #[test]
    fn a_native_name_matches_itself() {
        assert!(matches(Some("edit_file"), "edit_file"));
        assert!(matches(Some("bash"), "bash"));
        assert!(matches(Some("browser"), "browser"));
    }

    #[test]
    fn star_and_empty_and_absent_match_everything() {
        for m in [None, Some(""), Some("  "), Some("*")] {
            assert!(matches(m, "bash"));
            assert!(matches(m, "run_quality"));
        }
    }

    #[test]
    fn matching_is_case_sensitive() {
        assert!(!matches(Some("bash"), "Bash"));
        assert!(!matches(Some("EDIT"), "edit_file"));
        assert!(matches(Some("Bash"), "bash"));
    }

    #[test]
    fn an_invalid_regex_falls_back_to_exact_equality() {
        assert!(!matcher_is_valid_regex("Edit("));
        assert!(!matches(Some("Edit("), "edit_file"));
        assert!(matches(Some("edit_file"), "edit_file"));
    }

    #[test]
    fn an_mcp_matcher_needs_no_alias() {
        assert!(matches(Some("mcp__.*"), "mcp__serena__find_symbol"));
        assert!(matches(Some("mcp__serena__.*"), "mcp__serena__find_symbol"));
        assert!(!matches(Some("mcp__other__.*"), "mcp__serena__x"));
    }

    #[test]
    fn task_selects_spawn_agents_and_the_wire_name_is_task() {
        assert!(matches(Some("Task"), "spawn_agents"));
        assert_eq!(canonical_alias("spawn_agents"), "Task");
        assert_eq!(canonical_alias("browser"), "browser");
    }

    #[test]
    fn hits_answers_what_a_matcher_will_select() {
        assert_eq!(
            hits(Some("Edit|Bash"), &["edit_file", "bash", "grep"]),
            vec!["edit_file", "bash"]
        );
    }

    #[test]
    fn the_alias_table_has_no_duplicate_alias_and_no_duplicate_native() {
        let mut seen = std::collections::HashSet::new();
        for (native, aliases) in TOOL_ALIASES {
            assert!(seen.insert(native.to_string()), "duplicate native {native}");
            for a in *aliases {
                assert!(seen.insert(a.to_string()), "duplicate alias {a}");
            }
        }
    }

    #[test]
    fn precompact_and_session_start_matchers_select_the_trigger() {
        assert!(matches_literal(Some("manual"), "manual"));
        assert!(!matches_literal(Some("manual"), "auto"));
        assert!(matches_literal(Some("startup|resume"), "resume"));
        assert!(matches_literal(None, "compact"));
        // No aliasing: `Edit` must not select the source `startup`.
        assert!(!matches_literal(Some("Edit"), "startup"));
    }
}
