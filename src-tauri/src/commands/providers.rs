//! Tauri commands for external LLM providers: the OpenRouter OAuth PKCE
//! connect flow, the models.dev catalog, and generic connect/disconnect for
//! any OpenAI-compatible (or Anthropic-compatible) provider from that
//! catalog. Claudinio's own login stays in `commands::auth`.

use crate::agent::provider::{catalog, save_config, ProviderEntry};
use crate::commands::auth::{random_hex, wait_for_callback};
use crate::state::AppState;
use base64::Engine;
use serde::Serialize;
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
        return Err(format!("OpenRouter model list failed: HTTP {}", resp.status()));
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
            },
        );
        save_config(&cfg);
    }

    Ok(models)
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
/// listed live (its catalog churns daily); everything else comes from the
/// models.dev cache. An `enabled_models` curation filters both.
#[tauri::command]
pub async fn list_provider_models(
    provider_id: String,
    state: State<'_, AppState>,
) -> Result<Vec<String>, String> {
    let enabled = {
        let cfg = state.config.lock().await;
        cfg.providers
            .get(&provider_id)
            .map(|p| p.enabled_models.clone())
    };
    let mut models = if provider_id == OPENROUTER_ID {
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
        });
    }

    Ok(groups)
}
