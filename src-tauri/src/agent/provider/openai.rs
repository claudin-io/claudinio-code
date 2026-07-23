//! OpenAI chat-completions protocol client: serves OpenRouter and every
//! OpenAI-compatible provider from the models.dev catalog. The session keeps
//! its history in the internal Anthropic-shaped format (`Message` /
//! `ContentBlock`); this module translates at the wire boundary in both
//! directions so `StreamOutput` stays byte-compatible with what session.rs
//! expects from the Anthropic client.

use super::{
    AgentConfig, ContentBlock, Message, ResolvedProvider, STREAM_IDLE_TIMEOUT, StreamOutput,
    TEXT_DELTA_THROTTLE, ToolDescription, Usage, maybe_emit_text_delta,
};
use crate::agent::session::AgentEvent;
use futures::StreamExt;
use serde_json::{Value, json};
use std::sync::atomic::{AtomicBool, Ordering};
use tauri::ipc::Channel;

/// Translate the internal Anthropic-shaped history into OpenAI chat messages.
/// `tool_result` blocks become standalone `role:"tool"` messages emitted
/// BEFORE the residual user message (OpenAI requires them directly after the
/// assistant message that carried the tool_calls); assistant `tool_use`
/// blocks become the `tool_calls` array with JSON-string arguments.
fn build_messages(messages: &[Message], system: Option<&str>) -> Vec<Value> {
    let mut out: Vec<Value> = Vec::new();
    if let Some(s) = system {
        out.push(json!({"role": "system", "content": s}));
    }
    for msg in messages {
        match msg.role.as_str() {
            "assistant" => {
                let mut text = String::new();
                let mut tool_calls: Vec<Value> = Vec::new();
                for block in &msg.content {
                    match block {
                        ContentBlock::Text { text: t, .. } => text.push_str(t),
                        ContentBlock::ToolUse {
                            id, name, input, ..
                        } => {
                            tool_calls.push(json!({
                                "id": id,
                                "type": "function",
                                "function": {
                                    "name": name,
                                    "arguments": input.to_string(),
                                }
                            }));
                        }
                        _ => {}
                    }
                }
                let mut m = json!({
                    "role": "assistant",
                    "content": if text.is_empty() { Value::Null } else { Value::String(text) },
                });
                if !tool_calls.is_empty() {
                    m["tool_calls"] = Value::Array(tool_calls);
                }
                out.push(m);
            }
            _ => {
                // User message: split tool_results out first, then the rest.
                let mut parts: Vec<Value> = Vec::new();
                let mut only_text: Option<String> = Some(String::new());
                for block in &msg.content {
                    match block {
                        ContentBlock::ToolResult {
                            tool_use_id,
                            content,
                            ..
                        } => {
                            out.push(json!({
                                "role": "tool",
                                "tool_call_id": tool_use_id,
                                "content": content,
                            }));
                        }
                        ContentBlock::Text { text, .. } => {
                            parts.push(json!({"type": "text", "text": text}));
                            if let Some(acc) = only_text.as_mut() {
                                acc.push_str(text);
                            }
                        }
                        ContentBlock::Image { source, .. } => {
                            parts.push(json!({
                                "type": "image_url",
                                "image_url": {
                                    "url": format!(
                                        "data:{};base64,{}",
                                        source.media_type, source.data
                                    )
                                }
                            }));
                            only_text = None;
                        }
                        ContentBlock::ToolUse { .. } => {}
                    }
                }
                if !parts.is_empty() {
                    // Text-only messages go as a plain string for maximum
                    // compatibility; mixed content uses the parts array.
                    let content = match only_text {
                        Some(t) => Value::String(t),
                        None => Value::Array(parts),
                    };
                    out.push(json!({"role": msg.role, "content": content}));
                }
            }
        }
    }
    out
}

fn build_tools(tools: &[ToolDescription]) -> Option<Vec<Value>> {
    if tools.is_empty() {
        return None;
    }
    Some(
        tools
            .iter()
            .map(|t| {
                json!({
                    "type": "function",
                    "function": {
                        "name": t.name,
                        "description": t.description,
                        "parameters": t.input_schema,
                    }
                })
            })
            .collect(),
    )
}

/// OpenRouter `reasoning.effort` bucket for the configured thinking effort.
fn reasoning_effort(effort: &str) -> &'static str {
    match effort {
        "low" => "low",
        "high" | "xhigh" | "max" => "high",
        _ => "medium",
    }
}

fn build_request(
    rp: &ResolvedProvider,
    config: &AgentConfig,
    messages: &[Message],
    tools: &[ToolDescription],
    system: Option<&str>,
    stream: bool,
    max_tokens: u32,
) -> Value {
    let mut body = json!({
        "model": rp.model,
        "max_tokens": max_tokens,
        "stream": stream,
        "messages": build_messages(messages, system),
    });
    if let Some(t) = build_tools(tools) {
        body["tools"] = Value::Array(t);
    }
    if stream {
        body["stream_options"] = json!({"include_usage": true});
    }
    if rp.provider_id == "openrouter" {
        // OpenRouter-only extras: native cost reporting on the final usage
        // chunk, and the unified reasoning-effort knob. Other providers get
        // neither — unknown fields are a 400 on stricter backends.
        body["usage"] = json!({"include": true});
        if stream {
            body["reasoning"] = json!({"effort": reasoning_effort(&config.thinking_effort)});
        }
    }
    body
}

fn map_finish_reason(reason: &str) -> String {
    match reason {
        "stop" => "end_turn".into(),
        "tool_calls" => "tool_use".into(),
        "length" => "max_tokens".into(),
        other => other.to_string(),
    }
}

/// Parse an OpenAI usage object into the internal `Usage`, estimating cost
/// locally from models.dev pricing when the provider didn't return one
/// (OpenRouter does, via `usage.cost`).
fn parse_usage(v: &Value, pricing: Option<(f64, f64)>) -> Usage {
    let input = v.get("prompt_tokens").and_then(|t| t.as_u64()).unwrap_or(0) as u32;
    let output = v
        .get("completion_tokens")
        .and_then(|t| t.as_u64())
        .unwrap_or(0) as u32;
    let cached = v
        .pointer("/prompt_tokens_details/cached_tokens")
        .and_then(|t| t.as_u64())
        .unwrap_or(0) as u32;
    let mut cost = v.get("cost").and_then(|c| c.as_f64());
    if cost.is_none()
        && let Some((in_price, out_price)) = pricing
    {
        cost = Some(
            f64::from(input) * in_price / 1_000_000.0 + f64::from(output) * out_price / 1_000_000.0,
        );
    }
    Usage {
        input_tokens: input,
        output_tokens: output,
        cache_read_input_tokens: cached,
        cost,
        cost_input: None,
        cost_output: None,
        cost_cache_read: None,
    }
}

/// Per-index accumulator for streamed tool calls: the first chunk for an
/// index carries `id` + `function.name`, later chunks append
/// `function.arguments` fragments.
#[derive(Default)]
struct ToolCallAcc {
    id: String,
    name: String,
    args: String,
}

/// Fold a `choices[0].delta.tool_calls` array into the accumulator map.
fn accumulate_tool_calls(
    acc: &mut std::collections::BTreeMap<usize, ToolCallAcc>,
    tool_calls: &Value,
) {
    let Some(items) = tool_calls.as_array() else {
        return;
    };
    for item in items {
        let idx = item.get("index").and_then(|i| i.as_u64()).unwrap_or(0) as usize;
        let entry = acc.entry(idx).or_default();
        if let Some(id) = item.get("id").and_then(|i| i.as_str())
            && !id.is_empty()
        {
            entry.id = id.to_string();
        }
        if let Some(name) = item.pointer("/function/name").and_then(|n| n.as_str())
            && !name.is_empty()
        {
            entry.name = name.to_string();
        }
        if let Some(args) = item.pointer("/function/arguments").and_then(|a| a.as_str()) {
            entry.args.push_str(args);
        }
    }
}

/// Convert accumulated tool calls into Anthropic-shaped `tool_use` Values —
/// the exact shape session.rs consumes from `StreamOutput.tool_uses`. Empty
/// arguments mean a no-arg tool ({}); non-empty-but-unparseable arguments
/// mean the stream was cut mid-call (max_tokens) and the call is dropped so
/// a half-written tool never executes (mirrors the Anthropic client).
fn finalize_tool_calls(acc: std::collections::BTreeMap<usize, ToolCallAcc>) -> Vec<Value> {
    let mut out = Vec::new();
    for (_, tc) in acc {
        if tc.name.is_empty() {
            continue;
        }
        let input = if tc.args.trim().is_empty() {
            json!({})
        } else {
            match serde_json::from_str::<Value>(&tc.args) {
                Ok(v) => v,
                Err(_) => continue,
            }
        };
        out.push(json!({
            "type": "tool_use",
            "id": tc.id,
            "name": tc.name,
            "input": input,
        }));
    }
    out
}

/// Extract an error message from a chunk or response body like
/// `{"error":{"message":"...","code":...}}` (OpenRouter emits these both as
/// HTTP errors and mid-stream inside a 200 SSE).
fn error_message(v: &Value) -> Option<String> {
    let err = v.get("error")?;
    let msg = err
        .get("message")
        .and_then(|m| m.as_str())
        .unwrap_or("unknown error");
    Some(msg.to_string())
}

fn shape_http_error(status: reqwest::StatusCode, body: &str) -> String {
    if status.as_u16() == 401 {
        return "Unauthorized — check your API key".into();
    }
    if let Ok(v) = serde_json::from_str::<Value>(body)
        && let Some(msg) = error_message(&v)
    {
        return format!("API error: HTTP {status} — {msg}");
    }
    format!("API error: HTTP {status}")
}

/// Streaming chat-completions call, mirroring the Anthropic
/// `stream_message` contract: same idle timeout, interrupt handling,
/// throttled text deltas, thinking events, and `StreamOutput` shape.
#[allow(clippy::too_many_arguments)]
pub async fn stream_message(
    rp: &ResolvedProvider,
    config: &AgentConfig,
    messages: &[Message],
    tools: &[ToolDescription],
    system: Option<&str>,
    event_tx: &Channel<AgentEvent>,
    session_id: &str,
    assistant_text: &mut String,
    interrupt: &AtomicBool,
    emit_text_deltas: bool,
    net_detail: &str,
) -> Result<StreamOutput, String> {
    let client = crate::http::default_client_builder()
        .connect_timeout(std::time::Duration::from_secs(15))
        .build()
        .map_err(|e| format!("failed to build HTTP client: {e}"))?;
    let max_tokens = rp.max_output_tokens.map_or(32_000, |m| m.min(32_000));
    let body = build_request(rp, config, messages, tools, system, true, max_tokens);
    let url = format!("{}/chat/completions", rp.base_url.trim_end_matches('/'));

    let response = client
        .post(&url)
        .header("Content-Type", "application/json")
        .header("Authorization", format!("Bearer {}", rp.api_key))
        .json(&body)
        .send()
        .await
        .map_err(|e| format!("request failed: {e}"))?;

    let status = response.status();
    if !status.is_success() {
        let body = response.text().await.unwrap_or_default();
        return Err(shape_http_error(status, &body));
    }

    let net_guard =
        crate::net_activity::NetGuard::begin(crate::net_activity::NetSource::LlmStream, net_detail);
    net_guard.set_status(status.as_u16());

    let mut dump = if std::env::var("CLAUDINIO_DEBUG_DUMP").is_ok() {
        let dump_path =
            std::path::Path::new("/tmp").join(format!("claudinio_api_dump_{session_id}.txt"));
        std::fs::File::create(&dump_path)
            .map(std::io::BufWriter::new)
            .ok()
    } else {
        None
    };
    if let Some(ref mut f) = dump {
        use std::io::Write;
        let _ = writeln!(f, "--- API RAW DUMP (openai) session={session_id} ---");
    }

    let mut stream = response.bytes_stream();
    let mut buf = String::new();

    let mut text_deltas: Vec<String> = Vec::new();
    let mut thinking_text = String::new();
    let mut tool_acc: std::collections::BTreeMap<usize, ToolCallAcc> =
        std::collections::BTreeMap::new();
    let mut stop_reason: Option<String> = None;
    let mut usage: Option<Usage> = None;

    let mut last_sent_len: usize = 0;
    let mut last_flush = std::time::Instant::now() - TEXT_DELTA_THROTTLE;
    let mut done = false;

    'outer: loop {
        if interrupt.load(Ordering::SeqCst) {
            drop(stream);
            return Ok(StreamOutput {
                text_deltas,
                tool_uses: finalize_tool_calls(tool_acc),
                stop_reason: Some("interrupted".into()),
                usage: None,
                interrupted: true,
            });
        }

        let chunk_result = match tokio::time::timeout(STREAM_IDLE_TIMEOUT, stream.next()).await {
            Ok(Some(r)) => r,
            Ok(None) => break,
            Err(_) => {
                return Err("stream error: no data received for 90s, connection stalled".into());
            }
        };

        let chunk = chunk_result.map_err(|e| format!("stream error: {e}"))?;
        net_guard.add_bytes(chunk.len() as u64);
        let chunk_str = String::from_utf8_lossy(&chunk);
        if let Some(ref mut f) = dump {
            use std::io::Write;
            let _ = writeln!(f, "[CHUNK {} bytes]", chunk.len());
            let _ = writeln!(f, "{}", chunk_str);
        }
        buf.push_str(&chunk_str);

        while let Some(newline) = buf.find('\n') {
            let line = buf[..newline].trim_end_matches('\r').to_string();
            buf = buf[newline + 1..].to_string();

            // OpenAI streams carry only `data:` lines; OpenRouter interleaves
            // ": OPENROUTER PROCESSING" comment keep-alives — skip those.
            let Some(data) = line.strip_prefix("data: ") else {
                continue;
            };
            if data.trim() == "[DONE]" {
                done = true;
                break 'outer;
            }
            process_chunk(
                data,
                rp,
                event_tx,
                assistant_text,
                &mut thinking_text,
                &mut text_deltas,
                &mut tool_acc,
                &mut stop_reason,
                &mut usage,
            )?;
            maybe_emit_text_delta(
                emit_text_deltas,
                event_tx,
                assistant_text,
                &mut last_sent_len,
                &mut last_flush,
            );
        }
    }

    // Some providers close the stream without a trailing newline after the
    // last data line; salvage whatever is left in the buffer.
    if !done
        && let Some(data) = buf.trim().strip_prefix("data: ")
        && data.trim() != "[DONE]"
        && !data.trim().is_empty()
    {
        process_chunk(
            data,
            rp,
            event_tx,
            assistant_text,
            &mut thinking_text,
            &mut text_deltas,
            &mut tool_acc,
            &mut stop_reason,
            &mut usage,
        )?;
    }

    if emit_text_deltas && assistant_text.len() != last_sent_len {
        let _ = event_tx.send(AgentEvent::TextDelta {
            text: assistant_text.clone(),
        });
    }

    let tool_uses = finalize_tool_calls(tool_acc);
    if let Some(ref mut f) = dump {
        use std::io::Write;
        let _ = writeln!(
            f,
            "--- END --- tool_uses={} text_deltas={}",
            tool_uses.len(),
            text_deltas.len()
        );
    }

    Ok(StreamOutput {
        text_deltas,
        tool_uses,
        stop_reason,
        usage,
        interrupted: false,
    })
}

/// Process one streamed chat-completion chunk (the JSON after `data: `).
#[allow(clippy::too_many_arguments)]
fn process_chunk(
    data: &str,
    rp: &ResolvedProvider,
    event_tx: &Channel<AgentEvent>,
    assistant_text: &mut String,
    thinking_text: &mut String,
    text_deltas: &mut Vec<String>,
    tool_acc: &mut std::collections::BTreeMap<usize, ToolCallAcc>,
    stop_reason: &mut Option<String>,
    usage: &mut Option<Usage>,
) -> Result<(), String> {
    let value: Value =
        serde_json::from_str(data).map_err(|e| format!("json parse: {e} data: {data:.100}"))?;

    // Mid-stream error object inside a 200 SSE (OpenRouter does this).
    if let Some(msg) = error_message(&value) {
        return Err(format!("API error: {msg}"));
    }

    if let Some(choice) = value
        .get("choices")
        .and_then(|c| c.as_array())
        .and_then(|c| c.first())
    {
        if let Some(delta) = choice.get("delta") {
            if let Some(text) = delta.get("content").and_then(|t| t.as_str())
                && !text.is_empty()
            {
                text_deltas.push(text.to_string());
                assistant_text.push_str(text);
            }
            // OpenRouter surfaces reasoning as `reasoning`; DeepSeek-style
            // backends use `reasoning_content`. Both feed the same Thinking
            // event the Anthropic thinking_delta path emits.
            let reasoning = delta
                .get("reasoning")
                .and_then(|t| t.as_str())
                .or_else(|| delta.get("reasoning_content").and_then(|t| t.as_str()));
            if let Some(r) = reasoning
                && !r.is_empty()
            {
                thinking_text.push_str(r);
                let _ = event_tx.send(AgentEvent::Thinking(thinking_text.clone()));
            }
            if let Some(tc) = delta.get("tool_calls") {
                accumulate_tool_calls(tool_acc, tc);
            }
        }
        if let Some(reason) = choice.get("finish_reason").and_then(|r| r.as_str()) {
            *stop_reason = Some(map_finish_reason(reason));
        }
    }

    // Usage arrives on a final chunk with empty `choices` (stream_options
    // include_usage) — but some providers attach it to the last delta too.
    if let Some(u) = value.get("usage")
        && !u.is_null()
    {
        *usage = Some(parse_usage(u, rp.pricing));
    }

    Ok(())
}

/// Non-streaming single-shot completion (classify / one_shot path).
pub async fn complete(
    rp: &ResolvedProvider,
    system: &str,
    user: &str,
    max_tokens: u32,
    net_source: crate::net_activity::NetSource,
) -> Result<String, String> {
    let _net_guard = crate::net_activity::NetGuard::begin(net_source, rp.model.as_str());
    let client = crate::http::default_client_builder()
        .connect_timeout(std::time::Duration::from_secs(15))
        .timeout(std::time::Duration::from_secs(90))
        .build()
        .map_err(|e| format!("failed to build HTTP client: {e}"))?;
    let max_tokens = rp
        .max_output_tokens
        .map_or(max_tokens, |m| m.min(max_tokens));
    let body = json!({
        "model": rp.model,
        "max_tokens": max_tokens,
        "messages": [
            {"role": "system", "content": system},
            {"role": "user", "content": user},
        ],
    });
    let url = format!("{}/chat/completions", rp.base_url.trim_end_matches('/'));
    let response = client
        .post(&url)
        .header("Content-Type", "application/json")
        .header("Authorization", format!("Bearer {}", rp.api_key))
        .json(&body)
        .send()
        .await
        .map_err(|e| format!("request failed: {e}"))?;
    _net_guard.set_status(response.status().as_u16());
    let status = response.status();
    if !status.is_success() {
        let body = response.text().await.unwrap_or_default();
        return Err(shape_http_error(status, &body));
    }
    let json: Value = response
        .json()
        .await
        .map_err(|e| format!("failed to parse response: {e}"))?;
    let reply = json
        .pointer("/choices/0/message/content")
        .and_then(|c| c.as_str())
        .unwrap_or_default()
        .to_string();
    Ok(reply)
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::agent::provider::Protocol;
    use tauri::ipc::InvokeResponseBody;

    fn test_rp() -> ResolvedProvider {
        ResolvedProvider {
            protocol: Protocol::OpenAiChat,
            base_url: "https://openrouter.ai/api/v1".into(),
            api_key: "sk-or-test".into(),
            model: "openai/gpt-4o-mini".into(),
            provider_id: "openrouter".into(),
            pricing: None,
            max_output_tokens: None,
        }
    }

    #[test]
    fn test_build_messages_splits_tool_results_before_user_text() {
        let messages = vec![
            Message {
                role: "assistant".into(),
                content: vec![
                    ContentBlock::text("Let me read it."),
                    ContentBlock::tool_use(
                        "call_1",
                        "read_file",
                        serde_json::json!({"path": "a.rs"}),
                    ),
                ],
            },
            Message {
                role: "user".into(),
                content: vec![
                    ContentBlock::tool_result("call_1", "fn main() {}"),
                    ContentBlock::text("continue"),
                ],
            },
        ];
        let out = build_messages(&messages, Some("sys"));
        assert_eq!(out[0]["role"], "system");
        assert_eq!(out[1]["role"], "assistant");
        assert_eq!(out[1]["content"], "Let me read it.");
        assert_eq!(out[1]["tool_calls"][0]["id"], "call_1");
        assert_eq!(out[1]["tool_calls"][0]["function"]["name"], "read_file");
        assert_eq!(
            out[1]["tool_calls"][0]["function"]["arguments"],
            "{\"path\":\"a.rs\"}"
        );
        // tool message must come directly after the assistant tool_calls
        assert_eq!(out[2]["role"], "tool");
        assert_eq!(out[2]["tool_call_id"], "call_1");
        assert_eq!(out[2]["content"], "fn main() {}");
        // residual user text follows
        assert_eq!(out[3]["role"], "user");
        assert_eq!(out[3]["content"], "continue");
    }

    #[test]
    fn test_build_messages_image_becomes_data_url_parts() {
        let messages = vec![Message {
            role: "user".into(),
            content: vec![
                ContentBlock::text("look"),
                ContentBlock::image("image/png", "QUJD", 10, 10),
            ],
        }];
        let out = build_messages(&messages, None);
        assert_eq!(out.len(), 1);
        let parts = out[0]["content"].as_array().unwrap();
        assert_eq!(parts[0]["type"], "text");
        assert_eq!(parts[1]["type"], "image_url");
        assert_eq!(parts[1]["image_url"]["url"], "data:image/png;base64,QUJD");
    }

    #[test]
    fn test_build_messages_assistant_tool_only_has_null_content() {
        let messages = vec![Message {
            role: "assistant".into(),
            content: vec![ContentBlock::tool_use(
                "c1",
                "list_dir",
                serde_json::json!({}),
            )],
        }];
        let out = build_messages(&messages, None);
        assert!(out[0]["content"].is_null());
    }

    #[test]
    fn test_build_request_openrouter_extras_and_generic_omission() {
        let cfg = AgentConfig::default();
        let rp = test_rp();
        let body = build_request(&rp, &cfg, &[], &[], None, true, 32_000);
        assert_eq!(body["usage"], serde_json::json!({"include": true}));
        assert_eq!(body["reasoning"]["effort"], "medium");
        assert_eq!(
            body["stream_options"],
            serde_json::json!({"include_usage": true})
        );

        let mut generic = test_rp();
        generic.provider_id = "deepseek".into();
        let body = build_request(&generic, &cfg, &[], &[], None, true, 32_000);
        assert!(body.get("usage").is_none());
        assert!(body.get("reasoning").is_none());
    }

    #[test]
    fn test_tool_call_accumulation_across_chunks() {
        let mut acc = std::collections::BTreeMap::new();
        accumulate_tool_calls(
            &mut acc,
            &serde_json::json!([
                {"index": 0, "id": "call_a", "function": {"name": "read_file", "arguments": ""}}
            ]),
        );
        accumulate_tool_calls(
            &mut acc,
            &serde_json::json!([{"index": 0, "function": {"arguments": "{\"pa"}}]),
        );
        accumulate_tool_calls(
            &mut acc,
            &serde_json::json!([{"index": 0, "function": {"arguments": "th\":\"x.rs\"}"}}]),
        );
        let out = finalize_tool_calls(acc);
        assert_eq!(out.len(), 1);
        assert_eq!(out[0]["type"], "tool_use");
        assert_eq!(out[0]["id"], "call_a");
        assert_eq!(out[0]["name"], "read_file");
        assert_eq!(out[0]["input"], serde_json::json!({"path": "x.rs"}));
    }

    #[test]
    fn test_truncated_tool_args_drop_and_empty_args_keep() {
        let mut acc = std::collections::BTreeMap::new();
        accumulate_tool_calls(
            &mut acc,
            &serde_json::json!([
                {"index": 0, "id": "cut", "function": {"name": "edit_file", "arguments": "{\"path\": \"a"}},
                {"index": 1, "id": "noargs", "function": {"name": "list_dir"}}
            ]),
        );
        let out = finalize_tool_calls(acc);
        // truncated call dropped, no-arg call kept with {} input
        assert_eq!(out.len(), 1);
        assert_eq!(out[0]["name"], "list_dir");
        assert_eq!(out[0]["input"], serde_json::json!({}));
    }

    #[test]
    fn test_process_chunk_text_finish_and_usage_with_local_cost() {
        let chan = Channel::new(|_: InvokeResponseBody| Ok(()));
        let mut rp = test_rp();
        rp.provider_id = "deepseek".into();
        rp.pricing = Some((1.0, 2.0)); // $1/Mtok in, $2/Mtok out
        let mut assistant_text = String::new();
        let mut thinking_text = String::new();
        let mut text_deltas = Vec::new();
        let mut tool_acc = std::collections::BTreeMap::new();
        let mut stop_reason = None;
        let mut usage = None;

        process_chunk(
            r#"{"choices":[{"delta":{"content":"Hello"},"finish_reason":null}]}"#,
            &rp,
            &chan,
            &mut assistant_text,
            &mut thinking_text,
            &mut text_deltas,
            &mut tool_acc,
            &mut stop_reason,
            &mut usage,
        )
        .unwrap();
        process_chunk(
            r#"{"choices":[{"delta":{},"finish_reason":"stop"}]}"#,
            &rp,
            &chan,
            &mut assistant_text,
            &mut thinking_text,
            &mut text_deltas,
            &mut tool_acc,
            &mut stop_reason,
            &mut usage,
        )
        .unwrap();
        process_chunk(
            r#"{"choices":[],"usage":{"prompt_tokens":1000000,"completion_tokens":500000,"prompt_tokens_details":{"cached_tokens":100}}}"#,
            &rp, &chan, &mut assistant_text, &mut thinking_text,
            &mut text_deltas, &mut tool_acc, &mut stop_reason, &mut usage,
        )
        .unwrap();

        assert_eq!(assistant_text, "Hello");
        assert_eq!(stop_reason.as_deref(), Some("end_turn"));
        let u = usage.unwrap();
        assert_eq!(u.input_tokens, 1_000_000);
        assert_eq!(u.output_tokens, 500_000);
        assert_eq!(u.cache_read_input_tokens, 100);
        // 1M in × $1/M + 0.5M out × $2/M = $2.00 local estimate
        assert!((u.cost.unwrap() - 2.0).abs() < 1e-9);
    }

    #[test]
    fn test_process_chunk_native_cost_wins_over_estimate() {
        let chan = Channel::new(|_: InvokeResponseBody| Ok(()));
        let mut rp = test_rp();
        rp.pricing = Some((1.0, 2.0));
        let mut assistant_text = String::new();
        let mut thinking_text = String::new();
        let mut text_deltas = Vec::new();
        let mut tool_acc = std::collections::BTreeMap::new();
        let mut stop_reason = None;
        let mut usage = None;

        process_chunk(
            r#"{"choices":[],"usage":{"prompt_tokens":10,"completion_tokens":5,"cost":0.1234}}"#,
            &rp,
            &chan,
            &mut assistant_text,
            &mut thinking_text,
            &mut text_deltas,
            &mut tool_acc,
            &mut stop_reason,
            &mut usage,
        )
        .unwrap();
        assert_eq!(usage.unwrap().cost, Some(0.1234));
    }

    #[test]
    fn test_process_chunk_reasoning_feeds_thinking() {
        let chan = Channel::new(|_: InvokeResponseBody| Ok(()));
        let rp = test_rp();
        let mut assistant_text = String::new();
        let mut thinking_text = String::new();
        let mut text_deltas = Vec::new();
        let mut tool_acc = std::collections::BTreeMap::new();
        let mut stop_reason = None;
        let mut usage = None;

        process_chunk(
            r#"{"choices":[{"delta":{"reasoning":"hmm, "}}]}"#,
            &rp,
            &chan,
            &mut assistant_text,
            &mut thinking_text,
            &mut text_deltas,
            &mut tool_acc,
            &mut stop_reason,
            &mut usage,
        )
        .unwrap();
        process_chunk(
            r#"{"choices":[{"delta":{"reasoning_content":"let me check"}}]}"#,
            &rp,
            &chan,
            &mut assistant_text,
            &mut thinking_text,
            &mut text_deltas,
            &mut tool_acc,
            &mut stop_reason,
            &mut usage,
        )
        .unwrap();
        assert_eq!(thinking_text, "hmm, let me check");
        assert!(assistant_text.is_empty());
    }

    #[test]
    fn test_process_chunk_mid_stream_error_is_err() {
        let chan = Channel::new(|_: InvokeResponseBody| Ok(()));
        let rp = test_rp();
        let mut assistant_text = String::new();
        let mut thinking_text = String::new();
        let mut text_deltas = Vec::new();
        let mut tool_acc = std::collections::BTreeMap::new();
        let mut stop_reason = None;
        let mut usage = None;

        let err = process_chunk(
            r#"{"error":{"message":"Provider overloaded","code":502}}"#,
            &rp,
            &chan,
            &mut assistant_text,
            &mut thinking_text,
            &mut text_deltas,
            &mut tool_acc,
            &mut stop_reason,
            &mut usage,
        )
        .unwrap_err();
        assert!(err.contains("Provider overloaded"));
    }

    #[test]
    fn test_reasoning_effort_buckets() {
        assert_eq!(reasoning_effort("low"), "low");
        assert_eq!(reasoning_effort("medium"), "medium");
        assert_eq!(reasoning_effort("high"), "high");
        assert_eq!(reasoning_effort("xhigh"), "high");
        assert_eq!(reasoning_effort("max"), "high");
        assert_eq!(reasoning_effort("garbage"), "medium");
    }
}
