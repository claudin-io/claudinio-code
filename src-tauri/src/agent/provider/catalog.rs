//! models.dev provider catalog: fetched from https://models.dev/api.json,
//! trimmed to what the UI and connect flow need, and cached on disk with a
//! 24h TTL (stale cache is served when the network is down — the catalog has
//! no SLA and inference must never depend on it).

use serde_json::{json, Value};
use std::path::PathBuf;

pub const CATALOG_URL: &str = "https://models.dev/api.json";
const CACHE_TTL_SECS: u64 = 24 * 60 * 60;

fn cache_path() -> Result<PathBuf, String> {
    let dir = dirs::config_dir()
        .ok_or("no config dir")?
        .join("claudinio-code");
    std::fs::create_dir_all(&dir).map_err(|e| format!("create config dir: {e}"))?;
    Ok(dir.join("models_dev_catalog.json"))
}

fn now_secs() -> u64 {
    std::time::SystemTime::now()
        .duration_since(std::time::UNIX_EPOCH)
        .map(|d| d.as_secs())
        .unwrap_or(0)
}

/// Read the cached (already trimmed) catalog. Returns (age_secs, data).
fn read_cache() -> Option<(u64, Value)> {
    let path = cache_path().ok()?;
    let s = std::fs::read_to_string(path).ok()?;
    let v: Value = serde_json::from_str(&s).ok()?;
    let fetched_at = v.get("fetched_at")?.as_u64()?;
    let data = v.get("data")?.clone();
    Some((now_secs().saturating_sub(fetched_at), data))
}

fn write_cache(trimmed: &Value) {
    if let Ok(path) = cache_path() {
        let wrapped = json!({"fetched_at": now_secs(), "data": trimmed});
        if let Ok(s) = serde_json::to_string(&wrapped) {
            let _ = std::fs::write(path, s);
        }
    }
}

/// Derive the wire protocol from the models.dev `npm` hint.
fn protocol_from_npm(npm: Option<&str>) -> &'static str {
    match npm {
        Some("@ai-sdk/anthropic") => "anthropic",
        _ => "openai",
    }
}

/// Reduce the ~2 MB raw api.json to the fields the UI and connect flow use.
/// Output shape (camelCase for the frontend):
/// `{"providers": [{id, name, api, env, doc, protocol,
///    models: [{id, name, costInput, costOutput, context, outputLimit,
///              reasoning, toolCall}]}]}`, providers sorted by name.
/// Providers without an `api` base URL aren't directly connectable and are
/// dropped.
pub fn trim_catalog(raw: &Value) -> Value {
    let mut providers: Vec<Value> = Vec::new();
    let Some(map) = raw.as_object() else {
        return json!({ "providers": providers });
    };
    for (id, p) in map {
        let Some(api) = p.get("api").and_then(|a| a.as_str()) else {
            continue;
        };
        if api.is_empty() {
            continue;
        }
        let name = p.get("name").and_then(|n| n.as_str()).unwrap_or(id);
        let env: Vec<String> = p
            .get("env")
            .and_then(|e| e.as_array())
            .map(|arr| {
                arr.iter()
                    .filter_map(|v| v.as_str().map(String::from))
                    .collect()
            })
            .unwrap_or_default();
        let doc = p.get("doc").and_then(|d| d.as_str());
        let protocol = protocol_from_npm(p.get("npm").and_then(|n| n.as_str()));

        let mut models: Vec<Value> = Vec::new();
        if let Some(model_map) = p.get("models").and_then(|m| m.as_object()) {
            for (model_id, m) in model_map {
                models.push(json!({
                    "id": model_id,
                    "name": m.get("name").and_then(|n| n.as_str()).unwrap_or(model_id),
                    "costInput": m.pointer("/cost/input").and_then(|c| c.as_f64()),
                    "costOutput": m.pointer("/cost/output").and_then(|c| c.as_f64()),
                    "context": m.pointer("/limit/context").and_then(|c| c.as_u64()),
                    "outputLimit": m.pointer("/limit/output").and_then(|c| c.as_u64()),
                    "reasoning": m.get("reasoning").and_then(|r| r.as_bool()).unwrap_or(false),
                    "toolCall": m.get("tool_call").and_then(|t| t.as_bool()).unwrap_or(false),
                }));
            }
        }
        models.sort_by(|a, b| {
            a["id"]
                .as_str()
                .unwrap_or("")
                .cmp(b["id"].as_str().unwrap_or(""))
        });

        providers.push(json!({
            "id": id,
            "name": name,
            "api": api,
            "env": env,
            "doc": doc,
            "protocol": protocol,
            "models": models,
        }));
    }
    providers.sort_by(|a, b| {
        a["name"]
            .as_str()
            .unwrap_or("")
            .to_lowercase()
            .cmp(&b["name"].as_str().unwrap_or("").to_lowercase())
    });
    json!({ "providers": providers })
}

/// Fetch the trimmed catalog, honoring the disk cache: a fresh cache (< 24h)
/// short-circuits unless `force`; a network failure serves the stale cache
/// when one exists.
pub async fn fetch_catalog(force: bool) -> Result<Value, String> {
    if !force {
        if let Some((age, data)) = read_cache() {
            if age < CACHE_TTL_SECS {
                return Ok(data);
            }
        }
    }
    match fetch_remote().await {
        Ok(trimmed) => {
            write_cache(&trimmed);
            Ok(trimmed)
        }
        Err(e) => match read_cache() {
            Some((_, data)) => Ok(data),
            None => Err(e),
        },
    }
}

async fn fetch_remote() -> Result<Value, String> {
    let _net_guard = crate::net_activity::NetGuard::begin(
        crate::net_activity::NetSource::ProviderCatalog,
        "models.dev catalog",
    );
    let client = crate::http::default_client();
    let response = client
        .get(CATALOG_URL)
        .send()
        .await
        .map_err(|e| format!("catalog request failed: {e}"))?;
    _net_guard.set_status(response.status().as_u16());
    if !response.status().is_success() {
        return Err(format!("catalog fetch failed: HTTP {}", response.status()));
    }
    let raw: Value = response
        .json()
        .await
        .map_err(|e| format!("catalog parse failed: {e}"))?;
    Ok(trim_catalog(&raw))
}

/// Look up one provider in the trimmed catalog.
pub fn find_provider<'a>(trimmed: &'a Value, provider_id: &str) -> Option<&'a Value> {
    trimmed
        .get("providers")?
        .as_array()?
        .iter()
        .find(|p| p.get("id").and_then(|i| i.as_str()) == Some(provider_id))
}

/// Extract the pricing/output-limit snapshots stored on a `ProviderEntry` at
/// connect time, keyed by wire model id.
pub fn model_snapshots(
    provider: &Value,
) -> (
    std::collections::HashMap<String, (f64, f64)>,
    std::collections::HashMap<String, u32>,
) {
    let mut pricing = std::collections::HashMap::new();
    let mut limits = std::collections::HashMap::new();
    if let Some(models) = provider.get("models").and_then(|m| m.as_array()) {
        for m in models {
            let Some(id) = m.get("id").and_then(|i| i.as_str()) else {
                continue;
            };
            if let (Some(ci), Some(co)) = (
                m.get("costInput").and_then(|c| c.as_f64()),
                m.get("costOutput").and_then(|c| c.as_f64()),
            ) {
                pricing.insert(id.to_string(), (ci, co));
            }
            if let Some(limit) = m.get("outputLimit").and_then(|l| l.as_u64()) {
                limits.insert(id.to_string(), limit as u32);
            }
        }
    }
    (pricing, limits)
}

#[cfg(test)]
mod tests {
    use super::*;

    fn raw_fixture() -> Value {
        json!({
            "deepseek": {
                "id": "deepseek",
                "name": "DeepSeek",
                "api": "https://api.deepseek.com",
                "env": ["DEEPSEEK_API_KEY"],
                "npm": "@ai-sdk/openai-compatible",
                "doc": "https://platform.deepseek.com/docs",
                "models": {
                    "deepseek-chat": {
                        "id": "deepseek-chat",
                        "name": "DeepSeek Chat",
                        "cost": {"input": 0.27, "output": 1.1},
                        "limit": {"context": 65536, "output": 8192},
                        "reasoning": false,
                        "tool_call": true
                    }
                }
            },
            "anthropic": {
                "id": "anthropic",
                "name": "Anthropic",
                "api": "https://api.anthropic.com/v1",
                "env": ["ANTHROPIC_API_KEY"],
                "npm": "@ai-sdk/anthropic",
                "doc": "https://docs.anthropic.com",
                "models": {
                    "claude-sonnet-4-5": {
                        "id": "claude-sonnet-4-5",
                        "name": "Claude Sonnet 4.5",
                        "cost": {"input": 3.0, "output": 15.0},
                        "limit": {"context": 200000, "output": 64000},
                        "reasoning": true,
                        "tool_call": true
                    }
                }
            },
            "no-api-provider": {
                "id": "no-api-provider",
                "name": "Local Something",
                "models": {}
            }
        })
    }

    #[test]
    fn test_trim_catalog_drops_apiless_sorts_and_derives_protocol() {
        let trimmed = trim_catalog(&raw_fixture());
        let providers = trimmed["providers"].as_array().unwrap();
        assert_eq!(providers.len(), 2, "provider without api must be dropped");
        // sorted by name: Anthropic before DeepSeek
        assert_eq!(providers[0]["id"], "anthropic");
        assert_eq!(providers[0]["protocol"], "anthropic");
        assert_eq!(providers[1]["id"], "deepseek");
        assert_eq!(providers[1]["protocol"], "openai");
        let model = &providers[1]["models"][0];
        assert_eq!(model["id"], "deepseek-chat");
        assert_eq!(model["costInput"], 0.27);
        assert_eq!(model["outputLimit"], 8192);
        assert_eq!(model["toolCall"], true);
    }

    #[test]
    fn test_find_provider_and_model_snapshots() {
        let trimmed = trim_catalog(&raw_fixture());
        let p = find_provider(&trimmed, "deepseek").unwrap();
        let (pricing, limits) = model_snapshots(p);
        assert_eq!(pricing.get("deepseek-chat"), Some(&(0.27, 1.1)));
        assert_eq!(limits.get("deepseek-chat"), Some(&8192));
        assert!(find_provider(&trimmed, "nonexistent").is_none());
    }
}
