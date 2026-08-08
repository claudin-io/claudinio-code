//! Verified file downloads shared by everything that fetches a pinned asset.
//!
//! Extracted from `code_intel::embeddings`, which is where this pattern was
//! first needed (the embedding model) — the browser module needs the identical
//! guarantees for the Chromium build, and `browser/` may not depend on
//! `code_intel/`.
//!
//! The invariant is that a corrupt or truncated download can never become the
//! cache: bytes stream into a `.part` file while a sha256 is computed
//! incrementally, size and hash are both checked, and only then is the file
//! renamed into place.

use crate::net_activity::{NetGuard, NetSource};
use std::path::Path;
use std::sync::Arc;

/// Called with (bytes_so_far, content_length_or_0) as the download proceeds.
pub type ProgressFn = Arc<dyn Fn(u64, u64) + Send + Sync>;

pub const DEFAULT_RETRIES: usize = 3;

/// Download `url` to `dest`, verifying length and sha256 before committing.
#[allow(clippy::too_many_arguments)]
pub async fn download_verified(
    url: &str,
    dest: &Path,
    label: &str,
    sha256_hex: &str,
    expected_len: u64,
    source: NetSource,
    on_progress: Option<&ProgressFn>,
) -> Result<(), String> {
    use futures::StreamExt;
    use sha2::Digest;

    let net_guard = NetGuard::begin(source, label);
    let client = crate::http::default_client();
    let response = client
        .get(url)
        .send()
        .await
        .map_err(|e| format!("download {label}: {e}"))?;

    let status = response.status();
    if !status.is_success() {
        return Err(format!("download {label} failed: HTTP {status}"));
    }
    // Prefer the server's own length for the progress denominator: for a
    // pinned asset it should equal `expected_len`, but a proxy or a stale pin
    // makes them differ, and a progress bar that reads 130% is worse than one
    // driven by what is actually being transferred.
    let total = response.content_length().unwrap_or(expected_len);

    let part_path = dest.with_extension("part");
    if let Some(parent) = dest.parent() {
        std::fs::create_dir_all(parent).map_err(|e| format!("create {}: {e}", parent.display()))?;
    }
    let mut file = std::fs::File::create(&part_path)
        .map_err(|e| format!("create {}: {e}", part_path.display()))?;
    let mut hasher = sha2::Sha256::new();
    let mut written: u64 = 0;
    let mut stream = response.bytes_stream();
    while let Some(chunk) = stream.next().await {
        let chunk = chunk.map_err(|e| format!("read {label}: {e}"))?;
        std::io::Write::write_all(&mut file, &chunk).map_err(|e| format!("write {label}: {e}"))?;
        hasher.update(&chunk);
        written += chunk.len() as u64;
        net_guard.add_bytes(chunk.len() as u64);
        if let Some(cb) = on_progress {
            cb(written, total);
        }
    }
    drop(file);

    let digest = format!("{:x}", hasher.finalize());
    if written != expected_len || digest != sha256_hex {
        let _ = std::fs::remove_file(&part_path);
        return Err(format!(
            "verification failed for {label}: got {written} bytes sha256 {digest}, \
             expected {expected_len} bytes sha256 {sha256_hex}"
        ));
    }
    std::fs::rename(&part_path, dest).map_err(|e| format!("finalize {label}: {e}"))?;
    Ok(())
}

/// `download_verified` with exponential backoff. A hash mismatch is retried
/// like any other failure: the usual cause is a truncated body from a flaky
/// connection, not a genuinely changed artifact.
#[allow(clippy::too_many_arguments)]
pub async fn download_verified_with_retries(
    url: &str,
    dest: &Path,
    label: &str,
    sha256_hex: &str,
    expected_len: u64,
    source: NetSource,
    on_progress: Option<&ProgressFn>,
    retries: usize,
) -> Result<(), String> {
    let mut last_error = String::new();
    for attempt in 0..retries.max(1) {
        if attempt > 0 {
            tokio::time::sleep(std::time::Duration::from_secs(2 << (attempt - 1))).await;
        }
        match download_verified(
            url,
            dest,
            label,
            sha256_hex,
            expected_len,
            source,
            on_progress,
        )
        .await
        {
            Ok(()) => return Ok(()),
            Err(e) => {
                eprintln!("[download] {label} attempt {} failed: {e}", attempt + 1);
                last_error = e;
            }
        }
    }
    Err(format!(
        "download {label} failed after {} attempts: {last_error}",
        retries.max(1)
    ))
}
