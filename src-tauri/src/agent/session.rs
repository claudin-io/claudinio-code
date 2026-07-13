use crate::agent::permissions;
use crate::agent::permissions::PermissionLevel;
use crate::agent::persist::{now_ms, SessionRecord, SessionStore};
use crate::agent::provider::{self, AgentConfig, ContentBlock, Message, ToolDescription};
use crate::agent::subagent;
use crate::agent::tools::{self, ToolContext, ToolOutput};
use serde::{Deserialize, Serialize};
use serde_json::Value;
use std::collections::HashMap;
use std::sync::atomic::{AtomicBool, Ordering};
use std::sync::{Arc, Mutex as StdMutex};
use tauri::ipc::Channel;
use tokio::sync::{oneshot, Mutex};

/// Context window of the supported models (claudinio and claudius: 256K).
pub const MAX_CONTEXT_TOKENS: u64 = 256_000;

/// Threshold for auto-compaction: if the context exceeds this before a
/// request, the history is compacted first (75% of the window).
pub const COMPACT_THRESHOLD: u64 = MAX_CONTEXT_TOKENS * 75 / 100;

/// Prefix that identifies a golden task.
pub const GOLDEN_TASK_PREFIX: &str = "golden-";

/// Golden-loop safety caps used when the config leaves them unset.
const DEFAULT_MAX_GOLDEN_CYCLES: usize = 5;
const DEFAULT_MAX_GOLDEN_STALLS: usize = 2;

/// Parse <goal>...</goal> tags from user input.
/// Returns (cleaned_text, list_of_goals).
pub fn parse_goals(text: &str) -> (String, Vec<String>) {
    let re = regex::Regex::new(r"<goal>(.*?)</goal>").unwrap();
    let mut goals = Vec::new();
    for cap in re.captures_iter(text) {
        let goal_text = cap[1].trim().to_string();
        if !goal_text.is_empty() {
            goals.push(goal_text);
        }
    }
    let cleaned = re.replace_all(text, "").to_string();
    let cleaned = cleaned.trim().to_string();
    (cleaned, goals)
}

/// Rough token estimation: count chars / 3 + per-message overhead + system prompt + tools.
fn estimate_message_tokens(msg: &Message) -> u64 {
    let json = serde_json::to_string(msg).unwrap_or_default();
    json.len() as u64 / 3 + 4 // +4 for role/format overhead
}

fn estimate_tokens(history: &[Message], system: &str, tools: &[ToolDescription]) -> u64 {
    let mut total = system.len() as u64 / 3;
    if !tools.is_empty() {
        total += serde_json::to_string(tools).unwrap_or_default().len() as u64 / 3;
    }
    // Per-message overhead (~4 tokens each for role markers + turn formatting)
    total += (history.len() as u64) * 8;
    for msg in history {
        total += estimate_message_tokens(msg);
    }
    total
}

/// How many recent user↔agent exchanges stay verbatim after a compaction.
const TAIL_USER_TURNS: usize = 2;
/// Budget for the kept tail; if the recent exchanges alone exceed this, the
/// tail shrinks (down to zero) so compaction still frees the context.
const TAIL_MAX_TOKENS: u64 = 20_000;

/// Number of Turn records (counted back from the end) to keep verbatim when
/// compacting: the last `TAIL_USER_TURNS` real user exchanges, bounded by
/// `TAIL_MAX_TOKENS`. Only looks at records after the previous compaction.
fn compute_tail_turns(records: &[SessionRecord]) -> usize {
    let start = records
        .iter()
        .rposition(|r| matches!(r, SessionRecord::Compacted { .. }))
        .map(|i| i + 1)
        .unwrap_or(0);
    let mut turns = 0usize;
    let mut exchanges = 0usize;
    let mut tokens = 0u64;
    let mut best = 0usize;
    for rec in records[start..].iter().rev() {
        let SessionRecord::Turn { message, .. } = rec else { continue };
        turns += 1;
        tokens += estimate_message_tokens(message);
        if tokens > TAIL_MAX_TOKENS {
            break;
        }
        if crate::agent::persist::is_real_user_turn(rec) {
            exchanges += 1;
            best = turns; // a window starting at this user turn fits the budget
            if exchanges >= TAIL_USER_TURNS {
                break;
            }
        }
    }
    best
}

/// Compact the conversation history by spawning a subagent to read the JSONL
/// file and produce a summary. The subagent has a completely fresh context.
/// The last `TAIL_USER_TURNS` exchanges are kept verbatim (recorded as
/// `tail_turns` on the Compacted marker). Returns the generated summary.
pub async fn compact_history(
    config: &AgentConfig,
    store: &SessionStore,
    ctx: &ToolContext,
    event_tx: &Channel<AgentEvent>,
    approvals: &ApprovalMap,
    answers: &AnswerMap,
    session_id: &str,
    steering: &Arc<SteeringCtl>,
) -> Result<String, String> {
    let jsonl_path = store.path.to_string_lossy().to_string();
    let records = crate::agent::persist::load_records(&store.path).unwrap_or_default();
    let tail_turns = compute_tail_turns(&records);

    let summary = subagent::run_summary_agent(
        config,
        ctx,
        &jsonl_path,
        tail_turns,
        event_tx,
        approvals,
        answers,
        session_id,
        steering,
    )
    .await?;

    // If the summary agent somehow found an existing Compacted record and
    // returned it, still write ours — the format is append-only and the
    // history_from_records logic picks the LAST one.
    store.append(&SessionRecord::Compacted {
        summary: summary.clone(),
        tail_turns,
        ts: now_ms(),
    })?;

    // Record the post-compaction context size so the UI meter drops even for
    // manual compaction (no run in flight). The estimate excludes the system
    // prompt/tools; the next run's Status corrects it with the real number.
    let new_recs = crate::agent::persist::load_records(&store.path).unwrap_or_default();
    let new_history = crate::agent::persist::history_from_records(&new_recs);
    let (ci, co, cc, cci, cco, ccc) = crate::agent::persist::cumulative_stats(&new_recs);
    let new_context = estimate_tokens(&new_history, "", &[]);
    write_status(store, session_id, ci, co, cc, cci, cco, ccc, Some(new_context));

    Ok(summary)
}

/// The session's operating mode: Brain plans with read-only tools,
/// Builder executes with the full toolset.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "lowercase")]
pub enum SessionMode {
    Brain,
    Builder,
}

impl SessionMode {
    pub fn as_str(&self) -> &'static str {
        match self {
            SessionMode::Brain => "brain",
            SessionMode::Builder => "builder",
        }
    }

    pub fn parse(s: &str) -> Option<SessionMode> {
        match s {
            // "pensador"/"constructor" are the original names of these modes;
            // JSONL files written before the rename still carry them.
            "brain" | "pensador" => Some(SessionMode::Brain),
            "builder" | "constructor" => Some(SessionMode::Builder),
            _ => None,
        }
    }
}

/// Which situational prompt/toolset a workflow run uses. `Standard` is the
/// full agent (task system, Brain/Builder modes, skills, subagents, golden
/// tasks). Other variants are lean, purpose-built profiles for a single kind
/// of job — no task system, no modes, minimal toolset — so the model isn't
/// paying for ceremony it doesn't need.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum PromptProfile {
    Standard,
    /// Commit & push: a single-purpose git operator. Bash + ask_user only.
    GitSync,
}

/// Who put the session in its current mode. The agent may only exit Brain
/// on its own if it was the one who entered it; a human-initiated Brain
/// can only be exited by the human toggle.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "lowercase")]
pub enum ModeOrigin {
    Human,
    Agent,
}

impl ModeOrigin {
    pub fn as_str(&self) -> &'static str {
        match self {
            ModeOrigin::Human => "human",
            ModeOrigin::Agent => "agent",
        }
    }
}

/// Shared, mutable mode state for a session. Lives in AppState keyed by
/// session id so the UI toggle and the running workflow see the same value.
/// The Mutex is never held across await.
pub struct ModeCtl {
    state: StdMutex<(SessionMode, ModeOrigin)>,
}

impl ModeCtl {
    pub fn new(mode: SessionMode, origin: ModeOrigin) -> Self {
        Self {
            state: StdMutex::new((mode, origin)),
        }
    }

    pub fn get(&self) -> (SessionMode, ModeOrigin) {
        *self.state.lock().unwrap()
    }

    pub fn set(&self, mode: SessionMode, origin: ModeOrigin) {
        *self.state.lock().unwrap() = (mode, origin);
    }
}

/// A single steering message: text + pre-processed attachment data.
pub struct SteeringEntry {
    pub text: String,
    pub attachments: Vec<(ContentBlock, crate::agent::persist::AttachmentMeta)>,
}

/// Steering: a queue of mid-run user messages and an interrupt flag.
/// Thread-safe; the Mutex is never held across await.
pub struct SteeringCtl {
    pub queue: StdMutex<Vec<SteeringEntry>>,
    pub interrupt: Arc<AtomicBool>,
}

impl SteeringCtl {
    pub fn new() -> Self {
        Self {
            queue: StdMutex::new(Vec::new()),
            interrupt: Arc::new(AtomicBool::new(false)),
        }
    }

    pub fn drain(&self) -> Vec<SteeringEntry> {
        let mut q = self.queue.lock().unwrap();
        std::mem::take(&mut *q)
    }

    pub fn push(&self, entry: SteeringEntry) {
        let mut q = self.queue.lock().unwrap();
        q.push(entry);
    }

    pub fn clear(&self) {
        self.queue.lock().unwrap().clear();
        self.interrupt.store(false, Ordering::SeqCst);
    }
}

/// Cache-stable system prompt. This is the byte-identical prefix of every
/// request in a session — keep it constant so the provider's prefix cache stays
/// warm.
const SYSTEM_PROMPT: &str = r#"Role: Claudinio, AI coding agent inside Claudinio Code.
UI Mandate: The Task Panel is your only plan/progress UI. Never write plans in text.

# 1. TASK SYSTEM (STRICT WORKFLOW)
- You MUST call `tasks_get` first.
- Call `tasks_set` to create tasks (id, title, description, journal: [], status: 'todo'). 1 logical step = 1 task.
- Update in real time: strictly follow `todo` -> `doing` -> append to `journal` -> `done`. Never batch updates.
- `tasks_set` is a full replacement. You must pass ALL tasks every time.
- Before your final text response, you MUST make a final `tasks_set` call.
- If the user asks about progress, guide them to the Task Panel.

# 2. CODE TOOLS
- Accuracy hierarchy: LSP > `semantic_search` (conceptual) > `code_search` (keyword) > `grep` (fallback).
- Conceptual questions MUST start with `semantic_search`.
- Use `file_outline` before `read_file` on unfamiliar files.
- Never use bash search tools (grep/find/rg) when a dedicated tool exists.

# 3. SUBAGENTS (`spawn_agents`)
- Call shape: spawn_agents is ONE call carrying ALL parallel agents in its 'agents' array: {"agents": [{"name", "goal", "mode", "expected_output"}, ...]}. Never flatten a single agent's fields to the top level.
- There is NO 'agent', 'task', or per-agent tool — never emit one call per agent.
- Core strategy: aggressively use subagents for search and verification. Keeping the main context lean saves significant tokens and boosts your reasoning intelligence (the fuller the main context, the harder it is to reason).
- When delegating, provide clear hints about where to look or what to do, letting subagents filter and distill key results for you.
- Use for broad/parallel tasks. Max 4 per call. Modes: 'explore' or 'code'.
- Scope must never overlap. Avoid using for trivial or dependent tasks.
- Goals MUST be 100% independent instructions: exact paths, verbatim values/URLs/dimensions.
- You MUST resolve resources/URLs before delegating. Subagents must never guess or ask the user.

# 4. TURN COMPLETION
- You MUST finish all work, or block with `ask_user`. Never end with plain-text questions or TODOs.
- Only show your last message. It must be fully self-contained (no "see above").

# 5. GIT & ACTIONS
- Unless the user explicitly instructs, you MUST call `ask_user` before performing external/destructive operations (push, branch, PR).

# 6. LINKS (Markdown)
Your text responses are rendered as Markdown. Use standard Markdown links to make files, images, and URLs clickable. The chat UI detects the link type from the extension and opens it with the appropriate viewer or external browser.

Link types (auto-detected by extension):
- **External URLs**: `[label](https://example.com)` — opens in the default browser.
- **File links**: `[label](src/lib/ipc.ts)` or `[label](./relative/path.rs)` — relative to workspace root; opens a text viewer with Monaco editor.
- **Image links**: `[label](src/assets/screenshot.png)` — opens the image in a viewer; supported: png, jpg, jpeg, gif, webp, svg.
- **Video links**: `[label](demo.mp4)` — opens a video player; supported: mp4, webm, mov.
- **Audio links**: `[label](sound.mp3)` — opens an audio player; supported: mp3, wav, ogg, flac.

Examples:
```
See the main component: [ChatPanel.tsx](src/components/ChatPanel.tsx)
System prompt ref: [session.rs](src-tauri/src/agent/session.rs)
Architecture diagram: [diagram.png](docs/architecture.png)
Landing page: [Claudinio Code](https://claudin.io)
```
Use relative paths from the workspace root (no leading `/`). The file icon next to linked items is automatic — you just write the Markdown link.

# LANGUAGE POLICY
- User-facing replies: write in the language of the user's latest message. If it is unclear or mixed, default to English.
- Your reasoning/thinking and ALL tool inputs (search queries, subagent goals, file paths, command args, plan & task text) MUST be in English."#;

/// Appended to the system prompt in BOTH modes: golden tasks are mandatory
/// goals the session must reach before it is allowed to finish for real.
const GOLDEN_PROMPT: &str = "\n\n## GOLDEN TASKS (MANDATORY GOALS)\n\
Tasks whose id starts with 'golden-' are mandatory goals set by the user via <goal> tags:\n\
- They are the success criteria of the session: work is only finished when every golden task has status='done'.\n\
- Only mark a golden task 'done' after you VERIFIED the goal it describes is actually met (run the checks — build, tests, coverage, whatever the goal requires) — never on intention.\n\
- If you end your turn while golden tasks are pending, the system automatically switches mode (Brain to plan, Builder to execute) and sends you back to work on them, up to a cycle limit.\n\
- Never delete golden tasks in tasks_set; keep them in the list and update their status.";

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
- Final user-facing summary: language of the user's most recent message if known, else English.
- Reasoning and commands MUST be in English."#;

/// Build the per-session system prompt. The base is byte-identical for every
/// request in the same workspace so the provider's prefix cache stays warm;
/// the mode block is appended last and only changes when the mode switches.
fn system_prompt(
    workspace_root: Option<&str>,
    skills_section: Option<&str>,
    plan_save_path: Option<&str>,
    mode: SessionMode,
    profile: PromptProfile,
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
    let base = match workspace_root {
        Some(root) => format!(
            "{SYSTEM_PROMPT}\n\nProject workspace root: {root}. \
The bash tool already runs with this directory as its working directory - run commands directly \
(e.g. \"git status\"), use relative paths, and never cd into guessed paths. \
File tools take absolute paths inside this root."
        ),
        None => SYSTEM_PROMPT.to_string(),
    };
    let base = match skills_section {
        Some(s) if !s.is_empty() => format!("{base}\n{s}"),
        _ => base,
    };
    // Resolve the effective plans directory for the prompt.
    let plans_subdir = match plan_save_path {
        Some(path) if !path.is_empty() => format!(".claudinio.json (plan_save_path=\"{path}\")"),
        _ => ".claudinio/plans".to_string(),
    };

    match mode {
        SessionMode::Brain => {
            // build_brain_prompt builds the Brain mode prompt text.
            // Uses ascii-only punctuation to avoid Rust 2021 lexer issues
            // with multi-byte chars adjacent to \ continuations.
            #[rustfmt::skip]
            let brain_text = concat!(
                "\n\n## CURRENT MODE: BRAIN (PLANNING - READ ONLY)\n",
                "You are in Brain mode: the brain trust for the whole operation - explorer and requirements analyst. ",
                "You must never implement, edit files, or run state-changing commands - your editing tools are disabled, ",
                "bash only accepts read-only commands.\n",
                "\n### Mandatory deliverables\n",
                "A Brain session is not complete until both of the following exist, regardless of who enabled this mode:\n",
                "1. A Solution Design plan written via `write_plan` ({plans_subdir}/*.md).\n",
                "2. An executable task list created via `tasks_set` - one self-contained task per atomic step, ",
                "each task carrying enough description (file paths, symbols, constraints, plan file path, and all ",
                "user-provided VERBATIM values - URLs, exact asset/icon IDs, real SVG/code snippets, agreed sizes and ",
                "dimensions), so it can be handed to a Builder subagent that knows nothing about this conversation and cannot ask the user. ",
                "A task that references a design decision without stating its concrete value is incompletely defined. All status='todo'. ",
                "Never end your turn before both deliverables are in place.\n",
                "\n### Investigation: smart tools first\n",
                "Indexed tools are your primary tools - brute-force search is the last resort:\n",
                "* `semantic_search` is your first call for any conceptual question ('how does X work', 'where is behavior Y') - describe the behavior in English.\n",
                "* `code_search`/`symbol_lookup` only when you already know the exact symbol or keyword.\n",
                "* For unfamiliar files, `file_outline` before `read_file`; use `go_to_definition`/`find_references` to trace relationships.\n",
                "* `grep` and bash search are last resorts, used only after indexed tools return empty results.\n",
                "For any broad task, aggressively use `spawn_agents` ('explore' mode) - to map areas and verify theories without polluting your context - ",
                "and instruct each subagent to follow the same tool order. Explore before interviewing, so your questions are grounded in facts.\n",
                "\n### Requirements interview (MANDATORY - never skip)\n",
                "Before writing any plan, you must keep interviewing the user about the request until consensus is reached. ",
                "Every planning request has decisions only the user can own - if you wrote a plan without asking any questions, you did it wrong. ",
                "Walk each branch of the design tree, resolving decision dependencies one at a time:\n",
                "1. Ask one question at a time via the `ask_user` tool - one `ask_user` call with one question, wait for ",
                "the answer, and let it shape the next question. Batching questions is confusing.\n",
                "2. Put your recommended answer as the first option in every question, suffixed with ' (Recommended)'.\n",
                "3. If a fact can be found in the codebase, look it up with your tools instead of asking. The decision belongs to the user - hand each one to them and wait.\n",
                "4. Never call `write_plan` before the user confirms consensus - your last interview question must be confirming the agreed design.\n",
                "\n### UI/visual features: sizing and assets are mandatory decisions\n",
                "When a request involves any visual content (components, modals, dialogs, panels, buttons, layouts), ",
                "the user owns these decisions - you must interview them, never invent:\n",
                "* Sizing and layout: dimensions of the new interface (modal width/height - full-screen? fixed px? percentage of viewport? margins?), ",
                "position, and responsive behavior. A 'modal' with no agreed size is an incomplete spec - ask.\n",
                "* User-provided assets: if the user gave an icon name, URL, image, prototype, or exact copy, ",
                "that asset is GROUND TRUTH. Do not paraphrase it as 'an icon similar to X'. ",
                "Resolve it (fetch the URL/read the image) to get the real data, confirm you will use it exactly, and in the plan and ",
                "task descriptions record the reference VERBATIM (full URL, exact icon ID like 'lucide:notebook-pen', real SVG) ",
                "- so Builder and its subagents use real content, not guesses.\n",
                "\n### Workflow\n",
                "Explore (subagents + `semantic_search`) -> interview (protocol above) -> `write_plan` (sections: ",
                "Context, Solution Design, Risks, Tasks summary; call again to revise the full content) -> ",
                "`tasks_set` -> handoff: if you yourself entered this mode (via `enter_plan_mode`), call `exit_plan_mode` and start",
                "building; if the user enabled it, do not try to exit - just say the plan and tasks are ready, and wait for them to flip the switch to Builder mode.\n",
                "\n# LANGUAGE POLICY\n",
                "- User-facing replies: write in the language of the user's latest message. If unclear or mixed, default to English.\n",
                "- Your reasoning/thinking and ALL tool inputs (search queries, subagent goals, file paths, command args, plan & task text) MUST be in English.\n"
            );
            let brain_prompt = brain_text.replace("{plans_subdir}", &plans_subdir);
            format!("{base}{GOLDEN_PROMPT}{brain_prompt}")
        }
        SessionMode::Builder => {
            #[rustfmt::skip]
            let builder_text = concat!(
                "\n\n## CURRENT MODE: BUILDER (EXECUTION)\n",
                "You are in Builder mode: you execute the plan Brain prepared. The task list (normally created in Brain mode) ",
                "IS your worklist - every edit MUST be driven through it, exactly as the base ## TASK SYSTEM requires. ",
                "Working without updating the tasks in real time is a defect, not a shortcut.\n",
                "1. Call `tasks_get` FIRST - before any `edit_file` or state-changing command. This is not optional even when tasks ",
                "already exist: you must load them and follow them in order, respecting dependencies. They ARE the plan.\n",
                "2. Also read the most recent plan file in `{plans_subdir}/` (`list_dir`) before executing - it carries ",
                "the Solution Design context the tasks refer to.\n",
                "3. Execute ONE task at a time, in dependency order. BEFORE you touch any file or spawn a subagent for a task, ",
                "call `tasks_set` to mark THAT task status='doing'. NEVER implement or edit a task that is still ",
                "'todo' - mark it 'doing' first, always.\n",
                "4. Delegate: implement each task through `spawn_agents` in 'code' mode - one subagent per task, ",
                "in ONE call when tasks are independent (parallel), in sequential waves when they depend on each other. ",
                "This keeps your main context clean. Only implement directly yourself when a task is trivial (a single small edit) or needs mid-task user decisions.\n",
                "   Each subagent goal must be a COMPLETE technical spec: it must repeat every concrete value from the plan/task VERBATIM ",
                "(exact file paths and symbols, agreed sizes/dimensions, and any user-supplied asset - the real URL, exact icon id, real SVG). ",
                "The subagent has empty context and cannot ask the user, so if a value is missing it WILL guess and be wrong. ",
                "If the plan references an external asset by name/URL that isn't yet concrete data, RESOLVE it first (fetch the data) and paste the real data into the goal - ",
                "never tell a subagent to make something 'similar to' an asset the user already specified.\n",
                "5. When a task's work is verified, call `tasks_set` to mark THAT task status='done', with journal entries for the findings and the 'why'. ",
                "Do this task by task, as you go - NEVER batch several tasks into a single 'done' call at the end. Then move to the next task (back to step 3).\n",
                "6. Use the available skills whenever one matches the work.\n",
                "7. After all tasks, verify the whole (build/tests where applicable) and report.\n",
                "8. As your LAST step, once every task is done and verified, call `finalize_plan` with a journal of findings ",
                "(key decisions, gotchas, what was learned). It auto-records the changed files and commit(s) into the plan file, ",
                "so the journal should focus on the 'why' and what you learned - not a file list. This feeds the plan with data for future reference.\n",
                "Investigate with the smart tools first - `semantic_search` for behavior questions, `code_search`/`symbol_lookup` for known names, ",
                "`file_outline` before reading - and leave `grep`/bash searching as the last resort. Tell your subagents to do the same.\n",
                "\n# LANGUAGE POLICY\n",
                "- User-facing replies: write in the language of the user's latest message. If unclear or mixed, default to English.\n",
                "- Your reasoning/thinking and ALL tool inputs (search queries, subagent goals, file paths, command args, plan & task text) MUST be in English.\n"
            );
            let builder_prompt = builder_text.replace("{plans_subdir}", &plans_subdir);
            format!("{base}{GOLDEN_PROMPT}{builder_prompt}")
        }
    }
}



#[derive(Debug, Clone, Serialize, Deserialize)]
#[serde(tag = "event", content = "data")]
pub enum AgentEvent {
    #[serde(rename = "TextStep")]
    TextStep {
        text: String,
    },
    /// Live, accumulated snapshot of the assistant text currently streaming.
    /// Superseded by the next `TextStep`/`Done` for the same block; never persisted.
    #[serde(rename = "TextDelta")]
    TextDelta {
        text: String,
    },
    #[serde(rename = "Thinking")]
    Thinking(String),
    #[serde(rename = "ToolCall")]
    ToolCall {
        #[serde(rename = "sessionId")]
        session_id: String,
        #[serde(rename = "toolId")]
        tool_id: String,
        #[serde(rename = "toolName")]
        tool_name: String,
        args: Value,
        permission: String,
        #[serde(rename = "editProposal")]
        edit_proposal: Option<EditProposalData>,
    },
    #[serde(rename = "ToolResult")]
    ToolResult {
        #[serde(rename = "toolId")]
        tool_id: String,
        #[serde(rename = "toolName")]
        tool_name: String,
        output: String,
        error: Option<String>,
    },
    #[serde(rename = "AskUser")]
    AskUser {
        #[serde(rename = "sessionId")]
        session_id: String,
        #[serde(rename = "toolId")]
        tool_id: String,
        questions: Value,
    },
    #[serde(rename = "Done")]
    Done {
        #[serde(rename = "stopReason")]
        stop_reason: String,
        #[serde(rename = "textOutput")]
        text_output: String,
        #[serde(rename = "inputTokens")]
        input_tokens: u32,
        #[serde(rename = "outputTokens")]
        output_tokens: u32,
    },
    #[serde(rename = "SteeringInjected")]
    SteeringInjected {
        text: String,
        #[serde(skip_serializing_if = "Option::is_none")]
        attachments: Option<Vec<crate::agent::persist::AttachmentMeta>>,
    },
    #[serde(rename = "Error")]
    Error(String),
    #[serde(rename = "SubagentStarted")]
    SubagentStarted {
        #[serde(rename = "subagentId")]
        subagent_id: String,
        #[serde(rename = "parentToolId")]
        parent_tool_id: String,
        name: String,
        goal: String,
        mode: String,
    },
    #[serde(rename = "SubagentDone")]
    SubagentDone {
        #[serde(rename = "subagentId")]
        subagent_id: String,
        status: String,
        rounds: u32,
        #[serde(rename = "inputTokens")]
        input_tokens: u32,
        #[serde(rename = "outputTokens")]
        output_tokens: u32,
        #[serde(rename = "report")]
        report: String,
    },
    #[serde(rename = "Subagent")]
    Subagent {
        #[serde(rename = "subagentId")]
        subagent_id: String,
        event: Box<AgentEvent>,
    },
    #[serde(rename = "ModeChanged")]
    ModeChanged {
        mode: String,
        origin: String,
        reason: Option<String>,
    },
    /// The run tried to finish with golden tasks still pending: a new golden
    /// cycle starts in `mode`. `pending` lists the unfinished golden task ids.
    #[serde(rename = "GoldenLoop")]
    GoldenLoop {
        cycle: u32,
        #[serde(rename = "maxCycles")]
        max_cycles: u32,
        pending: Vec<String>,
        mode: String,
    },
    #[serde(rename = "SessionStats")]
    SessionStats {
        #[serde(rename = "inputTokens")]
        input_tokens: u32,
        #[serde(rename = "outputTokens")]
        output_tokens: u32,
        #[serde(rename = "cumulativeCost")]
        cumulative_cost: Option<f64>,
        #[serde(rename = "costInput")]
        cost_input: Option<f64>,
        #[serde(rename = "costOutput")]
        cost_output: Option<f64>,
        #[serde(rename = "costCacheRead")]
        cost_cache_read: Option<f64>,
        #[serde(rename = "contextTokens")]
        context_tokens: u64,
        #[serde(rename = "maxContextTokens")]
        max_context_tokens: u64,
        #[serde(rename = "compactThreshold")]
        compact_threshold: u64,
    },
}

#[derive(Debug, Clone, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct EditProposalData {
    pub path: String,
    pub old_string: String,
    pub new_string: String,
    pub unified_diff: String,
}

pub type ApprovalMap = Arc<Mutex<HashMap<String, oneshot::Sender<bool>>>>;

/// One answered question from the ask_user tool: the frontend echoes the
/// question text back with the option the user picked (or typed).
#[derive(Debug, Clone, Serialize, serde::Deserialize)]
pub struct UserAnswer {
    pub question: String,
    pub answer: String,
}

pub type AnswerMap = Arc<Mutex<HashMap<String, oneshot::Sender<Vec<UserAnswer>>>>>;

/// Tools offered to the model for a given mode/profile. `GitSync` gets only
/// `bash` + `ask_user` — no task system, no subagents, no MCP tools. Builder
/// gets the full registry plus enter_plan_mode; Brain drops edit_file and
/// gains write_plan + exit_plan_mode (bash stays but is gated to read-only
/// commands in run_workflow).
fn api_tools(mode: SessionMode, profile: PromptProfile, mcp_defs: &[tools::ToolDef], config: &AgentConfig) -> Vec<ToolDescription> {
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
    let mut defs = tools::get_defs(maxp);
    match mode {
        SessionMode::Builder => {
            defs.push(tools::enter_plan_mode_def());
            defs.push(tools::finalize_plan_def());
        }
        SessionMode::Brain => {
            defs.retain(|t| t.name != "edit_file");
            defs.push(tools::write_plan_def());
            defs.push(tools::exit_plan_mode_def());
        }
    }
    defs.extend(mcp_defs.iter().cloned());
    defs.iter()
        .map(|t| ToolDescription {
            name: t.name.clone(),
            description: t.description.clone(),
            input_schema: t.input_schema.clone(),
        })
        .collect()
}

/// Push a message onto history and persist it as a Turn record.
fn push_turn(history: &mut Vec<Message>, store: &SessionStore, message: Message) {
    store.try_append(&SessionRecord::Turn {
        message: message.clone(),
        ts: now_ms(),
    });
    history.push(message);
}

/// Add user-role content, merging into the previous message when it is already a
/// user turn. The Anthropic API requires strictly alternating roles, so this
/// prevents two consecutive user turns (which can happen when the model returns
/// nothing). Merges are intentionally not persisted as a new Turn record,
/// keeping the JSONL history alternating on reopen as well.
fn push_user_blocks(history: &mut Vec<Message>, store: &SessionStore, blocks: Vec<ContentBlock>) {
    if let Some(last) = history.last_mut() {
        if last.role == "user" {
            last.content.extend(blocks);
            return;
        }
    }
    push_turn(
        history,
        store,
        Message {
            role: "user".into(),
            content: blocks,
        },
    );
}

/// Drain the steering queue, persist each message, merge into the last user turn
/// (or create a new one), and emit SteeringInjected events. Returns true if any
/// steering was injected.
fn inject_steering(
    history: &mut Vec<Message>,
    store: &SessionStore,
    steering: &SteeringCtl,
    event_tx: &Channel<AgentEvent>,
) -> bool {
    let entries = steering.drain();
    if entries.is_empty() {
        return false;
    }
    for entry in &entries {
        // Build content blocks: text first, then attachments
        let mut blocks = vec![ContentBlock::text(&entry.text)];
        let mut attachment_metas: Vec<crate::agent::persist::AttachmentMeta> = Vec::new();
        for (block, meta) in &entry.attachments {
            blocks.push(block.clone());
            attachment_metas.push(meta.clone());
        }
        store.try_append(&SessionRecord::Steering {
            text: entry.text.clone(),
            attachments: Some(attachment_metas.clone()),
            ts: now_ms(),
        });
        push_user_blocks(history, store, blocks);
        let _ = event_tx.send(AgentEvent::SteeringInjected {
            text: entry.text.clone(),
            attachments: Some(attachment_metas),
        });
    }
    true
}

/// Run a single continuous provider→tool loop for one user input, until the
/// model produces a turn with no tool calls. Shares one conversation history
/// (append-only, cache-friendly) and persists every step to the session JSONL
/// store. The model decides at each round whether it still needs a tool call
/// or can answer directly — there are no forced phases.
#[allow(clippy::too_many_arguments)]
/// Reject messages that are not written in English.
fn reject_non_english(msg: &str) -> Result<(), String> {
    let non_ascii: Vec<char> = msg.chars().filter(|&c| c > '\u{7E}').collect();
    if non_ascii.is_empty() {
        return Ok(());
    }
    let total = msg.chars().count() as f64;
    let ratio = non_ascii.len() as f64 / total;
    if ratio > 0.10 {
        let sample: String = non_ascii.iter().take(5).collect();
        return Err(format!(
            "Only English is supported. Please write your message in English. \
             (Detected non-English characters: {})",
            sample
        ));
    }
    Ok(())
}

/// Write a Status record with cumulative token/cost stats and the size of
/// the context for the next request.
#[allow(clippy::too_many_arguments)]
fn write_status(
    store: &SessionStore,
    session_id: &str,
    cumul_in: u64,
    cumul_out: u64,
    cumul_cost: Option<f64>,
    cumul_cost_input: Option<f64>,
    cumul_cost_output: Option<f64>,
    cumul_cost_cache_read: Option<f64>,
    context_tokens: Option<u64>,
) {
    store.try_append(&SessionRecord::Status {
        session_id: session_id.to_string(),
        total_input_tokens: cumul_in,
        total_output_tokens: cumul_out,
        total_cost: cumul_cost,
        total_cost_input: cumul_cost_input,
        total_cost_output: cumul_cost_output,
        total_cost_cache_read: cumul_cost_cache_read,
        context_tokens,
        ts: now_ms(),
    });
}

/// Per-million-token rates for a model (claudin.io official pricing).
/// Fallback estimate for when the litellm proxy's cost_injector middleware
/// doesn't report a real breakdown (unpriced model, older proxy deploy).
struct Pricing {
    input: f64,
    cache_read: f64,
    output: f64,
}

fn model_pricing(model: &str) -> Pricing {
    if model.contains("claudius") {
        Pricing { input: 3.00, cache_read: 0.90, output: 8.00 }
    } else {
        // claudinio and unknown models: balanced tier
        Pricing { input: 0.50, cache_read: 0.15, output: 2.00 }
    }
}

/// Cost broken down by token category, in USD.
struct CostBreakdown {
    input: f64,
    output: f64,
    cache_read: f64,
}

/// Estimate cost breakdown for provider calls when the provider does not
/// report a real cost breakdown.
fn cost_breakdown_for(model: &str, input: u32, cache_read: u32, output: u32) -> CostBreakdown {
    let p = model_pricing(model);
    CostBreakdown {
        input: input as f64 * p.input / 1_000_000.0,
        output: output as f64 * p.output / 1_000_000.0,
        cache_read: cache_read as f64 * p.cache_read / 1_000_000.0,
    }
}

/// This round's cost breakdown: the provider-reported values when present,
/// otherwise the local per-million-token estimate.
#[allow(clippy::too_many_arguments)]
fn cost_or_estimate(
    model: &str,
    total_in: u32,
    total_cache: u32,
    total_out: u32,
    run_cost_input: Option<f64>,
    run_cost_output: Option<f64>,
    run_cost_cache: Option<f64>,
) -> (f64, f64, f64) {
    if run_cost_input.is_none() && run_cost_output.is_none() && run_cost_cache.is_none() {
        let b = cost_breakdown_for(model, total_in, total_cache, total_out);
        (b.input, b.output, b.cache_read)
    } else {
        (
            run_cost_input.unwrap_or(0.0),
            run_cost_output.unwrap_or(0.0),
            run_cost_cache.unwrap_or(0.0),
        )
    }
}

/// Roll this round's cost into the cumulative totals — both the blended
/// `cumul_cost` (kept independent so sessions persisted before the breakdown
/// existed don't lose their historical total) and the per-category breakdown.
#[allow(clippy::too_many_arguments)]
fn roll_cost(
    model: &str,
    total_in: u32,
    total_cache: u32,
    total_out: u32,
    run_cost_input: Option<f64>,
    run_cost_output: Option<f64>,
    run_cost_cache: Option<f64>,
    cumul_cost: &mut Option<f64>,
    cumul_cost_input: &mut Option<f64>,
    cumul_cost_output: &mut Option<f64>,
    cumul_cost_cache: &mut Option<f64>,
) {
    let (ci, co, cc) = cost_or_estimate(
        model, total_in, total_cache, total_out,
        run_cost_input, run_cost_output, run_cost_cache,
    );
    *cumul_cost = Some(cumul_cost.unwrap_or(0.0) + ci + co + cc);
    *cumul_cost_input = Some(cumul_cost_input.unwrap_or(0.0) + ci);
    *cumul_cost_output = Some(cumul_cost_output.unwrap_or(0.0) + co);
    *cumul_cost_cache = Some(cumul_cost_cache.unwrap_or(0.0) + cc);
}

/// True for errors worth retrying: network hiccups, stalled connections, and
/// rate-limit/server errors. False for things that will fail again immediately
/// (bad auth, malformed request) — retrying those just wastes time.
fn is_retryable_error(msg: &str) -> bool {
    // Budget esgotado do plano: retentar é inútil (o servidor recusará de novo
    // até o usuário fazer upgrade). O frontend mostra um banner de upgrade.
    if msg.starts_with(crate::agent::provider::BUDGET_EXCEEDED_MARKER) {
        return false;
    }
    if msg.starts_with("stream error:") || msg.starts_with("request failed:") {
        return true;
    }
    if let Some(code) = msg.strip_prefix("API error: HTTP ").and_then(|s| s.parse::<u16>().ok()) {
        return code == 429 || (500..600).contains(&code);
    }
    false
}

/// How a terminal `end_turn` (no tool call) should be handled.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum TurnVerdict {
    /// A complete, self-contained reply — end the run normally.
    Done,
    /// The model announced/implied an immediate next step (ask the user, spawn
    /// subagents, read a file, edit code…) but ended without taking it. The run
    /// must not go idle here — nudge the model to actually act and loop again.
    Continue,
}

/// Map the completion-judge model's reply to a verdict. Kept separate from the
/// HTTP call so the parsing is deterministic and unit-testable, and — crucially —
/// language-agnostic: the judge is instructed to answer with a fixed sentinel
/// token, never natural-language prose, so no per-language keyword list is ever
/// needed here (new UI languages need no changes).
///
/// Fails safe toward `Done`: an unrecognizable reply ends the run rather than
/// risking a spurious extra loop.
fn parse_turn_verdict(reply: &str) -> TurnVerdict {
    let norm = reply.trim().to_ascii_uppercase();
    // Accept the token anywhere in the reply so a chatty model ("CONTINUE — it
    // said it would ask a question") is still handled correctly. CONTINUE wins
    // ties: if the judge is unsure enough to emit both, keep working.
    if norm.contains("CONTINUE") {
        TurnVerdict::Continue
    } else if norm.contains("DONE") {
        TurnVerdict::Done
    } else {
        TurnVerdict::Done
    }
}

/// Ask the model itself whether a terminal turn is genuinely finished or merely
/// announced a next step it never took (the failure that stalled session
/// 912bb460: "Primeiro, preciso confirmar algo sobre o tempo:" with no tool
/// call, and the twin case where it said it would spawn subagents and didn't).
///
/// Uses the LLM instead of hardcoded phrases so it works in any language the UI
/// ever adds. Fails safe toward `Done` on any error — a judge outage must never
/// wedge the loop or fabricate an infinite continuation.
async fn judge_terminal_turn(
    config: &AgentConfig,
    model: &str,
    assistant_text: &str,
) -> TurnVerdict {
    match crate::agent::provider::classify_turn_completion(config, model, assistant_text).await {
        Ok(reply) => parse_turn_verdict(&reply),
        Err(_) => TurnVerdict::Done,
    }
}

/// Wraps `provider::stream_message` with a retry loop for transient network
/// failures (stalled streams, dropped connections, 429/5xx). A full 30-minute
/// hang like the one that killed session 1aafbfbf was silently unrecoverable
/// before this — a single reqwest error aborted the whole agent run.
#[allow(clippy::too_many_arguments)]
async fn stream_message_with_retry(
    config: &AgentConfig,
    model: &str,
    messages: &[Message],
    tools: &[ToolDescription],
    system: Option<&str>,
    event_tx: &Channel<AgentEvent>,
    session_id: &str,
    assistant_text: &mut String,
    interrupt: &AtomicBool,
) -> Result<provider::StreamOutput, String> {
    const BACKOFFS_MS: [u64; 8] = [2_000, 5_000, 15_000, 30_000, 60_000, 120_000, 180_000, 300_000];
    let mut attempt = 0usize;
    loop {
        assistant_text.clear();
        let result = provider::stream_message(
            config, model, messages, tools, system, event_tx, session_id, assistant_text, interrupt,
            true,
        )
        .await;
        match result {
            Ok(out) => return Ok(out),
            Err(e) if attempt < BACKOFFS_MS.len() && is_retryable_error(&e) => {
                if interrupt.load(Ordering::SeqCst) {
                    return Err(e);
                }
                tokio::time::sleep(std::time::Duration::from_millis(BACKOFFS_MS[attempt])).await;
                attempt += 1;
            }
            Err(e) => return Err(e),
        }
    }
}

pub async fn run_workflow(
    config: &AgentConfig,
    history: &mut Vec<Message>,
    user_message: String,
    attachment_blocks: Vec<ContentBlock>,
    event_tx: &Channel<AgentEvent>,
    approvals: &ApprovalMap,
    answers: &AnswerMap,
    session_id: &str,
    ctx: &ToolContext,
    store: &SessionStore,
    steering: &Arc<SteeringCtl>,
    mode_ctl: &Arc<ModeCtl>,
) -> Result<(), String> {
    run_workflow_with_profile(
        config, history, user_message, attachment_blocks, event_tx, approvals, answers,
        session_id, ctx, store, steering, mode_ctl, PromptProfile::Standard,
    )
    .await
}

/// Same loop as `run_workflow`, with an explicit prompt/toolset profile.
/// `run_workflow` is the `Standard`-profile shorthand used by normal chat
/// sessions; dedicated flows (e.g. commit & push) call this directly.
#[allow(clippy::too_many_arguments)]
pub async fn run_workflow_with_profile(
    config: &AgentConfig,
    history: &mut Vec<Message>,
    user_message: String,
    attachment_blocks: Vec<ContentBlock>,
    event_tx: &Channel<AgentEvent>,
    approvals: &ApprovalMap,
    answers: &AnswerMap,
    session_id: &str,
    ctx: &ToolContext,
    store: &SessionStore,
    steering: &Arc<SteeringCtl>,
    mode_ctl: &Arc<ModeCtl>,
    profile: PromptProfile,
) -> Result<(), String> {
    reject_non_english(&user_message)?;
    store.try_append(&SessionRecord::User {
        text: user_message.clone(),
        ts: now_ms(),
    });
    let mut blocks = vec![ContentBlock::text(&user_message)];
    blocks.extend(attachment_blocks);
    push_user_blocks(history, store, blocks);

    let skill_mgr = crate::agent::skills::SkillManager::new(
        ctx.workspace_root.as_ref().map(std::path::PathBuf::from)
    );
    let skills_section = crate::agent::skills::build_skills_system_prompt_section(&skill_mgr);
    let (mut cur_mode, _) = mode_ctl.get();
    let mut system = system_prompt(ctx.workspace_root.as_deref(), skills_section.as_deref(), ctx.plan_save_path.as_deref(), cur_mode, profile);
    // MCP tool discovery already happened before `run_workflow` was called
    // (the caller awaits `ensure_mcp_connected`), so this is a cheap sync
    // snapshot read, not a fresh connection attempt.
    let mcp_defs = ctx.mcp.as_ref().map(|m| m.cached_defs()).unwrap_or_default();
    let mut tools = api_tools(cur_mode, profile, &mcp_defs, config);

    // Auto-compact when the context exceeds the threshold. Prefer the real
    // input_tokens the API reported for the last request; the char-based
    // estimate is the fallback (take the max of the two for safety).
    let records = crate::agent::persist::load_records(&store.path).unwrap_or_default();
    let estimated = estimate_tokens(history, &system, &tools)
        .max(crate::agent::persist::last_context_tokens(&records).unwrap_or(0));
    if estimated >= COMPACT_THRESHOLD {
        let _ = event_tx.send(AgentEvent::TextStep {
            text: format!(
                "__compact_start__:{}/{}",
                estimated / 1000,
                MAX_CONTEXT_TOKENS / 1000
            ),
        });
        match compact_history(config, store, ctx, event_tx, approvals, answers, session_id, steering).await {
            Ok(_) => {
                // Rebuild the history exactly as a session reload would:
                // summary + kept-verbatim tail (which already contains the
                // just-persisted user message) + nothing else.
                *history = crate::agent::persist::history_from_records(
                    &crate::agent::persist::load_records(&store.path).unwrap_or_default(),
                );
                let new_context = estimate_tokens(history, &system, &tools);
                let (ci, co, cc, cci, cco, ccc) = crate::agent::persist::cumulative_stats(
                    &crate::agent::persist::load_records(&store.path).unwrap_or_default(),
                );
                write_status(store, session_id, ci, co, cc, cci, cco, ccc, Some(new_context));
                let _ = event_tx.send(AgentEvent::SessionStats {
                    input_tokens: ci as u32,
                    output_tokens: co as u32,
                    cumulative_cost: cc,
                    cost_input: cci,
                    cost_output: cco,
                    cost_cache_read: ccc,
                    context_tokens: new_context,
                    max_context_tokens: MAX_CONTEXT_TOKENS,
                    compact_threshold: COMPACT_THRESHOLD,
                });
                let _ = event_tx.send(AgentEvent::TextStep {
                    text: format!(
                        "__compact_done__:{}/{}",
                        estimated / 1000,
                        new_context / 1000
                    ),
                });
            }
            Err(e) => {
                let _ = event_tx.send(AgentEvent::TextStep {
                    text: format!("__compact_fail__:{e}"),
                });
            }
        }
    }

    // Load cumulative totals from the last Status record
    let cumul = crate::agent::persist::cumulative_stats(
        &crate::agent::persist::load_records(&store.path).unwrap_or_default()
    );
    let mut cumul_in: u64 = cumul.0;
    let mut cumul_out: u64 = cumul.1;
    let mut cumul_cost: Option<f64> = cumul.2;
    let mut cumul_cost_input: Option<f64> = cumul.3;
    let mut cumul_cost_output: Option<f64> = cumul.4;
    let mut cumul_cost_cache: Option<f64> = cumul.5;

    let mut total_in: u32 = 0;
    let mut total_out: u32 = 0;
    let mut total_cache: u32 = 0;
    let mut run_cost: Option<f64> = None;
    let mut run_cost_input: Option<f64> = None;
    let mut run_cost_output: Option<f64> = None;
    let mut run_cost_cache: Option<f64> = None;
    let mut last_text = String::new();
    // Size of the context for the next request: the real number reported by
    // the API when available, the char-based estimate otherwise.
    let mut last_context: u64 = estimate_tokens(history, &system, &tools);
    let mut truncation_streak: u32 = 0;
    let mut empty_streak: u32 = 0;
    let mut unfinished_streak: u32 = 0;

    // Golden-goals loop state. The cycle counter resumes from the session's
    // records so a restart doesn't reset the cap mid-loop.
    let mut golden_cycle: u32 = crate::agent::persist::golden_cycle_count(
        &crate::agent::persist::load_records(&store.path).unwrap_or_default(),
    );
    let mut golden_last_pending: Vec<String> = Vec::new();
    let mut golden_stalls: usize = 0;

    // Plan-finalization state. `plan_finalized` flips when the agent calls the
    // finalize_plan tool this run; `finalize_nudged` bounds the enforcement to a
    // single reminder before the harness falls back to auto-appending the log.
    let mut plan_finalized = false;
    let mut finalize_nudged = false;

    // Brain progress guard: track consecutive rounds where the agent only uses
    // explore tools without interviewing (ask_user), writing a plan (write_plan),
    // or creating tasks (tasks_set). After BRAIN_EXPLORE_LIMIT rounds, inject
    // a system reminder redirecting the agent to the required deliverables.
    const BRAIN_EXPLORE_LIMIT: u32 = 4;
    let mut brain_explore_streak: u32 = 0;

    // Anchor the diff window at the true start of the plan's work: record the
    // git HEAD once per session (guarded), so finalize_plan can report every
    // changed file / commit since planning began, even across resumed runs.
    if let Some(sha) = ctx.base_commit.as_deref() {
        let already = crate::agent::persist::has_base_commit(
            &crate::agent::persist::load_records(&store.path).unwrap_or_default(),
        );
        if !already {
            store.try_append(&SessionRecord::BaseCommit {
                sha: sha.to_string(),
                ts: now_ms(),
            });
        }
    }

    let max_rounds = config.max_rounds.unwrap_or(usize::MAX);
    for _ in 0..max_rounds {
        // The mode can change mid-run (human toggle, or the agent's own
        // enter/exit_plan_mode in the previous round) — refresh the prompt
        // and tool list before each request.
        let (mode_now, _) = mode_ctl.get();
        if mode_now != cur_mode {
            cur_mode = mode_now;
            system = system_prompt(ctx.workspace_root.as_deref(), skills_section.as_deref(), ctx.plan_save_path.as_deref(), cur_mode, profile);
            tools = api_tools(cur_mode, profile, &mcp_defs, config);
        }

        // Per-round context re-check: tool_results from the previous round may
        // have pushed the history over the compact threshold. Compact before
        // the next LLM call so we never feed an oversized context.
        if estimate_tokens(history, &system, &tools) >= COMPACT_THRESHOLD {
            let _ = event_tx.send(AgentEvent::TextStep {
                text: format!(
                    "__compact_start__:{}/{}",
                    estimate_tokens(history, &system, &tools) / 1000,
                    MAX_CONTEXT_TOKENS / 1000
                ),
            });
            match compact_history(config, store, ctx, event_tx, approvals, answers, session_id, steering).await {
                Ok(_) => {
                    *history = crate::agent::persist::history_from_records(
                        &crate::agent::persist::load_records(&store.path).unwrap_or_default(),
                    );
                    // Mode/system/tools may have changed mid-compact, refresh.
                    let (mode_now2, _) = mode_ctl.get();
                    if mode_now2 != cur_mode {
                        cur_mode = mode_now2;
                        system = system_prompt(
                            ctx.workspace_root.as_deref(),
                            skills_section.as_deref(),
                            ctx.plan_save_path.as_deref(),
                            cur_mode,
                            profile,
                        );
                        tools = api_tools(cur_mode, profile, &mcp_defs, config);
                    }
                    let new_ctx = estimate_tokens(history, &system, &tools);
                    let (ci, co, cc, cci, cco, ccc) = crate::agent::persist::cumulative_stats(
                        &crate::agent::persist::load_records(&store.path).unwrap_or_default(),
                    );
                    write_status(store, session_id, ci, co, cc, cci, cco, ccc, Some(new_ctx));
                    let _ = event_tx.send(AgentEvent::SessionStats {
                        input_tokens: ci as u32,
                        output_tokens: co as u32,
                        cumulative_cost: cc,
                        cost_input: cci,
                        cost_output: cco,
                        cost_cache_read: ccc,
                        context_tokens: new_ctx,
                        max_context_tokens: MAX_CONTEXT_TOKENS,
                        compact_threshold: COMPACT_THRESHOLD,
                    });
                    let _ = event_tx.send(AgentEvent::TextStep {
                        text: format!(
                            "__compact_done__:{}/{}",
                            estimate_tokens(history, &system, &tools) / 1000,
                            new_ctx / 1000
                        ),
                    });
                }
                Err(e) => {
                    let _ = event_tx.send(AgentEvent::TextStep {
                        text: format!("__compact_fail__:{e}"),
                    });
                }
            }
        }

        let mut assistant_text = String::new();
        let resolved_model = config.model_for_mode(cur_mode.as_str());
        let stream_output = stream_message_with_retry(
            config,
            resolved_model,
            history,
            &tools,
            Some(system.as_str()),
            event_tx,
            session_id,
            &mut assistant_text,
            &steering.interrupt,
        )
        .await?;

        let text_output = assistant_text;
        let tool_uses = stream_output.tool_uses;
        let was_interrupted = stream_output.interrupted;
        if let Some(u) = &stream_output.usage {
            total_in += u.input_tokens;
            total_out += u.output_tokens;
            total_cache += u.cache_read_input_tokens;
            // Use provider-reported cost if available, otherwise estimate
            if let Some(c) = u.cost {
                run_cost = Some(run_cost.unwrap_or(0.0) + c);
            }
            if let Some(c) = u.cost_input {
                run_cost_input = Some(run_cost_input.unwrap_or(0.0) + c);
            }
            if let Some(c) = u.cost_output {
                run_cost_output = Some(run_cost_output.unwrap_or(0.0) + c);
            }
            if let Some(c) = u.cost_cache_read {
                run_cost_cache = Some(run_cost_cache.unwrap_or(0.0) + c);
            }
        }
        // Context for the next request = the history just sent + this round's
        // output. Providers behind a prefix cache (claudin.io) report only
        // cache-miss tokens in input_tokens — verified: a fully cached 3k
        // prompt reports input_tokens=74 — so the char-based estimate is the
        // floor and the API number can only raise it, never shrink it.
        let out_tok = stream_output
            .usage
            .as_ref()
            .map(|u| u.output_tokens as u64)
            .unwrap_or(0);
        let api_ctx = stream_output
            .usage
            .as_ref()
            .map(|u| (u.input_tokens + u.cache_read_input_tokens + u.output_tokens) as u64)
            .unwrap_or(0);
        last_context = (estimate_tokens(history, &system, &tools) + out_tok).max(api_ctx);

        // Live stats for the context bar
        let (round_ci, round_co, round_cc) = cost_or_estimate(
            resolved_model, total_in, total_cache, total_out,
            run_cost_input, run_cost_output, run_cost_cache,
        );
        let live_cost_input = cumul_cost_input.unwrap_or(0.0) + round_ci;
        let live_cost_output = cumul_cost_output.unwrap_or(0.0) + round_co;
        let live_cost_cache = cumul_cost_cache.unwrap_or(0.0) + round_cc;
        let _ = event_tx.send(AgentEvent::SessionStats {
            input_tokens: total_in + cumul_in as u32,
            output_tokens: total_out + cumul_out as u32,
            cumulative_cost: Some(live_cost_input + live_cost_output + live_cost_cache),
            cost_input: Some(live_cost_input),
            cost_output: Some(live_cost_output),
            cost_cache_read: Some(live_cost_cache),
            context_tokens: last_context,
            max_context_tokens: MAX_CONTEXT_TOKENS,
            compact_threshold: COMPACT_THRESHOLD,
        });

        // A — Interrupt no meio do stream: persistir texto parcial se não vazio,
        //     resetar flag, tentar steering ou pausar.
        if was_interrupted {
            if !text_output.is_empty() {
                push_turn(
                    history,
                    store,
                    Message {
                        role: "assistant".into(),
                        content: vec![ContentBlock::text(&text_output)],
                    },
                );
                last_text = text_output;
            }
            steering.interrupt.store(false, Ordering::SeqCst);
            if inject_steering(history, store, steering, event_tx) {
                continue;
            }
            if last_text.is_empty() {
                last_text = "Pausado pelo usuário.".into();
            }
            store.try_append(&SessionRecord::Done {
                input_tokens: total_in,
                output_tokens: total_out,
                ts: now_ms(),
            });
            cumul_in += total_in as u64;
            cumul_out += total_out as u64;
            roll_cost(
                resolved_model, total_in, total_cache, total_out,
                run_cost_input, run_cost_output, run_cost_cache,
                &mut cumul_cost, &mut cumul_cost_input, &mut cumul_cost_output, &mut cumul_cost_cache,
            );
            write_status(
                store, session_id, cumul_in, cumul_out, cumul_cost,
                cumul_cost_input, cumul_cost_output, cumul_cost_cache, Some(last_context),
            );
            let _ = event_tx.send(AgentEvent::Done {
                stop_reason: "interrupted".into(),
                text_output: last_text,
                input_tokens: total_in,
                output_tokens: total_out,
            });
            return Ok(());
        }

        // Truncated at the output-token cap (stop_reason "max_tokens"): the
        // model was cut off mid-generation, so an empty tool list here does
        // NOT mean the turn is done. Persist any partial text and nudge the
        // model to continue instead of silently abandoning the task. When
        // complete tool calls did come through, fall through and run them —
        // the loop continues naturally.
        let truncated = stream_output.stop_reason.as_deref() == Some("max_tokens");
        if truncated && tool_uses.is_empty() {
            truncation_streak += 1;
            if !text_output.is_empty() {
                push_turn(
                    history,
                    store,
                    Message {
                        role: "assistant".into(),
                        content: vec![ContentBlock::text(&text_output)],
                    },
                );
                last_text = text_output;
            }
            if truncation_streak < 3 {
                push_user_blocks(
                    history,
                    store,
                    vec![ContentBlock::text(
                        "[system] Your previous response was cut off at the output token \
                         limit before completing a tool call or final answer. Continue from \
                         where you stopped, working in smaller steps — if you were emitting \
                         a large tool call (e.g. a whole-file edit), split it into several \
                         smaller edits.",
                    )],
                );
                continue;
            }
            // Three consecutive fruitless truncations: stop honestly instead
            // of burning the whole round budget.
            if last_text.is_empty() {
                last_text = "A resposta estourou o limite de tokens repetidamente sem concluir. \
                             Tente dividir o pedido em partes menores."
                    .into();
            } else {
                last_text =
                    format!("{last_text}\n\n(Resposta truncada no limite de tokens — pode não estar completa.)");
            }
            store.try_append(&SessionRecord::Done {
                input_tokens: total_in,
                output_tokens: total_out,
                ts: now_ms(),
            });
            cumul_in += total_in as u64;
            cumul_out += total_out as u64;
            roll_cost(
                resolved_model, total_in, total_cache, total_out,
                run_cost_input, run_cost_output, run_cost_cache,
                &mut cumul_cost, &mut cumul_cost_input, &mut cumul_cost_output, &mut cumul_cost_cache,
            );
            write_status(
                store, session_id, cumul_in, cumul_out, cumul_cost,
                cumul_cost_input, cumul_cost_output, cumul_cost_cache, Some(last_context),
            );
            let _ = event_tx.send(AgentEvent::Done {
                stop_reason: "max_tokens".into(),
                text_output: last_text,
                input_tokens: total_in,
                output_tokens: total_out,
            });
            return Ok(());
        }
        if !truncated {
            truncation_streak = 0;
        }

        // Empty response: no text AND no tool calls, without truncation. A real
        // final turn always carries text, so mid-task this is a model glitch —
        // nudge it to continue instead of silently ending the run (same pattern
        // as the truncation nudge above). Give up after repeated empties.
        if tool_uses.is_empty() && text_output.is_empty() {
            empty_streak += 1;
            if empty_streak < 3 {
                push_user_blocks(
                    history,
                    store,
                    vec![ContentBlock::text(
                        "[system] Your previous response was empty (no text and no \
                         tool calls). If the task is not finished, continue from \
                         where you stopped; otherwise reply with a short final \
                         summary of what was done.",
                    )],
                );
                continue;
            }
        } else {
            empty_streak = 0;
        }

        // Terminal turn: no tool calls — the loop is done, this text is the reply.
        if tool_uses.is_empty() {
            if !text_output.is_empty() {
                push_turn(
                    history,
                    store,
                    Message {
                        role: "assistant".into(),
                        content: vec![ContentBlock::text(&text_output)],
                    },
                );
                last_text = text_output;
            }
            // B — Antes de encerrar, verificar steering. Se houver, continuar.
            if inject_steering(history, store, steering, event_tx) {
                continue;
            }
            // A terminal end_turn whose text only *announces* a next step
            // ("Primeiro vou confirmar…:", "Vou explorar com subagentes:") but
            // carries no tool call is a model glitch, not a finish — the agent
            // narrated its intent and stopped instead of acting, leaving the run
            // dangling mid-task. Ask the model itself (language-agnostic, no
            // hardcoded phrases) whether the turn is really done; if not, nudge
            // it to actually take the action. Bounded so a genuinely-final reply
            // the judge misreads can't loop forever.
            if !last_text.is_empty() && unfinished_streak < 2 {
                // Always judge with the Brain model (planning/reasoning), never
                // the Builder model, regardless of the session's current mode.
                let judge_model = config.model_for_mode(SessionMode::Brain.as_str());
                let verdict = judge_terminal_turn(config, judge_model, &last_text).await;
                let will_nudge = verdict == TurnVerdict::Continue;
                // Transparent to the user (no event emitted, UI renders nothing)
                // but auditable: persist the judge's decision to the JSONL.
                store.try_append(&SessionRecord::ContinuationJudge {
                    verdict: match verdict {
                        TurnVerdict::Continue => "continue".into(),
                        TurnVerdict::Done => "done".into(),
                    },
                    nudged: will_nudge,
                    streak: unfinished_streak + if will_nudge { 1 } else { 0 },
                    ts: now_ms(),
                });
                if will_nudge {
                    unfinished_streak += 1;
                    push_user_blocks(
                        history,
                        store,
                        vec![ContentBlock::text(
                            "[system] Your previous message announced a next step \
                             but ended without taking it — no tool call followed. \
                             Do not stop here. Continue now and actually perform the \
                             action you described: call the appropriate tool (e.g. \
                             ask the user, spawn the subagents, read the file, make \
                             the edit). If the task is genuinely complete, instead \
                             reply with a short final summary of what was done.",
                        )],
                    );
                    continue;
                }
            }
            // Golden verification: an end_turn with golden tasks still pending
            // is not a real finish — flip Brain↔Builder and send the model
            // back to work, bounded by the cycle and stall caps.
            let mut stop_reason = "end_turn";
            let golden_pending: Vec<String> = ctx
                .session_store_path
                .as_deref()
                .and_then(|p| {
                    crate::commands::tasks::load_last_tasks(std::path::Path::new(p)).ok()
                })
                .map(|t| crate::agent::tools::tasks::golden_pending_ids(&t))
                .unwrap_or_default();
            if !golden_pending.is_empty() {
                let max_cycles = config.max_golden_cycles.unwrap_or(DEFAULT_MAX_GOLDEN_CYCLES);
                let max_stalls = config.max_golden_stalls.unwrap_or(DEFAULT_MAX_GOLDEN_STALLS);
                if golden_pending == golden_last_pending {
                    golden_stalls += 1;
                } else {
                    golden_stalls = 0;
                }
                golden_last_pending = golden_pending.clone();
                if (golden_cycle as usize) < max_cycles && golden_stalls < max_stalls {
                    golden_cycle += 1;
                    let next = match cur_mode {
                        SessionMode::Brain => SessionMode::Builder,
                        SessionMode::Builder => SessionMode::Brain,
                    };
                    mode_ctl.set(next, ModeOrigin::Agent);
                    store.try_append(&SessionRecord::Mode {
                        mode: next.as_str().into(),
                        origin: ModeOrigin::Agent.as_str().into(),
                        ts: now_ms(),
                    });
                    let _ = event_tx.send(AgentEvent::ModeChanged {
                        mode: next.as_str().into(),
                        origin: ModeOrigin::Agent.as_str().into(),
                        reason: Some(format!("golden cycle {golden_cycle}")),
                    });
                    store.try_append(&SessionRecord::GoldenCycle {
                        cycle: golden_cycle,
                        mode: next.as_str().into(),
                        goals: golden_pending.clone(),
                        ts: now_ms(),
                    });
                    let _ = event_tx.send(AgentEvent::GoldenLoop {
                        cycle: golden_cycle,
                        max_cycles: max_cycles as u32,
                        pending: golden_pending.clone(),
                        mode: next.as_str().into(),
                    });
                    push_user_blocks(
                        history,
                        store,
                        vec![ContentBlock::text(format!(
                            "[system] Golden tasks are still pending: {}. The session \
                             switched to {} mode (golden cycle {golden_cycle}/{max_cycles}). \
                             Resume work on the pending goals — plan or execute what is \
                             missing, and only mark a golden task 'done' after verifying \
                             the goal is truly met.",
                            golden_pending.join(", "),
                            next.as_str(),
                        ))],
                    );
                    continue;
                }
                // Cap hit: stop honestly with a specific reason so the user
                // sees WHY the loop gave up with goals unmet.
                stop_reason = if golden_stalls >= max_stalls {
                    "golden_stalled"
                } else {
                    "max_golden_cycles"
                };
                last_text = format!(
                    "{last_text}\n\n⚠️ Golden goals not achieved ({}): {}",
                    if stop_reason == "golden_stalled" {
                        "no progress across consecutive cycles"
                    } else {
                        "cycle limit reached"
                    },
                    golden_pending.join(", "),
                );
            }

            // Feed the plan its Implementation Log when a goal-driven build
            // truly finishes: golden tasks existed and are all done (so this is
            // an honest end_turn, not a cap give-up), a plan file exists, and
            // finalize_plan wasn't called yet. One reminder, then the harness
            // auto-appends the log so the plan is ALWAYS fed. Fail-open: this
            // never blocks the finish.
            if !plan_finalized && stop_reason == "end_turn" {
                let tasks = ctx
                    .session_store_path
                    .as_deref()
                    .and_then(|p| {
                        crate::commands::tasks::load_last_tasks(std::path::Path::new(p)).ok()
                    })
                    .unwrap_or_default();
                let had_golden = tasks
                    .iter()
                    .any(crate::agent::tools::tasks::is_golden);
                let plan_exists =
                    crate::agent::tools::finalize_plan::latest_plan_file(ctx).is_some();
                if had_golden && plan_exists {
                    if !finalize_nudged {
                        finalize_nudged = true;
                        push_user_blocks(
                            history,
                            store,
                            vec![ContentBlock::text(
                                "[system] All goals are met. Before finishing, call \
                                 finalize_plan with a journal of your findings — it records the \
                                 changed files and commits into the plan for future reference. \
                                 This is the required last step."
                                    .to_string(),
                            )],
                        );
                        continue;
                    }
                    // The model still skipped it — record the log ourselves.
                    if let Some(outcome) =
                        crate::agent::tools::finalize_plan::auto_finalize(ctx)
                    {
                        let _ = event_tx.send(AgentEvent::TextStep {
                            text: format!(
                                "📝 Implementation Log recorded to {}",
                                outcome.plan_file
                            ),
                        });
                    }
                }
            }

            // If the model didn't produce a final text response, provide a
            // generic closing so the user doesn't see a blank answer.
            if last_text.is_empty() {
                last_text = "Pronto. Como posso ajudar mais?".into();
            }
            store.try_append(&SessionRecord::Done {
                input_tokens: total_in,
                output_tokens: total_out,
                ts: now_ms(),
            });
            cumul_in += total_in as u64;
            cumul_out += total_out as u64;
            roll_cost(
                resolved_model, total_in, total_cache, total_out,
                run_cost_input, run_cost_output, run_cost_cache,
                &mut cumul_cost, &mut cumul_cost_input, &mut cumul_cost_output, &mut cumul_cost_cache,
            );
            write_status(
                store, session_id, cumul_in, cumul_out, cumul_cost,
                cumul_cost_input, cumul_cost_output, cumul_cost_cache, Some(last_context),
            );
            let _ = event_tx.send(AgentEvent::Done {
                stop_reason: stop_reason.into(),
                text_output: last_text,
                input_tokens: total_in,
                output_tokens: total_out,
            });
            return Ok(());
        }

        // The model recovered and is taking an action — reset the dangling-promise
        // guard so the cap only counts *consecutive* announce-but-don't-act turns.
        unfinished_streak = 0;

        // The model wants to use tools: the assistant message carries the
        // (optional) text plus every tool_use block; the following user message
        // carries the paired tool_result blocks. Any text alongside tool calls is
        // an intermediate step (e.g. a stated plan) — surface it in the timeline.
        let mut tool_assistant_blocks: Vec<ContentBlock> = Vec::new();
        if !text_output.is_empty() {
            tool_assistant_blocks.push(ContentBlock::text(&text_output));
            let _ = event_tx.send(AgentEvent::TextStep {
                text: text_output.clone(),
            });
        }
        let mut tool_result_blocks: Vec<ContentBlock> = Vec::new();

        for (ti, tool_use) in tool_uses.iter().enumerate() {
            // C — Entre tools: checar interrupt. Se setado, sintetizar
            // tool_result "interrompido" para este e todos os tool_uses restantes.
            if steering.interrupt.load(Ordering::SeqCst) {
                for remaining in tool_uses.iter().skip(ti) {
                    let tid = remaining
                        .get("id")
                        .and_then(|v| v.as_str())
                        .unwrap_or("")
                        .to_string();
                    let tname = remaining
                        .get("name")
                        .and_then(|v| v.as_str())
                        .unwrap_or("")
                        .to_string();
                    tool_assistant_blocks.push(ContentBlock::tool_use(
                        &tid,
                        &tname,
                        remaining.get("input").cloned().unwrap_or(Value::Null),
                    ));
                    let msg = "Interrompido pelo usuário — ferramenta não executada.";
                    let _ = event_tx.send(AgentEvent::ToolResult {
                        tool_id: tid.clone(),
                        tool_name: tname,
                        output: msg.into(),
                        error: None,
                    });
                    tool_result_blocks.push(ContentBlock::tool_result(&tid, msg));
                }
                break;
            }

            let tool_use_id = tool_use
                .get("id")
                .and_then(|v| v.as_str())
                .unwrap_or("")
                .to_string();
            let tool_name = tool_use
                .get("name")
                .and_then(|v| v.as_str())
                .unwrap_or("")
                .to_string();
            let tool_input = tool_use.get("input").cloned().unwrap_or(Value::Null);

            tool_assistant_blocks.push(ContentBlock::tool_use(
                &tool_use_id,
                &tool_name,
                tool_input.clone(),
            ));

            let in_brain = matches!(mode_ctl.get().0, SessionMode::Brain);
            let block = if tool_name == "enter_plan_mode" || tool_name == "exit_plan_mode" {
                handle_mode_switch(
                    &tool_name,
                    &tool_use_id,
                    &tool_input,
                    mode_ctl,
                    store,
                    event_tx,
                    session_id,
                )
            } else if tool_name == "spawn_agents" {
                // Brain may only spawn read-only subagents.
                let tool_input = if in_brain {
                    force_explore_mode(tool_input)
                } else {
                    tool_input
                };
                let (block, sub_in, sub_out) = subagent::run_spawn_agents(
                    config,
                    ctx,
                    &tool_use_id,
                    tool_input,
                    event_tx,
                    approvals,
                    answers,
                    session_id,
                    steering,
                )
                .await;
                total_in += sub_in;
                total_out += sub_out;
                block
            } else if in_brain && tool_name == "edit_file" {
                deny_tool(
                    &tool_name,
                    &tool_use_id,
                    &tool_input,
                    "edit_file is not available in Brain mode — it is read-only. \
                     Record the intended change in the plan (write_plan) instead.",
                    event_tx,
                    session_id,
                )
            } else if in_brain
                && tool_name == "bash"
                && !matches!(
                    permissions::bash_permission(
                        tool_input.get("command").and_then(|v| v.as_str()).unwrap_or(""),
                        false
                    ),
                    PermissionLevel::Auto
                )
            {
                deny_tool(
                    &tool_name,
                    &tool_use_id,
                    &tool_input,
                    "Command not allowed in Brain mode — only read-only allowlisted \
                     commands (git status/diff/log, ls, cat, cargo check, ...) run here.",
                    event_tx,
                    session_id,
                )
            } else {
                run_tool(
                    &tool_name,
                    &tool_use_id,
                    tool_input,
                    permissions::tool_permission(&tool_name),
                    event_tx,
                    approvals,
                    answers,
                    session_id,
                    ctx,
                    config,
                )
                .await
            };
            // Note a successful finalize_plan so the golden-completion gate
            // knows the plan was fed and skips the reminder / fallback.
            if tool_name == "finalize_plan" {
                if let ContentBlock::ToolResult { content, .. } = &block {
                    if !content.starts_with("Error") {
                        plan_finalized = true;
                    }
                }
            }
            tool_result_blocks.push(block);
        }

        push_turn(
            history,
            store,
            Message {
                role: "assistant".into(),
                content: tool_assistant_blocks,
            },
        );
        push_turn(
            history,
            store,
            Message {
                role: "user".into(),
                content: tool_result_blocks,
            },
        );

        // Brain progress guard: if the agent is in Brain mode and only using
        // explore tools without interviewing/planning/tasking, redirect it.
        let is_brain = matches!(mode_ctl.get().0, SessionMode::Brain);
        if is_brain {
            let explore_tools = [
                "spawn_agents", "semantic_search", "code_search", "grep",
                "read_file", "file_outline", "list_dir", "symbol_lookup",
                "go_to_definition", "find_references",
            ];
            let is_explore_only = tool_uses.iter().all(|t| {
                let name = t.get("name").and_then(|v| v.as_str()).unwrap_or("");
                explore_tools.contains(&name)
            });
            let has_progress_tool = tool_uses.iter().any(|t| {
                let name = t.get("name").and_then(|v| v.as_str()).unwrap_or("");
                name == "ask_user" || name == "write_plan" || name == "tasks_set"
            });

            if has_progress_tool {
                brain_explore_streak = 0;
            } else if is_explore_only {
                brain_explore_streak += 1;
                if brain_explore_streak >= BRAIN_EXPLORE_LIMIT {
                    brain_explore_streak = 0; // Reset so we don't loop-spam
                    push_user_blocks(
                        history,
                        store,
                        vec![ContentBlock::text(
                            "[system] You have been exploring for several consecutive rounds \
                             in Brain mode without making progress on the required deliverables. \
                             Brain mode is for planning — you MUST call ask_user to interview \
                             the user, then write_plan to create the plan, then tasks_set to \
                             create executable tasks. Do not continue exploring until you have \
                             gathered the information you need.",
                        )],
                    );
                    continue;
                }
            } else {
                // Some mixed tools that aren't purely explore — don't count.
            }
        }

        // Interrupt check after tool results.
        if steering.interrupt.swap(false, Ordering::SeqCst) {
            if inject_steering(history, store, steering, event_tx) {
                continue;
            }
            if last_text.is_empty() {
                last_text = "Pausado pelo usuário.".into();
            }
            store.try_append(&SessionRecord::Done {
                input_tokens: total_in,
                output_tokens: total_out,
                ts: now_ms(),
            });
            cumul_in += total_in as u64;
            cumul_out += total_out as u64;
            roll_cost(
                resolved_model, total_in, total_cache, total_out,
                run_cost_input, run_cost_output, run_cost_cache,
                &mut cumul_cost, &mut cumul_cost_input, &mut cumul_cost_output, &mut cumul_cost_cache,
            );
            write_status(
                store, session_id, cumul_in, cumul_out, cumul_cost,
                cumul_cost_input, cumul_cost_output, cumul_cost_cache, Some(last_context),
            );
            let _ = event_tx.send(AgentEvent::Done {
                stop_reason: "interrupted".into(),
                text_output: last_text,
                input_tokens: total_in,
                output_tokens: total_out,
            });
            return Ok(());
        }
        inject_steering(history, store, steering, event_tx);
    }

    // Safety cap hit: stop looping and report what we have so far rather than
    // running forever. Only reachable when config.max_rounds is set.
    let capped_text = if last_text.is_empty() {
        format!("Parei após {max_rounds} rounds de ferramentas sem concluir. Tente reformular o pedido em partes menores.")
    } else {
        format!("{last_text}\n\n(Parei após {max_rounds} rounds de ferramentas — pode não estar completo.)")
    };
    store.try_append(&SessionRecord::Done {
        input_tokens: total_in,
        output_tokens: total_out,
        ts: now_ms(),
    });
    cumul_in += total_in as u64;
    cumul_out += total_out as u64;
    roll_cost(
        config.model_for_mode(cur_mode.as_str()), total_in, total_cache, total_out,
        run_cost_input, run_cost_output, run_cost_cache,
        &mut cumul_cost, &mut cumul_cost_input, &mut cumul_cost_output, &mut cumul_cost_cache,
    );
    write_status(
        store, session_id, cumul_in, cumul_out, cumul_cost,
        cumul_cost_input, cumul_cost_output, cumul_cost_cache, Some(last_context),
    );
    let _ = event_tx.send(AgentEvent::Done {
        stop_reason: "max_rounds".into(),
        text_output: capped_text,
        input_tokens: total_in,
        output_tokens: total_out,
    });
    Ok(())
}

/// Execute one tool call (honoring its permission level) and return the
/// `tool_result` block to feed back to the model. Emits the matching UI events.
/// When `yolo_mode` is true and the tool is not in `yolo_blacklist`, tools that
/// normally require approval are auto-approved instead.
#[allow(clippy::too_many_arguments)]
pub(crate) async fn run_tool(
    tool_name: &str,
    tool_use_id: &str,
    tool_input: Value,
    perm: permissions::PermissionLevel,
    event_tx: &Channel<AgentEvent>,
    approvals: &ApprovalMap,
    answers: &AnswerMap,
    session_id: &str,
    ctx: &ToolContext,
    config: &AgentConfig,
) -> ContentBlock {
    // ask_user is inherently interactive: it never executes anything, it blocks
    // until the user answers in the UI (or the request is dropped).
    if tool_name == "ask_user" {
        return ask_user(tool_name, tool_use_id, tool_input, event_tx, answers, session_id).await;
    }

    // YOLO mode: auto-approve tools not on the blacklist, treating
    // `RequiresApproval` as `Auto` (bash still checks its allowlist/deny-list).
    let effective_perm = if config.yolo_mode
        && matches!(perm, PermissionLevel::RequiresApproval)
        && !config.yolo_blacklist.iter().any(|b| b == tool_name)
    {
        if tool_name == "bash" {
            let command = tool_input
                .get("command")
                .and_then(|v| v.as_str())
                .unwrap_or("");
            match permissions::bash_permission(command, ctx.auto_approve_git) {
                PermissionLevel::Denied => PermissionLevel::Denied,
                _ => PermissionLevel::Auto,
            }
        } else {
            PermissionLevel::Auto
        }
    } else {
        perm
    };

    match effective_perm {
        permissions::PermissionLevel::Auto => {
            let _ = event_tx.send(AgentEvent::ToolCall {
                session_id: session_id.to_string(),
                tool_id: tool_use_id.to_string(),
                tool_name: tool_name.to_string(),
                args: tool_input.clone(),
                permission: "auto".into(),
                edit_proposal: None,
            });

            match tools::execute(tool_name, tool_input, ctx).await {
                Ok(ToolOutput::Text { content }) => {
                    let truncated = truncate(&content, 2000);
                    let _ = event_tx.send(AgentEvent::ToolResult {
                        tool_id: tool_use_id.to_string(),
                        tool_name: tool_name.to_string(),
                        output: truncated,
                        error: None,
                    });
                    tool_result_block(tool_use_id, &content)
                }
                Ok(ToolOutput::EditProposal { path, old_string, new_string, unified_diff }) => {
                    let proposal = EditProposalData {
                        path: path.clone(),
                        old_string: old_string.clone(),
                        new_string: new_string.clone(),
                        unified_diff,
                    };
                    // Reconstruct args since tool_input was moved into execute()
                    let args = serde_json::json!({
                        "path": path,
                        "old_string": old_string,
                        "new_string": new_string,
                    });
                    let _ = event_tx.send(AgentEvent::ToolCall {
                        session_id: session_id.to_string(),
                        tool_id: tool_use_id.to_string(),
                        tool_name: tool_name.to_string(),
                        args: args.clone(),
                        permission: "auto".into(),
                        edit_proposal: Some(proposal),
                    });
                    match tools::apply_edit_with_ctx(args, ctx).await {
                        Ok(msg) => {
                            let _ = event_tx.send(AgentEvent::ToolResult {
                                tool_id: tool_use_id.to_string(),
                                tool_name: tool_name.to_string(),
                                output: msg.clone(),
                                error: None,
                            });
                            ContentBlock::tool_result(tool_use_id, &msg)
                        }
                        Err(e) => {
                            let _ = event_tx.send(AgentEvent::ToolResult {
                                tool_id: tool_use_id.to_string(),
                                tool_name: tool_name.to_string(),
                                output: String::new(),
                                error: Some(e.clone()),
                            });
                            ContentBlock::tool_result(tool_use_id, &format!("Error applying: {e}"))
                        }
                    }
                }
                Err(e) => {
                    let _ = event_tx.send(AgentEvent::ToolResult {
                        tool_id: tool_use_id.to_string(),
                        tool_name: tool_name.to_string(),
                        output: String::new(),
                        error: Some(e.clone()),
                    });
                    ContentBlock::tool_result(tool_use_id, &format!("Error: {e}"))
                }
            }
        }
        permissions::PermissionLevel::RequiresApproval if tool_name == "bash" => {
            let command = tool_input
                .get("command")
                .and_then(|v| v.as_str())
                .unwrap_or("");

            match permissions::bash_permission(command, ctx.auto_approve_git) {
                permissions::PermissionLevel::Denied => {
                    let msg = format!("Command blocked by security policy: {command}");
                    let _ = event_tx.send(AgentEvent::ToolCall {
                        session_id: session_id.to_string(),
                        tool_id: tool_use_id.to_string(),
                        tool_name: tool_name.to_string(),
                        args: tool_input.clone(),
                        permission: "denied".into(),
                        edit_proposal: None,
                    });
                    let _ = event_tx.send(AgentEvent::ToolResult {
                        tool_id: tool_use_id.to_string(),
                        tool_name: tool_name.to_string(),
                        output: msg.clone(),
                        error: Some("denied".into()),
                    });
                    ContentBlock::tool_result(tool_use_id, &msg)
                }
                permissions::PermissionLevel::Auto => {
                    let _ = event_tx.send(AgentEvent::ToolCall {
                        session_id: session_id.to_string(),
                        tool_id: tool_use_id.to_string(),
                        tool_name: tool_name.to_string(),
                        args: tool_input.clone(),
                        permission: "auto".into(),
                        edit_proposal: None,
                    });
                    match tools::execute(tool_name, tool_input.clone(), ctx).await {
                        Ok(ToolOutput::Text { content }) => {
                            let truncated = truncate(&content, 2000);
                            let _ = event_tx.send(AgentEvent::ToolResult {
                                tool_id: tool_use_id.to_string(),
                                tool_name: tool_name.to_string(),
                                output: truncated,
                                error: None,
                            });
                            tool_result_block(tool_use_id, &content)
                        }
                        _ => {
                            let err = "unexpected output type from bash".to_string();
                            let _ = event_tx.send(AgentEvent::ToolResult {
                                tool_id: tool_use_id.to_string(),
                                tool_name: tool_name.to_string(),
                                output: err.clone(),
                                error: Some("unexpected".into()),
                            });
                            ContentBlock::tool_result(tool_use_id, &err)
                        }
                    }
                }
                permissions::PermissionLevel::RequiresApproval => {
                    let approval_key = format!("{session_id}:{tool_use_id}");
                    let (approve_tx, approve_rx) = oneshot::channel::<bool>();
                    {
                        let mut map = approvals.lock().await;
                        map.insert(approval_key.clone(), approve_tx);
                    }

                    let _ = event_tx.send(AgentEvent::ToolCall {
                        session_id: session_id.to_string(),
                        tool_id: tool_use_id.to_string(),
                        tool_name: tool_name.to_string(),
                        args: tool_input.clone(),
                        permission: "requires_approval".into(),
                        edit_proposal: None,
                    });

                    match approve_rx.await {
                        Ok(true) => match tools::execute(tool_name, tool_input.clone(), ctx).await {
                            Ok(ToolOutput::Text { content }) => {
                                let truncated = truncate(&content, 2000);
                                let _ = event_tx.send(AgentEvent::ToolResult {
                                    tool_id: tool_use_id.to_string(),
                                    tool_name: tool_name.to_string(),
                                    output: truncated,
                                    error: None,
                                });
                                tool_result_block(tool_use_id, &content)
                            }
                            Ok(ToolOutput::EditProposal { .. }) => {
                                let err_msg: String =
                                    "bash should not produce edit proposals".into();
                                let _ = event_tx.send(AgentEvent::ToolResult {
                                    tool_id: tool_use_id.to_string(),
                                    tool_name: tool_name.to_string(),
                                    output: err_msg.clone(),
                                    error: Some("unexpected output type".into()),
                                });
                                ContentBlock::tool_result(tool_use_id, &err_msg)
                            }
                            Err(e) => {
                                let _ = event_tx.send(AgentEvent::ToolResult {
                                    tool_id: tool_use_id.to_string(),
                                    tool_name: tool_name.to_string(),
                                    output: String::new(),
                                    error: Some(e.clone()),
                                });
                                ContentBlock::tool_result(tool_use_id, &format!("Error: {e}"))
                            }
                        },
                        Ok(false) => {
                            let msg = "Command rejected by user".to_string();
                            let _ = event_tx.send(AgentEvent::ToolResult {
                                tool_id: tool_use_id.to_string(),
                                tool_name: tool_name.to_string(),
                                output: msg.clone(),
                                error: None,
                            });
                            ContentBlock::tool_result(tool_use_id, &msg)
                        }
                        Err(_) => {
                            ContentBlock::tool_result(tool_use_id, "Approval channel closed")
                        }
                    }
                }
            }
        }
        // MCP tools return `ToolOutput::Text`, not `EditProposal` — the
        // generic `RequiresApproval` arm below executes Text-producing tools
        // BEFORE approval (it only gates edit proposals), so MCP needs its
        // own approve-before-execute arm here, same shape as the bash one.
        permissions::PermissionLevel::RequiresApproval if tool_name.starts_with("mcp__") => {
            let approval_key = format!("{session_id}:{tool_use_id}");
            let (approve_tx, approve_rx) = oneshot::channel::<bool>();
            {
                let mut map = approvals.lock().await;
                map.insert(approval_key.clone(), approve_tx);
            }

            let _ = event_tx.send(AgentEvent::ToolCall {
                session_id: session_id.to_string(),
                tool_id: tool_use_id.to_string(),
                tool_name: tool_name.to_string(),
                args: tool_input.clone(),
                permission: "requires_approval".into(),
                edit_proposal: None,
            });

            match approve_rx.await {
                Ok(true) => match tools::execute(tool_name, tool_input.clone(), ctx).await {
                    Ok(ToolOutput::Text { content }) => {
                        let truncated = truncate(&content, 2000);
                        let _ = event_tx.send(AgentEvent::ToolResult {
                            tool_id: tool_use_id.to_string(),
                            tool_name: tool_name.to_string(),
                            output: truncated,
                            error: None,
                        });
                        tool_result_block(tool_use_id, &content)
                    }
                    Ok(ToolOutput::EditProposal { .. }) => {
                        let err_msg = "MCP tools should not produce edit proposals".to_string();
                        let _ = event_tx.send(AgentEvent::ToolResult {
                            tool_id: tool_use_id.to_string(),
                            tool_name: tool_name.to_string(),
                            output: err_msg.clone(),
                            error: Some("unexpected output type".into()),
                        });
                        ContentBlock::tool_result(tool_use_id, &err_msg)
                    }
                    Err(e) => {
                        let _ = event_tx.send(AgentEvent::ToolResult {
                            tool_id: tool_use_id.to_string(),
                            tool_name: tool_name.to_string(),
                            output: String::new(),
                            error: Some(e.clone()),
                        });
                        ContentBlock::tool_result(tool_use_id, &format!("Error: {e}"))
                    }
                },
                Ok(false) => {
                    let msg = "Tool call rejected by user".to_string();
                    let _ = event_tx.send(AgentEvent::ToolResult {
                        tool_id: tool_use_id.to_string(),
                        tool_name: tool_name.to_string(),
                        output: msg.clone(),
                        error: None,
                    });
                    ContentBlock::tool_result(tool_use_id, &msg)
                }
                Err(_) => ContentBlock::tool_result(tool_use_id, "Approval channel closed"),
            }
        }
        permissions::PermissionLevel::RequiresApproval => {
            match tools::execute(tool_name, tool_input.clone(), ctx).await {
                Ok(ToolOutput::Text { content }) => {
                    let _ = event_tx.send(AgentEvent::ToolResult {
                        tool_id: tool_use_id.to_string(),
                        tool_name: tool_name.to_string(),
                        output: content.clone(),
                        error: None,
                    });
                    tool_result_block(tool_use_id, &content)
                }
                Ok(ToolOutput::EditProposal {
                    path,
                    old_string,
                    new_string,
                    unified_diff,
                }) => {
                    let proposal = EditProposalData {
                        path,
                        old_string,
                        new_string,
                        unified_diff,
                    };

                    let approval_key = format!("{session_id}:{tool_use_id}");
                    let (approve_tx, approve_rx) = oneshot::channel::<bool>();
                    {
                        let mut map = approvals.lock().await;
                        map.insert(approval_key.clone(), approve_tx);
                    }

                    let _ = event_tx.send(AgentEvent::ToolCall {
                        session_id: session_id.to_string(),
                        tool_id: tool_use_id.to_string(),
                        tool_name: tool_name.to_string(),
                        args: tool_input.clone(),
                        permission: "requires_approval".into(),
                        edit_proposal: Some(proposal),
                    });

                    match approve_rx.await {
                        Ok(true) => match tools::apply_edit_with_ctx(tool_input, ctx).await {
                            Ok(msg) => {
                                let _ = event_tx.send(AgentEvent::ToolResult {
                                    tool_id: tool_use_id.to_string(),
                                    tool_name: tool_name.to_string(),
                                    output: msg.clone(),
                                    error: None,
                                });
                                ContentBlock::tool_result(tool_use_id, &msg)
                            }
                            Err(e) => {
                                let _ = event_tx.send(AgentEvent::ToolResult {
                                    tool_id: tool_use_id.to_string(),
                                    tool_name: tool_name.to_string(),
                                    output: String::new(),
                                    error: Some(e.clone()),
                                });
                                ContentBlock::tool_result(tool_use_id, &format!("Error applying: {e}"))
                            }
                        },
                        Ok(false) => {
                            let msg = "Edit rejected by user".to_string();
                            let _ = event_tx.send(AgentEvent::ToolResult {
                                tool_id: tool_use_id.to_string(),
                                tool_name: tool_name.to_string(),
                                output: msg.clone(),
                                error: None,
                            });
                            ContentBlock::tool_result(tool_use_id, &msg)
                        }
                        Err(_) => ContentBlock::tool_result(tool_use_id, "Approval channel closed"),
                    }
                }
                Err(e) => {
                    let _ = event_tx.send(AgentEvent::ToolResult {
                        tool_id: tool_use_id.to_string(),
                        tool_name: tool_name.to_string(),
                        output: String::new(),
                        error: Some(e.clone()),
                    });
                    ContentBlock::tool_result(tool_use_id, &format!("Error: {e}"))
                }
            }
        }
        permissions::PermissionLevel::Denied => {
            let msg = format!("Command '{tool_name}' is blocked by security policy");
            let _ = event_tx.send(AgentEvent::ToolCall {
                session_id: session_id.to_string(),
                tool_id: tool_use_id.to_string(),
                tool_name: tool_name.to_string(),
                args: tool_input.clone(),
                permission: "denied".into(),
                edit_proposal: None,
            });
            let _ = event_tx.send(AgentEvent::ToolResult {
                tool_id: tool_use_id.to_string(),
                tool_name: tool_name.to_string(),
                output: msg.clone(),
                error: Some("denied".into()),
            });
            ContentBlock::tool_result(tool_use_id, &msg)
        }
    }
}

/// Emit a denied ToolCall/ToolResult pair and return the tool_result block.
/// Used for tools blocked by the current mode (no approval prompt — a hard no).
fn deny_tool(
    tool_name: &str,
    tool_use_id: &str,
    tool_input: &Value,
    msg: &str,
    event_tx: &Channel<AgentEvent>,
    session_id: &str,
) -> ContentBlock {
    let _ = event_tx.send(AgentEvent::ToolCall {
        session_id: session_id.to_string(),
        tool_id: tool_use_id.to_string(),
        tool_name: tool_name.to_string(),
        args: tool_input.clone(),
        permission: "denied".into(),
        edit_proposal: None,
    });
    let _ = event_tx.send(AgentEvent::ToolResult {
        tool_id: tool_use_id.to_string(),
        tool_name: tool_name.to_string(),
        output: msg.to_string(),
        error: Some("denied".into()),
    });
    ContentBlock::tool_result(tool_use_id, msg)
}

/// Rewrite a spawn_agents input so every agent runs in 'explore' mode.
/// Brain must never spawn code-mode (write-capable) subagents.
/// Normalizes flattened single-spec inputs first so they can't bypass the
/// rewrite and reach run_spawn_agents still carrying mode='code'.
fn force_explore_mode(tool_input: Value) -> Value {
    let mut tool_input = subagent::normalize_spawn_input(tool_input);
    if let Some(agents) = tool_input.get_mut("agents").and_then(|v| v.as_array_mut()) {
        for agent in agents {
            if let Some(obj) = agent.as_object_mut() {
                obj.insert("mode".into(), Value::String("explore".into()));
            }
        }
    }
    tool_input
}

/// Handle the agent-initiated mode switch tools. enter_plan_mode always
/// works (origin becomes Agent); exit_plan_mode only works when the agent
/// itself entered Brain — a human-initiated Brain is exited only by
/// the human toggle.
fn handle_mode_switch(
    tool_name: &str,
    tool_use_id: &str,
    tool_input: &Value,
    mode_ctl: &Arc<ModeCtl>,
    store: &SessionStore,
    event_tx: &Channel<AgentEvent>,
    session_id: &str,
) -> ContentBlock {
    let _ = event_tx.send(AgentEvent::ToolCall {
        session_id: session_id.to_string(),
        tool_id: tool_use_id.to_string(),
        tool_name: tool_name.to_string(),
        args: tool_input.clone(),
        permission: "auto".into(),
        edit_proposal: None,
    });

    let (mode, origin) = mode_ctl.get();
    let (result, error): (String, Option<String>) = match tool_name {
        "enter_plan_mode" => {
            if mode == SessionMode::Brain {
                ("Already in Brain mode.".into(), Some("invalid".into()))
            } else {
                let reason = tool_input
                    .get("reason")
                    .and_then(|v| v.as_str())
                    .map(|s| s.to_string());
                mode_ctl.set(SessionMode::Brain, ModeOrigin::Agent);
                store.try_append(&SessionRecord::Mode {
                    mode: SessionMode::Brain.as_str().into(),
                    origin: ModeOrigin::Agent.as_str().into(),
                    ts: now_ms(),
                });
                let _ = event_tx.send(AgentEvent::ModeChanged {
                    mode: SessionMode::Brain.as_str().into(),
                    origin: ModeOrigin::Agent.as_str().into(),
                    reason,
                });
                (
                    "Entered Brain mode (read-only planning). Editing tools are now \
                     disabled. Explore, interview the user, write the plan with write_plan, \
                     create tasks with tasks_set, then call exit_plan_mode to build."
                        .into(),
                    None,
                )
            }
        }
        "exit_plan_mode" => {
            if mode != SessionMode::Brain {
                ("Not in Brain mode.".into(), Some("invalid".into()))
            } else if origin != ModeOrigin::Agent {
                (
                    "The USER enabled Brain mode — only they can switch back to \
                     Builder. Finish the plan and tasks, then end your turn telling \
                     the user everything is ready for them to flip the toggle."
                        .into(),
                    Some("denied".into()),
                )
            } else {
                mode_ctl.set(SessionMode::Builder, ModeOrigin::Agent);
                store.try_append(&SessionRecord::Mode {
                    mode: SessionMode::Builder.as_str().into(),
                    origin: ModeOrigin::Agent.as_str().into(),
                    ts: now_ms(),
                });
                let _ = event_tx.send(AgentEvent::ModeChanged {
                    mode: SessionMode::Builder.as_str().into(),
                    origin: ModeOrigin::Agent.as_str().into(),
                    reason: None,
                });
                (
                    "Back in Builder mode. Read the plan and execute the tasks \
                     (one code-mode subagent per task where possible)."
                        .into(),
                    None,
                )
            }
        }
        _ => ("unknown mode tool".into(), Some("invalid".into())),
    };

    let _ = event_tx.send(AgentEvent::ToolResult {
        tool_id: tool_use_id.to_string(),
        tool_name: tool_name.to_string(),
        output: result.clone(),
        error,
    });
    ContentBlock::tool_result(tool_use_id, &result)
}

/// Handle the ask_user tool: surface the questions in the UI, wait for the
/// user's answers and return them to the model as compiled question/answer
/// pairs. The ToolCall/ToolResult events keep the step visible in the timeline
/// and the tool_result block persists the answers in the session history.
async fn ask_user(
    tool_name: &str,
    tool_use_id: &str,
    tool_input: Value,
    event_tx: &Channel<AgentEvent>,
    answers: &AnswerMap,
    session_id: &str,
) -> ContentBlock {
    let _ = event_tx.send(AgentEvent::ToolCall {
        session_id: session_id.to_string(),
        tool_id: tool_use_id.to_string(),
        tool_name: tool_name.to_string(),
        args: tool_input.clone(),
        permission: "auto".into(),
        edit_proposal: None,
    });

    let questions = tool_input.get("questions").cloned().unwrap_or(Value::Null);
    if !questions.is_array() {
        let msg = "invalid ask_user input: 'questions' must be an array".to_string();
        let _ = event_tx.send(AgentEvent::ToolResult {
            tool_id: tool_use_id.to_string(),
            tool_name: tool_name.to_string(),
            output: String::new(),
            error: Some(msg.clone()),
        });
        return ContentBlock::tool_result(tool_use_id, &format!("Error: {msg}"));
    }

    let approval_key = format!("{session_id}:{tool_use_id}");
    let (answer_tx, answer_rx) = oneshot::channel::<Vec<UserAnswer>>();
    {
        let mut map = answers.lock().await;
        map.insert(approval_key, answer_tx);
    }

    let _ = event_tx.send(AgentEvent::AskUser {
        session_id: session_id.to_string(),
        tool_id: tool_use_id.to_string(),
        questions,
    });

    let compiled = match answer_rx.await {
        Ok(user_answers) => user_answers
            .iter()
            .map(|a| format!("Pergunta: {}\nResposta: {}", a.question, a.answer))
            .collect::<Vec<_>>()
            .join("\n\n"),
        Err(_) => "O usuário não respondeu.".to_string(),
    };

    let _ = event_tx.send(AgentEvent::ToolResult {
        tool_id: tool_use_id.to_string(),
        tool_name: tool_name.to_string(),
        output: compiled.clone(),
        error: None,
    });
    ContentBlock::tool_result(tool_use_id, &compiled)
}

/// Maximum chars for a tool_result stored in the conversation history.
/// Prevents a large subagent report or file read from blowing up the context.
const MAX_TOOL_RESULT_CHARS: usize = 24_000;

fn truncate(s: &str, max: usize) -> String {
    if s.len() > max {
        // Respect char boundaries so we never slice mid-codepoint.
        let mut end = max;
        while end < s.len() && !s.is_char_boundary(end) {
            end += 1;
        }
        format!("{}...(truncated, {} chars total)", &s[..end], s.len())
    } else {
        s.to_string()
    }
}

/// Build a truncated tool_result block for the conversation history.
/// The event stream already truncates to MAX_EVENT_CHARS (~2k) for display;
/// this cap limits the history copy so a giant tool result (e.g. a subagent
/// report, file read, or search) can't blow up the context.
fn tool_result_block(tool_use_id: &str, content: &str) -> ContentBlock {
    ContentBlock::tool_result(tool_use_id, &truncate(content, MAX_TOOL_RESULT_CHARS))
}

#[cfg(test)]
mod tests {
    use super::*;
    use serde_json::json;

    fn tmp_store() -> SessionStore {
        SessionStore {
            path: std::env::temp_dir().join(format!("claudinio_test_{}.jsonl", now_ms())),
        }
    }

    #[test]
    fn force_explore_mode_rewrites_all_agents() {
        let input = json!({ "agents": [
            { "name": "a", "goal": "g", "mode": "code" },
            { "name": "b", "goal": "g", "mode": "explore" }
        ]});
        let out = force_explore_mode(input);
        for agent in out["agents"].as_array().unwrap() {
            assert_eq!(agent["mode"], "explore");
        }
    }

    #[test]
    fn force_explore_mode_normalizes_flattened_code_spec() {
        // A flattened single spec must not bypass the explore rewrite.
        let input = json!({ "name": "a", "goal": "g", "mode": "code" });
        let out = force_explore_mode(input);
        let agents = out["agents"].as_array().unwrap();
        assert_eq!(agents.len(), 1);
        assert_eq!(agents[0]["mode"], "explore");
    }

    #[test]
    fn budget_exceeded_is_not_retryable() {
        let msg = format!(
            "{}Claudinio: Budget exceeded for window '1h'.",
            crate::agent::provider::BUDGET_EXCEEDED_MARKER
        );
        assert!(!is_retryable_error(&msg));
        // sanity: transient 500 stays retryable
        assert!(is_retryable_error("API error: HTTP 500"));
    }

    #[test]
    fn push_user_blocks_merges_consecutive_user_turns() {
        let store = tmp_store();
        let mut history = vec![Message {
            role: "user".into(),
            content: vec![ContentBlock::text("a")],
        }];
        push_user_blocks(&mut history, &store, vec![ContentBlock::text("b")]);
        assert_eq!(history.len(), 1, "second user turn must merge, not append");
        assert_eq!(history[0].content.len(), 2);
        let _ = std::fs::remove_file(&store.path);
    }

    #[test]
    fn push_user_blocks_appends_after_assistant() {
        let store = tmp_store();
        let mut history = vec![Message {
            role: "assistant".into(),
            content: vec![ContentBlock::text("a")],
        }];
        push_user_blocks(&mut history, &store, vec![ContentBlock::text("b")]);
        assert_eq!(history.len(), 2, "user turn after assistant must append");
        assert_eq!(history[1].role, "user");
        let _ = std::fs::remove_file(&store.path);
    }

    #[test]
    fn user_turn_carries_only_the_raw_message_no_injected_directive() {
        let store = tmp_store();
        let mut history: Vec<Message> = Vec::new();
        push_user_blocks(&mut history, &store, vec![ContentBlock::text("O que este projeto faz?")]);
        assert_eq!(history.len(), 1);
        assert_eq!(history[0].content.len(), 1, "no phase directive should be folded in");
        let _ = std::fs::remove_file(&store.path);
    }

    #[test]
    fn agent_event_round_trip_text_step() {
        let ev = AgentEvent::TextStep { text: "hello".into() };
        let json = serde_json::to_value(&ev).unwrap();
        let back: AgentEvent = serde_json::from_value(json).unwrap();
        assert!(matches!(back, AgentEvent::TextStep { text } if text == "hello"));
    }

    #[test]
    fn agent_event_round_trip_text_delta() {
        let ev = AgentEvent::TextDelta { text: "partial".into() };
        let json = serde_json::to_value(&ev).unwrap();
        assert_eq!(json["event"], "TextDelta");
        let back: AgentEvent = serde_json::from_value(json).unwrap();
        assert!(matches!(back, AgentEvent::TextDelta { text } if text == "partial"));
    }

    #[test]
    fn agent_event_round_trip_thinking() {
        let ev = AgentEvent::Thinking("thinking text".into());
        let json = serde_json::to_value(&ev).unwrap();
        let back: AgentEvent = serde_json::from_value(json).unwrap();
        assert!(matches!(back, AgentEvent::Thinking(t) if t == "thinking text"));
    }

    #[test]
    fn agent_event_round_trip_tool_call() {
        let ev = AgentEvent::ToolCall {
            session_id: "s1".into(),
            tool_id: "t1".into(),
            tool_name: "read_file".into(),
            args: json!({"path": "/foo"}),
            permission: "auto".into(),
            edit_proposal: None,
        };
        let json = serde_json::to_value(&ev).unwrap();
        let back: AgentEvent = serde_json::from_value(json).unwrap();
        match back {
            AgentEvent::ToolCall { session_id, tool_id, tool_name, .. } => {
                assert_eq!(session_id, "s1");
                assert_eq!(tool_id, "t1");
                assert_eq!(tool_name, "read_file");
            }
            _ => panic!("expected ToolCall"),
        }
    }

    #[test]
    fn agent_event_round_trip_tool_result() {
        let ev = AgentEvent::ToolResult {
            tool_id: "t1".into(),
            tool_name: "read_file".into(),
            output: "content".into(),
            error: None,
        };
        let json = serde_json::to_value(&ev).unwrap();
        let back: AgentEvent = serde_json::from_value(json).unwrap();
        assert!(matches!(back, AgentEvent::ToolResult { tool_id, .. } if tool_id == "t1"));
    }

    #[test]
    fn agent_event_round_trip_done() {
        let ev = AgentEvent::Done {
            stop_reason: "end_turn".into(),
            text_output: "done".into(),
            input_tokens: 10,
            output_tokens: 20,
        };
        let json = serde_json::to_value(&ev).unwrap();
        let back: AgentEvent = serde_json::from_value(json).unwrap();
        match back {
            AgentEvent::Done { stop_reason, input_tokens, output_tokens, .. } => {
                assert_eq!(stop_reason, "end_turn");
                assert_eq!(input_tokens, 10);
                assert_eq!(output_tokens, 20);
            }
            _ => panic!("expected Done"),
        }
    }

    #[test]
    fn agent_event_round_trip_subagent_started() {
        let ev = AgentEvent::SubagentStarted {
            subagent_id: "sa1".into(),
            parent_tool_id: "pt1".into(),
            name: "explorer".into(),
            goal: "find stuff".into(),
            mode: "explore".into(),
        };
        let json = serde_json::to_value(&ev).unwrap();
        let back: AgentEvent = serde_json::from_value(json).unwrap();
        match back {
            AgentEvent::SubagentStarted { subagent_id, name, .. } => {
                assert_eq!(subagent_id, "sa1");
                assert_eq!(name, "explorer");
            }
            _ => panic!("expected SubagentStarted"),
        }
    }

    #[test]
    fn agent_event_round_trip_subagent_done() {
        let ev = AgentEvent::SubagentDone {
            subagent_id: "sa1".into(),
            status: "completed".into(),
            rounds: 5,
            input_tokens: 100,
            output_tokens: 50,
            report: String::new(),
        };
        let json = serde_json::to_value(&ev).unwrap();
        let back: AgentEvent = serde_json::from_value(json).unwrap();
        match back {
            AgentEvent::SubagentDone { subagent_id, status, rounds, .. } => {
                assert_eq!(subagent_id, "sa1");
                assert_eq!(status, "completed");
                assert_eq!(rounds, 5);
            }
            _ => panic!("expected SubagentDone"),
        }
    }

    #[test]
    fn agent_event_round_trip_subagent_wrapped() {
        let inner = AgentEvent::Thinking("inner thought".into());
        let ev = AgentEvent::Subagent {
            subagent_id: "sa1".into(),
            event: Box::new(inner),
        };
        let json = serde_json::to_value(&ev).unwrap();
        let back: AgentEvent = serde_json::from_value(json).unwrap();
        match back {
            AgentEvent::Subagent { subagent_id, event } => {
                assert_eq!(subagent_id, "sa1");
                assert!(matches!(*event, AgentEvent::Thinking(t) if t == "inner thought"));
            }
            _ => panic!("expected Subagent"),
        }
    }

    #[test]
    fn agent_event_round_trip_error() {
        let ev = AgentEvent::Error("something broke".into());
        let json = serde_json::to_value(&ev).unwrap();
        let back: AgentEvent = serde_json::from_value(json).unwrap();
        assert!(matches!(back, AgentEvent::Error(e) if e == "something broke"));
    }

    #[test]
    fn agent_event_round_trip_steering_injected() {
        let ev = AgentEvent::SteeringInjected { text: "steer".into(), attachments: None };
        let json = serde_json::to_value(&ev).unwrap();
        let back: AgentEvent = serde_json::from_value(json).unwrap();
        assert!(matches!(back, AgentEvent::SteeringInjected { text, .. } if text == "steer"));

        // Round-trip with attachments
        let ev2 = AgentEvent::SteeringInjected {
            text: "steer2".into(),
            attachments: Some(vec![crate::agent::persist::AttachmentMeta {
                name: "photo.png".into(),
                media_type: "image/png".into(),
                size: 1024,
            }]),
        };
        let json2 = serde_json::to_value(&ev2).unwrap();
        let back2: AgentEvent = serde_json::from_value(json2).unwrap();
        match back2 {
            AgentEvent::SteeringInjected { text, attachments } => {
                assert_eq!(text, "steer2");
                let atts = attachments.unwrap();
                assert_eq!(atts.len(), 1);
                assert_eq!(atts[0].name, "photo.png");
                assert_eq!(atts[0].media_type, "image/png");
                assert_eq!(atts[0].size, 1024);
            }
            _ => panic!("expected SteeringInjected"),
        }
    }

    #[test]
    fn agent_event_round_trip_ask_user() {
        let ev = AgentEvent::AskUser {
            session_id: "s1".into(),
            tool_id: "t1".into(),
            questions: json!([{"question": "q?", "options": ["a", "b"]}]),
        };
        let json = serde_json::to_value(&ev).unwrap();
        let back: AgentEvent = serde_json::from_value(json).unwrap();
        match back {
            AgentEvent::AskUser { session_id, tool_id, .. } => {
                assert_eq!(session_id, "s1");
                assert_eq!(tool_id, "t1");
            }
            _ => panic!("expected AskUser"),
        }
    }

    #[test]
    fn agent_event_round_trip_session_stats() {
        let ev = AgentEvent::SessionStats {
            input_tokens: 500,
            output_tokens: 200,
            cumulative_cost: Some(0.003),
            cost_input: Some(0.001),
            cost_output: Some(0.0015),
            cost_cache_read: Some(0.0005),
            context_tokens: 42_000,
            max_context_tokens: MAX_CONTEXT_TOKENS,
            compact_threshold: COMPACT_THRESHOLD,
        };
        let json = serde_json::to_value(&ev).unwrap();
        assert_eq!(json["data"]["contextTokens"], 42_000);
        assert_eq!(json["data"]["maxContextTokens"], 256_000);
        let back: AgentEvent = serde_json::from_value(json).unwrap();
        match back {
            AgentEvent::SessionStats {
                input_tokens,
                output_tokens,
                cumulative_cost,
                context_tokens,
                ..
            } => {
                assert_eq!(input_tokens, 500);
                assert_eq!(output_tokens, 200);
                assert_eq!(cumulative_cost, Some(0.003));
                assert_eq!(context_tokens, 42_000);
            }
            _ => panic!("expected SessionStats"),
        }
    }

    #[test]
    fn session_stats_without_cost() {
        let ev = AgentEvent::SessionStats {
            input_tokens: 100,
            output_tokens: 50,
            cumulative_cost: None,
            cost_input: None,
            cost_output: None,
            cost_cache_read: None,
            context_tokens: 0,
            max_context_tokens: MAX_CONTEXT_TOKENS,
            compact_threshold: COMPACT_THRESHOLD,
        };
        let json = serde_json::to_value(&ev).unwrap();
        let back: AgentEvent = serde_json::from_value(json).unwrap();
        match back {
            AgentEvent::SessionStats { cumulative_cost, .. } => {
                assert_eq!(cumulative_cost, None);
            }
            _ => panic!("expected SessionStats"),
        }
    }

    #[test]
    fn estimate_tokens_returns_reasonable_value() {
        let msg = Message {
            role: "user".into(),
            content: vec![ContentBlock::text("hello world")],
        };
        let history = vec![msg];
        let system = "You are a helpful assistant.";
        let tools = vec![];
        let estimated = estimate_tokens(&history, system, &tools);
        assert!(estimated > 0, "should estimate some tokens");
        assert!(estimated < 1000, "short message should be < 1k tokens");
    }

    #[test]
    fn estimate_tokens_increases_with_history() {
        let msg1 = Message {
            role: "user".into(),
            content: vec![ContentBlock::text("a".repeat(1000))],
        };
        let msg2 = Message {
            role: "assistant".into(),
            content: vec![ContentBlock::text("b".repeat(1000))],
        };
        let small = estimate_tokens(&[msg1.clone()], "", &[]);
        let large = estimate_tokens(&[msg1, msg2], "", &[]);
        assert!(large > small, "more history should mean more tokens");
    }

    #[test]
    fn compute_tail_turns_covers_last_two_exchanges() {
        let user = |t: &str| SessionRecord::Turn {
            message: Message { role: "user".into(), content: vec![ContentBlock::text(t)] },
            ts: 0,
        };
        let asst = |t: &str| SessionRecord::Turn {
            message: Message { role: "assistant".into(), content: vec![ContentBlock::text(t)] },
            ts: 0,
        };
        let recs = vec![
            user("q1"), asst("a1"),
            user("q2"), asst("a2"),
            user("q3"), asst("a3"),
        ];
        // Last 2 exchanges = q2..a3 = 4 Turn records
        assert_eq!(compute_tail_turns(&recs), 4);
    }

    #[test]
    fn compute_tail_turns_shrinks_when_over_budget() {
        let big = "x".repeat((TAIL_MAX_TOKENS as usize) * 4); // way over budget alone
        let recs = vec![
            SessionRecord::Turn {
                message: Message { role: "user".into(), content: vec![ContentBlock::text(big)] },
                ts: 0,
            },
            SessionRecord::Turn {
                message: Message { role: "assistant".into(), content: vec![ContentBlock::text("a")] },
                ts: 0,
            },
        ];
        assert_eq!(compute_tail_turns(&recs), 0, "oversized tail must be dropped");
    }

    /// Estimate cost for provider calls when the provider does not report cost.
    fn cost_for(model: &str, input: u32, cache_read: u32, output: u32) -> f64 {
        let p = model_pricing(model);
        (input as f64 * p.input + cache_read as f64 * p.cache_read + output as f64 * p.output)
            / 1_000_000.0
    }

    #[test]
    fn cost_claudinio_rates() {
        // claudinio: $0.50/M input, $0.15/M cache read, $2.00/M output
        let cost = cost_for("claudinio", 1000, 0, 500);
        assert!((cost - 0.0015).abs() < 0.0001, "expected ~$0.0015, got {cost}");
    }

    #[test]
    fn cost_claudius_rates() {
        // claudius: $3.00/M input, $0.90/M cache read, $8.00/M output
        let cost = cost_for("claudius", 1000, 0, 500);
        assert!((cost - 0.007).abs() < 0.0001, "expected ~$0.007, got {cost}");
    }

    #[test]
    fn cost_includes_cache_read() {
        // 1M cache-read tokens at claudinio rates = $0.15
        let cost = cost_for("claudinio", 0, 1_000_000, 0);
        assert!((cost - 0.15).abs() < 0.0001, "expected ~$0.15, got {cost}");
    }

    #[test]
    fn cost_unknown_model_falls_back_to_claudinio() {
        assert_eq!(
            cost_for("some-other-model", 1000, 100, 500),
            cost_for("claudinio", 1000, 100, 500)
        );
    }

    #[test]
    fn compact_threshold_is_75_percent_of_window() {
        assert_eq!(MAX_CONTEXT_TOKENS, 256_000);
        assert_eq!(COMPACT_THRESHOLD, 192_000);
    }

    // --- completion-judge verdict parsing (harness must never go idle after
    // the model merely announces a next step without taking it) ---
    //
    // The decision is delegated to the Brain model at runtime (language-agnostic,
    // no hardcoded phrase lists), so the only pure logic to unit-test here is how
    // the judge's one-word reply maps to a verdict. The live HTTP path is covered
    // by `judge_backend_mock` below against a stubbed /v1/messages server.

    #[test]
    fn verdict_continue_when_reply_says_continue() {
        assert_eq!(parse_turn_verdict("CONTINUE"), TurnVerdict::Continue);
        assert_eq!(parse_turn_verdict(" continue "), TurnVerdict::Continue);
        // Chatty models: token embedded in prose still counts.
        assert_eq!(
            parse_turn_verdict("CONTINUE — it said it would ask a question"),
            TurnVerdict::Continue
        );
    }

    #[test]
    fn verdict_done_when_reply_says_done() {
        assert_eq!(parse_turn_verdict("DONE"), TurnVerdict::Done);
        assert_eq!(parse_turn_verdict("done.\n"), TurnVerdict::Done);
    }

    #[test]
    fn verdict_fails_safe_to_done_on_garbage() {
        // Unrecognizable / empty replies end the run rather than risk a spurious
        // extra loop.
        assert_eq!(parse_turn_verdict(""), TurnVerdict::Done);
        assert_eq!(parse_turn_verdict("¯\\_(ツ)_/¯"), TurnVerdict::Done);
    }

    #[test]
    fn verdict_continue_wins_when_both_tokens_present() {
        // If the judge hedges and emits both, keep working.
        assert_eq!(
            parse_turn_verdict("not DONE, you should CONTINUE"),
            TurnVerdict::Continue
        );
    }
}

/// Backend/mock coverage for the completion judge: spin a throwaway local HTTP
/// server that answers `/v1/messages` with a canned Anthropic-shaped body, point
/// an AgentConfig at it, and assert the judge maps the reply to the right
/// verdict. No external network, no mock crate — just tokio (already a dep).
#[cfg(test)]
mod judge_backend_tests {
    use super::*;
    use crate::agent::provider::{classify_turn_completion, AgentConfig};
    use tokio::io::{AsyncReadExt, AsyncWriteExt};
    use tokio::net::TcpListener;

    /// Start a one-shot HTTP server that replies to a single request with the
    /// given `content` text wrapped in an Anthropic messages response. Returns
    /// the base URL to point the client at.
    async fn spawn_stub(content: &'static str) -> String {
        let listener = TcpListener::bind("127.0.0.1:0").await.unwrap();
        let addr = listener.local_addr().unwrap();
        tokio::spawn(async move {
            // Serve every connection (a test may make several judge calls).
            loop {
                let (mut socket, _) = match listener.accept().await {
                    Ok(pair) => pair,
                    Err(_) => break,
                };
                // Drain the request headers enough to not RST the client; we
                // don't need the body for the stub.
                let mut buf = [0u8; 4096];
                let _ = socket.read(&mut buf).await;
                let body = serde_json::json!({
                    "content": [ { "type": "text", "text": content } ]
                })
                .to_string();
                let resp = format!(
                    "HTTP/1.1 200 OK\r\nContent-Type: application/json\r\nContent-Length: {}\r\nConnection: close\r\n\r\n{}",
                    body.len(),
                    body
                );
                let _ = socket.write_all(resp.as_bytes()).await;
                let _ = socket.flush().await;
            }
        });
        format!("http://{addr}")
    }

    fn cfg_for(base_url: String) -> AgentConfig {
        AgentConfig {
            base_url,
            api_key: "test-key".into(),
            ..AgentConfig::default()
        }
    }

    #[tokio::test]
    async fn judge_backend_mock_continue() {
        let base = spawn_stub("CONTINUE").await;
        let cfg = cfg_for(base);
        let reply = classify_turn_completion(&cfg, "claudinio", "Primeiro vou confirmar:")
            .await
            .unwrap();
        assert_eq!(parse_turn_verdict(&reply), TurnVerdict::Continue);
        assert_eq!(
            judge_terminal_turn(&cfg, "claudinio", "Primeiro vou confirmar:").await,
            TurnVerdict::Continue
        );
    }

    #[tokio::test]
    async fn judge_backend_mock_done() {
        let base = spawn_stub("DONE").await;
        let cfg = cfg_for(base);
        let verdict = judge_terminal_turn(&cfg, "claudinio", "Tudo pronto, testes passaram.").await;
        assert_eq!(verdict, TurnVerdict::Done);
    }

    #[tokio::test]
    async fn judge_fails_safe_to_done_when_backend_unreachable() {
        // Nothing listening here — the request errors and the judge must NOT
        // wedge the loop: it falls back to Done.
        let cfg = cfg_for("http://127.0.0.1:1".into());
        let verdict = judge_terminal_turn(&cfg, "claudinio", "Vou explorar com subagentes:").await;
        assert_eq!(verdict, TurnVerdict::Done);
    }

    /// Extract the text of the LAST assistant Turn from a session JSONL — the
    /// dangling message the run ended on.
    fn last_assistant_text(jsonl: &str) -> Option<String> {
        jsonl
            .lines()
            .filter_map(|l| serde_json::from_str::<serde_json::Value>(l).ok())
            .filter(|v| v.get("kind").and_then(|k| k.as_str()) == Some("turn")
                && v.get("role").and_then(|r| r.as_str()) == Some("assistant"))
            .filter_map(|v| {
                v.get("content").and_then(|c| c.as_array()).map(|blocks| {
                    blocks
                        .iter()
                        .filter(|b| b.get("type").and_then(|t| t.as_str()) == Some("text"))
                        .filter_map(|b| b.get("text").and_then(|t| t.as_str()))
                        .collect::<Vec<_>>()
                        .join("")
                })
            })
            .filter(|s| !s.trim().is_empty())
            .last()
    }

    /// End-to-end reproduction of the stall: replay the real session that
    /// stopped (912bb460), take the exact dangling assistant message it ended
    /// on, and ask the LIVE Brain model whether the turn was finished. It must
    /// answer CONTINUE — proving the harness would have kept going instead of
    /// going idle.
    ///
    /// Ignored by default (needs network + a key). Run with:
    ///   CLAUDINIO_API_KEY=sk-… cargo test --lib judge_real_api_replays -- --ignored --nocapture
    #[tokio::test]
    #[ignore = "hits the live claudin.io API; requires CLAUDINIO_API_KEY"]
    async fn judge_real_api_replays_stalled_session() {
        let Ok(api_key) = std::env::var("CLAUDINIO_API_KEY") else {
            eprintln!("skipping: CLAUDINIO_API_KEY not set");
            return;
        };
        let jsonl = std::fs::read_to_string(
            "/Users/victortavernari/claudinio_code/.claudinio/sessions/912bb460-7e9b-459a-968d-eb506e5e9ec9.jsonl",
        )
        .expect("session jsonl readable");
        let dangling = last_assistant_text(&jsonl).expect("a final assistant message");
        eprintln!("--- dangling final message ---\n{dangling}\n------------------------------");
        assert!(
            dangling.trim_end().ends_with(':'),
            "sanity: the replayed message is the mid-thought that stalled the run"
        );
        let cfg = AgentConfig {
            base_url: "https://api.claudin.io".into(),
            api_key,
            ..AgentConfig::default()
        };
        // Judge with the Brain model, exactly as the harness does.
        let brain = cfg.model_for_mode(SessionMode::Brain.as_str()).to_string();
        let reply = classify_turn_completion(&cfg, &brain, &dangling)
            .await
            .expect("live judge call");
        eprintln!("--- live judge reply: {reply:?} ---");
        assert_eq!(
            parse_turn_verdict(&reply),
            TurnVerdict::Continue,
            "the live Brain model must recognise the dangling promise as unfinished"
        );
    }
}

#[cfg(test)]
mod golden_goal_tests {
    use super::*;

    #[test]
    fn test_parse_goals_no_goals() {
        let (cleaned, goals) = parse_goals("hello world");
        assert_eq!(cleaned, "hello world");
        assert!(goals.is_empty());
    }

    #[test]
    fn test_parse_goals_single() {
        let (cleaned, goals) = parse_goals("do <goal>code coverage in 80%</goal> please");
        assert_eq!(goals, vec!["code coverage in 80%"]);
        assert!(!cleaned.contains("<goal>"));
    }

    #[test]
    fn test_parse_goals_multiple() {
        let (cleaned, goals) = parse_goals("<goal>coverage 80%</goal> and <goal>no lint errors</goal>");
        assert_eq!(goals.len(), 2);
        assert_eq!(goals[0], "coverage 80%");
        assert_eq!(goals[1], "no lint errors");
        assert!(!cleaned.contains("<goal>"));
    }

    #[test]
    fn test_parse_goals_empty_goal_text() {
        let (cleaned, goals) = parse_goals("<goal>  </goal>");
        assert!(goals.is_empty());
        assert!(cleaned.is_empty());
    }
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
    use crate::agent::provider::{one_shot, AgentConfig};

    const ROOT: &str = "/Users/victortavernari/claudinio_code";

    /// The verbatim first user message from session 44ec41c1 — a UI feature
    /// (button + modal) that carries a concrete icon reference (a URL naming
    /// the exact `lucide:notebook-pen` icon).
    const SESSION_REQUEST: &str = "Gostaria que o textarea do input tivesse um botão para editar \
https://icones.js.org/collection/all?s=notebook&icon=lucide:notebook-pen e ao clicar nele, ele pega \
o texto que está na text area e abre um editor de texto numa modal com multiplas linhas, e ao fechar \
essa modal este texto volte para a text area, e assim posso enviar o texto editado.";

    // ---- Deterministic prompt-invariant tests (no network) ----

    #[test]
    fn brain_prompt_mandates_size_and_verbatim_assets() {
        let sys = system_prompt(Some(ROOT), None, None, SessionMode::Brain, PromptProfile::Standard);
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

    #[test]
    fn builder_prompt_requires_complete_subagent_spec() {
        let sys = system_prompt(Some(ROOT), None, None, SessionMode::Builder, PromptProfile::Standard);
        assert!(
            sys.contains("COMPLETE technical spec"),
            "Builder prompt must require complete subagent specs"
        );
        assert!(
            sys.contains("VERBATIM"),
            "Builder prompt must require repeating concrete values verbatim to subagents"
        );
    }

    #[test]
    fn system_prompt_warns_against_similar_to_guessing() {
        let sys = system_prompt(Some(ROOT), None, None, SessionMode::Builder, PromptProfile::Standard);
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
        let sys = system_prompt(Some(ROOT), None, None, SessionMode::Builder, PromptProfile::GitSync);
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
        let defs = api_tools(SessionMode::Builder, PromptProfile::GitSync, &[], &AgentConfig::default());
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
        let system = system_prompt(Some(ROOT), None, None, SessionMode::Brain, PromptProfile::Standard);
        let model = cfg.model_for_mode(SessionMode::Brain.as_str()).to_string();
        let user = format!(
            "{SESSION_REQUEST}\n\n---\nDo NOT call any tool and do NOT write a plan. Instead, output \
ONLY a numbered list of the clarifying questions you must ask me before writing the plan."
        );
        let reply = one_shot(&cfg, &model, &system, &user, 1500)
            .await
            .expect("live brain call");
        eprintln!("--- brain clarifying questions ---\n{reply}\n----------------------------------");
        let lc = reply.to_lowercase();
        let asks_size = ["tamanho", "size", "dimens", "largura", "altura", "width", "height", "fullscreen", "tela cheia", "viewport", "margin", "margem"]
            .iter()
            .any(|k| lc.contains(k));
        let asks_asset = ["ícone", "icone", "icon", "lucide", "notebook-pen", "svg", "url"]
            .iter()
            .any(|k| lc.contains(k));
        assert!(asks_size, "Brain must interview about the modal size/dimensions");
        assert!(asks_asset, "Brain must confirm/preserve the exact icon asset the user linked");
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
        let system = system_prompt(Some(ROOT), None, None, SessionMode::Builder, PromptProfile::Standard);
        let model = cfg.model_for_mode(SessionMode::Builder.as_str()).to_string();
        let user = "Plan task: add a new icon named 'notebook-pen' to src/components/Icon.tsx. The user \
specified the EXACT icon to use with this reference: \
https://icones.js.org/collection/all?s=notebook&icon=lucide:notebook-pen (that is the Lucide \
'notebook-pen' icon).\n\n---\nDo NOT call any tool. Output ONLY the exact `goal` string you would \
pass to a single 'code' subagent to implement this task.";
        let reply = one_shot(&cfg, &model, &system, &user, 1200)
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
