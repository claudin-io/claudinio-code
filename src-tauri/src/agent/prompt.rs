//! What a run sends ahead of the conversation: the system prompt for each
//! profile, and the tool list beside it.
//!
//! Both are derived together (`run_prefix`) and never change between rounds,
//! because they are the cached prefix: anything per-round here
//! (a timestamp, a map's iteration order) would turn every request cold.

use crate::agent::provider::{AgentConfig, ToolDescription};
use crate::agent::session::PromptProfile;
use crate::agent::subagent;
use crate::agent::tools::{self, ToolContext};

mod standard;

/// Prompt budget for the scenario index. Enough for a real spec suite, small
/// enough that a thousand scenarios cannot crowd out the conversation.
const MAX_SPEC_SECTION_CHARS: usize = 4_000;

/// Lean, single-purpose prompt for the `GitSync` profile (commit & push).
/// No task system, no skills/subagents — the model's
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
pub(super) fn build_spec_prompt_section(ctx: &ToolContext) -> Option<String> {
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
    // The spec guard is code (`tools::validate_editable_path`), and its
    // refusal says to ask the user; the prompt does not restate it.
    Some(format!(
        "\n## SPECIFICATION (Gherkin — the user's requirement)\n\
         This project has executable specs: the requirement, and the one input here that \
         did not come from a model. Plan and implement to satisfy these scenarios, and name \
         the ones each task covers, so what is left stays visible. A scenario that looks \
         wrong or cannot be met goes to the user; the specs are theirs to change.\n\n\
         {index}"
    ))
}

/// A chat profile's prompt `text` followed by the workspace facts, the same
/// sentences for Standard and Compact.
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

/// Build the per-session system prompt. Byte-identical for every request in
/// the same workspace so the provider's prefix cache stays warm.
pub(super) fn system_prompt(
    workspace_root: Option<&str>,
    skills_section: Option<&str>,
    // Index of the project's Gherkin scenarios, when it has any. Placed with
    // the skills block: stable per workspace, so the prefix cache only moves
    // when the specs themselves do.
    spec_section: Option<&str>,
    profile: PromptProfile,
    max_parallel: usize,
    // Whether the session has golden tasks at run start: only then are the
    // golden-task rules sent.
    has_golden_goals: bool,
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
        // No skills, no specs: on the window this profile is for, each of
        // them is a file the model can no longer read.
        return with_workspace(crate::agent::surface::COMPACT_PROMPT, workspace_root);
    }
    let base = with_workspace(standard::SYSTEM_PROMPT, workspace_root);
    let base = match skills_section {
        Some(s) if !s.is_empty() => format!("{base}\n{s}"),
        _ => base,
    };
    let base = match spec_section {
        Some(s) if !s.is_empty() => format!("{base}\n{s}"),
        _ => base,
    };
    let base = base.replace("{max_parallel}", &max_parallel.to_string());
    let golden = if has_golden_goals {
        standard::GOLDEN_PROMPT
    } else {
        ""
    };
    format!("{base}{golden}{}", standard::WORK_BLOCK)
}

/// What a run sends ahead of the conversation: the profile the
/// model's window calls for, then the prompt and tools of that profile as the
/// user's surface for the model shapes them.
///
/// One function for the run's start and for every change of profile, so the three
/// can never be derived from different inputs: a prompt that names a tool the
/// list beside it does not carry costs a failed call each time it is believed.
#[allow(clippy::too_many_arguments)]
pub(super) fn run_prefix(
    config: &AgentConfig,
    ctx: &ToolContext,
    requested: PromptProfile,
    skills_section: Option<&str>,
    spec_section: Option<&str>,
    mcp_defs: &[tools::ToolDef],
    has_golden_goals: bool,
    // Deferred tools the session has loaded, in load order (its
    // `tools_loaded` records). Appended to the tool list; ignored by the
    // profiles that defer nothing.
    loaded: &[String],
) -> (PromptProfile, String, Vec<ToolDescription>) {
    let profile = crate::agent::surface::effective_profile(requested, config, has_golden_goals);
    let system = system_prompt(
        ctx.workspace_root.as_deref(),
        skills_section,
        spec_section,
        profile,
        subagent::effective_max_parallel(config),
        has_golden_goals,
    );
    let system = if profile == PromptProfile::Standard {
        crate::agent::surface::prompt_for(config.tool_surface_for(config.model()), system)
    } else {
        system
    };
    let tools = api_tools(profile, mcp_defs, config);
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

/// The tools a run defers — what `load_tools` (or a direct call) can add to
/// its list. Empty for the profiles that defer nothing.
pub(super) fn deferred_tools(
    profile: PromptProfile,
    mcp_defs: &[tools::ToolDef],
    config: &AgentConfig,
) -> Vec<ToolDescription> {
    if profile != PromptProfile::Standard {
        return Vec::new();
    }
    crate::agent::deferred::split(api_tools(profile, mcp_defs, config)).1
}

/// Tools offered to the model for a profile. `GitSync` gets only `bash` +
/// `ask_user` — no task system, no subagents, no MCP tools. `Compact` gets
/// `surface::COMPACT_TOOLS`, `edit_file` included. `Standard` gets the full
/// registry, the living document's tools and `remember`, shaped by the
/// user's per-model surface.
pub(super) fn api_tools(
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
    if !profile.edits_files() {
        defs.retain(|t| t.name != "edit_file");
    }
    // The living document (Phase 4b): `doc_create` core, the
    // rest deferred and unlisted until it loads them.
    defs.extend(tools::doc::session_defs());
    // The project memory's writer: core, because the memory guard's refusal
    // points at it (a harness text never orders a deferred tool).
    defs.push(tools::remember_def());
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

    // ---- Deterministic prompt-invariant tests (no network) ----

    // Any language is valid for the model's input and output: no prompt may
    // require the model to reason, call tools or reply in one language.
    #[test]
    fn no_system_prompt_imposes_a_language() {
        for profile in [PromptProfile::Standard, PromptProfile::GitSync] {
            let sys = system_prompt(
                Some(ROOT),
                None,
                None,
                profile,
                subagent::MAX_PARALLEL_AGENTS,
                false,
            );
            assert!(!sys.contains("MUST be in English"), "{profile:?}");
            assert!(!sys.contains("default to English"), "{profile:?}");
            assert!(!sys.contains("else English"), "{profile:?}");
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
        for golden in [false, true] {
            let (profile, system, _) = run_prefix(
                &AgentConfig::default(),
                &ctx,
                PromptProfile::Standard,
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
                "with golden goals = {golden}"
            );
        }
    }

    /// The base prompt and each mode block both ended on the language
    /// policy, so a Standard session carried it twice on every request.
    #[test]
    fn the_language_policy_is_stated_once() {
        let sys = system_prompt(
            Some(ROOT),
            None,
            None,
            PromptProfile::Standard,
            subagent::MAX_PARALLEL_AGENTS,
            false,
        );
        assert_eq!(sys.matches("# LANGUAGE POLICY").count(), 1);
    }

    /// Links are rendering facts, not instructions: two lines, no examples.
    #[test]
    fn the_links_section_is_two_lines() {
        let sys = system_prompt(
            Some(ROOT),
            None,
            None,
            PromptProfile::Standard,
            subagent::MAX_PARALLEL_AGENTS,
            false,
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

    /// Status changes go through the one-task tool. A prompt that still told
    /// the model to resend the list would make the delta dead code.
    #[test]
    fn status_changes_are_pointed_at_tasks_update_in_both_modes() {
        let sys = system_prompt(
            Some(ROOT),
            None,
            None,
            PromptProfile::Standard,
            subagent::MAX_PARALLEL_AGENTS,
            false,
        );
        // Phase 4: `doing` rides with the task's first tool call rather
        // than taking a round of its own.
        assert!(sys.contains(
        "Mark a task `doing` with `tasks_update` in the same round as the first tool call for it"
        ));
        assert!(sys.contains("Never resend the list just to change a status"));
        assert!(!sys.contains("you MUST make a final `tasks_set` call"));
        let names: Vec<String> = api_tools(PromptProfile::Standard, &[], &AgentConfig::default())
            .into_iter()
            .map(|t| t.name)
            .collect();
        for tool in ["tasks_get", "tasks_set", "tasks_update"] {
            assert!(names.contains(&tool.to_string()), "lacks {tool}");
        }
        let builder = system_prompt(
            Some(ROOT),
            None,
            None,
            PromptProfile::Standard,
            subagent::MAX_PARALLEL_AGENTS,
            false,
        );
        // Phase 4: closing one task and starting the next is one call.
        assert!(
            builder.contains("closing one task and starting the next is one `tasks_update` call")
        );
        // The git profile has no task system at all.
        let git = api_tools(PromptProfile::GitSync, &[], &AgentConfig::default());
        assert!(git.iter().all(|t| !t.name.starts_with("tasks_")));
    }

    // Decision D1 (lean-harness Phase 3): the Builder prompt must not tell a
    // session that has `edit_file` it cannot edit, and must say when a
    // subagent pays: explore for broad read-only work, code only for
    // parallel changes to disjoint files; otherwise the session edits.
    #[test]
    fn the_builder_prompt_edits_directly_and_says_when_to_delegate() {
        for golden in [false, true] {
            let sys = system_prompt(
                Some(ROOT),
                None,
                None,
                PromptProfile::Standard,
                subagent::MAX_PARALLEL_AGENTS,
                golden,
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

    // Decision D1 (lean-harness Phase 3): the Builder main session edits
    // files itself; Brain stays read-only and the git operator never edits.
    #[test]
    fn a_chat_session_is_offered_edit_file_and_the_git_operator_is_not() {
        let names = |profile| -> Vec<String> {
            api_tools(profile, &[], &AgentConfig::default())
                .into_iter()
                .map(|t| t.name)
                .collect()
        };
        let edit = "edit_file".to_string();
        assert!(names(PromptProfile::Standard).contains(&edit));
        assert!(!names(PromptProfile::GitSync).contains(&edit));
    }

    #[test]
    fn builder_prompt_requires_complete_subagent_spec() {
        let sys = system_prompt(
            Some(ROOT),
            None,
            None,
            PromptProfile::Standard,
            subagent::MAX_PARALLEL_AGENTS,
            false,
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

        let section = build_spec_prompt_section(&ctx).expect("specs found");
        assert!(section.contains("Member discounts"), "{section}");
        assert!(section.contains("Ten percent off"), "{section}");

        let sys = system_prompt(
            Some("/ws"),
            None,
            Some(&section),
            PromptProfile::Standard,
            4,
            false,
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
        assert!(build_spec_prompt_section(&ctx).is_none());
        std::fs::remove_dir_all(&root).ok();
    }

    #[test]
    fn system_prompt_warns_against_similar_to_guessing() {
        let sys = system_prompt(
            Some(ROOT),
            None,
            None,
            PromptProfile::Standard,
            subagent::MAX_PARALLEL_AGENTS,
            false,
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

    #[test]
    fn git_sync_prompt_has_no_task_system_or_modes() {
        let sys = system_prompt(
            Some(ROOT),
            None,
            None,
            PromptProfile::GitSync,
            subagent::MAX_PARALLEL_AGENTS,
            false,
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
        let defs = api_tools(PromptProfile::GitSync, &[], &AgentConfig::default());
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
            PromptProfile::Standard,
            subagent::MAX_PARALLEL_AGENTS,
            false,
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

    fn prefix_json(root: &std::path::Path) -> String {
        prefix_json_with_mcp(root, &[])
    }

    fn prefix_json_with_mcp(root: &std::path::Path, mcp_defs: &[tools::ToolDef]) -> String {
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
        let first = prefix_json(&root);
        let second = prefix_json(&root);
        assert!(first.contains("zeta-deploy") && first.contains("web-build"));
        assert_eq!(first, second, "the prefix changed between builds");
        std::fs::remove_dir_all(&root).ok();
    }

    /// Skills from the plugin scope join the catalog deterministically too:
    /// when two plugins ship one name, the plugin that sorts first keeps it.
    #[test]
    fn plugin_scope_skills_are_deterministic_in_the_prefix() {
        let root = workspace("plugins");
        let first = prefix_json(&root);
        for _ in 0..3 {
            assert_eq!(first, prefix_json(&root));
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
        let a = prefix_json_with_mcp(&root, &mcp_defs(false));
        let b = prefix_json_with_mcp(&root, &mcp_defs(true));
        assert_eq!(a, b, "MCP arrival order moved the prefix");
        assert_eq!(a, prefix_json_with_mcp(&root, &mcp_defs(false)));
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
/// One timeline (single-timeline plan, Phase 3): no session is told about
/// modes, plan files or plan tools, and none is offered them.
#[cfg(test)]
mod single_timeline_tests {
    use super::*;

    #[test]
    fn no_session_hears_of_modes_or_plan_tools() {
        let cfg = AgentConfig::default();
        let ctx = crate::agent::tools::tests_support::ctx();
        for golden in [false, true] {
            let (_, system, tools) = run_prefix(
                &cfg,
                &ctx,
                PromptProfile::Standard,
                None,
                None,
                &[],
                golden,
                &[],
            );
            for word in [
                "Brain",
                "Builder",
                "CURRENT MODE",
                "plan mode",
                "write_plan",
                "finalize_plan",
            ] {
                assert!(!system.contains(word), "the prompt names {word:?}");
            }
            let names: Vec<&str> = tools.iter().map(|t| t.name.as_str()).collect();
            for tool in [
                "enter_plan_mode",
                "exit_plan_mode",
                "write_plan",
                "finalize_plan",
            ] {
                assert!(!names.contains(&tool), "{tool} is offered: {names:?}");
            }
            assert!(names.contains(&"edit_file") && names.contains(&"doc_create"));
        }
    }
}

#[cfg(test)]
mod deferred_prefix_tests {
    use super::*;

    fn names(tools: &[ToolDescription]) -> Vec<&str> {
        tools.iter().map(|t| t.name.as_str()).collect()
    }

    fn prefix(
        config: &AgentConfig,
        mcp: &[tools::ToolDef],
        loaded: &[&str],
    ) -> (PromptProfile, String, Vec<ToolDescription>) {
        let loaded: Vec<String> = loaded.iter().map(|s| s.to_string()).collect();
        run_prefix(
            config,
            &crate::agent::tools::tests_support::ctx(),
            PromptProfile::Standard,
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
        let (profile, system, tools) = prefix(&AgentConfig::default(), &[], &[]);
        assert_eq!(profile, PromptProfile::Standard);
        // `edit_file` is core in Builder since decision D1 (Phase 3);
        // `tasks_get` is deferred since Phase 4 (the run recites the list);
        // `doc_create` is core since Phase 4b (the living document);
        // `remember` since one timeline (the memory's one writer).
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
                "doc_create",
                "remember",
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
        let (_, system, tools) = prefix(&config, &[], &[]);
        let sent = names(&tools);
        assert!(sent.contains(&"doc_create"), "{sent:?}");
        for t in crate::agent::deferred::DOCUMENT_TOOLS {
            assert!(!sent.contains(t), "sends {t} with no document");
            assert!(!catalog_names(&system).contains(&t.to_string()));
            assert!(crate::agent::deferred::is_deferred(t));
        }
        let loaded = crate::agent::deferred::DOCUMENT_TOOLS;
        let (_, _, tools) = prefix(&config, &[], loaded);
        assert_eq!(names(&tools)[tools.len() - 3..], *loaded);
        for profile in [PromptProfile::Compact, PromptProfile::GitSync] {
            let tools = api_tools(profile, &[], &config);
            assert!(
                !tools.iter().any(|t| t.name.starts_with("doc_")),
                "{profile:?}"
            );
        }
    }

    /// A rule that makes a tool the session's first or mandatory call must
    /// name a tool the request carries. Ordering a deferred one makes nearly
    /// every session spend a `tools_loaded` cache break on it before any work
    /// — against the plan's ≤ 2 breaks per session — and tells the model to
    /// call something its tool list does not define.
    #[test]
    fn no_standard_prompt_line_makes_a_deferred_tool_mandatory_or_first() {
        let config = AgentConfig::default();
        for golden in [false, true] {
            let (_, system, _) = run_prefix(
                &config,
                &crate::agent::tools::tests_support::ctx(),
                PromptProfile::Standard,
                None,
                None,
                &[],
                golden,
                &[],
            );
            let deferred = deferred_tools(PromptProfile::Standard, &[], &config);
            for line in system.lines() {
                // "call `x`" is an order wherever it stands.
                for t in &deferred {
                    assert!(
                        !line.contains(&format!("call `{}`", t.name)),
                        "(golden {golden}) orders deferred `{}`: {line}",
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
                        "(golden {golden}) requires deferred `{}`: {line}",
                        t.name
                    );
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
        for t in deferred_tools(PromptProfile::Standard, &[], &config) {
            if !deferred.contains(&t.name) {
                deferred.push(t.name);
            }
        }
        assert!(
            deferred.iter().any(|n| n == "semantic_search"),
            "{deferred:?}"
        );
        let mut texts: Vec<String> = nudges::ALL.iter().map(|(_, t)| t.to_string()).collect();
        texts.extend([
            nudges::golden_continue(&["g1".into()], 1, 3),
            nudges::quality_failed("1 failing", "detail", 1, 3),
            nudges::repeating("`read_file`"),
            nudges::still_doing(&[crate::agent::persist::TaskItem {
                id: "t1".into(),
                title: "wire the UI".into(),
                description: String::new(),
                journal: vec![],
                status: "doing".into(),
            }]),
            transition::compose_context_handoff_message("handoff", Some("docs/plans/p.md")),
        ]);
        // The gates' refusals: since prompt v2 they are the instruction.
        // Except the one-plan refusal: it fires only in a session with a
        // linked document, whose run starts with the document tools loaded
        // (`doc_session::seed_tools`), so naming them costs no load.
        texts.extend(
            crate::agent::gate_text::all()
                .into_iter()
                .filter(|(name, _)| *name != "TASKS_WITH_DOCUMENT")
                .map(|(_, t)| t),
        );
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
        for golden in [false, true] {
            let (_, system, _) = run_prefix(
                &config,
                &crate::agent::tools::tests_support::ctx(),
                PromptProfile::Standard,
                None,
                None,
                &[],
                golden,
                &[],
            );
            texts.push((format!("golden={golden}"), system));
        }
        texts.push((
            "compact".into(),
            crate::agent::surface::COMPACT_PROMPT.to_string(),
        ));
        for (name, text) in [
            (
                "context handoff",
                transition::compose_context_handoff_message("doc", Some("p.md")),
            ),
            (
                "golden continue",
                nudges::golden_continue(&["g".into()], 1, 3),
            ),
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
        let sys = system_prompt(
            Some("/ws"),
            None,
            None,
            PromptProfile::Standard,
            subagent::MAX_PARALLEL_AGENTS,
            false,
        );
        for said in [
            "`<tasks>`",
            "in the same round as the first tool call",
            "one-step change",
            "`updates`",
        ] {
            assert!(sys.contains(said), "does not say {said:?}");
        }
        let compact = crate::agent::surface::COMPACT_PROMPT;
        assert!(compact.contains("`<tasks>`") && compact.contains("same round"));
    }

    /// One plan per session, sized to the request (single-timeline plan,
    /// Phase 2): none for one step, a task list for multi-step code work, a
    /// short document of independent questions for research, design or a
    /// write-up — never both — and the document's sections go to subagents
    /// in one call. One section, in every request: kept short.
    #[test]
    fn the_v2_prompt_has_one_plan_sized_to_the_request() {
        let sys = system_prompt(
            Some("/ws"),
            None,
            None,
            PromptProfile::Standard,
            subagent::MAX_PARALLEL_AGENTS,
            false,
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
            assert!(section.contains(said), "does not say {said:?}: {section}");
        }
        let words = section.split_whitespace().count();
        assert!(words <= 210, "{words} words");
    }

    /// A feature that is off takes its tools out of the catalog as well as
    /// the list: a line for a tool that cannot run is a failed call waiting.
    #[test]
    fn a_tool_whose_feature_is_off_is_neither_sent_nor_listed() {
        let mut cfg = AgentConfig::default();
        cfg.browser.enabled = false;
        let (_, system, tools) = prefix(&cfg, &[], &[]);
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
        let (_, system, tools) = prefix(&cfg, &[], &["browser"]);
        assert!(!names(&tools).contains(&"browser"), "loaded but off");
        assert!(!system.contains("- browser "));
    }

    #[test]
    fn every_mcp_tool_is_deferred() {
        let defs = [mcp("mcp__gh__issue"), mcp("mcp__gh__search")];
        let (_, system, tools) = prefix(&AgentConfig::default(), &defs, &[]);
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
        let (_, before_system, before) = prefix(&cfg, &defs, &[]);
        let (_, system, tools) = prefix(&cfg, &defs, &["mcp__gh__search", "semantic_search"]);
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
        let again = prefix(&cfg, &defs, &["mcp__gh__search", "semantic_search"]);
        assert_eq!(
            serde_json::to_string(&again.2).unwrap(),
            serde_json::to_string(&tools).unwrap()
        );
    }

    /// A tool a session loaded before one timeline (`enter_plan_mode`) no
    /// longer exists: it is left out of the list rather than sent.
    #[test]
    fn a_loaded_tool_that_no_longer_exists_is_left_out() {
        let cfg = AgentConfig::default();
        let (_, _, tools) = prefix(&cfg, &[], &["enter_plan_mode"]);
        assert!(!names(&tools).contains(&"enter_plan_mode"));
        assert_eq!(names(&tools).last(), Some(&"load_tools"));
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

/// Phase 5 of the lean-harness plan: the v2 Standard prompt at the right
/// altitude. Budgets are on the harness's own meter
/// (`session::estimate_tokens`, chars/3), the one `run_config.prefix` and the
/// context bar use.
#[cfg(test)]
mod standard_tests {
    use super::*;
    use crate::agent::session::estimate_tokens;

    const ROOT: &str = "/home/user/projects/app";

    fn tokens(text: &str) -> u64 {
        estimate_tokens(&[], text, &[])
    }

    /// Identity, environment, how the tools relate, when to ask and stop,
    /// untrusted content, hook output, the living document and the language
    /// policy, workspace line included.
    #[test]
    fn the_base_prompt_fits_fifteen_hundred_tokens() {
        let base =
            with_workspace(standard::SYSTEM_PROMPT, Some(ROOT)).replace("{max_parallel}", "4");
        let total = tokens(&base);
        eprintln!("base: {total} tokens ({} chars)", base.len());
        assert!(total <= 1_500, "base prompt is {total} tokens");
    }

    #[test]
    fn the_work_block_fits_six_hundred_tokens() {
        let total = tokens(standard::WORK_BLOCK);
        eprintln!("work block: {total} tokens");
        assert!(total <= 600, "work block is {total} tokens");
    }

    fn all_texts() -> String {
        [
            standard::SYSTEM_PROMPT,
            standard::GOLDEN_PROMPT,
            standard::WORK_BLOCK,
        ]
        .concat()
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
        // The register the lean-harness Phase 5 retired, as the old prompt had it.
        let old = "You MUST call tasks_get first. Requirements interview (MANDATORY - never skip). \
                   Only VERIFIED work closes a task; the gate will REJECT it otherwise.";
        let words = capitalised_words(old);
        for old in ["MUST", "MANDATORY", "VERIFIED", "REJECT"] {
            assert!(words.iter().any(|w| w == old), "{old} not found: {words:?}");
        }
        assert!(capitalised_words("## TASK SYSTEM\nuse `MUST_X`, a UI and a URL").is_empty());
        assert_eq!(capitalised_words("## GOALS (MANDATORY)"), ["MANDATORY"]);
    }

    /// Design rule 4: an imperative in capitals stays only where it fixes an
    /// observed failure, and `standard::KEPT_IMPERATIVES` names that failure.
    #[test]
    fn the_prompt_keeps_at_most_three_capitalised_imperatives_each_with_its_failure() {
        let words = capitalised_words(&all_texts());
        assert!(
            words.len() <= 3,
            "{} capitalised words: {words:?}",
            words.len()
        );
        for word in &words {
            let kept = standard::KEPT_IMPERATIVES.iter().find(|(w, _)| w == word);
            assert!(
                kept.is_some_and(|(_, why)| !why.trim().is_empty()),
                "{word} is in capitals with no failure cited in KEPT_IMPERATIVES"
            );
        }
    }

    /// The gates the code enforces teach through their error messages
    /// (`gate_text::tests` pins each one's next step); the prompt says once
    /// that they exist and keeps none of the sentences that described them.
    #[test]
    fn the_prompt_describes_no_gate_the_code_enforces() {
        let all = all_texts();
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
                "the prompt still describes a gate: {described:?}"
            );
        }
        let gate_lines: Vec<&str> = standard::SYSTEM_PROMPT
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

    /// The tool list is prefix too: `tasks_set` once carried the Brain-only
    /// Low-Level Design gate in its description, paid by every request. No
    /// core description restates a gate; refusals teach.
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
    /// specs. It states no gate (the spec guard refuses an edit and its
    /// refusal says to ask the user: `quality::spec::edit_refusal`) and adds
    /// no capitals.
    #[test]
    fn the_spec_section_leaves_the_spec_guard_to_its_refusal() {
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
        let v2 = build_spec_prompt_section(&ctx).unwrap();
        std::fs::remove_dir_all(&root).ok();
        assert!(v2.contains("Pay by card"), "{v2}");
        assert!(v2.contains("the user"), "{v2}");
        assert!(!v2.contains("refuses any path"), "{v2}");
        assert!(
            capitalised_words(&v2).is_empty(),
            "{:?}",
            capitalised_words(&v2)
        );
    }

    /// Decision D5: ask only what changes the work, bundle independent
    /// questions, no interview for a fully specified request — and the
    /// checklists live in the `design` skill, named in one line.
    #[test]
    fn the_interview_is_adaptive_and_points_at_the_design_skill() {
        let work = standard::WORK_BLOCK;
        for said in [
            "changes the work",
            "one `ask_user` call",
            "fully specified request needs no interview",
        ] {
            assert!(work.contains(said), "the work block does not say {said:?}");
        }
        for gone in [
            "one question at a time",
            "Batching questions is confusing",
            "never skip",
        ] {
            assert!(!work.contains(gone), "the work block still says {gone:?}");
        }
        let skill_lines = work
            .lines()
            .filter(|l| l.contains("`design` skill"))
            .count();
        assert_eq!(skill_lines, 1, "{work}");
        for moved in [
            "false consensus",
            "Sizing and layout",
            "FULL Mermaid catalog",
        ] {
            assert!(
                !work.contains(moved),
                "{moved:?} belongs in the design skill"
            );
        }
    }
}
