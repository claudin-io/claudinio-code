//! The `doc_*` tools: the model's hands on the session's living document
//! (lean-harness plan, Phase 4b). Definitions and argument parsing; the work
//! is `agent::doc_session`'s.
//!
//! `doc_create` is core in a Standard main session and is answered by the run
//! loop, because a session that already has a document hands off to a new
//! one (`doc_session::create_call`). The other three are deferred and never
//! named in the catalog: they mean nothing without a document, and
//! `doc_create` loads them (`deferred::before_call`).

use super::{ToolDef, ToolOutput};
use crate::agent::document::Status;
use serde_json::Value;

pub fn doc_create_def() -> ToolDef {
    ToolDef {
        name: "doc_create".into(),
        description: "Create the session's living document (.claudinio/docs/<title>.md): a title and one placeholder per section topic.".into(),
        input_schema: serde_json::json!({
            "type": "object",
            "properties": {
                "title": { "type": "string" },
                "sections": { "type": "array", "items": { "type": "string" } }
            },
            "required": ["title", "sections"]
        }),
    }
}

pub fn doc_write_def() -> ToolDef {
    ToolDef {
        name: "doc_write".into(),
        description: "Write one section of the session's document by id. status 'doing' claims it while you work (body optional), 'done' (default) finishes it. Refused, with the section's current text, when the user edited it since you last read it: merge and write again. Answers with the outline.".into(),
        input_schema: serde_json::json!({
            "type": "object",
            "properties": {
                "id": { "type": "string", "description": "Section id, e.g. s2" },
                "body": { "type": "string", "description": "The section's Markdown text, without its heading" },
                "status": { "type": "string", "enum": ["doing", "done"] }
            },
            "required": ["id"]
        }),
    }
}

pub fn doc_read_def() -> ToolDef {
    ToolDef {
        name: "doc_read".into(),
        description: "Read one section of the session's document by id; without an id, the outline (id · status · owner · topic).".into(),
        input_schema: serde_json::json!({
            "type": "object",
            "properties": { "id": { "type": "string" } }
        }),
    }
}

pub fn doc_add_def() -> ToolDef {
    ToolDef {
        name: "doc_add".into(),
        description: "Add a placeholder section to the session's document, after section `after` or at the end.".into(),
        input_schema: serde_json::json!({
            "type": "object",
            "properties": {
                "topic": { "type": "string" },
                "after": { "type": "string", "description": "Section id to insert after" }
            },
            "required": ["topic"]
        }),
    }
}

/// The definitions a Standard main session gets: `doc_create` (core) and the
/// three deferred ones.
pub fn session_defs() -> Vec<ToolDef> {
    vec![
        doc_create_def(),
        doc_write_def(),
        doc_read_def(),
        doc_add_def(),
    ]
}

fn str_arg<'a>(args: &'a Value, key: &str) -> Option<&'a str> {
    args.get(key)
        .and_then(|v| v.as_str())
        .map(str::trim)
        .filter(|s| !s.is_empty())
}

/// `doc_write` / `doc_read` / `doc_add`.
pub fn execute(name: &str, args: &Value, ctx: &super::ToolContext) -> Result<ToolOutput, String> {
    let content = match name {
        "doc_write" => {
            let id = str_arg(args, "id").ok_or("doc_write needs a section id")?;
            let status = match str_arg(args, "status") {
                None | Some("done") => Status::Done,
                Some("doing") => Status::Doing,
                Some(other) => {
                    return Err(format!("status is 'doing' or 'done', not '{other}'"));
                }
            };
            let body = args.get("body").and_then(|v| v.as_str());
            crate::agent::doc_session::write(ctx, id, body, status)?
        }
        "doc_read" => crate::agent::doc_session::read(ctx, str_arg(args, "id"))?,
        "doc_add" => {
            let topic = str_arg(args, "topic").ok_or("doc_add needs a topic")?;
            crate::agent::doc_session::add(ctx, topic, str_arg(args, "after"))?
        }
        _ => return Err("doc_create is for the main session only".into()),
    };
    Ok(ToolOutput::Text { content })
}
