//! What a run sends ahead of the conversation: the system prompt for each mode
//! and profile, and the tool list beside it.
//!
//! Both are derived together (`run_prefix`) and never change between rounds of
//! the same mode, because they are the cached prefix: anything per-round here
//! (a timestamp, a map's iteration order) would turn every request cold.

use crate::agent::provider::{AgentConfig, ToolDescription};
use crate::agent::session::{PromptProfile, SessionMode};
use crate::agent::subagent;
use crate::agent::tools::{self, ToolContext};

mod v1;
mod v2;

/// Which Standard prompt a session is sent (`AgentConfig::prompt_version`).
///
/// v2 is the Phase 5 rewrite of the lean-harness plan: policy the code
/// enforces left to the gates' own error messages, an adaptive Brain
/// interview, the planning checklists in the `plan` skill. v1 is the prompt
/// as it stood before, kept for one release (decision D7) so a user can flip
/// back if v2 is worse for them. Compact and the git operator have one prompt
/// each and ignore it.
///
/// Stored as a string and read leniently: an unknown value is v2, so a config
/// written by a later build never fails to parse here (`load_config` would
/// replace the whole file with defaults).
#[derive(Debug, Clone, Copy, PartialEq, Eq, Default, serde::Serialize, serde::Deserialize)]
#[serde(from = "String", into = "String")]
pub enum PromptVersion {
    V1,
    #[default]
    V2,
}

impl PromptVersion {
    pub fn as_str(&self) -> &'static str {
        match self {
            PromptVersion::V1 => "v1",
            PromptVersion::V2 => "v2",
        }
    }
}

#[cfg(test)]
impl PromptVersion {
    pub const ALL: [PromptVersion; 2] = [PromptVersion::V1, PromptVersion::V2];
}

impl From<String> for PromptVersion {
    fn from(s: String) -> Self {
        match s.trim() {
            "v1" => PromptVersion::V1,
            _ => PromptVersion::V2,
        }
    }
}

impl From<PromptVersion> for String {
    fn from(v: PromptVersion) -> Self {
        v.as_str().to_string()
    }
}

/// Settings › Agent's prompt control (`set_config`'s `promptVersion`).
/// Strict where the file reader is lenient: only "v1" and "v2" change the
/// setting, so a value this build does not know leaves the user's choice alone.
pub fn apply_prompt_version_setting(cfg: &mut AgentConfig, value: Option<&str>) {
    match value {
        Some("v1") => cfg.prompt_version = PromptVersion::V1,
        Some("v2") => cfg.prompt_version = PromptVersion::V2,
        _ => {}
    }
}

/// Prompt budget for the scenario index. Enough for a real spec suite, small
/// enough that a thousand scenarios cannot crowd out the conversation.
const MAX_SPEC_SECTION_CHARS: usize = 4_000;

/// Lean, single-purpose prompt for the `GitSync` profile (commit & push).
/// No task system, no Brain/Builder modes, no skills/subagents — the model's
/// only job is to get local changes committed and pushed, fast.
const GIT_SYNC_PROMPT: &str = r#"Role: Claudinio git operator. Single goal: get the workspace's local changes committed and pushed to the remote, fast.

# WORKFLOW (minimal commands, no ceremony)
1. Run `git status --porcelain=v1 -b` and `git log --oneline -10` to see what changed and the commit message convention used in this project.
2. Stage the relevant changes (`git add -A` unless something clearly must be excluded), then commit with ONE message following the repo's convention.
3. `git push`. If it is rejected as non-fast-forward, run `git pull --rebase` and push again.
4. If the rebase hits conflicts, run `git rebase --abort` immediately and use `ask_user` to ask how to proceed. Never edit files to resolve a conflict yourself.

# RULES
- No task lists, no plans, no subagents, no skills — those tools don't exist in this session.
- Do not ask permission before pushing: pushing IS the goal the user already chose by opening this flow.
- Never run a destructive command (reset --hard, push --force, clean, checkout that discards changes).
- Finish with one short summary: branch, commit subject, and whether the push succeeded.

# LANGUAGE POLICY
- Any language is valid. Write the final summary in the language of the user's most recent message."#;

/// The scenario index for the planning prompt, if the project has specs.
///
/// This is the point of the whole layer: a specification nobody reads while
/// designing is decoration. Putting the scenarios in front of the planner makes
/// them the requirement they claim to be.
pub(super) fn build_spec_prompt_section(
    ctx: &ToolContext,
    version: PromptVersion,
) -> Option<String> {
    let root = ctx.workspace_root.as_deref()?;
    let cfg = crate::quality::QualityConfig::load(std::path::Path::new(root));
    let features = crate::quality::spec::load_features(
        std::path::Path::new(root),
        cfg.features_dir.as_deref(),
    );
    if features.is_empty() {
        return None;
    }
    let index = crate::quality::parsers::scenario_index(&features, MAX_SPEC_SECTION_CHARS);
    if version == PromptVersion::V2 {
        // The spec guard is code (`tools::validate_editable_path`), and its
        // refusal says to ask the user; v2 does not restate it.
        return Some(format!(
            "\n## SPECIFICATION (Gherkin — the user's requirement)\n\
             This project has executable specs: the requirement, and the one input here that \
             did not come from a model. Plan and implement to satisfy these scenarios, and name \
             the ones each task covers, so what is left stays visible. A scenario that looks \
             wrong or cannot be met goes to the user; the specs are theirs to change.\n\n\
             {index}"
        ));
    }
    Some(format!(
        "\n## SPECIFICATION (Gherkin — the user's requirement)\n\
         This project has executable specs. They are the requirement, and they are the one \
         input here that did not come from a model:\n\
         - Your plan and your implementation must satisfy these scenarios.\n\
         - You CANNOT edit them: `edit_file` refuses any path under the spec directory. If a \
         scenario looks wrong or cannot be met, say so and ask the user — never change the \
         spec to match the code.\n\
         - Reference the scenarios each task covers, so it is obvious what is left.\n\n\
         {index}"
    ))
}

/// The Standard prompt's four texts in one version. Each mode block ends the
/// prompt and holds the `{plans_subdir}` placeholder; the base holds
/// `{max_parallel}`.
struct PromptTexts {
    base: &'static str,
    golden: &'static str,
    brain: &'static str,
    builder: &'static str,
}

fn texts(version: PromptVersion) -> PromptTexts {
    match version {
        PromptVersion::V1 => PromptTexts {
            base: v1::SYSTEM_PROMPT,
            golden: v1::GOLDEN_PROMPT,
            brain: v1::BRAIN_BLOCK,
            builder: v1::BUILDER_BLOCK,
        },
        PromptVersion::V2 => PromptTexts {
            base: v2::SYSTEM_PROMPT,
            golden: v2::GOLDEN_PROMPT,
            brain: v2::BRAIN_BLOCK,
            builder: v2::BUILDER_BLOCK,
        },
    }
}

/// A chat profile's prompt `text` followed by the workspace facts, the same
/// sentences for Standard (both versions) and Compact.
fn with_workspace(text: &str, workspace_root: Option<&str>) -> String {
    match workspace_root {
        Some(root) => format!(
            "{text}\n\nProject workspace root: {root}. \
The bash tool already runs with this directory as its working directory - run commands directly \
(e.g. \"git status\"), use relative paths, and never cd into guessed paths. \
File tools take absolute paths inside this root."
        ),
        None => text.to_string(),
    }
}

/// Build the per-session system prompt. The base is byte-identical for every
/// request in the same workspace so the provider's prefix cache stays warm;
/// the mode block is appended last and only changes when the mode switches.
#[allow(clippy::too_many_arguments)]
pub(super) fn system_prompt(
    workspace_root: Option<&str>,
    skills_section: Option<&str>,
    // Index of the project's Gherkin scenarios, when it has any. Placed with
    // the skills block: stable per workspace, so the prefix cache only moves
    // when the specs themselves do.
    spec_section: Option<&str>,
    plan_save_path: Option<&str>,
    mode: SessionMode,
    profile: PromptProfile,
    max_parallel: usize,
    // Whether the session has golden tasks at run start: only then are the
    // golden-task rules sent.
    has_golden_goals: bool,
    // Which Standard texts (`AgentConfig::prompt_version`); Compact and the
    // git operator have one prompt each and ignore it.
    version: PromptVersion,
) -> String {
    if profile == PromptProfile::GitSync {
        return match workspace_root {
            Some(root) => format!(
                "{GIT_SYNC_PROMPT}\n\nProject workspace root: {root}. \
The bash tool already runs with this directory as its working directory - run commands directly \
(e.g. \"git status\"), use relative paths, and never cd into guessed paths."
            ),
            None => GIT_SYNC_PROMPT.to_string(),
        };
    }
    if profile == PromptProfile::Compact {
        // No skills, no specs, no mode block: on the window this profile is
        // for, each of them is a file the model can no longer read.
        return with_workspace(crate::agent::surface::COMPACT_PROMPT, workspace_root);
    }
    let texts = texts(version);
    let base = with_workspace(texts.base, workspace_root);
    let base = match skills_section {
        Some(s) if !s.is_empty() => format!("{base}\n{s}"),
        _ => base,
    };
    let base = match spec_section {
        Some(s) if !s.is_empty() => format!("{base}\n{s}"),
        _ => base,
    };
    // Resolve the effective plans directory for the prompt.
    let plans_subdir = match plan_save_path {
        Some(path) if !path.is_empty() => format!(".claudinio.json (plan_save_path=\"{path}\")"),
        _ => ".claudinio/plans".to_string(),
    };
    let base = base.replace("{max_parallel}", &max_parallel.to_string());
    let golden = if has_golden_goals { texts.golden } else { "" };

    match mode {
        SessionMode::Brain => {
            let brain_prompt = texts.brain.replace("{plans_subdir}", &plans_subdir);
            format!("{base}{golden}{brain_prompt}")
        }
        SessionMode::Builder => {
            let builder_prompt = texts.builder.replace("{plans_subdir}", &plans_subdir);
            format!("{base}{golden}{builder_prompt}")
        }
    }
}

/// What a run sends ahead of the conversation in `mode`: the profile the
/// model's window calls for, then the prompt and tools of that profile as the
/// user's surface for the model shapes them.
///
/// One function for the run's start and for every mode switch, so the three
/// can never be derived from different inputs: a prompt that names a tool the
/// list beside it does not carry costs a failed call each time it is believed.
#[allow(clippy::too_many_arguments)]
pub(super) fn run_prefix(
    config: &AgentConfig,
    ctx: &ToolContext,
    requested: PromptProfile,
    mode: SessionMode,
    skills_section: Option<&str>,
    spec_section: Option<&str>,
    mcp_defs: &[tools::ToolDef],
    has_golden_goals: bool,
    // Deferred tools the session has loaded, in load order (its
    // `tools_loaded` records). Appended to the tool list; ignored by the
    // profiles that defer nothing.
    loaded: &[String],
) -> (PromptProfile, String, Vec<ToolDescription>) {
    let profile =
        crate::agent::surface::effective_profile(requested, config, mode, has_golden_goals);
    let system = system_prompt(
        ctx.workspace_root.as_deref(),
        skills_section,
        spec_section,
        ctx.plan_save_path.as_deref(),
        mode,
        profile,
        subagent::effective_max_parallel(config),
        has_golden_goals,
        config.prompt_version,
    );
    let system = if profile == PromptProfile::Standard {
        crate::agent::surface::prompt_for(config.tool_surface_for(config.model()), system)
    } else {
        system
    };
    let tools = api_tools(mode, profile, mcp_defs, config);
    if profile != PromptProfile::Standard {
        return (profile, system, tools);
    }
    // Standard defers what is rarely called (`agent::deferred`): the catalog
    // joins the prompt whatever is loaded, so loading only ever grows the
    // tool list at its end.
    let (mut core, deferred) = crate::agent::deferred::split(tools);
    core.push(crate::agent::deferred::load_tools_def().into());
    let system = match crate::agent::deferred::catalog_section(&deferred) {
        Some(catalog) => format!("{system}{catalog}"),
        None => system,
    };
    let tools = crate::agent::deferred::materialize(core, &deferred, loaded);
    (profile, system, tools)
}

/// The tools a run in `mode` defers — what `load_tools` (or a direct call)
/// can add to its list. Empty for the profiles that defer nothing.
pub(super) fn deferred_tools(
    mode: SessionMode,
    profile: PromptProfile,
    mcp_defs: &[tools::ToolDef],
    config: &AgentConfig,
) -> Vec<ToolDescription> {
    if profile != PromptProfile::Standard {
        return Vec::new();
    }
    crate::agent::deferred::split(api_tools(mode, profile, mcp_defs, config)).1
}

/// Tools offered to the model for a given mode/profile. `GitSync` gets only
/// `bash` + `ask_user` — no task system, no subagents, no MCP tools. `Compact`
/// gets `surface::COMPACT_TOOLS`, `edit_file` included. Builder gets the full
/// registry (`edit_file` included) plus enter_plan_mode and finalize_plan;
/// Brain drops edit_file and gains write_plan + exit_plan_mode (bash stays but
/// is gated to read-only commands in run_workflow). Under `Standard` the
/// user's per-model surface then applies.
pub(super) fn api_tools(
    mode: SessionMode,
    profile: PromptProfile,
    mcp_defs: &[tools::ToolDef],
    config: &AgentConfig,
) -> Vec<ToolDescription> {
    let maxp = crate::agent::subagent::effective_max_parallel(config);
    if profile == PromptProfile::GitSync {
        return tools::get_defs(maxp)
            .into_iter()
            .filter(|t| t.name == "bash" || t.name == "ask_user")
            .map(|t| ToolDescription {
                name: t.name,
                description: t.description,
                input_schema: t.input_schema,
            })
            .collect();
    }
    if profile == PromptProfile::Compact {
        return crate::agent::surface::compact_defs(tools::get_defs(maxp))
            .into_iter()
            .map(|t| ToolDescription {
                name: t.name,
                description: t.description,
                input_schema: t.input_schema,
            })
            .collect();
    }
    let mut defs = tools::get_defs(maxp);
    crate::agent::surface::retain_for(config.tool_surface_for(config.model()), &mut defs);
    defs.retain(|t| t.name != "web_search" || config.is_claudinio_account());
    // Same treatment as web_search: when the feature is off the tools leave the
    // prompt entirely rather than sitting there costing tokens.
    defs.retain(|t| !t.name.starts_with("browser") || config.browser.enabled);
    // Brain is read-only. Builder edits files itself (decision D1,
    // lean-harness Phase 3); its write gates are `PromptProfile::edits_files`.
    if !profile.edits_files(mode) {
        defs.retain(|t| t.name != "edit_file");
    }
    match mode {
        SessionMode::Builder => {
            defs.push(tools::enter_plan_mode_def());
            defs.push(tools::finalize_plan_def());
        }
        SessionMode::Brain => {
            defs.push(tools::write_plan_def());
            defs.push(tools::exit_plan_mode_def());
        }
    }
    // The living document (Phase 4b), in both modes: `doc_create` core, the
    // rest deferred and unlisted until it loads them.
    defs.extend(tools::doc::session_defs());
    defs.extend(mcp_defs.iter().cloned());
    defs.iter()
        .map(|t| ToolDescription {
            name: t.name.clone(),
            description: t.description.clone(),
            input_schema: t.input_schema.clone(),
        })
        .collect()
}

/// Tests that lock in the "brain/builder must interview about size and preserve
/// user-supplied assets, and subagent goals must be complete specs" behavior.
///
/// The deterministic tests assert the prompt invariants (cheap, always run).
/// The `#[ignore]` tests replay the real session 44ec41c1 (textarea editor
/// modal) against the LIVE Brain/Builder models to prove the hardened prompts
/// actually make the model (a) ask about modal size and (b) carry the exact
/// icon reference into a subagent goal instead of saying "similar to".
///
/// Run the live evals with:
///   CLAUDINIO_API_KEY=sk-… cargo test --lib prompt_eval -- --ignored --nocapture
#[cfg(test)]
mod prompt_eval_tests {
    use super::*;
    use crate::agent::provider::{AgentConfig, one_shot};

    const ROOT: &str = "/Users/victortavernari/claudinio_code";

    /// The verbatim first user message from session 44ec41c1 — a UI feature
    /// (button + modal) that carries a concrete icon reference (a URL naming
    /// the exact `lucide:notebook-pen` icon). Left in the original Portuguese
    /// on purpose: it is captured regression data, not app copy, and the point
    /// is that the prompt keeps the asset reference verbatim whatever the
    /// language.
    const SESSION_REQUEST: &str = "Gostaria que o textarea do input tivesse um botão para editar \
https://icones.js.org/collection/all?s=notebook&icon=lucide:notebook-pen e ao clicar nele, ele pega \
o texto que está na text area e abre um editor de texto numa modal com multiplas linhas, e ao fechar \
essa modal este texto volte para a text area, e assim posso enviar o texto editado.";

    // ---- Deterministic prompt-invariant tests (no network) ----

    // Any language is valid for the model's input and output: no prompt may
    // require the model to reason, call tools or reply in one language.
    #[test]
    fn no_system_prompt_imposes_a_language() {
        for version in PromptVersion::ALL {
            for profile in [PromptProfile::Standard, PromptProfile::GitSync] {
                for mode in [SessionMode::Brain, SessionMode::Builder] {
                    let sys = system_prompt(
                        Some(ROOT),
                        None,
                        None,
                        None,
                        mode,
                        profile,
                        subagent::MAX_PARALLEL_AGENTS,
                        false,
                        version,
                    );
                    assert!(!sys.contains("MUST be in English"), "{profile:?}/{mode:?}");
                    assert!(!sys.contains("default to English"), "{profile:?}/{mode:?}");
                    assert!(!sys.contains("else English"), "{profile:?}/{mode:?}");
                }
            }
        }
    }

    // A user message in any language must reach the model. The run used to
    // refuse any message with >10% non-ASCII characters before it started,
    // which locked out Portuguese, Chinese, Farsi... users. Pinned on the
    // source because the gate sat at the top of the run, before any seam a
    // unit test can reach. That run lived in `session.rs` until Phase 8 split
    // it, so every file holding a piece of the old `session.rs` is scanned.
    #[test]
    fn a_message_in_any_language_is_never_refused() {
        let sources = [
            ("session.rs", include_str!("session.rs")),
            ("run_loop.rs", include_str!("run_loop.rs")),
            ("tool_exec.rs", include_str!("tool_exec.rs")),
            ("handoff.rs", include_str!("handoff.rs")),
            ("judge.rs", include_str!("judge.rs")),
            ("prompt.rs", include_str!("prompt.rs")),
            ("events.rs", include_str!("events.rs")),
            ("retry.rs", include_str!("retry.rs")),
        ];
        let gate = ["reject_non", "_english("].concat();
        let refusal = ["Only English", " is supported"].concat();
        for (file, src) in sources {
            assert!(!src.contains(&gate), "the language gate is back in {file}");
            assert!(
                !src.contains(&refusal),
                "an English-only refusal is back in {file}"
            );
        }
    }

    /// The golden-task rules are instructions about tasks that exist only
    /// when the user set `<goal>`s; a session without one was paying ~1.3k
    /// characters on every request for rules it could not apply. Read once at
    /// run start (`has_golden_goals`), so the prefix stays fixed for the run.
    #[test]
    fn golden_rules_are_sent_only_to_a_session_that_has_golden_goals() {
        let ctx = crate::agent::tools::tests_support::ctx();
        for mode in [SessionMode::Brain, SessionMode::Builder] {
            for golden in [false, true] {
                let (profile, system, _) = run_prefix(
                    &AgentConfig::default(),
                    &ctx,
                    PromptProfile::Standard,
                    mode,
                    None,
                    None,
                    &[],
                    golden,
                    &[],
                );
                assert_eq!(profile, PromptProfile::Standard);
                assert_eq!(
                    system.contains("## GOLDEN TASKS"),
                    golden,
                    "{mode:?} with golden goals = {golden}"
                );
            }
        }
    }

    /// The base prompt and each mode block both ended on the language
    /// policy, so a Standard session carried it twice on every request.
    #[test]
    fn the_language_policy_is_stated_once() {
        for version in PromptVersion::ALL {
            for mode in [SessionMode::Brain, SessionMode::Builder] {
                let sys = system_prompt(
                    Some(ROOT),
                    None,
                    None,
                    None,
                    mode,
                    PromptProfile::Standard,
                    subagent::MAX_PARALLEL_AGENTS,
                    false,
                    version,
                );
                assert_eq!(sys.matches("# LANGUAGE POLICY").count(), 1, "{mode:?}");
            }
        }
    }

    /// Links are rendering facts, not instructions: two lines, no examples.
    #[test]
    fn the_links_section_is_two_lines() {
        for version in PromptVersion::ALL {
            let sys = system_prompt(
                Some(ROOT),
                None,
                None,
                None,
                SessionMode::Builder,
                PromptProfile::Standard,
                subagent::MAX_PARALLEL_AGENTS,
                false,
                version,
            );
            // v1 numbers its sections ("# 6. LINKS"), v2 does not.
            let start = sys
                .match_indices("\n# ")
                .map(|(at, _)| at + 1)
                .find(|&at| {
                    sys[at..]
                        .lines()
                        .next()
                        .is_some_and(|l| l.contains("LINKS"))
                })
                .expect("links section");
            let section: Vec<&str> = sys[start..]
                .lines()
                .skip(1)
                .take_while(|l| !l.starts_with('#') && !l.is_empty())
                .collect();
            assert_eq!(section.len(), 2, "{section:?}");
            assert!(section[0].contains("relative") && section[0].contains("workspace"));
            assert!(section[1].contains("image") && section[1].contains("video"));
            assert!(!sys.contains("Examples:"));
        }
    }

    // Prompt v1 only. v2 carries the sizing and asset rules in the built-in `plan`
    // skill, which its Brain block names (`skills::plan_skill_tests`).
    #[test]
    fn brain_prompt_mandates_size_and_verbatim_assets() {
        let sys = system_prompt(
            Some(ROOT),
            None,
            None,
            None,
            SessionMode::Brain,
            PromptProfile::Standard,
            subagent::MAX_PARALLEL_AGENTS,
            false,
            PromptVersion::V1,
        );
        // Size/dimensions must be a mandatory interview item.
        assert!(
            sys.contains("Sizing and layout"),
            "Brain prompt must force interviewing about UI size/layout"
        );
        // User-supplied assets must be captured verbatim, not paraphrased.
        assert!(
            sys.contains("VERBATIM"),
            "Brain prompt must require recording user assets verbatim"
        );
        assert!(
            sys.to_lowercase().contains("ground truth"),
            "Brain prompt must treat a user-supplied asset as ground truth"
        );
    }

    // Prompt v1 only. v2 carries the Low-Level Design and consensus guidance in the built-in `plan`
    // skill, which its Brain block names (`skills::plan_skill_tests`).
    #[test]
    fn brain_prompt_mandates_lld_stage() {
        let sys = system_prompt(
            Some(ROOT),
            None,
            None,
            None,
            SessionMode::Brain,
            PromptProfile::Standard,
            subagent::MAX_PARALLEL_AGENTS,
            false,
            PromptVersion::V1,
        );
        assert!(
            sys.contains("## Low-Level Design"),
            "Brain prompt must require the Low-Level Design section"
        );
        assert!(
            sys.contains("all three deliverables"),
            "Brain prompt must list three mandatory deliverables"
        );
        assert!(
            sys.contains("false consensus"),
            "Brain prompt must include the interview consensus checklist"
        );
        assert!(
            sys.contains("code-gated"),
            "Brain prompt must state that tasks_set is gated on the LLD"
        );
    }

    /// Status changes go through the one-task tool. A prompt that still told
    /// the model to resend the list would make the delta dead code.
    #[test]
    fn status_changes_are_pointed_at_tasks_update_in_both_modes() {
        for version in PromptVersion::ALL {
            for mode in [SessionMode::Brain, SessionMode::Builder] {
                let sys = system_prompt(
                    Some(ROOT),
                    None,
                    None,
                    None,
                    mode,
                    PromptProfile::Standard,
                    subagent::MAX_PARALLEL_AGENTS,
                    false,
                    version,
                );
                // Phase 4: `doing` rides with the task's first tool call rather
                // than taking a round of its own.
                assert!(sys.contains(
                "Mark a task `doing` with `tasks_update` in the same round as the first tool call for it"
            ));
                assert!(sys.contains("Never resend the list just to change a status"));
                assert!(!sys.contains("you MUST make a final `tasks_set` call"));
                let names: Vec<String> =
                    api_tools(mode, PromptProfile::Standard, &[], &AgentConfig::default())
                        .into_iter()
                        .map(|t| t.name)
                        .collect();
                for tool in ["tasks_get", "tasks_set", "tasks_update"] {
                    assert!(names.contains(&tool.to_string()), "{mode:?} lacks {tool}");
                }
            }
            let builder = system_prompt(
                Some(ROOT),
                None,
                None,
                None,
                SessionMode::Builder,
                PromptProfile::Standard,
                subagent::MAX_PARALLEL_AGENTS,
                false,
                version,
            );
            // Phase 4: closing one task and starting the next is one call.
            assert!(
                builder
                    .contains("closing one task and starting the next is one `tasks_update` call")
            );
            // The git profile has no task system at all.
            let git = api_tools(
                SessionMode::Builder,
                PromptProfile::GitSync,
                &[],
                &AgentConfig::default(),
            );
            assert!(git.iter().all(|t| !t.name.starts_with("tasks_")));
        }
    }

    // Decision D1 (lean-harness Phase 3): the Builder prompt must not tell a
    // session that has `edit_file` it cannot edit, and must say when a
    // subagent pays: explore for broad read-only work, code only for
    // parallel changes to disjoint files; otherwise the session edits.
    #[test]
    fn the_builder_prompt_edits_directly_and_says_when_to_delegate() {
        for version in PromptVersion::ALL {
            for golden in [false, true] {
                let sys = system_prompt(
                    Some(ROOT),
                    None,
                    None,
                    None,
                    SessionMode::Builder,
                    PromptProfile::Standard,
                    subagent::MAX_PARALLEL_AGENTS,
                    golden,
                    version,
                );
                for gone in [
                    "CANNOT edit",
                    "there is no edit_file",
                    "ALL file modifications go through code-mode subagents",
                    "one subagent per task",
                ] {
                    assert!(!sys.contains(gone), "still says {gone:?}");
                }
                assert!(sys.contains("`edit_file`"), "names the edit tool");
                assert!(sys.contains("'explore'"), "says when to explore");
                assert!(sys.contains("disjoint files"), "says when code pays");
                assert!(sys.contains("Otherwise edit directly"));
            }
        }
    }

    // Decision D1 (lean-harness Phase 3): the Builder main session edits
    // files itself; Brain stays read-only and the git operator never edits.
    #[test]
    fn the_standard_builder_is_offered_edit_file_and_brain_is_not() {
        let names = |mode, profile| -> Vec<String> {
            api_tools(mode, profile, &[], &AgentConfig::default())
                .into_iter()
                .map(|t| t.name)
                .collect()
        };
        let edit = "edit_file".to_string();
        assert!(names(SessionMode::Builder, PromptProfile::Standard).contains(&edit));
        assert!(!names(SessionMode::Brain, PromptProfile::Standard).contains(&edit));
        assert!(!names(SessionMode::Builder, PromptProfile::GitSync).contains(&edit));
    }

    // Prompt v1 only. v2 carries the Mermaid guidance in the built-in `plan`
    // skill, which its Brain block names (`skills::plan_skill_tests`).
    #[test]
    fn brain_prompt_encourages_mermaid_diagrams() {
        let sys = system_prompt(
            Some(ROOT),
            None,
            None,
            None,
            SessionMode::Brain,
            PromptProfile::Standard,
            subagent::MAX_PARALLEL_AGENTS,
            false,
            PromptVersion::V1,
        );
        assert!(
            sys.contains("mermaid"),
            "Brain prompt must encourage expressing ideas with Mermaid diagrams"
        );
        // Must not restrict Brain to sequence/UML - the whole catalog is fair game.
        assert!(
            sys.contains("FULL Mermaid catalog"),
            "Brain prompt must not limit diagrams to sequence/UML"
        );
    }

    #[test]
    fn builder_prompt_mentions_lld_context() {
        for version in PromptVersion::ALL {
            let sys = system_prompt(
                Some(ROOT),
                None,
                None,
                None,
                SessionMode::Builder,
                PromptProfile::Standard,
                subagent::MAX_PARALLEL_AGENTS,
                false,
                version,
            );
            assert!(
                sys.contains("Low-Level Design"),
                "Builder prompt must point at the Low-Level Design as the technical spec"
            );
        }
    }

    #[test]
    fn builder_prompt_requires_complete_subagent_spec() {
        for version in PromptVersion::ALL {
            let sys = system_prompt(
                Some(ROOT),
                None,
                None,
                None,
                SessionMode::Builder,
                PromptProfile::Standard,
                subagent::MAX_PARALLEL_AGENTS,
                false,
                version,
            );
            // v2 says it without the capitals (design rule 4); the rule is
            // the same.
            assert!(
                sys.to_lowercase().contains("complete technical spec"),
                "Builder prompt must require complete subagent specs"
            );
            assert!(
                sys.contains("VERBATIM"),
                "Builder prompt must require repeating concrete values verbatim to subagents"
            );
        }
    }

    #[test]
    fn the_scenario_index_reaches_the_planning_prompt() {
        // A spec nobody reads while designing is decoration. This is the wire
        // that makes it a requirement.
        let root = std::env::temp_dir().join(format!("cq-specprompt-{}", std::process::id()));
        std::fs::remove_dir_all(&root).ok();
        std::fs::create_dir_all(root.join("features")).unwrap();
        std::fs::write(
            root.join("features/discount.feature"),
            "Feature: Member discounts\n  Scenario: Ten percent off\n    Then the total is 90\n",
        )
        .unwrap();
        let mut ctx = crate::agent::tools::tests_support::ctx();
        ctx.workspace_root = Some(root.to_string_lossy().to_string());

        let section = build_spec_prompt_section(&ctx, PromptVersion::V1).expect("specs found");
        assert!(section.contains("Member discounts"), "{section}");
        assert!(section.contains("Ten percent off"), "{section}");
        assert!(section.contains("CANNOT edit"), "{section}");

        let sys = system_prompt(
            Some("/ws"),
            None,
            Some(&section),
            None,
            SessionMode::Brain,
            PromptProfile::Standard,
            4,
            false,
            PromptVersion::default(),
        );
        assert!(
            sys.contains("Ten percent off"),
            "the planner must see the scenarios"
        );
        std::fs::remove_dir_all(&root).ok();
    }

    #[test]
    fn a_project_without_specs_adds_nothing_to_the_prompt() {
        // No spec directory must cost zero prompt budget.
        let root = std::env::temp_dir().join(format!("cq-nospec-{}", std::process::id()));
        std::fs::create_dir_all(&root).unwrap();
        let mut ctx = crate::agent::tools::tests_support::ctx();
        ctx.workspace_root = Some(root.to_string_lossy().to_string());
        for version in PromptVersion::ALL {
            assert!(build_spec_prompt_section(&ctx, version).is_none());
        }
        std::fs::remove_dir_all(&root).ok();
    }

    #[test]
    fn system_prompt_warns_against_similar_to_guessing() {
        for version in PromptVersion::ALL {
            let sys = system_prompt(
                Some(ROOT),
                None,
                None,
                None,
                SessionMode::Builder,
                PromptProfile::Standard,
                subagent::MAX_PARALLEL_AGENTS,
                false,
                version,
            );
            assert!(
                sys.contains("similar to"),
                "subagent guidance must call out the 'similar to X' anti-pattern"
            );
            assert!(
                sys.contains("isn't yet concrete data"),
                "subagent guidance must require resolving user references before delegating"
            );
        }
    }

    #[test]
    fn git_sync_prompt_has_no_task_system_or_modes() {
        let sys = system_prompt(
            Some(ROOT),
            None,
            None,
            None,
            SessionMode::Builder,
            PromptProfile::GitSync,
            subagent::MAX_PARALLEL_AGENTS,
            false,
            PromptVersion::default(),
        );
        assert!(
            !sys.contains("tasks_get") && !sys.contains("tasks_set"),
            "GitSync prompt must not mention the task system"
        );
        assert!(
            !sys.contains("CURRENT MODE"),
            "GitSync prompt must not include a Brain/Builder mode block"
        );
        assert!(
            sys.contains("git push"),
            "GitSync prompt must describe the git workflow"
        );
    }

    #[test]
    fn git_sync_tools_are_bash_and_ask_user_only() {
        let defs = api_tools(
            SessionMode::Builder,
            PromptProfile::GitSync,
            &[],
            &AgentConfig::default(),
        );
        let names: Vec<&str> = defs.iter().map(|t| t.name.as_str()).collect();
        assert_eq!(
            names.len(),
            2,
            "GitSync toolset must be exactly bash + ask_user, got {names:?}"
        );
        assert!(names.contains(&"bash"));
        assert!(names.contains(&"ask_user"));
    }

    // ---- Live-API evals (ignored by default; need CLAUDINIO_API_KEY) ----

    fn live_cfg() -> Option<AgentConfig> {
        let api_key = std::env::var("CLAUDINIO_API_KEY").ok()?;
        Some(AgentConfig {
            base_url: "https://api.claudin.io".into(),
            api_key,
            ..AgentConfig::default()
        })
    }

    /// Feed the REAL Brain system prompt + the real session request to the live
    /// Brain model and ask it to list the clarifying questions it would ask
    /// before writing a plan. It MUST surface (a) the modal SIZE and (b) the
    /// exact icon asset — the two things it silently invented last time.
    #[tokio::test]
    #[ignore = "hits the live claudin.io API; requires CLAUDINIO_API_KEY"]
    async fn brain_interview_covers_modal_size_and_icon_asset() {
        let Some(cfg) = live_cfg() else {
            eprintln!("skipping: CLAUDINIO_API_KEY not set");
            return;
        };
        let system = system_prompt(
            Some(ROOT),
            None,
            None,
            None,
            SessionMode::Brain,
            PromptProfile::Standard,
            subagent::MAX_PARALLEL_AGENTS,
            false,
            // v1: the question must come from the prompt alone (no tool may
            // be called here), and v2 keeps the sizing rules in the `plan`
            // skill, read with a tool.
            PromptVersion::V1,
        );
        let model = cfg.model().to_string();
        let user = format!(
            "{SESSION_REQUEST}\n\n---\nDo NOT call any tool and do NOT write a plan. Instead, output \
ONLY a numbered list of the clarifying questions you must ask me before writing the plan."
        );
        let reply = one_shot(&cfg, &model, &system, &user, 1500)
            .await
            .expect("live brain call");
        eprintln!(
            "--- brain clarifying questions ---\n{reply}\n----------------------------------"
        );
        assert_interview_covers_size_and_asset(&reply);
    }

    /// The interview session 44ec41c1 needed: it asks the modal's size and
    /// keeps the exact icon the user linked.
    fn assert_interview_covers_size_and_asset(reply: &str) {
        let lc = reply.to_lowercase();
        let asks_size = [
            "tamanho",
            "size",
            "dimens",
            "largura",
            "altura",
            "width",
            "height",
            "fullscreen",
            "tela cheia",
            "viewport",
            "margin",
            "margem",
        ]
        .iter()
        .any(|k| lc.contains(k));
        let asks_asset = [
            "ícone",
            "icone",
            "icon",
            "lucide",
            "notebook-pen",
            "svg",
            "url",
        ]
        .iter()
        .any(|k| lc.contains(k));
        assert!(
            asks_size,
            "Brain must interview about the modal size/dimensions"
        );
        assert!(
            asks_asset,
            "Brain must confirm/preserve the exact icon asset the user linked"
        );
    }

    /// The same interview under prompt v2, the default, which keeps the
    /// sizing and asset checklists in the built-in `plan` skill its Brain
    /// block sends the model to. This call is tool-free, so the skill's text
    /// is handed over as the `skills` read returns it: what is checked live is
    /// that v2's prompt with that checklist still asks both questions. Whether
    /// the model chooses to read the skill at all is the eval's v1-vs-v2 gate
    /// (lean-harness plan, Phase 5), not something one request can show.
    #[tokio::test]
    #[ignore = "hits the live claudin.io API; requires CLAUDINIO_API_KEY"]
    async fn brain_interview_covers_modal_size_and_icon_asset_with_the_plan_skill_read() {
        let Some(cfg) = live_cfg() else {
            eprintln!("skipping: CLAUDINIO_API_KEY not set");
            return;
        };
        let system = system_prompt(
            Some(ROOT),
            None,
            None,
            None,
            SessionMode::Brain,
            PromptProfile::Standard,
            subagent::MAX_PARALLEL_AGENTS,
            false,
            PromptVersion::V2,
        );
        assert!(
            system.contains("`plan`"),
            "v2's Brain block names the plan skill"
        );
        let plan = crate::agent::skills::SkillManager::builtin_entries()
            .into_iter()
            .find(|e| e.name == "plan")
            .and_then(|e| e.body)
            .expect("the built-in plan skill");
        let model = cfg.model().to_string();
        let user = format!(
            "{SESSION_REQUEST}\n\n---\nYou already read the `plan` skill; `skills` returned:\n\n\
{plan}\n\n---\nDo NOT call any tool and do NOT write a plan. Instead, output ONLY a numbered \
list of the clarifying questions you must ask me before writing the plan."
        );
        let reply = one_shot(&cfg, &model, &system, &user, 1500)
            .await
            .expect("live brain call");
        eprintln!("--- v2 brain clarifying questions ---\n{reply}\n---");
        assert_interview_covers_size_and_asset(&reply);
    }

    /// Give the live Builder model a plan step that references the user's exact
    /// icon URL and ask it for the subagent goal it would spawn. The goal MUST
    /// carry the concrete reference (URL / exact id / fetch instruction) rather
    /// than the "similar to lucide notebook-pen" guess that produced the wrong
    /// icon in the original session.
    #[tokio::test]
    #[ignore = "hits the live claudin.io API; requires CLAUDINIO_API_KEY"]
    async fn builder_subagent_goal_carries_user_asset() {
        let Some(cfg) = live_cfg() else {
            eprintln!("skipping: CLAUDINIO_API_KEY not set");
            return;
        };
        let system = system_prompt(
            Some(ROOT),
            None,
            None,
            None,
            SessionMode::Builder,
            PromptProfile::Standard,
            subagent::MAX_PARALLEL_AGENTS,
            false,
            PromptVersion::default(),
        );
        let model = cfg.model().to_string();
        let user = "Plan task: add a new icon named 'notebook-pen' to src/components/Icon.tsx. The user \
specified the EXACT icon to use with this reference: \
https://icones.js.org/collection/all?s=notebook&icon=lucide:notebook-pen (that is the Lucide \
'notebook-pen' icon).\n\n---\nDo NOT call any tool. Output ONLY the exact `goal` string you would \
pass to a single 'code' subagent to implement this task.";
        let reply = one_shot(&cfg, &model, &system, user, 1200)
            .await
            .expect("live builder call");
        eprintln!("--- builder subagent goal ---\n{reply}\n-----------------------------");
        let lc = reply.to_lowercase();
        let carries_ref = lc.contains("lucide:notebook-pen")
            || lc.contains("icones.js.org")
            || lc.contains("fetch");
        assert!(
            carries_ref,
            "subagent goal must embed the exact icon reference or instruct the agent to fetch it, \
not merely say 'similar to'"
        );
    }
}

/// The prefix — system prompt plus tool list — is what a provider caches, so
/// it must come out byte-identical every time the same run builds it (design
/// rule 2 of the lean-harness plan). A `HashMap` walked in iteration order, a
/// timestamp or a per-round value anywhere in it would silently turn every
/// request into a cache miss; these tests are the tripwire.
#[cfg(test)]
mod prefix_stability_tests {
    use super::*;
    use crate::agent::skills::{SkillManager, build_skills_system_prompt_section};

    fn skill(dir: &std::path::Path, name: &str, description: &str) {
        std::fs::create_dir_all(dir.join(name)).unwrap();
        std::fs::write(
            dir.join(name).join("SKILL.md"),
            format!("---\nname: {name}\ndescription: {description}\n---\n\nBody of {name}."),
        )
        .unwrap();
    }

    /// A workspace with skills in several scopes: project skills under two of
    /// the three conventional directories, and subfolder skills in two
    /// monorepo packages — one name defined in both packages.
    fn workspace(tag: &str) -> std::path::PathBuf {
        let root = std::env::temp_dir().join(format!(
            "claudinio-prefix-{tag}-{}-{}",
            std::process::id(),
            crate::agent::persist::now_ms()
        ));
        skill(
            &root.join(".agents/skills"),
            "zeta-deploy",
            "Deploys things",
        );
        skill(
            &root.join(".agents/skills"),
            "alpha-review",
            "Reviews diffs",
        );
        skill(&root.join(".claude/skills"), "mid-docs", "Writes docs");
        for (pkg, description) in [
            ("web", "Lint the web package"),
            ("api", "Lint the api package"),
        ] {
            skill(
                &root.join("packages").join(pkg).join(".agents/skills"),
                "lint",
                description,
            );
            skill(
                &root.join("packages").join(pkg).join(".agents/skills"),
                &format!("{pkg}-build"),
                "Builds a package",
            );
        }
        // Eight more packages sharing one name: enough that `read_dir` order
        // matching name order by accident would be a coincidence.
        for pkg in ["p5", "p2", "p7", "p0", "p6", "p3", "p1", "p4"] {
            skill(
                &root.join("packages").join(pkg).join(".claude/skills"),
                "shared",
                &format!("Shared from {pkg}"),
            );
        }
        // The plugin scope: eight project plugins, each shipping a skill under
        // one shared name, and one under a name of its own.
        for plugin in ["q5", "q2", "q7", "q0", "q6", "q3", "q1", "q4"] {
            let proot = root.join(".claudinio/plugins").join(plugin);
            std::fs::create_dir_all(&proot).unwrap();
            std::fs::write(
                proot.join("plugin.json"),
                format!(
                    "{{\"$schema\":\"{}\",\"name\":\"{plugin}\"}}",
                    crate::agent::plugins::PLUGIN_SCHEMA_ID
                ),
            )
            .unwrap();
            skill(
                &proot.join("skills"),
                "plugin-shared",
                &format!("Plugin skill from {plugin}"),
            );
            skill(
                &proot.join("skills"),
                &format!("{plugin}-own"),
                "A plugin's own skill",
            );
        }
        root
    }

    /// Two MCP servers' tools, raw as they arrive and stored through the
    /// same step `connect_all` uses; `swapped`
    /// reverses both the server arrival order and each server's tool order,
    /// which is what a different `HashMap` seed and a server that builds its
    /// tool list from a map produce on the next connect.
    fn mcp_defs(swapped: bool) -> Vec<tools::ToolDef> {
        let def = |name: &str| tools::ToolDef {
            name: name.into(),
            description: format!("{name} does one thing"),
            input_schema: serde_json::json!({
                "type": "object",
                "properties": {"query": {"type": "string"}, "limit": {"type": "integer"}}
            }),
        };
        let mut servers = vec![
            (
                "github".to_string(),
                vec![def("mcp__github__search"), def("mcp__github__issue")],
            ),
            (
                "linear".to_string(),
                vec![def("mcp__linear__list"), def("mcp__linear__create")],
            ),
        ];
        if swapped {
            servers.reverse();
            for (_, defs) in &mut servers {
                defs.reverse();
            }
        }
        let outcomes = servers.into_iter().map(|(n, d)| (n, Ok(d))).collect();
        crate::agent::mcp::snapshots(outcomes).0
    }

    fn prefix_json(root: &std::path::Path, mode: SessionMode) -> String {
        prefix_json_with_mcp(root, mode, &[])
    }

    fn prefix_json_with_mcp(
        root: &std::path::Path,
        mode: SessionMode,
        mcp_defs: &[tools::ToolDef],
    ) -> String {
        let config = AgentConfig::default();
        let ctx = ToolContext {
            workspace_root: Some(root.to_string_lossy().to_string()),
            ..crate::agent::tools::tests_support::ctx()
        };
        // Built the way the run builds it: a fresh scan per run.
        let skills = build_skills_system_prompt_section(&SkillManager::with_plugin_prefs(
            Some(root.to_path_buf()),
            &config.plugins,
        ));
        let (profile, system, tools) = run_prefix(
            &config,
            &ctx,
            PromptProfile::Standard,
            mode,
            skills.as_deref(),
            None,
            mcp_defs,
            false,
            &[],
        );
        assert_eq!(profile, PromptProfile::Standard);
        serde_json::to_string(&serde_json::json!({ "system": system, "tools": tools })).unwrap()
    }

    #[test]
    fn the_run_prefix_is_byte_identical_when_built_twice() {
        let root = workspace("twice");
        for mode in [SessionMode::Builder, SessionMode::Brain] {
            let first = prefix_json(&root, mode);
            let second = prefix_json(&root, mode);
            assert!(first.contains("zeta-deploy") && first.contains("web-build"));
            assert_eq!(first, second, "{mode:?}: the prefix changed between builds");
        }
        std::fs::remove_dir_all(&root).ok();
    }

    /// Skills from the plugin scope join the catalog deterministically too:
    /// when two plugins ship one name, the plugin that sorts first keeps it.
    #[test]
    fn plugin_scope_skills_are_deterministic_in_the_prefix() {
        let root = workspace("plugins");
        let first = prefix_json(&root, SessionMode::Builder);
        for _ in 0..3 {
            assert_eq!(first, prefix_json(&root, SessionMode::Builder));
        }
        assert!(first.contains("q4-own") && first.contains("- plugin-shared \u{2014} "));
        assert!(
            first.contains("Plugin skill from q0") && !first.contains("Plugin skill from q5"),
            "the first plugin in path order keeps a shared skill name"
        );
        std::fs::remove_dir_all(&root).ok();
    }

    /// MCP definitions from two servers keep the prefix byte-identical however
    /// the servers and their tools arrived, and are listed in the deferred
    /// catalog in server-then-tool name order.
    #[test]
    fn mcp_definitions_from_two_servers_keep_the_prefix_byte_identical() {
        let root = workspace("mcp");
        for mode in [SessionMode::Builder, SessionMode::Brain] {
            let a = prefix_json_with_mcp(&root, mode, &mcp_defs(false));
            let b = prefix_json_with_mcp(&root, mode, &mcp_defs(true));
            assert_eq!(a, b, "{mode:?}: MCP arrival order moved the prefix");
            assert_eq!(a, prefix_json_with_mcp(&root, mode, &mcp_defs(false)));
            // MCP tools are deferred (Phase 2): their order is the catalog's.
            let prefix: serde_json::Value = serde_json::from_str(&a).unwrap();
            let system = prefix["system"].as_str().unwrap();
            let names: Vec<&str> = system
                .lines()
                .filter_map(|l| l.strip_prefix("- "))
                .filter_map(|l| l.split(' ').next())
                .filter(|n| n.starts_with("mcp__"))
                .collect();
            assert_eq!(
                names,
                [
                    "mcp__github__issue",
                    "mcp__github__search",
                    "mcp__linear__create",
                    "mcp__linear__list"
                ]
            );
        }
        std::fs::remove_dir_all(&root).ok();
    }

    /// The skills catalog lists skills in scope-then-name order whatever
    /// order the directories were read in, and when one name exists in two
    /// packages of the same scope the winner does not depend on `read_dir`
    /// order either: the package that sorts first keeps it.
    #[test]
    fn the_skills_catalog_is_deterministic_across_scans() {
        let root = workspace("catalog");
        let sections: Vec<String> = (0..5)
            .map(|_| {
                build_skills_system_prompt_section(&SkillManager::new(Some(root.clone()))).unwrap()
            })
            .collect();
        assert!(sections.windows(2).all(|w| w[0] == w[1]));
        let s = &sections[0];
        // Project skills first, then the monorepo packages' (Subfolder).
        let order: Vec<usize> = [
            "alpha-review",
            "mid-docs",
            "zeta-deploy",
            "api-build",
            "lint",
            "web-build",
        ]
        .iter()
        .map(|n| {
            s.find(&format!("- {n} \u{2014} "))
                .unwrap_or_else(|| panic!("{n}"))
        })
        .collect();
        assert!(
            order.windows(2).all(|w| w[0] < w[1]),
            "scope-then-name order: {order:?}"
        );
        assert!(
            s.contains("Lint the api package") && !s.contains("Lint the web package"),
            "the first package in path order keeps a shared name"
        );
        assert!(
            s.contains("Shared from p0"),
            "and so does p0 among the eight"
        );
        std::fs::remove_dir_all(&root).ok();
    }
}

/// Deferred tools in the prefix (lean-harness Phase 2): a Standard main
/// session sends a core set, names the rest in the system prompt, and appends
/// what the session loaded.
#[cfg(test)]
mod deferred_prefix_tests {
    use super::*;

    fn names(tools: &[ToolDescription]) -> Vec<&str> {
        tools.iter().map(|t| t.name.as_str()).collect()
    }

    fn prefix(
        config: &AgentConfig,
        mode: SessionMode,
        mcp: &[tools::ToolDef],
        loaded: &[&str],
    ) -> (PromptProfile, String, Vec<ToolDescription>) {
        let loaded: Vec<String> = loaded.iter().map(|s| s.to_string()).collect();
        run_prefix(
            config,
            &crate::agent::tools::tests_support::ctx(),
            PromptProfile::Standard,
            mode,
            None,
            None,
            mcp,
            false,
            &loaded,
        )
    }

    fn mcp(name: &str) -> tools::ToolDef {
        tools::ToolDef {
            name: name.into(),
            description: format!("{name} does one thing"),
            input_schema: serde_json::json!({"type": "object"}),
        }
    }

    fn catalog_names(system: &str) -> Vec<String> {
        let start = system.find("## DEFERRED TOOLS").expect("a catalog");
        system[start..]
            .lines()
            .filter_map(|l| l.strip_prefix("- "))
            .map(|l| l.split(' ').next().unwrap().to_string())
            .collect()
    }

    #[test]
    fn a_standard_builder_sends_the_core_tools_and_lists_the_rest() {
        let (profile, system, tools) =
            prefix(&AgentConfig::default(), SessionMode::Builder, &[], &[]);
        assert_eq!(profile, PromptProfile::Standard);
        // `edit_file` is core in Builder since decision D1 (Phase 3);
        // `tasks_get` is deferred since Phase 4 (the run recites the list);
        // `doc_create` is core since Phase 4b (the living document).
        assert_eq!(
            names(&tools),
            [
                "read_file",
                "list_dir",
                "grep",
                "edit_file",
                "bash",
                "run_quality",
                "ask_user",
                "tasks_set",
                "tasks_update",
                "spawn_agents",
                "skills",
                "finalize_plan",
                "doc_create",
                "load_tools"
            ]
        );
        assert_eq!(
            catalog_names(&system),
            [
                "semantic_search",
                "code_search",
                "symbol_lookup",
                "file_outline",
                "go_to_definition",
                "find_references",
                "browser",
                "browser_inspect",
                "browser_screenshot",
                "enter_plan_mode",
                "tasks_get"
            ]
        );
    }

    /// The living document (Phase 4b): `doc_create` is core in both modes;
    /// the three tools that work on a document are deferred but never listed
    /// — a session without a document has no use for them, and `doc_create`
    /// loads them (`deferred::before_call`). Once loaded they ride at the
    /// end of the list like any loaded tool. Compact and the git operator
    /// get none of them.
    #[test]
    fn doc_create_is_core_and_the_document_tools_wait_unlisted_until_loaded() {
        let config = AgentConfig::default();
        for mode in [SessionMode::Builder, SessionMode::Brain] {
            let (_, system, tools) = prefix(&config, mode, &[], &[]);
            let sent = names(&tools);
            assert!(sent.contains(&"doc_create"), "{mode:?}: {sent:?}");
            for t in crate::agent::deferred::DOCUMENT_TOOLS {
                assert!(!sent.contains(t), "{mode:?} sends {t} with no document");
                assert!(!catalog_names(&system).contains(&t.to_string()));
                assert!(crate::agent::deferred::is_deferred(t));
            }
            let loaded = crate::agent::deferred::DOCUMENT_TOOLS;
            let (_, _, tools) = prefix(&config, mode, &[], loaded);
            assert_eq!(names(&tools)[tools.len() - 3..], *loaded);
        }
        for profile in [PromptProfile::Compact, PromptProfile::GitSync] {
            let tools = api_tools(SessionMode::Builder, profile, &[], &config);
            assert!(
                !tools.iter().any(|t| t.name.starts_with("doc_")),
                "{profile:?}"
            );
        }
    }

    #[test]
    fn a_standard_brain_keeps_its_plan_tools_in_the_core() {
        let (_, system, tools) = prefix(&AgentConfig::default(), SessionMode::Brain, &[], &[]);
        let sent = names(&tools);
        assert!(sent.contains(&"write_plan") && sent.contains(&"exit_plan_mode"));
        assert_eq!(sent.last(), Some(&"load_tools"));
        let listed = catalog_names(&system);
        assert!(!listed.contains(&"finalize_plan".to_string()));
        assert!(!listed.contains(&"enter_plan_mode".to_string()));
        assert!(listed.contains(&"semantic_search".to_string()));
    }

    /// A rule that makes a tool the session's first or mandatory call must
    /// name a tool the request carries. Ordering a deferred one makes nearly
    /// every session spend a `tools_loaded` cache break on it before any work
    /// — against the plan's ≤ 2 breaks per session — and tells the model to
    /// call something its tool list does not define.
    #[test]
    fn no_standard_prompt_line_makes_a_deferred_tool_mandatory_or_first() {
        let config = AgentConfig::default();
        for mode in [SessionMode::Builder, SessionMode::Brain] {
            for golden in [false, true] {
                let (_, system, _) = run_prefix(
                    &config,
                    &crate::agent::tools::tests_support::ctx(),
                    PromptProfile::Standard,
                    mode,
                    None,
                    None,
                    &[],
                    golden,
                    &[],
                );
                let deferred = deferred_tools(mode, PromptProfile::Standard, &[], &config);
                for line in system.lines() {
                    // "call `x`" is an order wherever it stands.
                    for t in &deferred {
                        assert!(
                            !line.contains(&format!("call `{}`", t.name)),
                            "{mode:?} (golden {golden}) orders deferred `{}`: {line}",
                            t.name
                        );
                    }
                    // A last step is as mandatory as a first one: a load at
                    // the end of a session rewrites the cache in front of the
                    // whole conversation.
                    if !(line.contains("MUST")
                        || line.contains("FIRST")
                        || line.contains("first call")
                        || line.contains("LAST")
                        || line.contains("required"))
                    {
                        continue;
                    }
                    for t in &deferred {
                        assert!(
                            !line.contains(&format!("`{}`", t.name)),
                            "{mode:?} (golden {golden}) requires deferred `{}`: {line}",
                            t.name
                        );
                    }
                }
            }
        }
    }

    /// Whether `text` names `tool` as a word of its own (not as part of a
    /// longer identifier such as `mcp__x__browser_tabs`).
    fn names_tool(text: &str, tool: &str) -> bool {
        let ident = |c: char| c.is_ascii_alphanumeric() || c == '_';
        text.match_indices(tool).any(|(at, _)| {
            let before = text[..at].chars().next_back();
            let after = text[at + tool.len()..].chars().next();
            !before.is_some_and(ident) && !after.is_some_and(ident)
        })
    }

    /// Every text the harness itself puts in front of the model is an order —
    /// a nudge, a kickoff, the handoff request — so none may name a tool that
    /// is deferred in either mode. The model would load it to obey, and a
    /// load mid-session (the finalize_plan nudge comes at the very end)
    /// rewrites the cached prefix in front of the whole conversation.
    ///
    /// The fixed texts come from `nudges::ALL`, which a test in `nudges`
    /// keeps equal to the file's constants; the composed ones (the
    /// functions in `nudges` and the kickoffs in `transition`) are listed
    /// here and must grow with them. The `<hook-feedback>` and
    /// `<hook-context>` wrappers add only their tags to a hook's own text.
    #[test]
    fn no_harness_text_orders_a_deferred_tool() {
        use crate::agent::{nudges, transition};
        let config = AgentConfig::default();
        let mut deferred: Vec<String> = Vec::new();
        for mode in [SessionMode::Builder, SessionMode::Brain] {
            for t in deferred_tools(mode, PromptProfile::Standard, &[], &config) {
                if !deferred.contains(&t.name) {
                    deferred.push(t.name);
                }
            }
        }
        assert!(
            deferred.iter().any(|n| n == "semantic_search"),
            "{deferred:?}"
        );
        let mut texts: Vec<String> = nudges::ALL.iter().map(|(_, t)| t.to_string()).collect();
        texts.extend([
            nudges::golden_flip(&["g1".into()], "builder", 1, 3),
            nudges::quality_failed("1 failing", "detail", 1, 3),
            nudges::repeating("`read_file`"),
            nudges::still_doing(&[crate::agent::persist::TaskItem {
                id: "t1".into(),
                title: "wire the UI".into(),
                description: String::new(),
                journal: vec![],
                status: "doing".into(),
            }]),
            transition::compose_builder_kickoff(Some("docs/plans/p.md"), Some("plan")),
            transition::compose_builder_kickoff(None, None),
            transition::compose_context_handoff_message("handoff", Some("docs/plans/p.md")),
        ]);
        // The gates' refusals: since prompt v2 they are the instruction.
        texts.extend(crate::agent::gate_text::all().into_iter().map(|(_, t)| t));
        for text in &texts {
            for name in &deferred {
                assert!(
                    !names_tool(text, name),
                    "a harness text orders deferred `{name}`: {text}"
                );
            }
        }
    }

    /// The lines of `text` that make the model spend a round on the task
    /// list alone (lean-harness Phase 4): fetching it first, a standalone
    /// `doing` before the work, a final update before the answer, or a ban
    /// on batching.
    fn mandated_task_round_trips(text: &str) -> Vec<String> {
        text.lines()
            .filter(|line| {
                let low = line.to_lowercase();
                (low.contains("tasks_get") && (low.contains("first") || low.contains("call ")))
                    || low.contains("never batch")
                    || low.contains("final `tasks_update`")
                    || low.contains("final tasks_update")
                    || (low.contains("doing") && low.contains("before"))
            })
            .map(str::to_string)
            .collect()
    }

    #[test]
    fn the_round_trip_detector_finds_the_old_mandates() {
        for old in [
            "- You MUST call `tasks_get` first.",
            "1. Call `tasks_get` FIRST to load the task list.",
            "strictly follow `todo` -> `doing` -> `done`. Never batch updates.",
            "- Before your final text response, make a final `tasks_update` (or `tasks_set`) call.",
            "  - Mark it 'doing' BEFORE touching code.",
            "- Move each task with `tasks_update`: 'doing' before you start it.",
        ] {
            assert_eq!(mandated_task_round_trips(old).len(), 1, "{old}");
        }
    }

    /// The gate of Phase 4: no prompt, kickoff or nudge mandates a round
    /// spent on the task list. The list comes with the run (`<tasks>`),
    /// `doing` rides with the first tool call of the task, and status
    /// changes are batched.
    #[test]
    fn no_prompt_mandates_a_task_round_trip() {
        use crate::agent::{nudges, transition};
        let config = AgentConfig::default();
        let mut texts: Vec<(String, String)> = Vec::new();
        for mode in [SessionMode::Builder, SessionMode::Brain] {
            for golden in [false, true] {
                let (_, system, _) = run_prefix(
                    &config,
                    &crate::agent::tools::tests_support::ctx(),
                    PromptProfile::Standard,
                    mode,
                    None,
                    None,
                    &[],
                    golden,
                    &[],
                );
                texts.push((format!("{mode:?} golden={golden}"), system));
            }
        }
        texts.push((
            "compact".into(),
            crate::agent::surface::COMPACT_PROMPT.to_string(),
        ));
        for (name, text) in [
            (
                "builder kickoff",
                transition::compose_builder_kickoff(Some("p.md"), Some("plan")),
            ),
            (
                "builder kickoff, no plan",
                transition::compose_builder_kickoff(None, None),
            ),
            (
                "context handoff",
                transition::compose_context_handoff_message("doc", Some("p.md")),
            ),
            (
                "golden flip",
                nudges::golden_flip(&["g".into()], "builder", 1, 3),
            ),
            ("finalize", nudges::FINALIZE_PLAN.to_string()),
            (
                "still doing",
                nudges::still_doing(&[crate::agent::persist::TaskItem {
                    id: "t1".into(),
                    title: "wire the UI".into(),
                    description: String::new(),
                    journal: vec![],
                    status: "doing".into(),
                }]),
            ),
            ("handoff request", nudges::HANDOFF_REQUEST.to_string()),
        ] {
            texts.push((name.into(), text));
        }
        let found: Vec<(String, Vec<String>)> = texts
            .into_iter()
            .map(|(name, text)| (name, mandated_task_round_trips(&text)))
            .filter(|(_, lines)| !lines.is_empty())
            .collect();
        assert!(found.is_empty(), "mandated task round-trips: {found:#?}");
    }

    /// What replaced the mandates: the prompt says the list comes with the
    /// run, that `doing` goes with the first tool call, that changes are
    /// batched, and that a one-step change needs no list.
    #[test]
    fn the_standard_prompt_says_how_tasks_work_without_round_trips() {
        for version in PromptVersion::ALL {
            for mode in [SessionMode::Builder, SessionMode::Brain] {
                let sys = system_prompt(
                    Some("/ws"),
                    None,
                    None,
                    None,
                    mode,
                    PromptProfile::Standard,
                    subagent::MAX_PARALLEL_AGENTS,
                    false,
                    version,
                );
                for said in [
                    "`<tasks>`",
                    "in the same round as the first tool call",
                    "one-step change",
                    "`updates`",
                ] {
                    assert!(sys.contains(said), "{mode:?} does not say {said:?}");
                }
            }
            let compact = crate::agent::surface::COMPACT_PROMPT;
            assert!(compact.contains("`<tasks>`") && compact.contains("same round"));
        }
    }

    /// The living document (Phase 4b) in a few lines of the Standard prompt:
    /// when to start one, how it is filled and delegated, that the user's
    /// edits win, and that a new top-level topic is a new document in a new
    /// session. Short: it is in every request of every session.
    ///
    /// v1 only since the single-timeline plan (Phase 2): v2 merged TASKS and
    /// LIVING DOCUMENT into one PLAN section on purpose — "several distinct
    /// parts" matched a multi-step task list too, and the maintainer's
    /// session kept both. v2's rule is pinned by the next test.
    #[test]
    fn the_standard_prompt_says_when_to_start_a_document_and_that_the_users_edits_win() {
        for version in [PromptVersion::V1] {
            for mode in [SessionMode::Builder, SessionMode::Brain] {
                let sys = system_prompt(
                    Some("/ws"),
                    None,
                    None,
                    None,
                    mode,
                    PromptProfile::Standard,
                    subagent::MAX_PARALLEL_AGENTS,
                    false,
                    version,
                );
                let start = sys.find("LIVING DOCUMENT").expect("a document section");
                let section =
                    &sys[start..sys[start..].find("\n# ").map_or(sys.len(), |e| start + e)];
                for said in [
                    "several distinct parts",
                    "`doc_create`",
                    "`doc_write`",
                    "`section`",
                    "edits win",
                    "new session",
                    "`<document>`",
                ] {
                    assert!(
                        section.contains(said),
                        "{mode:?} does not say {said:?}: {section}"
                    );
                }
                let words = section.split_whitespace().count();
                assert!(words <= 120, "{words} words");
            }
        }
    }

    /// One plan per session, sized to the request (single-timeline plan,
    /// Phase 2): none for one step, a task list for multi-step code work, a
    /// short document of independent questions for research, design or a
    /// write-up — never both — and the document's sections go to subagents
    /// in one call. One section, in every request: kept short.
    #[test]
    fn the_v2_prompt_has_one_plan_sized_to_the_request() {
        for mode in [SessionMode::Builder, SessionMode::Brain] {
            let sys = system_prompt(
                Some("/ws"),
                None,
                None,
                None,
                mode,
                PromptProfile::Standard,
                subagent::MAX_PARALLEL_AGENTS,
                false,
                PromptVersion::V2,
            );
            assert!(
                !sys.contains("# TASKS") && !sys.contains("LIVING DOCUMENT"),
                "{sys}"
            );
            let start = sys.find("# PLAN").expect("a plan section");
            let section = &sys[start
                ..sys[start + 1..]
                    .find("\n# ")
                    .map_or(sys.len(), |e| start + 1 + e)];
            for said in [
                "one-step change needs none",
                "`tasks_set`",
                "`doc_create`",
                "never both",
                "3-6 sections",
                "independent question",
                "one `spawn_agents` call",
                "`section`",
                "`<tasks>`",
                "`<document>`",
                "edits win",
                "new session",
            ] {
                assert!(
                    section.contains(said),
                    "{mode:?} does not say {said:?}: {section}"
                );
            }
            let words = section.split_whitespace().count();
            assert!(words <= 210, "{words} words");
        }
    }

    /// A feature that is off takes its tools out of the catalog as well as
    /// the list: a line for a tool that cannot run is a failed call waiting.
    #[test]
    fn a_tool_whose_feature_is_off_is_neither_sent_nor_listed() {
        let mut cfg = AgentConfig::default();
        cfg.browser.enabled = false;
        let (_, system, tools) = prefix(&cfg, SessionMode::Builder, &[], &[]);
        let listed = catalog_names(&system);
        for gone in [
            "browser",
            "browser_inspect",
            "browser_screenshot",
            "web_search",
        ] {
            assert!(!listed.contains(&gone.to_string()), "{gone} is listed");
            assert!(!names(&tools).contains(&gone), "{gone} is sent");
        }
        let (_, system, tools) = prefix(&cfg, SessionMode::Builder, &[], &["browser"]);
        assert!(!names(&tools).contains(&"browser"), "loaded but off");
        assert!(!system.contains("- browser "));
    }

    #[test]
    fn every_mcp_tool_is_deferred() {
        let defs = [mcp("mcp__gh__issue"), mcp("mcp__gh__search")];
        let (_, system, tools) = prefix(&AgentConfig::default(), SessionMode::Builder, &defs, &[]);
        assert!(names(&tools).iter().all(|n| !n.starts_with("mcp__")));
        let listed = catalog_names(&system);
        assert!(listed.ends_with(&["mcp__gh__issue".to_string(), "mcp__gh__search".to_string()]));
    }

    /// The point of the design: loading grows the tool list at its end and
    /// moves nothing else — the system prompt is the same bytes it was.
    #[test]
    fn loaded_tools_join_the_end_of_the_list_and_the_system_prompt_does_not_move() {
        let defs = [mcp("mcp__gh__issue"), mcp("mcp__gh__search")];
        let cfg = AgentConfig::default();
        let (_, before_system, before) = prefix(&cfg, SessionMode::Builder, &defs, &[]);
        let (_, system, tools) = prefix(
            &cfg,
            SessionMode::Builder,
            &defs,
            &["mcp__gh__search", "semantic_search"],
        );
        assert_eq!(system, before_system);
        assert_eq!(names(&tools)[..before.len()], names(&before)[..]);
        assert_eq!(
            names(&tools)[before.len()..],
            ["mcp__gh__search", "semantic_search"]
        );
        // The definition sent is the tool's own, schema and all.
        let sem = tools.iter().find(|t| t.name == "semantic_search").unwrap();
        assert!(sem.input_schema["properties"]["query"].is_object());
        // And it comes out the same twice.
        let again = prefix(
            &cfg,
            SessionMode::Builder,
            &defs,
            &["mcp__gh__search", "semantic_search"],
        );
        assert_eq!(
            serde_json::to_string(&again.2).unwrap(),
            serde_json::to_string(&tools).unwrap()
        );
    }

    /// A tool loaded in Builder that Brain does not defer stays out of Brain's
    /// list, and comes back when the session returns to Builder.
    #[test]
    fn a_loaded_tool_the_mode_does_not_offer_is_left_out() {
        let cfg = AgentConfig::default();
        let (_, _, brain) = prefix(&cfg, SessionMode::Brain, &[], &["enter_plan_mode"]);
        assert!(!names(&brain).contains(&"enter_plan_mode"));
        let (_, _, builder) = prefix(&cfg, SessionMode::Builder, &[], &["enter_plan_mode"]);
        assert_eq!(names(&builder).last(), Some(&"enter_plan_mode"));
    }
}

/// The Phase 2 gate of the lean-harness plan
/// (`docs/plans/2026-10-18_lean-harness-audit-and-plan.md`): what a Standard
/// Builder run sends ahead of the conversation, on the harness's own meter
/// (`session::estimate_tokens`, chars/3 — the number the context meter and
/// `run_config.prefix` use). The prefix is re-sent on every request; on a
/// caching provider it is a cache read each round and a cold write on the
/// first, so its size is paid in every session whether or not a tool or skill
/// in it is ever used.
#[cfg(test)]
mod prefix_budget_tests {
    use super::*;
    use crate::agent::session::estimate_tokens;

    /// A fresh install: the three built-in skills (find-skills, plan,
    /// plugin-crafter), no MCP server.
    const DEFAULT_TARGET: u64 = 5_000;
    /// Ten skills and one MCP server of twenty tools.
    const LOADED_TARGET: u64 = 10_000;

    fn ctx() -> ToolContext {
        ToolContext {
            workspace_root: Some("/home/user/projects/app".into()),
            ..crate::agent::tools::tests_support::ctx()
        }
    }

    fn builtin_skills() -> Option<String> {
        crate::agent::skills::build_skills_system_prompt_section(
            &crate::agent::skills::SkillManager::builtin_only(),
        )
    }

    /// Ten skills with descriptions of the length real ones have (the
    /// frontmatter allows up to 1,024 characters; the ones installed in the
    /// wild run 150-400).
    fn ten_skills() -> Option<String> {
        let mut mgr = crate::agent::skills::SkillManager::builtin_only();
        for i in 0..10 {
            mgr.insert_for_test(crate::agent::skills::SkillEntry {
                name: format!("skill-{i:02}"),
                description: format!(
                    "Skill number {i}: use when the user asks to deploy, release, lint, \
                     format, migrate or document part of the project, or mentions the \
                     tooling this skill wraps. Covers setup, the commands, verification \
                     and rollback, with references for the edge cases."
                ),
                location: format!("/home/user/.claudinio/skills/skill-{i:02}/SKILL.md"),
                scope: crate::agent::skills::SkillScope::User,
                body: Some("Body.".into()),
            });
        }
        crate::agent::skills::build_skills_system_prompt_section(&mgr)
    }

    /// One MCP server of twenty tools, each the size of a typical server's
    /// definition: a two-sentence description and a three-property schema.
    fn twenty_mcp_tools() -> Vec<tools::ToolDef> {
        (0..20)
            .map(|i| tools::ToolDef {
                name: format!("mcp__tracker__tool_{i:02}"),
                description: format!(
                    "Operation {i} of the issue tracker: lists, creates or updates issues, \
                     comments and labels in the configured workspace. Returns the affected \
                     records as JSON, paginated by cursor when there are more than fifty."
                ),
                input_schema: serde_json::json!({
                    "type": "object",
                    "properties": {
                        "project": {"type": "string", "description": "Project key, e.g. ENG"},
                        "query": {"type": "string", "description": "Free-text filter"},
                        "cursor": {"type": "string", "description": "Pagination cursor from a previous call"}
                    },
                    "required": ["project"]
                }),
            })
            .collect()
    }

    fn builder_prefix(skills: Option<&str>, mcp: &[tools::ToolDef]) -> u64 {
        let (profile, system, tools) = run_prefix(
            &AgentConfig::default(),
            &ctx(),
            PromptProfile::Standard,
            SessionMode::Builder,
            skills,
            None,
            mcp,
            false,
            &[],
        );
        assert_eq!(profile, PromptProfile::Standard);
        let total = estimate_tokens(&[], &system, &tools);
        eprintln!(
            "builder prefix: {total} tokens (system {} chars, tools {} chars, {} tools)",
            system.len(),
            serde_json::to_string(&tools).unwrap().len(),
            tools.len()
        );
        total
    }

    #[test]
    fn the_default_builder_prefix_fits_five_thousand_tokens() {
        let total = builder_prefix(builtin_skills().as_deref(), &[]);
        assert!(
            total <= DEFAULT_TARGET,
            "default Builder prefix is {total} tokens, over {DEFAULT_TARGET}"
        );
    }

    #[test]
    fn ten_skills_and_a_twenty_tool_mcp_server_fit_ten_thousand_tokens() {
        let total = builder_prefix(ten_skills().as_deref(), &twenty_mcp_tools());
        assert!(
            total <= LOADED_TARGET,
            "Builder prefix with 10 skills and 20 MCP tools is {total} tokens, over {LOADED_TARGET}"
        );
    }
}

/// Prompt version v1 is the Standard prompt as it stood when Phase 5 of the
/// lean-harness plan began (`b6e0cf0`), kept selectable for one release
/// (decision D7) so a user who finds the rewrite worse can flip back. The
/// fixtures were written by that commit's `system_prompt`; "v1" means those
/// bytes, so any edit to a v1 text — even a typo fix — fails here.
#[cfg(test)]
mod v1_pin_tests {
    use super::*;

    const FIXTURES: [(&str, &str); 6] = [
        ("builder", include_str!("prompt/v1_fixtures/builder.txt")),
        (
            "builder_golden",
            include_str!("prompt/v1_fixtures/builder_golden.txt"),
        ),
        ("brain", include_str!("prompt/v1_fixtures/brain.txt")),
        (
            "brain_golden",
            include_str!("prompt/v1_fixtures/brain_golden.txt"),
        ),
        (
            "brain_no_root_custom_plans",
            include_str!("prompt/v1_fixtures/brain_no_root_custom_plans.txt"),
        ),
        (
            "builder_no_root_custom_plans",
            include_str!("prompt/v1_fixtures/builder_no_root_custom_plans.txt"),
        ),
    ];

    fn v1(name: &str) -> String {
        let at = |mode, golden| {
            system_prompt(
                Some("/ws"),
                None,
                None,
                None,
                mode,
                PromptProfile::Standard,
                4,
                golden,
                PromptVersion::V1,
            )
        };
        let elsewhere = |mode| {
            system_prompt(
                None,
                None,
                None,
                Some("docs/plans"),
                mode,
                PromptProfile::Standard,
                6,
                false,
                PromptVersion::V1,
            )
        };
        match name {
            "builder" => at(SessionMode::Builder, false),
            "builder_golden" => at(SessionMode::Builder, true),
            "brain" => at(SessionMode::Brain, false),
            "brain_golden" => at(SessionMode::Brain, true),
            "brain_no_root_custom_plans" => elsewhere(SessionMode::Brain),
            "builder_no_root_custom_plans" => elsewhere(SessionMode::Builder),
            other => panic!("no fixture {other}"),
        }
    }

    #[test]
    fn prompt_v1_is_byte_identical_to_the_prompt_before_phase_5() {
        for (name, fixture) in FIXTURES {
            assert!(v1(name) == fixture, "v1 {name} drifted from its fixture");
        }
    }
}

/// `AgentConfig::prompt_version` (decision D7): which Standard prompt a
/// session is sent. v2 is the default; v1 stays selectable for one release.
/// The config file is the user's, and `load_config` falls back to an empty
/// config when it does not parse, so a value this build does not know must
/// not cost the user every other setting.
#[cfg(test)]
mod prompt_version_tests {
    use crate::agent::provider::AgentConfig;

    fn load(json: &str) -> AgentConfig {
        serde_json::from_str(json).expect("the config parses")
    }

    #[test]
    fn a_config_written_before_the_setting_gets_prompt_v2() {
        let cfg = load(r#"{"base_url": "u", "api_key": "k"}"#);
        assert_eq!(serde_json::to_value(&cfg).unwrap()["prompt_version"], "v2");
        assert_eq!(
            serde_json::to_value(AgentConfig::default()).unwrap()["prompt_version"],
            "v2"
        );
    }

    #[test]
    fn prompt_version_round_trips_through_the_config_file() {
        for version in ["v1", "v2"] {
            let cfg = load(&format!(
                r#"{{"base_url": "u", "api_key": "k", "prompt_version": "{version}"}}"#
            ));
            let saved = serde_json::to_string(&cfg).unwrap();
            let again = load(&saved);
            assert_eq!(
                serde_json::to_value(&again).unwrap()["prompt_version"],
                version
            );
        }
    }

    #[test]
    fn an_unknown_prompt_version_reads_as_v2_and_keeps_every_other_setting() {
        let cfg =
            load(r#"{"base_url": "u", "api_key": "k", "model": "b", "prompt_version": "v9"}"#);
        assert_eq!(cfg.model, "b");
        assert_eq!(serde_json::to_value(&cfg).unwrap()["prompt_version"], "v2");
    }

    /// Settings › Agent sends "v1" or "v2". Unlike the lenient file reader,
    /// the setter changes nothing on a value it does not know: a typo from
    /// the UI must not silently switch a user who chose v1 back to v2.
    #[test]
    fn the_settings_control_switches_the_version_and_ignores_an_unknown_value() {
        use super::{PromptVersion, apply_prompt_version_setting};
        let mut cfg = AgentConfig::default();
        apply_prompt_version_setting(&mut cfg, Some("v1"));
        assert_eq!(cfg.prompt_version, PromptVersion::V1);
        apply_prompt_version_setting(&mut cfg, Some("v9"));
        assert_eq!(cfg.prompt_version, PromptVersion::V1);
        apply_prompt_version_setting(&mut cfg, None);
        assert_eq!(cfg.prompt_version, PromptVersion::V1);
        apply_prompt_version_setting(&mut cfg, Some("v2"));
        assert_eq!(cfg.prompt_version, PromptVersion::V2);
    }
}

/// Phase 5 of the lean-harness plan: the v2 Standard prompt at the right
/// altitude. Budgets are on the harness's own meter
/// (`session::estimate_tokens`, chars/3), the one `run_config.prefix` and the
/// context bar use.
#[cfg(test)]
mod v2_tests {
    use super::*;
    use crate::agent::session::estimate_tokens;

    const ROOT: &str = "/home/user/projects/app";

    fn tokens(text: &str) -> u64 {
        estimate_tokens(&[], text, &[])
    }

    fn block(text: &str) -> String {
        text.replace("{plans_subdir}", ".claudinio/plans")
    }

    /// Identity, environment, how the tools relate, when to ask and stop,
    /// untrusted content, hook output, the living document and the language
    /// policy, workspace line included.
    #[test]
    fn the_v2_base_prompt_fits_fifteen_hundred_tokens() {
        let base = with_workspace(texts(PromptVersion::V2).base, Some(ROOT))
            .replace("{max_parallel}", "4");
        let total = tokens(&base);
        eprintln!("v2 base: {total} tokens ({} chars)", base.len());
        assert!(total <= 1_500, "v2 base prompt is {total} tokens");
    }

    #[test]
    fn each_v2_mode_block_fits_six_hundred_tokens() {
        let t = texts(PromptVersion::V2);
        for (name, text) in [("brain", t.brain), ("builder", t.builder)] {
            let total = tokens(&block(text));
            eprintln!("v2 {name} block: {total} tokens");
            assert!(total <= 600, "v2 {name} block is {total} tokens");
        }
    }

    /// The MUST / NEVER / MANDATORY / STRICT register the plan's design rule
    /// 4 retires: in prose, any word in capitals outside a code span that is
    /// not an acronym; in a heading — where capitals are the style — only the
    /// imperatives themselves.
    fn capitalised_words(text: &str) -> Vec<String> {
        const ACRONYMS: [&str; 13] = [
            "AI", "UI", "UX", "URL", "SVG", "API", "MCP", "LSP", "JSON", "PR", "ID", "OK", "LLD",
        ];
        const IMPERATIVES: [&str; 9] = [
            "MUST",
            "NEVER",
            "MANDATORY",
            "STRICT",
            "ALWAYS",
            "ONLY",
            "REQUIRED",
            "CRITICAL",
            "IMPORTANT",
        ];
        let mut found = Vec::new();
        for line in text.lines() {
            let heading = line.trim_start().starts_with('#');
            let prose: String = line
                .split('`')
                .enumerate()
                .filter(|(i, _)| i % 2 == 0)
                .map(|(_, part)| part)
                .collect::<Vec<_>>()
                .join(" ");
            for word in prose.split(|c: char| !c.is_ascii_alphabetic()) {
                let shouted = word.len() >= 2 && word.chars().all(|c| c.is_ascii_uppercase());
                let counts = if heading {
                    IMPERATIVES.contains(&word)
                } else {
                    !ACRONYMS.contains(&word)
                };
                if shouted && counts {
                    found.push(word.to_string());
                }
            }
        }
        found
    }

    #[test]
    fn the_capital_detector_finds_the_old_register() {
        let v1 = texts(PromptVersion::V1);
        let all = [v1.base, v1.golden, v1.brain, v1.builder].concat();
        let words = capitalised_words(&all);
        for old in ["MUST", "MANDATORY", "VERIFIED", "REJECT"] {
            assert!(words.iter().any(|w| w == old), "{old} not found: {words:?}");
        }
        assert!(capitalised_words("## TASK SYSTEM\nuse `MUST_X`, a UI and a URL").is_empty());
        assert_eq!(capitalised_words("## GOALS (MANDATORY)"), ["MANDATORY"]);
    }

    /// Design rule 4: an imperative in capitals stays only where it fixes an
    /// observed failure, and `v2::KEPT_IMPERATIVES` names that failure.
    #[test]
    fn v2_keeps_at_most_three_capitalised_imperatives_each_with_its_failure() {
        let t = texts(PromptVersion::V2);
        let all = [t.base, t.golden, t.brain, t.builder].concat();
        let words = capitalised_words(&all);
        assert!(
            words.len() <= 3,
            "{} capitalised words: {words:?}",
            words.len()
        );
        for word in &words {
            let kept = v2::KEPT_IMPERATIVES.iter().find(|(w, _)| w == word);
            assert!(
                kept.is_some_and(|(_, why)| !why.trim().is_empty()),
                "{word} is in capitals with no failure cited in KEPT_IMPERATIVES"
            );
        }
    }

    /// The gates the code enforces teach through their error messages
    /// (`gate_text::tests` pins each one's next step); v2 says once that
    /// they exist and keeps none of the sentences that described them.
    #[test]
    fn v2_describes_no_gate_the_code_enforces() {
        let t = texts(PromptVersion::V2);
        let all = [t.base, t.golden, t.brain, t.builder].concat();
        for described in [
            "code-gated",
            "returns an error",
            "REJECT",
            "rejects tasks until",
            "Never delete golden tasks",
            "automatically switches mode",
            "re-checks it before",
            "You must never implement",
            "Stating that the tests pass has no effect",
            // The quality gate; the base line names it ("closing a goal
            // without a passing check") and its refusal says to run it.
            "needs a passing `run_quality`",
        ] {
            assert!(
                !all.contains(described),
                "v2 still describes a gate: {described:?}"
            );
        }
        let gate_lines: Vec<&str> = t
            .base
            .lines()
            .filter(|l| l.contains("refuse") && l.contains("error"))
            .collect();
        assert_eq!(gate_lines.len(), 1, "{gate_lines:?}");
        assert!(
            gate_lines[0].contains("what to do next"),
            "{}",
            gate_lines[0]
        );
    }

    /// The tool list is prefix too, in both modes: `tasks_set` carried the
    /// Brain-only Low-Level Design gate in its description, paid by every
    /// Builder request, while the gate's refusal (`gate_text::lld_missing`)
    /// says what to write and then to retry.
    #[test]
    fn no_core_tool_description_restates_the_lld_gate() {
        for def in crate::agent::tools::get_defs(4) {
            let d = def.description.to_lowercase();
            assert!(
                !(d.contains("refused until") || d.contains("rejects tasks until")),
                "{} restates a gate: {}",
                def.name,
                def.description
            );
        }
    }

    /// The spec section rides in the prefix of every project with Gherkin
    /// specs. Under v2 it states no gate (the spec guard refuses an edit and
    /// its refusal says to ask the user: `quality::spec::edit_refusal`) and
    /// adds no capitals; v1 keeps its text.
    #[test]
    fn the_v2_spec_section_leaves_the_spec_guard_to_its_refusal() {
        let root = std::env::temp_dir().join(format!("cq-specv2-{}", std::process::id()));
        std::fs::remove_dir_all(&root).ok();
        std::fs::create_dir_all(root.join("features")).unwrap();
        std::fs::write(
            root.join("features/a.feature"),
            "Feature: Checkout\n  Scenario: Pay by card\n    Then it is paid\n",
        )
        .unwrap();
        let mut ctx = crate::agent::tools::tests_support::ctx();
        ctx.workspace_root = Some(root.to_string_lossy().to_string());
        let v1 = build_spec_prompt_section(&ctx, PromptVersion::V1).unwrap();
        let v2 = build_spec_prompt_section(&ctx, PromptVersion::V2).unwrap();
        std::fs::remove_dir_all(&root).ok();
        assert!(v1.contains("You CANNOT edit them"), "{v1}");
        assert!(v2.contains("Pay by card"), "{v2}");
        assert!(v2.contains("the user"), "{v2}");
        assert!(!v2.contains("refuses any path"), "{v2}");
        assert!(
            capitalised_words(&v2).is_empty(),
            "{:?}",
            capitalised_words(&v2)
        );
    }

    /// Decision D5: ask only what changes the plan, bundle independent
    /// questions, no interview for a fully specified request, one confirming
    /// question before the plan is written — and the checklists live in the
    /// `plan` skill, named in one line.
    #[test]
    fn the_v2_brain_interview_is_adaptive_and_points_at_the_plan_skill() {
        let brain = texts(PromptVersion::V2).brain;
        for said in [
            "changes the plan",
            "one `ask_user` call",
            "fully specified request needs no interview",
            "Confirm the design in one question",
        ] {
            assert!(brain.contains(said), "v2 Brain does not say {said:?}");
        }
        for gone in [
            "one question at a time",
            "Batching questions is confusing",
            "never skip",
        ] {
            assert!(!brain.contains(gone), "v2 Brain still says {gone:?}");
        }
        let skill_lines = brain.lines().filter(|l| l.contains("`plan` skill")).count();
        assert_eq!(skill_lines, 1, "{brain}");
        for moved in [
            "false consensus",
            "Sizing and layout",
            "FULL Mermaid catalog",
        ] {
            assert!(
                !brain.contains(moved),
                "{moved:?} belongs in the plan skill"
            );
        }
    }

    /// The setting reaches the run: the default config sends v2, and a config
    /// set to v1 sends the v1 texts (pinned byte for byte in `v1_pin_tests`).
    #[test]
    fn the_run_prefix_follows_the_prompt_version_setting() {
        let ctx = crate::agent::tools::tests_support::ctx();
        let system_for = |version: PromptVersion, mode| {
            let config = AgentConfig {
                prompt_version: version,
                ..AgentConfig::default()
            };
            run_prefix(
                &config,
                &ctx,
                PromptProfile::Standard,
                mode,
                None,
                None,
                &[],
                false,
                &[],
            )
            .1
        };
        for mode in [SessionMode::Brain, SessionMode::Builder] {
            let v1 = system_for(PromptVersion::V1, mode);
            let v2 = system_for(PromptVersion::V2, mode);
            let default = run_prefix(
                &AgentConfig::default(),
                &ctx,
                PromptProfile::Standard,
                mode,
                None,
                None,
                &[],
                false,
                &[],
            )
            .1;
            assert_eq!(default, v2, "{mode:?}: the default is v2");
            assert!(v1.contains("UI Mandate: The Task Panel"), "{mode:?}");
            assert!(!v2.contains("UI Mandate"), "{mode:?}");
        }
    }
}
