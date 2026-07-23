use crate::agent::app_sign;
use crate::agent::provider::save_config;
use crate::state::AppState;
use serde::{Deserialize, Serialize};
use serde_json::Value;
use sha2::{Digest, Sha256};
use tauri::State;
use tauri_plugin_opener::OpenerExt;
use tokio::io::{AsyncReadExt, AsyncWriteExt};
use tokio::net::TcpListener;
use tokio::time::{Duration, timeout};

pub(crate) const CALLBACK_OK: &str = "HTTP/1.1 200 OK\r\nContent-Type: text/html; charset=utf-8\r\nConnection: close\r\n\r\n<html><body style=\"font-family:sans-serif;text-align:center;padding-top:64px\"><h2>Signed in</h2><p>You can return to Claudinio Code.</p></body></html>";
pub(crate) const CALLBACK_ERR: &str = "HTTP/1.1 400 Bad Request\r\nContent-Type: text/html; charset=utf-8\r\nConnection: close\r\n\r\n<html><body style=\"font-family:sans-serif;text-align:center;padding-top:64px\"><h2>Login failed</h2><p>Please return to Claudinio Code and try again.</p></body></html>";

#[derive(Serialize)]
pub struct LoginResult {
    pub login: String,
    pub tier: Option<String>,
}

#[derive(Deserialize)]
struct ExchangeResponse {
    key: String,
    user: ExchangeUser,
}

#[derive(Deserialize)]
struct ExchangeUser {
    login: String,
    tier: Option<String>,
}

#[derive(Deserialize)]
struct ExchangeError {
    error: String,
    upgrade_url: Option<String>,
}

pub(crate) fn hex_encode(bytes: &[u8]) -> String {
    bytes.iter().map(|b| format!("{b:02x}")).collect()
}

/// Random hex string of `n_bytes` bytes, sourced from `uuid`'s CSPRNG-backed
/// v4 generator so we don't need to add a dedicated `rand` dependency.
pub(crate) fn random_hex(n_bytes: usize) -> String {
    let mut bytes = Vec::with_capacity(n_bytes);
    while bytes.len() < n_bytes {
        bytes.extend_from_slice(uuid::Uuid::new_v4().as_bytes());
    }
    bytes.truncate(n_bytes);
    hex_encode(&bytes)
}

/// Percent-decode a query string value. Our own params (code/state) never
/// need it in practice, but this keeps the callback parser correct if a
/// browser or proxy ever encodes them.
fn percent_decode(s: &str) -> String {
    let bytes = s.as_bytes();
    let mut out = Vec::with_capacity(bytes.len());
    let mut i = 0;
    while i < bytes.len() {
        match bytes[i] {
            b'%' if i + 2 < bytes.len() => {
                if let Ok(byte) = u8::from_str_radix(&s[i + 1..i + 3], 16) {
                    out.push(byte);
                    i += 3;
                    continue;
                }
                out.push(bytes[i]);
                i += 1;
            }
            b'+' => {
                out.push(b' ');
                i += 1;
            }
            b => {
                out.push(b);
                i += 1;
            }
        }
    }
    String::from_utf8_lossy(&out).into_owned()
}

/// Parse `code` (and `state`, when present) from the callback request line.
/// OpenRouter's PKCE flow has no state param, so state is optional here;
/// callers that require it enforce the match in `wait_for_callback`.
pub(crate) fn parse_callback_query(request_line: &str) -> Option<(String, Option<String>)> {
    let path = request_line.split_whitespace().nth(1)?;
    let (_, query) = path.split_once('?')?;
    let mut code = None;
    let mut state = None;
    for pair in query.split('&') {
        let (k, v) = pair.split_once('=')?;
        match k {
            "code" => code = Some(percent_decode(v)),
            "state" => state = Some(percent_decode(v)),
            _ => {}
        }
    }
    Some((code?, state))
}

/// Accept one loopback connection and extract the OAuth code. When
/// `expected_state` is Some, the callback must echo a matching `state` param
/// (CSRF check); None skips the check for flows without a state round-trip
/// (OpenRouter PKCE — a forged code is useless without our in-memory
/// verifier).
pub(crate) async fn wait_for_callback(
    listener: TcpListener,
    expected_state: Option<&str>,
) -> Result<String, String> {
    let (mut stream, _) = timeout(Duration::from_secs(120), listener.accept())
        .await
        .map_err(|_| "login timed out waiting for browser callback".to_string())?
        .map_err(|e| format!("callback listener error: {e}"))?;

    let mut buf = vec![0u8; 8192];
    let n = timeout(Duration::from_secs(10), stream.read(&mut buf))
        .await
        .map_err(|_| "timed out reading login callback".to_string())?
        .map_err(|e| format!("failed to read login callback: {e}"))?;

    let request = String::from_utf8_lossy(&buf[..n]);
    let request_line = request.lines().next().unwrap_or("");
    let parsed = parse_callback_query(request_line);

    let (code, state) = match parsed {
        Some(v) => v,
        None => {
            let _ = stream.write_all(CALLBACK_ERR.as_bytes()).await;
            return Err("malformed login callback".into());
        }
    };

    if let Some(expected) = expected_state
        && state.as_deref() != Some(expected)
    {
        let _ = stream.write_all(CALLBACK_ERR.as_bytes()).await;
        return Err("login state mismatch — possible CSRF, aborting".into());
    }

    let _ = stream.write_all(CALLBACK_OK.as_bytes()).await;
    Ok(code)
}

/// Opens the browser to claudin.io's app-authorize consent screen, waits for
/// the one-time code on a localhost loopback listener, then exchanges it for
/// the user's active API key. See dashboard `/app/authorize` +
/// `/api/app/exchange` on the provider side for the other half of this flow.
#[tauri::command]
pub async fn login_with_claudinio(
    app: tauri::AppHandle,
    state: State<'_, AppState>,
) -> Result<LoginResult, String> {
    let (services_url, install_id) = {
        let mut cfg = state.config.lock().await;
        let install_id = crate::agent::provider::ensure_install_id(&mut cfg);
        save_config(&cfg);
        (cfg.services_url.clone(), install_id)
    };

    let listener = TcpListener::bind(("127.0.0.1", 0))
        .await
        .map_err(|e| format!("failed to bind local callback port: {e}"))?;
    let port = listener
        .local_addr()
        .map_err(|e| format!("failed to read callback port: {e}"))?
        .port();

    let verifier = random_hex(32);
    let challenge = hex_encode(&Sha256::digest(verifier.as_bytes()));
    let oauth_state = random_hex(16);

    let authorize_url = format!(
        "{}/app/authorize?port={}&state={}&challenge={}",
        services_url.trim_end_matches('/'),
        port,
        oauth_state,
        challenge
    );

    app.opener()
        .open_url(authorize_url, None::<&str>)
        .map_err(|e| format!("failed to open browser: {e}"))?;

    let code = wait_for_callback(listener, Some(oauth_state.as_str())).await?;

    let exchange_path = "/api/app/exchange";
    let exchange_url = format!("{}{}", services_url.trim_end_matches('/'), exchange_path);
    let body = serde_json::json!({ "code": code, "verifier": verifier, "install_id": install_id });
    let body_bytes =
        serde_json::to_vec(&body).map_err(|e| format!("encode exchange request: {e}"))?;
    let signature_headers = app_sign::sign("POST", exchange_path, &body_bytes);

    let _net_guard = crate::net_activity::NetGuard::begin(
        crate::net_activity::NetSource::Auth,
        "login exchange",
    );
    let client = crate::http::default_client();
    let mut req = client
        .post(&exchange_url)
        .header("Content-Type", "application/json")
        .body(body_bytes);
    for (name, value) in signature_headers {
        req = req.header(name, value);
    }
    let resp = req
        .send()
        .await
        .map_err(|e| format!("login exchange request failed: {e}"))?;
    let status = resp.status();
    _net_guard.set_status(status.as_u16());

    if status == reqwest::StatusCode::FORBIDDEN {
        let err: ExchangeError = resp.json().await.map_err(|e| e.to_string())?;
        let hint = err
            .upgrade_url
            .map(|u| format!(" — upgrade at {u}"))
            .unwrap_or_default();
        return Err(format!("{}{hint}", err.error));
    }
    if !status.is_success() {
        return Err(format!("login exchange failed with status {status}"));
    }

    let parsed: ExchangeResponse = resp
        .json()
        .await
        .map_err(|e| format!("invalid login exchange response: {e}"))?;

    {
        let mut cfg = state.config.lock().await;
        cfg.api_key = parsed.key;
        cfg.account_login = Some(parsed.user.login.clone());
        cfg.account_tier = parsed.user.tier.clone();
        save_config(&cfg);
    }

    Ok(LoginResult {
        login: parsed.user.login,
        tier: parsed.user.tier,
    })
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
/// Unlike `list_models`, this does NOT swallow errors — the caller needs
/// to know whether the key is valid.
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
