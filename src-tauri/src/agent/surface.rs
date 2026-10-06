//! Which tools — and how much prompt — a model is offered.
//!
//! One catalog for every model was two mistakes at once.
//!
//! **Too much for a small window.** The Standard prompt and its tool schemas
//! measured about 33k tokens on the harness's own meter by October 2026, most
//! of it skills and MCP schemas. That is the whole of a 32k window: a local
//! model could not be sent a single request. The study this follows ("An
//! Empirical Study of Harness Design for Coding Agents", arXiv 2609.20804)
//! found small models are the ones that need structured tools and a task
//! scaffold — its 30B model gained 15 points on SWE-Bench from them — so the
//! Compact profile ([`COMPACT_PROMPT`], [`COMPACT_TOOLS`]) keeps exactly those
//! and drops everything else, including the detour through subagents:
//! delegating an edit costs a second prefix and a second context on a model
//! that has neither to spare.
//!
//! **Schemas nobody calls.** `go_to_definition`, `find_references` and
//! `symbol_lookup` were called 12 times in 16,945 tool calls across the
//! sessions measured, while their schemas and the prompt text steering toward
//! them rode along on every request. [`ToolSurface::Lean`] drops them, per
//! model, when the user asks for it.
//!
//! Lean goes no further than that on purpose. The same study has a strong
//! model doing better and 53% cheaper with bash alone — and another losing 23
//! points the same way. Which of the two a given model is has to be measured
//! on that model, so nothing here is a default.
//!
//! Whatever is chosen is chosen when the run starts and again only if the
//! model changes with the mode. Never per turn: the tool list is part of the
//! cached prefix.

use crate::agent::provider::AgentConfig;
use crate::agent::session::{PromptProfile, SessionMode};
use crate::agent::tools::ToolDef;

/// The tool catalog a model gets under the Standard profile.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Default)]
pub enum ToolSurface {
    #[default]
    Full,
    Lean,
}

impl ToolSurface {
    pub fn as_str(&self) -> &'static str {
        match self {
            ToolSurface::Full => "full",
            ToolSurface::Lean => "lean",
        }
    }

    pub fn parse(s: &str) -> Option<ToolSurface> {
        match s.trim() {
            "full" => Some(ToolSurface::Full),
            "lean" => Some(ToolSurface::Lean),
            _ => None,
        }
    }
}

/// What Lean leaves out.
pub const LEAN_DROPPED: [&str; 3] = ["go_to_definition", "find_references", "symbol_lookup"];

impl AgentConfig {
    /// The surface the user set for `model`; Full for any model they did not.
    pub fn tool_surface_for(&self, model: &str) -> ToolSurface {
        self.tool_surface
            .get(model)
            .and_then(|s| ToolSurface::parse(s))
            .unwrap_or_default()
    }
}

/// Record the surface the user picked for `model`. Full is the absence of an
/// entry, so a config only ever lists the models someone changed — and a value
/// this build cannot parse is dropped rather than stored.
pub fn set_for(config: &mut AgentConfig, model: &str, surface: &str) {
    match ToolSurface::parse(surface) {
        Some(ToolSurface::Lean) if !model.trim().is_empty() => {
            config
                .tool_surface
                .insert(model.to_string(), ToolSurface::Lean.as_str().into());
        }
        _ => {
            config.tool_surface.remove(model);
        }
    }
}

/// Remove from `defs` what `surface` does not offer — and from the tools that
/// stay, the lines of their descriptions that send the model to one that went.
pub fn retain_for(surface: ToolSurface, defs: &mut Vec<ToolDef>) {
    if surface == ToolSurface::Lean {
        defs.retain(|t| !LEAN_DROPPED.contains(&t.name.as_str()));
        for t in defs.iter_mut() {
            t.description = prompt_for(surface, std::mem::take(&mut t.description));
        }
    }
}

/// Text that steers toward a dropped tool — in a prompt or in another tool's
/// description — and what stands in for it. Anything that still named a tool
/// the model cannot call would cost a failed call every time the model
/// believed it, so the table is checked by test against every prompt and
/// every description: none may mention a dropped tool once rewritten.
const LEAN_REWRITES: [(&str, &str); 8] = [
    (
        "code_search/symbol_lookup for known names",
        "code_search for known names",
    ),
    (
        "Ranking: go_to_definition (precise) \u{2192} semantic_search",
        "Ranking: semantic_search",
    ),
    (
        "- Accuracy hierarchy: LSP > `semantic_search` (conceptual) > `code_search` (keyword) > `grep` (fallback).",
        "- Accuracy hierarchy: `semantic_search` (conceptual) > `code_search` (keyword) > `grep` (fallback).",
    ),
    (
        "* `code_search`/`symbol_lookup` only when you already know the exact symbol or keyword.",
        "* `code_search` only when you already know the exact symbol or keyword.",
    ),
    (
        "* For unfamiliar files, `file_outline` before `read_file`; use `go_to_definition`/`find_references` to trace relationships.",
        "* For unfamiliar files, `file_outline` before `read_file`.",
    ),
    (
        "`code_search`/`symbol_lookup` for known names, ",
        "`code_search` for known names, ",
    ),
    (
        "\u{2022} go_to_definition / find_references \u{2014} navigate symbol relationships precisely \
         \u{2022} symbol_lookup \u{2014} exact symbol name lookup across workspace ",
        "",
    ),
    (
        "Accuracy hierarchy: LSP tools (precise) \u{2192} semantic_search (conceptual) \u{2192} ",
        "Accuracy hierarchy: semantic_search (conceptual) \u{2192} ",
    ),
];

/// `prompt` as it should read under `surface`.
pub fn prompt_for(surface: ToolSurface, prompt: String) -> String {
    if surface == ToolSurface::Full {
        return prompt;
    }
    LEAN_REWRITES
        .iter()
        .fold(prompt, |text, (from, to)| text.replace(from, to))
}

/// Windows below this get the Compact profile. A 64k model still has room for
/// the Standard prompt once its lines are scaled (`agent::budget`); under that
/// the prompt is too large a share of what the model can hold.
pub const COMPACT_BELOW: u64 = 64_000;

/// The tools of the Compact profile: the structured file tools, a shell, the
/// task scaffold, the quality gate and a way to ask. No subagents — the session
/// edits files itself — no MCP, and nothing whose schema is paid for and
/// rarely used.
pub const COMPACT_TOOLS: [&str; 10] = [
    "read_file",
    "list_dir",
    "grep",
    "edit_file",
    "bash",
    "ask_user",
    "tasks_get",
    "tasks_set",
    "tasks_update",
    // The finish-line gate sends a failing run back with "call run_quality
    // again"; a profile without the tool would be told to call what it lacks.
    "run_quality",
];

/// What a Compact tool's description says about tools the profile leaves out.
/// `grep` calls itself a last resort behind the indexed searches — which here
/// do not exist, while the prompt names `grep` as the way to find things.
const COMPACT_REWRITES: [(&str, &str); 1] = [(
    " LAST RESORT for search \u{2014} prefer the indexed tools first: semantic_search for \
     behavior/concepts, code_search/symbol_lookup for known names. Reach for grep only when \
     those come up empty or you need a literal regex over raw file text.",
    "",
)];

/// The Compact catalog out of the full one: only [`COMPACT_TOOLS`], and no
/// description pointing at a tool that is not among them.
pub fn compact_defs(defs: Vec<ToolDef>) -> Vec<ToolDef> {
    defs.into_iter()
        .filter(|t| COMPACT_TOOLS.contains(&t.name.as_str()))
        .map(|mut t| {
            for (from, to) in COMPACT_REWRITES {
                t.description = t.description.replace(from, to);
            }
            t
        })
        .collect()
}

/// The whole system prompt of the Compact profile. Everything in it is there
/// because a small model does worse without it; anything else was left out
/// because on a 32k window every line here is a line of the user's code that
/// does not fit.
pub const COMPACT_PROMPT: &str = "\
Role: Claudinio, AI coding agent inside Claudinio Code. You work directly in the user's project: \
you read, search, edit files and run commands yourself.

# TASKS
- For anything beyond a one-step change, call `tasks_set` once to list the steps \
(id, title, description, journal: [], status: 'todo'). One logical step = one task.
- Move each task with `tasks_update`: 'doing' before you start it, 'done' once its work is verified. \
Never resend the whole list just to change a status.
- The user follows progress in the Task Panel. Never write the plan out as text.

# WORK
- Find things with `grep` and `list_dir`. Read with `read_file`, and on a large file pass \
start_line/end_line for the part you need: your context is small, so never read more than you will use.
- You MUST read a file before you change it. Change files with `edit_file`.
- Run builds and tests with `bash`, and fix what they report before you call a task done.
- When the work is finished, call `run_quality`: it runs the project's own checks and records the result.
- Finish the work, or block with `ask_user`. Never end with a plain-text question or a TODO.
- Unless the user told you to, call `ask_user` before external or destructive operations \
(push, branch, PR, deleting files).
- Your last message must stand on its own, with no \"see above\".

# HOOK OUTPUT
- `<hook-context>` is background the user's own setup added to this turn. Use it; do not quote it back.
- `<hook-feedback>` is a correction about what just happened. Act on it before continuing.

# LANGUAGE
- Reply in the language of the user's latest message.";

/// Should a run that asked for `profile` use Compact instead?
///
/// Only a Standard session, only in Builder, and only on a window that needs
/// it. Brain keeps the Standard profile whatever the window: its read-only
/// protocol — interview, design, tasks — is the thing Compact leaves out, and a
/// model too small for it is told so rather than handed something else under
/// the same name. Golden goals stay on Standard for the same reason: their
/// loop flips the session into Brain.
pub fn effective_profile(
    profile: PromptProfile,
    config: &AgentConfig,
    mode: SessionMode,
    has_golden_goals: bool,
) -> PromptProfile {
    let small = config.context_window_for(config.model_for_mode(mode.as_str())) < COMPACT_BELOW;
    if profile == PromptProfile::Standard
        && mode == SessionMode::Builder
        && small
        && !has_golden_goals
    {
        PromptProfile::Compact
    } else {
        profile
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::agent::provider::ProviderEntry;

    fn config_with_window(window: Option<u32>) -> AgentConfig {
        let mut config = AgentConfig {
            builder_model: "local-ish/m".into(),
            ..AgentConfig::default()
        };
        config.providers.insert(
            "local-ish".into(),
            ProviderEntry {
                api_key: String::new(),
                base_url: "http://localhost:1/v1".into(),
                protocol: "openai".into(),
                enabled_models: vec![],
                label: None,
                model_pricing: Default::default(),
                model_output_limits: Default::default(),
                custom: true,
                custom_models: vec!["m".into()],
                model_context_limits: Default::default(),
                context_window: window,
            },
        );
        config
    }

    #[test]
    fn compact_is_for_a_small_window_in_builder_and_nothing_else() {
        let small = config_with_window(Some(32_768));
        let std = PromptProfile::Standard;
        assert_eq!(
            effective_profile(std, &small, SessionMode::Builder, false),
            PromptProfile::Compact
        );
        // Brain runs on brain_model, which here is the 200k default — and Brain
        // keeps its own protocol whatever the window.
        assert_eq!(
            effective_profile(std, &small, SessionMode::Brain, false),
            std
        );
        // Golden goals flip the session into Brain; they stay on Standard.
        assert_eq!(
            effective_profile(std, &small, SessionMode::Builder, true),
            std
        );
        // A purpose-built profile is never swapped for another.
        assert_eq!(
            effective_profile(PromptProfile::GitSync, &small, SessionMode::Builder, false),
            PromptProfile::GitSync
        );
    }

    #[test]
    fn a_window_that_is_large_or_unknown_keeps_the_standard_profile() {
        let std = PromptProfile::Standard;
        for window in [None, Some(64_000), Some(128_000), Some(200_000)] {
            let cfg = config_with_window(window);
            assert_eq!(
                effective_profile(std, &cfg, SessionMode::Builder, false),
                std,
                "{window:?}"
            );
        }
        // The default models are 200k: nothing changes for them.
        let cfg = AgentConfig::default();
        assert_eq!(
            effective_profile(std, &cfg, SessionMode::Builder, false),
            std
        );
    }

    #[test]
    fn a_brain_on_a_small_model_keeps_standard_too() {
        let mut cfg = config_with_window(Some(32_768));
        cfg.brain_model = "local-ish/m".into();
        assert_eq!(
            effective_profile(PromptProfile::Standard, &cfg, SessionMode::Brain, false),
            PromptProfile::Standard
        );
    }

    #[test]
    fn the_surface_is_full_unless_the_user_set_lean_for_that_model() {
        let mut cfg = AgentConfig::default();
        assert_eq!(cfg.tool_surface_for("claudinio"), ToolSurface::Full);
        cfg.tool_surface.insert("claudinio".into(), "lean".into());
        assert_eq!(cfg.tool_surface_for("claudinio"), ToolSurface::Lean);
        assert_eq!(cfg.tool_surface_for("claudius"), ToolSurface::Full);
        // A value this build does not know is Full, not an error.
        cfg.tool_surface
            .insert("claudius".into(), "bash-only".into());
        assert_eq!(cfg.tool_surface_for("claudius"), ToolSurface::Full);
    }

    #[test]
    fn setting_a_surface_stores_only_what_differs_from_full() {
        let mut cfg = AgentConfig::default();
        set_for(&mut cfg, "claudinio", "lean");
        assert_eq!(cfg.tool_surface_for("claudinio"), ToolSurface::Lean);
        set_for(&mut cfg, "claudinio", "full");
        assert!(cfg.tool_surface.is_empty(), "full is no entry at all");
        set_for(&mut cfg, "claudinio", "lean");
        set_for(&mut cfg, "claudinio", "bash-only");
        assert!(cfg.tool_surface.is_empty(), "an unknown value is not kept");
        set_for(&mut cfg, " ", "lean");
        assert!(cfg.tool_surface.is_empty(), "nor is a model with no name");
    }

    #[test]
    fn lean_drops_exactly_the_three_tools_and_full_drops_none() {
        let all = crate::agent::tools::get_defs(4);
        let mut full = all.clone();
        retain_for(ToolSurface::Full, &mut full);
        assert_eq!(full.len(), all.len());

        let mut lean = all.clone();
        retain_for(ToolSurface::Lean, &mut lean);
        assert_eq!(lean.len(), all.len() - LEAN_DROPPED.len());
        for name in LEAN_DROPPED {
            assert!(all.iter().any(|t| t.name == name), "{name} is a real tool");
            assert!(!lean.iter().any(|t| t.name == name), "{name} is dropped");
        }
        // The search tools the model does use stay.
        for name in ["semantic_search", "code_search", "file_outline", "grep"] {
            assert!(lean.iter().any(|t| t.name == name), "{name} stays");
        }
    }

    // `grep` and `semantic_search` both rank themselves against the LSP tools
    // in their own descriptions.
    #[test]
    fn no_tool_that_stays_still_points_at_one_that_went() {
        let all = crate::agent::tools::get_defs(4);
        let pointing = |defs: &[ToolDef]| -> Vec<String> {
            defs.iter()
                .filter(|t| !LEAN_DROPPED.contains(&t.name.as_str()))
                .filter(|t| {
                    let schema = t.input_schema.to_string();
                    LEAN_DROPPED
                        .iter()
                        .any(|d| t.description.contains(d) || schema.contains(d))
                })
                .map(|t| t.name.clone())
                .collect()
        };
        assert!(
            !pointing(&all).is_empty(),
            "the full catalog cross-references"
        );
        let mut lean = all.clone();
        retain_for(ToolSurface::Lean, &mut lean);
        assert_eq!(pointing(&lean), Vec::<String>::new());

        // Full leaves every description as it was.
        let mut full = all.clone();
        retain_for(ToolSurface::Full, &mut full);
        for (a, b) in all.iter().zip(&full) {
            assert_eq!(a.description, b.description);
        }
    }

    #[test]
    fn every_compact_tool_exists_and_the_prompt_names_only_those() {
        let all = crate::agent::tools::get_defs(4);
        for name in COMPACT_TOOLS {
            assert!(all.iter().any(|t| t.name == name), "{name} is a real tool");
        }
        // The prompt must not send a small model after a tool it was not given.
        for t in &all {
            if !COMPACT_TOOLS.contains(&t.name.as_str()) {
                assert!(
                    !COMPACT_PROMPT.contains(&format!("`{}`", t.name)),
                    "the Compact prompt names {}, which the profile does not offer",
                    t.name
                );
            }
        }
        for name in [
            "tasks_set",
            "tasks_update",
            "edit_file",
            "read_file",
            "ask_user",
        ] {
            assert!(COMPACT_PROMPT.contains(&format!("`{name}`")), "{name}");
        }
    }

    // The description of a tool is prompt too. One that sends a small model
    // after `semantic_search` costs it a failed call out of a window with none
    // to spare.
    #[test]
    fn no_compact_tool_points_at_a_tool_the_profile_leaves_out() {
        let all = crate::agent::tools::get_defs(4);
        let compact = compact_defs(all.clone());
        assert_eq!(compact.len(), COMPACT_TOOLS.len());
        let mut left_out: Vec<String> = all
            .iter()
            .map(|t| t.name.clone())
            .filter(|n| !COMPACT_TOOLS.contains(&n.as_str()))
            .collect();
        for name in [
            "write_plan",
            "finalize_plan",
            "enter_plan_mode",
            "exit_plan_mode",
        ] {
            left_out.push(name.into());
        }
        for tool in &compact {
            for name in &left_out {
                assert!(
                    !tool.description.contains(name.as_str()),
                    "{} points at {name}",
                    tool.name
                );
            }
        }
        // `grep` is still described as what it is.
        let grep = compact.iter().find(|t| t.name == "grep").unwrap();
        assert!(grep.description.starts_with("Search for a regex pattern"));
        assert!(!grep.description.contains("LAST RESORT"));
    }

    #[test]
    fn full_leaves_a_prompt_byte_identical() {
        let text = "- Accuracy hierarchy: LSP > `semantic_search` (conceptual)".to_string();
        assert_eq!(prompt_for(ToolSurface::Full, text.clone()), text);
    }
}
