use serde::Serialize;
use std::path::Path;
use tauri::State;

use crate::state::AppState;

/// Stats about the project's context injection files and skills.
#[derive(Serialize)]
#[serde(rename_all = "camelCase")]
pub struct ContextWarningData {
    /// Size of AGENTS.md (or CLAUDE.md) in bytes. 0 if absent.
    pub agents_md_size: u64,
    /// Number of lines in AGENTS.md (or CLAUDE.md).
    pub agents_md_lines: u64,
    /// Estimated token count (chars/4) for AGENTS.md.
    pub agents_md_tokens: u64,
    /// Number of issues/lines matching common patterns (TODO, FIXME, HACK, XXX).
    pub agents_md_issues: u64,
    /// Path to the AGENTS.md/CLAUDE.md file found.
    pub agents_md_path: Option<String>,
    /// Total number of installed skills.
    pub skills_count: usize,
    /// Combined estimated tokens consumed by all skill SKILL.md bodies.
    pub skills_total_tokens: u64,
    /// Per-skill breakdown: name + estimated tokens.
    pub skills_breakdown: Vec<SkillTokenEntry>,
}

#[derive(Serialize)]
#[serde(rename_all = "camelCase")]
pub struct SkillTokenEntry {
    pub name: String,
    pub description: String,
    pub estimated_tokens: u64,
    pub location: String,
}

/// Scan the workspace for AGENTS.md / CLAUDE.md and skill stats.
/// Returns data the frontend displays in a warning modal.
#[tauri::command]
pub async fn get_context_warning(
    workspace: String,
    state: State<'_, AppState>,
) -> Result<ContextWarningData, String> {
    let ws = state.workspace(&workspace).await?;

    // --- 1. Find AGENTS.md / CLAUDE.md ---
    let root = Path::new(&ws.root);
    let candidates = [
        ("AGENTS.md", root.join("AGENTS.md")),
        ("agents.md", root.join("agents.md")),
        ("AGENTS", root.join("AGENTS")),
        ("CLAUDE.md", root.join("CLAUDE.md")),
        ("claude.md", root.join("claude.md")),
    ];

    let mut agents_md_size: u64 = 0;
    let mut agents_md_lines: u64 = 0;
    let mut agents_md_tokens: u64 = 0;
    let mut agents_md_issues: u64 = 0;
    let mut agents_md_path: Option<String> = None;

    for (name, path) in &candidates {
        if path.exists() && path.is_file() {
            agents_md_path = Some(name.to_string());
            if let Ok(content) = std::fs::read_to_string(path) {
                let bytes = content.len() as u64;
                let lines = content.lines().count() as u64;
                let tokens = (bytes / 4).max(1);

                // Count issue-like lines
                let issues = content
                    .lines()
                    .filter(|l| {
                        let t = l.trim();
                        t.starts_with("TODO")
                            || t.starts_with("FIXME")
                            || t.starts_with("HACK")
                            || t.starts_with("XXX")
                            || t.starts_with("BUG")
                            || t.starts_with("ISSUE")
                            || t.starts_with("# TODO")
                            || t.starts_with("# FIXME")
                            || t.starts_with("// TODO")
                            || t.starts_with("// FIXME")
                            || t.starts_with("<!-- TODO")
                    })
                    .count() as u64;

                agents_md_size = bytes;
                agents_md_lines = lines;
                agents_md_tokens = tokens;
                agents_md_issues = issues;
            }
            break;
        }
    }

    // --- 2. Skill stats ---
    let mgr = ws.skills_manager.lock().await;
    let skills = mgr.list();
    let skills_count = skills.len();

    let mut skills_total_tokens: u64 = 0;
    let mut skills_breakdown: Vec<SkillTokenEntry> = Vec::new();

    for skill in &skills {
        if let Some(body) = mgr.get_body(&skill.name) {
            let body_tokens = (body.len() as u64 / 4).max(1);
            skills_total_tokens += body_tokens;
            skills_breakdown.push(SkillTokenEntry {
                name: skill.name.clone(),
                description: skill.description.clone(),
                estimated_tokens: body_tokens,
                location: skill.location.clone(),
            });
        }
    }

    Ok(ContextWarningData {
        agents_md_size,
        agents_md_lines,
        agents_md_tokens,
        agents_md_issues,
        agents_md_path,
        skills_count,
        skills_total_tokens,
        skills_breakdown,
    })
}
