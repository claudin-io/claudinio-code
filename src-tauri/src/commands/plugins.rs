//! Tauri commands backing the Agent Plugins explorer: discover installed
//! plugins, install one from a local folder or a remote git/GitHub URL, toggle
//! it, uninstall it, and scaffold a new spec-compliant package (the crafter).

use crate::agent::plugins::{
    self, MCP_SCHEMA_ID, PLUGIN_SCHEMA_ID, PluginInfo, PluginRecord, PluginScope,
};
use crate::agent::provider::PluginPrefs;
use crate::procutil::no_window_tokio;
use crate::state::AppState;
use serde::{Deserialize, Serialize};
use std::path::{Path, PathBuf};
use tauri::State;
use tokio::process::Command;

// ─── Listing ──────────────────────────────────────────────────────────────────

fn record_enabled(config: &crate::agent::provider::AgentConfig, record: &PluginRecord) -> bool {
    plugins::is_record_enabled(record, &config.plugins)
}

/// Every installed plugin, valid or not, with the components it contributes.
#[tauri::command]
pub async fn plugins_list(
    workspace: Option<String>,
    state: State<'_, AppState>,
) -> Result<Vec<PluginInfo>, String> {
    let config = state.config.lock().await.clone();
    let root = workspace.as_deref().map(Path::new);
    let records = plugins::discover(root);
    Ok(records
        .iter()
        .map(|r| PluginInfo::from_record(r, record_enabled(&config, r)))
        .collect())
}

/// Load a plugin directory without installing it, so the UI can show the
/// manifest and any diagnostics before the user commits.
#[tauri::command]
pub async fn plugins_inspect(path: String) -> Result<PluginInfo, String> {
    let dir = PathBuf::from(&path);
    let record = match plugins::load_plugin(&dir, PluginScope::User) {
        Ok(plugin) => PluginRecord::Loaded(Box::new(plugin)),
        Err(error) => PluginRecord::Rejected {
            root: dir,
            scope: PluginScope::User,
            error,
        },
    };
    Ok(PluginInfo::from_record(&record, true))
}

// ─── Enable / disable ─────────────────────────────────────────────────────────

#[tauri::command]
pub async fn plugins_set_enabled(
    name: String,
    enabled: bool,
    workspace: Option<String>,
    state: State<'_, AppState>,
) -> Result<Vec<PluginInfo>, String> {
    {
        let mut config = state.config.lock().await;
        config.plugins.insert(name, PluginPrefs { enabled });
        crate::agent::provider::save_config(&config);
    }
    refresh_workspace(&workspace, &state).await;
    plugins_list(workspace, state).await
}

/// Re-scan skills so a toggled/installed plugin takes effect without a restart.
/// The MCP manager reconnects on its own: its fingerprint covers plugin servers.
async fn refresh_workspace(workspace: &Option<String>, state: &State<'_, AppState>) {
    if let Some(root) = workspace
        && let Ok(ws) = state.workspace(root).await
    {
        let mut mgr = ws.skills_manager.lock().await;
        mgr.scan();
    }
}

// ─── Install ──────────────────────────────────────────────────────────────────

#[derive(Debug, Clone, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct InstallScope {
    /// "user" (default) or "project".
    pub scope: Option<String>,
    pub workspace: Option<String>,
}

/// Where a newly installed plugin lands.
fn install_root(scope: &InstallScope) -> Result<PathBuf, String> {
    match scope.scope.as_deref() {
        Some("project") => {
            let ws = scope
                .workspace
                .as_deref()
                .ok_or("project scope needs an open workspace")?;
            Ok(PathBuf::from(ws).join(".claudinio").join("plugins"))
        }
        _ => plugins::user_install_dir().ok_or_else(|| "no home directory".to_string()),
    }
}

fn copy_dir_recursive(from: &Path, to: &Path) -> Result<(), String> {
    std::fs::create_dir_all(to).map_err(|e| format!("create {}: {e}", to.display()))?;
    for entry in std::fs::read_dir(from).map_err(|e| format!("read {}: {e}", from.display()))? {
        let entry = entry.map_err(|e| format!("read entry: {e}"))?;
        let name = entry.file_name();
        // Never copy VCS metadata into the installed package.
        if name == ".git" {
            continue;
        }
        let src = entry.path();
        let dst = to.join(&name);
        let file_type = entry
            .file_type()
            .map_err(|e| format!("stat {}: {e}", src.display()))?;
        if file_type.is_dir() {
            copy_dir_recursive(&src, &dst)?;
        } else {
            // Symlinks are resolved into plain files: an installed package must
            // not reach outside itself (§4.1).
            std::fs::copy(&src, &dst).map_err(|e| format!("copy {}: {e}", src.display()))?;
        }
    }
    Ok(())
}

/// Validate `source` as a plugin, then copy it into the install root under its
/// manifest name. Returns the freshly loaded plugin.
async fn install_from_dir(
    source: &Path,
    scope: &InstallScope,
    state: &State<'_, AppState>,
) -> Result<PluginInfo, String> {
    let loaded = plugins::load_plugin(source, PluginScope::User)?;
    let name = loaded.manifest.name.clone();

    let root = install_root(scope)?;
    std::fs::create_dir_all(&root).map_err(|e| format!("create {}: {e}", root.display()))?;
    let dest = root.join(&name);

    // Refuse to install onto a path that is not a plugin we manage.
    if dest.exists() {
        if !dest.join("plugin.json").exists() {
            return Err(format!(
                "{} already exists and is not a plugin directory",
                dest.display()
            ));
        }
        std::fs::remove_dir_all(&dest).map_err(|e| format!("replace {}: {e}", dest.display()))?;
    }
    copy_dir_recursive(source, &dest)?;

    let scope_kind = match scope.scope.as_deref() {
        Some("project") => PluginScope::Project,
        _ => PluginScope::User,
    };
    let installed = plugins::load_plugin(&dest, scope_kind)?;
    let record = PluginRecord::Loaded(Box::new(installed));

    let config = state.config.lock().await.clone();
    let enabled = record_enabled(&config, &record);
    refresh_workspace(&scope.workspace, state).await;
    Ok(PluginInfo::from_record(&record, enabled))
}

#[derive(Debug, Clone, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct InstallFromPathArgs {
    pub path: String,
    #[serde(flatten)]
    pub scope: InstallScope,
}

/// Install a plugin from a folder on disk.
#[tauri::command]
pub async fn plugins_install_from_path(
    args: InstallFromPathArgs,
    state: State<'_, AppState>,
) -> Result<PluginInfo, String> {
    let source = PathBuf::from(&args.path);
    if !source.is_dir() {
        return Err(format!("{} is not a directory", source.display()));
    }
    install_from_dir(&source, &args.scope, &state).await
}

#[derive(Debug, Clone, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct InstallFromUrlArgs {
    /// A git URL, or a GitHub repo/tree URL (`https://github.com/owner/repo`,
    /// optionally `/tree/<ref>/<subdir>`).
    pub url: String,
    /// Branch or tag override.
    pub git_ref: Option<String>,
    /// Directory inside the repository holding `plugin.json`.
    pub subdir: Option<String>,
    #[serde(flatten)]
    pub scope: InstallScope,
}

/// Parsed remote source: the repo to clone, plus where the plugin lives in it.
#[derive(Debug, Clone, PartialEq)]
pub struct RemoteSource {
    pub clone_url: String,
    pub git_ref: Option<String>,
    pub subdir: Option<String>,
}

/// Understand the URL forms a user is likely to paste. GitHub `tree` URLs carry
/// the ref and the subdirectory, so they install a plugin that lives inside a
/// monorepo without any extra input.
pub fn parse_remote_source(
    url: &str,
    git_ref: Option<&str>,
    subdir: Option<&str>,
) -> Result<RemoteSource, String> {
    let trimmed = url.trim().trim_end_matches('/');
    if trimmed.is_empty() {
        return Err("empty URL".into());
    }

    let mut source = if let Some(rest) = trimmed
        .strip_prefix("https://github.com/")
        .or_else(|| trimmed.strip_prefix("http://github.com/"))
    {
        let parts: Vec<&str> = rest.split('/').collect();
        if parts.len() < 2 {
            return Err(format!("'{url}' is not a GitHub repository URL"));
        }
        let owner = parts[0];
        let repo = parts[1].trim_end_matches(".git");
        let clone_url = format!("https://github.com/{owner}/{repo}.git");
        // .../tree/<ref>/<subdir...>
        if parts.len() > 3 && (parts[2] == "tree" || parts[2] == "blob") {
            let git_ref = parts[3].to_string();
            let subdir = if parts.len() > 4 {
                Some(parts[4..].join("/"))
            } else {
                None
            };
            RemoteSource {
                clone_url,
                git_ref: Some(git_ref),
                subdir,
            }
        } else {
            RemoteSource {
                clone_url,
                git_ref: None,
                subdir: None,
            }
        }
    } else if trimmed.starts_with("https://")
        || trimmed.starts_with("http://")
        || trimmed.starts_with("git@")
        || trimmed.starts_with("ssh://")
        || trimmed.starts_with("git://")
    {
        RemoteSource {
            clone_url: trimmed.to_string(),
            git_ref: None,
            subdir: None,
        }
    } else {
        return Err(format!(
            "'{url}' is not a supported plugin URL (use a git or GitHub URL)"
        ));
    };

    // Explicit arguments win over anything inferred from the URL.
    if let Some(r) = git_ref.filter(|s| !s.trim().is_empty()) {
        source.git_ref = Some(r.trim().to_string());
    }
    if let Some(s) = subdir.filter(|s| !s.trim().is_empty()) {
        source.subdir = Some(s.trim().trim_matches('/').to_string());
    }
    Ok(source)
}

/// Install a plugin from a remote git repository (GitHub URLs included).
#[tauri::command]
pub async fn plugins_install_from_url(
    args: InstallFromUrlArgs,
    state: State<'_, AppState>,
) -> Result<PluginInfo, String> {
    let source = parse_remote_source(&args.url, args.git_ref.as_deref(), args.subdir.as_deref())?;

    let temp = std::env::temp_dir().join(format!("claudinio-plugin-{}", uuid::Uuid::new_v4()));
    std::fs::create_dir_all(&temp).map_err(|e| format!("create temp dir: {e}"))?;

    let result = clone_and_install(&source, &temp, &args.scope, &state).await;
    let _ = std::fs::remove_dir_all(&temp);
    result
}

async fn clone_and_install(
    source: &RemoteSource,
    temp: &Path,
    scope: &InstallScope,
    state: &State<'_, AppState>,
) -> Result<PluginInfo, String> {
    let checkout = temp.join("repo");
    let _net_guard = crate::net_activity::NetGuard::begin(
        crate::net_activity::NetSource::SkillFetch,
        format!("git clone {}", source.clone_url),
    );

    let mut cmd = Command::new("git");
    cmd.arg("clone").arg("--depth").arg("1");
    if let Some(r) = &source.git_ref {
        cmd.arg("--branch").arg(r);
    }
    cmd.arg(&source.clone_url).arg(&checkout);
    // Never block on credential prompts inside a GUI app.
    cmd.env("GIT_TERMINAL_PROMPT", "0");
    no_window_tokio(&mut cmd);

    let output = cmd
        .output()
        .await
        .map_err(|e| format!("failed to run git: {e}"))?;
    if !output.status.success() {
        return Err(format!(
            "git clone failed: {}",
            String::from_utf8_lossy(&output.stderr).trim()
        ));
    }

    let plugin_dir = match &source.subdir {
        Some(sub) => {
            let joined = checkout.join(sub);
            let normalized = joined
                .canonicalize()
                .map_err(|e| format!("{}: {e}", joined.display()))?;
            let base = checkout
                .canonicalize()
                .map_err(|e| format!("{}: {e}", checkout.display()))?;
            if !normalized.starts_with(&base) {
                return Err("subdirectory escapes the cloned repository".into());
            }
            normalized
        }
        None => checkout.clone(),
    };

    if !plugin_dir.join("plugin.json").exists() {
        return Err(
            "no plugin.json found in the repository — point the URL at the plugin directory"
                .to_string(),
        );
    }

    install_from_dir(&plugin_dir, scope, state).await
}

// ─── Uninstall ────────────────────────────────────────────────────────────────

#[tauri::command]
pub async fn plugins_uninstall(
    name: String,
    workspace: Option<String>,
    state: State<'_, AppState>,
) -> Result<Vec<PluginInfo>, String> {
    let root = workspace.as_deref().map(Path::new);
    let records = plugins::discover(root);
    let target = records
        .iter()
        .find(|r| r.name() == name)
        .ok_or_else(|| format!("plugin '{name}' is not installed"))?;

    let dir = target.root().to_path_buf();
    if !dir.join("plugin.json").exists() {
        return Err(format!("{} is not a plugin directory", dir.display()));
    }
    std::fs::remove_dir_all(&dir).map_err(|e| format!("remove {}: {e}", dir.display()))?;

    {
        let mut config = state.config.lock().await;
        config.plugins.remove(&name);
        crate::agent::provider::save_config(&config);
    }
    refresh_workspace(&workspace, &state).await;
    plugins_list(workspace, state).await
}

// ─── Crafter: scaffold a new plugin ───────────────────────────────────────────

#[derive(Debug, Clone, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct ScaffoldSkill {
    pub name: String,
    pub description: String,
}

#[derive(Debug, Clone, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct ScaffoldMcpServer {
    pub name: String,
    /// "stdio" or "streamable-http".
    pub transport: String,
    /// stdio: the executable token. remote: unused.
    pub command: Option<String>,
    #[serde(default)]
    pub args: Vec<String>,
    /// remote: the endpoint URL. stdio: unused.
    pub url: Option<String>,
}

#[derive(Debug, Clone, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct ScaffoldArgs {
    pub name: String,
    pub description: Option<String>,
    pub version: Option<String>,
    pub author_name: Option<String>,
    pub author_email: Option<String>,
    pub author_url: Option<String>,
    pub homepage: Option<String>,
    pub repository: Option<String>,
    pub license: Option<String>,
    #[serde(default)]
    pub keywords: Vec<String>,
    #[serde(default)]
    pub skills: Vec<ScaffoldSkill>,
    #[serde(default)]
    pub mcp_servers: Vec<ScaffoldMcpServer>,
    /// Where to write it. Defaults to the install root for `scope`.
    pub dest: Option<String>,
    #[serde(flatten)]
    pub scope: InstallScope,
}

#[derive(Debug, Clone, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct ScaffoldResult {
    pub root: String,
    pub files: Vec<String>,
    pub plugin: PluginInfo,
}

/// Build the `plugin.json` document for a scaffold. Pure so it can be tested
/// without touching the filesystem.
pub fn build_manifest_json(args: &ScaffoldArgs) -> serde_json::Value {
    let mut manifest = serde_json::Map::new();
    manifest.insert("$schema".into(), PLUGIN_SCHEMA_ID.into());
    manifest.insert("name".into(), args.name.clone().into());
    manifest.insert(
        "version".into(),
        args.version
            .clone()
            .unwrap_or_else(|| "0.1.0".into())
            .into(),
    );
    if let Some(d) = args.description.clone().filter(|s| !s.is_empty()) {
        manifest.insert("description".into(), d.into());
    }
    let mut author = serde_json::Map::new();
    for (key, value) in [
        ("name", &args.author_name),
        ("email", &args.author_email),
        ("url", &args.author_url),
    ] {
        if let Some(v) = value.clone().filter(|s| !s.is_empty()) {
            author.insert(key.into(), v.into());
        }
    }
    if !author.is_empty() {
        manifest.insert("author".into(), serde_json::Value::Object(author));
    }
    for (key, value) in [
        ("homepage", &args.homepage),
        ("repository", &args.repository),
        ("license", &args.license),
    ] {
        if let Some(v) = value.clone().filter(|s| !s.is_empty()) {
            manifest.insert(key.into(), v.into());
        }
    }
    if !args.keywords.is_empty() {
        manifest.insert("keywords".into(), args.keywords.clone().into());
    }
    serde_json::Value::Object(manifest)
}

/// Build `mcp.json` for a scaffold, or `None` when no servers were requested.
pub fn build_mcp_json(args: &ScaffoldArgs) -> Result<Option<serde_json::Value>, String> {
    if args.mcp_servers.is_empty() {
        return Ok(None);
    }
    let mut servers = serde_json::Map::new();
    for server in &args.mcp_servers {
        let entry = match server.transport.as_str() {
            "stdio" => {
                let command = server
                    .command
                    .clone()
                    .filter(|s| !s.is_empty())
                    .ok_or_else(|| format!("MCP server '{}' needs a command", server.name))?;
                let mut obj = serde_json::Map::new();
                obj.insert("type".into(), "stdio".into());
                obj.insert("command".into(), command.into());
                if !server.args.is_empty() {
                    obj.insert("args".into(), server.args.clone().into());
                }
                obj.insert(
                    "cwd".into(),
                    serde_json::Value::String("${PLUGIN_ROOT}".into()),
                );
                serde_json::Value::Object(obj)
            }
            "streamable-http" => {
                let url = server
                    .url
                    .clone()
                    .filter(|s| !s.is_empty())
                    .ok_or_else(|| format!("MCP server '{}' needs a url", server.name))?;
                serde_json::json!({ "type": "streamable-http", "url": url })
            }
            other => return Err(format!("unsupported transport '{other}'")),
        };
        servers.insert(server.name.clone(), entry);
    }
    Ok(Some(serde_json::json!({
        "$schema": MCP_SCHEMA_ID,
        "mcpServers": servers,
    })))
}

fn skill_template(name: &str, description: &str) -> String {
    format!(
        "---\nname: {name}\ndescription: {description}\n---\n\n\
         # {name}\n\n\
         Describe when this skill applies and what the agent should do, step by step.\n\n\
         ## When to use\n\n\
         - Trigger conditions that match the description above.\n\n\
         ## Steps\n\n\
         1. First action.\n2. Second action.\n\n\
         ## Notes\n\n\
         Bundle helper scripts under `scripts/` and long reference material under\n\
         `references/`, and refer to them by relative path.\n"
    )
}

/// Create a spec-compliant plugin package on disk and load it back.
#[tauri::command]
pub async fn plugins_scaffold(
    args: ScaffoldArgs,
    state: State<'_, AppState>,
) -> Result<ScaffoldResult, String> {
    plugins::validate_plugin_name(&args.name)?;

    let base = match args.dest.clone().filter(|s| !s.is_empty()) {
        Some(dest) => PathBuf::from(dest),
        None => install_root(&args.scope)?,
    };
    let root = base.join(&args.name);
    if root.exists() {
        return Err(format!("{} already exists", root.display()));
    }
    std::fs::create_dir_all(&root).map_err(|e| format!("create {}: {e}", root.display()))?;

    let mut files = Vec::new();

    let manifest = build_manifest_json(&args);
    let manifest_text = format!(
        "{}\n",
        serde_json::to_string_pretty(&manifest).map_err(|e| e.to_string())?
    );
    std::fs::write(root.join("plugin.json"), manifest_text)
        .map_err(|e| format!("write plugin.json: {e}"))?;
    files.push("plugin.json".to_string());

    for skill in &args.skills {
        let dir = root.join("skills").join(&skill.name);
        std::fs::create_dir_all(dir.join("references"))
            .map_err(|e| format!("create {}: {e}", dir.display()))?;
        std::fs::create_dir_all(dir.join("scripts"))
            .map_err(|e| format!("create {}: {e}", dir.display()))?;
        std::fs::write(
            dir.join("SKILL.md"),
            skill_template(&skill.name, &skill.description),
        )
        .map_err(|e| format!("write SKILL.md: {e}"))?;
        files.push(format!("skills/{}/SKILL.md", skill.name));
    }

    if let Some(mcp) = build_mcp_json(&args)? {
        let text = format!(
            "{}\n",
            serde_json::to_string_pretty(&mcp).map_err(|e| e.to_string())?
        );
        std::fs::write(root.join("mcp.json"), text).map_err(|e| format!("write mcp.json: {e}"))?;
        files.push("mcp.json".to_string());
    }

    let readme = format!(
        "# {name}\n\n{description}\n\n\
         An [Agent Plugin](https://agent-plugins.org) — a portable package of\n\
         skills and MCP servers.\n\n\
         ## Layout\n\n\
         ```\n{name}/\n├── plugin.json\n├── skills/\n└── mcp.json\n```\n\n\
         ## Install\n\n\
         Copy this directory into `~/.claudinio/plugins/`, or install it from the\n\
         Plugins explorer by folder or by git URL.\n",
        name = args.name,
        description = args
            .description
            .clone()
            .unwrap_or_else(|| "A portable Agent Plugin.".into()),
    );
    std::fs::write(root.join("README.md"), readme).map_err(|e| format!("write README.md: {e}"))?;
    files.push("README.md".to_string());

    let scope_kind = match args.scope.scope.as_deref() {
        Some("project") => PluginScope::Project,
        _ => PluginScope::User,
    };
    let loaded = plugins::load_plugin(&root, scope_kind)?;
    let record = PluginRecord::Loaded(Box::new(loaded));
    let config = state.config.lock().await.clone();
    let enabled = record_enabled(&config, &record);
    refresh_workspace(&args.scope.workspace, &state).await;

    Ok(ScaffoldResult {
        root: root.to_string_lossy().to_string(),
        files,
        plugin: PluginInfo::from_record(&record, enabled),
    })
}

#[cfg(test)]
mod tests {
    use super::*;

    fn scaffold_args(name: &str) -> ScaffoldArgs {
        ScaffoldArgs {
            name: name.to_string(),
            description: Some("Does a thing".into()),
            version: None,
            author_name: Some("Ada".into()),
            author_email: None,
            author_url: None,
            homepage: None,
            repository: None,
            license: Some("MIT".into()),
            keywords: vec!["demo".into()],
            skills: vec![],
            mcp_servers: vec![],
            dest: None,
            scope: InstallScope {
                scope: None,
                workspace: None,
            },
        }
    }

    #[test]
    fn parses_a_plain_github_url() {
        let source = parse_remote_source("https://github.com/acme/tools", None, None).unwrap();
        assert_eq!(source.clone_url, "https://github.com/acme/tools.git");
        assert_eq!(source.git_ref, None);
        assert_eq!(source.subdir, None);
    }

    #[test]
    fn parses_a_github_tree_url_with_ref_and_subdir() {
        let source = parse_remote_source(
            "https://github.com/acme/tools/tree/main/plugins/deploy",
            None,
            None,
        )
        .unwrap();
        assert_eq!(source.clone_url, "https://github.com/acme/tools.git");
        assert_eq!(source.git_ref.as_deref(), Some("main"));
        assert_eq!(source.subdir.as_deref(), Some("plugins/deploy"));
    }

    #[test]
    fn explicit_ref_and_subdir_win_over_the_url() {
        let source = parse_remote_source(
            "https://github.com/acme/tools/tree/main/plugins/deploy",
            Some("v2"),
            Some("/other/path/"),
        )
        .unwrap();
        assert_eq!(source.git_ref.as_deref(), Some("v2"));
        assert_eq!(source.subdir.as_deref(), Some("other/path"));
    }

    #[test]
    fn accepts_generic_git_urls_and_rejects_junk() {
        assert!(parse_remote_source("git@github.com:acme/tools.git", None, None).is_ok());
        assert!(parse_remote_source("https://gitlab.com/acme/tools.git", None, None).is_ok());
        assert!(parse_remote_source("acme/tools", None, None).is_err());
        assert!(parse_remote_source("", None, None).is_err());
    }

    #[test]
    fn scaffolded_manifest_matches_the_spec() {
        let manifest = build_manifest_json(&scaffold_args("demo-plugin"));
        assert_eq!(manifest["$schema"], PLUGIN_SCHEMA_ID);
        assert_eq!(manifest["name"], "demo-plugin");
        assert_eq!(manifest["version"], "0.1.0");
        assert_eq!(manifest["author"]["name"], "Ada");
        assert!(manifest.get("homepage").is_none());
        // The generated manifest must survive our own loader.
        let text = serde_json::to_string(&manifest).unwrap();
        assert!(plugins::parse_manifest(&text).is_ok());
    }

    #[test]
    fn scaffolded_mcp_config_is_valid_or_absent() {
        let mut args = scaffold_args("demo-plugin");
        assert!(build_mcp_json(&args).unwrap().is_none());

        args.mcp_servers = vec![ScaffoldMcpServer {
            name: "local".into(),
            transport: "stdio".into(),
            command: Some("npx".into()),
            args: vec!["-y".into(), "server".into()],
            url: None,
        }];
        let mcp = build_mcp_json(&args).unwrap().unwrap();
        assert_eq!(mcp["$schema"], MCP_SCHEMA_ID);
        assert_eq!(mcp["mcpServers"]["local"]["command"], "npx");
        assert_eq!(mcp["mcpServers"]["local"]["cwd"], "${PLUGIN_ROOT}");
    }

    #[test]
    fn scaffolded_mcp_config_requires_transport_fields() {
        let mut args = scaffold_args("demo-plugin");
        args.mcp_servers = vec![ScaffoldMcpServer {
            name: "broken".into(),
            transport: "stdio".into(),
            command: None,
            args: vec![],
            url: None,
        }];
        assert!(build_mcp_json(&args).is_err());
    }

    #[test]
    fn skill_template_parses_as_a_skill() {
        let text = skill_template("deploy", "Deploy the service to staging");
        let parsed = crate::agent::skills::SkillManager::parse_skill_md_from_str(
            &text,
            std::path::Path::new("SKILL.md"),
        );
        assert!(parsed.is_ok());
    }
}
