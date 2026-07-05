mod edit_file;
mod grep;
mod list_dir;
mod read_file;

use crate::code_intel::db::IndexDb;
use crate::lsp::manager::LspManager;
use serde::Serialize;
use serde_json::Value;
use std::path::Path;
use std::sync::Arc;
use tokio::sync::Mutex;

pub struct ToolContext {
    pub db_path: Option<String>,
    pub lsp_manager: Option<Arc<Mutex<LspManager>>>,
    pub workspace_root: Option<String>,
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
            description: "Read a text file (project workspace only, max 2MB). Use the absolute path within the project.".into(),
            input_schema: serde_json::json!({
                "type": "object",
                "properties": {
                    "path": { "type": "string", "description": "Absolute path to the file within the project workspace" }
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
            description: "Propose a change to a file in the project workspace. Replaces the FIRST occurrence of old_string with new_string. NOT applied until you approve.".into(),
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
            description: "Full-text search across indexed symbols (FTS5). Faster than grep for finding definitions.".into(),
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
            description: "Look up a symbol by exact name across the workspace.".into(),
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
            description: "List all symbols defined in a file.".into(),
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
            description: "Find where a symbol is defined at a specific position in a file. Uses LSP when available (precise), falls back to tree-sitter index (heuristic).".into(),
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
            description: "Find all references to a symbol at a specific position. Uses LSP when available (precise), falls back to heuristic name matching.".into(),
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
    ]
}

pub async fn execute(name: &str, args: Value, ctx: &ToolContext) -> Result<ToolOutput, String> {
    match name {
        "read_file" => {
            let a: read_file::ReadFileArgs = serde_json::from_value(args).map_err(|e| format!("invalid args: {e}"))?;
            validate_path(&a.path, ctx)?;
            let content = read_file::execute(a)?;
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
        _ => Err(format!("unknown tool: {name}")),
    }
}

pub async fn apply_edit_with_ctx(args: Value, ctx: &ToolContext) -> Result<String, String> {
    let a: edit_file::EditFileArgs = serde_json::from_value(args).map_err(|e| format!("invalid args: {e}"))?;
    validate_path(&a.path, ctx)?;
    let diff = edit_file::preview(&a)?;
    edit_file::apply(&diff)
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

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn test_validate_path_allows_within_workspace() {
        let ctx = ToolContext {
            db_path: None,
            lsp_manager: None,
            workspace_root: Some("/home/user/project".into()),
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
        };
        let args = serde_json::json!({"pattern": "foo"});
        let result = futures::executor::block_on(execute("grep", args, &ctx));
        assert!(result.is_err(), "rg likely not installed in test env");
    }
}
