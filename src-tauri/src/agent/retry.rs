//! Riding out transient provider failures: which errors are worth another
//! attempt (`is_retryable_error`) and the interruptible backoff around
//! `provider::stream_message`.

use crate::agent::events::AgentEvent;
use crate::agent::provider::{self, AgentConfig, Message, ToolDescription};
use std::sync::atomic::{AtomicBool, Ordering};
use tauri::ipc::Channel;

/// True for errors worth retrying: network hiccups, stalled connections, and
/// rate-limit/server errors. False for things that will fail again immediately
/// (bad auth, malformed request) — retrying those just wastes time.
fn is_retryable_error(msg: &str) -> bool {
    // Plan budget exhausted: retrying is pointless (the server will refuse
    // again until the user upgrades). The frontend shows an upgrade banner.
    if msg.starts_with(crate::agent::provider::BUDGET_EXCEEDED_MARKER) {
        return false;
    }
    if msg.starts_with("stream error:") || msg.starts_with("request failed:") {
        return true;
    }
    // The wire message is "API error: HTTP 502 Bad Gateway" — StatusCode's
    // Display includes the canonical reason phrase, so parse only the first
    // token. (Parsing the whole remainder silently classified every 5xx as
    // non-retryable: a claudin.io failover 502 aborted the run instead of
    // waiting out the ~2min the backup server takes to pick up.)
    if let Some(rest) = msg.strip_prefix("API error: HTTP ")
        && let Some(code) = rest
            .split_whitespace()
            .next()
            .and_then(|s| s.parse::<u16>().ok())
    {
        return code == 429 || (500..600).contains(&code);
    }
    // Mid-stream SSE errors ("API error: overloaded_error — Overloaded"): the
    // server accepted the request and died mid-turn — same transient class.
    if msg.contains("overloaded") {
        return true;
    }
    // MTPLX cancels its own stream after a complete tool call and then reports
    // it as an external cancel (upstream race, 2.9.1/2.9.2 — youssofal/MTPLX#343).
    // The provider already salvages the turn whenever anything was streamed, so
    // reaching here means the frame arrived before any output: nothing was
    // consumed and the next attempt is free.
    if msg.contains("/v1/mtplx/cancel") {
        return true;
    }
    false
}

/// Wraps `provider::stream_message` with a retry loop for transient network
/// failures (stalled streams, dropped connections, 429/5xx). A full 30-minute
/// hang like the one that killed session 1aafbfbf was silently unrecoverable
/// before this — a single reqwest error aborted the whole agent run.
#[allow(clippy::too_many_arguments)]
pub(super) async fn stream_message_with_retry(
    config: &AgentConfig,
    model: &str,
    messages: &[Message],
    tools: &[ToolDescription],
    system: Option<&str>,
    event_tx: &Channel<AgentEvent>,
    session_id: &str,
    assistant_text: &mut String,
    interrupt: &AtomicBool,
    net_detail: &str,
) -> Result<provider::StreamOutput, String> {
    const BACKOFFS_MS: [u64; 8] = [
        2_000, 5_000, 15_000, 30_000, 60_000, 120_000, 180_000, 300_000,
    ];
    let mut attempt = 0usize;
    loop {
        assistant_text.clear();
        let result = provider::stream_message(
            config,
            model,
            messages,
            tools,
            system,
            event_tx,
            session_id,
            assistant_text,
            interrupt,
            true,
            net_detail,
        )
        .await;
        match result {
            Ok(out) => return Ok(out),
            Err(e) if attempt < BACKOFFS_MS.len() && is_retryable_error(&e) => {
                if interrupt.load(Ordering::SeqCst) {
                    return Err(e);
                }
                let delay_ms = BACKOFFS_MS[attempt];
                let _ = event_tx.send(AgentEvent::Retrying {
                    attempt: (attempt + 1) as u32,
                    max_attempts: BACKOFFS_MS.len() as u32,
                    delay_ms,
                    error: e.clone(),
                });
                // Interruptible wait: a 5-minute uninterruptible sleep would
                // ignore the user's Stop for its whole duration.
                let deadline =
                    std::time::Instant::now() + std::time::Duration::from_millis(delay_ms);
                while std::time::Instant::now() < deadline {
                    if interrupt.load(Ordering::SeqCst) {
                        return Err(e);
                    }
                    tokio::time::sleep(std::time::Duration::from_millis(250)).await;
                }
                attempt += 1;
            }
            Err(e) => return Err(e),
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn budget_exceeded_is_not_retryable() {
        let msg = format!(
            "{}Claudinio: Budget exceeded for window '1h'.",
            crate::agent::provider::BUDGET_EXCEEDED_MARKER
        );
        assert!(!is_retryable_error(&msg));
        // sanity: transient 500 stays retryable
        assert!(is_retryable_error("API error: HTTP 500"));
    }

    #[test]
    fn http_errors_with_reason_phrase_are_retryable() {
        // Real wire format: StatusCode's Display appends the canonical reason
        // ("502 Bad Gateway"), which the old whole-string parse rejected —
        // the exact bug that aborted runs during claudin.io failover.
        assert!(is_retryable_error("API error: HTTP 502 Bad Gateway"));
        assert!(is_retryable_error(
            "API error: HTTP 503 Service Unavailable"
        ));
        assert!(is_retryable_error(
            "API error: HTTP 529 <unknown status code>"
        ));
        assert!(is_retryable_error("API error: HTTP 429 Too Many Requests"));
        // openai-protocol shape appends the body message after the status
        assert!(is_retryable_error(
            "API error: HTTP 502 Bad Gateway — upstream connect error"
        ));
        // mid-stream SSE overload errors are the same transient class
        assert!(is_retryable_error(
            "API error: overloaded_error — Overloaded"
        ));
        // non-transient statuses must NOT retry
        assert!(is_retryable_error(
            "API error: request cancelled via POST /v1/mtplx/cancel after 12 streamed tokens"
        ));
        assert!(!is_retryable_error("API error: HTTP 400 Bad Request"));
        assert!(!is_retryable_error("API error: HTTP 404 Not Found"));
        assert!(!is_retryable_error("Unauthorized — check your API key"));
    }

    /// Every HTTP error now carries the provider's message after the status
    /// (`provider::http_error::api_error`). The class comes from the status
    /// alone: a 400 whose message mentions an overload is still the request's
    /// fault, and a 5xx with any message is still worth another attempt.
    #[test]
    fn the_providers_message_after_the_status_does_not_change_the_class() {
        let real = |status, body| {
            crate::agent::provider::http_error::api_error(
                reqwest::StatusCode::from_u16(status).unwrap(),
                body,
                "k",
            )
        };
        let thinking = real(
            400,
            r#"{"error":{"message":"The `content[].thinking` in the thinking mode must be passed back to the API.","type":"invalid_request_error"}}"#,
        );
        assert!(thinking.contains("thinking mode"), "{thinking}");
        assert!(!is_retryable_error(&thinking));
        assert!(!is_retryable_error(&real(
            400,
            r#"{"error":{"message":"upstream overloaded, request cancelled via POST /v1/mtplx/cancel"}}"#
        )));
        assert!(!is_retryable_error(&real(404, "no such model")));
        assert!(is_retryable_error(&real(
            529,
            r#"{"type":"error","error":{"type":"overloaded_error","message":"Overloaded"}}"#
        )));
        assert!(is_retryable_error(&real(
            503,
            "<html>Service Unavailable</html>"
        )));
        assert!(is_retryable_error(&real(
            429,
            r#"{"error":{"message":"rate limited"}}"#
        )));
    }
}
