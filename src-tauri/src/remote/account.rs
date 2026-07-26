//! Talking to the account server about this device.
//!
//! # What this is for, and what it is not
//!
//! Two calls, both optional, neither on the path to a working pairing:
//!
//! - **register** — records this machine against the signed-in account, so the account
//!   knows the device exists and can refuse to mint a code for one it has never seen.
//! - **mint a typed code** — the short code someone types when the camera is not an
//!   option: a desktop browser, a phone that will not focus, a code read out over a
//!   call.
//!
//! The QR path uses neither. §1.1 says `claudin.io` must never be a hard dependency of
//! remote access, and it is not: the device shows a URL carrying the channel, its
//! token and its key, the browser reads it, and Noise runs straight through the relay
//! with no account anywhere. **Everything here fails soft.** A machine that is not
//! signed in, or offline, or pointed at a self-hosted setup with no account server at
//! all, still pairs by QR — it simply cannot offer the typed code, and the UI says so
//! rather than treating it as an error.
//!
//! # Why the typed code has to be a lookup
//!
//! Ten characters cannot carry 128 bits of channel plus 256 bits of key plus a relay
//! token, so a typed code is a handle something resolves. What makes that acceptable
//! rather than a bearer token for this machine is that the account server refuses to
//! resolve a code for anyone but the account that minted it. That check is the whole
//! reason this module exists, and it is why the code is worth minting server-side
//! rather than inventing a local one.
//!
//! # The channel token travels
//!
//! `mint` sends the relay's channel token, because a code that resolved to a channel
//! and no token would resolve to a pairing the browser cannot attach to — the relay
//! refuses an attach without one. So the account server holds, for two minutes, a
//! credential that can attach to that channel. Stated rather than hidden: it is
//! proportionate because the token is not the boundary. Whoever attaches still has to
//! complete Noise IK and then match three words on this machine's own screen, and an
//! interposer produces a mismatch. The QR puts the same token on a screen for the same
//! reason.

use serde::Deserialize;

use crate::agent::app_sign;

/// A minted typed code, as the pairing panel shows it.
#[derive(Debug, Clone, Deserialize, PartialEq, Eq)]
pub struct TypedCode {
    /// Grouped for reading: `A1B2C-D3E4F`.
    pub code: String,
    /// Unix millis. The account server's own window, which is the same length as the
    /// device's — but its clock, so it is reported rather than assumed.
    #[serde(rename = "expires_at")]
    pub expires_at: u64,
}

/// Where to reach the account, and as whom.
///
/// Passed in rather than read from the config here, so this module has no opinion
/// about where settings live and the tests can point it at a local server.
#[derive(Debug, Clone)]
pub struct Account {
    /// `https://claudin.io` by default; `services_url` in the config.
    pub base_url: String,
    /// The LiteLLM virtual key the app already holds from `/api/app/exchange`. It is
    /// what says which account this device belongs to.
    pub api_key: String,
}

/// Why a call did not happen.
///
/// Separated from a bare string because the *first* variant is not a failure the user
/// should see as one: not being signed in is an ordinary state of this app, and the
/// panel offers the QR without comment rather than reporting an error nobody asked
/// about.
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum AccountError {
    /// No key. Nothing was sent.
    NotSignedIn,
    /// The server said no, with its own words where it had them.
    Refused(String),
    /// Could not get there at all.
    Unreachable(String),
}

impl AccountError {
    pub fn message(&self) -> String {
        match self {
            Self::NotSignedIn => "sign in with claudin.io to use a typed code".into(),
            Self::Refused(why) | Self::Unreachable(why) => why.clone(),
        }
    }
}

const DEVICES_PATH: &str = "/api/app/devices";
const CODES_PATH: &str = "/api/app/pairing-codes";

impl Account {
    /// The account as configured, if there is a key.
    pub fn from_config() -> Option<Self> {
        let config = crate::agent::provider::load_config();
        if config.api_key.trim().is_empty() {
            return None;
        }
        Some(Self {
            base_url: config.services_url,
            api_key: config.api_key,
        })
    }

    /// Record this machine against the account. Idempotent, and safe to call on start.
    ///
    /// Registering grants nothing. It is what lets the account refuse to mint a code
    /// naming a device it has never seen — without which a caller with a valid key
    /// could mint a code for any key at all, and the resulting code would look exactly
    /// as legitimate as a real one to whoever claimed it.
    pub async fn register_device(&self, device_key: &str, label: &str) -> Result<(), AccountError> {
        let body = serde_json::json!({ "device_key": device_key, "label": label });
        self.post(DEVICES_PATH, &body).await.map(|_| ())
    }

    /// Mint a typed code for a pairing window this device has just opened.
    pub async fn mint_typed_code(
        &self,
        device_key: &str,
        channel: &str,
        channel_token: &str,
        relay_url: &str,
    ) -> Result<TypedCode, AccountError> {
        let body = serde_json::json!({
            "device_key": device_key,
            "channel": channel,
            "token": channel_token,
            "relay_url": relay_url,
        });
        let text = self.post(CODES_PATH, &body).await?;
        serde_json::from_str(&text)
            .map_err(|e| AccountError::Refused(format!("the account server's answer was not a code: {e}")))
    }

    async fn post(&self, path: &str, body: &serde_json::Value) -> Result<String, AccountError> {
        if self.api_key.trim().is_empty() {
            return Err(AccountError::NotSignedIn);
        }

        let bytes = serde_json::to_vec(body)
            .map_err(|e| AccountError::Refused(format!("encode request: {e}")))?;
        // The same signing the app's other endpoints use. A bar-raising layer rather
        // than a boundary — see `app_sign` — and the path is signed, so it has to be
        // the path and not the whole URL.
        let signature = app_sign::sign("POST", path, &bytes);

        let mut request = crate::http::default_client()
            .post(format!("{}{}", self.base_url.trim_end_matches('/'), path))
            .header("Content-Type", "application/json")
            .header("Authorization", format!("Bearer {}", self.api_key))
            .body(bytes);
        for (name, value) in signature {
            request = request.header(name, value);
        }

        let response = request
            .send()
            .await
            .map_err(|e| AccountError::Unreachable(format!("could not reach your account: {e}")))?;

        let status = response.status();
        let text = response.text().await.unwrap_or_default();
        if status.is_success() {
            return Ok(text);
        }
        Err(AccountError::Refused(explain(status, &text)))
    }
}

/// The server's own sentence when it wrote one.
///
/// It writes better messages than a status code does — "that device is not registered
/// to this account" is a different problem from "rate limit exceeded" — and it is our
/// own server. Bounded, because a body is still a body.
fn explain(status: reqwest::StatusCode, body: &str) -> String {
    #[derive(Deserialize)]
    struct Refusal {
        error: Option<String>,
    }
    match serde_json::from_str::<Refusal>(body) {
        Ok(Refusal { error: Some(error) }) if !error.is_empty() && error.len() <= 200 => error,
        _ => format!("the account server refused ({status})"),
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn a_device_with_no_key_is_not_signed_in_rather_than_broken() {
        // The distinction the panel depends on: not being signed in is an ordinary
        // state of this app, and it must not be reported as a failure of pairing —
        // which still works, by QR, with no account at all.
        let account = Account {
            base_url: "https://claudin.io".into(),
            api_key: "   ".into(),
        };
        let error = tokio::runtime::Builder::new_current_thread()
            .build()
            .unwrap()
            .block_on(account.register_device(&"aa".repeat(32), "MacBook"))
            .expect_err("an empty key cannot register");

        assert_eq!(error, AccountError::NotSignedIn);
        assert!(error.message().contains("sign in"));
    }

    #[test]
    fn the_servers_own_wording_survives() {
        assert_eq!(
            explain(
                reqwest::StatusCode::BAD_REQUEST,
                r#"{"error":"that device is not registered to this account"}"#
            ),
            "that device is not registered to this account"
        );
    }

    #[test]
    fn an_unbounded_body_is_not_shown_to_the_user() {
        let long = format!(r#"{{"error":"{}"}}"#, "x".repeat(500));
        assert!(explain(reqwest::StatusCode::BAD_REQUEST, &long).contains("refused"));
    }

    #[test]
    fn a_body_that_is_not_json_still_says_something() {
        let said = explain(reqwest::StatusCode::BAD_GATEWAY, "<html>502</html>");
        assert!(said.contains("502"), "got {said}");
    }

    #[test]
    fn a_minted_code_is_read_as_the_server_writes_it() {
        // The field names are the contract with `dashboard/remote_pairing.py`. Snake
        // case there, and a rename here rather than a serde attribute on the struct's
        // whole shape, so a change on either side fails at this test rather than by
        // silently producing a code that expires in 1970.
        let parsed: TypedCode =
            serde_json::from_str(r#"{"code":"A1B2C-D3E4F","expires_at":1753500000000}"#).unwrap();

        assert_eq!(parsed.code, "A1B2C-D3E4F");
        assert_eq!(parsed.expires_at, 1_753_500_000_000);
    }
}
