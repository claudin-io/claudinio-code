//! Tauri commands for external LLM providers: the OpenRouter OAuth PKCE
//! connect flow, the models.dev catalog, and generic connect/disconnect for
//! any OpenAI-compatible (or Anthropic-compatible) provider from that
//! catalog — plus custom providers, which the user types in by hand (a
//! localhost server, a company LiteLLM proxy) and which have no catalog entry
//! behind them. Claudinio's own login stays in `commands::auth`.

use crate::agent::provider::{ANTHROPIC_VERSION, ProviderEntry, catalog, openai, save_config};
use crate::commands::auth::{random_hex, wait_for_callback};
use crate::state::AppState;
use base64::Engine;
use serde::{Deserialize, Serialize};
use serde_json::Value;
use sha2::{Digest, Sha256};
use tauri::State;
use tauri_plugin_opener::OpenerExt;
use tokio::net::TcpListener;

pub const OPENROUTER_ID: &str = "openrouter";
const OPENROUTER_BASE_URL: &str = "https://openrouter.ai/api/v1";

/// One picker group per provider: Claudinio first with unqualified model ids,
/// then each connected provider with "<provider_id>/<model>" qualified ids.
#[derive(Serialize)]
pub struct ModelGroup {
    #[serde(rename = "providerId")]
    pub provider_id: String,
    #[serde(rename = "providerName")]
    pub provider_name: String,
    pub models: Vec<String>,
    /// Human-readable name per model id, for ids that do not read as names.
    /// A locally served model is keyed by a content hash — right for a
    /// directory, unusable in a picker — so it ships its label alongside.
    /// Empty for providers whose ids are already their names.
    #[serde(default, skip_serializing_if = "std::collections::HashMap::is_empty")]
    pub labels: std::collections::HashMap<String, String>,
}

/// OpenRouter OAuth PKCE connect: browser consent → loopback callback →
/// key exchange → stored `ProviderEntry`. Unlike the Claudinio flow there is
/// no state param round-trip; PKCE itself protects the exchange (a forged
/// callback code is useless without our in-memory verifier). Note the
/// challenge is standard base64url(SHA256), not the hex encoding the
/// Claudinio flow uses.
#[tauri::command]
pub async fn openrouter_login(
    app: tauri::AppHandle,
    state: State<'_, AppState>,
) -> Result<Vec<String>, String> {
    let listener = TcpListener::bind(("127.0.0.1", 0))
        .await
        .map_err(|e| format!("failed to bind local callback port: {e}"))?;
    let port = listener
        .local_addr()
        .map_err(|e| format!("failed to read callback port: {e}"))?
        .port();

    let verifier = random_hex(32);
    let challenge = base64::engine::general_purpose::URL_SAFE_NO_PAD
        .encode(Sha256::digest(verifier.as_bytes()));

    let authorize_url = format!(
        "https://openrouter.ai/auth?callback_url=http%3A%2F%2F127.0.0.1%3A{port}%2Fcallback&code_challenge={challenge}&code_challenge_method=S256"
    );
    app.opener()
        .open_url(authorize_url, None::<&str>)
        .map_err(|e| format!("failed to open browser: {e}"))?;

    let cancel = std::sync::Arc::new(tokio::sync::Notify::new());
    *state.oauth_cancel.lock().await = Some(cancel.clone());
    let code = tokio::select! {
        code = wait_for_callback(listener, None) => {
            *state.oauth_cancel.lock().await = None;
            code?
        }
        _ = cancel.notified() => {
            *state.oauth_cancel.lock().await = None;
            return Err("login cancelled".into());
        }
    };

    let _net_guard = crate::net_activity::NetGuard::begin(
        crate::net_activity::NetSource::Auth,
        "OpenRouter key exchange",
    );
    let client = crate::http::default_client();
    let resp = client
        .post("https://openrouter.ai/api/v1/auth/keys")
        .header("Content-Type", "application/json")
        .json(&serde_json::json!({
            "code": code,
            "code_verifier": verifier,
            "code_challenge_method": "S256",
        }))
        .send()
        .await
        .map_err(|e| format!("OpenRouter key exchange failed: {e}"))?;
    let status = resp.status();
    _net_guard.set_status(status.as_u16());
    if !status.is_success() {
        let body = resp.text().await.unwrap_or_default();
        return Err(format!(
            "OpenRouter key exchange failed (HTTP {status}): {body}"
        ));
    }
    let parsed: Value = resp
        .json()
        .await
        .map_err(|e| format!("invalid OpenRouter exchange response: {e}"))?;
    let key = parsed
        .get("key")
        .and_then(|k| k.as_str())
        .ok_or("OpenRouter exchange response missing key")?
        .to_string();

    // Pricing/output-limit snapshots from the models.dev catalog are
    // best-effort — OpenRouter reports cost natively on each response, so a
    // missing catalog only loses the max_tokens clamp.
    let (model_pricing, model_output_limits) = match catalog::fetch_catalog(false).await {
        Ok(cat) => catalog::find_provider(&cat, OPENROUTER_ID)
            .map(catalog::model_snapshots)
            .unwrap_or_default(),
        Err(_) => Default::default(),
    };

    {
        let mut cfg = state.config.lock().await;
        cfg.providers.insert(
            OPENROUTER_ID.to_string(),
            ProviderEntry {
                api_key: key,
                base_url: OPENROUTER_BASE_URL.to_string(),
                protocol: "openai".into(),
                enabled_models: Vec::new(),
                label: Some("OpenRouter".into()),
                model_pricing,
                model_output_limits,
                custom: false,
                custom_models: Vec::new(),
            },
        );
        save_config(&cfg);
    }

    list_openrouter_models_live().await
}

/// Abort a pending `openrouter_login` stuck waiting for the browser callback
/// (user closed the consent page). No-op when no login is in flight.
#[tauri::command]
pub async fn openrouter_login_cancel(state: State<'_, AppState>) -> Result<(), String> {
    if let Some(cancel) = state.oauth_cancel.lock().await.take() {
        cancel.notify_waiters();
    }
    Ok(())
}

/// Live model listing from OpenRouter ({data:[{id}]} shape).
async fn list_openrouter_models_live() -> Result<Vec<String>, String> {
    let _net_guard = crate::net_activity::NetGuard::begin(
        crate::net_activity::NetSource::ListModels,
        "openrouter /models",
    );
    let client = crate::http::default_client();
    let resp = client
        .get("https://openrouter.ai/api/v1/models")
        .send()
        .await
        .map_err(|e| format!("OpenRouter model list failed: {e}"))?;
    _net_guard.set_status(resp.status().as_u16());
    if !resp.status().is_success() {
        return Err(format!(
            "OpenRouter model list failed: HTTP {}",
            resp.status()
        ));
    }
    let body: Value = resp
        .json()
        .await
        .map_err(|e| format!("invalid OpenRouter model list: {e}"))?;
    let models = body
        .get("data")
        .and_then(|d| d.as_array())
        .map(|arr| {
            arr.iter()
                .filter_map(|m| m.get("id").and_then(|i| i.as_str()).map(String::from))
                .collect::<Vec<_>>()
        })
        .unwrap_or_default();
    Ok(models)
}

/// Trimmed models.dev catalog for the provider modal.
#[tauri::command]
pub async fn fetch_provider_catalog(force: Option<bool>) -> Result<Value, String> {
    catalog::fetch_catalog(force.unwrap_or(false)).await
}

/// Connect a catalog provider with a pasted API key. Base URL and protocol
/// default from the catalog (overridable), pricing/output limits are
/// snapshotted per model, and the key is sanity-checked against
/// `GET {base}/models` when the provider speaks OpenAI protocol — a 401/403
/// rejects the key; any other failure (404, network) accepts it unvalidated
/// since plenty of compatible backends don't expose /models.
#[tauri::command]
pub async fn connect_provider(
    provider_id: String,
    api_key: String,
    base_url: Option<String>,
    state: State<'_, AppState>,
) -> Result<Vec<String>, String> {
    if api_key.trim().is_empty() {
        return Err("API key is required".into());
    }
    // A custom provider that already owns this id would be silently replaced
    // by the catalog one — and every model slot pointing at it re-routed.
    if state
        .config
        .lock()
        .await
        .providers
        .get(&provider_id)
        .is_some_and(|p| p.custom)
    {
        return Err(format!(
            "a custom provider already uses the id \"{provider_id}\" — remove it first"
        ));
    }
    let cat = catalog::fetch_catalog(false).await?;
    let provider = catalog::find_provider(&cat, &provider_id)
        .ok_or_else(|| format!("unknown provider: {provider_id}"))?;

    let catalog_api = provider
        .get("api")
        .and_then(|a| a.as_str())
        .unwrap_or_default()
        .to_string();
    let base = base_url
        .filter(|u| !u.trim().is_empty())
        .unwrap_or(catalog_api);
    if base.is_empty() {
        return Err("provider has no API base URL".into());
    }
    let protocol = provider
        .get("protocol")
        .and_then(|p| p.as_str())
        .unwrap_or("openai")
        .to_string();
    let label = provider
        .get("name")
        .and_then(|n| n.as_str())
        .map(String::from);
    let (model_pricing, model_output_limits) = catalog::model_snapshots(provider);
    let models: Vec<String> = model_pricing
        .keys()
        .cloned()
        .chain(model_output_limits.keys().cloned())
        .collect::<std::collections::BTreeSet<_>>()
        .into_iter()
        .collect();
    let models = if models.is_empty() {
        provider
            .get("models")
            .and_then(|m| m.as_array())
            .map(|arr| {
                arr.iter()
                    .filter_map(|m| m.get("id").and_then(|i| i.as_str()).map(String::from))
                    .collect()
            })
            .unwrap_or_default()
    } else {
        models
    };

    if protocol == "openai" {
        let _net_guard = crate::net_activity::NetGuard::begin(
            crate::net_activity::NetSource::Auth,
            format!("{provider_id} key validation"),
        );
        let client = crate::http::default_client();
        let url = format!("{}/models", base.trim_end_matches('/'));
        if let Ok(resp) = client
            .get(&url)
            .header("Authorization", format!("Bearer {}", api_key.trim()))
            .send()
            .await
        {
            _net_guard.set_status(resp.status().as_u16());
            let code = resp.status().as_u16();
            if code == 401 || code == 403 {
                return Err("Authentication failed — check your API key".into());
            }
        }
    }

    {
        let mut cfg = state.config.lock().await;
        cfg.providers.insert(
            provider_id.clone(),
            ProviderEntry {
                api_key: api_key.trim().to_string(),
                base_url: base,
                protocol,
                enabled_models: Vec::new(),
                label,
                model_pricing,
                model_output_limits,
                custom: false,
                custom_models: Vec::new(),
            },
        );
        save_config(&cfg);
    }

    Ok(models)
}

/// Ids a custom provider can never take: each already means something to
/// `resolve_provider` or to the pickers.
const RESERVED_PROVIDER_IDS: [&str; 3] =
    ["claudinio", crate::llama::LOCAL_PROVIDER_ID, OPENROUTER_ID];

/// What the "custom provider" form sends. `provider_id` is set when editing an
/// existing one; `api_key` left out (or blank) on an edit keeps the saved key,
/// because the form never receives it back.
#[derive(Debug, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct CustomProviderInput {
    #[serde(default)]
    pub provider_id: Option<String>,
    pub name: String,
    pub base_url: String,
    pub protocol: String,
    #[serde(default)]
    pub api_key: Option<String>,
    #[serde(default)]
    pub models: Vec<String>,
}

#[derive(Debug, Serialize)]
pub struct CustomProviderSaved {
    #[serde(rename = "providerId")]
    pub provider_id: String,
    pub models: Vec<String>,
}

/// Why a custom endpoint did not hand over a model list. Kept apart because
/// the two are handled differently: a rejected key always fails a save, while
/// an endpoint that simply has no `/models` is fine once the ids are typed in.
#[derive(Debug, PartialEq)]
enum ProbeError {
    Auth,
    Unavailable(String),
}

/// Provider id from a display name: the id is the prefix of every model id
/// ("litellm/gpt-4o"), so it is lowercase ASCII with dashes and — above all —
/// no slash, which is where `resolve_provider` splits.
fn slugify_provider_id(name: &str) -> String {
    let mut out = String::new();
    for c in name.trim().chars() {
        if c.is_ascii_alphanumeric() {
            out.push(c.to_ascii_lowercase());
        } else if !out.is_empty() && !out.ends_with('-') {
            out.push('-');
        }
    }
    let out = out.trim_end_matches('-');
    if out.is_empty() {
        "custom".into()
    } else {
        out.to_string()
    }
}

/// `slugify_provider_id`, moved out of the way of every id already spoken for
/// ("openai" → "openai-2" when the catalog has an "openai").
fn unique_provider_id(name: &str, taken: &std::collections::HashSet<String>) -> String {
    let base = slugify_provider_id(name);
    if !taken.contains(&base) {
        return base;
    }
    (2u32..)
        .map(|n| format!("{base}-{n}"))
        .find(|candidate| !taken.contains(candidate))
        .expect("an unbounded counter always finds a free id")
}

/// Trim, drop blanks and duplicates, keep the order the user gave.
fn normalize_model_ids(models: Vec<String>) -> Vec<String> {
    let mut seen = std::collections::HashSet::new();
    models
        .into_iter()
        .map(|m| m.trim().to_string())
        .filter(|m| !m.is_empty() && seen.insert(m.clone()))
        .collect()
}

fn normalize_protocol(protocol: &str) -> Result<&'static str, String> {
    match protocol.trim() {
        "openai" => Ok("openai"),
        "anthropic" => Ok("anthropic"),
        other => Err(format!("unknown protocol: {other}")),
    }
}

/// A custom base URL is whatever the user typed, so it is checked before it is
/// stored: a missing scheme ("localhost:4000") otherwise only surfaces as an
/// opaque request error on the first chat message.
fn normalize_base_url(raw: &str) -> Result<String, String> {
    let trimmed = raw.trim().trim_end_matches('/');
    if trimmed.is_empty() {
        return Err("Base URL is required".into());
    }
    let invalid =
        || "Base URL must be a full http(s) URL, for example http://localhost:4000/v1".to_string();
    let parsed = reqwest::Url::parse(trimmed).map_err(|_| invalid())?;
    if !matches!(parsed.scheme(), "http" | "https") || parsed.host_str().is_none() {
        return Err(invalid());
    }
    Ok(trimmed.to_string())
}

/// Where a custom endpoint lists its models. The Anthropic client appends
/// "/v1/..." itself (see `resolve_provider`), so a base URL given with or
/// without a trailing "/v1" lands on the same place.
fn models_url(base_url: &str, protocol: &str) -> String {
    let base = base_url.trim_end_matches('/');
    if protocol == "anthropic" {
        let root = base.strip_suffix("/v1").unwrap_or(base);
        format!("{root}/v1/models")
    } else {
        format!("{base}/models")
    }
}

/// Model ids out of a `/models` response. `{"data":[{"id"}]}` is what OpenAI,
/// Anthropic, LiteLLM, Ollama and llama.cpp all answer; a bare array and plain
/// string items are accepted too, since "compatible" servers vary.
fn parse_models_response(body: &Value) -> Vec<String> {
    let items = body
        .get("data")
        .or_else(|| body.get("models"))
        .unwrap_or(body);
    let ids = items
        .as_array()
        .map(|arr| {
            arr.iter()
                .filter_map(|m| {
                    m.as_str()
                        .or_else(|| m.get("id").and_then(|i| i.as_str()))
                        .map(String::from)
                })
                .collect()
        })
        .unwrap_or_default();
    normalize_model_ids(ids)
}

/// Ask a custom endpoint for its models. Short timeouts: this runs while the
/// user watches a form, against hosts that are often a laptop's own loopback
/// or a VPN address that is not there today.
async fn fetch_custom_models(
    base_url: &str,
    protocol: &str,
    api_key: &str,
) -> Result<Vec<String>, ProbeError> {
    let url = models_url(base_url, protocol);
    let _net_guard = crate::net_activity::NetGuard::begin(
        crate::net_activity::NetSource::ListModels,
        format!("custom provider {url}"),
    );
    let client = crate::http::default_client_builder()
        .connect_timeout(std::time::Duration::from_secs(5))
        .timeout(std::time::Duration::from_secs(15))
        .build()
        .map_err(|e| ProbeError::Unavailable(format!("failed to build HTTP client: {e}")))?;
    let request = client.get(&url);
    let request = if protocol == "anthropic" {
        let request = request.header("anthropic-version", ANTHROPIC_VERSION);
        if api_key.is_empty() {
            request
        } else {
            request.header("x-api-key", api_key)
        }
    } else {
        openai::with_bearer(request, api_key)
    };
    let resp = request
        .send()
        .await
        .map_err(|e| ProbeError::Unavailable(format!("request failed: {e}")))?;
    let status = resp.status();
    _net_guard.set_status(status.as_u16());
    if matches!(status.as_u16(), 401 | 403) {
        return Err(ProbeError::Auth);
    }
    if !status.is_success() {
        return Err(ProbeError::Unavailable(format!("HTTP {status}")));
    }
    let body: Value = resp
        .json()
        .await
        .map_err(|e| ProbeError::Unavailable(format!("not a JSON model list: {e}")))?;
    Ok(parse_models_response(&body))
}

const AUTH_FAILED: &str = "Authentication failed — check your API key";

/// The key a custom-provider request should use: the one just typed, or — on
/// an edit where the field was left blank — the one already saved.
fn effective_api_key(typed: Option<&str>, saved: Option<&ProviderEntry>) -> String {
    match typed.map(str::trim).filter(|k| !k.is_empty()) {
        Some(key) => key.to_string(),
        None => saved.map(|p| p.api_key.clone()).unwrap_or_default(),
    }
}

/// "Fetch models" in the custom provider form: list what the endpoint serves
/// without saving anything. `provider_id` lets an edit reuse the saved key.
#[tauri::command]
pub async fn probe_custom_provider(
    base_url: String,
    protocol: String,
    api_key: Option<String>,
    provider_id: Option<String>,
    state: State<'_, AppState>,
) -> Result<Vec<String>, String> {
    let protocol = normalize_protocol(&protocol)?;
    let base_url = normalize_base_url(&base_url)?;
    let key = {
        let cfg = state.config.lock().await;
        let saved = provider_id
            .as_deref()
            .and_then(|id| cfg.providers.get(id))
            .filter(|p| p.custom);
        effective_api_key(api_key.as_deref(), saved)
    };
    match fetch_custom_models(&base_url, protocol, &key).await {
        Ok(models) if models.is_empty() => Err(format!(
            "{} returned no models",
            models_url(&base_url, protocol)
        )),
        Ok(models) => Ok(models),
        Err(ProbeError::Auth) => Err(AUTH_FAILED.into()),
        Err(ProbeError::Unavailable(why)) => Err(format!(
            "could not list models from {} ({why})",
            models_url(&base_url, protocol)
        )),
    }
}

/// Create or update a custom provider: a name, a base URL, a protocol, an
/// optional API key and a model list. With no models typed in, the endpoint's
/// own `/models` supplies them; with models typed in, the endpoint is still
/// asked once so a rejected key fails here rather than on the first message —
/// but being unreachable is not an error then (the server may just be off).
#[tauri::command]
pub async fn save_custom_provider(
    input: CustomProviderInput,
    state: State<'_, AppState>,
) -> Result<CustomProviderSaved, String> {
    let name = input.name.trim().to_string();
    if name.is_empty() {
        return Err("Name is required".into());
    }
    let protocol = normalize_protocol(&input.protocol)?;
    let base_url = normalize_base_url(&input.base_url)?;
    let editing = input.provider_id.filter(|id| !id.trim().is_empty());

    let existing = {
        let cfg = state.config.lock().await;
        match &editing {
            Some(id) => Some(
                cfg.providers
                    .get(id)
                    .filter(|p| p.custom)
                    .cloned()
                    .ok_or_else(|| format!("unknown custom provider: {id}"))?,
            ),
            None => None,
        }
    };
    let api_key = effective_api_key(input.api_key.as_deref(), existing.as_ref());

    let typed = normalize_model_ids(input.models);
    let models = match fetch_custom_models(&base_url, protocol, &api_key).await {
        Err(ProbeError::Auth) => return Err(AUTH_FAILED.into()),
        _ if !typed.is_empty() => typed,
        Ok(found) if !found.is_empty() => found,
        Ok(_) => {
            return Err(format!(
                "{} returned no models — add the model ids manually",
                models_url(&base_url, protocol)
            ));
        }
        Err(ProbeError::Unavailable(why)) => {
            return Err(format!(
                "could not list models from {} ({why}) — add the model ids manually",
                models_url(&base_url, protocol)
            ));
        }
    };

    let mut cfg = state.config.lock().await;
    let provider_id = match editing {
        Some(id) => id,
        None => {
            let taken: std::collections::HashSet<String> = RESERVED_PROVIDER_IDS
                .iter()
                .map(|id| id.to_string())
                .chain(cfg.providers.keys().cloned())
                .chain(catalog::cached_provider_ids())
                .collect();
            unique_provider_id(&name, &taken)
        }
    };
    // An edit keeps what the user curated; a new entry has nothing to keep.
    // No pricing either way: there is no catalog to take it from, and a wrong
    // number is worse than the "unknown" the accounting already handles.
    let enabled_models = existing
        .map(|p| p.enabled_models)
        .unwrap_or_default()
        .into_iter()
        .filter(|m| models.contains(m))
        .collect();
    cfg.providers.insert(
        provider_id.clone(),
        ProviderEntry {
            api_key,
            base_url,
            protocol: protocol.into(),
            enabled_models,
            label: Some(name),
            model_pricing: Default::default(),
            model_output_limits: Default::default(),
            custom: true,
            custom_models: models.clone(),
        },
    );
    save_config(&cfg);

    Ok(CustomProviderSaved {
        provider_id,
        models,
    })
}

/// Remove a connected provider; model slots pointing at it fall back to the
/// Claudinio defaults so no session ever resolves to a dangling provider.
#[tauri::command]
pub async fn disconnect_provider(
    provider_id: String,
    state: State<'_, AppState>,
) -> Result<(), String> {
    let mut cfg = state.config.lock().await;
    cfg.providers.remove(&provider_id);
    let prefix = format!("{provider_id}/");
    if cfg.brain_model.starts_with(&prefix) {
        cfg.brain_model = "claudius".into();
    }
    if cfg.builder_model.starts_with(&prefix) {
        cfg.builder_model = "claudinio".into();
    }
    save_config(&cfg);
    Ok(())
}

/// Wire model ids for one connected provider (unqualified). OpenRouter is
/// listed live (its catalog churns daily); a custom provider answers from the
/// list saved with it; everything else comes from the models.dev cache. An
/// `enabled_models` curation filters all three.
#[tauri::command]
pub async fn list_provider_models(
    provider_id: String,
    state: State<'_, AppState>,
) -> Result<Vec<String>, String> {
    let (enabled, custom_models) = {
        let cfg = state.config.lock().await;
        let entry = cfg.providers.get(&provider_id);
        (
            entry.map(|p| p.enabled_models.clone()),
            entry.filter(|p| p.custom).map(|p| p.custom_models.clone()),
        )
    };
    let mut models = if let Some(custom_models) = custom_models {
        custom_models
    } else if provider_id == OPENROUTER_ID {
        list_openrouter_models_live().await?
    } else {
        let cat = catalog::fetch_catalog(false).await?;
        let provider = catalog::find_provider(&cat, &provider_id)
            .ok_or_else(|| format!("unknown provider: {provider_id}"))?;
        provider
            .get("models")
            .and_then(|m| m.as_array())
            .map(|arr| {
                arr.iter()
                    .filter_map(|m| m.get("id").and_then(|i| i.as_str()).map(String::from))
                    .collect()
            })
            .unwrap_or_default()
    };
    if let Some(enabled) = enabled.filter(|e| !e.is_empty()) {
        models.retain(|m| enabled.contains(m));
    }
    Ok(models)
}

/// All model groups for the pickers: Claudinio first (unqualified ids, same
/// fallback as `list_models`), then each connected provider with qualified
/// "<provider_id>/<model>" ids. Per-provider listing failures degrade to
/// that provider's snapshot keys rather than failing the whole call.
#[tauri::command]
pub async fn list_all_models(state: State<'_, AppState>) -> Result<Vec<ModelGroup>, String> {
    let mut groups = vec![ModelGroup {
        provider_id: "claudinio".into(),
        provider_name: "Claudinio".into(),
        models: crate::commands::agent::list_models(state.clone())
            .await
            .unwrap_or_else(|_| vec!["claudinio".into(), "claudius".into()]),
        labels: std::collections::HashMap::new(),
    }];

    let connected: Vec<(String, Option<String>)> = {
        let cfg = state.config.lock().await;
        let mut ids: Vec<_> = cfg
            .providers
            .iter()
            .map(|(id, p)| (id.clone(), p.label.clone()))
            .collect();
        // OpenRouter is the featured external provider — list it first.
        ids.sort_by_key(|(id, _)| (id != OPENROUTER_ID, id.clone()));
        ids
    };

    for (id, label) in connected {
        let models = match list_provider_models(id.clone(), state.clone()).await {
            Ok(m) if !m.is_empty() => m,
            _ => {
                let cfg = state.config.lock().await;
                cfg.providers
                    .get(&id)
                    .map(|p| {
                        p.model_pricing
                            .keys()
                            .cloned()
                            .collect::<std::collections::BTreeSet<_>>()
                            .into_iter()
                            .collect()
                    })
                    .unwrap_or_default()
            }
        };
        groups.push(ModelGroup {
            provider_name: label.unwrap_or_else(|| id.clone()),
            models: models.into_iter().map(|m| format!("{id}/{m}")).collect(),
            provider_id: id,
            labels: std::collections::HashMap::new(),
        });
    }

    // Locally served models. Not a connected provider — they have no account
    // and no catalog entry — so they are appended from what is on disk rather
    // than from `config.providers`. An empty group is skipped: an empty
    // "Local" heading in the picker reads as something being broken.
    let local_enabled = { state.config.lock().await.local.enabled };
    let local: Vec<(String, String)> = if !local_enabled {
        Vec::new()
    } else {
        crate::llama::catalog::load()
            .map(|c| {
                c.entries
                    .into_iter()
                    .filter(crate::llama::catalog::is_complete)
                    // A drafter has no tokenizer and answers nothing — the
                    // same reason the settings list hides them. Offered here
                    // it is a chat model that loads and then says nothing.
                    .filter(|m| {
                        !crate::llama::mlx_mtp::is_drafter(&m.repo)
                            && !crate::llama::catalog::is_mlx_drafter(m)
                    })
                    .map(|m| {
                        (
                            format!("{}/{}", crate::llama::LOCAL_PROVIDER_ID, m.key),
                            m.display_name,
                        )
                    })
                    .collect()
            })
            .unwrap_or_default()
    };
    if !local.is_empty() {
        groups.push(ModelGroup {
            provider_id: crate::llama::LOCAL_PROVIDER_ID.into(),
            // Not "Local (llama.cpp)": which engine serves these is decided
            // per model, and for an MLX or MTPLX user that label was a lie.
            provider_name: "Local".into(),
            models: local.iter().map(|(id, _)| id.clone()).collect(),
            labels: local.into_iter().collect(),
        });
    }

    Ok(groups)
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::agent::jev::test_support::spawn_stub;
    use serde_json::json;

    fn taken(ids: &[&str]) -> std::collections::HashSet<String> {
        ids.iter().map(|id| id.to_string()).collect()
    }

    /// The stub's URL ends in the Jev path; a provider base URL stops at "/v1".
    fn stub_base(url: &str) -> String {
        url.strip_suffix("/systemone").unwrap().to_string()
    }

    #[test]
    fn a_provider_id_is_a_slash_free_slug_of_the_name() {
        assert_eq!(slugify_provider_id("LiteLLM"), "litellm");
        assert_eq!(
            slugify_provider_id("  My Company / LLM proxy!  "),
            "my-company-llm-proxy"
        );
        assert_eq!(slugify_provider_id("localhost:11434"), "localhost-11434");
        // Nothing usable left — still a valid id rather than an empty prefix.
        assert_eq!(slugify_provider_id("???"), "custom");
        assert_eq!(slugify_provider_id("Não/é"), "n-o");
    }

    #[test]
    fn a_taken_id_gets_a_numbered_suffix() {
        assert_eq!(unique_provider_id("LiteLLM", &taken(&[])), "litellm");
        assert_eq!(
            unique_provider_id("OpenAI", &taken(&["openai"])),
            "openai-2"
        );
        assert_eq!(
            unique_provider_id("Local", &taken(&["local", "local-2"])),
            "local-3"
        );
    }

    #[test]
    fn reserved_ids_cover_every_built_in_route() {
        for id in ["claudinio", "local", "openrouter"] {
            assert!(RESERVED_PROVIDER_IDS.contains(&id), "{id} must be reserved");
        }
    }

    #[test]
    fn model_ids_are_trimmed_and_deduplicated_in_order() {
        let got = normalize_model_ids(vec![
            " gpt-4o ".into(),
            String::new(),
            "claude-sonnet".into(),
            "gpt-4o".into(),
            "   ".into(),
        ]);
        assert_eq!(got, vec!["gpt-4o", "claude-sonnet"]);
    }

    #[test]
    fn a_base_url_needs_a_scheme_and_loses_its_trailing_slash() {
        assert_eq!(
            normalize_base_url(" http://localhost:4000/v1/ ").unwrap(),
            "http://localhost:4000/v1"
        );
        assert_eq!(
            normalize_base_url("https://llm.corp.example").unwrap(),
            "https://llm.corp.example"
        );
        assert!(normalize_base_url("").is_err());
        assert!(normalize_base_url("localhost:4000/v1").is_err());
        assert!(normalize_base_url("ftp://example.com").is_err());
    }

    #[test]
    fn only_the_two_wire_protocols_are_accepted() {
        assert_eq!(normalize_protocol("openai").unwrap(), "openai");
        assert_eq!(normalize_protocol(" anthropic ").unwrap(), "anthropic");
        assert!(normalize_protocol("gemini").is_err());
    }

    #[test]
    fn the_models_url_follows_the_protocol() {
        assert_eq!(
            models_url("http://localhost:4000/v1/", "openai"),
            "http://localhost:4000/v1/models"
        );
        // With or without "/v1", the Anthropic route is the same.
        assert_eq!(
            models_url("http://localhost:4000/v1", "anthropic"),
            "http://localhost:4000/v1/models"
        );
        assert_eq!(
            models_url("http://localhost:4000", "anthropic"),
            "http://localhost:4000/v1/models"
        );
    }

    #[test]
    fn a_models_response_is_read_in_every_shape_servers_use() {
        let openai = json!({"object": "list", "data": [{"id": "gpt-4o"}, {"id": "o3"}]});
        assert_eq!(parse_models_response(&openai), vec!["gpt-4o", "o3"]);
        let bare = json!([{"id": "a"}, "b", {"name": "no id"}]);
        assert_eq!(parse_models_response(&bare), vec!["a", "b"]);
        let keyed = json!({"models": [{"id": "m"}, {"id": "m"}]});
        assert_eq!(parse_models_response(&keyed), vec!["m"]);
        assert!(parse_models_response(&json!({"error": "nope"})).is_empty());
    }

    #[test]
    fn a_blank_key_on_an_edit_keeps_the_saved_one() {
        let saved = ProviderEntry {
            api_key: "sk-saved".into(),
            base_url: "http://localhost:4000/v1".into(),
            protocol: "openai".into(),
            enabled_models: vec![],
            label: None,
            model_pricing: Default::default(),
            model_output_limits: Default::default(),
            custom: true,
            custom_models: vec![],
        };
        assert_eq!(effective_api_key(Some(" sk-new "), Some(&saved)), "sk-new");
        assert_eq!(effective_api_key(Some("  "), Some(&saved)), "sk-saved");
        assert_eq!(effective_api_key(None, Some(&saved)), "sk-saved");
        assert_eq!(effective_api_key(None, None), "");
    }

    #[tokio::test]
    async fn an_openai_endpoint_is_asked_with_a_bearer_key() {
        let (url, stub) = spawn_stub(200, r#"{"data":[{"id":"gpt-4o"},{"id":"qwen3"}]}"#);
        let models = fetch_custom_models(&stub_base(&url), "openai", "sk-test")
            .await
            .unwrap();
        assert_eq!(models, vec!["gpt-4o", "qwen3"]);
        let request = stub.join().unwrap().to_ascii_lowercase();
        assert!(request.starts_with("get /v1/models "), "{request}");
        assert!(
            request.contains("authorization: bearer sk-test"),
            "{request}"
        );
    }

    #[tokio::test]
    async fn a_keyless_endpoint_gets_no_authorization_header() {
        let (url, stub) = spawn_stub(200, r#"{"data":[{"id":"llama3"}]}"#);
        let models = fetch_custom_models(&stub_base(&url), "openai", "")
            .await
            .unwrap();
        assert_eq!(models, vec!["llama3"]);
        let request = stub.join().unwrap().to_ascii_lowercase();
        assert!(!request.contains("authorization:"), "{request}");
    }

    #[tokio::test]
    async fn an_anthropic_endpoint_is_asked_with_x_api_key() {
        let (url, stub) = spawn_stub(200, r#"{"data":[{"id":"claude-sonnet-4-5"}]}"#);
        let models = fetch_custom_models(&stub_base(&url), "anthropic", "sk-ant")
            .await
            .unwrap();
        assert_eq!(models, vec!["claude-sonnet-4-5"]);
        let request = stub.join().unwrap().to_ascii_lowercase();
        assert!(request.starts_with("get /v1/models "), "{request}");
        assert!(request.contains("x-api-key: sk-ant"), "{request}");
        assert!(request.contains("anthropic-version: "), "{request}");
        assert!(!request.contains("authorization:"), "{request}");
    }

    #[tokio::test]
    async fn a_rejected_key_is_told_apart_from_a_missing_models_route() {
        let (url, stub) = spawn_stub(401, r#"{"error":"bad key"}"#);
        let got = fetch_custom_models(&stub_base(&url), "openai", "sk-bad").await;
        assert_eq!(got, Err(ProbeError::Auth));
        stub.join().unwrap();

        let (url, stub) = spawn_stub(404, r#"{"error":"not found"}"#);
        let got = fetch_custom_models(&stub_base(&url), "openai", "sk-ok").await;
        assert!(matches!(got, Err(ProbeError::Unavailable(_))), "{got:?}");
        stub.join().unwrap();
    }

    #[test]
    fn the_form_payload_deserializes_from_camel_case() {
        let input: CustomProviderInput = serde_json::from_value(json!({
            "providerId": null,
            "name": "LiteLLM",
            "baseUrl": "http://localhost:4000/v1",
            "protocol": "openai",
            "apiKey": null,
            "models": ["gpt-4o"],
        }))
        .unwrap();
        assert!(input.provider_id.is_none());
        assert_eq!(input.base_url, "http://localhost:4000/v1");
        assert_eq!(input.models, vec!["gpt-4o"]);
    }
}
