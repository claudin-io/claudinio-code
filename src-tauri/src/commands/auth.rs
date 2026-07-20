use crate::agent::provider::save_config;
use crate::state::AppState;
use serde_json::Value;
use tauri::State;
use tauri_plugin_opener::OpenerExt;

// O fluxo OAuth loopback + PKCE + exchange vive em `claudinio_core::auth`,
// compartilhado com o CLI. Aqui fica apenas o wrapper Tauri (abrir browser +
// sincronizar a config em memória).
pub use claudinio_core::auth::LoginResult;

/// Opens the browser to claudin.io's consent screen via the loopback OAuth flow
/// in the core, then syncs the freshly-saved API key into the in-memory config.
#[tauri::command]
pub async fn login_with_claudinio(
    app: tauri::AppHandle,
    state: State<'_, AppState>,
) -> Result<LoginResult, String> {
    let result = claudinio_core::auth::login_claudinio(|url| {
        app.opener()
            .open_url(url, None::<&str>)
            .map_err(|e| format!("failed to open browser: {e}"))
    })
    .await?;

    // O core salvou a config em disco; recarrega para o AppState em memória.
    *state.config.lock().await = crate::agent::provider::load_config();
    Ok(result)
}

#[tauri::command]
pub async fn logout_claudinio(state: State<'_, AppState>) -> Result<(), String> {
    let mut cfg = state.config.lock().await;
    cfg.api_key = String::new();
    cfg.account_login = None;
    cfg.account_tier = None;
    save_config(&cfg);
    Ok(())
}

/// Validates an API key by calling GET {base_url}/v1/models with it.
/// Returns the model list on success, or a descriptive error on failure.
#[tauri::command]
pub async fn validate_api_key(
    api_key: String,
    state: State<'_, AppState>,
) -> Result<Vec<String>, String> {
    let base_url = {
        let cfg = state.config.lock().await;
        cfg.base_url.trim_end_matches('/').to_string()
    };

    let url = format!("{base_url}/v1/models");
    let _net_guard = crate::net_activity::NetGuard::begin(
        crate::net_activity::NetSource::Auth,
        "API key validation",
    );
    let client = crate::http::default_client();
    let response = client
        .get(&url)
        .header("x-api-key", &api_key)
        .send()
        .await
        .map_err(|e| format!("Network error: {e}"))?;
    _net_guard.set_status(response.status().as_u16());

    if !response.status().is_success() {
        let status = response.status();
        let body = response.text().await.unwrap_or_default();
        return Err(format!("Authentication failed (HTTP {status}): {body}"));
    }

    let body: Value = response
        .json()
        .await
        .map_err(|e| format!("Invalid API response: {e}"))?;

    let models = body
        .get("data")
        .and_then(|d| d.as_array())
        .map(|arr| {
            arr.iter()
                .filter_map(|m| m.get("id").and_then(|id| id.as_str().map(String::from)))
                .collect::<Vec<_>>()
        })
        .unwrap_or_default();

    Ok(models)
}
