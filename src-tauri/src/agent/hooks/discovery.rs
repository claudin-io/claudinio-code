//! Finding every hook that applies to a workspace.
//!
//! Six places may declare one, and the rule between them is **union, not
//! override**. A user-level guard and a project-level guard both run; neither
//! shadows the other. That is Claude Code's behaviour and it is the only one
//! under which "I put a guard in my home settings" means anything on a
//! repository that also ships hooks.
//!
//! Precedence exists here for exactly two purposes: a stable order (so injected
//! context does not reshuffle between runs and cost a prompt-cache hit), and a
//! deterministic winner when the same command is declared twice.

use super::config::{HookEvent, HookGroup, HooksBlock, parse_block, parse_file};
use crate::agent::plugins;
use crate::agent::provider::AgentConfig;
use serde::Serialize;
use sha2::{Digest, Sha256};
use std::collections::HashMap;
use std::path::{Path, PathBuf};

/// Where a hook was declared. The panel shows this verbatim: approving a command
/// you cannot trace to a file is not consent.
#[derive(Debug, Clone, PartialEq, Eq, Serialize)]
#[serde(tag = "kind", rename_all = "camelCase")]
pub enum HookSource {
    /// `~/.claude/settings.json`
    UserSettings { path: String },
    /// The app's own config.json
    AppConfig,
    /// A plugin's `hooks/hooks.json` or its manifest
    Plugin { name: String, root: String },
    /// `<workspace>/.claudinio.json`
    WorkspaceConfig { path: String },
    /// `<workspace>/.claude/settings.json`
    ProjectSettings { path: String },
    /// `<workspace>/.claude/settings.local.json`
    LocalSettings { path: String },
}

impl HookSource {
    /// Resolution order. Broad-to-narrow: what the machine says, then what the
    /// project says, then what this checkout says.
    fn rank(&self) -> u8 {
        match self {
            HookSource::UserSettings { .. } => 0,
            HookSource::AppConfig => 1,
            HookSource::Plugin { .. } => 2,
            HookSource::WorkspaceConfig { .. } => 3,
            HookSource::ProjectSettings { .. } => 4,
            HookSource::LocalSettings { .. } => 5,
        }
    }

    pub fn label(&self) -> String {
        match self {
            HookSource::UserSettings { path } => path.clone(),
            HookSource::AppConfig => "app settings".into(),
            HookSource::Plugin { name, .. } => format!("plugin: {name}"),
            HookSource::WorkspaceConfig { path } => path.clone(),
            HookSource::ProjectSettings { path } => path.clone(),
            HookSource::LocalSettings { path } => path.clone(),
        }
    }

    fn key(&self) -> String {
        match self {
            HookSource::UserSettings { path }
            | HookSource::WorkspaceConfig { path }
            | HookSource::ProjectSettings { path }
            | HookSource::LocalSettings { path } => format!("file:{path}"),
            HookSource::AppConfig => "app".into(),
            HookSource::Plugin { name, root } => format!("plugin:{name}:{root}"),
        }
    }
}

/// One hook, ready to run: placeholders already expanded, timeout already
/// resolved. Expansion happens here rather than at spawn time so the trust
/// fingerprint covers the command that will actually execute.
#[derive(Debug, Clone, PartialEq, Eq, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct ResolvedHook {
    pub event: HookEvent,
    pub matcher: Option<String>,
    pub command: String,
    pub args: Vec<String>,
    pub timeout_secs: u64,
    pub status_message: Option<String>,
    pub source: HookSource,
    #[serde(skip)]
    pub plugin_root: Option<PathBuf>,
    /// The other places that declared the identical command. It runs once.
    pub also_from: Vec<HookSource>,
    #[serde(skip)]
    pub(crate) order: (u8, usize, usize),
}

impl ResolvedHook {
    /// The line the trust hash is computed over, and the line the panel shows.
    pub fn signature(&self) -> String {
        format!(
            "{}\t{}\t{}\t{}\t{}\t{}",
            self.event.as_str(),
            self.matcher.as_deref().unwrap_or("*"),
            self.command,
            self.args.join("\u{1f}"),
            self.timeout_secs,
            self.source.key(),
        )
    }

    pub fn display_command(&self) -> String {
        if self.args.is_empty() {
            self.command.clone()
        } else {
            format!("{} {}", self.command, self.args.join(" "))
        }
    }
}

#[derive(Debug, Clone, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct HookDiagnostic {
    pub source: String,
    pub message: String,
}

/// Everything a workspace declares, in the order it will run.
#[derive(Debug, Clone, Default, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct HookSet {
    pub hooks: Vec<ResolvedHook>,
    pub diagnostics: Vec<HookDiagnostic>,
    /// `sha256:…` over every hook's signature. What the user approves.
    pub fingerprint: String,
    pub workspace: Option<String>,
}

impl HookSet {
    /// The hooks that fire for a tool call.
    pub fn select_tool(&self, event: HookEvent, native_tool: &str) -> Vec<&ResolvedHook> {
        self.hooks
            .iter()
            .filter(|h| {
                h.event == event && super::matcher::matches(h.matcher.as_deref(), native_tool)
            })
            .collect()
    }

    /// The hooks that fire for an event whose matcher is a literal trigger word
    /// (`PreCompact`, `SessionStart`) or which has no matcher dimension at all.
    pub fn select_literal(&self, event: HookEvent, value: &str) -> Vec<&ResolvedHook> {
        self.hooks
            .iter()
            .filter(|h| {
                h.event == event && super::matcher::matches_literal(h.matcher.as_deref(), value)
            })
            .collect()
    }

    pub fn select(&self, event: HookEvent) -> Vec<&ResolvedHook> {
        self.hooks.iter().filter(|h| h.event == event).collect()
    }
}

/// Substitute the placeholders a hook config may use.
///
/// Single-pass and non-recursive, the same construction as
/// `plugins::expand_placeholders` and for the same reason: a value that expands
/// into another placeholder must not be able to keep expanding.
pub fn expand(input: &str, project_dir: &str, plugin_root: Option<&str>) -> String {
    let mut out = String::with_capacity(input.len());
    let mut rest = input;
    loop {
        let Some(at) = rest.find("${") else {
            out.push_str(rest);
            return out;
        };
        let Some(close) = rest[at..].find('}') else {
            out.push_str(rest);
            return out;
        };
        out.push_str(&rest[..at]);
        let name = &rest[at + 2..at + close];
        let sub = match name {
            "CLAUDE_PROJECT_DIR" | "CLAUDINIO_PROJECT_DIR" => Some(project_dir),
            "CLAUDE_PLUGIN_ROOT" | "CLAUDINIO_PLUGIN_ROOT" => plugin_root,
            _ => None,
        };
        match sub {
            Some(v) => out.push_str(v),
            // An unknown placeholder is left verbatim rather than blanked. A
            // hook whose path silently became `/hooks/x.sh` is far harder to
            // diagnose than one that visibly still says `${WHATEVER}`.
            None => out.push_str(&rest[at..at + close + 1]),
        }
        rest = &rest[at + close + 1..];
    }
}

/// Read a JSON file's `hooks` block, tolerating every way it can be absent.
fn block_from_file(path: &Path, diags: &mut Vec<HookDiagnostic>) -> HooksBlock {
    let Ok(text) = std::fs::read_to_string(path) else {
        return HooksBlock::new();
    };
    let mut local = Vec::new();
    // These files carry far more than hooks (permissions, env, …). Only the
    // `hooks` key is read; a settings file with no hooks is not a diagnostic.
    let v: serde_json::Value = match serde_json::from_str(&text) {
        Ok(v) => v,
        Err(e) => {
            diags.push(HookDiagnostic {
                source: path.display().to_string(),
                message: format!("not valid JSON: {e}"),
            });
            return HooksBlock::new();
        }
    };
    let Some(hooks) = v.get("hooks") else {
        return HooksBlock::new();
    };
    let block = parse_block(hooks, &mut local);
    for m in local {
        diags.push(HookDiagnostic {
            source: path.display().to_string(),
            message: m,
        });
    }
    block
}

struct Contribution {
    source: HookSource,
    block: HooksBlock,
    plugin_root: Option<PathBuf>,
}

/// Resolve every hook that applies to `workspace`.
///
/// A session with no workspace gets nothing: there is no project to scope trust
/// to, and a hook that could run against "wherever the app happens to be" is a
/// hook with no blast radius anybody can reason about.
pub fn resolve(workspace: Option<&Path>, config: &AgentConfig) -> HookSet {
    resolve_with_home(workspace, config, dirs::home_dir().as_deref())
}

/// The same, with the user's home directory named explicitly.
///
/// Exists so a test can be a test: `resolve` reads `~/.claude/settings.json` and
/// `~/.claudinio/plugins/`, and a test that silently inherits whatever the
/// developer has installed there is a test that passes on one machine.
pub fn resolve_with_home(
    workspace: Option<&Path>,
    config: &AgentConfig,
    home: Option<&Path>,
) -> HookSet {
    let mut set = HookSet {
        workspace: workspace.map(|w| w.display().to_string()),
        ..Default::default()
    };
    if !config.hooks_enabled {
        return set;
    }
    let Some(ws) = workspace else {
        return set;
    };
    let project_dir = ws.display().to_string();
    let mut diags: Vec<HookDiagnostic> = Vec::new();
    let mut contributions: Vec<Contribution> = Vec::new();

    if let Some(home) = home {
        let p = home.join(".claude").join("settings.json");
        let block = block_from_file(&p, &mut diags);
        contributions.push(Contribution {
            source: HookSource::UserSettings {
                path: p.display().to_string(),
            },
            block,
            plugin_root: None,
        });
    }

    if let Some(h) = &config.hooks {
        let mut local = Vec::new();
        let block = parse_block(h, &mut local);
        for m in local {
            diags.push(HookDiagnostic {
                source: "app settings".into(),
                message: m,
            });
        }
        contributions.push(Contribution {
            source: HookSource::AppConfig,
            block,
            plugin_root: None,
        });
    }

    contributions.extend(plugin_contributions(ws, home, config, &mut diags));

    let wsc = ws.join(".claudinio.json");
    let block = block_from_file(&wsc, &mut diags);
    contributions.push(Contribution {
        source: HookSource::WorkspaceConfig {
            path: wsc.display().to_string(),
        },
        block,
        plugin_root: None,
    });

    for (file, mk) in [
        (
            "settings.json",
            &(|p: String| HookSource::ProjectSettings { path: p }) as &dyn Fn(String) -> HookSource,
        ),
        (
            "settings.local.json",
            &(|p: String| HookSource::LocalSettings { path: p }) as &dyn Fn(String) -> HookSource,
        ),
    ] {
        let p = ws.join(".claude").join(file);
        let block = block_from_file(&p, &mut diags);
        contributions.push(Contribution {
            source: mk(p.display().to_string()),
            block,
            plugin_root: None,
        });
    }

    // Flatten, expanding as we go so the fingerprint covers the real command.
    let mut flat: Vec<ResolvedHook> = Vec::new();
    for c in &contributions {
        let plugin_root_str = c.plugin_root.as_ref().map(|p| p.display().to_string());
        let mut events: Vec<&HookEvent> = c.block.keys().collect();
        events.sort();
        for event in events {
            for (gi, group) in c.block[event].iter().enumerate() {
                push_group(
                    &mut flat,
                    *event,
                    group,
                    c,
                    gi,
                    &project_dir,
                    plugin_root_str.as_deref(),
                );
            }
        }
    }

    flat.sort_by_key(|h| h.order);

    // Dedup on the expanded command. The same hook declared in two files is one
    // program; running it twice would double an injected context and could
    // double a denial message.
    let mut seen: HashMap<String, usize> = HashMap::new();
    let mut deduped: Vec<ResolvedHook> = Vec::new();
    for h in flat {
        let key = format!(
            "{}\u{1e}{}\u{1e}{}\u{1e}{}",
            h.event.as_str(),
            h.matcher.as_deref().unwrap_or("*"),
            h.command,
            h.args.join("\u{1f}")
        );
        match seen.get(&key) {
            Some(ix) => {
                let first: &mut ResolvedHook = &mut deduped[*ix];
                if !first.also_from.contains(&h.source) {
                    first.also_from.push(h.source);
                }
            }
            None => {
                seen.insert(key, deduped.len());
                deduped.push(h);
            }
        }
    }

    for h in &deduped {
        if let Some(m) = &h.matcher
            && !super::matcher::matcher_is_valid_regex(m)
        {
            {
                diags.push(HookDiagnostic {
                    source: h.source.label(),
                    message: format!(
                        "matcher `{m}` is not a valid regex — it will only match that exact name"
                    ),
                });
            }
        }
    }

    set.fingerprint = fingerprint(&deduped);
    set.hooks = deduped;
    set.diagnostics = diags;
    set
}

fn push_group(
    out: &mut Vec<ResolvedHook>,
    event: HookEvent,
    group: &HookGroup,
    c: &Contribution,
    gi: usize,
    project_dir: &str,
    plugin_root: Option<&str>,
) {
    for (hi, action) in group.hooks.iter().enumerate() {
        out.push(ResolvedHook {
            event,
            matcher: group.matcher.clone(),
            command: expand(action.command(), project_dir, plugin_root),
            args: action
                .args()
                .iter()
                .map(|a| expand(a, project_dir, plugin_root))
                .collect(),
            timeout_secs: action.timeout_secs(),
            status_message: action.status_message().map(str::to_string),
            source: c.source.clone(),
            plugin_root: c.plugin_root.clone(),
            also_from: Vec::new(),
            order: (c.source.rank(), gi, hi),
        });
    }
}

/// Hooks contributed by plugins, from the manifest and from the convention.
///
/// The convention matters more than the manifest field. The plugin this feature
/// was built against (`claudinio-brain`) declares no `hooks` key at all — Claude
/// Code finds `hooks/hooks.json` by looking for it. Requiring a manifest entry
/// would mean telling users to edit a package they do not own, which is exactly
/// the "works with zero edits" this is supposed to deliver.
fn plugin_contributions(
    ws: &Path,
    home: Option<&Path>,
    config: &AgentConfig,
    diags: &mut Vec<HookDiagnostic>,
) -> Vec<Contribution> {
    let mut out = Vec::new();
    let mut seen_names: Vec<String> = Vec::new();

    for dir in plugin_dirs(ws, home) {
        let Ok(entries) = std::fs::read_dir(&dir) else {
            continue;
        };
        let mut roots: Vec<PathBuf> = entries
            .flatten()
            .map(|e| e.path())
            .filter(|p| p.is_dir())
            .collect();
        // Stable across filesystems: `read_dir` order is not.
        roots.sort();
        for root in roots {
            let manifest = root.join("plugin.json");
            let (name, declared) = if manifest.exists() {
                // An Agent Plugins package. A manifest that does not parse
                // contributes nothing: hooks from a package we could not
                // validate is exactly the trust we are not extending.
                match plugins::load_plugin(&root, plugins::PluginScope::Project) {
                    Ok(p) => {
                        if !plugins::is_plugin_enabled(&p, &config.plugins) {
                            continue;
                        }
                        (p.manifest.name.clone(), p.manifest.hooks.clone())
                    }
                    Err(_) => continue,
                }
            } else if root.join("hooks").join("hooks.json").exists() {
                // A package laid out the way Claude Code's own plugins are —
                // manifest under `.claude-plugin/`, hooks under `hooks/`. Not an
                // Agent Plugins package, so it contributes hooks and nothing
                // else: no skills, no MCP servers, no manifest trust. Supporting
                // it is what makes "works with zero edits" true for packages the
                // user does not own.
                let name = claude_plugin_name(&root);
                if config
                    .plugins
                    .get(&name)
                    .map(|p| !p.enabled)
                    .unwrap_or(false)
                {
                    continue;
                }
                (name, None)
            } else {
                continue;
            };

            // Project scope is scanned first; a user-scope package of the same
            // name is the same plugin installed twice.
            if seen_names.contains(&name) {
                continue;
            }
            seen_names.push(name.clone());

            if let Some(block) = plugin_block(&root, &name, declared.as_ref(), diags) {
                out.push(Contribution {
                    source: HookSource::Plugin {
                        name,
                        root: root.display().to_string(),
                    },
                    block,
                    plugin_root: Some(root),
                });
            }
        }
    }
    out
}

/// Where plugins live: project scope first, then user scope. Mirrors
/// `plugins::plugin_search_dirs`, with the home directory passed in rather than
/// read from the environment.
fn plugin_dirs(ws: &Path, home: Option<&Path>) -> Vec<PathBuf> {
    let mut dirs: Vec<PathBuf> = plugins::PLUGIN_DIR_NAMES
        .iter()
        .map(|n| ws.join(n).join("plugins"))
        .collect();
    if let Some(h) = home {
        dirs.extend(
            plugins::PLUGIN_DIR_NAMES
                .iter()
                .map(|n| h.join(n).join("plugins")),
        );
    }
    dirs
}

fn claude_plugin_name(root: &Path) -> String {
    std::fs::read_to_string(root.join(".claude-plugin").join("plugin.json"))
        .ok()
        .and_then(|t| serde_json::from_str::<serde_json::Value>(&t).ok())
        .and_then(|v| v.get("name").and_then(|n| n.as_str()).map(str::to_string))
        .unwrap_or_else(|| {
            root.file_name()
                .and_then(|n| n.to_str())
                .unwrap_or("(plugin)")
                .to_string()
        })
}

/// A plugin's hooks: its manifest's `hooks` if it declares one, otherwise the
/// conventional `hooks/hooks.json`.
fn plugin_block(
    root: &Path,
    name: &str,
    declared: Option<&serde_json::Value>,
    diags: &mut Vec<HookDiagnostic>,
) -> Option<HooksBlock> {
    let src = format!("plugin: {name}");
    let mut local = Vec::new();
    let block = match declared {
        // Inline: the manifest carries the event map itself.
        Some(serde_json::Value::Object(_)) => parse_block(declared.unwrap(), &mut local),
        // A path, or a list of them, relative to the plugin root.
        Some(serde_json::Value::String(p)) => {
            read_plugin_hook_file(root, p, &mut local, diags, &src)?
        }
        Some(serde_json::Value::Array(items)) => {
            let mut merged = HooksBlock::new();
            for item in items {
                let Some(p) = item.as_str() else { continue };
                if let Some(b) = read_plugin_hook_file(root, p, &mut local, diags, &src) {
                    for (k, v) in b {
                        merged.entry(k).or_default().extend(v);
                    }
                }
            }
            merged
        }
        Some(_) => {
            diags.push(HookDiagnostic {
                source: src.clone(),
                message: "`hooks` in the manifest must be an object, a path, or a list of paths"
                    .into(),
            });
            HooksBlock::new()
        }
        None => {
            let conventional = root.join("hooks").join("hooks.json");
            if !conventional.exists() {
                return None;
            }
            let text = std::fs::read_to_string(&conventional).ok()?;
            parse_file(&text, &mut local)
        }
    };
    for m in local {
        diags.push(HookDiagnostic {
            source: src.clone(),
            message: m,
        });
    }
    if block.is_empty() { None } else { Some(block) }
}

fn read_plugin_hook_file(
    root: &Path,
    rel: &str,
    local: &mut Vec<String>,
    diags: &mut Vec<HookDiagnostic>,
    src: &str,
) -> Option<HooksBlock> {
    // Containment: a plugin declaring `../../../.ssh/config` gets a diagnostic,
    // not a read. Same rule the MCP loader applies to plugin-relative paths.
    let candidate = root.join(rel);
    let normalized = normalize(&candidate);
    if !normalized.starts_with(normalize(root)) {
        diags.push(HookDiagnostic {
            source: src.to_string(),
            message: format!("`{rel}` escapes the plugin directory — ignored"),
        });
        return None;
    }
    match std::fs::read_to_string(&normalized) {
        Ok(text) => Some(parse_file(&text, local)),
        Err(e) => {
            diags.push(HookDiagnostic {
                source: src.to_string(),
                message: format!("cannot read `{rel}`: {e}"),
            });
            None
        }
    }
}

fn normalize(p: &Path) -> PathBuf {
    let mut out = PathBuf::new();
    for c in p.components() {
        match c {
            std::path::Component::ParentDir => {
                out.pop();
            }
            std::path::Component::CurDir => {}
            other => out.push(other.as_os_str()),
        }
    }
    out
}

/// What the user approves: every command, in order, and nothing cosmetic.
///
/// `statusMessage` is excluded on purpose — changing a spinner label must not
/// invalidate an approval, or nobody will read the next prompt.
pub fn fingerprint(hooks: &[ResolvedHook]) -> String {
    if hooks.is_empty() {
        return String::new();
    }
    let mut h = Sha256::new();
    for hook in hooks {
        h.update(hook.signature().as_bytes());
        h.update(b"\n");
    }
    format!("sha256:{:x}", h.finalize())
}

#[cfg(test)]
mod tests {
    use super::*;

    fn tmp(name: &str) -> PathBuf {
        let p = std::env::temp_dir().join(format!("cc-hooks-disc-{name}-{}", std::process::id()));
        std::fs::remove_dir_all(&p).ok();
        std::fs::create_dir_all(&p).unwrap();
        p
    }

    /// A home directory that is not the workspace. Passing the workspace as
    /// `home` would make `~/.claude/settings.json` and
    /// `<ws>/.claude/settings.json` the same file, which is a fixture bug and
    /// not a behaviour worth asserting.
    fn hm(ws: &Path) -> PathBuf {
        let h = ws.join("_home");
        std::fs::create_dir_all(&h).unwrap();
        h
    }

    fn write(path: &Path, text: &str) {
        std::fs::create_dir_all(path.parent().unwrap()).unwrap();
        std::fs::write(path, text).unwrap();
    }

    fn cfg() -> AgentConfig {
        AgentConfig {
            hooks_enabled: true,
            ..Default::default()
        }
    }

    fn hooks_json(cmd: &str) -> String {
        format!(r#"{{"hooks":{{"Stop":[{{"hooks":[{{"type":"command","command":"{cmd}"}}]}}]}}}}"#)
    }

    #[test]
    fn every_source_contributes_and_none_shadows_another() {
        let ws = tmp("union");
        write(&ws.join(".claude/settings.json"), &hooks_json("project"));
        write(
            &ws.join(".claude/settings.local.json"),
            &hooks_json("local"),
        );
        write(&ws.join(".claudinio.json"), &hooks_json("wsconfig"));
        let mut c = cfg();
        // The app-config source carries the event map itself, not the wrapper.
        let wrapped: serde_json::Value = serde_json::from_str(&hooks_json("app")).unwrap();
        c.hooks = wrapped.get("hooks").cloned();

        let set = resolve_with_home(Some(&ws), &c, Some(&hm(&ws)));
        let cmds: Vec<&str> = set.hooks.iter().map(|h| h.command.as_str()).collect();
        assert!(cmds.contains(&"project"), "{cmds:?}");
        assert!(cmds.contains(&"local"), "{cmds:?}");
        assert!(cmds.contains(&"wsconfig"), "{cmds:?}");
        assert!(cmds.contains(&"app"), "{cmds:?}");
        // Order is broad-to-narrow and stable.
        let pos = |c: &str| cmds.iter().position(|x| *x == c).unwrap();
        assert!(pos("app") < pos("wsconfig"));
        assert!(pos("wsconfig") < pos("project"));
        assert!(pos("project") < pos("local"));
        std::fs::remove_dir_all(&ws).ok();
    }

    #[test]
    fn an_identical_command_from_two_sources_runs_once() {
        let ws = tmp("dedup");
        write(&ws.join(".claude/settings.json"), &hooks_json("same"));
        write(&ws.join(".claudinio.json"), &hooks_json("same"));
        let set = resolve_with_home(Some(&ws), &cfg(), Some(&hm(&ws)));
        let same: Vec<_> = set.hooks.iter().filter(|h| h.command == "same").collect();
        assert_eq!(same.len(), 1);
        assert_eq!(same[0].also_from.len(), 1);
        std::fs::remove_dir_all(&ws).ok();
    }

    #[test]
    fn no_workspace_means_no_hooks() {
        assert!(resolve_with_home(None, &cfg(), None).hooks.is_empty());
    }

    #[test]
    fn the_global_switch_turns_everything_off() {
        let ws = tmp("off");
        write(&ws.join(".claude/settings.json"), &hooks_json("x"));
        let mut c = cfg();
        c.hooks_enabled = false;
        assert!(
            resolve_with_home(Some(&ws), &c, Some(&hm(&ws)))
                .hooks
                .is_empty()
        );
        std::fs::remove_dir_all(&ws).ok();
    }

    #[test]
    fn an_unreadable_settings_file_is_a_diagnostic_not_an_error() {
        let ws = tmp("bad");
        write(&ws.join(".claude/settings.json"), "{ not json");
        write(&ws.join(".claudinio.json"), &hooks_json("good"));
        let set = resolve_with_home(Some(&ws), &cfg(), Some(&hm(&ws)));
        assert_eq!(set.hooks.len(), 1);
        assert_eq!(set.hooks[0].command, "good");
        assert_eq!(set.diagnostics.len(), 1);
        std::fs::remove_dir_all(&ws).ok();
    }

    #[test]
    fn a_settings_file_with_no_hooks_is_not_a_diagnostic() {
        let ws = tmp("nohooks");
        write(
            &ws.join(".claude/settings.json"),
            r#"{"permissions":{"allow":["Bash(ls)"]}}"#,
        );
        let set = resolve_with_home(Some(&ws), &cfg(), Some(&hm(&ws)));
        assert!(set.hooks.is_empty());
        assert!(set.diagnostics.is_empty(), "{:?}", set.diagnostics);
        std::fs::remove_dir_all(&ws).ok();
    }

    #[test]
    fn plugin_hooks_json_is_found_by_convention_when_the_manifest_omits_it() {
        let ws = tmp("plugin-convention");
        let root = ws.join(".claudinio/plugins/brainy");
        write(
            &root.join("plugin.json"),
            r#"{"$schema":"https://agent-plugins.org/schemas/1.0.0/plugin.schema.json","name":"brainy","version":"1.0.0"}"#,
        );
        write(
            &root.join("hooks/hooks.json"),
            r#"{"hooks":{"UserPromptSubmit":[{"hooks":[{"type":"command",
               "command":"${CLAUDE_PLUGIN_ROOT}/hooks/brain-hook.sh","args":["recall"],"timeout":10}]}]}}"#,
        );
        let set = resolve_with_home(Some(&ws), &cfg(), Some(&hm(&ws)));
        assert_eq!(set.hooks.len(), 1);
        let h = &set.hooks[0];
        assert_eq!(h.event, HookEvent::UserPromptSubmit);
        assert_eq!(
            h.command,
            root.join("hooks/brain-hook.sh").display().to_string()
        );
        assert_eq!(h.args, ["recall"]);
        assert_eq!(h.timeout_secs, 10);
        assert!(matches!(h.source, HookSource::Plugin { .. }));
        std::fs::remove_dir_all(&ws).ok();
    }

    #[test]
    fn a_claude_code_plugin_directory_contributes_hooks_only() {
        let ws = tmp("claude-plugin");
        let root = ws.join(".claude/plugins/brain");
        write(
            &root.join(".claude-plugin/plugin.json"),
            r#"{"name":"claudinio-brain","version":"0.1.0"}"#,
        );
        write(&root.join("hooks/hooks.json"), &hooks_json("./run.sh"));
        let set = resolve_with_home(Some(&ws), &cfg(), Some(&hm(&ws)));
        assert_eq!(set.hooks.len(), 1);
        assert_eq!(
            set.hooks[0].source,
            HookSource::Plugin {
                name: "claudinio-brain".into(),
                root: root.display().to_string()
            }
        );
        std::fs::remove_dir_all(&ws).ok();
    }

    #[test]
    fn a_disabled_plugin_contributes_no_hooks() {
        let ws = tmp("plugin-disabled");
        let root = ws.join(".claude/plugins/brain");
        write(
            &root.join(".claude-plugin/plugin.json"),
            r#"{"name":"claudinio-brain"}"#,
        );
        write(&root.join("hooks/hooks.json"), &hooks_json("./run.sh"));
        let mut c = cfg();
        c.plugins.insert(
            "claudinio-brain".into(),
            crate::agent::provider::PluginPrefs { enabled: false },
        );
        assert!(
            resolve_with_home(Some(&ws), &c, Some(&hm(&ws)))
                .hooks
                .is_empty()
        );
        std::fs::remove_dir_all(&ws).ok();
    }

    #[test]
    fn a_manifest_hooks_path_must_stay_inside_the_plugin() {
        let ws = tmp("escape");
        let root = ws.join(".claudinio/plugins/evil");
        write(
            &root.join("plugin.json"),
            r#"{"$schema":"https://agent-plugins.org/schemas/1.0.0/plugin.schema.json","name":"evil","version":"1.0.0","hooks":"../../../elsewhere.json"}"#,
        );
        write(&ws.join("elsewhere.json"), &hooks_json("pwned"));
        let set = resolve_with_home(Some(&ws), &cfg(), Some(&hm(&ws)));
        assert!(set.hooks.is_empty());
        assert!(
            set.diagnostics
                .iter()
                .any(|d| d.message.contains("escapes"))
        );
        std::fs::remove_dir_all(&ws).ok();
    }

    #[test]
    fn placeholders_expand_in_the_command_and_in_every_arg() {
        assert_eq!(expand("${CLAUDE_PROJECT_DIR}/x", "/ws", None), "/ws/x");
        assert_eq!(
            expand("${CLAUDE_PLUGIN_ROOT}/h.sh", "/ws", Some("/p")),
            "/p/h.sh"
        );
        assert_eq!(expand("${CLAUDINIO_PROJECT_DIR}/x", "/ws", None), "/ws/x");
        // Unknown placeholders survive verbatim rather than becoming empty.
        assert_eq!(expand("${NOPE}/x", "/ws", None), "${NOPE}/x");
        assert_eq!(expand("no placeholders", "/ws", None), "no placeholders");
        // A plugin placeholder in a non-plugin hook is left alone, not blanked.
        assert_eq!(
            expand("${CLAUDE_PLUGIN_ROOT}/x", "/ws", None),
            "${CLAUDE_PLUGIN_ROOT}/x"
        );
    }

    #[test]
    fn the_fingerprint_changes_with_the_command_and_not_with_the_status_message() {
        let ws = tmp("fp");
        write(
            &ws.join(".claudinio.json"),
            r#"{"hooks":{"Stop":[{"hooks":[{"command":"a","statusMessage":"one"}]}]}}"#,
        );
        let before = resolve_with_home(Some(&ws), &cfg(), Some(&hm(&ws))).fingerprint;
        write(
            &ws.join(".claudinio.json"),
            r#"{"hooks":{"Stop":[{"hooks":[{"command":"a","statusMessage":"two"}]}]}}"#,
        );
        assert_eq!(
            resolve_with_home(Some(&ws), &cfg(), Some(&hm(&ws))).fingerprint,
            before
        );
        write(
            &ws.join(".claudinio.json"),
            r#"{"hooks":{"Stop":[{"hooks":[{"command":"b","statusMessage":"two"}]}]}}"#,
        );
        assert_ne!(
            resolve_with_home(Some(&ws), &cfg(), Some(&hm(&ws))).fingerprint,
            before
        );
        std::fs::remove_dir_all(&ws).ok();
    }

    #[test]
    fn an_invalid_matcher_is_reported_rather_than_left_silent() {
        let ws = tmp("badmatcher");
        write(
            &ws.join(".claudinio.json"),
            r#"{"hooks":{"PreToolUse":[{"matcher":"Edit(","hooks":[{"command":"a"}]}]}}"#,
        );
        let set = resolve_with_home(Some(&ws), &cfg(), Some(&hm(&ws)));
        assert_eq!(set.hooks.len(), 1);
        assert!(set.diagnostics.iter().any(|d| d.message.contains("regex")));
        std::fs::remove_dir_all(&ws).ok();
    }

    #[test]
    fn selection_uses_the_alias_table_and_the_literal_matcher() {
        let ws = tmp("select");
        write(
            &ws.join(".claudinio.json"),
            r#"{"hooks":{
                 "PreToolUse":[{"matcher":"Edit|Write","hooks":[{"command":"guard"}]}],
                 "PreCompact":[{"matcher":"manual","hooks":[{"command":"flush"}]}]}}"#,
        );
        let set = resolve_with_home(Some(&ws), &cfg(), Some(&hm(&ws)));
        assert_eq!(set.select_tool(HookEvent::PreToolUse, "edit_file").len(), 1);
        assert_eq!(set.select_tool(HookEvent::PreToolUse, "bash").len(), 0);
        assert_eq!(set.select_literal(HookEvent::PreCompact, "manual").len(), 1);
        assert_eq!(set.select_literal(HookEvent::PreCompact, "auto").len(), 0);
        std::fs::remove_dir_all(&ws).ok();
    }
}
