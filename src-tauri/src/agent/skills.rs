use serde::{Deserialize, Serialize};
use std::collections::HashMap;
use std::path::{Path, PathBuf};

// ─── Data types ───────────────────────────────────────────────────────────────

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct SkillMeta {
    pub name: String,
    pub description: String,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub license: Option<String>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub compatibility: Option<String>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub metadata: Option<HashMap<String, String>>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub allowed_tools: Option<String>,
}

#[derive(Debug, Clone, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct SkillEntry {
    pub name: String,
    pub description: String,
    /// Absolute path to the SKILL.md file
    pub location: String,
    /// Scope where the skill was found
    pub scope: SkillScope,
    /// The full body content (markdown after frontmatter). Filled lazily.
    #[serde(skip_serializing)]
    pub body: Option<String>,
}

#[derive(Debug, Clone, Serialize, Deserialize, PartialEq)]
#[serde(rename_all = "camelCase")]
pub enum SkillScope {
    Project,
    Subfolder,
    User,
}

/// A catalog entry that goes into the system prompt — only name + description.
#[derive(Debug, Clone, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct SkillCatalogEntry {
    pub name: String,
    pub description: String,
    pub location: String,
    pub scope: SkillScope,
}

/// Remote skill listing from the registry API.
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct RemoteSkill {
    pub name: String,
    pub description: String,
    /// URL to the raw SKILL.md
    pub url: String,
    pub source: SkillSource,
}

#[derive(Debug, Clone, Serialize, Deserialize, PartialEq)]
#[serde(rename_all = "camelCase")]
pub enum SkillSource {
    GitHub { owner: String, repo: String, path: String },
    Url(String),
}

// ─── Directories to scan, in priority order ───────────────────────────────────
// Project wins over Subfolder wins over User.
// Within each scope, we scan in order: .agents/skills/ → .claudinio/skills/ → .claude/skills/
// The first skill with a given name wins (higher priority scopes override lower).

const SKILL_DIR_NAMES: &[&str] = &[".agents", ".claudinio", ".claude"];

fn scan_for_skills(root: &Path, scope: SkillScope, skills: &mut HashMap<String, (SkillEntry, SkillScope)>) {
    for dir_name in SKILL_DIR_NAMES {
        let skills_dir = root.join(dir_name).join("skills");
        if !skills_dir.exists() {
            continue;
        }
        scan_skills_dir(&skills_dir, scope.clone(), skills);
    }
}

fn scan_skills_dir(
    dir: &Path,
    scope: SkillScope,
    skills: &mut HashMap<String, (SkillEntry, SkillScope)>,
) {
    let entries = match std::fs::read_dir(dir) {
        Ok(e) => e,
        Err(_) => return,
    };

    for entry in entries.flatten() {
        let path = entry.path();
        if !path.is_dir() {
            continue;
        }

        // Skip dot dirs, node_modules, target
        let dir_name = path.file_name().and_then(|n| n.to_str()).unwrap_or("");
        if dir_name.starts_with('.') || dir_name == "node_modules" || dir_name == "target" {
            continue;
        }

        let skill_md = path.join("SKILL.md");
        if !skill_md.exists() {
            continue;
        }

        match SkillManager::parse_skill_md(&skill_md) {
            Ok((meta, body)) => {
                let name = meta.name.clone();
                // Check if we already have this skill with a HIGHER priority scope
                if let Some((_existing_entry, existing_scope)) = skills.get(&name) {
                    if has_higher_priority(&existing_scope, &scope) {
                        // Existing has higher priority — skip
                        continue;
                    }
                    // Existing has lower priority — overwrite
                }

                let entry = SkillEntry {
                    name,
                    description: meta.description.clone(),
                    location: skill_md.to_string_lossy().to_string(),
                    scope: scope.clone(),
                    body: Some(body),
                };
                skills.insert(entry.name.clone(), (entry, scope.clone()));
            }
            Err(_) => {
                // Silently skip malformed skills
            }
        }
    }
}

/// Priority order: Project > Subfolder > User
fn has_higher_priority(current: &SkillScope, incoming: &SkillScope) -> bool {
    fn rank(s: &SkillScope) -> u8 {
        match s {
            SkillScope::Project => 3,
            SkillScope::Subfolder => 2,
            SkillScope::User => 1,
        }
    }
    rank(current) >= rank(incoming)
}

// ─── Skill Manager ────────────────────────────────────────────────────────────

pub struct SkillManager {
    /// Absolute path to the workspace root (project).
    workspace_root: Option<PathBuf>,
    /// Skills discovered at the last scan, keyed by name.
    skills: HashMap<String, SkillEntry>,
    /// Cache of parsed frontmatter for loaded skills.
    frontmatter_cache: HashMap<String, SkillMeta>,
}

impl SkillManager {
    pub fn new(workspace_root: Option<PathBuf>) -> Self {
        let mut mgr = Self {
            workspace_root,
            skills: HashMap::new(),
            frontmatter_cache: HashMap::new(),
        };
        mgr.scan();
        mgr
    }

    /// Re-scan all skill directories. Returns how many skills were found.
    pub fn scan(&mut self) -> usize {
        let mut all: HashMap<String, (SkillEntry, SkillScope)> = HashMap::new();

        // 1. Project-level skills (highest priority)
        if let Some(ref root) = self.workspace_root {
            scan_for_skills(root, SkillScope::Project, &mut all);

            // 2. Monorepo subfolders: packages/*/
            let pkg_dirs = [
                root.join("packages"),
                root.join("apps"),
                root.join("libs"),
                root.join("modules"),
            ];
            for pkg_root in pkg_dirs.iter() {
                if let Ok(entries) = std::fs::read_dir(pkg_root) {
                    for entry in entries.flatten() {
                        let path = entry.path();
                        if path.is_dir() && !path.file_name().and_then(|n| n.to_str()).unwrap_or("").starts_with('.') {
                            scan_for_skills(&path, SkillScope::Subfolder, &mut all);
                        }
                    }
                }
            }
        }

        // 3. User-level skills (lowest priority)
        if let Some(home) = dirs::home_dir() {
            scan_for_skills(&home, SkillScope::User, &mut all);
        }

        // Flatten: keep only the entry, drop the scope (already embedded in the entry)
        self.skills = all.into_iter().map(|(k, v)| (k, v.0)).collect();
        self.skills.len()
    }

    /// Parse a SKILL.md file into its frontmatter metadata and body content.
    pub fn parse_skill_md(path: &Path) -> Result<(SkillMeta, String), String> {
        let content = std::fs::read_to_string(path)
            .map_err(|e| format!("read {}: {}", path.display(), e))?;
        Self::parse_skill_md_from_str(&content, path)
    }

    /// Parse SKILL.md content from a string (used for remote skills too).
    pub fn parse_skill_md_from_str(content: &str, _source: &Path) -> Result<(SkillMeta, String), String> {
        let content = content.trim();

        if !content.starts_with("---") {
            return Err("missing frontmatter delimiters".into());
        }

        let rest = &content[3..];
        let end = rest.find("\n---")
            .ok_or_else::<String, _>(|| "missing closing frontmatter delimiter".into())?;

        let yaml_str = &rest[..end];
        let body = rest[end + 4..].trim().to_string();

        let meta: SkillMeta = serde_yaml::from_str(yaml_str)
            .map_err(|e| format!("yaml parse error: {e}"))?;

        if meta.name.is_empty() {
            return Err("skill name cannot be empty".into());
        }
        if meta.name.len() > 64 {
            return Err("skill name exceeds 64 characters".into());
        }
        if meta.description.is_empty() || meta.description.len() > 1024 {
            return Err("description must be 1-1024 characters".into());
        }

        Ok((meta, body))
    }

    /// Get the catalog (name + description only) for system prompt disclosure.
    pub fn catalog(&self) -> Vec<SkillCatalogEntry> {
        let mut entries: Vec<SkillCatalogEntry> = self
            .skills
            .values()
            .map(|s| SkillCatalogEntry {
                name: s.name.clone(),
                description: s.description.clone(),
                location: s.location.clone(),
                scope: s.scope.clone(),
            })
            .collect();
        entries.sort_by(|a, b| a.name.cmp(&b.name));
        entries
    }

    /// Get a sorted list of all skills with full metadata.
    pub fn list(&self) -> Vec<SkillEntry> {
        let mut skills: Vec<SkillEntry> = self.skills.values().cloned().collect();
        skills.sort_by(|a, b| a.name.cmp(&b.name));
        skills
    }

    /// Get the full SKILL.md body for a skill by name.
    pub fn get_body(&self, name: &str) -> Option<String> {
        self.skills.get(name).and_then(|s| s.body.clone())
    }

    /// Reload the body for a skill from disk.
    pub fn reload_body(&mut self, name: &str) -> Option<String> {
        let location = self.skills.get(name)?.location.clone();
        let path = Path::new(&location);
        match Self::parse_skill_md(path) {
            Ok((_meta, body)) => {
                if let Some(entry) = self.skills.get_mut(name) {
                    entry.body = Some(body.clone());
                }
                Some(body)
            }
            Err(_) => None,
        }
    }

    /// Get a skill entry by name.
    pub fn get(&self, name: &str) -> Option<&SkillEntry> {
        self.skills.get(name)
    }

    /// Number of skills loaded.
    pub fn len(&self) -> usize {
        self.skills.len()
    }
}

// ─── Remote Skill Registry ────────────────────────────────────────────────────

/// Default registry URL. Points to the vercel-labs skills collection.
const REMOTE_REGISTRY_BASE: &str = "https://raw.githubusercontent.com/vercel-labs/skills/refs/heads/main";

const REMOTE_INDEX_URL: &str = "https://raw.githubusercontent.com/vercel-labs/skills/refs/heads/main/llms.txt";

/// Find skills from the remote registry by querying the centralized index.
pub async fn find_remote_skills(query: Option<&str>) -> Result<Vec<RemoteSkill>, String> {
    let client = reqwest::Client::builder()
        .timeout(std::time::Duration::from_secs(10))
        .build()
        .map_err(|e| format!("http client: {e}"))?;

    let index_text = client
        .get(REMOTE_INDEX_URL)
        .send()
        .await
        .map_err(|e| format!("fetch registry: {e}"))?
        .text()
        .await
        .map_err(|e| format!("read registry: {e}"))?;

    let all_skills = parse_remote_index(&index_text)?;

    // Filter by query if provided
    match query {
        Some(q) if !q.is_empty() => {
            let q_lower = q.to_lowercase();
            Ok(all_skills
                .into_iter()
                .filter(|s| {
                    s.name.to_lowercase().contains(&q_lower)
                        || s.description.to_lowercase().contains(&q_lower)
                })
                .collect())
        }
        _ => Ok(all_skills),
    }
}

fn parse_remote_index(index_text: &str) -> Result<Vec<RemoteSkill>, String> {
    let mut skills = Vec::new();

    for line in index_text.lines() {
        let trimmed = line.trim();
        // Match markdown list items: - [name](url): description
        if let Some(skill) = parse_markdown_link_line(trimmed) {
            // Skip the llms.txt self-reference and non-skill links
            if skill.url.contains("SKILL.md") || skill.url.contains("/home.md") {
                skills.push(skill);
            }
        }
    }

    Ok(skills)
}

fn parse_markdown_link_line(line: &str) -> Option<RemoteSkill> {
    // Match: - [name](url): description
    // or:   - [name](url) — description
    // or:   - [name](url) description
    if !line.starts_with("- [") {
        return None;
    }

    let rest = &line[3..]; // skip "- ["

    let close_bracket = rest.find("](")?;
    let name = &rest[..close_bracket];

    let after_bracket = &rest[close_bracket + 2..];
    let close_paren = after_bracket.find(')')?;
    let url = &after_bracket[..close_paren];

    let desc_part = after_bracket[close_paren + 1..].trim();

    // Skip if it's a Docs link (usually to agentskills.io docs, not skills)
    if url.starts_with("https://agentskills.io") {
        return None;
    }

    // Clean up the description: strip "colon dash" separators
    let description = desc_part
        .trim_start_matches(|c: char| c == ':' || c == '—' || c == '-' || c == ' ' || c == '\u{2014}')
        .trim()
        .to_string();

    // Skip links without descriptions
    if description.is_empty() {
        return None;
    }

    let source = infer_source_from_url(url).unwrap_or(SkillSource::Url(url.to_string()));

    Some(RemoteSkill {
        name: name.to_string(),
        description,
        url: url.to_string(),
        source,
    })
}

fn infer_source_from_url(url: &str) -> Option<SkillSource> {
    // GitHub raw URLs: https://raw.githubusercontent.com/{owner}/{repo}/refs/heads/{ref}/{path}
    // or:                     https://raw.githubusercontent.com/{owner}/{repo}/{branch}/{path}
    if let Some(rest) = url.strip_prefix("https://raw.githubusercontent.com/") {
        let parts: Vec<&str> = rest.split('/').collect();
        // GitHub raw URLs have at least 4 segments: owner, repo, branch, path
        if parts.len() >= 4 {
            let owner = parts[0].to_string();
            let repo = parts[1].to_string();
            // Skip "refs/heads/<ref>" if present — those are GitHub refs
            let path_start = if parts.len() >= 5 && parts[2] == "refs" && parts[3] == "heads" {
                5 // skip refs/heads/<name>
            } else {
                3 // skip owner/repo/branch
            };
            if path_start < parts.len() {
                let path = parts[path_start..].join("/");
                return Some(SkillSource::GitHub { owner, repo, path });
            }
        }
    }
    // GitHub regular URLs
    if let Some(rest) = url.strip_prefix("https://github.com/") {
        let parts: Vec<&str> = rest.split('/').collect();
        if parts.len() >= 4 && parts[2] == "blob" {
            return Some(SkillSource::GitHub {
                owner: parts[0].to_string(),
                repo: parts[1].to_string(),
                path: parts[3..].join("/"),
            });
        }
    }
    None
}

/// Download a remote SKILL.md and return its parsed metadata + body.
pub async fn fetch_remote_skill_meta(url: &str) -> Result<(SkillMeta, String), String> {
    let client = reqwest::Client::builder()
        .timeout(std::time::Duration::from_secs(15))
        .build()
        .map_err(|e| format!("http client: {e}"))?;

    let content = client
        .get(url)
        .send()
        .await
        .map_err(|e| format!("fetch skill: {e}"))?
        .text()
        .await
        .map_err(|e| format!("read skill: {e}"))?;

    let source_path = Path::new(url);
    SkillManager::parse_skill_md_from_str(&content, source_path)
}

/// Install a remote skill locally: download the SKILL.md and place it in the
/// user-level skills directory (`~/.claudinio/skills/<name>/SKILL.md`).
pub async fn install_remote_skill(remote: &RemoteSkill) -> Result<SkillEntry, String> {
    let (meta, body) = fetch_remote_skill_meta(&remote.url).await?;

    let home = dirs::home_dir().ok_or("no home directory")?;
    let install_dir = home.join(".claudinio").join("skills").join(&meta.name);
    std::fs::create_dir_all(&install_dir)
        .map_err(|e| format!("create install dir: {e}"))?;

    let skill_path = install_dir.join("SKILL.md");

    // Rebuild SKILL.md from parsed data so it's normalized
    let yaml = serde_yaml::to_string(&meta)
        .map_err(|e| format!("serialize yaml: {e}"))?;
    // serde_yaml may append trailing "..."
    let yaml = yaml.trim_end_matches("...\n").trim_end().to_string();

    let skill_content = format!("---\n{}---\n\n{}", yaml, body);

    std::fs::write(&skill_path, &skill_content)
        .map_err(|e| format!("write skill file: {e}"))?;

    Ok(SkillEntry {
        name: meta.name.clone(),
        description: meta.description.clone(),
        location: skill_path.to_string_lossy().to_string(),
        scope: SkillScope::User,
        body: Some(body),
    })
}

/// Preview a remote skill without installing it.
pub async fn preview_remote_skill(url: &str) -> Result<SkillEntry, String> {
    let remote = RemoteSkill {
        name: "(preview)".into(),
        description: String::new(),
        url: url.to_string(),
        source: SkillSource::Url(url.to_string()),
    };
    let (meta, body) = fetch_remote_skill_meta(&remote.url).await?;

    Ok(SkillEntry {
        name: meta.name.clone(),
        description: meta.description.clone(),
        location: url.to_string(),
        scope: SkillScope::User, // not really user scope, but placeholder
        body: Some(body),
    })
}

// ─── System Prompt Injection ──────────────────────────────────────────────────

/// Build the skills section to inject into the system prompt.
/// Returns None if no skills are available.
pub fn build_skills_system_prompt_section(catalog: &[SkillCatalogEntry]) -> Option<String> {
    if catalog.is_empty() {
        return None;
    }

    let mut xml = String::from(
        "\n\n<available_skills>\n"
    );

    for skill in catalog {
        xml.push_str("  <skill>\n");
        xml.push_str(&format!("    <name>{}</name>\n", escape_xml(&skill.name)));
        xml.push_str(&format!("    <description>{}</description>\n", escape_xml(&skill.description)));
        xml.push_str(&format!("    <location>{}</location>\n", escape_xml(&skill.location)));
        xml.push_str("  </skill>\n");
    }
    xml.push_str("</available_skills>\n\n");
    xml.push_str(
        "The following skills provide specialized instructions for specific tasks.\n\
         When a task matches a skill's description, use your read_file tool to load\n\
         the SKILL.md at the listed location before proceeding.\n\
         When a skill references relative paths, resolve them against the skill's\n\
         directory (the parent of SKILL.md) and use absolute paths in tool calls."
    );

    Some(xml)
}

fn escape_xml(s: &str) -> String {
    s.replace('&', "&amp;")
        .replace('<', "&lt;")
        .replace('>', "&gt;")
        .replace('"', "&quot;")
        .replace('\'', "&apos;")
}

#[cfg(test)]
mod tests {
    use super::*;

    fn sample_skill_md() -> &'static str {
        "---\nname: test-skill\ndescription: A test skill for testing\n---\n\n# Test Skill\n\nThis is a test."
    }

    #[test]
    fn test_parse_valid_skill() {
        let path = std::env::temp_dir().join("test_skill_valid.md");
        std::fs::write(&path, sample_skill_md()).unwrap();
        let (meta, body) = SkillManager::parse_skill_md(&path).unwrap();
        assert_eq!(meta.name, "test-skill");
        assert_eq!(meta.description, "A test skill for testing");
        assert_eq!(body, "# Test Skill\n\nThis is a test.");
        let _ = std::fs::remove_file(&path);
    }

    #[test]
    fn test_parse_from_str() {
        let path = Path::new("/virtual/path");
        let (meta, body) = SkillManager::parse_skill_md_from_str(sample_skill_md(), path).unwrap();
        assert_eq!(meta.name, "test-skill");
        assert_eq!(body, "# Test Skill\n\nThis is a test.");
    }

    #[test]
    fn test_parse_invalid_no_frontmatter() {
        let path = std::env::temp_dir().join("test_skill_no_fm.md");
        std::fs::write(&path, "# Just markdown").unwrap();
        assert!(SkillManager::parse_skill_md(&path).is_err());
        let _ = std::fs::remove_file(&path);
    }

    #[test]
    fn test_catalog_returns_vec() {
        // Catalog should always return a Vec (possibly empty)
        let mgr = SkillManager::new(None);
        let catalog = mgr.catalog();
        // It's a valid vector regardless of whether User has skills
        assert!(catalog.len() <= 500, "sanity: catalog shouldn't be huge");
        // If there are skills, they should be sorted
        for pair in catalog.windows(2) {
            assert!(pair[0].name <= pair[1].name, "catalog must be sorted by name");
        }
    }

    #[test]
    fn test_build_system_prompt_none() {
        let result = build_skills_system_prompt_section(&[]);
        assert!(result.is_none());
    }

    #[test]
    fn test_build_system_prompt_with_skills() {
        let catalog = vec![
            SkillCatalogEntry {
                name: "pdf".into(),
                description: "PDF processing".into(),
                location: "/home/user/.agents/skills/pdf/SKILL.md".into(),
                scope: SkillScope::User,
            },
        ];
        let result = build_skills_system_prompt_section(&catalog);
        assert!(result.is_some());
        let text = result.unwrap();
        assert!(text.contains("pdf"));
        assert!(text.contains("PDF processing"));
        assert!(text.contains("SKILL.md"));
    }

    #[test]
    fn test_parse_remote_index() {
        let index = "- [pdf-processing](https://raw.githubusercontent.com/vercel-labs/skills/refs/heads/main/skills/pdf-processing/SKILL.md): Extract PDF text, fill forms.\n\
                     - [csv-analyzer](https://raw.githubusercontent.com/vercel-labs/skills/refs/heads/main/skills/csv-analyzer/SKILL.md): Analyze CSV files.\n";
        let skills = parse_remote_index(index).unwrap();
        assert_eq!(skills.len(), 2);
        assert!(skills.iter().any(|s| s.name == "pdf-processing"));
        assert!(skills.iter().any(|s| s.name == "csv-analyzer"));
    }

    #[test]
    fn test_infer_github_source_from_raw_url() {
        let url = "https://raw.githubusercontent.com/vercel-labs/skills/refs/heads/main/skills/pdf-processing/SKILL.md";
        let source = infer_source_from_url(url);
        assert!(source.is_some());
        if let Some(SkillSource::GitHub { owner, repo, path }) = source {
            assert_eq!(owner, "vercel-labs");
            assert_eq!(repo, "skills");
            assert_eq!(path, "skills/pdf-processing/SKILL.md");
        } else {
            panic!("expected GitHub source");
        }
    }

    #[test]
    fn test_priority_project_over_user() {
        assert!(has_higher_priority(&SkillScope::Project, &SkillScope::User));
        assert!(!has_higher_priority(&SkillScope::User, &SkillScope::Project));
    }
}
