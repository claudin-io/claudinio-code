//! The message a provider's HTTP error becomes:
//! `API error: HTTP <status> — <the provider's message>`.
//!
//! Until 2026-10-18 the body was read and thrown away, so the session's error
//! record and the UI said only "API error: HTTP 400 Bad Request". That is what
//! hid the DeepSeek thinking-mode refusal the first live eval hit after a
//! prune (`prune::apply`): the reason was in the body all along. The prefix is
//! unchanged — `retry::is_retryable_error` classifies by the status right
//! after it — and the message only ever comes after ` — `.

use serde_json::Value;

/// Characters of the provider's message kept in the error. Enough for every
/// provider message seen so far; a proxy's HTML error page is cut well before
/// it floods the timeline.
pub(crate) const DETAIL_CHARS: usize = 300;

/// A key shorter than this is a placeholder (a keyless local server sends
/// none, tests use "k"), not a secret; redacting it would garble the message.
const MIN_SECRET_CHARS: usize = 8;

/// `API error: HTTP <status>`, followed by ` — <message>` when the body has
/// one: the `error.message` of an Anthropic or OpenAI error object, an
/// `error` that is a bare string, or else the start of the body itself. The
/// message is one line, at most [`DETAIL_CHARS`] characters, and never
/// contains `api_key` (a provider may quote the key it refused). Request
/// headers are never part of it: only the response body is read.
pub(crate) fn api_error(status: reqwest::StatusCode, body: &str, api_key: &str) -> String {
    let detail = provider_message(body).unwrap_or_else(|| body.to_string());
    let detail = if api_key.len() < MIN_SECRET_CHARS {
        detail
    } else {
        detail.replace(api_key, "[redacted]")
    };
    let detail = one_line(&detail);
    if detail.is_empty() {
        return format!("API error: HTTP {status}");
    }
    format!("API error: HTTP {status} — {detail}")
}

/// The Anthropic one-shot calls' (the completion judge, `one_shot`) check of
/// a response: a success passes through, a failure becomes the error they
/// return — a plan's budget refusal on claudin.io as the budget marker,
/// anything else as [`api_error`].
pub(crate) async fn one_shot_checked(
    response: reqwest::Response,
    rp: &super::ResolvedProvider,
) -> Result<reqwest::Response, String> {
    if response.status().is_success() {
        return Ok(response);
    }
    let status = response.status();
    let body = response.text().await.unwrap_or_default();
    Err(
        match super::budget_exceeded_message(&body).filter(|_| rp.is_claudinio()) {
            Some(m) => format!("{}{m}", super::BUDGET_EXCEEDED_MARKER),
            None => api_error(status, &body, &rp.api_key),
        },
    )
}

/// `error.message` (Anthropic: `{"type":"error","error":{"type":…,"message":…}}`,
/// OpenAI and OpenRouter: `{"error":{"message":…}}`) or a bare-string `error`.
fn provider_message(body: &str) -> Option<String> {
    let v: Value = serde_json::from_str(body).ok()?;
    let err = v.get("error")?;
    match err {
        Value::String(s) => Some(s.clone()),
        _ => err
            .get("message")
            .and_then(Value::as_str)
            .map(str::to_string),
    }
}

/// Whitespace collapsed to single spaces, clipped to [`DETAIL_CHARS`] with an
/// ellipsis when anything was cut.
fn one_line(s: &str) -> String {
    let collapsed = s.split_whitespace().collect::<Vec<_>>().join(" ");
    if collapsed.chars().count() <= DETAIL_CHARS {
        return collapsed;
    }
    let mut out: String = collapsed.chars().take(DETAIL_CHARS).collect();
    out.push('…');
    out
}

#[cfg(test)]
mod tests {
    use super::*;
    use reqwest::StatusCode;

    #[test]
    fn an_anthropic_error_body_gives_its_message() {
        let body = r#"{"type":"error","error":{"type":"invalid_request_error","message":"The `content[].thinking` in the thinking mode must be passed back to the API."}}"#;
        assert_eq!(
            api_error(StatusCode::BAD_REQUEST, body, "sk-secret"),
            "API error: HTTP 400 Bad Request — The `content[].thinking` in the thinking mode \
             must be passed back to the API."
        );
    }

    #[test]
    fn an_openai_error_body_gives_its_message() {
        let body = r#"{"error":{"message":"This model's maximum context length is 65536 tokens.","type":"invalid_request_error","code":"context_length_exceeded"}}"#;
        assert_eq!(
            api_error(StatusCode::BAD_REQUEST, body, "sk-secret"),
            "API error: HTTP 400 Bad Request — This model's maximum context length is 65536 tokens."
        );
    }

    #[test]
    fn an_error_that_is_a_bare_string_is_the_message() {
        assert_eq!(
            api_error(
                StatusCode::from_u16(529).unwrap(),
                r#"{"error":"overloaded"}"#,
                "k"
            ),
            "API error: HTTP 529 <unknown status code> — overloaded"
        );
    }

    #[test]
    fn a_body_that_is_not_an_error_object_is_quoted_on_one_line_and_clipped() {
        let body = format!(
            "upstream connect error\n  or disconnect\r\n{}",
            "x".repeat(1_000)
        );
        let msg = api_error(StatusCode::BAD_GATEWAY, &body, "k");
        assert!(
            msg.starts_with(
                "API error: HTTP 502 Bad Gateway — upstream connect error or disconnect x"
            ),
            "{msg}"
        );
        assert!(!msg.contains('\n') && !msg.contains('\r'), "{msg}");
        let detail = msg.split_once(" — ").unwrap().1;
        assert!(
            detail.chars().count() <= DETAIL_CHARS + 1,
            "{}",
            detail.len()
        );
        assert!(detail.ends_with('…'), "a clipped body says so: {detail}");
    }

    #[test]
    fn an_empty_body_adds_nothing() {
        assert_eq!(
            api_error(StatusCode::BAD_REQUEST, "  \n", "k"),
            "API error: HTTP 400 Bad Request"
        );
    }

    /// A placeholder key (a keyless local server, a test's "k") is not a
    /// secret, and replacing every "k" would garble the message.
    #[test]
    fn a_key_too_short_to_be_a_secret_is_not_redacted() {
        let body = r#"{"error":{"message":"thinking mode"}}"#;
        assert_eq!(
            api_error(StatusCode::BAD_REQUEST, body, "k"),
            "API error: HTTP 400 Bad Request — thinking mode"
        );
    }

    #[test]
    fn the_api_key_never_appears_in_the_message() {
        let body = r#"{"error":{"message":"Incorrect API key provided: sk-live-123456."}}"#;
        let msg = api_error(StatusCode::FORBIDDEN, body, "sk-live-123456");
        assert!(!msg.contains("sk-live-123456"), "{msg}");
        assert!(msg.contains("Incorrect API key provided"), "{msg}");
        // Nor in a body that is not JSON.
        let msg = api_error(
            StatusCode::FORBIDDEN,
            "bad key sk-live-123456",
            "sk-live-123456",
        );
        assert!(!msg.contains("sk-live-123456"), "{msg}");
    }

    // The detail is cut at DETAIL_CHARS characters, not bytes: a provider's
    // message in Portuguese, Chinese or with emoji is cut whole.
    #[test]
    fn a_long_multibyte_detail_is_cut_on_characters() {
        let detail = one_line(&"não — 日本 😀 ".repeat(100));
        assert_eq!(detail.chars().count(), DETAIL_CHARS + 1);
        assert!(detail.ends_with('…'));
    }
}

/// Every `API error: HTTP` site, against the scripted stub refusing the
/// request with the DeepSeek 400 that a bare status hid in the first eval.
#[cfg(test)]
mod stub_tests {
    use crate::agent::provider::{
        AgentConfig, ContentBlock, Message, Protocol, ResolvedProvider, openai, stream_anthropic,
    };
    use crate::agent::subagent::scripted_model::{Reply, spawn_recording};
    use std::sync::atomic::AtomicBool;
    use tauri::ipc::{Channel, InvokeResponseBody};

    const REFUSAL: &str = r#"{"type":"error","error":{"type":"invalid_request_error","message":"The `content[].thinking` in the thinking mode must be passed back to the API."}}"#;
    const EXPECTED: &str = "API error: HTTP 400 Bad Request — The `content[].thinking` in the \
                            thinking mode must be passed back to the API.";

    fn provider(base_url: &str, protocol: Protocol) -> ResolvedProvider {
        ResolvedProvider {
            protocol,
            base_url: base_url.into(),
            api_key: "sk-stub-key".into(),
            model: "m".into(),
            provider_id: format!("stub-{base_url}"),
            pricing: None,
            max_output_tokens: Some(8_192),
        }
    }

    fn history() -> Vec<Message> {
        vec![Message {
            role: "user".into(),
            content: vec![ContentBlock::text("hi")],
        }]
    }

    #[tokio::test]
    async fn an_anthropic_stream_refusal_carries_the_providers_message() {
        let (base, _, _) = spawn_recording(vec![Reply::Status(400, REFUSAL)]).await;
        let chan = Channel::new(|_: InvokeResponseBody| Ok(()));
        let mut text = String::new();
        let err = stream_anthropic(
            &provider(&base, Protocol::Anthropic),
            false,
            &AgentConfig::default(),
            &history(),
            &[],
            Some("system"),
            &chan,
            "http-error-test",
            &mut text,
            &AtomicBool::new(false),
            false,
            "test",
        )
        .await
        .expect_err("refused");
        assert_eq!(err, EXPECTED);
    }

    #[tokio::test]
    async fn an_openai_stream_refusal_carries_the_providers_message() {
        let (base, _, _) = spawn_recording(vec![Reply::Status(400, REFUSAL)]).await;
        let chan = Channel::new(|_: InvokeResponseBody| Ok(()));
        let mut text = String::new();
        let err = openai::stream_message(
            &provider(&base, Protocol::OpenAiChat),
            &AgentConfig::default(),
            &history(),
            &[],
            Some("system"),
            &chan,
            "http-error-test",
            &mut text,
            &AtomicBool::new(false),
            false,
            "test",
        )
        .await
        .expect_err("refused");
        assert_eq!(err, EXPECTED);
    }

    /// The completion judge and `one_shot` resolve their provider from the
    /// live config, so they are checked through the step they share: a
    /// refusal carries the provider's message, a claudin.io budget refusal
    /// the budget marker, and a success passes through.
    #[tokio::test]
    async fn an_anthropic_one_shot_refusal_carries_the_providers_message() {
        const BUDGET: &str =
            r#"{"error":{"message":"Claudinio: Budget exceeded for window '1h'."}}"#;
        let (base, _, _) = spawn_recording(vec![
            Reply::Status(400, REFUSAL),
            Reply::Status(500, BUDGET),
            Reply::Text("ok"),
        ])
        .await;
        let send = || async {
            reqwest::Client::new()
                .post(format!("{base}/v1/messages"))
                .json(&serde_json::json!({}))
                .send()
                .await
                .unwrap()
        };
        let anthropic = provider(&base, Protocol::Anthropic);
        let err = super::one_shot_checked(send().await, &anthropic)
            .await
            .expect_err("refused");
        assert_eq!(err, EXPECTED);
        let claudinio = ResolvedProvider {
            provider_id: "claudinio".into(),
            ..provider(&base, Protocol::Anthropic)
        };
        let err = super::one_shot_checked(send().await, &claudinio)
            .await
            .expect_err("over budget");
        assert!(
            err.starts_with(crate::agent::provider::BUDGET_EXCEEDED_MARKER),
            "{err}"
        );
        assert!(
            super::one_shot_checked(send().await, &anthropic)
                .await
                .is_ok()
        );
    }

    #[tokio::test]
    async fn an_openai_one_shot_refusal_carries_the_providers_message() {
        let (base, _, _) = spawn_recording(vec![Reply::Status(400, REFUSAL)]).await;
        let err = openai::complete(
            &provider(&base, Protocol::OpenAiChat),
            "system",
            "hi",
            64,
            crate::net_activity::NetSource::LlmOneShot,
        )
        .await
        .expect_err("refused");
        assert_eq!(err, EXPECTED);
    }
}
