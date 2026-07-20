//! Login OAuth loopback com claudin.io, compartilhado por app e CLI.
//!
//! Todo o fluxo (bind do listener local, PKCE, troca do código assinada) vive
//! aqui, livre de Tauri. Cada frontend fornece apenas COMO abrir o browser
//! (callback `open_browser`): o app usa o plugin opener; o CLI usa o crate `open`.

use crate::agent::{app_sign, provider};
use sha2::{Digest, Sha256};
use tokio::io::{AsyncReadExt, AsyncWriteExt};
use tokio::net::TcpListener;
use tokio::time::{timeout, Duration};

const CALLBACK_OK: &str = "HTTP/1.1 200 OK\r\nContent-Type: text/html; charset=utf-8\r\nConnection: close\r\n\r\n<html><body style=\"font-family:sans-serif;text-align:center;padding-top:64px\"><h2>Signed in</h2><p>You can return to Claudinio Code.</p></body></html>";
const CALLBACK_ERR: &str = "HTTP/1.1 400 Bad Request\r\nContent-Type: text/html; charset=utf-8\r\nConnection: close\r\n\r\n<html><body style=\"font-family:sans-serif;text-align:center;padding-top:64px\"><h2>Login failed</h2><p>Please return to Claudinio Code and try again.</p></body></html>";

/// Resultado do login: identidade e tier da conta.
#[derive(Debug, Clone, serde::Serialize)]
pub struct LoginResult {
    pub login: String,
    pub tier: Option<String>,
}

#[derive(serde::Deserialize)]
struct ExchangeResponse {
    key: String,
    user: ExchangeUser,
}

#[derive(serde::Deserialize)]
struct ExchangeUser {
    login: String,
    tier: Option<String>,
}

#[derive(serde::Deserialize)]
struct ExchangeError {
    error: String,
    upgrade_url: Option<String>,
}

pub fn hex_encode(bytes: &[u8]) -> String {
    bytes.iter().map(|b| format!("{b:02x}")).collect()
}

/// Hex aleatório de `n_bytes`, via o CSPRNG do `uuid` v4 (evita uma dep `rand`).
pub fn random_hex(n_bytes: usize) -> String {
    let mut bytes = Vec::with_capacity(n_bytes);
    while bytes.len() < n_bytes {
        bytes.extend_from_slice(uuid::Uuid::new_v4().as_bytes());
    }
    bytes.truncate(n_bytes);
    hex_encode(&bytes)
}

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

/// Extrai `code` (e `state`, quando presente) da linha de request do callback.
pub fn parse_callback_query(request_line: &str) -> Option<(String, Option<String>)> {
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

/// Aceita uma conexão loopback e extrai o código OAuth. Com `expected_state`,
/// exige o echo do `state` (CSRF); `None` pula (fluxos PKCE sem state).
pub async fn wait_for_callback(
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

    if let Some(expected) = expected_state {
        if state.as_deref() != Some(expected) {
            let _ = stream.write_all(CALLBACK_ERR.as_bytes()).await;
            return Err("login state mismatch — possible CSRF, aborting".into());
        }
    }

    let _ = stream.write_all(CALLBACK_OK.as_bytes()).await;
    Ok(code)
}

/// Fluxo completo de login com claudin.io. `open_browser` recebe a URL de
/// consentimento. Em caso de sucesso, salva a API key na config global.
pub async fn login_claudinio<F>(open_browser: F) -> Result<LoginResult, String>
where
    F: FnOnce(&str) -> Result<(), String>,
{
    let (services_url, install_id) = {
        let mut cfg = provider::load_config();
        let install_id = provider::ensure_install_id(&mut cfg);
        provider::save_config(&cfg);
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

    open_browser(&authorize_url)?;

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

    let mut cfg = provider::load_config();
    cfg.api_key = parsed.key;
    cfg.account_login = Some(parsed.user.login.clone());
    cfg.account_tier = parsed.user.tier.clone();
    provider::save_config(&cfg);

    Ok(LoginResult {
        login: parsed.user.login,
        tier: parsed.user.tier,
    })
}
