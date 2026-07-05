use crate::lsp::client::Location;
use crate::state::AppState;
use serde::{Deserialize, Serialize};
use tauri::State;

#[derive(Serialize)]
#[serde(rename_all = "camelCase")]
pub struct LspLocation {
    pub uri: String,
    pub start_line: u64,
    pub start_char: u64,
    pub end_line: u64,
    pub end_char: u64,
}

#[derive(Deserialize)]
pub struct LspPositionArgs {
    pub file_path: String,
    pub line: u64,
    pub character: u64,
}

#[derive(Serialize)]
#[serde(rename_all = "camelCase")]
pub struct HoverInfo {
    pub contents: String,
    pub start_line: Option<u64>,
    pub start_char: Option<u64>,
    pub end_line: Option<u64>,
    pub end_char: Option<u64>,
}

fn convert(loc: &Location) -> LspLocation {
    LspLocation {
        uri: loc.uri.clone(),
        start_line: loc.range.start_line,
        start_char: loc.range.start_char,
        end_line: loc.range.end_line,
        end_char: loc.range.end_char,
    }
}

#[tauri::command]
pub async fn lsp_definition(
    args: LspPositionArgs,
    state: State<'_, AppState>,
) -> Result<Vec<LspLocation>, String> {
    let mut mgr = state.lsp_manager.lock().await;
    let locations = mgr.goto_definition(&args.file_path, args.line, args.character)?;
    Ok(locations.iter().map(convert).collect())
}

#[tauri::command]
pub async fn lsp_references(
    args: LspPositionArgs,
    state: State<'_, AppState>,
) -> Result<Vec<LspLocation>, String> {
    let mut mgr = state.lsp_manager.lock().await;
    let locations = mgr.find_references(&args.file_path, args.line, args.character)?;
    Ok(locations.iter().map(convert).collect())
}

#[tauri::command]
pub async fn lsp_hover(
    args: LspPositionArgs,
    state: State<'_, AppState>,
) -> Result<Option<HoverInfo>, String> {
    let mut mgr = state.lsp_manager.lock().await;
    let result = mgr.hover(&args.file_path, args.line, args.character)?;
    Ok(result.map(|h| HoverInfo {
        contents: h.contents,
        start_line: h.range.as_ref().map(|r| r.start_line),
        start_char: h.range.as_ref().map(|r| r.start_char),
        end_line: h.range.as_ref().map(|r| r.end_line),
        end_char: h.range.as_ref().map(|r| r.end_char),
    }))
}
