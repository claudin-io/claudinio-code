use crate::agent::session::AgentEvent;
use futures::StreamExt;
use serde::{Deserialize, Serialize};
use serde_json::Value;
use std::fmt;
use std::path::Path;
use std::sync::atomic::{AtomicBool, Ordering};
use tauri::ipc::Channel;

pub mod catalog;
pub mod openai;

const ANTHROPIC_VERSION: &str = "2023-06-01";

/// Sentinel prefix for budget-exhausted errors. The frontend keys off it to
/// swap the retry error bar for an upgrade banner, and the retry loop
/// (`is_retryable_error` in session.rs) treats it as non-retryable.
pub const BUDGET_EXCEEDED_MARKER: &str = "BUDGET_EXCEEDED::";

/// Extract `error.message` from an Anthropic/LiteLLM-style error body; if
/// contiver "budget" (case-insensitive), é um estouro de budget do plano.
/// A API retorna HTTP 500 com corpo
/// `{"error":{"message":"Claudinio: Budget exceeded for window '1h'. ..."}}`.
fn budget_exceeded_message(body: &str) -> Option<String> {
    let v: serde_json::Value = serde_json::from_str(body).ok()?;
    let msg = v.get("error")?.get("message")?.as_str()?;
    if msg.to_lowercase().contains("budget") {
        Some(msg.to_string())
    } else {
        None
    }
}

/// Max time to wait for the *next* SSE chunk before treating the connection
/// as dead. Resets on every chunk received, so a long-but-healthy stream
/// (many minutes of steady deltas) is never killed by this — only a stalled
/// connection (e.g. the socket surviving sleep/network-change with no more
/// bytes coming) is.
const STREAM_IDLE_TIMEOUT: std::time::Duration = std::time::Duration::from_secs(90);

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct AgentConfig {
    pub base_url: String,
    pub api_key: String,
    /// Legacy single model field — kept for backward compat with old config.json.
    /// New configs use brain_model + builder_model.
    #[serde(default)]
    pub model: String,
    /// Model used in Brain (planning) mode.
    #[serde(default = "default_claudius")]
    pub brain_model: String,
    /// Model used in Builder (execution) mode.
    #[serde(default = "default_claudinio")]
    pub builder_model: String,
    /// Max tool-call rounds for the main agent loop. None = infinite.
    pub max_rounds: Option<usize>,
    /// Max tool-call rounds for subagents. None = infinite.
    pub sub_max_rounds: Option<usize>,
    /// YOLO mode: auto-approve all tool calls except those in yolo_blacklist.
    #[serde(default)]
    pub yolo_mode: bool,
    /// Tool names that still require approval even when yolo_mode is on.
    #[serde(default)]
    pub yolo_blacklist: Vec<String>,
    /// Base URL for account/app-bridge services (login exchange, websearch).
    /// Distinct from `base_url`, which is the `/v1/*` inference endpoint.
    #[serde(default = "default_services_url")]
    pub services_url: String,
    /// Login the active api_key is associated with, if it was obtained via
    /// `login_with_claudinio` rather than pasted manually.
    #[serde(default)]
    pub account_login: Option<String>,
    /// Subscription tier of the linked account, as reported at login time.
    #[serde(default)]
    pub account_tier: Option<String>,
    /// Max golden-loop cycles before the run stops with "max_golden_cycles".
    /// None falls back to the built-in default (5); Some(0) disables the loop.
    #[serde(default)]
    pub max_golden_cycles: Option<usize>,
    /// Max consecutive cycles without golden-task progress before the run
    /// stops with "golden_stalled". None falls back to the default (2).
    #[serde(default)]
    pub max_golden_stalls: Option<usize>,
    /// Custom path for saving plans, relative to workspace root.
    /// None = use default (.claudinio/plans).
    #[serde(default)]
    pub plan_save_path: Option<String>,
    /// Opaque per-device id (salted SHA-256 of the OS install GUID), sent at
    /// login to gate the once-per-device app-install promo. Cached here so it's
    /// stable across runs. See `agent::install_id`.
    #[serde(default)]
    pub install_id: Option<String>,
    /// Random uuid used as the hash source when the OS install GUID can't be
    /// read, so the fallback `install_id` is itself stable. Never transmitted.
    #[serde(default)]
    pub install_fallback_seed: Option<String>,
    /// Max number of subagents that can run in parallel in a single
    /// spawn_agents call. None = default (4). Clamped to 1-8 at the
    /// set_config boundary and at every point of use.
    #[serde(default)]
    pub max_parallel_agents: Option<usize>,
    /// Override base URL for LLM inference calls only (stream_message,
    /// classify_turn_completion, one_shot). When set, `/v1/messages` is sent
    /// here instead of `base_url`. Does NOT affect login, websearch, or
    /// list_models.
    #[serde(default)]
    pub override_base_url: Option<String>,
    /// Override API key for LLM inference calls only. When set, used instead
    /// of `api_key` for `/v1/messages` requests. Does NOT affect login,
    /// websearch, or list_models.
    #[serde(default)]
    pub override_api_key: Option<String>,
    /// Configured MCP (Model Context Protocol) servers, keyed by server name —
    /// matches the `{ "mcp": { "name": { "type": ..., ... } } }` shape used by
    /// other MCP-aware tools. Global servers live here; workspace-level
    /// `.claudinio.json` servers are merged in on top (see
    /// `merge_workspace_config`), overriding a global entry of the same name.
    #[serde(default)]
    pub mcp: std::collections::HashMap<String, McpServerEntry>,
    /// Prevent the system from sleeping while an agent session is actively
    /// running (display can still turn off). See commands::power.
    #[serde(default = "default_true")]
    pub keep_awake: bool,
    /// Whether the code-intelligence subsystem (FTS5 index, semantic search,
    /// LSP integration) is enabled. Set false to disable when not needed.
    #[serde(default = "default_true")]
    pub code_intel_enabled: bool,
    /// Preferred IDE for code-related actions. When set, used for operations
    /// like "open file in editor". None = auto-detect.
    #[serde(default)]
    pub preferred_ide: Option<String>,
    /// Context size (tokens) at which a running session hands off to a fresh
    /// linked session via a model-written handoff document. None = default
    /// (120k). Clamped to 120k-256k (the Settings slider range).
    #[serde(default)]
    pub handoff_context_tokens: Option<u64>,
    /// When true, automatically `git add` + `git commit` the plan file after the
    /// final write_plan call (with Low-Level Design) or when exiting Brain mode.
    #[serde(default = "default_true")]
    pub auto_commit_plan: bool,
    /// Thinking effort level: "low" | "medium" | "high" | "xhigh" | "max".
    /// Mapped to an Anthropic `thinking.budget_tokens` value on each
    /// stream_message request; the claudin_router buckets it upstream.
    #[serde(default = "default_thinking_effort")]
    pub thinking_effort: String,
    /// External LLM providers connected by the user (OpenRouter, models.dev
    /// catalog entries), keyed by provider id. A model id of the form
    /// "<provider_id>/<model>" routes to the matching entry; anything else
    /// goes to Claudinio. base_url/protocol/pricing are snapshotted at
    /// connect time so inference never depends on the catalog being reachable.
    #[serde(default)]
    pub providers: std::collections::HashMap<String, ProviderEntry>,
}

/// A connected external provider. Credentials follow the existing plaintext
/// config.json precedent (same as `api_key`).
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct ProviderEntry {
    pub api_key: String,
    pub base_url: String,
    /// Wire protocol: "openai" (chat/completions) or "anthropic" (/v1/messages).
    #[serde(default = "default_openai_protocol")]
    pub protocol: String,
    /// Models the user chose to expose in the pickers; empty = all.
    #[serde(default)]
    pub enabled_models: Vec<String>,
    /// Display name from the catalog (e.g. "OpenRouter", "DeepSeek").
    #[serde(default)]
    pub label: Option<String>,
    /// (input, output) USD per million tokens, snapshotted from models.dev at
    /// connect time, keyed by wire model id. Used for local cost estimation
    /// when the provider doesn't return cost itself.
    #[serde(default)]
    pub model_pricing: std::collections::HashMap<String, (f64, f64)>,
    /// Max output tokens per model, snapshotted from models.dev `limit.output`.
    /// Clamps the request max_tokens so smaller models don't 400.
    #[serde(default)]
    pub model_output_limits: std::collections::HashMap<String, u32>,
}

fn default_openai_protocol() -> String {
    "openai".into()
}

/// Wire protocol a request is spoken in. Decided per request from the model id
/// (the "Strategy" resolved at model-selection time).
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Protocol {
    Anthropic,
    OpenAiChat,
}

/// Everything a single LLM request needs to know about where and how to talk:
/// resolved from `(AgentConfig, model id)` at the top of each entry point.
#[derive(Debug, Clone)]
pub struct ResolvedProvider {
    pub protocol: Protocol,
    pub base_url: String,
    pub api_key: String,
    /// Wire model id (provider prefix stripped for external providers).
    pub model: String,
    pub provider_id: String,
    /// (input, output) USD per Mtok for local cost estimation.
    pub pricing: Option<(f64, f64)>,
    pub max_output_tokens: Option<u32>,
}

impl ResolvedProvider {
    /// Claudinio-only behaviors (budget-exceeded marker, proxy cost fields)
    /// must not leak onto external providers.
    pub fn is_claudinio(&self) -> bool {
        self.provider_id == "claudinio"
    }
}

impl AgentConfig {
    /// True when the session uses claudinio's own API key (not a BYOK override)
    /// and that key is present. This gates subscriber-only features like web_search.
    /// If override_api_key is set to the same value as api_key (or not set at all),
    /// it's still a Claudinio account — only a truly different override key is BYOK.
    pub fn is_claudinio_account(&self) -> bool {
        if self.api_key.is_empty() {
            return false;
        }
        // override_api_key being set to the same value as api_key is NOT real BYOK
        // Only a truly different override key disqualifies the account
        match self.override_api_key.as_deref() {
            None => true,
            Some(k) => k == self.api_key,
        }
    }

    /// The effective context-handoff threshold in tokens. The
    /// `CLAUDINIO_HANDOFF_TOKENS` env var overrides everything UNCLAMPED —
    /// the slider floor (120k) would otherwise make the mechanism untestable
    /// end-to-end.
    pub fn effective_handoff_threshold(&self) -> u64 {
        if let Ok(v) = std::env::var("CLAUDINIO_HANDOFF_TOKENS")
            && let Ok(n) = v.trim().parse::<u64>()
        {
            return n;
        }
        self.handoff_context_tokens
            .unwrap_or(120_000)
            .clamp(120_000, 256_000)
    }
}

/// How to reach an MCP server: a locally spawned process talking JSON-RPC
/// over stdio, or a remote server speaking Streamable HTTP/SSE.
#[derive(Debug, Clone, Serialize, Deserialize)]
#[serde(tag = "type", rename_all = "lowercase")]
pub enum McpTransportConfig {
    Stdio {
        command: String,
        #[serde(default)]
        args: Vec<String>,
        #[serde(default)]
        env: std::collections::HashMap<String, String>,
    },
    Remote {
        url: String,
        #[serde(default)]
        headers: std::collections::HashMap<String, String>,
    },
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct McpServerEntry {
    #[serde(flatten)]
    pub transport: McpTransportConfig,
    #[serde(default = "default_true")]
    pub enabled: bool,
}

fn default_true() -> bool {
    true
}

fn default_thinking_effort() -> String {
    "medium".into()
}

fn default_claudinio() -> String {
    "claudinio".into()
}

fn default_claudius() -> String {
    "claudius".into()
}

fn default_services_url() -> String {
    "https://claudin.io".into()
}

impl Default for AgentConfig {
    fn default() -> Self {
        Self {
            base_url: "https://api.claudin.io".into(),
            api_key: String::new(),
            model: "claudinio".into(),
            brain_model: "claudius".into(),
            builder_model: "claudinio".into(),
            max_rounds: None,
            sub_max_rounds: None,
            yolo_mode: false,
            yolo_blacklist: Vec::new(),
            services_url: default_services_url(),
            account_login: None,
            account_tier: None,
            max_golden_cycles: None,
            max_golden_stalls: None,
            plan_save_path: None,
            install_id: None,
            install_fallback_seed: None,
            max_parallel_agents: None,
            override_base_url: None,
            override_api_key: None,
            mcp: std::collections::HashMap::new(),
            keep_awake: true,
            code_intel_enabled: true,
            preferred_ide: None,
            handoff_context_tokens: None,
            auto_commit_plan: true,
            thinking_effort: default_thinking_effort(),
            providers: std::collections::HashMap::new(),
        }
    }
}

impl AgentConfig {
    /// Resolve which model to use for a given session mode.
    pub fn model_for_mode(&self, mode: &str) -> &str {
        match mode {
            "brain" | "pensador" => &self.brain_model,
            _ => &self.builder_model,
        }
    }

    /// Anthropic `thinking.budget_tokens` for the configured effort level.
    /// Values align with the claudin_router bucket ceilings (low ≤4096,
    /// medium ≤8192, high ≤16384, xhigh ≤24576, max >24576). "max" is 30_000
    /// rather than 32_768 because budget_tokens must stay below the
    /// stream_message max_tokens (32_000).
    pub fn thinking_budget_tokens(&self) -> u32 {
        match self.thinking_effort.as_str() {
            "low" => 4_096,
            "high" => 16_384,
            "xhigh" => 24_576,
            "max" => 30_000,
            _ => 8_192,
        }
    }

    /// Resolve which provider/protocol/credentials serve a model id.
    /// A "<provider_id>/<rest>" prefix matching a connected provider routes
    /// there (split at the FIRST slash only — OpenRouter model ids contain
    /// slashes themselves, e.g. "openrouter/openai/gpt-4o-mini" → provider
    /// "openrouter", wire model "openai/gpt-4o-mini"). Anything else — no
    /// slash, or an unknown prefix — is the unchanged Claudinio path,
    /// preserving the override_base_url/override_api_key BYOK precedence.
    pub fn resolve_provider(&self, model: &str) -> ResolvedProvider {
        if let Some((prefix, rest)) = model.split_once('/')
            && let Some(entry) = self.providers.get(prefix)
        {
            let protocol = if entry.protocol == "anthropic" {
                Protocol::Anthropic
            } else {
                Protocol::OpenAiChat
            };
            let mut base_url = entry.base_url.trim_end_matches('/').to_string();
            // The Anthropic client appends "/v1/messages", but models.dev
            // base URLs already end in "/v1" — strip it so external
            // Anthropic-protocol entries don't produce "/v1/v1/messages".
            if protocol == Protocol::Anthropic
                && let Some(stripped) = base_url.strip_suffix("/v1")
            {
                base_url = stripped.to_string();
            }
            return ResolvedProvider {
                protocol,
                base_url,
                api_key: entry.api_key.clone(),
                model: rest.to_string(),
                provider_id: prefix.to_string(),
                pricing: entry.model_pricing.get(rest).copied(),
                max_output_tokens: entry.model_output_limits.get(rest).copied(),
            };
        }
        ResolvedProvider {
            protocol: Protocol::Anthropic,
            base_url: self
                .override_base_url
                .clone()
                .unwrap_or_else(|| self.base_url.clone()),
            api_key: self
                .override_api_key
                .clone()
                .unwrap_or_else(|| self.api_key.clone()),
            model: model.to_string(),
            provider_id: "claudinio".into(),
            pricing: None,
            max_output_tokens: None,
        }
    }
}

pub fn config_path() -> Result<std::path::PathBuf, String> {
    let dir = dirs::config_dir()
        .ok_or("no config dir")?
        .join("claudinio-code");
    std::fs::create_dir_all(&dir).map_err(|e| format!("create config dir: {e}"))?;
    Ok(dir.join("config.json"))
}

/// Load AgentConfig from the config file, migrating old configs that only have
/// a single `model` field to the new `brain_model` + `builder_model`.
pub fn load_config() -> AgentConfig {
    let path = match config_path() {
        Ok(p) => p,
        Err(_) => return AgentConfig::default(),
    };
    let s = match std::fs::read_to_string(&path) {
        Ok(s) => s,
        Err(_) => return AgentConfig::default(),
    };
    let mut cfg: serde_json::Value = match serde_json::from_str(&s) {
        Ok(v) => v,
        Err(_) => return AgentConfig::default(),
    };
    // Migration: if the old `model` field exists but `brain_model`/`builder_model`
    // don't, seed both from `model`.
    if cfg.get("brain_model").is_none() || cfg.get("builder_model").is_none() {
        let legacy = cfg
            .get("model")
            .and_then(|v| v.as_str())
            .unwrap_or("claudinio")
            .to_string();
        if cfg.get("brain_model").is_none() {
            cfg["brain_model"] = serde_json::json!(legacy);
        }
        if cfg.get("builder_model").is_none() {
            cfg["builder_model"] = serde_json::json!(legacy);
        }
    }
    serde_json::from_value(cfg).unwrap_or_default()
}

pub fn save_config(config: &AgentConfig) {
    if let Ok(path) = config_path()
        && let Ok(json) = serde_json::to_string_pretty(config)
    {
        let _ = std::fs::write(path, json);
    }
}

/// Ensure `cfg.install_id` is populated (generating and caching it, plus the
/// random fallback seed, on first call) and return it. Does NOT persist — the
/// caller saves via `save_config` after this so the write happens once.
pub fn ensure_install_id(cfg: &mut AgentConfig) -> String {
    if cfg.install_fallback_seed.is_none() {
        cfg.install_fallback_seed = Some(uuid::Uuid::new_v4().to_string());
    }
    if cfg.install_id.is_none() {
        let seed = cfg.install_fallback_seed.as_deref().unwrap_or("");
        cfg.install_id = Some(crate::agent::install_id::compute_install_id(seed));
    }
    cfg.install_id.clone().unwrap_or_default()
}

/// Read the workspace-level config from `<workspace_root>/.claudinio.json`.
/// Returns `None` if the file doesn't exist, is unreadable, or has invalid JSON.
pub fn read_workspace_config(workspace_root: &str) -> Option<Value> {
    let path = Path::new(workspace_root).join(".claudinio.json");
    let s = std::fs::read_to_string(&path).ok()?;
    serde_json::from_str(&s).ok()
}

/// Merge workspace config values into an AgentConfig.
/// Workspace values OVERRIDE the local config for these fields ONLY:
/// plan_save_path, brain_model, builder_model, max_rounds, sub_max_rounds,
/// yolo_mode, yolo_blacklist, max_parallel_agents.
/// Fields like api_key, base_url, account_*, max_golden_*, services_url
/// are NEVER read from workspace config.
pub fn merge_workspace_config(cfg: &mut AgentConfig, ws: &Value) {
    let obj = match ws.as_object() {
        Some(o) => o,
        None => return,
    };
    if let Some(v) = obj.get("plan_save_path") {
        if v.is_null() {
            cfg.plan_save_path = None;
        } else if let Some(s) = v.as_str() {
            cfg.plan_save_path = Some(s.to_string());
        }
    }
    if let Some(v) = obj.get("brain_model").and_then(|v| v.as_str()) {
        cfg.brain_model = v.to_string();
    }
    if let Some(v) = obj.get("builder_model").and_then(|v| v.as_str()) {
        cfg.builder_model = v.to_string();
    }
    if let Some(v) = obj.get("max_rounds") {
        cfg.max_rounds = v.as_u64().map(|n| n as usize);
    }
    if let Some(v) = obj.get("sub_max_rounds") {
        cfg.sub_max_rounds = v.as_u64().map(|n| n as usize);
    }
    if let Some(v) = obj.get("max_parallel_agents") {
        cfg.max_parallel_agents = v.as_u64().map(|n| n as usize);
    }
    if let Some(v) = obj.get("yolo_mode").and_then(|v| v.as_bool()) {
        cfg.yolo_mode = v;
    }
    if let Some(v) = obj.get("yolo_blacklist").and_then(|v| v.as_array()) {
        cfg.yolo_blacklist = v
            .iter()
            .filter_map(|item| item.as_str().map(|s| s.to_string()))
            .collect();
    }
    if let Some(v) = obj.get("mcp")
        && let Ok(ws_servers) =
            serde_json::from_value::<std::collections::HashMap<String, McpServerEntry>>(v.clone())
    {
        for (name, entry) in ws_servers {
            cfg.mcp.insert(name, entry);
        }
    }
    if let Some(v) = obj.get("handoff_context_tokens") {
        cfg.handoff_context_tokens = v.as_u64();
    }
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct Message {
    pub role: String,
    pub content: Vec<ContentBlock>,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
#[serde(untagged)]
pub enum ContentBlock {
    Text {
        #[serde(rename = "type")]
        type_: String,
        text: String,
    },
    Image {
        #[serde(rename = "type")]
        type_: String,
        source: ImageSource,
    },
    ToolUse {
        #[serde(rename = "type")]
        type_: String,
        id: String,
        name: String,
        input: Value,
    },
    ToolResult {
        #[serde(rename = "type")]
        type_: String,
        tool_use_id: String,
        content: String,
    },
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct ImageSource {
    #[serde(rename = "type")]
    pub type_: String,
    #[serde(rename = "media_type")]
    pub media_type: String,
    pub data: String,
    /// Pixel dimensions (non-serialized, used by token estimator).
    /// Populated after compress_image returns, so the estimator can use
    /// w*h/750 instead of base64.len()/3 (which overestimates ~50x).
    #[serde(skip)]
    pub width: u32,
    #[serde(skip)]
    pub height: u32,
}

impl ContentBlock {
    pub fn text(text: impl Into<String>) -> Self {
        ContentBlock::Text {
            type_: "text".into(),
            text: text.into(),
        }
    }

    pub fn image(
        media_type: impl Into<String>,
        data: impl Into<String>,
        width: u32,
        height: u32,
    ) -> Self {
        ContentBlock::Image {
            type_: "image".into(),
            source: ImageSource {
                type_: "base64".into(),
                media_type: media_type.into(),
                data: data.into(),
                width,
                height,
            },
        }
    }

    pub fn tool_use(id: impl Into<String>, name: impl Into<String>, input: Value) -> Self {
        ContentBlock::ToolUse {
            type_: "tool_use".into(),
            id: id.into(),
            name: name.into(),
            input,
        }
    }

    pub fn tool_result(tool_use_id: impl Into<String>, content: impl Into<String>) -> Self {
        ContentBlock::ToolResult {
            type_: "tool_result".into(),
            tool_use_id: tool_use_id.into(),
            content: content.into(),
        }
    }

    /// Extract text from a Text block. Used in tests.
    #[allow(dead_code)]
    pub fn get_text(&self) -> Option<&str> {
        match self {
            ContentBlock::Text { text, .. } => Some(text.as_str()),
            _ => None,
        }
    }
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct ToolDescription {
    pub name: String,
    pub description: String,
    pub input_schema: Value,
}

#[derive(Serialize)]
struct RequestBody {
    model: String,
    max_tokens: u32,
    stream: bool,
    messages: Vec<Message>,
    #[serde(skip_serializing_if = "Option::is_none")]
    tools: Option<Vec<ToolDescription>>,
    system: Option<String>,
    #[serde(skip_serializing_if = "Option::is_none")]
    thinking: Option<ThinkingConfig>,
}

/// Anthropic extended-thinking block: `{"type":"enabled","budget_tokens":N}`.
/// The claudin_router proxy buckets budget_tokens into a reasoning-effort
/// level per routed deployment tier.
#[derive(Serialize)]
struct ThinkingConfig {
    #[serde(rename = "type")]
    kind: &'static str,
    budget_tokens: u32,
}

#[derive(Debug, Clone, Serialize, Deserialize, Default)]
pub struct Usage {
    pub input_tokens: u32,
    pub output_tokens: u32,
    #[serde(default)]
    pub cache_read_input_tokens: u32,
    /// Cost in USD as returned by the provider (if supported), otherwise None.
    /// We calculate an estimate when the provider does not supply it.
    #[serde(default)]
    pub cost: Option<f64>,
    /// Markup cost breakdown in USD, as reported by the litellm proxy's
    /// cost_injector middleware. None when the provider doesn't inject it
    /// (unpriced model, older proxy deploy) — falls back to local estimate.
    #[serde(default)]
    pub cost_input: Option<f64>,
    #[serde(default)]
    pub cost_output: Option<f64>,
    #[serde(default)]
    pub cost_cache_read: Option<f64>,
}

/// Merge usage fields from an SSE event into the accumulated usage.
/// Anthropic reports input_tokens in message_start and output_tokens in
/// message_delta; other providers (claudin.io) send everything in
/// message_delta with zeros in message_start — so merge, never replace,
/// and only take token counts that are > 0.
fn merge_usage(usage: &mut Option<Usage>, value: &Value) {
    let Some(obj) = value.as_object() else { return };
    let u = usage.get_or_insert_with(Usage::default);
    if let Some(v) = obj.get("input_tokens").and_then(|v| v.as_u64())
        && v > 0
    {
        u.input_tokens = v as u32;
    }
    if let Some(v) = obj.get("output_tokens").and_then(|v| v.as_u64())
        && v > 0
    {
        u.output_tokens = v as u32;
    }
    if let Some(v) = obj.get("cache_read_input_tokens").and_then(|v| v.as_u64())
        && v > 0
    {
        u.cache_read_input_tokens = v as u32;
    }
    if let Some(v) = obj.get("cost").and_then(|v| v.as_f64()) {
        u.cost = Some(v);
    }
    if let Some(v) = obj.get("cost_input").and_then(|v| v.as_f64()) {
        u.cost_input = Some(v);
    }
    if let Some(v) = obj.get("cost_output").and_then(|v| v.as_f64()) {
        u.cost_output = Some(v);
    }
    if let Some(v) = obj.get("cost_cache_read").and_then(|v| v.as_f64()) {
        u.cost_cache_read = Some(v);
    }
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct StreamOutput {
    pub text_deltas: Vec<String>,
    pub tool_uses: Vec<Value>,
    pub stop_reason: Option<String>,
    pub usage: Option<Usage>,
    pub interrupted: bool,
}

/// System prompt for the completion judge. Deliberately demands a single
/// sentinel token so the caller never has to parse natural language — this is
/// what keeps the mechanism language-agnostic (the judged text may be in any
/// language the UI supports, but the answer is always CONTINUE / DONE).
const COMPLETION_JUDGE_SYSTEM: &str = "You are a strict classifier inside an agentic coding harness. \
You are given the assistant's final message of a turn that ended WITHOUT calling any tool. \
Decide whether the turn is genuinely complete, or whether the assistant merely announced or \
implied an immediate next step (e.g. said it would ask the user a question, spawn subagents, \
read a file, run a command, or make an edit) but stopped before actually doing it. \
Answer with EXACTLY ONE WORD and nothing else: \
CONTINUE if the assistant promised or implied a next action it did not take, \
or if the message is clearly cut off mid-thought; \
DONE if the message is a complete, self-contained reply that needs no further action right now. \
The message may be in any language; your one-word answer must still be CONTINUE or DONE.";

/// Non-streaming, single-shot classification call used by the workflow loop to
/// decide whether a terminal `end_turn` is a real finish or a dangling promise.
/// Returns the model's raw reply (expected to contain CONTINUE or DONE); the
/// caller maps it to a verdict. Kept small (tiny max_tokens, no tools) so it is
/// cheap enough to run on every terminal turn.
pub async fn classify_turn_completion(
    config: &AgentConfig,
    model: &str,
    assistant_text: &str,
) -> Result<String, String> {
    let rp = config.resolve_provider(model);
    if rp.protocol == Protocol::OpenAiChat {
        return openai::complete(
            &rp,
            COMPLETION_JUDGE_SYSTEM,
            &format!("Assistant's final message of the turn:\n\n{assistant_text}"),
            1024,
            crate::net_activity::NetSource::LlmClassify,
        )
        .await;
    }
    let _net_guard =
        crate::net_activity::NetGuard::begin(crate::net_activity::NetSource::LlmClassify, model);
    let client = crate::http::default_client_builder()
        .connect_timeout(std::time::Duration::from_secs(15))
        .timeout(std::time::Duration::from_secs(45))
        .build()
        .map_err(|e| format!("failed to build HTTP client: {e}"))?;
    let body = RequestBody {
        model: rp.model.clone(),
        // The Brain model is a thinking model: it burns output tokens on a
        // reasoning block before the one-word verdict. Too small a cap (e.g. 8)
        // is entirely consumed by thinking and yields an empty answer, so leave
        // ample headroom — the answer itself is still a single token.
        max_tokens: 1024,
        stream: false,
        messages: vec![Message {
            role: "user".into(),
            content: vec![ContentBlock::text(format!(
                "Assistant's final message of the turn:\n\n{assistant_text}"
            ))],
        }],
        tools: None,
        system: Some(COMPLETION_JUDGE_SYSTEM.to_string()),
        // No thinking block: budget_tokens must be < max_tokens (1024 here),
        // and forced thinking only adds cost/latency to a one-token verdict.
        thinking: None,
    };
    let url = format!("{}/v1/messages", rp.base_url.trim_end_matches('/'));
    let response = client
        .post(&url)
        .header("Content-Type", "application/json")
        .header("x-api-key", &rp.api_key)
        .header("anthropic-version", ANTHROPIC_VERSION)
        .json(&body)
        .send()
        .await
        .map_err(|e| format!("request failed: {e}"))?;
    _net_guard.set_status(response.status().as_u16());
    if !response.status().is_success() {
        let status = response.status();
        let body = response.text().await.unwrap_or_default();
        return match budget_exceeded_message(&body).filter(|_| rp.is_claudinio()) {
            Some(m) => Err(format!("{BUDGET_EXCEEDED_MARKER}{m}")),
            None => Err(format!("API error: HTTP {status}")),
        };
    }
    let json: Value = response
        .json()
        .await
        .map_err(|e| format!("failed to parse judge response: {e}"))?;
    // Anthropic-shaped response: { "content": [ { "type": "text", "text": "..." } ] }
    let reply = json
        .get("content")
        .and_then(|c| c.as_array())
        .map(|blocks| {
            blocks
                .iter()
                .filter_map(|b| b.get("text").and_then(|t| t.as_str()))
                .collect::<Vec<_>>()
                .join("")
        })
        .unwrap_or_default();
    Ok(reply)
}

/// Single-shot, non-streaming completion: send one `system` + one `user`
/// message and return the concatenated text of the reply. No tools, no
/// history. Primarily an eval/test utility for grading prompts against the
/// live model, but generic enough for any one-off classification call.
#[allow(dead_code)]
pub async fn one_shot(
    config: &AgentConfig,
    model: &str,
    system: &str,
    user: &str,
    max_tokens: u32,
) -> Result<String, String> {
    let rp = config.resolve_provider(model);
    if rp.protocol == Protocol::OpenAiChat {
        return openai::complete(
            &rp,
            system,
            user,
            max_tokens,
            crate::net_activity::NetSource::LlmOneShot,
        )
        .await;
    }
    let _net_guard =
        crate::net_activity::NetGuard::begin(crate::net_activity::NetSource::LlmOneShot, model);
    let client = crate::http::default_client_builder()
        .connect_timeout(std::time::Duration::from_secs(15))
        .timeout(std::time::Duration::from_secs(90))
        .build()
        .map_err(|e| format!("failed to build HTTP client: {e}"))?;
    let body = RequestBody {
        model: rp.model.clone(),
        max_tokens,
        stream: false,
        messages: vec![Message {
            role: "user".into(),
            content: vec![ContentBlock::text(user.to_string())],
        }],
        tools: None,
        system: Some(system.to_string()),
        thinking: None,
    };
    let url = format!("{}/v1/messages", rp.base_url.trim_end_matches('/'));
    let response = client
        .post(&url)
        .header("Content-Type", "application/json")
        .header("x-api-key", &rp.api_key)
        .header("anthropic-version", ANTHROPIC_VERSION)
        .json(&body)
        .send()
        .await
        .map_err(|e| format!("request failed: {e}"))?;
    _net_guard.set_status(response.status().as_u16());
    if !response.status().is_success() {
        let status = response.status();
        let body = response.text().await.unwrap_or_default();
        return match budget_exceeded_message(&body).filter(|_| rp.is_claudinio()) {
            Some(m) => Err(format!("{BUDGET_EXCEEDED_MARKER}{m}")),
            None => Err(format!("API error: HTTP {status}")),
        };
    }
    let json: Value = response
        .json()
        .await
        .map_err(|e| format!("failed to parse response: {e}"))?;
    let reply = json
        .get("content")
        .and_then(|c| c.as_array())
        .map(|blocks| {
            blocks
                .iter()
                .filter_map(|b| b.get("text").and_then(|t| t.as_str()))
                .collect::<Vec<_>>()
                .join("")
        })
        .unwrap_or_default();
    Ok(reply)
}

/// Minimum interval between live `TextDelta` snapshots sent to the frontend,
/// so a fast model doesn't flood the IPC channel with one event per token.
const TEXT_DELTA_THROTTLE: std::time::Duration = std::time::Duration::from_millis(80);

/// Sends an `AgentEvent::TextDelta` snapshot of `assistant_text` if enabled,
/// the text grew since the last send, and the throttle interval has elapsed.
fn maybe_emit_text_delta(
    emit: bool,
    event_tx: &Channel<AgentEvent>,
    assistant_text: &str,
    last_sent_len: &mut usize,
    last_flush: &mut std::time::Instant,
) {
    if !emit || assistant_text.len() == *last_sent_len || last_flush.elapsed() < TEXT_DELTA_THROTTLE
    {
        return;
    }
    let _ = event_tx.send(AgentEvent::TextDelta {
        text: assistant_text.to_string(),
    });
    *last_sent_len = assistant_text.len();
    *last_flush = std::time::Instant::now();
}

#[allow(clippy::too_many_arguments)]
pub async fn stream_message(
    config: &AgentConfig,
    model: &str,
    messages: &[Message],
    tools: &[ToolDescription],
    system: Option<&str>,
    event_tx: &Channel<AgentEvent>,
    session_id: &str,
    assistant_text: &mut String,
    interrupt: &AtomicBool,
    emit_text_deltas: bool,
    // Shown in the network indicator: who opened this stream (model + mode,
    // subagent goal, …). Not sent to the API.
    net_detail: &str,
) -> Result<StreamOutput, String> {
    let rp = config.resolve_provider(model);
    if rp.protocol == Protocol::OpenAiChat {
        return openai::stream_message(
            &rp,
            config,
            messages,
            tools,
            system,
            event_tx,
            session_id,
            assistant_text,
            interrupt,
            emit_text_deltas,
            net_detail,
        )
        .await;
    }
    let client = crate::http::default_client_builder()
        .connect_timeout(std::time::Duration::from_secs(15))
        .build()
        .map_err(|e| format!("failed to build HTTP client: {e}"))?;
    // Large tasks (thinking + a whole-file edit in one tool call) easily
    // blow past 8k output tokens; a truncated stream ends the turn with
    // the work half-done. 32k fits every current Claude model's cap; external
    // Anthropic-protocol models with a smaller cap get clamped to it.
    let max_tokens = rp.max_output_tokens.map_or(32_000, |m| m.min(32_000));
    // budget_tokens must stay below max_tokens (and ≥ the API minimum of
    // 1024), so a clamped max_tokens also clamps the thinking budget.
    let budget_tokens = config
        .thinking_budget_tokens()
        .min(max_tokens.saturating_sub(2_000))
        .max(1_024);
    let body = RequestBody {
        model: rp.model.clone(),
        max_tokens,
        stream: true,
        messages: messages.to_vec(),
        tools: if tools.is_empty() {
            None
        } else {
            Some(tools.to_vec())
        },
        system: system.map(|s| s.to_string()),
        thinking: Some(ThinkingConfig {
            kind: "enabled",
            budget_tokens,
        }),
    };

    let url = format!("{}/v1/messages", rp.base_url.trim_end_matches('/'));

    let response = client
        .post(&url)
        .header("Content-Type", "application/json")
        .header("x-api-key", &rp.api_key)
        .header("anthropic-version", ANTHROPIC_VERSION)
        .json(&body)
        .send()
        .await
        .map_err(|e| format!("request failed: {e}"))?;

    let status = response.status();
    if !status.is_success() {
        let body = response.text().await.unwrap_or_default();
        let err_msg = if status.as_u16() == 401 {
            "Unauthorized — check your API key".into()
        } else if let Some(m) = budget_exceeded_message(&body).filter(|_| rp.is_claudinio()) {
            format!("{BUDGET_EXCEEDED_MARKER}{m}")
        } else {
            format!("API error: HTTP {status}")
        };
        return Err(err_msg);
    }

    // Registered only after a successful response: from here the SSE stream
    // stays open for the whole turn, which is exactly what the network
    // indicator must surface. Dropped on every exit path (RAII).
    let net_guard =
        crate::net_activity::NetGuard::begin(crate::net_activity::NetSource::LlmStream, net_detail);
    net_guard.set_status(response.status().as_u16());

    let mut dump = if std::env::var("CLAUDINIO_DEBUG_DUMP").is_ok() {
        let dump_path =
            std::path::Path::new("/tmp").join(format!("claudinio_api_dump_{session_id}.txt"));
        std::fs::File::create(&dump_path)
            .map(std::io::BufWriter::new)
            .ok()
    } else {
        None
    };
    if let Some(ref mut f) = dump {
        use std::io::Write;
        let _ = writeln!(f, "--- API RAW DUMP session={session_id} ---");
    }

    let mut stream = response.bytes_stream();
    let mut buf = String::new();

    let mut current_event = String::new();
    let mut current_data = String::new();

    let mut text_deltas: Vec<String> = Vec::new();
    let mut thinking_text: String = String::new();
    let mut tool_uses: Vec<Value> = Vec::new();
    let mut tool_inputs: std::collections::HashMap<usize, String> =
        std::collections::HashMap::new();
    let mut stop_reason: Option<String> = None;
    let mut usage: Option<Usage> = None;

    let mut last_sent_len: usize = 0;
    let mut last_flush = std::time::Instant::now() - TEXT_DELTA_THROTTLE;

    loop {
        // Check interrupt before processing each chunk
        if interrupt.load(Ordering::SeqCst) {
            // Drop the stream to cancel the HTTP request eagerly
            drop(stream);
            return Ok(StreamOutput {
                text_deltas,
                tool_uses,
                stop_reason: Some("interrupted".into()),
                usage: None,
                interrupted: true,
            });
        }

        let chunk_result = match tokio::time::timeout(STREAM_IDLE_TIMEOUT, stream.next()).await {
            Ok(Some(r)) => r,
            Ok(None) => break,
            Err(_) => {
                return Err("stream error: no data received for 90s, connection stalled".into());
            }
        };

        let chunk = chunk_result.map_err(|e| format!("stream error: {e}"))?;
        net_guard.add_bytes(chunk.len() as u64);
        let chunk_str = String::from_utf8_lossy(&chunk);
        if let Some(ref mut f) = dump {
            use std::io::Write;
            let _ = writeln!(f, "[CHUNK {} bytes]", chunk.len());
            let _ = writeln!(f, "{}", chunk_str);
        }
        buf.push_str(&chunk_str);

        while let Some(newline) = buf.find('\n') {
            let line = buf[..newline].trim_end_matches('\r').to_string();
            buf = buf[newline + 1..].to_string();

            if line.is_empty() {
                if !current_event.is_empty() {
                    if let Some(ref mut f) = dump {
                        use std::io::Write;
                        let _ = writeln!(f, "[EVENT] {} | DATA: {}", current_event, current_data);
                    }
                    process_line(
                        &current_event,
                        &current_data,
                        event_tx,
                        session_id,
                        assistant_text,
                        &mut thinking_text,
                        &mut text_deltas,
                        &mut tool_uses,
                        &mut tool_inputs,
                        &mut stop_reason,
                        &mut usage,
                    )?;
                    maybe_emit_text_delta(
                        emit_text_deltas,
                        event_tx,
                        assistant_text,
                        &mut last_sent_len,
                        &mut last_flush,
                    );
                    current_event.clear();
                    current_data.clear();
                }
                continue;
            }

            if let Some(data) = line.strip_prefix("event: ") {
                current_event = data.to_string();
            } else if let Some(data) = line.strip_prefix("data: ") {
                current_data = data.to_string();
            } else if let Some(ref mut f) = dump {
                use std::io::Write;
                let _ = writeln!(f, "[RAW] {}", line);
            }
        }
    }

    if !current_event.is_empty() {
        process_line(
            &current_event,
            &current_data,
            event_tx,
            session_id,
            assistant_text,
            &mut thinking_text,
            &mut text_deltas,
            &mut tool_uses,
            &mut tool_inputs,
            &mut stop_reason,
            &mut usage,
        )?;
    }

    // Unconditional catch-up flush: TextStep/Done follow immediately after and
    // carry the authoritative text, so this just lets the live preview close
    // the gap before it's replaced, ignoring the throttle interval.
    if emit_text_deltas && assistant_text.len() != last_sent_len {
        let _ = event_tx.send(AgentEvent::TextDelta {
            text: assistant_text.clone(),
        });
    }

    if !buf.is_empty()
        && let Ok(full) = serde_json::from_str::<Value>(&buf)
    {
        if let Some(blocks) = full.get("content").and_then(|c| c.as_array()) {
            for block in blocks {
                if block.get("type").and_then(|t| t.as_str()) == Some("tool_use")
                    && let Some(input) = block.get("input")
                    && !input.is_null()
                {
                    let id = block.get("id").and_then(|i| i.as_str()).unwrap_or("");
                    let _name = block.get("name").and_then(|n| n.as_str()).unwrap_or("");
                    if let Some(existing) = tool_uses
                        .iter_mut()
                        .find(|t| t.get("id").and_then(|i| i.as_str()) == Some(id))
                    {
                        if let Some(obj) = existing.as_object_mut() {
                            obj.insert("input".into(), input.clone());
                        }
                    } else {
                        tool_uses.push(block.clone());
                    }
                }
            }
        }
        if let Some(reason) = full.get("stop_reason").and_then(|r| r.as_str()) {
            stop_reason = Some(reason.to_string());
        }
        if let Some(u) = full.get("usage") {
            merge_usage(&mut usage, u);
        }
    }

    if !buf.is_empty()
        && let Some(ref mut f) = dump
    {
        use std::io::Write;
        let _ = writeln!(f, "--- REMAINING BUF ---\n{}", buf);
    }

    if let Some(ref mut f) = dump {
        use std::io::Write;
        let _ = writeln!(
            f,
            "--- END --- tool_uses={} text_deltas={}",
            tool_uses.len(),
            text_deltas.len()
        );
    }

    // Blocks still in tool_inputs never got their content_block_stop — the
    // stream was cut mid-input (e.g. max_tokens). Salvage what parses, drop
    // the rest so a half-written tool call never executes.
    for (idx, accumulated) in tool_inputs.drain() {
        match serde_json::from_str::<Value>(&accumulated) {
            Ok(parsed) => {
                if let Some(tool) = tool_uses.iter_mut().find(|t| {
                    t.get("_index").and_then(|i| i.as_u64()).map(|i| i as usize) == Some(idx)
                }) && let Some(obj) = tool.as_object_mut()
                {
                    obj.insert("input".into(), parsed);
                }
            }
            Err(_) if !accumulated.is_empty() => {
                tool_uses.retain(|t| {
                    t.get("_index").and_then(|i| i.as_u64()).map(|i| i as usize) != Some(idx)
                });
            }
            Err(_) => {}
        }
    }

    Ok(StreamOutput {
        text_deltas,
        tool_uses,
        stop_reason,
        usage,
        interrupted: false,
    })
}

#[allow(clippy::too_many_arguments)]
fn process_line(
    event_type: &str,
    data: &str,
    event_tx: &Channel<AgentEvent>,
    _session_id: &str,
    assistant_text: &mut String,
    thinking_text: &mut String,
    text_deltas: &mut Vec<String>,
    tool_uses: &mut Vec<Value>,
    tool_inputs: &mut std::collections::HashMap<usize, String>,
    stop_reason: &mut Option<String>,
    usage: &mut Option<Usage>,
) -> Result<(), String> {
    let value: Value =
        serde_json::from_str(data).map_err(|e| format!("json parse: {e} data: {data:.100}"))?;

    let index = value
        .get("index")
        .and_then(|v| v.as_u64())
        .map(|i| i as usize);

    match event_type {
        "content_block_start" => {
            if let Some(block) = value.get("content_block")
                && block.get("type").and_then(|t| t.as_str()) == Some("tool_use")
            {
                let mut tool = block.clone();
                if let Some(idx) = index {
                    if let Some(obj) = tool.as_object_mut() {
                        obj.insert("_index".into(), serde_json::json!(idx));
                    }
                    tool_inputs.insert(idx, String::new());
                }
                tool_uses.push(tool);
            }
        }
        "content_block_delta" => {
            if let Some(delta) = value.get("delta") {
                match delta.get("type").and_then(|t| t.as_str()) {
                    Some("text_delta") => {
                        if let Some(text) = delta.get("text").and_then(|t| t.as_str())
                            && !text.is_empty()
                        {
                            text_deltas.push(text.to_string());
                            assistant_text.push_str(text);
                        }
                    }
                    Some("thinking_delta") => {
                        if let Some(thinking) = delta.get("thinking").and_then(|t| t.as_str())
                            && !thinking.is_empty()
                        {
                            thinking_text.push_str(thinking);
                            let _ = event_tx.send(AgentEvent::Thinking(thinking_text.clone()));
                        }
                    }
                    Some("input_json_delta") => {
                        if let Some(idx) = index
                            && let Some(partial) = delta.get("partial_json")
                        {
                            let fragment = match partial {
                                Value::String(s) => s.clone(),
                                other => serde_json::to_string(other).unwrap_or_default(),
                            };
                            tool_inputs.entry(idx).or_default().push_str(&fragment);
                        }
                    }
                    _ => {}
                }
            }
        }
        "content_block_stop" => {
            if let Some(idx) = index
                && let Some(accumulated) = tool_inputs.remove(&idx)
            {
                match serde_json::from_str::<Value>(&accumulated) {
                    Ok(parsed) => {
                        if let Some(tool) = tool_uses.iter_mut().find(|t| {
                            t.get("_index").and_then(|i| i.as_u64()).map(|i| i as usize)
                                == Some(idx)
                        }) && let Some(obj) = tool.as_object_mut()
                        {
                            obj.insert("input".into(), parsed);
                        }
                    }
                    // Empty accumulation is a no-arg tool: keep the {}
                    // input from content_block_start. Non-empty JSON that
                    // fails to parse means the stream was cut mid-input
                    // (max_tokens) — drop the block instead of running the
                    // tool with a bogus empty input.
                    Err(_) if !accumulated.is_empty() => {
                        tool_uses.retain(|t| {
                            t.get("_index").and_then(|i| i.as_u64()).map(|i| i as usize)
                                != Some(idx)
                        });
                    }
                    Err(_) => {}
                }
            }
        }
        "message_delta" => {
            if let Some(delta) = value.get("delta")
                && let Some(reason) = delta.get("stop_reason").and_then(|r| r.as_str())
            {
                *stop_reason = Some(reason.to_string());
            }
            if let Some(u) = value.get("usage") {
                merge_usage(usage, u);
            }
        }
        "message_start" => {
            if let Some(u) = value.pointer("/message/usage") {
                merge_usage(usage, u);
            }
        }
        "ping" => {}
        "error" => {
            // Parse SSE error events like:
            // {"type":"error","error":{"type":"...","message":"..."}}
            // The API accepted the request (200) but failed mid-stream
            // (e.g. context overflow, overloaded). Propagate as an Err so the
            // retry loop and error bar in the UI handle it.
            if let Some(err_obj) = value.get("error") {
                let msg = err_obj
                    .get("message")
                    .and_then(|m| m.as_str())
                    .unwrap_or("unknown error");
                let err_type = err_obj
                    .get("type")
                    .and_then(|t| t.as_str())
                    .unwrap_or("unknown");
                return Err(format!("API error: {err_type} — {msg}"));
            }
            return Err("API error: unknown".into());
        }
        _ => {}
    }

    Ok(())
}

impl fmt::Display for StreamOutput {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        let text: String = self.text_deltas.join("");
        write!(f, "{text}")
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use serde_json::json;
    use tauri::ipc::InvokeResponseBody;

    #[test]
    fn test_merge_workspace_config_mcp_servers_appends_and_overrides_by_name() {
        let mut cfg = AgentConfig::default();
        cfg.mcp.insert(
            "global-server".into(),
            McpServerEntry {
                transport: McpTransportConfig::Stdio {
                    command: "global-cmd".into(),
                    args: vec![],
                    env: Default::default(),
                },
                enabled: true,
            },
        );

        let ws = json!({
            "mcp": {
                "global-server": { "type": "stdio", "command": "overridden-cmd", "args": [] },
                "workspace-only": { "type": "remote", "url": "http://localhost:9000/mcp" }
            }
        });
        merge_workspace_config(&mut cfg, &ws);

        assert_eq!(cfg.mcp.len(), 2);
        let global = cfg.mcp.get("global-server").unwrap();
        match &global.transport {
            McpTransportConfig::Stdio { command, .. } => assert_eq!(command, "overridden-cmd"),
            _ => panic!("expected stdio transport"),
        }
        let workspace_only = cfg.mcp.get("workspace-only").unwrap();
        match &workspace_only.transport {
            McpTransportConfig::Remote { url, .. } => assert_eq!(url, "http://localhost:9000/mcp"),
            _ => panic!("expected remote transport"),
        }
    }

    #[test]
    fn test_thinking_budget_tokens_maps_all_levels_and_falls_back() {
        let mut cfg = AgentConfig::default();
        for (level, expected) in [
            ("low", 4_096),
            ("medium", 8_192),
            ("high", 16_384),
            ("xhigh", 24_576),
            ("max", 30_000),
            ("garbage", 8_192),
        ] {
            cfg.thinking_effort = level.into();
            assert_eq!(cfg.thinking_budget_tokens(), expected, "level {level}");
        }
    }

    #[test]
    fn test_config_without_thinking_effort_defaults_to_medium() {
        let cfg: AgentConfig = serde_json::from_str(r#"{"base_url":"x","api_key":"","model":"m"}"#)
            .expect("config without thinking_effort must deserialize");
        assert_eq!(cfg.thinking_effort, "medium");
    }

    #[test]
    fn test_legacy_config_without_providers_deserializes_empty() {
        let cfg: AgentConfig = serde_json::from_str(
            r#"{"base_url":"https://api.claudin.io","api_key":"sk-x","model":"claudinio"}"#,
        )
        .expect("pre-providers config must deserialize");
        assert!(cfg.providers.is_empty());
    }

    fn cfg_with_openrouter() -> AgentConfig {
        let mut cfg = AgentConfig {
            api_key: "sk-claudinio".into(),
            ..Default::default()
        };
        cfg.providers.insert(
            "openrouter".into(),
            ProviderEntry {
                api_key: "sk-or-abc".into(),
                base_url: "https://openrouter.ai/api/v1/".into(),
                protocol: "openai".into(),
                enabled_models: vec![],
                label: Some("OpenRouter".into()),
                model_pricing: [("openai/gpt-4o-mini".to_string(), (0.15, 0.6))]
                    .into_iter()
                    .collect(),
                model_output_limits: [("openai/gpt-4o-mini".to_string(), 16_384u32)]
                    .into_iter()
                    .collect(),
            },
        );
        cfg
    }

    #[test]
    fn test_resolve_provider_unqualified_goes_to_claudinio() {
        let cfg = cfg_with_openrouter();
        let rp = cfg.resolve_provider("claudius");
        assert_eq!(rp.protocol, Protocol::Anthropic);
        assert_eq!(rp.provider_id, "claudinio");
        assert_eq!(rp.model, "claudius");
        assert_eq!(rp.base_url, "https://api.claudin.io");
        assert_eq!(rp.api_key, "sk-claudinio");
    }

    #[test]
    fn test_resolve_provider_splits_first_slash_only() {
        let cfg = cfg_with_openrouter();
        let rp = cfg.resolve_provider("openrouter/openai/gpt-4o-mini");
        assert_eq!(rp.protocol, Protocol::OpenAiChat);
        assert_eq!(rp.provider_id, "openrouter");
        assert_eq!(rp.model, "openai/gpt-4o-mini");
        assert_eq!(rp.base_url, "https://openrouter.ai/api/v1");
        assert_eq!(rp.api_key, "sk-or-abc");
        assert_eq!(rp.pricing, Some((0.15, 0.6)));
        assert_eq!(rp.max_output_tokens, Some(16_384));
        assert!(!rp.is_claudinio());
    }

    #[test]
    fn test_resolve_provider_unknown_prefix_falls_back_to_claudinio() {
        let cfg = cfg_with_openrouter();
        let rp = cfg.resolve_provider("anthropic/claude-sonnet-4");
        assert_eq!(rp.protocol, Protocol::Anthropic);
        assert_eq!(rp.provider_id, "claudinio");
        // model id passes through unchanged on the Claudinio path
        assert_eq!(rp.model, "anthropic/claude-sonnet-4");
    }

    #[test]
    fn test_resolve_provider_preserves_byok_override_precedence() {
        let mut cfg = cfg_with_openrouter();
        cfg.override_base_url = Some("https://my-proxy.example".into());
        cfg.override_api_key = Some("sk-byok".into());
        let rp = cfg.resolve_provider("claudinio");
        assert_eq!(rp.base_url, "https://my-proxy.example");
        assert_eq!(rp.api_key, "sk-byok");
        // qualified external models ignore the BYOK override entirely
        let rp = cfg.resolve_provider("openrouter/openai/gpt-4o-mini");
        assert_eq!(rp.api_key, "sk-or-abc");
    }

    #[test]
    fn test_resolve_provider_anthropic_protocol_entry() {
        let mut cfg = AgentConfig::default();
        cfg.providers.insert(
            "anthropic".into(),
            ProviderEntry {
                api_key: "sk-ant".into(),
                base_url: "https://api.anthropic.com/v1".into(),
                protocol: "anthropic".into(),
                enabled_models: vec![],
                label: Some("Anthropic".into()),
                model_pricing: Default::default(),
                model_output_limits: Default::default(),
            },
        );
        let rp = cfg.resolve_provider("anthropic/claude-sonnet-4-5");
        assert_eq!(rp.protocol, Protocol::Anthropic);
        assert_eq!(rp.provider_id, "anthropic");
        assert_eq!(rp.model, "claude-sonnet-4-5");
        // models.dev base URLs end in /v1; the client appends /v1/messages
        assert_eq!(rp.base_url, "https://api.anthropic.com");
        assert!(!rp.is_claudinio());
    }

    #[test]
    fn test_request_body_serializes_thinking_block_and_omits_when_none() {
        let base = RequestBody {
            model: "claudinio".into(),
            max_tokens: 32_000,
            stream: true,
            messages: vec![],
            tools: None,
            system: None,
            thinking: Some(ThinkingConfig {
                kind: "enabled",
                budget_tokens: 8_192,
            }),
        };
        let v = serde_json::to_value(&base).unwrap();
        assert_eq!(
            v["thinking"],
            json!({"type": "enabled", "budget_tokens": 8192})
        );

        let without = RequestBody {
            model: "claudinio".into(),
            max_tokens: 1024,
            stream: false,
            messages: vec![],
            tools: None,
            system: None,
            thinking: None,
        };
        let v = serde_json::to_value(&without).unwrap();
        assert!(
            v.get("thinking").is_none(),
            "thinking must be omitted when None"
        );
    }

    #[test]
    fn test_budget_exceeded_message_detects_real_body() {
        let body = r#"{"error":{"message":"Claudinio: Budget exceeded for window '1h'. Please check your dashboard for details.","type":"None","param":"None","code":"500"}}"#;
        assert_eq!(
            budget_exceeded_message(body).as_deref(),
            Some(
                "Claudinio: Budget exceeded for window '1h'. Please check your dashboard for details."
            )
        );
    }

    #[test]
    fn test_budget_exceeded_message_ignores_non_budget_errors() {
        let body = r#"{"error":{"message":"Internal Server Error","code":"500"}}"#;
        assert_eq!(budget_exceeded_message(body), None);
        assert_eq!(budget_exceeded_message("not json"), None);
    }

    #[test]
    fn test_usage_merged_across_message_start_and_delta_anthropic_style() {
        // Anthropic: input_tokens in message_start, output_tokens in message_delta
        let mut usage: Option<Usage> = None;
        merge_usage(
            &mut usage,
            &json!({"input_tokens": 1200, "output_tokens": 0, "cache_read_input_tokens": 300}),
        );
        merge_usage(&mut usage, &json!({"output_tokens": 64}));
        let u = usage.unwrap();
        assert_eq!(u.input_tokens, 1200);
        assert_eq!(u.output_tokens, 64);
        assert_eq!(u.cache_read_input_tokens, 300);
        assert_eq!(u.cost, None);
    }

    #[test]
    fn test_usage_merged_claudinio_style_delta_only() {
        // claudin.io: zeros in message_start, full usage in message_delta
        let mut usage: Option<Usage> = None;
        merge_usage(
            &mut usage,
            &json!({"input_tokens": 0, "output_tokens": 0, "cache_read_input_tokens": 0}),
        );
        merge_usage(
            &mut usage,
            &json!({"input_tokens": 15, "output_tokens": 64}),
        );
        let u = usage.unwrap();
        assert_eq!(u.input_tokens, 15);
        assert_eq!(u.output_tokens, 64);
        assert_eq!(u.cache_read_input_tokens, 0);
    }

    #[test]
    fn test_process_line_populates_usage_from_message_start() {
        let chan = Channel::new(|_: InvokeResponseBody| Ok(()));
        let mut text_deltas = Vec::new();
        let mut tool_uses = Vec::new();
        let mut tool_inputs = std::collections::HashMap::new();
        let mut stop_reason: Option<String> = None;
        let mut usage: Option<Usage> = None;
        let mut assistant_text = String::new();
        let mut thinking_text = String::new();

        process_line(
            "message_start",
            r#"{"type":"message_start","message":{"role":"assistant","usage":{"input_tokens":500,"output_tokens":0}}}"#,
            &chan, "s1", &mut assistant_text, &mut thinking_text,
            &mut text_deltas, &mut tool_uses, &mut tool_inputs,
            &mut stop_reason, &mut usage,
        ).unwrap();
        process_line(
            "message_delta",
            r#"{"type":"message_delta","delta":{"stop_reason":"end_turn"},"usage":{"output_tokens":42}}"#,
            &chan, "s1", &mut assistant_text, &mut thinking_text,
            &mut text_deltas, &mut tool_uses, &mut tool_inputs,
            &mut stop_reason, &mut usage,
        ).unwrap();

        let u = usage.unwrap();
        assert_eq!(u.input_tokens, 500);
        assert_eq!(u.output_tokens, 42);
        assert_eq!(stop_reason.as_deref(), Some("end_turn"));
    }

    #[test]
    fn test_input_json_delta_accumulates_tool_args() {
        let chan = Channel::new(|_: InvokeResponseBody| Ok(()));

        let mut text_deltas = Vec::new();
        let mut tool_uses = Vec::new();
        let mut tool_inputs = std::collections::HashMap::new();
        let mut stop_reason: Option<String> = None;
        let mut usage: Option<Usage> = None;
        let mut assistant_text = String::new();
        let mut thinking_text = String::new();
        // Step 1: content_block_start for tool_use with empty input placeholder
        process_line(
            "content_block_start",
            r#"{"type":"content_block_start","index":0,"content_block":{"type":"tool_use","id":"toolu_abc","name":"list_dir","input":{}}}"#,
    &chan, "s1", &mut assistant_text, &mut thinking_text,
        &mut text_deltas, &mut tool_uses, &mut tool_inputs,
            &mut stop_reason, &mut usage,
        ).unwrap();

        assert_eq!(tool_uses.len(), 1);
        assert_eq!(tool_uses[0]["name"], "list_dir");
        assert_eq!(tool_uses[0]["input"], json!({}));

        // Step 2: first input_json_delta fragment
        process_line(
            "content_block_delta",
            r#"{"type":"content_block_delta","index":0,"delta":{"type":"input_json_delta","partial_json":"{\"path\":"}}"#,
    &chan, "s1", &mut assistant_text, &mut thinking_text,
        &mut text_deltas, &mut tool_uses, &mut tool_inputs,
            &mut stop_reason, &mut usage,
        ).unwrap();

        // Step 3: second fragment
        process_line(
            "content_block_delta",
            r#"{"type":"content_block_delta","index":0,"delta":{"type":"input_json_delta","partial_json":" \"/home/user/project\""}}"#,
    &chan, "s1", &mut assistant_text, &mut thinking_text,
        &mut text_deltas, &mut tool_uses, &mut tool_inputs,
            &mut stop_reason, &mut usage,
        ).unwrap();

        // Step 4: third fragment (closing brace)
        process_line(
            "content_block_delta",
            r#"{"type":"content_block_delta","index":0,"delta":{"type":"input_json_delta","partial_json":"}"}}"#,
    &chan, "s1", &mut assistant_text, &mut thinking_text,
        &mut text_deltas, &mut tool_uses, &mut tool_inputs,
            &mut stop_reason, &mut usage,
        ).unwrap();

        // Step 5: content_block_stop — should finalize the input
        process_line(
            "content_block_stop",
            r#"{"type":"content_block_stop","index":0}"#,
            &chan,
            "s1",
            &mut assistant_text,
            &mut thinking_text,
            &mut text_deltas,
            &mut tool_uses,
            &mut tool_inputs,
            &mut stop_reason,
            &mut usage,
        )
        .unwrap();

        // The tool_use must now have the complete parsed input
        assert_eq!(tool_uses[0]["input"], json!({"path": "/home/user/project"}));
    }

    #[test]
    fn test_truncated_tool_input_drops_block() {
        let chan = Channel::new(|_: InvokeResponseBody| Ok(()));

        let mut text_deltas = Vec::new();
        let mut tool_uses = Vec::new();
        let mut tool_inputs = std::collections::HashMap::new();
        let mut stop_reason: Option<String> = None;
        let mut usage: Option<Usage> = None;
        let mut assistant_text = String::new();
        let mut thinking_text = String::new();

        process_line(
            "content_block_start",
            r#"{"type":"content_block_start","index":0,"content_block":{"type":"tool_use","id":"toolu_cut","name":"edit_file","input":{}}}"#,
            &chan, "s1", &mut assistant_text, &mut thinking_text,
            &mut text_deltas, &mut tool_uses, &mut tool_inputs,
            &mut stop_reason, &mut usage,
        ).unwrap();

        // Stream hits max_tokens mid-input: the JSON never closes.
        process_line(
            "content_block_delta",
            r#"{"type":"content_block_delta","index":0,"delta":{"type":"input_json_delta","partial_json":"{\"path\": \"/workspace/Icon.tsx\", \"content\": \"const icons = "}}"#,
            &chan, "s1", &mut assistant_text, &mut thinking_text,
            &mut text_deltas, &mut tool_uses, &mut tool_inputs,
            &mut stop_reason, &mut usage,
        ).unwrap();

        process_line(
            "content_block_stop",
            r#"{"type":"content_block_stop","index":0}"#,
            &chan,
            "s1",
            &mut assistant_text,
            &mut thinking_text,
            &mut text_deltas,
            &mut tool_uses,
            &mut tool_inputs,
            &mut stop_reason,
            &mut usage,
        )
        .unwrap();

        // The half-written tool call must not survive to execution.
        assert!(tool_uses.is_empty());
    }

    #[test]
    fn test_tool_use_with_complete_input_in_start_keeps_it() {
        let chan = Channel::new(|_: InvokeResponseBody| Ok(()));

        let mut text_deltas = Vec::new();
        let mut tool_uses = Vec::new();
        let mut tool_inputs = std::collections::HashMap::new();
        let mut stop_reason: Option<String> = None;
        let mut usage: Option<Usage> = None;
        let mut assistant_text = String::new();
        let mut thinking_text = String::new();

        // Some APIs send the complete input directly in content_block_start
        process_line(
            "content_block_start",
            r#"{"type":"content_block_start","index":0,"content_block":{"type":"tool_use","id":"toolu_def","name":"read_file","input":{"path":"/workspace/main.rs"}}}"#,
    &chan, "s1", &mut assistant_text, &mut thinking_text,
        &mut text_deltas, &mut tool_uses, &mut tool_inputs,
            &mut stop_reason, &mut usage,
        ).unwrap();

        // content_block_stop with no input_json_delta in between
        process_line(
            "content_block_stop",
            r#"{"type":"content_block_stop","index":0}"#,
            &chan,
            "s1",
            &mut assistant_text,
            &mut thinking_text,
            &mut text_deltas,
            &mut tool_uses,
            &mut tool_inputs,
            &mut stop_reason,
            &mut usage,
        )
        .unwrap();

        // Input should still be the original complete value
        assert_eq!(tool_uses[0]["input"], json!({"path": "/workspace/main.rs"}));
    }

    #[test]
    fn test_tool_use_args_deserialize_successfully() {
        let chan = Channel::new(|_: InvokeResponseBody| Ok(()));

        let mut text_deltas = Vec::new();
        let mut tool_uses = Vec::new();
        let mut tool_inputs = std::collections::HashMap::new();
        let mut stop_reason: Option<String> = None;
        let mut usage: Option<Usage> = None;
        let mut assistant_text = String::new();
        let mut thinking_text = String::new();

        // Full streaming sequence
        process_line(
            "content_block_start",
            r#"{"type":"content_block_start","index":0,"content_block":{"type":"tool_use","id":"toolu_ghi","name":"read_file","input":{}}}"#,
    &chan, "s1", &mut assistant_text, &mut thinking_text,
        &mut text_deltas, &mut tool_uses, &mut tool_inputs,
            &mut stop_reason, &mut usage,
        ).unwrap();

        process_line(
            "content_block_delta",
            r#"{"type":"content_block_delta","index":0,"delta":{"type":"input_json_delta","partial_json":"{\"path\":"}}"#,
    &chan, "s1", &mut assistant_text, &mut thinking_text,
        &mut text_deltas, &mut tool_uses, &mut tool_inputs,
            &mut stop_reason, &mut usage,
        ).unwrap();

        process_line(
            "content_block_delta",
            r#"{"type":"content_block_delta","index":0,"delta":{"type":"input_json_delta","partial_json":"\"src/main.ts\""}}"#,
    &chan, "s1", &mut assistant_text, &mut thinking_text,
        &mut text_deltas, &mut tool_uses, &mut tool_inputs,
            &mut stop_reason, &mut usage,
        ).unwrap();

        process_line(
            "content_block_delta",
            r#"{"type":"content_block_delta","index":0,"delta":{"type":"input_json_delta","partial_json":"}"}}"#,
    &chan, "s1", &mut assistant_text, &mut thinking_text,
        &mut text_deltas, &mut tool_uses, &mut tool_inputs,
            &mut stop_reason, &mut usage,
        ).unwrap();

        process_line(
            "content_block_stop",
            r#"{"type":"content_block_stop","index":0}"#,
            &chan,
            "s1",
            &mut assistant_text,
            &mut thinking_text,
            &mut text_deltas,
            &mut tool_uses,
            &mut tool_inputs,
            &mut stop_reason,
            &mut usage,
        )
        .unwrap();

        // Now simulate what session.rs does: deserialize the input
        let input = tool_uses[0].get("input").unwrap();
        let path = input.get("path").and_then(|v| v.as_str()).unwrap();
        assert_eq!(path, "src/main.ts");
    }

    #[test]
    fn test_mixed_text_and_tool_stream() {
        let chan = Channel::new(|_: InvokeResponseBody| Ok(()));

        let mut text_deltas = Vec::new();
        let mut tool_uses = Vec::new();
        let mut tool_inputs = std::collections::HashMap::new();
        let mut stop_reason: Option<String> = None;
        let mut usage: Option<Usage> = None;
        let mut assistant_text = String::new();
        let mut thinking_text = String::new();

        // Text block at index 0
        process_line(
            "content_block_start",
            r#"{"type":"content_block_start","index":0,"content_block":{"type":"text","text":"Let me look at "}}"#,
    &chan, "s1", &mut assistant_text, &mut thinking_text,
        &mut text_deltas, &mut tool_uses, &mut tool_inputs,
            &mut stop_reason, &mut usage,
        ).unwrap();

        // Tool at index 1
        process_line(
            "content_block_start",
            r#"{"type":"content_block_start","index":1,"content_block":{"type":"tool_use","id":"toolu_xyz","name":"list_dir","input":{}}}"#,
    &chan, "s1", &mut assistant_text, &mut thinking_text,
        &mut text_deltas, &mut tool_uses, &mut tool_inputs,
            &mut stop_reason, &mut usage,
        ).unwrap();

        // Text delta
        process_line(
            "content_block_delta",
            r#"{"type":"content_block_delta","index":0,"delta":{"type":"text_delta","text":"the source"}}"#,
    &chan, "s1", &mut assistant_text, &mut thinking_text,
        &mut text_deltas, &mut tool_uses, &mut tool_inputs,
            &mut stop_reason, &mut usage,
        ).unwrap();

        // Tool input delta
        process_line(
            "content_block_delta",
            r#"{"type":"content_block_delta","index":1,"delta":{"type":"input_json_delta","partial_json":"{\"path\":"}}"#,
    &chan, "s1", &mut assistant_text, &mut thinking_text,
        &mut text_deltas, &mut tool_uses, &mut tool_inputs,
            &mut stop_reason, &mut usage,
        ).unwrap();

        // Text block stops (index 0) — should NOT interfere with tool at index 1
        process_line(
            "content_block_stop",
            r#"{"type":"content_block_stop","index":0}"#,
            &chan,
            "s1",
            &mut assistant_text,
            &mut thinking_text,
            &mut text_deltas,
            &mut tool_uses,
            &mut tool_inputs,
            &mut stop_reason,
            &mut usage,
        )
        .unwrap();

        // More tool input
        process_line(
            "content_block_delta",
            r#"{"type":"content_block_delta","index":1,"delta":{"type":"input_json_delta","partial_json":"\"/src\""}}"#,
    &chan, "s1", &mut assistant_text, &mut thinking_text,
        &mut text_deltas, &mut tool_uses, &mut tool_inputs,
            &mut stop_reason, &mut usage,
        ).unwrap();

        // More tool input
        process_line(
            "content_block_delta",
            r#"{"type":"content_block_delta","index":1,"delta":{"type":"input_json_delta","partial_json":"}"}}"#,
    &chan, "s1", &mut assistant_text, &mut thinking_text,
        &mut text_deltas, &mut tool_uses, &mut tool_inputs,
            &mut stop_reason, &mut usage,
        ).unwrap();

        // Tool stops
        process_line(
            "content_block_stop",
            r#"{"type":"content_block_stop","index":1}"#,
            &chan,
            "s1",
            &mut assistant_text,
            &mut thinking_text,
            &mut text_deltas,
            &mut tool_uses,
            &mut tool_inputs,
            &mut stop_reason,
            &mut usage,
        )
        .unwrap();

        // Text: content_block_start for text isn't accumulated, only deltas are
        assert_eq!(assistant_text, "the source");

        // Tool should have complete input
        assert_eq!(tool_uses.len(), 1);
        assert_eq!(tool_uses[0]["input"], json!({"path": "/src"}));

        // Text block stop at index 0 must NOT clear tool input at index 1
        assert!(
            !tool_inputs.contains_key(&1),
            "tool_inputs should be empty after content_block_stop"
        );
    }
}
