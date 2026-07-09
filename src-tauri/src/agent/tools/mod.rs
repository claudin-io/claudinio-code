pub(crate) mod bash;
mod edit_file;
pub mod finalize_plan;
mod grep;
mod list_dir;
mod read_file;
pub mod tasks;
mod web_search;
pub(crate) mod write_plan;

pub use finalize_plan::git_head;

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
    /// Loaded AgentConfig — used by tools that call claudin.io services
    /// (currently just web_search). None when unavailable (e.g. some tests).
    pub agent_config: Option<crate::agent::provider::AgentConfig>,
    /// Custom plan save path (relative to workspace_root).
    /// None = use default (.claudinio/plans).
    pub plan_save_path: Option<String>,
    /// The git commit HEAD pointed at when this run started, used by
    /// finalize_plan to compute the changed files / commits since work began.
    /// None when not a git repo (or git unavailable).
    pub base_commit: Option<String>,
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
            /* Original (EN): Read a text file (project workspace only, max 2MB). Use the absolute path within the project. Optionally specify start_line and end_line (1-based, inclusive) to read only a subset of lines. Reading a file is REQUIRED before you can edit it with edit_file. */
            description: "读取文本文件（仅限项目工作区，最大2MB）。使用项目内的绝对路径。可选择指定start_line和end_line（从1开始，包含两端）以仅读取部分行。在通过edit_file编辑文件之前，必须先读取该文件。".into(),
            input_schema: serde_json::json!({
                "type": "object",
                "properties": {
                    "path": { "type": "string", "description": "项目工作区中文件的绝对路径" },
                    "start_line": { "type": "integer", "description": "可选的起始行号（从1开始，包含该行）。如果省略，则从头开始读取。" },
                    "end_line": { "type": "integer", "description": "可选的结束行号（从1开始，包含该行）。如果省略，则读取到末尾。" }
                },
                "required": ["path"]
            }),
        },
        ToolDef {
            name: "list_dir".into(),
            /* Original (EN): List files and directories at a given path (one level, project workspace only, respects .gitignore). Start from the project root. */
            description: "列出指定路径下的文件和目录（仅限一级，仅限项目工作区，遵循.gitignore）。从项目根目录开始。".into(),
            input_schema: serde_json::json!({
                "type": "object",
                "properties": {
                    "path": { "type": "string", "description": "项目工作区中目录的绝对路径" }
                },
                "required": ["path"]
            }),
        },
        ToolDef {
            name: "grep".into(),
            /* Original (EN): Search for a regex pattern across files in the project workspace using ripgrep. */
            description: "使用ripgrep在项目工作区的文件中搜索正则表达式模式。".into(),
            input_schema: serde_json::json!({
                "type": "object",
                "properties": {
                    "pattern": { "type": "string", "description": "正则表达式模式" },
                    "path": { "type": "string", "description": "项目工作区内的可选子目录，用于限定搜索范围" }
                },
                "required": ["pattern"]
            }),
        },
        ToolDef {
            name: "edit_file".into(),
            /* Original (EN): Propose a change to a file in the project workspace. Replaces the FIRST occurrence of old_string with new_string. NOT applied until you approve. IMPORTANT: You MUST call read_file on the file first (or at least the line range containing the edit) before using edit_file — otherwise the edit will be rejected with an error. */
            description: "对项目工作区中的文件提出修改。将old_string的首次出现替换为new_string。在您批准前不会应用。重要：在使用edit_file之前，您必须先调用read_file读取文件（或至少包含编辑内容的行范围）——否则编辑将被拒绝并返回错误。".into(),
            input_schema: serde_json::json!({
                "type": "object",
                "properties": {
                    "path": { "type": "string", "description": "项目工作区内的绝对路径" },
                    "old_string": { "type": "string", "description": "要替换的文本" },
                    "new_string": { "type": "string", "description": "替换后的文本" }
                },
                "required": ["path", "old_string", "new_string"]
            }),
        },
        ToolDef {
            name: "code_search".into(),
            /* Original (EN): Full-text search across indexed symbols (FTS5). Faster and more targeted than grep for finding definitions — prefer this over grep. */
            description: "跨索引符号表进行全文搜索（FTS5）。比grep更快、更精准地查找定义——优先使用此工具而非grep。".into(),
            input_schema: serde_json::json!({
                "type": "object",
                "properties": {
                    "query": { "type": "string", "description": "搜索词" },
                    "limit": { "type": "integer", "default": 20 }
                },
                "required": ["query"]
            }),
        },
        ToolDef {
            name: "symbol_lookup".into(),
            /* Original (EN): Look up a symbol by exact name across the workspace. Use when you know the exact symbol name. */
            description: "按精确名称在工作区中查找符号。当你知道确切的符号名称时使用。".into(),
            input_schema: serde_json::json!({
                "type": "object",
                "properties": {
                    "name": { "type": "string", "description": "精确的符号名称" }
                },
                "required": ["name"]
            }),
        },
        ToolDef {
            name: "file_outline".into(),
            /* Original (EN): List all symbols defined in a file. Use this before read_file to understand a file's structure at a glance. */
            description: "列出文件中定义的所有符号。在read_file之前使用，快速了解文件结构。".into(),
            input_schema: serde_json::json!({
                "type": "object",
                "properties": {
                    "file_path": { "type": "string", "description": "文件的绝对路径" }
                },
                "required": ["file_path"]
            }),
        },
        ToolDef {
            name: "go_to_definition".into(),
            /* Original (EN): Find where a symbol is defined at a specific position. Uses LSP (precise) or indexed fallback. Prefer over grep for finding definitions. */
            description: "查找符号在特定位置的定义。使用LSP（精确）或索引回退。查找定义时优先于grep。".into(),
            input_schema: serde_json::json!({
                "type": "object",
                "properties": {
                    "file_path": { "type": "string", "description": "文件的绝对路径" },
                    "line": { "type": "integer", "description": "从0开始的行号" },
                    "character": { "type": "integer", "description": "从0开始的字符偏移量" }
                },
                "required": ["file_path", "line", "character"]
            }),
        },
        ToolDef {
            name: "find_references".into(),
            /* Original (EN): Find all references to a symbol at a specific position. Uses LSP (precise) or index. Prefer over grep for finding usages. */
            description: "查找符号在特定位置的所有引用。使用LSP（精确）或索引。查找用法时优先于grep。".into(),
            input_schema: serde_json::json!({
                "type": "object",
                "properties": {
                    "file_path": { "type": "string", "description": "文件的绝对路径" },
                    "line": { "type": "integer", "description": "从0开始的行号" },
                    "character": { "type": "integer", "description": "从0开始的字符偏移量" }
                },
                "required": ["file_path", "line", "character"]
            }),
        },
        ToolDef {
            name: "semantic_search".into(),
            /* Original (EN): Semantic (concept-based) code & documentation search using CodeBERT embeddings. Finds code and documentation by meaning and behavior, not keywords — e.g. 'message queue system' finds SteeringCtl.drain/push/queue even without identifier match. Prefer this when you can describe the functionality but don't know the exact symbol name. The embedding model is ENGLISH-ONLY: always translate the user's phrasing to English before querying — never pass a query in another language. Top results include a source snippet. Ranking: go_to_definition (precise) → semantic_search (conceptual) → code_search (keyword) → grep (fallback). */
            description: "使用CodeBERT嵌入进行基于语义（概念）的代码与文档搜索。通过含义和行为而非关键词查找代码和文档——例如，'message queue system'可以找到SteeringCtl.drain/push/queue，即使标识符不匹配。当你能够描述功能但不知道确切符号名称时，优先使用此工具。嵌入模型仅支持英文：在查询前始终将用户的表述翻译为英文——切勿传递其他语言的查询。顶部结果包含源代码片段。排序规则：go_to_definition（精确）→ semantic_search（概念）→ code_search（关键词）→ grep（回退）。".into(),
            input_schema: serde_json::json!({
                "type": "object",
                "properties": {
                    "query": { "type": "string", "description": "代码功能的自然语言描述。必须使用英文——如果用户使用了其他语言，请先翻译。" },
                    "limit": { "type": "integer", "default": 15 }
                },
                "required": ["query"]
            }),
        },
        ToolDef {
            name: "bash".into(),
            /* Original (EN): Run a shell command. It already runs with the project workspace root as its working directory — run commands directly (e.g. "git status") with relative paths; never cd into guessed paths. Requires approval for non-allowlisted commands. Danger-sensitive commands are blocked automatically. */
            description: "运行shell命令。默认以项目工作区根目录为工作目录——直接使用相对路径运行命令（例如\"git status\"）；切勿cd到猜测的路径。非白名单命令需要批准。危险敏感命令会被自动阻止。".into(),
            input_schema: serde_json::json!({
                "type": "object",
                "properties": {
                    "command": { "type": "string", "description": "要运行的Shell命令" },
                    "workdir": { "type": "string", "description": "工作目录（默认为项目根目录）" },
                    "stdin": { "type": "string", "description": "可选的命令标准输入" },
                    "timeout_seconds": { "type": "integer", "description": "超时时间（秒，默认30秒，如果命令需要更多时间请覆盖此值）" }
                },
                "required": ["command"]
            }),
        },
        ToolDef {
            name: "web_search".into(),
            /* Original (EN): Search the web via claudin.io (requires being signed in). Returns a short answer plus a list of results with titles, URLs, and snippets. Rate limited per account tier — use it when you need current information not in your training data or the codebase. */
            description: "通过claudin.io搜索网络（需要登录）。返回简短答案以及包含标题、URL和摘要的结果列表。根据账户层级进行速率限制——当你需要训练数据或代码库中不包含的当前信息时使用。".into(),
            input_schema: serde_json::json!({
                "type": "object",
                "properties": {
                    "query": { "type": "string", "description": "搜索查询" },
                    "max_results": { "type": "integer", "description": "最大返回结果数（1-10，默认5）" }
                },
                "required": ["query"]
            }),
        },
        ToolDef {
            name: "ask_user".into(),
            /* Original (EN): Ask the user one or more questions when you are missing information or need a decision only they can make. Questions to the user MUST go through this tool — a question written as plain assistant text ends the turn unanswered and stalls the task. Each question can have: - `multi_select: false` (default): radio buttons, user picks exactly ONE option. Best for mutually exclusive decisions. - `multi_select: true`: checkboxes, user can pick SEVERAL options. The UI shows square indicators instead of circles. The UI automatically appends an "Other" option with a free-text input field to EVERY question (single and multi). When the user types in "Other", their text is appended to the chosen options. So NEVER add an "Other" option manually to your options list — it's always there automatically. Blocks until answered and returns the compiled question/answer pairs. */
            description: "当缺少信息或需要只有用户才能做的决定时，向用户提出一个或多个问题。问题必须通过此工具提出——以普通助手文本形式书写的问题会导致回合无法回答并使任务停滞。\n\n每个问题可以有：\n- `multi_select: false`（默认）：单选按钮，用户只能选择一个选项。最适合互斥决策。\n- `multi_select: true`：复选框，用户可以选择多个选项。界面显示方框指示器而非圆圈。\n\n界面会自动为每个问题附加一个带有自由文本输入字段的\"其他\"选项（包括单选项和多选项）。当用户在\"其他\"中输入时，其文本会追加到所选选项中。因此，切勿手动向选项列表中添加\"其他\"选项——它始终自动存在。\n\n阻塞直到被回答，并返回编译后的问题/答案对。".into(),
            input_schema: serde_json::json!({
                "type": "object",
                "properties": {
                    "questions": {
                        "type": "array",
                        "description": "向用户提出的问题",
                        "items": {
                            "type": "object",
                            "properties": {
                                "question": { "type": "string", "description": "完整的问题，以问号结尾" },
                                "options": { "type": "array", "items": { "type": "string" }, "description": "2-4个简洁选项供用户选择。请勿包含'其他'选项——界面会自动添加。" },
                                "multi_select": { "type": "boolean", "description": "false（默认）= 单选按钮，用户选择一个。true = 复选框，用户选择多个并可通过自动添加的'其他'字段输入自由文本。" }
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
            /* Original (EN): Return the current list of tasks. Each task has an id, title, description, journal (array of notes/findings), and status (todo | doing | done). Use this at the start of a session to understand what needs to be done. */
            description: "返回当前任务列表。每个任务包含id、title、description、journal（笔记/发现的数组）和status（todo | doing | done）。在会话开始时使用，以了解需要完成什么。".into(),
            input_schema: serde_json::json!({"type": "object", "properties": {}, "required": []}),
        },
        ToolDef {
            name: "tasks_set".into(),
            /* Original (EN): Fully replace the task list (stateless — pass ALL tasks with updated statuses). Each task has: id (unique string), title, description, journal (array of findings/memory entries), status (todo | doing | done). Always read current tasks first with tasks_get before modifying. */
            description: "完全替换任务列表（无状态——传递所有带有更新状态的任务）。每个任务包含：id（唯一字符串）、title、description、journal（发现/记忆条目的数组）、status（todo | doing | done）。在修改前始终先使用tasks_get读取当前任务。".into(),
            input_schema: serde_json::json!({
                "type": "object",
                "properties": {
                    "tasks": {
                        "type": "array",
                        "description": "所有任务（完全替换——包含每个任务，而不仅仅是你修改的那个）",
                        "items": {
                            "type": "object",
                            "properties": {
                                "id": { "type": "string", "description": "唯一任务标识符" },
                                "title": { "type": "string", "description": "简短任务标题" },
                                "description": { "type": "string", "description": "任务描述/目标" },
                                "journal": { "type": "array", "items": { "type": "string" }, "description": "作为记忆条目的发现或相关信息" },
                                "status": { "type": "string", "enum": ["todo", "doing", "done"], "description": "任务状态" }
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
            /* Original (EN): Spawn 1-4 parallel subagents, each with a fresh context and its own goal. Returns each agent's final report. Use for broad multi-file investigation ('explore' mode) or independent atomic code changes ('code' mode). Goals must be self-contained: include file paths, symbols and constraints. All agents in one call run in parallel. */
            description: "生成1-4个并行子代理，每个代理拥有全新的上下文和自己的目标。返回每个代理的最终报告。用于广泛的跨文件调查（'explore'模式）或独立的原子级代码修改（'code'模式）。目标必须自包含：包括文件路径、符号和约束条件。一次调用中的所有代理并行运行。".into(),
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
                                "name": { "type": "string", "description": "向用户显示的简短标签，例如'auth-flow-investigator'" },
                                "goal": { "type": "string", "description": "自包含的指令：任务、已知文件路径/符号、约束条件" },
                                "mode": { "type": "string", "enum": ["explore", "code"], "description": "explore = 仅读工具；code = 可以编辑文件并运行bash（需要用户批准）" },
                                "expected_output": { "type": "string", "description": "最终报告必须包含的内容" }
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
        /* Original (EN): Write the Solution Design plan to <workspace>/.claudinio/plans/YYYY-MM-DD_<name>.md. Overwrites the file, so always pass the FULL plan content — call again with the same name and the complete updated text to revise. Structure: Context, Solution Design, Risks, Tasks summary. */
        description: "将解决方案设计计划写入<workspace>/.claudinio/plans/YYYY-MM-DD_<name>.md。覆盖文件，因此始终传递完整的计划内容——使用相同名称和完整更新文本再次调用以进行修订。结构：上下文、解决方案设计、风险、任务摘要。".into(),
        input_schema: serde_json::json!({
            "type": "object",
            "properties": {
                "name": { "type": "string", "description": "简短的计划名称；成为文件标识符（例如'dark mode toggle'）" },
                "content": { "type": "string", "description": "计划的完整Markdown内容" }
            },
            "required": ["name", "content"]
        }),
    }
}

/// Definition of the finalize_plan tool. Only offered in Builder mode — it
/// appends an Implementation Log (changed files, commits, journal) to the plan
/// `.md`, feeding the plan with data for future reference once a build is done.
pub fn finalize_plan_def() -> ToolDef {
    ToolDef {
        name: "finalize_plan".into(),
        /* Original (EN): Append an Implementation Log to the plan file once its implementation is DONE and verified. Records the CHANGED FILES and COMMITS automatically (read from git), so your `journal` should focus on findings, decisions, and gotchas — not a file list. Call this as the LAST step of a Builder run, after all tasks are done. Defaults to the most recent plan in the plans directory. */
        description: "当计划实施完成并验证后，将实施日志追加到计划文件中。自动记录变更文件和提交信息（从git读取），因此你的`journal`应专注于发现、决策和陷阱——而非文件列表。在构建器运行的最后一步调用，在所有任务完成后。默认为plans目录中最新的计划。".into(),
        input_schema: serde_json::json!({
            "type": "object",
            "properties": {
                "journal": { "type": "string", "description": "实施过程中的发现、决策和陷阱（'为什么'以及学到了什么）。" },
                "plan_file": { "type": "string", "description": "可选的目标计划文件（basename如'2026-07-09_x.md'或路径）。默认为最近修改的计划。" },
                "summary": { "type": "string", "description": "可选的实施单行摘要。" }
            },
            "required": ["journal"]
        }),
    }
}

/// Definition of the enter_plan_mode tool. Only offered in Builder mode.
pub fn enter_plan_mode_def() -> ToolDef {
    ToolDef {
        name: "enter_plan_mode".into(),
        /* Original (EN): Switch this session into Brain (planning) mode. Use when the task turns out to be genuinely hard or ambiguous — unclear requirements, large design space, conflicting constraints — and designing first beats guessing. Editing tools are disabled until the plan and tasks are ready; because you initiated it, you can return with exit_plan_mode. */
        description: "将会话切换到Brain（规划）模式。当任务确实困难或模糊不清时使用——需求不明确、设计空间大、约束冲突——先设计优于猜测。在计划和任务准备好之前，编辑工具将被禁用；因为是你发起的，你可以通过exit_plan_mode返回。".into(),
        input_schema: serde_json::json!({
            "type": "object",
            "properties": {
                "reason": { "type": "string", "description": "一句话：为什么此任务需要先规划（向用户显示）" }
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
        /* Original (EN): Leave Brain mode and return to Builder to execute the plan. Only works if YOU entered Brain via enter_plan_mode; when the user enabled Brain, only their toggle can exit — finish by telling them the plan and tasks are ready. */
        description: "离开Brain模式并返回Builder以执行计划。仅当你通过enter_plan_mode进入Brain时有效；当用户启用Brain时，只有他们的开关可以退出——通过告知他们计划和任务已就绪来完成。".into(),
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
                tracker.record_read(&path, start_line, end_line, &content);
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
            let file_path = args.get("file_path").or_else(|| args.get("path")).and_then(|v| v.as_str()).ok_or("missing file_path")?;
            validate_path(file_path, ctx)?;
            let results = db.symbols_in_file(file_path)?;
            Ok(ToolOutput::Text { content: serde_json::to_string_pretty(&results).unwrap_or_default() })
        }
        "go_to_definition" => {
            let file_path = args.get("file_path").or_else(|| args.get("path")).and_then(|v| v.as_str()).ok_or("missing file_path")?;
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
            let file_path = args.get("file_path").or_else(|| args.get("path")).and_then(|v| v.as_str()).ok_or("missing file_path")?;
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
        "finalize_plan" => {
            let a: finalize_plan::FinalizePlanArgs = serde_json::from_value(args).map_err(|e| format!("invalid args: {e}"))?;
            let content = finalize_plan::execute(a, ctx)?;
            Ok(ToolOutput::Text { content })
        }
        "web_search" => {
            let a: web_search::WebSearchArgs = serde_json::from_value(args).map_err(|e| format!("invalid args: {e}"))?;
            let config = ctx
                .agent_config
                .as_ref()
                .ok_or("web_search unavailable: no agent config loaded")?;
            let content = web_search::execute(a, config).await?;
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
    /// Verbatim text of each range that was read. We verify edits against the
    /// actual text the model saw rather than line numbers, so the gate is
    /// immune to duplicate lines and to line shifts caused by earlier edits.
    pub chunks: Vec<String>,
}

#[derive(Debug, Clone, Default)]
pub struct ReadTracker {
    pub files: HashMap<String, ReadFileRecord>,
}

impl ReadTracker {
    pub fn record_read(
        &mut self,
        path: &str,
        start_line: Option<usize>,
        end_line: Option<usize>,
        content: &str,
    ) {
        let entry = self.files.entry(path.to_string()).or_default();
        match (start_line, end_line) {
            (Some(_), Some(_)) => {
                entry.chunks.push(content.to_string());
            }
            _ => {
                // No range or incomplete range = full file read
                entry.full_read = true;
                entry.chunks.clear();
                entry.chunks.push(content.to_string());
            }
        }
    }

    /// Check whether editing `old_string` in `path` is allowed based on
    /// previously recorded reads. Returns Ok if the file was fully read or the
    /// exact `old_string` appears in some text the model previously read.
    ///
    /// Verifying by content (rather than by line number) avoids a failure mode
    /// where a repetitive first line (e.g. `</div>`) resolves to a spurious
    /// earlier match, making the gate impossible to satisfy no matter which
    /// range the model reads.
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

        if old_string.is_empty() {
            return Err("old_string cannot be empty".to_string());
        }

        // The model may edit any text it has actually read verbatim.
        if entry.chunks.iter().any(|c| c.contains(old_string)) {
            return Ok(());
        }

        // Not yet read. Point at where the text really is so the model can read
        // the correct range instead of guessing (the source of the old loop).
        let hint = match std::fs::read_to_string(path) {
            Ok(content) => match content.find(old_string) {
                Some(byte_idx) => {
                    let line_num = content[..byte_idx].matches('\n').count() + 1;
                    format!(
                        " The text is at line {line_num}. Call read_file with start_line/end_line \
                         covering it, then edit."
                    )
                }
                None => " That exact text was not found in the current file — re-read the file \
                         and copy old_string verbatim, including indentation."
                    .to_string(),
            },
            Err(e) => format!(" (could not re-read file: {e})"),
        };
        Err(format!(
            "read_file was called on {path}, but you have not read the exact text you're trying \
             to edit.{hint}"
        ))
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
            agent_config: None,
            plan_save_path: None,
            base_commit: None,
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
            err.contains("you have not read the exact text") && err.contains("line 10"),
            "error should explain the text wasn't read and point to the real line: {err}"
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
            agent_config: None,
            plan_save_path: None,
            base_commit: None,
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
            agent_config: None,
            plan_save_path: None,
            base_commit: None,
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
            agent_config: None,
            plan_save_path: None,
            base_commit: None,
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
            agent_config: None,
            plan_save_path: None,
            base_commit: None,
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
            agent_config: None,
            plan_save_path: None,
            base_commit: None,
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
            agent_config: None,
            plan_save_path: None,
            base_commit: None,
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
            agent_config: None,
            plan_save_path: None,
            base_commit: None,
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
            agent_config: None,
            plan_save_path: None,
            base_commit: None,
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
    // ── file_path ↔ path fallback tests ──

    #[test]
    fn test_read_file_accepts_file_path_fallback() {
        let p = write_20line_file("fallback_file_path");
        let ctx = test_ctx();
        let args = serde_json::json!({"file_path": p.to_string_lossy()});
        let result = futures::executor::block_on(execute("read_file", args, &ctx));
        let output = result.expect("read_file with file_path should succeed");
        match output {
            ToolOutput::Text { content } => {
                assert_eq!(content.lines().count(), 20, "should return all 20 lines");
            }
            _ => panic!("expected Text variant"),
        }
        let _ = std::fs::remove_file(&p);
    }

    #[test]
    fn test_list_dir_accepts_file_path_fallback() {
        let tmp = std::env::temp_dir().join("claudinio_test_list_dir_fallback");
        let _ = std::fs::create_dir_all(&tmp);
        let ctx = test_ctx();
        let args = serde_json::json!({"file_path": tmp.to_string_lossy()});
        let result = futures::executor::block_on(execute("list_dir", args, &ctx));
        result.expect("list_dir with file_path should succeed");
        let _ = std::fs::remove_dir_all(&tmp);
    }

    #[test]
    fn test_file_outline_accepts_path_fallback() {
        let p = write_20line_file("fallback_outline_path");
        let ctx = test_ctx();
        let args = serde_json::json!({"path": p.to_string_lossy()});
        let result = futures::executor::block_on(execute("file_outline", args, &ctx));
        match result {
            Ok(_) => {}
            Err(e) => {
                assert!(
                    !e.contains("missing file_path"),
                    "should NOT fail with 'missing file_path', got: {e}"
                );
            }
        }
        let _ = std::fs::remove_file(&p);
    }
}
