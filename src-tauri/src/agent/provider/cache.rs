//! Prompt-cache breakpoints for the Anthropic Messages API.
//!
//! On the Messages API a prefix is cached only up to an explicit
//! `cache_control` breakpoint: a request without one re-reads its whole
//! context at the full input price, every round. The 2026-10-06 measurement
//! (509 sessions) found $33.01 of input against $1.06 of cache reads, while the
//! claudin.io proxy, whose upstreams cache implicitly, reports ~96% of input as
//! cache reads. This module is the difference for everyone else speaking the
//! Anthropic protocol: direct Anthropic, OpenRouter's Anthropic routes, a
//! company proxy in front of either.
//!
//! The markers are added to the *serialized* request, never to `Message` or
//! `ContentBlock`: those types are also the session JSONL format, and a
//! breakpoint is a property of one request, not of the history.

use super::ResolvedProvider;
use serde_json::{Value, json};
use std::collections::HashSet;
use std::sync::{LazyLock, Mutex};

/// Block types a breakpoint may never sit on. The API rejects `cache_control`
/// on thinking blocks (they are cached as part of the turn around them).
const INELIGIBLE_TYPES: [&str; 2] = ["thinking", "redacted_thinking"];

fn ephemeral() -> Value {
    json!({"type": "ephemeral"})
}

/// Marks `body` (a serialized Messages API request) for the prompt cache.
///
/// Three breakpoints, of the four the API allows:
///
/// 1. `system`, sent as one text block. Tools come before the system prompt in
///    the cache order, so this one covers tools and prompt together; a
///    separate marker on the last tool would add nothing while both are
///    byte-stable for the run (they are: `surface.rs` derives them once).
///    It is what still hits after a prune or a compaction rewrote the history.
/// 2. The last eligible block of the last message: this request writes the
///    whole conversation to the cache.
/// 3. The last eligible block of the most recent *earlier* user message: where
///    the previous request wrote. The API looks back only ~20 blocks from a
///    breakpoint for an earlier cache entry, and one round of parallel tool
///    calls can add more than that, so the read position is marked
///    explicitly rather than left to the lookback. Without it a wide round
///    would silently re-read the whole history cold.
///
/// Any `cache_control` already in the body is removed first, so the function
/// owns every breakpoint, is idempotent and can never exceed the limit.
pub fn apply_cache_breakpoints(body: &mut Value) {
    strip_cache_control(body);
    if let Some(system) = body.get_mut("system") {
        if let Some(text) = system.as_str().filter(|s| !s.is_empty()) {
            *system = json!([{"type": "text", "text": text}]);
        }
        // Already blocks (a second pass): the marker goes back on the last one.
        if let Some(blocks) = system.as_array_mut() {
            mark_last_eligible(blocks);
        }
    }
    let Some(messages) = body.get_mut("messages").and_then(Value::as_array_mut) else {
        return;
    };
    let Some(last) = messages.len().checked_sub(1) else {
        return;
    };
    let earlier_user = messages[..last]
        .iter()
        .rposition(|m| m.get("role").and_then(Value::as_str) == Some("user"));
    mark_last_eligible_block(&mut messages[last]);
    if let Some(i) = earlier_user {
        mark_last_eligible_block(&mut messages[i]);
    }
}

fn strip_cache_control(v: &mut Value) {
    match v {
        Value::Object(map) => {
            map.remove("cache_control");
            map.values_mut().for_each(strip_cache_control);
        }
        Value::Array(items) => items.iter_mut().for_each(strip_cache_control),
        _ => {}
    }
}

fn is_eligible(block: &Value) -> bool {
    let kind = block.get("type").and_then(Value::as_str).unwrap_or("");
    if INELIGIBLE_TYPES.contains(&kind) {
        return false;
    }
    // An empty text block is itself a 400; a marker on it would be twice wrong.
    !(kind == "text"
        && block
            .get("text")
            .and_then(Value::as_str)
            .unwrap_or("")
            .is_empty())
}

fn mark_last_eligible_block(message: &mut Value) {
    let Some(content) = message.get_mut("content") else {
        return;
    };
    // The Messages API also takes `content` as a bare string. Our own types
    // always serialize an array, but a marker needs a block to sit on.
    if let Some(text) = content.as_str() {
        if text.is_empty() {
            return;
        }
        *content = json!([{"type": "text", "text": text}]);
    }
    if let Some(blocks) = content.as_array_mut() {
        mark_last_eligible(blocks);
    }
}

fn mark_last_eligible(blocks: &mut [Value]) {
    if let Some(block) = blocks.iter_mut().rev().find(|b| is_eligible(b))
        && let Some(obj) = block.as_object_mut()
    {
        obj.insert("cache_control".into(), ephemeral());
    }
}

/// Whether requests to `rp` should carry `cache_control` markers.
///
/// - **OpenAI protocol: no.** Caching there is automatic and has no request
///   field; `cached_tokens` is already read back.
/// - **claudin.io: no.** The proxy runs a prompt manager that rewrites the
///   system prompt server-side, and its upstreams cache implicitly (the proxy
///   measures ~96% of executor input as cache reads). Changing the shape of
///   `system` there is production risk with nothing to gain.
/// - **Local engines (llama.cpp, MLX, anything on loopback): no.** A local
///   server keeps its own KV cache; markers are at best ignored and at worst a
///   400 from a shim that does not know the field.
/// - **Every other Anthropic-protocol endpoint: yes.**
pub fn wants_cache_markers(rp: &ResolvedProvider) -> bool {
    if rp.protocol != super::Protocol::Anthropic
        || rp.provider_id == crate::llama::LOCAL_PROVIDER_ID
    {
        return false;
    }
    let Ok(url) = reqwest::Url::parse(rp.base_url.trim()) else {
        return false;
    };
    let Some(host) = url.host_str().map(str::to_ascii_lowercase) else {
        return false;
    };
    if let Ok(ip) = host
        .trim_matches(|c| c == '[' || c == ']')
        .parse::<std::net::IpAddr>()
    {
        return !(ip.is_loopback() || ip.is_unspecified());
    }
    let is_claudinio = host == "claudin.io" || host.ends_with(".claudin.io");
    !(is_claudinio || host == "localhost" || host.ends_with(".localhost"))
}

/// Providers that refused a marked request in this process, keyed by
/// `provider_key`. In memory on purpose: a shim that rejects the field today
/// may be upgraded tomorrow, and a refusal costs one extra request per
/// process, not a setting the user has to find and reset.
static REFUSED: LazyLock<Mutex<HashSet<String>>> = LazyLock::new(Default::default);

fn provider_key(rp: &ResolvedProvider) -> String {
    format!("{}\n{}", rp.provider_id, rp.base_url.trim_end_matches('/'))
}

/// True when a marked request to this provider was refused earlier in the
/// process and markers are no longer sent to it.
pub fn markers_refused(rp: &ResolvedProvider) -> bool {
    REFUSED
        .lock()
        .map(|set| set.contains(&provider_key(rp)))
        .unwrap_or(false)
}

fn remember_refusal(rp: &ResolvedProvider) {
    if let Ok(mut set) = REFUSED.lock() {
        set.insert(provider_key(rp));
    }
}

/// Markers for this request: the provider wants them and has not refused them.
pub fn should_mark(rp: &ResolvedProvider) -> bool {
    wants_cache_markers(rp) && !markers_refused(rp)
}

/// Sends `body` through `post`, marked when `mark` is set, and falls back to
/// the unmarked request once if the marked one is refused with a 400.
///
/// Some OpenAI-compatible shims behind an Anthropic façade validate the body
/// strictly and reject a field they do not know. Any 400 on a marked request
/// is retried without the markers; the provider is remembered as refusing them
/// when the error names `cache_control` or the unmarked retry succeeds. A 400
/// that survives the retry was about something else (an oversized prompt, a
/// bad tool schema) and must not switch caching off for the whole process —
/// on a provider that does cache, that would quietly make every later round
/// 5–10× more expensive.
///
/// `post` resolves to `None` when the user pressed Stop; so does this.
pub(super) async fn post_with_cache_fallback<F, Fut>(
    rp: &ResolvedProvider,
    mark: bool,
    body: Value,
    post: F,
) -> Result<Option<reqwest::Response>, String>
where
    F: Fn(Value) -> Fut,
    Fut: std::future::Future<Output = Option<Result<reqwest::Response, reqwest::Error>>>,
{
    let send = |body: Value| async {
        match post(body).await {
            None => Ok(None),
            Some(r) => r.map(Some).map_err(|e| format!("request failed: {e}")),
        }
    };
    if !mark {
        return send(body).await;
    }
    let mut marked = body.clone();
    apply_cache_breakpoints(&mut marked);
    let Some(response) = send(marked).await? else {
        return Ok(None);
    };
    if response.status() != reqwest::StatusCode::BAD_REQUEST {
        return Ok(Some(response));
    }
    let refusal = response.text().await.unwrap_or_default();
    let Some(retry) = send(body).await? else {
        return Ok(None);
    };
    if refusal.contains("cache_control") || retry.status().is_success() {
        eprintln!(
            "[cache] {} refused cache_control markers (HTTP 400); sending unmarked requests \
             to it for the rest of this process",
            rp.provider_id
        );
        remember_refusal(rp);
    }
    Ok(Some(retry))
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::agent::provider::Protocol;
    use serde_json::json;

    fn marker() -> Value {
        json!({"type": "ephemeral"})
    }

    /// Every `cache_control` in the body, as a path a reader can check:
    /// "system[0]" or "messages[3].content[1]".
    fn marked_paths(body: &Value) -> Vec<String> {
        let mut out = Vec::new();
        if let Some(blocks) = body["system"].as_array() {
            for (i, b) in blocks.iter().enumerate() {
                if b.get("cache_control").is_some() {
                    out.push(format!("system[{i}]"));
                }
            }
        }
        for (m, msg) in body["messages"].as_array().unwrap().iter().enumerate() {
            if let Some(blocks) = msg["content"].as_array() {
                for (i, b) in blocks.iter().enumerate() {
                    if b.get("cache_control").is_some() {
                        out.push(format!("messages[{m}].content[{i}]"));
                    }
                }
            }
        }
        if let Some(tools) = body["tools"].as_array() {
            for (i, t) in tools.iter().enumerate() {
                if t.get("cache_control").is_some() {
                    out.push(format!("tools[{i}]"));
                }
            }
        }
        out
    }

    fn text(t: &str) -> Value {
        json!({"type": "text", "text": t})
    }

    fn tool_use(id: &str) -> Value {
        json!({"type": "tool_use", "id": id, "name": "read_file", "input": {"path": "a"}})
    }

    fn tool_result(id: &str) -> Value {
        json!({"type": "tool_result", "tool_use_id": id, "content": "ok"})
    }

    /// A three-round conversation: prompt → tool call → results → tool call → results.
    fn body_with_history() -> Value {
        json!({
            "model": "claude-x",
            "max_tokens": 1000,
            "stream": true,
            "system": "You are a coding agent.",
            "tools": [
                {"name": "read_file", "description": "d", "input_schema": {}},
                {"name": "edit_file", "description": "d", "input_schema": {}}
            ],
            "messages": [
                {"role": "user", "content": [text("fix the bug")]},
                {"role": "assistant", "content": [text("reading"), tool_use("t1")]},
                {"role": "user", "content": [tool_result("t1")]},
                {"role": "assistant", "content": [tool_use("t2"), tool_use("t3")]},
                {"role": "user", "content": [tool_result("t2"), tool_result("t3")]}
            ]
        })
    }

    #[test]
    fn the_system_prompt_becomes_one_marked_text_block() {
        let mut body = body_with_history();
        apply_cache_breakpoints(&mut body);
        assert_eq!(
            body["system"],
            json!([{"type": "text", "text": "You are a coding agent.", "cache_control": marker()}])
        );
    }

    #[test]
    fn the_last_message_and_the_previous_user_turn_carry_the_moving_pair() {
        let mut body = body_with_history();
        apply_cache_breakpoints(&mut body);
        // The last block of the last message is where this request writes;
        // the last block of the user turn before it is where the previous
        // request wrote, so this one reads everything up to there.
        assert_eq!(
            marked_paths(&body),
            vec![
                "system[0]",
                "messages[2].content[0]",
                "messages[4].content[1]"
            ]
        );
        assert_eq!(body["messages"][4]["content"][1]["cache_control"], marker());
        assert_eq!(body["messages"][2]["content"][0]["cache_control"], marker());
    }

    #[test]
    fn the_tool_list_is_left_unmarked_because_the_system_marker_covers_it() {
        let mut body = body_with_history();
        let tools_before = body["tools"].clone();
        apply_cache_breakpoints(&mut body);
        assert_eq!(body["tools"], tools_before);
    }

    #[test]
    fn never_more_than_four_markers_even_on_an_already_marked_body() {
        let mut body = body_with_history();
        // Marks left over from an earlier pass, or from wherever: the
        // function owns every breakpoint in the request.
        for m in body["messages"].as_array_mut().unwrap() {
            for b in m["content"].as_array_mut().unwrap() {
                b["cache_control"] = marker();
            }
        }
        apply_cache_breakpoints(&mut body);
        apply_cache_breakpoints(&mut body);
        let marks = marked_paths(&body);
        assert!(marks.len() <= 4, "{marks:?}");
        assert_eq!(
            marks,
            vec![
                "system[0]",
                "messages[2].content[0]",
                "messages[4].content[1]"
            ]
        );
    }

    #[test]
    fn thinking_and_empty_text_blocks_are_never_marked() {
        let mut body = json!({
            "system": "s",
            "messages": [
                {"role": "user", "content": [text("first"), text("")]},
                {"role": "assistant", "content": [
                    tool_use("t1"),
                    {"type": "thinking", "thinking": "hmm", "signature": "x"},
                    {"type": "redacted_thinking", "data": "abc"},
                    text("")
                ]}
            ]
        });
        apply_cache_breakpoints(&mut body);
        // The last *eligible* block of each marked message.
        assert_eq!(
            marked_paths(&body),
            vec![
                "system[0]",
                "messages[0].content[0]",
                "messages[1].content[0]"
            ]
        );
    }

    #[test]
    fn a_message_with_no_eligible_block_is_skipped() {
        let mut body = json!({
            "system": "s",
            "messages": [
                {"role": "user", "content": [text("q")]},
                {"role": "assistant", "content": [{"type": "thinking", "thinking": "t", "signature": "x"}]}
            ]
        });
        apply_cache_breakpoints(&mut body);
        assert_eq!(
            marked_paths(&body),
            vec!["system[0]", "messages[0].content[0]"]
        );
    }

    #[test]
    fn a_first_request_has_no_earlier_user_turn_and_marks_only_the_last_message() {
        let mut body = json!({
            "system": "s",
            "messages": [{"role": "user", "content": [text("hello")]}]
        });
        apply_cache_breakpoints(&mut body);
        assert_eq!(
            marked_paths(&body),
            vec!["system[0]", "messages[0].content[0]"]
        );
    }

    #[test]
    fn a_request_without_a_system_prompt_gets_no_system_block() {
        for system in [Value::Null, json!("")] {
            let mut body = body_with_history();
            body["system"] = system.clone();
            apply_cache_breakpoints(&mut body);
            // An empty text block is a 400 on the Messages API; absent stays absent.
            assert_eq!(body["system"], system);
            assert_eq!(
                marked_paths(&body),
                vec!["messages[2].content[0]", "messages[4].content[1]"]
            );
        }
        let mut body = body_with_history();
        body.as_object_mut().unwrap().remove("system");
        apply_cache_breakpoints(&mut body);
        assert!(body.get("system").is_none());
    }

    #[test]
    fn string_content_is_turned_into_one_marked_text_block() {
        let mut body = json!({
            "system": "s",
            "messages": [{"role": "user", "content": "hello"}]
        });
        apply_cache_breakpoints(&mut body);
        assert_eq!(
            body["messages"][0]["content"],
            json!([{"type": "text", "text": "hello", "cache_control": marker()}])
        );
    }

    #[test]
    fn nothing_but_cache_control_changes() {
        let before = body_with_history();
        let mut after = before.clone();
        apply_cache_breakpoints(&mut after);
        fn strip(v: &mut Value) {
            match v {
                Value::Object(map) => {
                    map.remove("cache_control");
                    map.values_mut().for_each(strip);
                }
                Value::Array(items) => items.iter_mut().for_each(strip),
                _ => {}
            }
        }
        strip(&mut after);
        after["system"] = after["system"][0]["text"].clone();
        assert_eq!(after, before);
    }

    fn resolved(protocol: Protocol, provider_id: &str, base_url: &str) -> ResolvedProvider {
        ResolvedProvider {
            protocol,
            base_url: base_url.into(),
            api_key: "k".into(),
            model: "m".into(),
            provider_id: provider_id.into(),
            pricing: None,
            max_output_tokens: None,
        }
    }

    #[test]
    fn direct_anthropic_and_openrouter_anthropic_routes_want_markers() {
        assert!(wants_cache_markers(&resolved(
            Protocol::Anthropic,
            "anthropic",
            "https://api.anthropic.com"
        )));
        assert!(wants_cache_markers(&resolved(
            Protocol::Anthropic,
            "openrouter",
            "https://openrouter.ai/api"
        )));
        // A BYOK override on the default provider pointed at Anthropic.
        assert!(wants_cache_markers(&resolved(
            Protocol::Anthropic,
            "claudinio",
            "https://api.anthropic.com/"
        )));
    }

    #[test]
    fn claudin_io_is_never_marked() {
        for url in [
            "https://api.claudin.io",
            "https://claudin.io",
            "https://staging.api.claudin.io/v1",
            "HTTPS://API.CLAUDIN.IO",
        ] {
            assert!(
                !wants_cache_markers(&resolved(Protocol::Anthropic, "claudinio", url)),
                "{url}"
            );
        }
        // A host that merely ends in the same letters is somebody else.
        assert!(wants_cache_markers(&resolved(
            Protocol::Anthropic,
            "x",
            "https://notclaudin.io"
        )));
    }

    #[test]
    fn local_engines_are_never_marked() {
        for url in [
            "http://localhost:8080",
            "http://127.0.0.1:4000",
            "http://[::1]:9000",
            "http://0.0.0.0:1234",
        ] {
            assert!(
                !wants_cache_markers(&resolved(Protocol::Anthropic, "custom", url)),
                "{url}"
            );
        }
        assert!(!wants_cache_markers(&resolved(
            Protocol::Anthropic,
            crate::llama::LOCAL_PROVIDER_ID,
            "http://192.168.1.5:8080"
        )));
    }

    #[test]
    fn the_openai_protocol_is_never_marked() {
        assert!(!wants_cache_markers(&resolved(
            Protocol::OpenAiChat,
            "openrouter",
            "https://openrouter.ai/api/v1"
        )));
    }

    #[test]
    fn an_unparseable_base_url_is_not_marked() {
        assert!(!wants_cache_markers(&resolved(
            Protocol::Anthropic,
            "x",
            "not a url"
        )));
    }
}

/// The marked request end to end, against the scripted Anthropic stub: what
/// goes on the wire, what comes back, and the fallback when it is refused.
#[cfg(test)]
mod stub_tests {
    use super::*;
    use crate::agent::persist::SessionRecord;
    use crate::agent::provider::{
        AgentConfig, ContentBlock, Message, Protocol, ToolDescription, stream_anthropic,
    };
    use crate::agent::subagent::scripted_model::{Reply, spawn_recording, spawn_with_usage};
    use serde_json::json;
    use std::sync::atomic::AtomicBool;
    use tauri::ipc::{Channel, InvokeResponseBody};

    fn stub_provider(base_url: &str) -> ResolvedProvider {
        ResolvedProvider {
            protocol: Protocol::Anthropic,
            base_url: base_url.into(),
            api_key: "k".into(),
            model: "m".into(),
            // The port makes every test's provider distinct, so one test's
            // remembered refusal never leaks into another.
            provider_id: format!("stub-{base_url}"),
            pricing: None,
            max_output_tokens: Some(8_192),
        }
    }

    fn turn(role: &str, text: &str) -> Message {
        Message {
            role: role.into(),
            content: vec![ContentBlock::text(text)],
        }
    }

    fn history() -> Vec<Message> {
        vec![
            turn("user", "first question"),
            turn("assistant", "first answer"),
            turn("user", "second question"),
        ]
    }

    fn tools() -> Vec<ToolDescription> {
        vec![ToolDescription {
            name: "read_file".into(),
            description: "read".into(),
            input_schema: json!({"type": "object"}),
        }]
    }

    async fn send(
        rp: &ResolvedProvider,
        mark: bool,
    ) -> Result<crate::agent::provider::StreamOutput, String> {
        let chan = Channel::new(|_: InvokeResponseBody| Ok(()));
        let mut text = String::new();
        stream_anthropic(
            rp,
            mark,
            &AgentConfig::default(),
            &history(),
            &tools(),
            Some("You are a coding agent."),
            &chan,
            "cache-test",
            &mut text,
            &AtomicBool::new(false),
            false,
            "test",
        )
        .await
    }

    fn body(raw: &str) -> Value {
        serde_json::from_str(raw).expect("a JSON request body")
    }

    #[tokio::test]
    async fn a_marked_request_carries_exactly_the_three_breakpoints_and_its_cache_usage_is_recorded()
     {
        let usage = json!({
            "input_tokens": 40,
            "cache_read_input_tokens": 3_000,
            "cache_creation_input_tokens": 500,
            "output_tokens": 1
        });
        let (base, _, bodies) = spawn_with_usage(vec![Reply::Text("ok")], Some(usage)).await;
        let rp = stub_provider(&base);
        let out = send(&rp, true).await.expect("the request succeeds");

        let sent = body(&bodies.lock().unwrap()[0]);
        assert_eq!(
            sent["system"],
            json!([{"type": "text", "text": "You are a coding agent.",
                    "cache_control": {"type": "ephemeral"}}])
        );
        assert_eq!(
            sent["messages"][2]["content"][0]["cache_control"],
            json!({"type": "ephemeral"})
        );
        assert_eq!(
            sent["messages"][0]["content"][0]["cache_control"],
            json!({"type": "ephemeral"})
        );
        assert!(
            sent["messages"][1]["content"][0]
                .get("cache_control")
                .is_none()
        );
        assert!(sent["tools"][0].get("cache_control").is_none());
        assert_eq!(sent.to_string().matches("cache_control").count(), 3);

        // The per-request record shows what the marked request bought.
        let u = out.usage.expect("usage parsed from the stream");
        assert_eq!(u.cache_read_input_tokens, 3_000);
        assert_eq!(u.cache_creation_input_tokens, 500);
        match SessionRecord::usage(1, "stub/m", Some(&u), None) {
            SessionRecord::Usage {
                input_tokens,
                cache_read_tokens,
                cache_creation_tokens,
                ..
            } => {
                assert_eq!(input_tokens, 40);
                assert_eq!(cache_read_tokens, 3_000);
                assert_eq!(cache_creation_tokens, 500);
            }
            other => panic!("expected a usage record, got {other:?}"),
        }
    }

    #[tokio::test]
    async fn an_unmarked_request_carries_no_cache_control_and_keeps_system_a_string() {
        let (base, _, bodies) = spawn_recording(vec![Reply::Text("ok")]).await;
        send(&stub_provider(&base), false).await.unwrap();
        let sent = body(&bodies.lock().unwrap()[0]);
        assert!(!sent.to_string().contains("cache_control"));
        assert_eq!(sent["system"], json!("You are a coding agent."));
    }

    #[tokio::test]
    async fn a_refused_marked_request_is_retried_unmarked_once_and_the_provider_is_remembered() {
        let (base, _, bodies) = spawn_recording(vec![
            Reply::Status(
                400,
                r#"{"type":"error","error":{"type":"invalid_request_error","message":"cache_control: Extra inputs are not permitted"}}"#,
            ),
            Reply::Text("ok"),
        ])
        .await;
        let rp = stub_provider(&base);
        let out = send(&rp, true).await.expect("the unmarked retry succeeds");
        assert_eq!(out.text_deltas.concat(), "ok");

        let bodies = bodies.lock().unwrap().clone();
        assert_eq!(bodies.len(), 2, "one refusal, one retry");
        assert!(bodies[0].contains("cache_control"));
        assert!(!bodies[1].contains("cache_control"));
        assert_eq!(body(&bodies[1])["system"], json!("You are a coding agent."));
        assert!(markers_refused(&rp));
        // A provider the predicate would mark is no longer marked.
        let mut public = rp.clone();
        public.base_url = "https://anthropic.example.com".into();
        assert!(should_mark(&public));
        remember_refusal(&public);
        assert!(!should_mark(&public));
    }

    #[tokio::test]
    async fn any_400_on_a_marked_request_is_retried_and_remembered_when_the_retry_succeeds() {
        let (base, _, bodies) = spawn_recording(vec![
            Reply::Status(400, r#"{"error":{"message":"unknown field"}}"#),
            Reply::Text("ok"),
        ])
        .await;
        let rp = stub_provider(&base);
        send(&rp, true).await.expect("the unmarked retry succeeds");
        assert_eq!(bodies.lock().unwrap().len(), 2);
        assert!(markers_refused(&rp));
    }

    #[tokio::test]
    async fn a_400_that_survives_the_unmarked_retry_does_not_switch_caching_off() {
        let refusal = r#"{"error":{"message":"prompt is too long"}}"#;
        let (base, _, bodies) = spawn_recording(vec![
            Reply::Status(400, refusal),
            Reply::Status(400, refusal),
        ])
        .await;
        let rp = stub_provider(&base);
        let err = send(&rp, true).await.expect_err("both attempts refused");
        assert!(err.contains("400"), "{err}");
        assert_eq!(bodies.lock().unwrap().len(), 2, "retried once, not more");
        assert!(!markers_refused(&rp));
    }

    #[tokio::test]
    async fn a_400_on_an_unmarked_request_is_not_retried() {
        let (base, _, bodies) =
            spawn_recording(vec![Reply::Status(400, r#"{"error":{"message":"bad"}}"#)]).await;
        let rp = stub_provider(&base);
        send(&rp, false).await.expect_err("refused");
        assert_eq!(bodies.lock().unwrap().len(), 1);
        assert!(!markers_refused(&rp));
    }
}
