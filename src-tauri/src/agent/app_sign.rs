//! HMAC request signing for the `/api/app/*` endpoints on claudin.io.
//!
//! This is a bar-raising layer, not a security boundary: any secret shipped
//! inside a desktop binary can be extracted via reverse engineering, and
//! there is no hardware attestation on desktop platforms. The real
//! protections live server-side — revocable per-user API keys, budgets, and
//! per-key rate limits. This signature just makes casual/scripted abuse of
//! the app-dedicated endpoints (websearch, login exchange) more annoying,
//! and is versioned so the secret can be rotated per app release.
use hmac::{Hmac, Mac};
use sha2::{Digest, Sha256};
use std::time::{SystemTime, UNIX_EPOCH};

type HmacSha256 = Hmac<Sha256>;

const SIGNATURE_VERSION: &str = "1";

/// The v1 secret. Deliberately **not** a real value in source control —
/// this placeholder must never match the server's `APP_HMAC_SECRET_V1`.
/// Override it at build time with `APP_HMAC_SECRET_V1_BUILD` (e.g. injected
/// by CI from a secret store) so the real signing key never lands in git
/// history; with the placeholder, signatures simply won't validate, which
/// is safe as long as `APP_SIGNATURE_REQUIRED=false` on the server (the
/// default until a signed app build has shipped). Rotate per release by
/// bumping SIGNATURE_VERSION and adding a new APP_HMAC_SECRET_V<n> server-side.
const SECRET_V1_PLACEHOLDER: &str = "REPLACE-VIA-APP_HMAC_SECRET_V1_BUILD-AT-BUILD-TIME";

fn secret_v1() -> &'static [u8] {
    option_env!("APP_HMAC_SECRET_V1_BUILD")
        .unwrap_or(SECRET_V1_PLACEHOLDER)
        .as_bytes()
}

fn hex_encode(bytes: &[u8]) -> String {
    bytes.iter().map(|b| format!("{b:02x}")).collect()
}

/// Sign a request, returning the three headers to attach:
/// `X-App-Version`, `X-App-Timestamp`, `X-App-Signature`.
pub fn sign(method: &str, path: &str, body: &[u8]) -> [(&'static str, String); 3] {
    let ts = SystemTime::now()
        .duration_since(UNIX_EPOCH)
        .map(|d| d.as_secs())
        .unwrap_or(0);
    let body_hash = hex_encode(&Sha256::digest(body));
    let message = format!("{ts}\n{method}\n{path}\n{body_hash}");

    let secret = secret_v1();
    let mut mac = HmacSha256::new_from_slice(secret).expect("HMAC accepts any key length");
    mac.update(message.as_bytes());
    let signature = hex_encode(&mac.finalize().into_bytes());

    [
        ("X-App-Version", SIGNATURE_VERSION.to_string()),
        ("X-App-Timestamp", ts.to_string()),
        ("X-App-Signature", signature),
    ]
}
