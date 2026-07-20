//! Cross-platform device `install_id` for the app-install trial promo.
//!
//! This is a **bar-raiser, not a security boundary** — the same philosophy as
//! `app_sign`. A determined attacker can spoof `/etc/machine-id`, use a VM, or
//! reset it; the real protections are server-side (per-device idempotent credit
//! grant + velocity radar). The goal here is only to make casual credit farming
//! (new throwaway account on the same machine) more annoying.
//!
//! Privacy: we read the opaque OS *install GUID* (`machine-uid`) — never a MAC
//! address, disk/CPU serial, or username — and we **never transmit it raw**. Only
//! `sha256(APP_SALT || ":" || guid)` leaves the machine, which is non-reversible
//! and non-correlatable. This mirrors systemd's own
//! `sd_id128_get_machine_app_specific` recommendation. If the OS GUID can't be
//! read we fall back to a locally-persisted random uuid (still hashed), so the
//! flow never breaks.

use sha2::{Digest, Sha256};

/// Application-specific salt so the hash can't be correlated with the raw
/// machine-id or with any other app that reads the same OS GUID. Bump the
/// version suffix if the derivation ever needs to change.
const APP_SALT: &str = "claudinio-app-install-v1";

fn hex_encode(bytes: &[u8]) -> String {
    bytes.iter().map(|b| format!("{b:02x}")).collect()
}

/// Compute the opaque, non-reversible `install_id` for this device.
///
/// Prefers the OS install GUID (`machine_uid::get()`); if that fails on this
/// platform, hashes `fallback_seed` instead (a caller-persisted random uuid, so
/// the fallback id is itself stable across runs). The raw GUID never leaves this
/// function — only the salted SHA-256 hash is returned.
pub fn compute_install_id(fallback_seed: &str) -> String {
    let source = machine_uid::get().unwrap_or_else(|_| fallback_seed.to_string());
    let mut hasher = Sha256::new();
    hasher.update(APP_SALT.as_bytes());
    hasher.update(b":");
    hasher.update(source.as_bytes());
    hex_encode(&hasher.finalize())
}
