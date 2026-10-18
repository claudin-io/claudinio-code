//! Deferred tools: definitions the model sees by name until it needs them.
//!
//! The Standard prefix carried every tool schema on every request — about
//! 20k characters built in, and without bound once MCP servers joined
//! (Anthropic measured a five-server setup at ~55k tokens of definitions).
//! Most of it is rarely called: the three LSP tools 12 times in 16,945 calls
//! across the sessions measured for the lean-harness plan. Anthropic's tool
//! search took a 55k-token tool prefix to ~8.7k (−85%) and *raised* accuracy
//! on their MCP eval (49% → 74%), because fewer irrelevant schemas is also
//! less distraction.
//!
//! So a Standard main session sends a core set, lists the rest in the system
//! prompt as one line each ([`catalog_section`]), and materializes them with
//! `load_tools`. A loaded definition is *appended* to the end of the tool list
//! and stays for the session — never swapped in and out, which would rewrite
//! the cached prefix each time (Manus's rule) — and the load is persisted as a
//! `tools_loaded` record so a later run of the same session, or a resumed one,
//! rebuilds the identical array in the identical order. A call to a deferred
//! tool the model did not load is not refused: it is loaded on the spot and
//! run. Each round that loads anything is one `cache_break` (reason
//! `tools_loaded`) — the prefix is rewritten once at the top of the next
//! round however many tools joined — so what the deferral costs in cache
//! writes is countable against its budget (decision D2: one write per load is
//! acceptable).
//!
//! A direct call loads its tool when the call is dispatched, before the
//! approval prompt and the mode gates: a call the user then rejects still
//! leaves the definition in the list. That is deliberate. The model has shown
//! it wants the tool, a rejected call is usually retried in another shape,
//! and telling "ran" from "refused" would mean reading the result's text.
//! Only a PreToolUse hook's `deny` stops the load; its `ask` is not consulted
//! for `load_tools`, which only changes what the model is offered.
//!
//! The catalog never depends on what is loaded: the system prompt stays byte
//! identical for the run, and only the tool list grows.

use crate::agent::events::AgentEvent;
use crate::agent::persist::{CacheBreakReason, SessionRecord, SessionStore, now_ms};
use crate::agent::provider::ToolDescription;
use crate::agent::tools::{ToolContext, ToolDef};
use serde_json::Value;
use tauri::ipc::Channel;

/// The built-in tools a Standard main session does not send until they are
/// loaded, each with the line the catalog shows for it. Everything else
/// built in is core: `read_file`, `list_dir`, `grep`, `bash`, `ask_user`,
/// `tasks_get`, `tasks_set`, `tasks_update`, `run_quality`, `spawn_agents`,
/// `skills`, `load_tools`, and the mode's own (`finalize_plan` in Builder,
/// `write_plan`, `exit_plan_mode` in Brain). Every MCP tool is deferred too. The order here is the catalog's
/// order.
///
/// `tasks_get` is core although the brief listed it here: the Builder prompt
/// and every handoff kickoff (`agent::transition`) make it the session's
/// first call — the Builder has no other way to read the task descriptions
/// Brain wrote — so deferring it bought ~70 tokens of prefix for a
/// `tools_loaded` cache break in nearly every Builder session.
///
/// `finalize_plan` is core in Builder (the only mode that has it) for the
/// same reason at the other end: the Builder prompt makes it the last step,
/// the plan kickoff ends on it and the loop's last nudge
/// (`nudges::FINALIZE_PLAN`) orders it. A load at the end of a session
/// rewrites the cached prefix in front of the whole conversation — the most
/// expensive cache write a session can make. The rule is pinned by
/// `prompt::tests::no_harness_text_orders_a_deferred_tool` and
/// `no_standard_prompt_line_makes_a_deferred_tool_mandatory_or_first`: no
/// text the harness or the prompt puts in front of the model orders a
/// deferred tool.
pub const DEFERRED_BUILTIN: &[(&str, &str)] = &[
    (
        "semantic_search",
        "find code and docs by meaning or by a rare term (English queries); the first call for \"how does X work\" or \"where is Y\"",
    ),
    (
        "code_search",
        "find definitions by symbol name or part of one in the code index",
    ),
    ("symbol_lookup", "find a symbol by its exact name"),
    (
        "file_outline",
        "list the symbols a file defines without reading it",
    ),
    (
        "go_to_definition",
        "where the symbol at a file position is defined (language server)",
    ),
    (
        "find_references",
        "every use of the symbol at a file position (language server)",
    ),
    (
        "web_search",
        "search the web for current information that is not in the code or your training",
    ),
    (
        "browser",
        "open a URL in a dedicated browser and operate the page (click, type, wait)",
    ),
    (
        "browser_inspect",
        "read the open page's console, network requests, text or HTML",
    ),
    (
        "browser_screenshot",
        "see the open page, or part of it, as an image",
    ),
    (
        "enter_plan_mode",
        "switch to Brain (planning) mode when a task needs a design before code",
    ),
];

/// How many MCP tools the catalog names. A server can expose hundreds; past
/// this the catalog says how many more there are and from which servers, and
/// `load_tools` still loads any of them.
pub const MCP_CATALOG_LINES: usize = 60;

/// How much of an MCP tool's own description its catalog line keeps.
pub const MCP_DESCRIPTION_CHARS: usize = 100;

/// Whether `name` is sent only once loaded.
pub fn is_deferred(name: &str) -> bool {
    name.starts_with("mcp__") || DEFERRED_BUILTIN.iter().any(|(n, _)| *n == name)
}

pub fn load_tools_def() -> ToolDef {
    ToolDef {
        name: "load_tools".into(),
        description: "Add deferred tools (the DEFERRED TOOLS list in the system prompt) to your \
                      tool list; they stay for the rest of the session. Load only what the task \
                      needs. A name ending in * loads every deferred tool that starts with it, \
                      e.g. \"mcp__github__*\"."
            .into(),
        input_schema: serde_json::json!({
            "type": "object",
            "properties": {
                "names": {
                    "type": "array",
                    "minItems": 1,
                    "items": { "type": "string" },
                    "description": "Tool names exactly as the list shows them"
                }
            },
            "required": ["names"]
        }),
    }
}

impl From<ToolDef> for ToolDescription {
    fn from(t: ToolDef) -> Self {
        ToolDescription {
            name: t.name,
            description: t.description,
            input_schema: t.input_schema,
        }
    }
}

/// `tools` split into what is sent (core, in order) and what is deferred
/// (in order).
pub fn split(tools: Vec<ToolDescription>) -> (Vec<ToolDescription>, Vec<ToolDescription>) {
    tools.into_iter().partition(|t| !is_deferred(&t.name))
}

/// The system prompt section naming the deferred tools, or `None` when there
/// are none. One line per tool: built-ins with the line [`DEFERRED_BUILTIN`]
/// gives them, MCP tools with their own description cut to
/// [`MCP_DESCRIPTION_CHARS`], at most [`MCP_CATALOG_LINES`] of them.
pub fn catalog_section(deferred: &[ToolDescription]) -> Option<String> {
    if deferred.is_empty() {
        return None;
    }
    let mut text = String::from(
        "\n\n## DEFERRED TOOLS\n\
         These tools exist but are not in your tool list yet. `load_tools` {\"names\": [...]} \
         adds the ones a task needs for the rest of the session; calling one directly loads it \
         too.\n",
    );
    for (name, purpose) in DEFERRED_BUILTIN {
        if deferred.iter().any(|t| t.name == *name) {
            text.push_str(&format!("- {name} \u{2014} {purpose}\n"));
        }
    }
    let mcp: Vec<&ToolDescription> = deferred
        .iter()
        .filter(|t| t.name.starts_with("mcp__"))
        .collect();
    for t in mcp.iter().take(MCP_CATALOG_LINES) {
        text.push_str(&format!(
            "- {} \u{2014} {}\n",
            t.name,
            crate::agent::skills::one_line(&t.description, MCP_DESCRIPTION_CHARS)
        ));
    }
    if mcp.len() > MCP_CATALOG_LINES {
        let mut servers: Vec<&str> = mcp[MCP_CATALOG_LINES..]
            .iter()
            .filter_map(|t| t.name.strip_prefix("mcp__")?.split("__").next())
            .collect();
        servers.dedup();
        text.push_str(&format!(
            "({} more MCP tools from {}: load them by name, or all of a server's with \"mcp__<server>__*\".)\n",
            mcp.len() - MCP_CATALOG_LINES,
            servers.join(", ")
        ));
    }
    Some(text)
}

/// The tool list a request carries: `core`, then each loaded tool that is
/// deferred here, in the order it was loaded. A loaded name this prefix does
/// not defer (a mode without it, a feature switched off since) is skipped.
pub fn materialize(
    mut core: Vec<ToolDescription>,
    deferred: &[ToolDescription],
    loaded: &[String],
) -> Vec<ToolDescription> {
    for name in loaded {
        if let Some(t) = deferred.iter().find(|t| &t.name == name)
            && !core.iter().any(|c| &c.name == name)
        {
            core.push(t.clone());
        }
    }
    core
}

/// Every tool the session loaded, in load order, from its records.
pub fn loaded_names(records: &[SessionRecord]) -> Vec<String> {
    let mut out: Vec<String> = Vec::new();
    for r in records {
        if let SessionRecord::ToolsLoaded { names, .. } = r {
            for n in names {
                if !out.contains(n) {
                    out.push(n.clone());
                }
            }
        }
    }
    out
}

/// What one `load_tools` call does.
#[derive(Debug, Default, PartialEq)]
pub struct Load {
    /// Newly loaded, in the order they join the list.
    pub added: Vec<String>,
    pub already: Vec<String>,
    /// Requested names (or patterns) that match no deferred tool here.
    pub unknown: Vec<String>,
}

/// Resolve `requested` against the tools deferred here and those already
/// loaded. A name ending in `*` matches every deferred tool starting with the
/// rest, in catalog order.
pub fn resolve(requested: &[String], deferred: &[ToolDescription], loaded: &[String]) -> Load {
    let mut out = Load::default();
    for req in requested {
        let req = req.trim();
        let matched: Vec<&str> = match req.strip_suffix('*') {
            Some(prefix) => deferred
                .iter()
                .filter(|t| t.name.starts_with(prefix))
                .map(|t| t.name.as_str())
                .collect(),
            None => deferred
                .iter()
                .filter(|t| t.name == req)
                .map(|t| t.name.as_str())
                .collect(),
        };
        if matched.is_empty() {
            out.unknown.push(req.to_string());
        }
        for name in matched {
            let name = name.to_string();
            if loaded.contains(&name) || out.already.contains(&name) {
                if !out.already.contains(&name) {
                    out.already.push(name);
                }
            } else if !out.added.contains(&name) {
                out.added.push(name);
            }
        }
    }
    out
}

impl Load {
    /// The tool result: what joined, what was there, what does not exist.
    /// An error only when nothing requested could be loaded.
    pub fn answer(&self, deferred: &[ToolDescription]) -> Result<String, String> {
        let mut parts = Vec::new();
        if !self.added.is_empty() {
            parts.push(format!(
                "Loaded: {}. They are in your tool list from your next call on.",
                self.added.join(", ")
            ));
        }
        if !self.already.is_empty() {
            parts.push(format!("Already loaded: {}.", self.already.join(", ")));
        }
        if !self.unknown.is_empty() {
            let names: Vec<&str> = deferred.iter().map(|t| t.name.as_str()).collect();
            parts.push(format!(
                "Not a deferred tool here: {}. Deferred tools: {}.",
                self.unknown.join(", "),
                if names.is_empty() {
                    "none".to_string()
                } else {
                    names.join(", ")
                }
            ));
        }
        let text = parts.join(" ");
        if self.added.is_empty() && self.already.is_empty() {
            Err(text)
        } else {
            Ok(text)
        }
    }
}

/// Persist what one round loaded: the names, in load order, and the one
/// cache break they cause together. The main loop calls this once per round,
/// after every call in it — the prefix is rebuilt once at the top of the next
/// round however many tools joined, and the plan's gate counts these breaks.
pub fn record(store: &SessionStore, ctx: &ToolContext, names: &[String]) {
    if names.is_empty() {
        return;
    }
    store.try_append(&SessionRecord::ToolsLoaded {
        names: names.to_vec(),
        ts: now_ms(),
    });
    store.try_append(&SessionRecord::cache_break(CacheBreakReason::ToolsLoaded));
    crate::agent::persist::invalidate_cache(&store.path, &ctx.records_cache);
}

/// The main loop's step for one tool call, before it is dispatched.
///
/// `load_tools` is answered here (`Some(result)`). A call to a deferred tool
/// that is not loaded yet loads it and returns `None`, so the call runs as any
/// other. Either way the names join `loaded`; persisting them is the caller's,
/// once per round ([`record`]). `deferred` is asked for only when one of the
/// two applies.
pub(crate) fn before_call(
    tool_name: &str,
    tool_use_id: &str,
    tool_input: &Value,
    deferred: impl FnOnce() -> Vec<ToolDescription>,
    loaded: &mut Vec<String>,
    event_tx: &Channel<AgentEvent>,
    session_id: &str,
) -> Option<crate::agent::provider::ContentBlock> {
    if tool_name != "load_tools" {
        if is_deferred(tool_name) && !loaded.iter().any(|n| n == tool_name) {
            let deferred = deferred();
            if deferred.iter().any(|t| t.name == tool_name) {
                loaded.push(tool_name.to_string());
            }
        }
        return None;
    }
    let _ = event_tx.send(AgentEvent::ToolCall {
        session_id: session_id.to_string(),
        tool_id: tool_use_id.to_string(),
        tool_name: tool_name.to_string(),
        args: tool_input.clone(),
        permission: "auto".into(),
        edit_proposal: None,
    });
    let requested: Vec<String> = tool_input
        .get("names")
        .and_then(|v| v.as_array())
        .map(|a| {
            a.iter()
                .filter_map(|v| v.as_str().map(str::to_string))
                .collect()
        })
        .unwrap_or_default();
    let deferred = deferred();
    let result = if requested.is_empty() {
        Err("load_tools needs the names to load: {\"names\": [\"code_search\"]}".to_string())
    } else {
        let load = resolve(&requested, &deferred, loaded);
        loaded.extend(load.added.iter().cloned());
        load.answer(&deferred)
    };
    let (text, error) = match result {
        Ok(text) => (text, None),
        Err(e) => (format!("Error: {e}"), Some(e)),
    };
    let _ = event_tx.send(AgentEvent::ToolResult {
        tool_id: tool_use_id.to_string(),
        tool_name: tool_name.to_string(),
        output: if error.is_some() {
            String::new()
        } else {
            text.clone()
        },
        error,
    });
    Some(crate::agent::provider::ContentBlock::tool_result(
        tool_use_id,
        text,
    ))
}

#[cfg(test)]
mod tests {
    use super::*;

    fn tool(name: &str, description: &str) -> ToolDescription {
        ToolDescription {
            name: name.into(),
            description: description.into(),
            input_schema: serde_json::json!({"type": "object"}),
        }
    }

    fn names(tools: &[ToolDescription]) -> Vec<&str> {
        tools.iter().map(|t| t.name.as_str()).collect()
    }

    #[test]
    fn every_deferred_builtin_is_a_real_tool_and_mcp_is_always_deferred() {
        let mut all: Vec<String> = crate::agent::tools::get_defs(4)
            .into_iter()
            .map(|t| t.name)
            .collect();
        all.push("finalize_plan".into());
        all.push("enter_plan_mode".into());
        for (name, purpose) in DEFERRED_BUILTIN {
            assert!(all.contains(&name.to_string()), "{name} is not a tool");
            assert!(!purpose.is_empty() && purpose.len() <= 160, "{name}");
        }
        assert!(is_deferred("mcp__github__search"));
        for core in [
            "read_file",
            "list_dir",
            "grep",
            "bash",
            "ask_user",
            "tasks_set",
            "tasks_update",
            "run_quality",
            "spawn_agents",
            "skills",
            "load_tools",
            "tasks_get",
            "edit_file",
            "write_plan",
            "exit_plan_mode",
            "finalize_plan",
        ] {
            assert!(!is_deferred(core), "{core} is core");
        }
    }

    #[test]
    fn split_keeps_the_order_of_both_halves() {
        let (core, deferred) = split(vec![
            tool("read_file", ""),
            tool("code_search", ""),
            tool("bash", ""),
            tool("mcp__a__x", ""),
            tool("enter_plan_mode", ""),
            tool("finalize_plan", ""),
        ]);
        assert_eq!(names(&core), ["read_file", "bash", "finalize_plan"]);
        assert_eq!(
            names(&deferred),
            ["code_search", "mcp__a__x", "enter_plan_mode"]
        );
    }

    #[test]
    fn the_catalog_is_one_line_per_deferred_tool_in_a_fixed_order() {
        let deferred = vec![
            tool("enter_plan_mode", "x"),
            tool("mcp__b__two", "Second server tool"),
            tool("code_search", "x"),
            tool("mcp__a__one", &format!("First line\n{}", "y".repeat(300))),
        ];
        let text = catalog_section(&deferred).unwrap();
        let lines: Vec<&str> = text.lines().filter(|l| l.starts_with("- ")).collect();
        assert_eq!(lines.len(), 4, "{text}");
        // Built-ins in the table's order, then MCP tools in the order they came
        // (server then tool name, as `mcp::snapshots` stores them).
        assert!(lines[0].starts_with("- code_search \u{2014} find definitions"));
        assert!(lines[1].starts_with("- enter_plan_mode \u{2014} "));
        assert!(lines[2].starts_with("- mcp__b__two \u{2014} Second server tool"));
        let mcp_desc = lines[3].strip_prefix("- mcp__a__one \u{2014} ").unwrap();
        assert!(mcp_desc.starts_with("First line yyy"), "{mcp_desc}");
        assert!(mcp_desc.chars().count() <= MCP_DESCRIPTION_CHARS + 1);
        assert!(text.contains("`load_tools`"));
        assert!(catalog_section(&[]).is_none());
    }

    #[test]
    fn the_mcp_catalog_stops_at_sixty_lines_and_names_the_servers_of_the_rest() {
        let deferred: Vec<ToolDescription> = (0..70)
            .map(|i| {
                let server = if i < 65 { "big" } else { "tail" };
                tool(&format!("mcp__{server}__t{i:02}"), "d")
            })
            .collect();
        let text = catalog_section(&deferred).unwrap();
        assert_eq!(text.lines().filter(|l| l.starts_with("- ")).count(), 60);
        let last = text.trim_end().lines().last().unwrap();
        assert!(last.contains("10 more MCP tools from big, tail"), "{last}");
        // Every one of them can still be loaded, by name or by server.
        let load = resolve(&["mcp__tail__*".into()], &deferred, &[]);
        assert_eq!(load.added.len(), 5);
        let load = resolve(&["mcp__big__t64".into()], &deferred, &[]);
        assert_eq!(load.added, ["mcp__big__t64"]);
    }

    #[test]
    fn loaded_tools_are_appended_after_the_core_in_load_order() {
        let core = vec![tool("read_file", ""), tool("bash", "")];
        let deferred = vec![
            tool("code_search", ""),
            tool("semantic_search", ""),
            tool("mcp__a__x", ""),
        ];
        let loaded: Vec<String> = ["mcp__a__x", "gone", "code_search", "mcp__a__x"]
            .map(String::from)
            .to_vec();
        let tools = materialize(core.clone(), &deferred, &loaded);
        assert_eq!(
            names(&tools),
            ["read_file", "bash", "mcp__a__x", "code_search"]
        );
        assert_eq!(materialize(core, &deferred, &[]).len(), 2);
    }

    #[test]
    fn the_loaded_names_come_back_from_the_records_in_order() {
        let records = vec![
            SessionRecord::ToolsLoaded {
                names: vec!["semantic_search".into()],
                ts: 1,
            },
            SessionRecord::cache_break(CacheBreakReason::ToolsLoaded),
            SessionRecord::ToolsLoaded {
                names: vec!["mcp__a__x".into(), "semantic_search".into()],
                ts: 2,
            },
        ];
        assert_eq!(loaded_names(&records), ["semantic_search", "mcp__a__x"]);
        assert!(loaded_names(&[]).is_empty());
    }

    #[test]
    fn a_load_reports_what_joined_what_was_there_and_what_does_not_exist() {
        let deferred = vec![tool("code_search", ""), tool("enter_plan_mode", "")];
        let load = resolve(
            &[
                "code_search".into(),
                "enter_plan_mode".into(),
                "nope".into(),
            ],
            &deferred,
            &["enter_plan_mode".into()],
        );
        assert_eq!(load.added, ["code_search"]);
        assert_eq!(load.already, ["enter_plan_mode"]);
        assert_eq!(load.unknown, ["nope"]);
        let text = load.answer(&deferred).unwrap();
        assert!(text.contains("Loaded: code_search") && text.contains("nope"));
        let err = resolve(&["nope".into()], &deferred, &[])
            .answer(&deferred)
            .unwrap_err();
        assert!(err.contains("code_search, enter_plan_mode"), "{err}");
    }

    /// Hooks select by the name of the tool that was called, through the
    /// alias table — never through the tool list — so a matcher written
    /// against Claude Code's names still selects a tool that is deferred,
    /// loaded or not.
    #[test]
    fn a_hook_matcher_selects_deferred_tools_through_the_alias_table() {
        let deferred: Vec<&str> = DEFERRED_BUILTIN.iter().map(|(n, _)| *n).collect();
        assert_eq!(
            crate::agent::hooks::matcher::hits(Some("WebSearch|symbol_lookup"), &deferred),
            ["symbol_lookup", "web_search"]
        );
        assert_eq!(
            crate::agent::hooks::matcher::hits(Some("mcp__gh__.*"), &["mcp__gh__issue"]),
            ["mcp__gh__issue"]
        );
    }

    #[test]
    fn the_load_tools_description_is_short() {
        assert!(load_tools_def().description.len() <= 400);
    }
}
