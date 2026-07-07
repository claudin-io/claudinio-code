mod bash;
mod edit_file;
mod grep;
mod list_dir;
mod read_file;
mod tasks;
mod write_plan;

use crate::code_intel::db::IndexDb;
use crate::code_intel::embeddings::SharedEmbedder;
use crate::lsp::manager::LspManager;
use serde::Serialize;
use serde_json::Value;
use std::path::Path;
use std::sync::Arc;
use std::sync::atomic::AtomicBool;
use tokio::sync::Mutex;

#[derive(Clone)]
pub struct ToolContext {
    pub db_path: Option<String>,
    pub lsp_manager: Option<Arc<Mutex<LspManager>>>,
    pub workspace_root: Option<String>,
    /// Live handle into AppState so a model that finishes loading mid-session
    /// becomes visible without recreating the context.
    pub embedding_model: Arc<Mutex<Option<SharedEmbedder>>>,
    /// Path to the active session's JSONL file. Used by tasks_get/tasks_set
    /// tools to persist the task list as SessionRecord::Tasks lines.
    pub session_store_path: Option<String>,
    /// Tracks which files the agent has read via the read_file tool.
    /// edit_file checks this before allowing edits.
    pub read_tracker: Arc<Mutex<ReadTracker>>,
    pub interrupt: Option<Arc<AtomicBool>>,
}

pub fn validate_path(requested: &str, ctx: &ToolContext) -> Result<(), String> {
    let root = match &ctx.workspace_root {
        Some(r) => r,
        None => return Ok(()),
    };
    let req_clean = std::path::Path::new(requested);
    let root_clean = std::path::Path::new(root);

    if let (Ok(canon_req), Ok(canon_root)) = (req_clean.canonicalize(), root_clean.canonicalize()) {
        if canon_req.starts_with(&canon_root) {
            return Ok(());
        }
    } else {
        if req_clean.starts_with(root_clean) {
            return Ok(());
        }
    }

    Err(format!(
        "path '{}' is outside the workspace '{}'. All file operations are restricted to the project workspace.",
        requested, root
    ))
}

#[derive(Debug, Clone, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct ToolDef {
    pub name: String,
    pub description: String,
    pub input_schema: Value,
}

#[derive(Debug, Clone, Serialize)]
#[serde(tag = "type")]
pub enum ToolOutput {
    Text { content: String },
    EditProposal { path: String, old_string: String, new_string: String, unified_diff: String },
}

pub fn get_defs() -> Vec<ToolDef> {
    vec![
        ToolDef {
            name: "read_file".into(),
            description: "Read a text file (project workspace only, max 2MB). Use the absolute path within the project. Optionally specify start_line and end_line (1-based, inclusive) to read only a subset of lines. Reading a file is REQUIRED before you can edit it with edit_file.".into(),
            input_schema: serde_json::json!({
                "type": "object",
                "properties": {
                    "path": { "type": "string", "description": "Absolute path to the file within the project workspace" },
                    "start_line": { "type": "integer", "description": "Optional 1-based start line (inclusive). If omitted, reads from the beginning." },
                    "end_line": { "type": "integer", "description": "Optional 1-based end line (inclusive). If omitted, reads to the end." }
                },
                "required": ["path"]
            }),
        },
        ToolDef {
            name: "list_dir".into(),
            description: "List files and directories at a given path (one level, project workspace only, respects .gitignore). Start from the project root.".into(),
            input_schema: serde_json::json!({
                "type": "object",
                "properties": {
                    "path": { "type": "string", "description": "Absolute path to the directory within the project workspace" }
                },
                "required": ["path"]
            }),
        },
        ToolDef {
            name: "grep".into(),
            description: "Search for a regex pattern across files in the project workspace using ripgrep.".into(),
            input_schema: serde_json::json!({
                "type": "object",
                "properties": {
                    "pattern": { "type": "string", "description": "Regex pattern" },
                    "path": { "type": "string", "description": "Optional subdirectory within the project workspace to limit search" }
                },
                "required": ["pattern"]
            }),
        },
        ToolDef {
            name: "edit_file".into(),
            description: "Propose a change to a file in the project workspace. Replaces the FIRST occurrence of old_string with new_string. NOT applied until you approve. IMPORTANT: You MUST call read_file on the file first (or at least the line range containing the edit) before using edit_file — otherwise the edit will be rejected with an error.".into(),
            input_schema: serde_json::json!({
                "type": "object",
                "properties": {
                    "path": { "type": "string", "description": "Absolute path within the project workspace" },
                    "old_string": { "type": "string", "description": "Text to replace" },
                    "new_string": { "type": "string", "description": "Replacement text" }
                },
                "required": ["path", "old_string", "new_string"]
            }),
        },
        ToolDef {
            name: "code_search".into(),
            description: "Full-text search across indexed symbols (FTS5). Faster and more targeted than grep for finding definitions — prefer this over grep.".into(),
            input_schema: serde_json::json!({
                "type": "object",
                "properties": {
                    "query": { "type": "string", "description": "Search term" },
                    "limit": { "type": "integer", "default": 20 }
                },
                "required": ["query"]
            }),
        },
        ToolDef {
            name: "symbol_lookup".into(),
            description: "Look up a symbol by exact name across the workspace. Use when you know the exact symbol name.".into(),
            input_schema: serde_json::json!({
                "type": "object",
                "properties": {
                    "name": { "type": "string", "description": "Exact symbol name" }
                },
                "required": ["name"]
            }),
        },
        ToolDef {
            name: "file_outline".into(),
            description: "List all symbols defined in a file. Use this before read_file to understand a file's structure at a glance.".into(),
            input_schema: serde_json::json!({
                "type": "object",
                "properties": {
                    "file_path": { "type": "string", "description": "Absolute file path" }
                },
                "required": ["file_path"]
            }),
        },
        ToolDef {
            name: "go_to_definition".into(),
            description: "Find where a symbol is defined at a specific position. Uses LSP (precise) or indexed fallback. Prefer over grep for finding definitions.".into(),
            input_schema: serde_json::json!({
                "type": "object",
                "properties": {
                    "file_path": { "type": "string", "description": "Absolute file path" },
                    "line": { "type": "integer", "description": "0-based line number" },
                    "character": { "type": "integer", "description": "0-based character offset" }
                },
                "required": ["file_path", "line", "character"]
            }),
        },
        ToolDef {
            name: "find_references".into(),
            description: "Find all references to a symbol at a specific position. Uses LSP (precise) or index. Prefer over grep for finding usages.".into(),
            input_schema: serde_json::json!({
                "type": "object",
                "properties": {
                    "file_path": { "type": "string", "description": "Absolute file path" },
                    "line": { "type": "integer", "description": "0-based line number" },
                    "character": { "type": "integer", "description": "0-based character offset" }
                },
                "required": ["file_path", "line", "character"]
            }),
        },
        ToolDef {
            name: "semantic_search".into(),
            description: "Semantic (concept-based) code search using CodeBERT embeddings. \
Finds code by meaning and behavior, not keywords — e.g. 'message queue system' finds \
SteeringCtl.drain/push/queue even without identifier match. Prefer this when you can \
describe the functionality but don't know the exact symbol name. The embedding model \
is ENGLISH-ONLY: always translate the user's phrasing to English before querying — \
never pass a query in another language. Top results include a source snippet. \
Ranking: go_to_definition (precise) → semantic_search (conceptual) → \
code_search (keyword) → grep (fallback).".into(),
            input_schema: serde_json::json!({
                "type": "object",
                "properties": {
                    "query": { "type": "string", "description": "Natural language description of what the code does. MUST be in English — translate first if the user wrote in another language." },
                    "limit": { "type": "integer", "default": 15 }
                },
                "required": ["query"]
            }),
        },
        ToolDef {
            name: "bash".into(),
            description: "Run a shell command. It already runs with the project workspace root as its working directory — run commands directly (e.g. \"git status\") with relative paths; never cd into guessed paths. Requires approval for non-allowlisted commands. Danger-sensitive commands are blocked automatically.".into(),
            input_schema: serde_json::json!({
                "type": "object",
                "properties": {
                    "command": { "type": "string", "description": "Shell command to run" },
                    "workdir": { "type": "string", "description": "Working directory (defaults to project root)" },
                    "stdin": { "type": "string", "description": "Optional stdin input for the command" },
                    "timeout_seconds": { "type": "integer", "description": "Timeout in seconds (default 30, override if command needs more time)" }
                },
                "required": ["command"]
            }),
        },
        ToolDef {
            name: "ask_user".into(),
            description: "Ask the user one or more questions when you are missing information or need a decision only they can make. Questions to the user MUST go through this tool — a question written as plain assistant text ends the turn unanswered and stalls the task. Each question carries concrete options; the UI automatically appends a final free-text option, so never add an 'Other' option yourself. Blocks until answered and returns the compiled question/answer pairs.".into(),
            input_schema: serde_json::json!({
                "type": "object",
                "properties": {
                    "questions": {
                        "type": "array",
                        "description": "Questions to ask the user",
                        "items": {
                            "type": "object",
                            "properties": {
                                "question": { "type": "string", "description": "The complete question, ending with a question mark" },
                                "options": { "type": "array", "items": { "type": "string" }, "description": "2-4 concise, mutually exclusive choices" },
                                "multi_select": { "type": "boolean", "description": "Allow picking more than one option (default false)" }
                            },
                            "required": ["question", "options"]
                        }
                    }
                },
                "required": ["questions"]
            }),
        },
        ToolDef {
            name: "tasks_get".into(),
            description: "Return the current list of tasks. Each task has an id, title, description, journal (array of notes/findings), and status (todo | doing | done). Use this at the start of a session to understand what needs to be done.".into(),
            input_schema: serde_json::json!({"type": "object", "properties": {}, "required": []}),
        },
        ToolDef {
            name: "tasks_set".into(),
            description: "Fully replace the task list (stateless — pass ALL tasks with updated statuses). Each task has: id (unique string), title, description, journal (array of findings/memory entries), status (todo | doing | done). Always read current tasks first with tasks_get before modifying.".into(),
            input_schema: serde_json::json!({
                "type": "object",
                "properties": {
                    "tasks": {
                        "type": "array",
                        "description": "All tasks (full replace — include every task, not just the one you changed)",
                        "items": {
                            "type": "object",
                            "properties": {
                                "id": { "type": "string", "description": "Unique task identifier" },
                                "title": { "type": "string", "description": "Short task title" },
                                "description": { "type": "string", "description": "Task description / goal" },
                                "journal": { "type": "array", "items": { "type": "string" }, "description": "Findings or relevant information as memory entries" },
                                "status": { "type": "string", "enum": ["todo", "doing", "done"], "description": "Task status" }
                            },
                            "required": ["id", "title", "description", "journal", "status"]
                        }
                    }
                },
                "required": ["tasks"]
            }),
        },
        ToolDef {
            name: "spawn_agents".into(),
            description: "Spawn 1-4 parallel subagents, each with a fresh context and its own goal. Returns each agent's final report. Use for broad multi-file investigation ('explore' mode) or independent atomic code changes ('code' mode). Goals must be self-contained: include file paths, symbols and constraints. All agents in one call run in parallel.".into(),
            input_schema: serde_json::json!({
                "type": "object",
                "required": ["agents"],
                "properties": {
                    "agents": {
                        "type": "array", "minItems": 1, "maxItems": 4,
                        "items": {
                            "type": "object",
                            "required": ["name", "goal", "mode"],
                            "properties": {
                                "name": { "type": "string", "description": "Short label shown to the user, e.g. 'auth-flow-investigator'" },
                                "goal": { "type": "string", "description": "Self-contained instructions: task, known file paths/symbols, constraints" },
                                "mode": { "type": "string", "enum": ["explore", "code"], "description": "explore = read-only tools; code = can edit files and run bash (with user approval)" },
                                "expected_output": { "type": "string", "description": "What the final report must contain" }
                            }
                        }
                    }
                }
            }),
        },
    ]
}



/// Definition of the write_plan tool. Only offered in Brain mode — it is
/// the one write the planning mode is allowed to perform, and its target path
/// is confined to `<workspace>/.claudinio/plans/`.
pub fn write_plan_def() -> ToolDef {
    ToolDef {
        name: "write_plan".into(),
        description: "Write the Solution Design plan to <workspace>/.claudinio/plans/YYYY-MM-DD_<name>.md. \
Overwrites the file, so always pass the FULL plan content — call again with the same name and the \
complete updated text to revise. Structure: Context, Solution Design, Risks, Tasks summary.".into(),
        input_schema: serde_json::json!({
            "type": "object",
            "properties": {
                "name": { "type": "string", "description": "Short plan name; becomes the file slug (e.g. 'dark mode toggle')" },
                "content": { "type": "string", "description": "Full markdown content of the plan" }
            },
            "required": ["name", "content"]
        }),
    }
}

/// Definition of the enter_plan_mode tool. Only offered in Builder mode.
pub fn enter_plan_mode_def() -> ToolDef {
    ToolDef {
        name: "enter_plan_mode".into(),
        description: "Switch this session into Brain (planning) mode. Use when the task turns out to be \
genuinely hard or ambiguous — unclear requirements, large design space, conflicting constraints — and \
designing first beats guessing. Editing tools are disabled until the plan and tasks are ready; because \
you initiated it, you can return with exit_plan_mode.".into(),
        input_schema: serde_json::json!({
            "type": "object",
            "properties": {
                "reason": { "type": "string", "description": "One sentence: why this task needs planning first (shown to the user)" }
            },
            "required": ["reason"]
        }),
    }
}

/// Definition of the exit_plan_mode tool. Only offered in Brain mode, and
/// only succeeds when the agent itself entered Brain via enter_plan_mode.
pub fn exit_plan_mode_def() -> ToolDef {
    ToolDef {
        name: "exit_plan_mode".into(),
        description: "Leave Brain mode and return to Builder to execute the plan. Only works if YOU \
entered Brain via enter_plan_mode; when the user enabled Brain, only their toggle can exit — \
finish by telling them the plan and tasks are ready.".into(),
        input_schema: serde_json::json!({"type": "object", "properties": {}, "required": []}),
    }
}

pub async fn execute(name: &str, args: Value, ctx: &ToolContext) -> Result<ToolOutput, String> {
    match name {
        "read_file" => {
            let a: read_file::ReadFileArgs = serde_json::from_value(args).map_err(|e| format!("invalid args: {e}"))?;
            validate_path(&a.path, ctx)?;
            let path = a.path.clone();
            let start_line = a.start_line;
            let end_line = a.end_line;
            let content = read_file::execute(a)?;
            // Record the read for edit_file validation
            {
                let mut tracker = ctx.read_tracker.lock().await;
                tracker.record_read(&path, start_line, end_line);
            }
            Ok(ToolOutput::Text { content })
        }
        "list_dir" => {
            let a: list_dir::ListDirArgs = serde_json::from_value(args).map_err(|e| format!("invalid args: {e}"))?;
            validate_path(&a.path, ctx)?;
            let entries = list_dir::execute(a)?;
            Ok(ToolOutput::Text { content: serde_json::to_string_pretty(&entries).unwrap_or_default() })
        }
        "grep" => {
            let a: grep::GrepArgs = serde_json::from_value(args).map_err(|e| format!("invalid args: {e}"))?;
            if let Some(ref path) = a.path {
                validate_path(path, ctx)?;
            } else if let Some(ref root) = ctx.workspace_root {
                let a2 = grep::GrepArgs { pattern: a.pattern.clone(), path: Some(root.clone()) };
                let matches = grep::execute(a2)?;
                return Ok(ToolOutput::Text { content: serde_json::to_string_pretty(&matches).unwrap_or_default() });
            }
            let matches = grep::execute(a)?;
            Ok(ToolOutput::Text { content: serde_json::to_string_pretty(&matches).unwrap_or_default() })
        }
        "edit_file" => {
            let a: edit_file::EditFileArgs = serde_json::from_value(args).map_err(|e| format!("invalid args: {e}"))?;
            validate_path(&a.path, ctx)?;
            // Enforce read-before-edit
            {
                let tracker = ctx.read_tracker.lock().await;
                tracker.check_can_edit(&a.path, &a.old_string)?;
            }
            let diff = edit_file::preview(&a)?;
            Ok(ToolOutput::EditProposal { path: diff.path, old_string: diff.old_string, new_string: diff.new_string, unified_diff: diff.unified_diff })
        }
        "code_search" => {
            let db = open_db(&ctx.db_path)?;
            let query = args.get("query").and_then(|v| v.as_str()).ok_or("missing query")?;
            let limit = args.get("limit").and_then(|v| v.as_i64()).unwrap_or(20);
            let results = db.search_symbols(query, limit)?;
            Ok(ToolOutput::Text { content: serde_json::to_string_pretty(&results).unwrap_or_default() })
        }
        "symbol_lookup" => {
            let db = open_db(&ctx.db_path)?;
            let name = args.get("name").and_then(|v| v.as_str()).ok_or("missing name")?;
            let results = db.search_symbols(name, 20)?;
            Ok(ToolOutput::Text { content: serde_json::to_string_pretty(&results).unwrap_or_default() })
        }
        "file_outline" => {
            let db = open_db(&ctx.db_path)?;
            let file_path = args.get("file_path").and_then(|v| v.as_str()).ok_or("missing file_path")?;
            validate_path(file_path, ctx)?;
            let results = db.symbols_in_file(file_path)?;
            Ok(ToolOutput::Text { content: serde_json::to_string_pretty(&results).unwrap_or_default() })
        }
        "go_to_definition" => {
            let file_path = args.get("file_path").and_then(|v| v.as_str()).ok_or("missing file_path")?;
            let line = args.get("line").and_then(|v| v.as_u64()).ok_or("missing line")?;
            let character = args.get("character").and_then(|v| v.as_u64()).ok_or("missing character")?;

            if let Some(ref lsp) = ctx.lsp_manager {
                let mut mgr = lsp.lock().await;
                match mgr.goto_definition(file_path, line, character) {
                    Ok(locs) => {
                        let content = serde_json::to_string_pretty(&locs).unwrap_or_default();
                        return Ok(ToolOutput::Text { content });
                    }
                    Err(_) => {}
                }
            }

            heuristically_find_definition(file_path, line, character, &ctx.db_path)
        }
        "find_references" => {
            let file_path = args.get("file_path").and_then(|v| v.as_str()).ok_or("missing file_path")?;
            let line = args.get("line").and_then(|v| v.as_u64()).ok_or("missing line")?;
            let character = args.get("character").and_then(|v| v.as_u64()).ok_or("missing character")?;

            if let Some(ref lsp) = ctx.lsp_manager {
                let mut mgr = lsp.lock().await;
                match mgr.find_references(file_path, line, character) {
                    Ok(locs) => {
                        let content = serde_json::to_string_pretty(&locs).unwrap_or_default();
                        return Ok(ToolOutput::Text { content });
                    }
                    Err(_) => {}
                }
            }

            heuristically_find_references(file_path, line, character, &ctx.db_path)
        }
        "semantic_search" => {
            let query = args.get("query").and_then(|v| v.as_str()).ok_or("missing query")?;
            let limit = args.get("limit").and_then(|v| v.as_i64()).unwrap_or(15);
            let db = open_db(&ctx.db_path)?;
            let model = ctx.embedding_model.lock().await.clone().ok_or(
                "semantic search not available — the embedding model is still loading or failed to load (check app logs)",
            )?;
            let query = query.to_string();
            let query_vec = tokio::task::spawn_blocking(move || {
                let mut model = model.lock().map_err(|e| format!("embedder lock: {e}"))?;
                model.encode_query(&query)
            })
            .await
            .map_err(|e| format!("encode task panicked: {e}"))??;
            let mut results = db.search_by_embedding(&query_vec, limit as usize)?;
            attach_snippets(&mut results);
            Ok(ToolOutput::Text { content: serde_json::to_string_pretty(&results).unwrap_or_default() })
        }
        "bash" => {
            let mut a: bash::BashArgs = serde_json::from_value(args).map_err(|e| format!("invalid args: {e}"))?;
            if a.workdir.is_none() {
                a.workdir = ctx.workspace_root.clone();
            }
            let content = bash::execute(a, ctx).await?;
            Ok(ToolOutput::Text { content })
        }
        "tasks_get" => {
            let content = tasks::execute_get(ctx)?;
            Ok(ToolOutput::Text { content })
        }
        "tasks_set" => {
            let a: tasks::SetTasksArgs = serde_json::from_value(args).map_err(|e| format!("invalid args: {e}"))?;
            let content = tasks::execute_set(a, ctx)?;
            Ok(ToolOutput::Text { content })
        }
        "write_plan" => {
            let a: write_plan::WritePlanArgs = serde_json::from_value(args).map_err(|e| format!("invalid args: {e}"))?;
            let content = write_plan::execute(a, ctx)?;
            Ok(ToolOutput::Text { content })
        }
        "spawn_agents" => {
            Err("spawn_agents is handled by the session orchestrator".into())
        }
        "enter_plan_mode" | "exit_plan_mode" => {
            Err("mode switch tools are handled by the session orchestrator".into())
        }
        _ => Err(format!("unknown tool: {name}")),
    }
}

pub async fn apply_edit_with_ctx(args: Value, ctx: &ToolContext) -> Result<String, String> {
    let a: edit_file::EditFileArgs = serde_json::from_value(args).map_err(|e| format!("invalid args: {e}"))?;
    validate_path(&a.path, ctx)?;
    // Enforce read-before-edit
    {
        let tracker = ctx.read_tracker.lock().await;
        tracker.check_can_edit(&a.path, &a.old_string)?;
    }
    let diff = edit_file::preview(&a)?;
    edit_file::apply(&diff)
}

/// How many top semantic hits get a source snippet, and how large each can be.
/// A bare name+signature is usually too little context to judge relevance, but
/// full bodies for every hit would blow up the tool result.
const SNIPPET_TOP_HITS: usize = 5;
const SNIPPET_MAX_LINES: usize = 40;
const SNIPPET_MAX_CHARS: usize = 2400;

fn attach_snippets(results: &mut [crate::code_intel::db::SemanticSearchResult]) {
    for r in results.iter_mut().take(SNIPPET_TOP_HITS) {
        let Ok(content) = std::fs::read_to_string(&r.file_path) else { continue };
        let start = (r.start_line.max(0)) as usize;
        let end = (r.end_line.max(r.start_line)) as usize;
        let mut snippet: String = content
            .lines()
            .skip(start)
            .take((end - start + 1).min(SNIPPET_MAX_LINES))
            .collect::<Vec<_>>()
            .join("\n");
        if snippet.len() > SNIPPET_MAX_CHARS {
            let cut = snippet
                .char_indices()
                .take_while(|(i, _)| *i < SNIPPET_MAX_CHARS)
                .last()
                .map(|(i, c)| i + c.len_utf8())
                .unwrap_or(0);
            snippet.truncate(cut);
            snippet.push_str("\n… [truncated — read_file for the rest]");
        }
        if !snippet.is_empty() {
            r.snippet = Some(snippet);
        }
    }
}

fn open_db(db_path: &Option<String>) -> Result<IndexDb, String> {
    let path = db_path.as_ref().ok_or("index not available — open a workspace first")?;
    IndexDb::open(Path::new(path))
}

fn heuristically_find_definition(
    file_path: &str,
    _line: u64,
    _character: u64,
    db_path: &Option<String>,
) -> Result<ToolOutput, String> {
    let content = std::fs::read_to_string(file_path).map_err(|e| format!("read {file_path}: {e}"))?;
    let lines: Vec<&str> = content.lines().collect();
    let line_idx = _line as usize;
    if line_idx >= lines.len() {
        return Ok(ToolOutput::Text { content: "line out of range".into() });
    }

    let line_text = lines[line_idx];
    let col = _character as usize;
    if col >= line_text.len() {
        return Ok(ToolOutput::Text { content: "character out of range".into() });
    }

    let cursor_text = &line_text[col..];
    let word: String = cursor_text.chars().take_while(|c| c.is_alphanumeric() || *c == '_').collect();
    if word.is_empty() {
        return Ok(ToolOutput::Text { content: "no symbol at cursor".into() });
    }

    let db = match open_db(db_path) {
        Ok(d) => d,
        Err(_) => return Ok(ToolOutput::Text { content: format!("LSP unavailable; symbol at cursor: {word}") }),
    };

    let results = db.search_symbols(&word, 10).unwrap_or_default();
    let content = serde_json::to_string_pretty(&results).unwrap_or_default();
    Ok(ToolOutput::Text { content })
}

fn heuristically_find_references(
    file_path: &str,
    _line: u64,
    _character: u64,
    db_path: &Option<String>,
) -> Result<ToolOutput, String> {
    let content = std::fs::read_to_string(file_path).map_err(|e| format!("read {file_path}: {e}"))?;
    let lines: Vec<&str> = content.lines().collect();
    let line_idx = _line as usize;
    if line_idx >= lines.len() {
        return Ok(ToolOutput::Text { content: "line out of range".into() });
    }

    let col = _character as usize;
    let line_text = lines[line_idx];
    if col >= line_text.len() {
        return Ok(ToolOutput::Text { content: "character out of range".into() });
    }

    let cursor_text = &line_text[col..];
    let word: String = cursor_text.chars().take_while(|c| c.is_alphanumeric() || *c == '_').collect();
    if word.is_empty() {
        return Ok(ToolOutput::Text { content: "no symbol at cursor".into() });
    }

    let db = match open_db(db_path) {
        Ok(d) => d,
        Err(_) => return Ok(ToolOutput::Text { content: format!("LSP unavailable; checking references for {word}") }),
    };

    let results = db.callers_of(&word, file_path).unwrap_or_default();
    let content = serde_json::to_string_pretty(&results).unwrap_or_default();
    Ok(ToolOutput::Text { content })
}

// ── ReadTracker: enforce read_file before edit_file ──

use std::collections::HashMap;

#[derive(Debug, Clone, Default)]
pub struct ReadFileRecord {
    pub full_read: bool,
    pub ranges: Vec<(usize, usize)>,
}

#[derive(Debug, Clone, Default)]
pub struct ReadTracker {
    pub files: HashMap<String, ReadFileRecord>,
}

impl ReadTracker {
    pub fn record_read(&mut self, path: &str, start_line: Option<usize>, end_line: Option<usize>) {
        let entry = self.files.entry(path.to_string()).or_default();
        match (start_line, end_line) {
            (Some(s), Some(e)) => {
                entry.ranges.push((s, e));
            }
            _ => {
                // No range or incomplete range = full file read
                entry.full_read = true;
                entry.ranges.clear();
            }
        }
    }

    /// Check whether editing `old_string` in `path` is allowed based on
    /// previously recorded reads. Returns Ok if the file was fully read or
    /// the old_string's first line falls within a recorded range.
    pub fn check_can_edit(&self, path: &str, old_string: &str) -> Result<(), String> {
        let entry = self.files.get(path).ok_or_else(|| {
            format!(
                "read_file must be called on {} before editing it",
                path
            )
        })?;

        if entry.full_read {
            return Ok(());
        }

        // Read the file to find which line old_string starts on
        let content =
            std::fs::read_to_string(path).map_err(|e| format!("cannot read {path}: {e}"))?;

        let first_line_text = old_string.lines().next().unwrap_or("");
        if first_line_text.is_empty() {
            return Err("old_string cannot be empty".to_string());
        }

        let line_num = content
            .lines()
            .position(|l| l.contains(first_line_text))
            .map(|idx| idx + 1) // 1-based
            .ok_or_else(|| format!("old_string not found in {path}"))?;

        if entry.ranges.iter().any(|(start, end)| line_num >= *start && line_num <= *end) {
            Ok(())
        } else {
            let ranges_str = entry
                .ranges
                .iter()
                .map(|(s, e)| format!("{s}-{e}"))
                .collect::<Vec<_>>()
                .join(", ");
            Err(format!(
                "read_file was called on {path}, but the old_string's first line ({line_num}) \
                 is outside the read range(s) ({ranges_str}). \
                 Call read_file on the relevant lines before editing."
            ))
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    /// Helper: a ToolContext with a fresh ReadTracker and no workspace root
    /// (so any path is valid).
    fn test_ctx() -> ToolContext {
        ToolContext {
            db_path: None,
            lsp_manager: None,
            workspace_root: None,
            embedding_model: Arc::new(Mutex::new(None)),
            session_store_path: None,
            read_tracker: Arc::new(Mutex::new(ReadTracker::default())),
            interrupt: None,
        }
    }

    /// Write a temp file with 20 numbered lines, return its path.
    fn write_20line_file(name: &str) -> std::path::PathBuf {
        let p = std::env::temp_dir().join(format!("claudinio_test_{name}"));
        let lines: Vec<String> = (1..=20).map(|i| format!("line{i}")).collect();
        std::fs::write(&p, lines.join("\n")).unwrap();
        p
    }

    // ── read_file range tests ──

    #[test]
    fn test_read_file_no_range_returns_all() {
        let p = write_20line_file("no_range_returns_all");
        let ctx = test_ctx();
        let args = serde_json::json!({"path": p.to_string_lossy()});
        let result = futures::executor::block_on(execute("read_file", args, &ctx));
        let output = result.expect("read_file should succeed");
        match output {
            ToolOutput::Text { content } => {
                assert_eq!(content.lines().count(), 20, "should return all 20 lines");
            }
            _ => panic!("expected Text variant"),
        }
        let _ = std::fs::remove_file(&p);
    }

    #[test]
    fn test_read_file_range_1based() {
        let p = write_20line_file("range_1based");
        let ctx = test_ctx();
        let args = serde_json::json!({
            "path": p.to_string_lossy(),
            "start_line": 1,
            "end_line": 3,
        });
        let result = futures::executor::block_on(execute("read_file", args, &ctx));
        let output = result.expect("read_file should succeed");
        match output {
            ToolOutput::Text { content } => {
                let lines: Vec<&str> = content.lines().collect();
                assert_eq!(lines.len(), 3, "should return 3 lines");
                assert_eq!(lines[0], "line1");
                assert_eq!(lines[1], "line2");
                assert_eq!(lines[2], "line3");
            }
            _ => panic!("expected Text variant"),
        }
        let _ = std::fs::remove_file(&p);
    }

    // ── edit_file read-before-edit tests ──

    #[test]
    fn test_edit_file_rejected_without_read() {
        let p = write_20line_file("rejected_no_read");
        let ctx = test_ctx();
        let args = serde_json::json!({
            "path": p.to_string_lossy(),
            "old_string": "line7",
            "new_string": "line7_edited",
        });
        let result = futures::executor::block_on(execute("edit_file", args, &ctx));
        assert!(result.is_err(), "edit_file should be rejected without read_file");
        let err = result.unwrap_err();
        assert!(
            err.contains("read_file must be called"),
            "error should mention read_file: {err}"
        );
        let _ = std::fs::remove_file(&p);
    }

    #[test]
    fn test_edit_file_accepted_after_full_read() {
        let p = write_20line_file("accepted_full_read");
        let ctx = test_ctx();

        // First read the whole file
        let read_args = serde_json::json!({"path": p.to_string_lossy()});
        let read_result = futures::executor::block_on(execute("read_file", read_args, &ctx));
        assert!(read_result.is_ok(), "read_file should succeed");

        // Now edit should work
        let edit_args = serde_json::json!({
            "path": p.to_string_lossy(),
            "old_string": "line7",
            "new_string": "line7_edited",
        });
        let result = futures::executor::block_on(execute("edit_file", edit_args, &ctx));
        assert!(result.is_ok(), "edit_file should be accepted after full read");
        let _ = std::fs::remove_file(&p);
    }

    #[test]
    fn test_edit_file_accepted_after_range_read() {
        let p = write_20line_file("accepted_range");
        let ctx = test_ctx();

        // Read lines 3-5
        let read_args = serde_json::json!({
            "path": p.to_string_lossy(),
            "start_line": 3,
            "end_line": 5,
        });
        let read_result = futures::executor::block_on(execute("read_file", read_args, &ctx));
        assert!(read_result.is_ok(), "read_file should succeed");

        // Edit at line 4 (within range)
        let edit_args = serde_json::json!({
            "path": p.to_string_lossy(),
            "old_string": "line4",
            "new_string": "line4_edited",
        });
        let result = futures::executor::block_on(execute("edit_file", edit_args, &ctx));
        assert!(result.is_ok(), "edit at line 4 (within range 3-5) should be accepted");
        let _ = std::fs::remove_file(&p);
    }

    #[test]
    fn test_edit_file_rejected_outside_range() {
        let p = write_20line_file("rejected_outside_range");
        let ctx = test_ctx();

        // Read lines 3-5
        let read_args = serde_json::json!({
            "path": p.to_string_lossy(),
            "start_line": 3,
            "end_line": 5,
        });
        let read_result = futures::executor::block_on(execute("read_file", read_args, &ctx));
        assert!(read_result.is_ok(), "read_file should succeed");

        // Edit at line 10 (outside range)
        let edit_args = serde_json::json!({
            "path": p.to_string_lossy(),
            "old_string": "line10",
            "new_string": "line10_edited",
        });
        let result = futures::executor::block_on(execute("edit_file", edit_args, &ctx));
        assert!(result.is_err(), "edit at line 10 (outside range 3-5) should be rejected");
        let err = result.unwrap_err();
        assert!(
            err.contains("outside the read range"),
            "error should mention outside range: {err}"
        );
        let _ = std::fs::remove_file(&p);
    }

    #[test]
    fn test_edit_file_read_whole_after_partial() {
        let p = write_20line_file("whole_after_partial");
        let ctx = test_ctx();

        // First read a range
        let r1 = serde_json::json!({
            "path": p.to_string_lossy(),
            "start_line": 3,
            "end_line": 5,
        });
        futures::executor::block_on(execute("read_file", r1, &ctx)).unwrap();

        // Then read the whole file
        let r2 = serde_json::json!({"path": p.to_string_lossy()});
        futures::executor::block_on(execute("read_file", r2, &ctx)).unwrap();

        // Edit anywhere should work
        let edit_args = serde_json::json!({
            "path": p.to_string_lossy(),
            "old_string": "line18",
            "new_string": "line18_edited",
        });
        let result = futures::executor::block_on(execute("edit_file", edit_args, &ctx));
        assert!(result.is_ok(), "edit anywhere should work after full read");
        let _ = std::fs::remove_file(&p);
    }

    #[test]
    fn test_edit_file_multiple_ranges_overlap() {
        let p = write_20line_file("multi_range");
        let ctx = test_ctx();

        // Read lines 1-5
        let r1 = serde_json::json!({
            "path": p.to_string_lossy(),
            "start_line": 1,
            "end_line": 5,
        });
        futures::executor::block_on(execute("read_file", r1, &ctx)).unwrap();

        // Read lines 15-20
        let r2 = serde_json::json!({
            "path": p.to_string_lossy(),
            "start_line": 15,
            "end_line": 20,
        });
        futures::executor::block_on(execute("read_file", r2, &ctx)).unwrap();

        // Edit at line 18 (within second range)
        let edit_args = serde_json::json!({
            "path": p.to_string_lossy(),
            "old_string": "line18",
            "new_string": "line18_edited",
        });
        let result = futures::executor::block_on(execute("edit_file", edit_args, &ctx));
        assert!(result.is_ok(), "edit at line 18 should be accepted (within 15-20)");

        // Edit at line 3 (within first range)
        let edit2 = serde_json::json!({
            "path": p.to_string_lossy(),
            "old_string": "line3",
            "new_string": "line3_edited",
        });
        let result2 = futures::executor::block_on(execute("edit_file", edit2, &ctx));
        assert!(result2.is_ok(), "edit at line 3 should be accepted (within 1-5)");

        // Edit at line 10 (outside both)
        let edit3 = serde_json::json!({
            "path": p.to_string_lossy(),
            "old_string": "line10",
            "new_string": "line10_edited",
        });
        let result3 = futures::executor::block_on(execute("edit_file", edit3, &ctx));
        assert!(result3.is_err(), "edit at line 10 (outside both ranges) should be rejected");

        let _ = std::fs::remove_file(&p);
    }

    // Existing tests (unchanged)
    #[test]
    fn test_validate_path_allows_within_workspace() {
        let ctx = ToolContext {
            db_path: None,
            lsp_manager: None,
            workspace_root: Some("/home/user/project".into()),
            embedding_model: Arc::new(Mutex::new(None)),
            session_store_path: None,
            read_tracker: Arc::new(Mutex::new(ReadTracker::default())),
            interrupt: None,
        };
        assert!(validate_path("/home/user/project/src/main.ts", &ctx).is_ok());
        assert!(validate_path("/home/user/project", &ctx).is_ok());
        assert!(validate_path("/home/user/project/src", &ctx).is_ok());
        assert!(validate_path("/home/user/project/./src", &ctx).is_ok());
    }

    #[test]
    fn test_validate_path_rejects_outside_workspace() {
        let ctx = ToolContext {
            db_path: None,
            lsp_manager: None,
            workspace_root: Some("/home/user/project".into()),
            embedding_model: Arc::new(Mutex::new(None)),
            session_store_path: None,
            read_tracker: Arc::new(Mutex::new(ReadTracker::default())),
            interrupt: None,
        };
        assert!(validate_path("/etc/passwd", &ctx).is_err());
        assert!(validate_path("/home/user/other", &ctx).is_err());
        assert!(validate_path("/", &ctx).is_err());
        assert!(validate_path("/tmp", &ctx).is_err());
    }

    #[test]
    fn test_validate_path_allows_when_no_workspace_set() {
        let ctx = ToolContext {
            db_path: None,
            lsp_manager: None,
            workspace_root: None,
            embedding_model: Arc::new(Mutex::new(None)),
            session_store_path: None,
            read_tracker: Arc::new(Mutex::new(ReadTracker::default())),
            interrupt: None,
        };
        assert!(validate_path("/any/path", &ctx).is_ok());
        assert!(validate_path("/etc/passwd", &ctx).is_ok());
    }

    #[test]
    fn test_execute_list_dir_rejects_outside_workspace() {
        let ctx = ToolContext {
            db_path: None,
            lsp_manager: None,
            workspace_root: Some("/home/user/project".into()),
            embedding_model: Arc::new(Mutex::new(None)),
            session_store_path: None,
            read_tracker: Arc::new(Mutex::new(ReadTracker::default())),
            interrupt: None,
        };
        let args = serde_json::json!({"path": "/etc"});
        let result = futures::executor::block_on(execute("list_dir", args, &ctx));
        assert!(result.is_err());
        let err = result.unwrap_err();
        assert!(err.contains("outside the workspace"), "got: {err}");
    }

    #[test]
    fn test_execute_read_file_rejects_outside_workspace() {
        let ctx = ToolContext {
            db_path: None,
            lsp_manager: None,
            workspace_root: Some("/home/user/project".into()),
            embedding_model: Arc::new(Mutex::new(None)),
            session_store_path: None,
            read_tracker: Arc::new(Mutex::new(ReadTracker::default())),
            interrupt: None,
        };
        let args = serde_json::json!({"path": "/etc/passwd"});
        let result = futures::executor::block_on(execute("read_file", args, &ctx));
        assert!(result.is_err());
        let err = result.unwrap_err();
        assert!(err.contains("outside the workspace"), "got: {err}");
    }

    #[test]
    fn test_grep_defaults_to_workspace_root() {
        let ctx = ToolContext {
            db_path: None,
            lsp_manager: None,
            workspace_root: Some("/home/user/project".into()),
            embedding_model: Arc::new(Mutex::new(None)),
            session_store_path: None,
            read_tracker: Arc::new(Mutex::new(ReadTracker::default())),
            interrupt: None,
        };
        let args = serde_json::json!({"pattern": "foo"});
        let result = futures::executor::block_on(execute("grep", args, &ctx));
        assert!(result.is_err(), "rg likely not installed in test env");
    }

    #[test]
    fn test_bash_dispatch_echo() {
        let rt = tokio::runtime::Runtime::new().unwrap();
        let ctx = ToolContext {
            db_path: None,
            lsp_manager: None,
            workspace_root: None,
            embedding_model: Arc::new(Mutex::new(None)),
            session_store_path: None,
            read_tracker: Arc::new(Mutex::new(ReadTracker::default())),
            interrupt: None,
        };
        let args = serde_json::json!({"command": "echo hello"});
        let result = rt.block_on(execute("bash", args, &ctx));
        let output = result.expect("bash should succeed");
        match output {
            ToolOutput::Text { content } => assert_eq!(content.trim(), "hello"),
            _ => panic!("expected Text variant"),
        }
    }

    #[test]
    fn test_bash_dispatch_unknown_tool() {
        let ctx = ToolContext {
            db_path: None,
            lsp_manager: None,
            workspace_root: None,
            embedding_model: Arc::new(Mutex::new(None)),
            session_store_path: None,
            read_tracker: Arc::new(Mutex::new(ReadTracker::default())),
            interrupt: None,
        };
        let args = serde_json::json!({"command": "echo"});
        let result = futures::executor::block_on(execute("nonexistent_tool", args, &ctx));
        assert!(result.is_err());
        let err = result.unwrap_err();
        assert!(err.contains("unknown tool"), "got: {err}");
    }

    // ── read_file large-file truncation tests ──

    fn write_large_file(name: &str, line_count: usize) -> std::path::PathBuf {
        let p = std::env::temp_dir().join(format!("claudinio_test_{name}"));
        let lines: Vec<String> = (1..=line_count).map(|i| format!("line{i}")).collect();
        std::fs::write(&p, lines.join("\n")).unwrap();
        p
    }

    #[test]
    fn test_read_file_small_no_truncation() {
        // Small file (< 5000 tokens) — should return full content, no warning.
        let p = write_large_file("small_no_trunc", 20);
        let ctx = test_ctx();
        let args = serde_json::json!({"path": p.to_string_lossy()});
        let result = futures::executor::block_on(execute("read_file", args, &ctx));
        let output = result.expect("read_file should succeed");
        match output {
            ToolOutput::Text { content } => {
                assert_eq!(content.lines().count(), 20, "should return all 20 lines");
                assert!(!content.contains("FILE SIZE WARNING"), "should NOT have truncation warning");
            }
            _ => panic!("expected Text variant"),
        }
        let _ = std::fs::remove_file(&p);
    }

    #[test]
    fn test_read_file_large_truncated() {
        // Large file (> 5000 tokens) — should return warning + truncated content.
        // Each line "lineN" is ~7 chars → ~2 tokens per line in cl100k_base.
        // With 5000 lines (~10000 tokens), truncation stops at ~2500 lines (~5000 tokens).
        // Output = ~9 lines of warning + ~2500 lines content = ~2509 total.
        let p = write_large_file("large_truncated", 5000);
        let ctx = test_ctx();
        let args = serde_json::json!({"path": p.to_string_lossy()});
        let result = futures::executor::block_on(execute("read_file", args, &ctx));
        let output = result.expect("read_file should succeed");
        match output {
            ToolOutput::Text { content } => {
                assert!(
                    content.contains("FILE SIZE WARNING"),
                    "should have truncation warning"
                );
                assert!(
                    content.contains("start_line/end_line"),
                    "should suggest start_line/end_line"
                );
                let line_count = content.lines().count();
                assert!(
                    line_count < 4900,
                    "truncated content should have far fewer than 4900 lines, got {line_count}"
                );
                assert!(
                    line_count > 10,
                    "truncated content should have more than 10 lines, got {line_count}"
                );
            }
            _ => panic!("expected Text variant"),
        }
        let _ = std::fs::remove_file(&p);
    }

    #[test]
    fn test_read_file_large_with_explicit_range_no_truncation() {
        // Large file with explicit start_line/end_line — no truncation even if file is huge.
        let p = write_large_file("large_with_range", 5000);
        let ctx = test_ctx();
        let args = serde_json::json!({
            "path": p.to_string_lossy(),
            "start_line": 1,
            "end_line": 10,
        });
        let result = futures::executor::block_on(execute("read_file", args, &ctx));
        let output = result.expect("read_file should succeed");
        match output {
            ToolOutput::Text { content } => {
                assert_eq!(content.lines().count(), 10, "should return exactly 10 lines");
                assert!(!content.contains("FILE SIZE WARNING"), "should NOT have truncation warning");
            }
            _ => panic!("expected Text variant"),
        }
        let _ = std::fs::remove_file(&p);
    }
}
