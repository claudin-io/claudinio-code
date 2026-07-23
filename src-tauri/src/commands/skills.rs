use crate::agent::skills::{self, build_skills_system_prompt_section, RemoteSkill, SkillEntry};
use serde::{Deserialize, Serialize};
use tauri::State;

use crate::state::AppState;

#[derive(Serialize)]
#[serde(rename_all = "camelCase")]
pub struct SkillsResponse {
    pub skills: Vec<SkillEntry>,
    pub count: usize,
}

/// List all locally discovered skills for a workspace.
#[tauri::command]
pub async fn list_skills(
    workspace: String,
    state: State<'_, AppState>,
) -> Result<SkillsResponse, String> {
    let ws = state.workspace(&workspace).await?;
    let mgr = ws.skills_manager.lock().await;
    let skills = mgr.list();
    let count = skills.len();
    Ok(SkillsResponse { skills, count })
}

/// Get the catalog (name + description only) for system prompt injection.
#[tauri::command]
pub async fn get_skill_catalog(
    workspace: String,
    state: State<'_, AppState>,
) -> Result<Vec<String>, String> {
    let ws = state.workspace(&workspace).await?;
    let mgr = ws.skills_manager.lock().await;
    let section = build_skills_system_prompt_section(&mgr);
    Ok(vec![section.unwrap_or_default()])
}

/// Get the full SKILL.md content for a skill by name.
#[tauri::command]
pub async fn get_skill_content(
    workspace: String,
    name: String,
    state: State<'_, AppState>,
) -> Result<serde_json::Value, String> {
    let ws = state.workspace(&workspace).await?;
    let mgr = ws.skills_manager.lock().await;
    match mgr.get(&name) {
        Some(entry) => {
            let body = mgr.get_body(&name).unwrap_or_default();
            Ok(serde_json::json!({
                "name": entry.name,
                "description": entry.description,
                "location": entry.location,
                "scope": entry.scope,
                "body": body,
            }))
        }
        None => Err(format!("skill '{}' not found", name)),
    }
}

/// Re-scan all skill directories for a workspace.
#[tauri::command]
pub async fn rescan_skills(
    workspace: String,
    state: State<'_, AppState>,
) -> Result<SkillsResponse, String> {
    let ws = state.workspace(&workspace).await?;
    let mut mgr = ws.skills_manager.lock().await;
    let count = mgr.scan();
    let skills = mgr.list();
    Ok(SkillsResponse { skills, count })
}

/// Find remote skills from the registry, optionally filtered by query.
#[tauri::command]
pub async fn find_remote_skills(query: Option<String>) -> Result<Vec<RemoteSkill>, String> {
    skills::find_remote_skills(query.as_deref()).await
}

/// Preview a remote skill without installing it.
#[tauri::command]
pub async fn preview_remote_skill(url: String) -> Result<SkillEntry, String> {
    skills::preview_remote_skill(&url).await
}

#[derive(Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct InstallRemoteSkillArgs {
    pub name: String,
    pub url: String,
    pub description: String,
}

/// Install a remote skill (with user approval already obtained on frontend).
#[tauri::command]
pub async fn install_remote_skill(
    workspace: String,
    args: InstallRemoteSkillArgs,
    state: State<'_, AppState>,
) -> Result<SkillEntry, String> {
    let remote = RemoteSkill {
        name: args.name,
        description: args.description,
        url: args.url,
        source: skills::SkillSource::Url(String::new()),
    };

    let entry = skills::install_remote_skill(&remote).await?;

    // Re-scan so the new skill is available immediately
    let ws = state.workspace(&workspace).await?;
    let mut mgr = ws.skills_manager.lock().await;
    mgr.scan();

    Ok(entry)
}
