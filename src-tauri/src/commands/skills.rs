use crate::agent::skills::{
    self, build_skills_system_prompt_section, RemoteSkill, SkillEntry,
};
use serde::{Deserialize, Serialize};
use tauri::State;

use crate::state::AppState;

#[derive(Serialize)]
#[serde(rename_all = "camelCase")]
pub struct SkillsResponse {
    pub skills: Vec<SkillEntry>,
    pub count: usize,
}

/// List all locally discovered skills.
#[tauri::command]
pub async fn list_skills(state: State<'_, AppState>) -> Result<SkillsResponse, String> {
    let mgr = state.skills_manager.lock().await;
    let skills = mgr.list();
    let count = skills.len();
    Ok(SkillsResponse { skills, count })
}

/// Get the catalog (name + description only) for system prompt injection.
#[tauri::command]
pub async fn get_skill_catalog(state: State<'_, AppState>) -> Result<Vec<String>, String> {
    let mgr = state.skills_manager.lock().await;
    let catalog = mgr.catalog();
    let section = build_skills_system_prompt_section(&catalog);
    Ok(vec![section.unwrap_or_default()])
}

/// Get the full SKILL.md content for a skill by name.
#[tauri::command]
pub async fn get_skill_content(
    name: String,
    state: State<'_, AppState>,
) -> Result<serde_json::Value, String> {
    let mgr = state.skills_manager.lock().await;
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

/// Re-scan all skill directories.
#[tauri::command]
pub async fn rescan_skills(state: State<'_, AppState>) -> Result<SkillsResponse, String> {
    let mut mgr = state.skills_manager.lock().await;
    let count = mgr.scan();
    let skills = mgr.list();
    Ok(SkillsResponse { skills, count })
}

/// Find remote skills from the registry, optionally filtered by query.
#[tauri::command]
pub async fn find_remote_skills(
    query: Option<String>,
) -> Result<Vec<RemoteSkill>, String> {
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
    let mut mgr = state.skills_manager.lock().await;
    mgr.scan();

    Ok(entry)
}
