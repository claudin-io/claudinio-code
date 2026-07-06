use serde::Deserialize;
use std::path::PathBuf;

/// Write (or overwrite) a plan document at
/// `<workspace>/.claudinio/plans/YYYY-MM-DD_<slug>.md`.
#[derive(Deserialize)]
pub struct WritePlanArgs {
    pub name: String,
    pub content: String,
}

/// Turn a free-form plan name into a filesystem-safe slug.
fn slugify(name: &str) -> String {
    let mut slug = String::new();
    let mut last_dash = true; // suppress a leading dash
    for c in name.chars() {
        let c = c.to_ascii_lowercase();
        if c.is_ascii_alphanumeric() {
            slug.push(c);
            last_dash = false;
        } else if !last_dash {
            slug.push('-');
            last_dash = true;
        }
    }
    while slug.ends_with('-') {
        slug.pop();
    }
    if slug.is_empty() {
        slug.push_str("plan");
    }
    slug
}

pub fn plans_dir(workspace_root: &str) -> PathBuf {
    PathBuf::from(workspace_root).join(".claudinio").join("plans")
}

pub fn execute(args: WritePlanArgs, ctx: &crate::agent::tools::ToolContext) -> Result<String, String> {
    let root = ctx
        .workspace_root
        .as_ref()
        .ok_or("write_plan requires an open workspace")?;
    let dir = plans_dir(root);
    std::fs::create_dir_all(&dir).map_err(|e| format!("create plans dir: {e}"))?;

    let date = chrono::Local::now().format("%Y-%m-%d");
    let path = dir.join(format!("{date}_{}.md", slugify(&args.name)));
    std::fs::write(&path, &args.content).map_err(|e| format!("write plan: {e}"))?;
    Ok(format!(
        "Plan written to {} ({} bytes). Call write_plan again with the same name and the \
         full updated content to revise it.",
        path.to_string_lossy(),
        args.content.len()
    ))
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn slugify_basic() {
        assert_eq!(slugify("Modo Pensador / Constructor"), "modo-pensador-constructor");
        assert_eq!(slugify("  weird__name!! "), "weird-name");
        assert_eq!(slugify("///"), "plan");
    }
}
