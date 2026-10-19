//! The `skills` tool: the catalog in the system prompt is a reference (one
//! line per skill, `skills::build_skills_system_prompt_section`), and this is
//! where a skill's instructions come from when a task needs them.

use super::ToolDef;
use crate::agent::skills::SkillManager;
use serde_json::Value;

pub fn def() -> ToolDef {
    ToolDef {
        name: "skills".into(),
        description: "Skills are instructions for specific kinds of task. action='list' returns \
                      every installed skill (name, scope, location, description); action='read' \
                      returns one skill's instructions."
            .into(),
        input_schema: serde_json::json!({
            "type": "object",
            "properties": {
                "action": { "type": "string", "enum": ["list", "read"] },
                "name": { "type": "string", "description": "For read: the skill's name, as the catalog lists it" }
            },
            "required": ["action"]
        }),
    }
}

/// `list` or `read`, against the skills `mgr` found.
pub fn execute(args: Value, mgr: &SkillManager) -> Result<String, String> {
    match args.get("action").and_then(|v| v.as_str()) {
        Some("list") => Ok(list(mgr)),
        Some("read") => {
            let name = args
                .get("name")
                .and_then(|v| v.as_str())
                .filter(|n| !n.trim().is_empty())
                .ok_or("action='read' needs the skill's name: {\"action\": \"read\", \"name\": \"...\"}")?;
            read(mgr, name.trim())
        }
        other => Err(format!(
            "unknown action {}: use {{\"action\": \"list\"}} or {{\"action\": \"read\", \"name\": \"...\"}}",
            other
                .map(|a| format!("'{a}'"))
                .unwrap_or_else(|| "(none)".into())
        )),
    }
}

/// Every skill in priority order, each with what the prompt leaves out: its
/// scope, where it lives and the whole description.
fn list(mgr: &SkillManager) -> String {
    let entries = mgr.by_priority();
    if entries.is_empty() {
        return "No skills are installed.".into();
    }
    let mut out = format!("{} skills, the user's own first:\n", entries.len());
    for s in entries {
        out.push_str(&format!(
            "- {} [{}] {}\n  {}\n",
            s.name,
            s.scope.as_str(),
            s.location,
            crate::agent::skills::one_line(&s.description, usize::MAX)
        ));
    }
    out
}

/// One skill's instructions. A skill on disk is read again rather than taken
/// from the scan, so an edit made during the session is what the model gets;
/// the scanned body is the fallback when the file has gone. A built-in has no
/// file and is answered from memory.
fn read(mgr: &SkillManager, name: &str) -> Result<String, String> {
    let Some(entry) = mgr.get(name) else {
        let known: Vec<&str> = mgr.by_priority().iter().map(|s| s.name.as_str()).collect();
        return Err(format!(
            "no skill named '{name}'. Installed: {}",
            if known.is_empty() {
                "none".to_string()
            } else {
                known.join(", ")
            }
        ));
    };
    if entry.scope == crate::agent::skills::SkillScope::Builtin {
        let body = mgr.get_body(name).unwrap_or_default();
        return Ok(format!("# Skill: {name} (built-in)\n\n{body}"));
    }
    let path = std::path::Path::new(&entry.location);
    let body = SkillManager::parse_skill_md(path)
        .map(|(_, body)| body)
        .ok()
        .or_else(|| mgr.get_body(name))
        .ok_or_else(|| format!("skill '{name}' could not be read from {}", entry.location))?;
    let dir = path
        .parent()
        .map(|p| p.to_string_lossy().to_string())
        .unwrap_or_default();
    Ok(format!(
        "# Skill: {name}\nDirectory: {dir}\nWhen this skill names relative paths, they resolve \
         against that directory; use absolute paths in tool calls.\n\n{body}"
    ))
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::agent::skills::{SkillEntry, SkillScope};

    fn on_disk(tag: &str) -> (std::path::PathBuf, SkillManager) {
        let dir = std::env::temp_dir().join(format!(
            "claudinio-skills-tool-{tag}-{}-{}",
            std::process::id(),
            crate::agent::persist::now_ms()
        ));
        std::fs::create_dir_all(dir.join("deploy")).unwrap();
        let md = dir.join("deploy").join("SKILL.md");
        std::fs::write(
            &md,
            "---\nname: deploy\ndescription: Ship the app to production\n---\n\n# Deploy\n\nRun `scripts/deploy.sh`.",
        )
        .unwrap();
        let mut mgr = SkillManager::builtin_only();
        mgr.insert_for_test(SkillEntry {
            name: "deploy".into(),
            description: "Ship the app to production".into(),
            location: md.to_string_lossy().to_string(),
            scope: SkillScope::Project,
            body: Some("stale body from the scan".into()),
        });
        (dir, mgr)
    }

    #[test]
    fn list_returns_every_skill_with_scope_location_and_the_whole_description() {
        let (dir, mgr) = on_disk("list");
        let out = execute(serde_json::json!({"action": "list"}), &mgr).unwrap();
        let deploy = out.find("deploy").expect("deploy listed");
        let builtin = out.find("find-skills").expect("built-ins listed");
        assert!(deploy < builtin, "the user's skills come first: {out}");
        assert!(out.contains("project") && out.contains("builtin"), "{out}");
        assert!(out.contains(&dir.join("deploy/SKILL.md").to_string_lossy().to_string()));
        // `list` is where the full description lives — the prompt cuts it.
        assert!(
            out.contains("might exist as an installable skill."),
            "{out}"
        );
        std::fs::remove_dir_all(&dir).ok();
    }

    #[test]
    fn read_returns_a_skill_from_disk_with_its_directory() {
        let (dir, mgr) = on_disk("read");
        let out = execute(
            serde_json::json!({"action": "read", "name": "deploy"}),
            &mgr,
        )
        .unwrap();
        assert!(out.contains("Run `scripts/deploy.sh`."), "{out}");
        assert!(
            !out.contains("stale body"),
            "read from disk, not the scan: {out}"
        );
        let skill_dir = dir.join("deploy").to_string_lossy().to_string();
        assert!(out.contains(&skill_dir), "{out}");
        assert!(out.contains("relative paths"), "{out}");
        std::fs::remove_dir_all(&dir).ok();
    }

    #[test]
    fn read_returns_a_built_in_skill_from_memory() {
        let mgr = SkillManager::builtin_only();
        let out = execute(
            serde_json::json!({"action": "read", "name": "plugin-crafter"}),
            &mgr,
        )
        .unwrap();
        assert!(out.contains("## 1. Interview first"), "{out}");
        assert!(
            !out.contains("relative paths"),
            "a built-in has no directory: {out}"
        );
    }

    #[test]
    fn an_unknown_skill_or_action_is_an_error_that_says_what_exists() {
        let mgr = SkillManager::builtin_only();
        let err = execute(serde_json::json!({"action": "read", "name": "nope"}), &mgr).unwrap_err();
        assert!(err.contains("nope") && err.contains("find-skills"), "{err}");
        let err = execute(serde_json::json!({"action": "read"}), &mgr).unwrap_err();
        assert!(err.contains("name"), "{err}");
        let err = execute(serde_json::json!({"action": "write"}), &mgr).unwrap_err();
        assert!(err.contains("list") && err.contains("read"), "{err}");
    }
}
