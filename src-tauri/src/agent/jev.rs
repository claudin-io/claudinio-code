//! Jev — TypeSafe's "System One" decision model — as a cheap judge inside the
//! harness.
//!
//! Jev never generates text. It reads a `state` and answers typed questions
//! (`noul` = P(yes), `choice`, `score`) with calibrated probabilities, at
//! ~$0.04 per million input tokens and ~0.2–0.7 s. The harness uses it for
//! decisions that are judgements of meaning but too frequent to pay a chat
//! model for: is this turn really finished, is the agent repeating itself,
//! which block of a long command output matters.
//!
//! Two ways in, both the user's own credential:
//!   * a TypeSafe key (`jev.api_key`) → `POST api.typesafe.ai/v1/systemone`
//!   * the connected OpenRouter provider → `POST openrouter.ai/api/alpha/decisions`
//!
//! Fail-open by construction: no credential, a timeout, an HTTP error or a body
//! we do not understand all yield `None`, and every caller falls back to what
//! it did before Jev existed. A decision must never be worse than no decision.

use serde::{Deserialize, Serialize};
use serde_json::{json, Map, Value};
use std::time::Duration;

use crate::agent::provider::AgentConfig;

pub const TYPESAFE_URL: &str = "https://api.typesafe.ai/v1/systemone";
/// Pinned, never `jev-latest`: the thresholds in this crate were measured
/// against one model and a silent bump would move them under us.
pub const TYPESAFE_MODEL: &str = "jev-1.13.0";
pub const OPENROUTER_URL: &str = "https://openrouter.ai/api/alpha/decisions";
pub const OPENROUTER_MODEL: &str = "typesafe/jev-1.13-20260917";
/// Provider id of the OpenRouter connection (`commands::providers::OPENROUTER_ID`).
const OPENROUTER_PROVIDER_ID: &str = "openrouter";
/// USD per input token. Output is free. Used when the backend does not report
/// a cost itself (TypeSafe's own API reports only tokens).
pub const PRICE_PER_INPUT_TOKEN: f64 = 0.042 / 1_000_000.0;
/// Jev's state window is 32k tokens; clip by characters well under it so a
/// large output can never turn a decision into a 422.
pub const MAX_STATE_CHARS: usize = 60_000;
const TIMEOUT: Duration = Duration::from_secs(6);
const CONNECT_TIMEOUT: Duration = Duration::from_secs(3);

/// User settings for Jev. On by default, but inert until a credential exists.
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct JevPrefs {
    #[serde(default = "default_true")]
    pub enabled: bool,
    /// A TypeSafe API key. Wins over the OpenRouter connection when both exist.
    #[serde(default)]
    pub api_key: Option<String>,
}

fn default_true() -> bool {
    true
}

impl Default for JevPrefs {
    fn default() -> Self {
        Self {
            enabled: true,
            api_key: None,
        }
    }
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum JevSource {
    TypeSafe,
    OpenRouter,
}

/// Where and how to ask. Resolved from the config at the point of use so a key
/// added mid-session takes effect on the next decision.
#[derive(Debug, Clone)]
pub struct JevBackend {
    pub source: JevSource,
    pub url: String,
    pub api_key: String,
    pub model: String,
}

/// The backend to use, or `None` when Jev is off or there is no credential.
pub fn backend(config: &AgentConfig) -> Option<JevBackend> {
    if !config.jev.enabled {
        return None;
    }
    if let Some(key) = config.jev.api_key.as_deref().map(str::trim) {
        if !key.is_empty() {
            return Some(JevBackend {
                source: JevSource::TypeSafe,
                url: TYPESAFE_URL.into(),
                api_key: key.into(),
                model: TYPESAFE_MODEL.into(),
            });
        }
    }
    let or = config.providers.get(OPENROUTER_PROVIDER_ID)?;
    let key = or.api_key.trim();
    if key.is_empty() {
        return None;
    }
    Some(JevBackend {
        source: JevSource::OpenRouter,
        url: OPENROUTER_URL.into(),
        api_key: key.into(),
        model: OPENROUTER_MODEL.into(),
    })
}

/// A yes/no question. `yes` / `no` say what each answer means — Jev reads the
/// question literally, so both sides are spelled out.
pub fn noul(instructions: &str, yes: &str, no: &str) -> Value {
    json!({
        "type": "noul",
        "instructions": instructions,
        "criteria": { "true": yes, "false": no },
    })
}

/// The answers to one request.
#[derive(Debug, Clone, Default)]
pub struct Decision {
    pub answers: Map<String, Value>,
    pub input_tokens: u64,
    pub cost: f64,
}

impl Decision {
    /// P(yes) of a `noul` question, or `None` if Jev did not answer it.
    pub fn noul(&self, id: &str) -> Option<f64> {
        self.answers.get(id)?.get("noul")?.as_f64()
    }
}

/// The JSON body for one request.
pub fn request_body(backend: &JevBackend, state: &str, questions: &Value) -> Value {
    json!({
        "model": backend.model,
        "state": clip(state, MAX_STATE_CHARS),
        "questions": questions,
    })
}

/// At most `max` bytes of `s`, cut on a char boundary, with an ellipsis when cut.
fn clip(s: &str, max: usize) -> String {
    if s.len() <= max {
        return s.to_string();
    }
    let mut end = max;
    while !s.is_char_boundary(end) {
        end -= 1;
    }
    format!("{}…", &s[..end])
}

/// Parse a decisions response; `None` for anything without an `answers` map.
pub fn parse_response(body: &Value) -> Option<Decision> {
    let answers = body.get("answers")?.as_object()?.clone();
    let usage = body.get("usage");
    let input_tokens = usage
        .and_then(|u| u.get("input_tokens"))
        .and_then(Value::as_u64)
        .unwrap_or(0);
    let cost = usage
        .and_then(|u| u.get("cost"))
        .and_then(Value::as_f64)
        .unwrap_or(input_tokens as f64 * PRICE_PER_INPUT_TOKEN);
    Some(Decision {
        answers,
        input_tokens,
        cost,
    })
}

/// Ask Jev `questions` about `state`. `None` on any failure — see module docs.
pub async fn decide(backend: &JevBackend, state: &str, questions: &Value) -> Option<Decision> {
    let _net = crate::net_activity::NetGuard::begin(
        crate::net_activity::NetSource::Jev,
        &backend.model,
    );
    let client = crate::http::default_client_builder()
        .connect_timeout(CONNECT_TIMEOUT)
        .timeout(TIMEOUT)
        .build()
        .ok()?;
    let mut req = client
        .post(&backend.url)
        .bearer_auth(&backend.api_key)
        .json(&request_body(backend, state, questions));
    if backend.source == JevSource::OpenRouter {
        req = req
            .header("HTTP-Referer", "https://claudin.io")
            .header("X-Title", "Claudinio Code");
    }
    let resp = req.send().await.ok()?;
    if !resp.status().is_success() {
        return None;
    }
    let body: Value = resp.json().await.ok()?;
    parse_response(&body)
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::agent::provider::ProviderEntry;
    use std::io::{Read, Write};
    use std::net::TcpListener;

    fn openrouter_entry(key: &str) -> ProviderEntry {
        ProviderEntry {
            api_key: key.into(),
            base_url: "https://openrouter.ai/api/v1".into(),
            protocol: "openai".into(),
            enabled_models: vec![],
            label: Some("OpenRouter".into()),
            model_pricing: Default::default(),
            model_output_limits: Default::default(),
        }
    }

    #[test]
    fn no_credential_means_no_backend() {
        let cfg = AgentConfig::default();
        assert!(backend(&cfg).is_none());
    }

    #[test]
    fn a_typesafe_key_goes_to_typesafe_with_the_pinned_model() {
        let mut cfg = AgentConfig::default();
        cfg.jev.api_key = Some("ts-key".into());
        let b = backend(&cfg).expect("backend");
        assert_eq!(b.source, JevSource::TypeSafe);
        assert_eq!(b.url, TYPESAFE_URL);
        assert_eq!(b.api_key, "ts-key");
        assert_eq!(b.model, TYPESAFE_MODEL);
    }

    #[test]
    fn the_openrouter_connection_is_used_when_there_is_no_typesafe_key() {
        let mut cfg = AgentConfig::default();
        cfg.providers
            .insert("openrouter".into(), openrouter_entry("or-key"));
        let b = backend(&cfg).expect("backend");
        assert_eq!(b.source, JevSource::OpenRouter);
        assert_eq!(b.url, OPENROUTER_URL);
        assert_eq!(b.api_key, "or-key");
        assert_eq!(b.model, OPENROUTER_MODEL);
    }

    #[test]
    fn a_typesafe_key_wins_over_openrouter() {
        let mut cfg = AgentConfig::default();
        cfg.jev.api_key = Some("ts-key".into());
        cfg.providers
            .insert("openrouter".into(), openrouter_entry("or-key"));
        assert_eq!(backend(&cfg).unwrap().source, JevSource::TypeSafe);
    }

    #[test]
    fn blank_keys_do_not_count_as_credentials() {
        let mut cfg = AgentConfig::default();
        cfg.jev.api_key = Some("   ".into());
        cfg.providers.insert("openrouter".into(), openrouter_entry(""));
        assert!(backend(&cfg).is_none());
    }

    #[test]
    fn disabled_means_no_backend_even_with_a_key() {
        let mut cfg = AgentConfig::default();
        cfg.jev.enabled = false;
        cfg.jev.api_key = Some("ts-key".into());
        assert!(backend(&cfg).is_none());
    }

    #[test]
    fn jev_is_on_by_default_and_old_configs_load() {
        let cfg: AgentConfig =
            serde_json::from_str(r#"{"base_url":"x","api_key":"y","max_rounds":null,"sub_max_rounds":null}"#)
                .unwrap();
        assert!(cfg.jev.enabled);
        assert!(cfg.jev.api_key.is_none());
    }

    fn ts_backend(url: &str) -> JevBackend {
        JevBackend {
            source: JevSource::TypeSafe,
            url: url.into(),
            api_key: "k".into(),
            model: TYPESAFE_MODEL.into(),
        }
    }

    #[test]
    fn the_body_carries_model_state_and_questions() {
        let q = json!({"done": noul("Is it done?", "yes", "no")});
        let body = request_body(&ts_backend(TYPESAFE_URL), "the state", &q);
        assert_eq!(body["model"], TYPESAFE_MODEL);
        assert_eq!(body["state"], "the state");
        assert_eq!(body["questions"]["done"]["type"], "noul");
        assert_eq!(body["questions"]["done"]["criteria"]["true"], "yes");
    }

    #[test]
    fn the_state_is_clipped_on_a_char_boundary() {
        let state = "é".repeat(MAX_STATE_CHARS); // 2 bytes per char
        let body = request_body(&ts_backend(TYPESAFE_URL), &state, &json!({}));
        let sent = body["state"].as_str().unwrap();
        assert!(sent.len() <= MAX_STATE_CHARS + 3);
        assert!(sent.chars().all(|c| c == 'é' || c == '…'));
    }

    #[test]
    fn a_response_with_cost_is_parsed() {
        let d = parse_response(&json!({
            "answers": {"done": {"type": "noul", "noul": 0.91}},
            "usage": {"input_tokens": 500, "output_tokens": 37, "cost": 2.1e-5}
        }))
        .unwrap();
        assert_eq!(d.noul("done"), Some(0.91));
        assert_eq!(d.noul("missing"), None);
        assert_eq!(d.input_tokens, 500);
        assert!((d.cost - 2.1e-5).abs() < 1e-12);
    }

    #[test]
    fn without_a_reported_cost_it_is_priced_from_input_tokens() {
        let d = parse_response(&json!({
            "answers": {"done": {"type": "noul", "noul": 0.1}},
            "usage": {"input_tokens": 1_000_000, "output_tokens": 37}
        }))
        .unwrap();
        assert!((d.cost - 0.042).abs() < 1e-9);
    }

    #[test]
    fn a_body_without_answers_is_no_decision() {
        assert!(parse_response(&json!({"error": {"message": "bad"}})).is_none());
        assert!(parse_response(&json!({"answers": "nope"})).is_none());
    }

    /// One-shot HTTP stub: answers the first request with `status` + `body`
    /// and hands back the raw request it received.
    fn spawn_stub(status: u16, body: &'static str) -> (String, std::thread::JoinHandle<String>) {
        let listener = TcpListener::bind("127.0.0.1:0").unwrap();
        let url = format!("http://{}/v1/systemone", listener.local_addr().unwrap());
        let handle = std::thread::spawn(move || {
            // Never hang the suite when the client does not call at all.
            listener.set_nonblocking(true).unwrap();
            let deadline = std::time::Instant::now() + std::time::Duration::from_secs(10);
            let mut sock = loop {
                match listener.accept() {
                    Ok((s, _)) => break s,
                    Err(_) if std::time::Instant::now() < deadline => {
                        std::thread::sleep(std::time::Duration::from_millis(20))
                    }
                    Err(_) => return String::new(),
                }
            };
            sock.set_nonblocking(false).unwrap();
            let mut req = Vec::new();
            let mut buf = [0u8; 8192];
            loop {
                let n = sock.read(&mut buf).unwrap();
                req.extend_from_slice(&buf[..n]);
                let text = String::from_utf8_lossy(&req);
                if let Some(head_end) = text.find("\r\n\r\n") {
                    let len = text[..head_end]
                        .lines()
                        .find_map(|l| {
                            l.to_ascii_lowercase()
                                .strip_prefix("content-length:")
                                .map(|v| v.trim().parse::<usize>().unwrap())
                        })
                        .unwrap_or(0);
                    if req.len() >= head_end + 4 + len {
                        break;
                    }
                }
                if n == 0 {
                    break;
                }
            }
            let resp = format!(
                "HTTP/1.1 {status} X\r\nContent-Type: application/json\r\nContent-Length: {}\r\nConnection: close\r\n\r\n{body}",
                body.len()
            );
            sock.write_all(resp.as_bytes()).unwrap();
            String::from_utf8_lossy(&req).to_string()
        });
        (url, handle)
    }

    #[tokio::test]
    async fn decide_posts_with_a_bearer_key_and_parses_the_answer() {
        let (url, handle) = spawn_stub(
            200,
            r#"{"answers":{"done":{"type":"noul","noul":0.97}},"usage":{"input_tokens":10}}"#,
        );
        let q = json!({"done": noul("Is it done?", "yes", "no")});
        let d = decide(&ts_backend(&url), "state", &q).await.expect("decision");
        assert_eq!(d.noul("done"), Some(0.97));
        let req = handle.join().unwrap();
        assert!(req.contains("authorization: Bearer k") || req.contains("Authorization: Bearer k"));
        assert!(req.contains(r#""state":"state""#));
    }

    #[tokio::test]
    async fn an_http_error_is_no_decision() {
        let (url, handle) = spawn_stub(529, r#"{"error":"overloaded"}"#);
        assert!(decide(&ts_backend(&url), "s", &json!({})).await.is_none());
        handle.join().unwrap();
    }

    #[tokio::test]
    async fn an_unreachable_backend_is_no_decision() {
        // Bind then drop: nothing listens on this port any more.
        let port = TcpListener::bind("127.0.0.1:0").unwrap().local_addr().unwrap().port();
        let url = format!("http://127.0.0.1:{port}/v1/systemone");
        assert!(decide(&ts_backend(&url), "s", &json!({})).await.is_none());
    }

    /// Live check against the real service. Needs JEV_LIVE_OPENROUTER_KEY.
    #[tokio::test]
    #[ignore]
    async fn live_openrouter_decision() {
        let key = std::env::var("JEV_LIVE_OPENROUTER_KEY").expect("JEV_LIVE_OPENROUTER_KEY");
        let b = JevBackend {
            source: JevSource::OpenRouter,
            url: OPENROUTER_URL.into(),
            api_key: key,
            model: OPENROUTER_MODEL.into(),
        };
        let q = json!({"done": noul("Does the message say the work is finished?", "yes", "no")});
        let d = decide(&b, "Done. All 42 tests pass.", &q).await.expect("decision");
        assert!(d.noul("done").unwrap() > 0.8);
        assert!(d.cost > 0.0);
    }
}
