//! Agent Plugins v1.0.0 client (https://agent-plugins.org/specification).
//!
//! A plugin is a directory with a required `plugin.json` manifest and two
//! optional component locations: `skills/` (Agent Skills) and `mcp.json` (MCP
//! servers). This module loads, validates and maps those components onto the
//! app's native skill catalog and MCP configuration.
//!
//! Failure boundaries follow §11.3: a bad manifest rejects the whole plugin,
//! but a bad skill or a bad MCP server entry only skips that entry — every
//! other component still loads, and the reason lands in `diagnostics`.

use crate::agent::provider::{McpServerEntry, McpTransportConfig};
use crate::agent::skills::{SkillEntry, SkillManager, SkillScope};
use serde::{Deserialize, Serialize};
use std::collections::HashMap;
use std::path::{Component, Path, PathBuf};

// ─── Spec constants ───────────────────────────────────────────────────────────

/// Canonical `$schema` identifier for `plugin.json` (§5.2).
pub const PLUGIN_SCHEMA_ID: &str = "https://agent-plugins.org/schemas/1.0.0/plugin.schema.json";
/// Canonical `$schema` identifier for `mcp.json` (§7.2.1).
pub const MCP_SCHEMA_ID: &str = "https://agent-plugins.org/schemas/1.0.0/mcp.schema.json";
/// Agent Plugins version this client implements.
pub const SPEC_VERSION: &str = "1.0.0";
/// Our reverse-domain client extension namespace (§8).
pub const EXTENSION_NAMESPACE: &str = "io.claudin.claudinio";

/// Directories scanned for installed plugins, under each scope root. Mirrors
/// the skill directory convention: `<root>/<dir>/plugins/<plugin>/`.
pub const PLUGIN_DIR_NAMES: &[&str] = &[".agents", ".claudinio", ".claude"];

// ─── Data types ───────────────────────────────────────────────────────────────

#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub enum PluginScope {
    Project,
    User,
}

#[derive(Debug, Clone, Default, PartialEq, Serialize, Deserialize)]
pub struct PluginAuthor {
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub name: Option<String>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub email: Option<String>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub url: Option<String>,
}

#[derive(Debug, Clone, Copy, PartialEq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub enum Severity {
    Error,
    Warning,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct PluginDiagnostic {
    pub severity: Severity,
    pub message: String,
}

impl PluginDiagnostic {
    fn error(message: impl Into<String>) -> Self {
        Self {
            severity: Severity::Error,
            message: message.into(),
        }
    }
    fn warning(message: impl Into<String>) -> Self {
        Self {
            severity: Severity::Warning,
            message: message.into(),
        }
    }
}

/// The portable manifest, after validation (§5.2).
#[derive(Debug, Clone, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct PluginManifest {
    pub name: String,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub version: Option<String>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub description: Option<String>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub author: Option<PluginAuthor>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub homepage: Option<String>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub repository: Option<String>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub license: Option<String>,
    #[serde(default)]
    pub keywords: Vec<String>,
    /// Client extension data keyed by reverse-domain namespace (§8.1).
    #[serde(default)]
    pub extensions: serde_json::Map<String, serde_json::Value>,
    /// Lifecycle hooks: an inline `hooks` object, a path to a hooks file, or a
    /// list of such paths, all relative to the plugin root. A plugin that omits
    /// this is still searched for the conventional `hooks/hooks.json` — most
    /// published plugins declare nothing and rely on that convention. See
    /// `crate::agent::hooks::discovery`.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub hooks: Option<serde_json::Value>,
}

impl PluginManifest {
    /// Our own slice of `extensions` (§8.1). Other namespaces are ignored
    /// without validating their contents, as the spec requires.
    pub fn client_extension(&self) -> Option<&serde_json::Map<String, serde_json::Value>> {
        self.extensions.get(EXTENSION_NAMESPACE)?.as_object()
    }

    /// `extensions["io.claudin.claudinio"].enabledByDefault` — lets a plugin
    /// ship switched off, so installing it is not the same as trusting it to
    /// run. Anything other than an explicit `false` means enabled.
    pub fn enabled_by_default(&self) -> bool {
        self.client_extension()
            .and_then(|ext| ext.get("enabledByDefault"))
            .and_then(|v| v.as_bool())
            .unwrap_or(true)
    }
}

/// One MCP server contributed by a plugin, already validated and expanded.
#[derive(Debug, Clone)]
pub struct PluginMcpServer {
    /// Name as declared in `mcp.json`.
    pub local_name: String,
    /// Name used in the app's MCP map: `<plugin>.<server>`.
    pub qualified_name: String,
    pub entry: McpServerEntry,
}

/// A plugin that passed manifest validation, with whatever components loaded.
#[derive(Debug, Clone)]
pub struct LoadedPlugin {
    pub manifest: PluginManifest,
    pub root: PathBuf,
    pub data_dir: PathBuf,
    pub scope: PluginScope,
    pub skills: Vec<SkillEntry>,
    pub mcp_servers: Vec<PluginMcpServer>,
    pub diagnostics: Vec<PluginDiagnostic>,
}

/// A discovered plugin directory: either loaded, or rejected with a reason.
#[derive(Debug, Clone)]
pub enum PluginRecord {
    Loaded(Box<LoadedPlugin>),
    Rejected {
        root: PathBuf,
        scope: PluginScope,
        error: String,
    },
}

impl PluginRecord {
    pub fn root(&self) -> &Path {
        match self {
            PluginRecord::Loaded(p) => &p.root,
            PluginRecord::Rejected { root, .. } => root,
        }
    }
    pub fn name(&self) -> String {
        match self {
            PluginRecord::Loaded(p) => p.manifest.name.clone(),
            PluginRecord::Rejected { root, .. } => root
                .file_name()
                .and_then(|n| n.to_str())
                .unwrap_or("(unknown)")
                .to_string(),
        }
    }
}

// ─── UI-facing summaries ──────────────────────────────────────────────────────

#[derive(Debug, Clone, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct PluginSkillInfo {
    pub name: String,
    pub description: String,
    pub location: String,
}

#[derive(Debug, Clone, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct PluginMcpInfo {
    pub name: String,
    pub qualified_name: String,
    pub transport: String,
}

#[derive(Debug, Clone, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct PluginInfo {
    pub name: String,
    pub root: String,
    pub scope: PluginScope,
    pub enabled: bool,
    /// False when the manifest itself was rejected (§11.3).
    pub valid: bool,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub version: Option<String>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub description: Option<String>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub author: Option<PluginAuthor>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub homepage: Option<String>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub repository: Option<String>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub license: Option<String>,
    pub keywords: Vec<String>,
    pub skills: Vec<PluginSkillInfo>,
    pub mcp_servers: Vec<PluginMcpInfo>,
    pub diagnostics: Vec<PluginDiagnostic>,
}

impl PluginInfo {
    pub fn from_record(record: &PluginRecord, enabled: bool) -> Self {
        match record {
            PluginRecord::Loaded(p) => PluginInfo {
                name: p.manifest.name.clone(),
                root: p.root.to_string_lossy().to_string(),
                scope: p.scope.clone(),
                enabled,
                valid: true,
                version: p.manifest.version.clone(),
                description: p.manifest.description.clone(),
                author: p.manifest.author.clone(),
                homepage: p.manifest.homepage.clone(),
                repository: p.manifest.repository.clone(),
                license: p.manifest.license.clone(),
                keywords: p.manifest.keywords.clone(),
                skills: p
                    .skills
                    .iter()
                    .map(|s| PluginSkillInfo {
                        name: s.name.clone(),
                        description: s.description.clone(),
                        location: s.location.clone(),
                    })
                    .collect(),
                mcp_servers: p
                    .mcp_servers
                    .iter()
                    .map(|s| PluginMcpInfo {
                        name: s.local_name.clone(),
                        qualified_name: s.qualified_name.clone(),
                        transport: match s.entry.transport {
                            McpTransportConfig::Stdio { .. } => "stdio".into(),
                            McpTransportConfig::Remote { .. } => "streamable-http".into(),
                        },
                    })
                    .collect(),
                diagnostics: p.diagnostics.clone(),
            },
            PluginRecord::Rejected { root, scope, error } => PluginInfo {
                name: record.name(),
                root: root.to_string_lossy().to_string(),
                scope: scope.clone(),
                enabled,
                valid: false,
                version: None,
                description: None,
                author: None,
                homepage: None,
                repository: None,
                license: None,
                keywords: Vec::new(),
                skills: Vec::new(),
                mcp_servers: Vec::new(),
                diagnostics: vec![PluginDiagnostic::error(error.clone())],
            },
        }
    }
}

// ─── Path containment (§4.1) ──────────────────────────────────────────────────

/// Lexically normalize a path (resolve `.` and `..` textually, no filesystem
/// access). Used before canonicalization so non-existent paths can still be
/// checked for escapes.
fn normalize_lexically(path: &Path) -> PathBuf {
    let mut out = PathBuf::new();
    for comp in path.components() {
        match comp {
            Component::ParentDir => {
                out.pop();
            }
            Component::CurDir => {}
            other => out.push(other.as_os_str()),
        }
    }
    out
}

/// Resolve `candidate` and assert it stays inside `root` (§4.1.3). Symlinks are
/// followed when the path exists, so a link pointing outside is rejected.
fn resolve_contained(root: &Path, candidate: &Path) -> Result<PathBuf, String> {
    let joined = if candidate.is_absolute() {
        candidate.to_path_buf()
    } else {
        root.join(candidate)
    };
    let lexical = normalize_lexically(&joined);
    if !lexical.starts_with(root) {
        return Err(format!(
            "path '{}' resolves outside the plugin root",
            candidate.display()
        ));
    }
    if let Ok(real) = std::fs::canonicalize(&lexical) {
        let real_root = std::fs::canonicalize(root).unwrap_or_else(|_| root.to_path_buf());
        if !real.starts_with(&real_root) {
            return Err(format!(
                "path '{}' resolves outside the plugin root",
                candidate.display()
            ));
        }
        return Ok(real);
    }
    Ok(lexical)
}

/// A plugin-relative path per §4.1.4: must begin with `./` and stay inside.
fn resolve_plugin_relative(root: &Path, value: &str) -> Result<PathBuf, String> {
    if !value.starts_with("./") {
        return Err(format!(
            "plugin-relative path '{value}' must begin with './'"
        ));
    }
    resolve_contained(root, Path::new(&value[2..]))
}

// ─── Placeholder expansion (§9.2) ─────────────────────────────────────────────

/// Single, non-recursive replacement of `${PLUGIN_ROOT}` and `${PLUGIN_DATA}`.
/// Text introduced by a replacement is never rescanned.
pub fn expand_placeholders(input: &str, plugin_root: &str, plugin_data: &str) -> String {
    const ROOT: &str = "${PLUGIN_ROOT}";
    const DATA: &str = "${PLUGIN_DATA}";
    let mut out = String::with_capacity(input.len());
    let mut rest = input;
    loop {
        let root_at = rest.find(ROOT);
        let data_at = rest.find(DATA);
        let (at, len, value) = match (root_at, data_at) {
            (Some(r), Some(d)) if r <= d => (r, ROOT.len(), plugin_root),
            (Some(_), Some(d)) => (d, DATA.len(), plugin_data),
            (Some(r), None) => (r, ROOT.len(), plugin_root),
            (None, Some(d)) => (d, DATA.len(), plugin_data),
            (None, None) => break,
        };
        out.push_str(&rest[..at]);
        out.push_str(value);
        rest = &rest[at + len..];
    }
    out.push_str(rest);
    out
}

// ─── Manifest validation (§5) ─────────────────────────────────────────────────

const MANIFEST_FIELDS: &[&str] = &[
    "$schema",
    "name",
    "version",
    "description",
    "author",
    "homepage",
    "repository",
    "license",
    "keywords",
    "extensions",
    // Not in the Agent Plugins 1.0.0 schema. Recognised anyway because the
    // ecosystem this client has to interoperate with declares lifecycle hooks
    // here, and reporting a real, load-bearing field as "unknown" would be
    // telling the truth about the spec and a lie about the plugin.
    "hooks",
];

/// §5.5 plugin name constraints.
pub fn validate_plugin_name(name: &str) -> Result<(), String> {
    if name.is_empty() || name.len() > 64 {
        return Err("plugin name must be 1-64 characters".into());
    }
    if !name
        .chars()
        .all(|c| c.is_ascii_lowercase() || c.is_ascii_digit() || c == '-' || c == '.')
    {
        return Err("plugin name may only contain lowercase letters, digits, '-' and '.'".into());
    }
    let first = name.chars().next().unwrap();
    let last = name.chars().next_back().unwrap();
    if !first.is_ascii_alphanumeric() || !last.is_ascii_alphanumeric() {
        return Err("plugin name must start and end with an alphanumeric character".into());
    }
    if name.contains("--") || name.contains("..") {
        return Err("plugin name must not contain '--' or '..'".into());
    }
    Ok(())
}

fn opt_string(
    obj: &serde_json::Map<String, serde_json::Value>,
    key: &str,
) -> Result<Option<String>, String> {
    match obj.get(key) {
        None => Ok(None),
        Some(serde_json::Value::String(s)) => Ok(Some(s.clone())),
        Some(_) => Err(format!("'{key}' must be a string")),
    }
}

/// Parse and validate `plugin.json`. Returns the manifest plus non-fatal
/// diagnostics (unknown fields, non-object `extensions`).
pub fn parse_manifest(text: &str) -> Result<(PluginManifest, Vec<PluginDiagnostic>), String> {
    let value: serde_json::Value =
        serde_json::from_str(text).map_err(|e| format!("plugin.json is not valid JSON: {e}"))?;
    let obj = value
        .as_object()
        .ok_or_else(|| "plugin.json must contain a top-level object".to_string())?;

    let mut diagnostics = Vec::new();

    // §5.2 — unknown top-level fields are reported and ignored.
    for key in obj.keys() {
        if !MANIFEST_FIELDS.contains(&key.as_str()) {
            diagnostics.push(PluginDiagnostic::warning(format!(
                "unknown plugin.json field '{key}' ignored"
            )));
        }
    }

    let schema = obj
        .get("$schema")
        .and_then(|v| v.as_str())
        .ok_or_else(|| "plugin.json is missing the required '$schema' string".to_string())?;
    if schema != PLUGIN_SCHEMA_ID {
        return Err(format!(
            "unsupported Agent Plugins version: this client implements {SPEC_VERSION}, \
             so '$schema' must be '{PLUGIN_SCHEMA_ID}' (got '{schema}')"
        ));
    }

    let name = obj
        .get("name")
        .and_then(|v| v.as_str())
        .ok_or_else(|| "plugin.json is missing the required 'name' string".to_string())?
        .to_string();
    validate_plugin_name(&name)?;

    let version = opt_string(obj, "version")?;
    let description = opt_string(obj, "description")?;
    let homepage = opt_string(obj, "homepage")?;
    let repository = opt_string(obj, "repository")?;
    let license = opt_string(obj, "license")?;

    let author = match obj.get("author") {
        None => None,
        Some(serde_json::Value::Object(a)) => {
            for (k, v) in a {
                if !["name", "email", "url"].contains(&k.as_str()) {
                    return Err(format!("'author.{k}' is not a permitted author field"));
                }
                if !v.is_string() {
                    return Err(format!("'author.{k}' must be a string"));
                }
            }
            Some(PluginAuthor {
                name: a.get("name").and_then(|v| v.as_str()).map(String::from),
                email: a.get("email").and_then(|v| v.as_str()).map(String::from),
                url: a.get("url").and_then(|v| v.as_str()).map(String::from),
            })
        }
        Some(_) => return Err("'author' must be an object".into()),
    };

    let keywords = match obj.get("keywords") {
        None => Vec::new(),
        Some(serde_json::Value::Array(items)) => {
            let mut out = Vec::with_capacity(items.len());
            for item in items {
                match item.as_str() {
                    Some(s) => out.push(s.to_string()),
                    None => return Err("'keywords' must be an array of strings".into()),
                }
            }
            out
        }
        Some(_) => return Err("'keywords' must be an array of strings".into()),
    };

    // §8.1 — a non-object `extensions` is reported and ignored, not fatal.
    let extensions = match obj.get("extensions") {
        None => serde_json::Map::new(),
        Some(serde_json::Value::Object(map)) => {
            let mut kept = serde_json::Map::new();
            for (ns, v) in map {
                if v.is_object() {
                    kept.insert(ns.clone(), v.clone());
                } else {
                    diagnostics.push(PluginDiagnostic::warning(format!(
                        "extensions['{ns}'] must be an object; ignored"
                    )));
                }
            }
            kept
        }
        Some(_) => {
            diagnostics.push(PluginDiagnostic::warning(
                "'extensions' must be an object; ignored".to_string(),
            ));
            serde_json::Map::new()
        }
    };

    Ok((
        PluginManifest {
            name,
            version,
            description,
            author,
            homepage,
            repository,
            license,
            keywords,
            extensions,
            hooks: obj.get("hooks").cloned(),
        },
        diagnostics,
    ))
}

// ─── MCP configuration (§7.2) ─────────────────────────────────────────────────

const STDIO_FIELDS: &[&str] = &["type", "command", "args", "env", "cwd"];
const REMOTE_FIELDS: &[&str] = &["type", "url", "headers"];

fn string_map(value: &serde_json::Value, field: &str) -> Result<Vec<(String, String)>, String> {
    let obj = value
        .as_object()
        .ok_or_else(|| format!("'{field}' must be an object of strings"))?;
    let mut out = Vec::with_capacity(obj.len());
    for (k, v) in obj {
        let s = v
            .as_str()
            .ok_or_else(|| format!("'{field}.{k}' must be a string"))?;
        out.push((k.clone(), s.to_string()));
    }
    Ok(out)
}

/// True for `localhost` and loopback IP literals (§7.2.1).
fn is_loopback_host(host: &str) -> bool {
    let host = host.trim_start_matches('[').trim_end_matches(']');
    if host.eq_ignore_ascii_case("localhost") {
        return true;
    }
    if host == "::1" {
        return true;
    }
    if let Ok(ip) = host.parse::<std::net::IpAddr>() {
        return ip.is_loopback();
    }
    false
}

fn validate_remote_url(url: &str) -> Result<(), String> {
    let (scheme, rest) = if let Some(rest) = url.strip_prefix("https://") {
        ("https", rest)
    } else if let Some(rest) = url.strip_prefix("http://") {
        ("http", rest)
    } else {
        return Err(format!("'url' must be an absolute http(s) URL: {url}"));
    };
    if url.contains('#') {
        return Err("'url' must not contain a fragment".into());
    }
    let authority_end = rest.find(['/', '?']).unwrap_or(rest.len());
    let authority = &rest[..authority_end];
    if authority.is_empty() {
        return Err(format!("'url' has no host: {url}"));
    }
    if authority.contains('@') {
        return Err("'url' must not contain user information".into());
    }
    // Strip the port, taking IPv6 literals into account.
    let host = if let Some(end) = authority.find(']') {
        &authority[..=end]
    } else if let Some(colon) = authority.rfind(':') {
        &authority[..colon]
    } else {
        authority
    };
    if scheme == "http" && !is_loopback_host(host) {
        return Err(format!("non-loopback endpoint '{host}' must use https"));
    }
    Ok(())
}

fn validate_headers(headers: &[(String, String)]) -> Result<HashMap<String, String>, String> {
    let mut seen: HashMap<String, String> = HashMap::new();
    let mut out = HashMap::new();
    for (name, value) in headers {
        if http::HeaderName::from_bytes(name.as_bytes()).is_err() {
            return Err(format!("'{name}' is not a valid HTTP header name"));
        }
        if http::HeaderValue::from_str(value).is_err() {
            return Err(format!("header '{name}' has an invalid value"));
        }
        let lower = name.to_ascii_lowercase();
        if seen.contains_key(&lower) {
            return Err(format!("header '{name}' is declared more than once"));
        }
        seen.insert(lower, value.clone());
        out.insert(name.clone(), value.clone());
    }
    Ok(out)
}

fn is_reserved_env_name(key: &str) -> bool {
    if cfg!(windows) {
        key.eq_ignore_ascii_case("PLUGIN_ROOT") || key.eq_ignore_ascii_case("PLUGIN_DATA")
    } else {
        key == "PLUGIN_ROOT" || key == "PLUGIN_DATA"
    }
}

/// Validate one `mcpServers` entry and turn it into a native server entry with
/// placeholders already expanded (§7.2.1, §9).
fn build_server_entry(
    value: &serde_json::Value,
    root: &Path,
    data_dir: &Path,
) -> Result<McpServerEntry, String> {
    let obj = value
        .as_object()
        .ok_or_else(|| "server entry must be an object".to_string())?;
    let kind = obj
        .get("type")
        .and_then(|v| v.as_str())
        .ok_or_else(|| "server entry is missing the required 'type' field".to_string())?;

    let root_str = root.to_string_lossy().to_string();
    let data_str = data_dir.to_string_lossy().to_string();

    match kind {
        "stdio" => {
            for key in obj.keys() {
                if !STDIO_FIELDS.contains(&key.as_str()) {
                    return Err(format!(
                        "'{key}' is not a permitted field for a stdio server"
                    ));
                }
            }
            let command = obj
                .get("command")
                .and_then(|v| v.as_str())
                .ok_or_else(|| "stdio server is missing 'command'".to_string())?;
            if command.is_empty() {
                return Err("'command' must not be empty".into());
            }
            // §7.2.1 — `command` is one token: a bare name or a `./` path. No
            // placeholder expansion happens here.
            let resolved_command = if command.starts_with("./") {
                resolve_plugin_relative(root, command)?
                    .to_string_lossy()
                    .to_string()
            } else if command.contains('/') || command.contains('\\') {
                return Err(format!(
                    "'command' must be a bare executable name or a './' plugin-relative path: {command}"
                ));
            } else {
                command.to_string()
            };

            let args = match obj.get("args") {
                None => Vec::new(),
                Some(serde_json::Value::Array(items)) => {
                    let mut out = Vec::with_capacity(items.len());
                    for item in items {
                        let s = item
                            .as_str()
                            .ok_or_else(|| "'args' must be an array of strings".to_string())?;
                        out.push(expand_placeholders(s, &root_str, &data_str));
                    }
                    out
                }
                Some(_) => return Err("'args' must be an array of strings".into()),
            };

            let mut env = HashMap::new();
            if let Some(raw) = obj.get("env") {
                for (k, v) in string_map(raw, "env")? {
                    if is_reserved_env_name(&k) {
                        return Err(format!("'env.{k}' is reserved and supplied by the client"));
                    }
                    env.insert(k, expand_placeholders(&v, &root_str, &data_str));
                }
            }
            // §9.1 — reserved variables are set last so they always win.
            env.insert("PLUGIN_ROOT".to_string(), root_str.clone());
            env.insert("PLUGIN_DATA".to_string(), data_str.clone());

            let cwd = match obj.get("cwd") {
                None => root_str.clone(),
                Some(serde_json::Value::String(raw)) => resolve_cwd(raw, root, data_dir)?,
                Some(_) => return Err("'cwd' must be a string".into()),
            };

            Ok(McpServerEntry {
                transport: McpTransportConfig::Stdio {
                    command: resolved_command,
                    args,
                    env,
                    cwd: Some(cwd),
                },
                enabled: true,
            })
        }
        "streamable-http" => {
            for key in obj.keys() {
                if !REMOTE_FIELDS.contains(&key.as_str()) {
                    return Err(format!(
                        "'{key}' is not a permitted field for a streamable-http server"
                    ));
                }
            }
            let url = obj
                .get("url")
                .and_then(|v| v.as_str())
                .ok_or_else(|| "remote server is missing 'url'".to_string())?;
            validate_remote_url(url)?;
            let headers = match obj.get("headers") {
                None => HashMap::new(),
                Some(raw) => validate_headers(&string_map(raw, "headers")?)?,
            };
            Ok(McpServerEntry {
                transport: McpTransportConfig::Remote {
                    url: url.to_string(),
                    headers,
                },
                enabled: true,
            })
        }
        "sse" => Err(
            "transport 'sse' (legacy HTTP+SSE) is not supported by this client; \
             use 'streamable-http'"
                .into(),
        ),
        other => Err(format!("unknown server type '{other}'")),
    }
}

/// §7.2.1 — `cwd` is `./…`, `${PLUGIN_ROOT}[/…]` or `${PLUGIN_DATA}[/…]`.
fn resolve_cwd(raw: &str, root: &Path, data_dir: &Path) -> Result<String, String> {
    let root_str = root.to_string_lossy().to_string();
    let data_str = data_dir.to_string_lossy().to_string();

    if raw == "${PLUGIN_DATA}" || raw.starts_with("${PLUGIN_DATA}/") {
        let expanded = expand_placeholders(raw, &root_str, &data_str);
        let resolved = resolve_contained(data_dir, Path::new(&expanded))?;
        return Ok(resolved.to_string_lossy().to_string());
    }
    if raw == "${PLUGIN_ROOT}" || raw.starts_with("${PLUGIN_ROOT}/") {
        let expanded = expand_placeholders(raw, &root_str, &data_str);
        let resolved = resolve_contained(root, Path::new(&expanded))?;
        return Ok(resolved.to_string_lossy().to_string());
    }
    if raw.starts_with("./") {
        let resolved = resolve_plugin_relative(root, raw)?;
        return Ok(resolved.to_string_lossy().to_string());
    }
    Err(format!(
        "'cwd' must be './…', '${{PLUGIN_ROOT}}…' or '${{PLUGIN_DATA}}…': {raw}"
    ))
}

/// Parse `mcp.json` into validated server entries. `Err` disables MCP for the
/// plugin (§7.2.2 rule 2); per-entry failures land in `diagnostics`.
fn load_mcp_config(
    text: &str,
    plugin_name: &str,
    root: &Path,
    data_dir: &Path,
    diagnostics: &mut Vec<PluginDiagnostic>,
) -> Result<Vec<PluginMcpServer>, String> {
    let value: serde_json::Value =
        serde_json::from_str(text).map_err(|e| format!("mcp.json is not valid JSON: {e}"))?;
    let obj = value
        .as_object()
        .ok_or_else(|| "mcp.json must contain a top-level object".to_string())?;

    for key in obj.keys() {
        if key != "$schema" && key != "mcpServers" {
            return Err(format!("'{key}' is not a permitted mcp.json field"));
        }
    }

    let schema = obj
        .get("$schema")
        .and_then(|v| v.as_str())
        .ok_or_else(|| "mcp.json is missing the required '$schema' string".to_string())?;
    if schema != MCP_SCHEMA_ID {
        return Err(format!(
            "unsupported mcp.json '$schema' '{schema}', expected '{MCP_SCHEMA_ID}'"
        ));
    }

    let servers = obj
        .get("mcpServers")
        .ok_or_else(|| "mcp.json is missing the required 'mcpServers' object".to_string())?
        .as_object()
        .ok_or_else(|| "'mcpServers' must be an object".to_string())?;

    let mut out = Vec::new();
    for (name, entry) in servers {
        match build_server_entry(entry, root, data_dir) {
            Ok(built) => out.push(PluginMcpServer {
                local_name: name.clone(),
                qualified_name: format!("{plugin_name}.{name}"),
                entry: built,
            }),
            Err(e) => diagnostics.push(PluginDiagnostic::warning(format!(
                "MCP server '{name}' skipped: {e}"
            ))),
        }
    }
    out.sort_by(|a, b| a.local_name.cmp(&b.local_name));
    Ok(out)
}

// ─── Loading (§6) ─────────────────────────────────────────────────────────────

/// Persistent, client-managed data directory for an installed plugin (§9.1).
/// Keyed by name plus a hash of the root so two installs of the same plugin in
/// different scopes never share state.
pub fn plugin_data_dir(name: &str, root: &Path) -> PathBuf {
    let hash = xxhash_rust::xxh3::xxh3_64(root.to_string_lossy().as_bytes());
    let base = dirs::home_dir()
        .unwrap_or_else(std::env::temp_dir)
        .join(".claudinio")
        .join("plugin-data");
    base.join(format!("{name}-{hash:x}"))
}

/// Load one plugin directory. `Err` means the plugin is rejected outright.
pub fn load_plugin(dir: &Path, scope: PluginScope) -> Result<LoadedPlugin, String> {
    let root = std::fs::canonicalize(dir).map_err(|e| format!("{}: {e}", dir.display()))?;
    if !root.is_dir() {
        return Err(format!("{} is not a directory", root.display()));
    }

    let manifest_path = resolve_contained(&root, Path::new("plugin.json"))?;
    if !manifest_path.is_file() {
        return Err("missing plugin.json".to_string());
    }
    let manifest_text =
        std::fs::read_to_string(&manifest_path).map_err(|e| format!("read plugin.json: {e}"))?;
    let (manifest, mut diagnostics) = parse_manifest(&manifest_text)?;

    let data_dir = plugin_data_dir(&manifest.name, &root);

    // §7.1 — skills/
    let mut skills = Vec::new();
    let skills_dir = root.join("skills");
    if skills_dir.exists() {
        if skills_dir.is_dir() {
            skills = load_plugin_skills(&root, &skills_dir, &manifest.name, &mut diagnostics);
        } else {
            diagnostics.push(PluginDiagnostic::warning(
                "'skills' is not a directory; skills disabled for this plugin".to_string(),
            ));
        }
    }

    // §7.2 — mcp.json
    let mut mcp_servers = Vec::new();
    let mcp_path = root.join("mcp.json");
    if mcp_path.exists() {
        if mcp_path.is_file() {
            match std::fs::read_to_string(&mcp_path) {
                Ok(text) => {
                    match load_mcp_config(&text, &manifest.name, &root, &data_dir, &mut diagnostics)
                    {
                        Ok(servers) => mcp_servers = servers,
                        Err(e) => diagnostics.push(PluginDiagnostic::error(format!(
                            "MCP disabled for this plugin: {e}"
                        ))),
                    }
                }
                Err(e) => diagnostics.push(PluginDiagnostic::error(format!(
                    "MCP disabled for this plugin: read mcp.json: {e}"
                ))),
            }
        } else {
            diagnostics.push(PluginDiagnostic::warning(
                "'mcp.json' is not a regular file; MCP disabled for this plugin".to_string(),
            ));
        }
    }

    Ok(LoadedPlugin {
        manifest,
        root,
        data_dir,
        scope,
        skills,
        mcp_servers,
        diagnostics,
    })
}

/// §7.1 — each immediate child of `skills/` holding a `SKILL.md` regular file
/// is one skill. No deeper recursion.
fn load_plugin_skills(
    root: &Path,
    skills_dir: &Path,
    plugin_name: &str,
    diagnostics: &mut Vec<PluginDiagnostic>,
) -> Vec<SkillEntry> {
    let mut out = Vec::new();
    let entries = match std::fs::read_dir(skills_dir) {
        Ok(e) => e,
        Err(e) => {
            diagnostics.push(PluginDiagnostic::warning(format!("read skills/: {e}")));
            return out;
        }
    };

    for entry in entries.flatten() {
        let path = entry.path();
        if !path.is_dir() {
            continue;
        }
        let skill_md = path.join("SKILL.md");
        if !skill_md.is_file() {
            continue;
        }
        let dir_label = path
            .file_name()
            .and_then(|n| n.to_str())
            .unwrap_or("(unnamed)")
            .to_string();

        let contained = match resolve_contained(root, &skill_md) {
            Ok(p) => p,
            Err(e) => {
                diagnostics.push(PluginDiagnostic::warning(format!(
                    "skill '{dir_label}' skipped: {e}"
                )));
                continue;
            }
        };

        match SkillManager::parse_skill_md(&contained) {
            Ok((meta, body)) => out.push(SkillEntry {
                name: meta.name,
                description: meta.description,
                location: contained.to_string_lossy().to_string(),
                scope: SkillScope::Plugin,
                body: Some(body),
            }),
            Err(e) => diagnostics.push(PluginDiagnostic::warning(format!(
                "skill '{dir_label}' skipped: {e}"
            ))),
        }
    }

    out.sort_by(|a, b| a.name.cmp(&b.name));
    let _ = plugin_name;
    out
}

// ─── Discovery ────────────────────────────────────────────────────────────────

/// Every directory scanned for installed plugins, in priority order.
pub fn plugin_search_dirs(workspace_root: Option<&Path>) -> Vec<(PathBuf, PluginScope)> {
    let mut dirs = Vec::new();
    if let Some(root) = workspace_root {
        for name in PLUGIN_DIR_NAMES {
            dirs.push((root.join(name).join("plugins"), PluginScope::Project));
        }
    }
    if let Some(home) = dirs::home_dir() {
        for name in PLUGIN_DIR_NAMES {
            dirs.push((home.join(name).join("plugins"), PluginScope::User));
        }
    }
    dirs
}

/// The default install directory for user-scope plugins.
pub fn user_install_dir() -> Option<PathBuf> {
    dirs::home_dir().map(|h| h.join(".claudinio").join("plugins"))
}

/// Discover and load every installed plugin. Project scope wins over user scope
/// when the same plugin name appears in both.
pub fn discover(workspace_root: Option<&Path>) -> Vec<PluginRecord> {
    let mut by_name: HashMap<String, PluginRecord> = HashMap::new();
    let mut order: Vec<String> = Vec::new();

    for (dir, scope) in plugin_search_dirs(workspace_root) {
        let entries = match std::fs::read_dir(&dir) {
            Ok(e) => e,
            Err(_) => continue,
        };
        for entry in entries.flatten() {
            let path = entry.path();
            if !path.is_dir() {
                continue;
            }
            if !path.join("plugin.json").exists() {
                continue;
            }
            let record = match load_plugin(&path, scope.clone()) {
                Ok(plugin) => PluginRecord::Loaded(Box::new(plugin)),
                Err(error) => PluginRecord::Rejected {
                    root: path.clone(),
                    scope: scope.clone(),
                    error,
                },
            };
            let key = record.name();
            if let Some(existing) = by_name.get(&key) {
                // Project scope was inserted first — keep it.
                if matches!(existing_scope(existing), PluginScope::Project)
                    && scope == PluginScope::User
                {
                    continue;
                }
            } else {
                order.push(key.clone());
            }
            by_name.insert(key, record);
        }
    }

    order
        .into_iter()
        .filter_map(|k| by_name.remove(&k))
        .collect()
}

fn existing_scope(record: &PluginRecord) -> PluginScope {
    match record {
        PluginRecord::Loaded(p) => p.scope.clone(),
        PluginRecord::Rejected { scope, .. } => scope.clone(),
    }
}

/// Whether a plugin's components should load: the user's explicit choice if
/// they made one, otherwise whatever the plugin declares as its default.
pub fn is_plugin_enabled(
    plugin: &LoadedPlugin,
    prefs: &HashMap<String, crate::agent::provider::PluginPrefs>,
) -> bool {
    match prefs.get(&plugin.manifest.name) {
        Some(p) => p.enabled,
        None => plugin.manifest.enabled_by_default(),
    }
}

/// Whether a discovered plugin — valid or not — counts as enabled, for display.
pub fn is_record_enabled(
    record: &PluginRecord,
    prefs: &HashMap<String, crate::agent::provider::PluginPrefs>,
) -> bool {
    match record {
        PluginRecord::Loaded(plugin) => is_plugin_enabled(plugin, prefs),
        PluginRecord::Rejected { .. } => {
            prefs.get(&record.name()).map(|p| p.enabled).unwrap_or(true)
        }
    }
}

/// Skills contributed by enabled plugins, ready to merge into the catalog.
pub fn enabled_plugin_skills(
    records: &[PluginRecord],
    prefs: &HashMap<String, crate::agent::provider::PluginPrefs>,
) -> Vec<SkillEntry> {
    let mut out = Vec::new();
    for record in records {
        if let PluginRecord::Loaded(plugin) = record
            && is_plugin_enabled(plugin, prefs)
        {
            out.extend(plugin.skills.iter().cloned());
        }
    }
    out
}

/// MCP servers contributed by enabled plugins, keyed by `<plugin>.<server>`.
/// Creates each plugin's `PLUGIN_DATA` directory before the server is launched.
pub fn enabled_plugin_mcp_servers(
    records: &[PluginRecord],
    prefs: &HashMap<String, crate::agent::provider::PluginPrefs>,
) -> HashMap<String, McpServerEntry> {
    let mut out = HashMap::new();
    for record in records {
        if let PluginRecord::Loaded(plugin) = record
            && is_plugin_enabled(plugin, prefs)
            && !plugin.mcp_servers.is_empty()
        {
            if let Err(e) = std::fs::create_dir_all(&plugin.data_dir) {
                eprintln!(
                    "[plugins] failed to create PLUGIN_DATA for '{}': {e}",
                    plugin.manifest.name
                );
                continue;
            }
            for server in &plugin.mcp_servers {
                out.insert(server.qualified_name.clone(), server.entry.clone());
            }
        }
    }
    out
}

#[cfg(test)]
mod tests {
    use super::*;

    fn manifest_json(extra: &str) -> String {
        format!("{{\"$schema\":\"{PLUGIN_SCHEMA_ID}\",\"name\":\"demo\"{extra}}}")
    }

    #[test]
    fn parses_a_minimal_manifest() {
        let (manifest, diags) = parse_manifest(&manifest_json("")).unwrap();
        assert_eq!(manifest.name, "demo");
        assert!(diags.is_empty());
    }

    #[test]
    fn rejects_wrong_schema_id() {
        let text = "{\"$schema\":\"https://agent-plugins.org/schemas/9.9.9/plugin.schema.json\",\"name\":\"demo\"}";
        assert!(parse_manifest(text).is_err());
    }

    #[test]
    fn rejects_missing_name() {
        let text = format!("{{\"$schema\":\"{PLUGIN_SCHEMA_ID}\"}}");
        assert!(parse_manifest(&text).is_err());
    }

    #[test]
    fn reports_and_ignores_unknown_fields() {
        let (manifest, diags) = parse_manifest(&manifest_json(",\"sparkles\":{}")).unwrap();
        assert_eq!(manifest.name, "demo");
        assert_eq!(diags.len(), 1);
        assert!(diags[0].message.contains("sparkles"));
    }

    #[test]
    fn hooks_is_a_recognised_field_and_is_kept() {
        // It used to be reported as unknown. It is load-bearing now: see
        // MANIFEST_FIELDS and `crate::agent::hooks::discovery`.
        let (manifest, diags) =
            parse_manifest(&manifest_json(",\"hooks\":\"./hooks/hooks.json\"")).unwrap();
        assert!(diags.is_empty(), "{diags:?}");
        assert_eq!(manifest.hooks.unwrap(), "./hooks/hooks.json");
    }

    #[test]
    fn reports_and_ignores_non_object_extensions() {
        let (manifest, diags) = parse_manifest(&manifest_json(",\"extensions\":42")).unwrap();
        assert!(manifest.extensions.is_empty());
        assert_eq!(diags.len(), 1);
    }

    #[test]
    fn rejects_invalid_author_fields() {
        let text = manifest_json(",\"author\":{\"nickname\":\"x\"}");
        assert!(parse_manifest(&text).is_err());
    }

    #[test]
    fn honours_the_client_extension_default_enable_flag() {
        let (plain, _) = parse_manifest(&manifest_json("")).unwrap();
        assert!(plain.enabled_by_default());

        let (off, _) = parse_manifest(&manifest_json(
            ",\"extensions\":{\"io.claudin.claudinio\":{\"enabledByDefault\":false}}",
        ))
        .unwrap();
        assert!(!off.enabled_by_default());

        // Another client's namespace is ignored without being validated.
        let (other, diags) = parse_manifest(&manifest_json(
            ",\"extensions\":{\"com.example.client\":{\"whatever\":[1,2]}}",
        ))
        .unwrap();
        assert!(other.enabled_by_default());
        assert!(other.client_extension().is_none());
        assert!(diags.is_empty());
    }

    #[test]
    fn an_explicit_preference_overrides_the_plugin_default() {
        let (manifest, _) = parse_manifest(&manifest_json(
            ",\"extensions\":{\"io.claudin.claudinio\":{\"enabledByDefault\":false}}",
        ))
        .unwrap();
        let plugin = LoadedPlugin {
            manifest,
            root: PathBuf::from("/tmp/plugins/demo"),
            data_dir: PathBuf::from("/tmp/plugin-data/demo"),
            scope: PluginScope::User,
            skills: Vec::new(),
            mcp_servers: Vec::new(),
            diagnostics: Vec::new(),
        };
        assert!(!is_plugin_enabled(&plugin, &HashMap::new()));
        let prefs = HashMap::from([(
            "demo".to_string(),
            crate::agent::provider::PluginPrefs { enabled: true },
        )]);
        assert!(is_plugin_enabled(&plugin, &prefs));
    }

    #[test]
    fn validates_plugin_names() {
        assert!(validate_plugin_name("my-plugin").is_ok());
        assert!(validate_plugin_name("acme.tools").is_ok());
        assert!(validate_plugin_name("a").is_ok());
        assert!(validate_plugin_name("My-Plugin").is_err());
        assert!(validate_plugin_name("-start").is_err());
        assert!(validate_plugin_name("has--double").is_err());
        assert!(validate_plugin_name("too.many..dots").is_err());
        assert!(validate_plugin_name("").is_err());
    }

    #[test]
    fn expands_placeholders_once_and_non_recursively() {
        let out = expand_placeholders("${PLUGIN_ROOT}/x:${PLUGIN_DATA}", "/root", "/data");
        assert_eq!(out, "/root/x:/data");
        // Text introduced by a replacement is not rescanned.
        let out = expand_placeholders("${PLUGIN_ROOT}", "${PLUGIN_DATA}", "/data");
        assert_eq!(out, "${PLUGIN_DATA}");
        // Unknown placeholder-like text stays literal.
        assert_eq!(expand_placeholders("${HOME}", "/root", "/data"), "${HOME}");
    }

    #[test]
    fn rejects_paths_escaping_the_plugin_root() {
        let root = Path::new("/tmp/plugins/demo");
        assert!(resolve_plugin_relative(root, "./bin/server").is_ok());
        assert!(resolve_plugin_relative(root, "../bin/server").is_err());
        assert!(resolve_plugin_relative(root, "bin/server").is_err());
        assert!(resolve_plugin_relative(root, "./a/../../b").is_err());
    }

    #[test]
    fn validates_remote_urls() {
        assert!(validate_remote_url("https://example.com/mcp").is_ok());
        assert!(validate_remote_url("http://localhost:3000/mcp").is_ok());
        assert!(validate_remote_url("http://127.0.0.1:3000/mcp").is_ok());
        assert!(validate_remote_url("http://example.com/mcp").is_err());
        assert!(validate_remote_url("https://user:pw@example.com/mcp").is_err());
        assert!(validate_remote_url("https://example.com/mcp#frag").is_err());
        assert!(validate_remote_url("ftp://example.com").is_err());
    }

    #[test]
    fn builds_a_stdio_entry_with_reserved_env_and_default_cwd() {
        let root = Path::new("/tmp/plugins/demo");
        let data = Path::new("/tmp/plugin-data/demo");
        let value = serde_json::json!({
            "type": "stdio",
            "command": "npx",
            "args": ["--config", "${PLUGIN_ROOT}/config.json"],
            "env": {"DATA_DIR": "${PLUGIN_DATA}/db"}
        });
        let entry = build_server_entry(&value, root, data).unwrap();
        match entry.transport {
            McpTransportConfig::Stdio {
                command,
                args,
                env,
                cwd,
            } => {
                assert_eq!(command, "npx");
                assert_eq!(args[1], "/tmp/plugins/demo/config.json");
                assert_eq!(env.get("DATA_DIR").unwrap(), "/tmp/plugin-data/demo/db");
                assert_eq!(env.get("PLUGIN_ROOT").unwrap(), "/tmp/plugins/demo");
                assert_eq!(env.get("PLUGIN_DATA").unwrap(), "/tmp/plugin-data/demo");
                assert_eq!(cwd.unwrap(), "/tmp/plugins/demo");
            }
            _ => panic!("expected stdio"),
        }
    }

    #[test]
    fn rejects_reserved_env_names_and_unknown_fields() {
        let root = Path::new("/tmp/plugins/demo");
        let data = Path::new("/tmp/plugin-data/demo");
        let reserved = serde_json::json!({
            "type": "stdio", "command": "npx", "env": {"PLUGIN_ROOT": "/x"}
        });
        assert!(build_server_entry(&reserved, root, data).is_err());

        let unknown = serde_json::json!({
            "type": "stdio", "command": "npx", "url": "https://x.example"
        });
        assert!(build_server_entry(&unknown, root, data).is_err());
    }

    #[test]
    fn rejects_multi_segment_bare_commands() {
        let root = Path::new("/tmp/plugins/demo");
        let data = Path::new("/tmp/plugin-data/demo");
        let value = serde_json::json!({"type": "stdio", "command": "../bin/server"});
        assert!(build_server_entry(&value, root, data).is_err());
    }

    #[test]
    fn skips_unsupported_sse_transport() {
        let root = Path::new("/tmp/plugins/demo");
        let data = Path::new("/tmp/plugin-data/demo");
        let value = serde_json::json!({"type": "sse", "url": "https://x.example/sse"});
        assert!(build_server_entry(&value, root, data).is_err());
    }

    #[test]
    fn resolves_cwd_forms() {
        let root = Path::new("/tmp/plugins/demo");
        let data = Path::new("/tmp/plugin-data/demo");
        assert_eq!(
            resolve_cwd("${PLUGIN_ROOT}", root, data).unwrap(),
            "/tmp/plugins/demo"
        );
        assert_eq!(
            resolve_cwd("${PLUGIN_DATA}/work", root, data).unwrap(),
            "/tmp/plugin-data/demo/work"
        );
        assert_eq!(
            resolve_cwd("./data", root, data).unwrap(),
            "/tmp/plugins/demo/data"
        );
        assert!(resolve_cwd("data", root, data).is_err());
        assert!(resolve_cwd("/etc", root, data).is_err());
        assert!(resolve_cwd("${PLUGIN_ROOT}/../escape", root, data).is_err());
    }

    #[test]
    fn skips_invalid_servers_but_keeps_valid_ones() {
        let text = format!(
            "{{\"$schema\":\"{MCP_SCHEMA_ID}\",\"mcpServers\":{{\
               \"good\":{{\"type\":\"streamable-http\",\"url\":\"https://a.example/mcp\"}},\
               \"bad\":{{\"type\":\"nope\"}}}}}}"
        );
        let mut diags = Vec::new();
        let servers = load_mcp_config(
            &text,
            "demo",
            Path::new("/tmp/plugins/demo"),
            Path::new("/tmp/plugin-data/demo"),
            &mut diags,
        )
        .unwrap();
        assert_eq!(servers.len(), 1);
        assert_eq!(servers[0].qualified_name, "demo.good");
        assert_eq!(diags.len(), 1);
    }

    #[test]
    fn rejects_mcp_config_with_mismatched_schema_version() {
        let text = "{\"$schema\":\"https://agent-plugins.org/schemas/0.9.0/mcp.schema.json\",\"mcpServers\":{}}";
        let mut diags = Vec::new();
        assert!(
            load_mcp_config(
                text,
                "demo",
                Path::new("/tmp/plugins/demo"),
                Path::new("/tmp/plugin-data/demo"),
                &mut diags
            )
            .is_err()
        );
    }

    #[test]
    fn rejects_extra_top_level_mcp_fields() {
        let text = format!("{{\"$schema\":\"{MCP_SCHEMA_ID}\",\"mcpServers\":{{}},\"extra\":1}}");
        let mut diags = Vec::new();
        assert!(
            load_mcp_config(
                &text,
                "demo",
                Path::new("/tmp/plugins/demo"),
                Path::new("/tmp/plugin-data/demo"),
                &mut diags
            )
            .is_err()
        );
    }

    #[test]
    fn loads_a_plugin_directory_end_to_end() {
        let base =
            std::env::temp_dir().join(format!("claudinio-plugin-test-{}", uuid::Uuid::new_v4()));
        let root = base.join("demo");
        std::fs::create_dir_all(root.join("skills").join("summarize")).unwrap();
        std::fs::write(
            root.join("plugin.json"),
            manifest_json(",\"version\":\"1.0.0\""),
        )
        .unwrap();
        std::fs::write(
            root.join("skills").join("summarize").join("SKILL.md"),
            "---\nname: summarize\ndescription: Summarize long documents into bullets\n---\n\n# Summarize\n",
        )
        .unwrap();
        std::fs::write(
            root.join("mcp.json"),
            format!(
                "{{\"$schema\":\"{MCP_SCHEMA_ID}\",\"mcpServers\":{{\"api\":{{\"type\":\"streamable-http\",\"url\":\"https://a.example/mcp\"}}}}}}"
            ),
        )
        .unwrap();

        let plugin = load_plugin(&root, PluginScope::User).unwrap();
        assert_eq!(plugin.manifest.name, "demo");
        assert_eq!(plugin.skills.len(), 1);
        assert_eq!(plugin.skills[0].name, "summarize");
        assert_eq!(plugin.skills[0].scope, SkillScope::Plugin);
        assert_eq!(plugin.mcp_servers.len(), 1);
        assert_eq!(plugin.mcp_servers[0].qualified_name, "demo.api");
        assert!(plugin.diagnostics.is_empty());

        let _ = std::fs::remove_dir_all(&base);
    }

    #[test]
    fn missing_component_locations_are_not_errors() {
        let base =
            std::env::temp_dir().join(format!("claudinio-plugin-empty-{}", uuid::Uuid::new_v4()));
        let root = base.join("bare");
        std::fs::create_dir_all(&root).unwrap();
        std::fs::write(root.join("plugin.json"), manifest_json("")).unwrap();

        let plugin = load_plugin(&root, PluginScope::User).unwrap();
        assert!(plugin.skills.is_empty());
        assert!(plugin.mcp_servers.is_empty());
        assert!(plugin.diagnostics.is_empty());

        let _ = std::fs::remove_dir_all(&base);
    }

    #[test]
    fn enabled_filters_apply_to_components() {
        let base =
            std::env::temp_dir().join(format!("claudinio-plugin-filter-{}", uuid::Uuid::new_v4()));
        let root = base.join("demo");
        std::fs::create_dir_all(root.join("skills").join("s1")).unwrap();
        std::fs::write(root.join("plugin.json"), manifest_json("")).unwrap();
        std::fs::write(
            root.join("skills").join("s1").join("SKILL.md"),
            "---\nname: s1\ndescription: A skill that does one thing well\n---\n\nbody",
        )
        .unwrap();

        let records = vec![PluginRecord::Loaded(Box::new(
            load_plugin(&root, PluginScope::User).unwrap(),
        ))];
        let on = HashMap::new();
        let off = HashMap::from([(
            "demo".to_string(),
            crate::agent::provider::PluginPrefs { enabled: false },
        )]);
        assert_eq!(enabled_plugin_skills(&records, &on).len(), 1);
        assert_eq!(enabled_plugin_skills(&records, &off).len(), 0);

        let _ = std::fs::remove_dir_all(&base);
    }
}
