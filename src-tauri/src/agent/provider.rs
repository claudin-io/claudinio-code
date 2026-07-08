use crate::agent::session::AgentEvent;
use futures::StreamExt;
use serde::{Deserialize, Serialize};
use serde_json::Value;
use std::fmt;
use std::sync::atomic::{AtomicBool, Ordering};
use tauri::ipc::Channel;

const ANTHROPIC_VERSION: &str = "2023-06-01";

/// Max time to wait for the *next* SSE chunk before treating the connection
/// as dead. Resets on every chunk received, so a long-but-healthy stream
/// (many minutes of steady deltas) is never killed by this — only a stalled
/// connection (e.g. the socket surviving sleep/network-change with no more
/// bytes coming) is.
const STREAM_IDLE_TIMEOUT: std::time::Duration = std::time::Duration::from_secs(90);

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct AgentConfig {
    pub base_url: String,
    pub api_key: String,
    /// Legacy single model field — kept for backward compat with old config.json.
    /// New configs use brain_model + builder_model.
    #[serde(default)]
    pub model: String,
    /// Model used in Brain (planning) mode.
    #[serde(default = "default_claudinio")]
    pub brain_model: String,
    /// Model used in Builder (execution) mode.
    #[serde(default = "default_claudinio")]
    pub builder_model: String,
    /// Max tool-call rounds for the main agent loop. None = infinite.
    pub max_rounds: Option<usize>,
    /// Max tool-call rounds for subagents. None = infinite.
    pub sub_max_rounds: Option<usize>,
    /// YOLO mode: auto-approve all tool calls except those in yolo_blacklist.
    #[serde(default)]
    pub yolo_mode: bool,
    /// Tool names that still require approval even when yolo_mode is on.
    #[serde(default)]
    pub yolo_blacklist: Vec<String>,
    /// Base URL for account/app-bridge services (login exchange, websearch).
    /// Distinct from `base_url`, which is the `/v1/*` inference endpoint.
    #[serde(default = "default_services_url")]
    pub services_url: String,
    /// Login the active api_key is associated with, if it was obtained via
    /// `login_with_claudinio` rather than pasted manually.
    #[serde(default)]
    pub account_login: Option<String>,
    /// Subscription tier of the linked account, as reported at login time.
    #[serde(default)]
    pub account_tier: Option<String>,
    /// Max golden-loop cycles before the run stops with "max_golden_cycles".
    /// None falls back to the built-in default (5); Some(0) disables the loop.
    #[serde(default)]
    pub max_golden_cycles: Option<usize>,
    /// Max consecutive cycles without golden-task progress before the run
    /// stops with "golden_stalled". None falls back to the default (2).
    #[serde(default)]
    pub max_golden_stalls: Option<usize>,
}

fn default_claudinio() -> String {
    "claudinio".into()
}

fn default_services_url() -> String {
    "https://claudin.io".into()
}

impl Default for AgentConfig {
    fn default() -> Self {
        Self {
            base_url: "https://api.claudin.io".into(),
            api_key: String::new(),
            model: "claudinio".into(),
            brain_model: "claudinio".into(),
            builder_model: "claudinio".into(),
            max_rounds: None,
            sub_max_rounds: None,
            yolo_mode: false,
            yolo_blacklist: Vec::new(),
            services_url: default_services_url(),
            account_login: None,
            account_tier: None,
            max_golden_cycles: None,
            max_golden_stalls: None,
        }
    }
}

impl AgentConfig {
    /// Resolve which model to use for a given session mode.
    pub fn model_for_mode(&self, mode: &str) -> &str {
        match mode {
            "brain" | "pensador" => &self.brain_model,
            _ => &self.builder_model,
        }
    }
}

pub fn config_path() -> Result<std::path::PathBuf, String> {
    let dir = dirs::config_dir().ok_or("no config dir")?.join("claudinio-code");
    std::fs::create_dir_all(&dir).map_err(|e| format!("create config dir: {e}"))?;
    Ok(dir.join("config.json"))
}

/// Load AgentConfig from the config file, migrating old configs that only have
/// a single `model` field to the new `brain_model` + `builder_model`.
pub fn load_config() -> AgentConfig {
    let path = match config_path() {
        Ok(p) => p,
        Err(_) => return AgentConfig::default(),
    };
    let s = match std::fs::read_to_string(&path) {
        Ok(s) => s,
        Err(_) => return AgentConfig::default(),
    };
    let mut cfg: serde_json::Value = match serde_json::from_str(&s) {
        Ok(v) => v,
        Err(_) => return AgentConfig::default(),
    };
    // Migration: if the old `model` field exists but `brain_model`/`builder_model`
    // don't, seed both from `model`.
    if cfg.get("brain_model").is_none() || cfg.get("builder_model").is_none() {
        let legacy = cfg
            .get("model")
            .and_then(|v| v.as_str())
            .unwrap_or("claudinio")
            .to_string();
        if cfg.get("brain_model").is_none() {
            cfg["brain_model"] = serde_json::json!(legacy);
        }
        if cfg.get("builder_model").is_none() {
            cfg["builder_model"] = serde_json::json!(legacy);
        }
    }
    serde_json::from_value(cfg).unwrap_or_default()
}

pub fn save_config(config: &AgentConfig) {
    if let Ok(path) = config_path() {
        if let Ok(json) = serde_json::to_string_pretty(config) {
            let _ = std::fs::write(path, json);
        }
    }
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct Message {
    pub role: String,
    pub content: Vec<ContentBlock>,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
#[serde(untagged)]
pub enum ContentBlock {
    Text {
        #[serde(rename = "type")]
        type_: String,
        text: String,
    },
    Image {
        #[serde(rename = "type")]
        type_: String,
        source: ImageSource,
    },
    ToolUse {
        #[serde(rename = "type")]
        type_: String,
        id: String,
        name: String,
        input: Value,
    },
    ToolResult {
        #[serde(rename = "type")]
        type_: String,
        tool_use_id: String,
        content: String,
    },
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct ImageSource {
    #[serde(rename = "type")]
    pub type_: String,
    #[serde(rename = "media_type")]
    pub media_type: String,
    pub data: String,
}

impl ContentBlock {
    pub fn text(text: impl Into<String>) -> Self {
        ContentBlock::Text {
            type_: "text".into(),
            text: text.into(),
        }
    }

    pub fn image(media_type: impl Into<String>, data: impl Into<String>) -> Self {
        ContentBlock::Image {
            type_: "image".into(),
            source: ImageSource {
                type_: "base64".into(),
                media_type: media_type.into(),
                data: data.into(),
            },
        }
    }

    pub fn tool_use(id: impl Into<String>, name: impl Into<String>, input: Value) -> Self {
        ContentBlock::ToolUse {
            type_: "tool_use".into(),
            id: id.into(),
            name: name.into(),
            input,
        }
    }

    pub fn tool_result(tool_use_id: impl Into<String>, content: impl Into<String>) -> Self {
        ContentBlock::ToolResult {
            type_: "tool_result".into(),
            tool_use_id: tool_use_id.into(),
            content: content.into(),
        }
    }

    /// Extract text from a Text block. Used in tests.
    #[allow(dead_code)]
    pub fn get_text(&self) -> Option<&str> {
        match self {
            ContentBlock::Text { text, .. } => Some(text.as_str()),
            _ => None,
        }
    }
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct ToolDescription {
    pub name: String,
    pub description: String,
    pub input_schema: Value,
}

#[derive(Serialize)]
struct RequestBody {
    model: String,
    max_tokens: u32,
    stream: bool,
    messages: Vec<Message>,
    #[serde(skip_serializing_if = "Option::is_none")]
    tools: Option<Vec<ToolDescription>>,
    system: Option<String>,
}

#[derive(Debug, Clone, Serialize, Deserialize, Default)]
pub struct Usage {
    pub input_tokens: u32,
    pub output_tokens: u32,
    #[serde(default)]
    pub cache_read_input_tokens: u32,
    /// Cost in USD as returned by the provider (if supported), otherwise None.
    /// We calculate an estimate when the provider does not supply it.
    #[serde(default)]
    pub cost: Option<f64>,
    /// Markup cost breakdown in USD, as reported by the litellm proxy's
    /// cost_injector middleware. None when the provider doesn't inject it
    /// (unpriced model, older proxy deploy) — falls back to local estimate.
    #[serde(default)]
    pub cost_input: Option<f64>,
    #[serde(default)]
    pub cost_output: Option<f64>,
    #[serde(default)]
    pub cost_cache_read: Option<f64>,
}

/// Merge usage fields from an SSE event into the accumulated usage.
/// Anthropic reports input_tokens in message_start and output_tokens in
/// message_delta; other providers (claudin.io) send everything in
/// message_delta with zeros in message_start — so merge, never replace,
/// and only take token counts that are > 0.
fn merge_usage(usage: &mut Option<Usage>, value: &Value) {
    let Some(obj) = value.as_object() else { return };
    let u = usage.get_or_insert_with(Usage::default);
    if let Some(v) = obj.get("input_tokens").and_then(|v| v.as_u64()) {
        if v > 0 {
            u.input_tokens = v as u32;
        }
    }
    if let Some(v) = obj.get("output_tokens").and_then(|v| v.as_u64()) {
        if v > 0 {
            u.output_tokens = v as u32;
        }
    }
    if let Some(v) = obj.get("cache_read_input_tokens").and_then(|v| v.as_u64()) {
        if v > 0 {
            u.cache_read_input_tokens = v as u32;
        }
    }
    if let Some(v) = obj.get("cost").and_then(|v| v.as_f64()) {
        u.cost = Some(v);
    }
    if let Some(v) = obj.get("cost_input").and_then(|v| v.as_f64()) {
        u.cost_input = Some(v);
    }
    if let Some(v) = obj.get("cost_output").and_then(|v| v.as_f64()) {
        u.cost_output = Some(v);
    }
    if let Some(v) = obj.get("cost_cache_read").and_then(|v| v.as_f64()) {
        u.cost_cache_read = Some(v);
    }
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct StreamOutput {
    pub text_deltas: Vec<String>,
    pub tool_uses: Vec<Value>,
    pub stop_reason: Option<String>,
    pub usage: Option<Usage>,
    pub interrupted: bool,
}

pub async fn stream_message(
    config: &AgentConfig,
    model: &str,
    messages: &[Message],
    tools: &[ToolDescription],
    system: Option<&str>,
    event_tx: &Channel<AgentEvent>,
    session_id: &str,
    assistant_text: &mut String,
    interrupt: &AtomicBool,
) -> Result<StreamOutput, String> {
    let client = reqwest::Client::builder()
        .connect_timeout(std::time::Duration::from_secs(15))
        .build()
        .map_err(|e| format!("failed to build HTTP client: {e}"))?;
    let body = RequestBody {
        model: model.to_string(),
        // Large tasks (thinking + a whole-file edit in one tool call) easily
        // blow past 8k output tokens; a truncated stream ends the turn with
        // the work half-done. 32k fits every current Claude model's cap.
        max_tokens: 32_000,
        stream: true,
        messages: messages.to_vec(),
        tools: if tools.is_empty() { None } else { Some(tools.to_vec()) },
        system: system.map(|s| s.to_string()),
    };

    let url = format!("{}/v1/messages", config.base_url.trim_end_matches('/'));

    let response = client
        .post(&url)
        .header("Content-Type", "application/json")
        .header("x-api-key", &config.api_key)
        .header("anthropic-version", ANTHROPIC_VERSION)
        .json(&body)
        .send()
        .await
        .map_err(|e| format!("request failed: {e}"))?;

    let status = response.status();
    if !status.is_success() {
        let err_msg = if status.as_u16() == 401 {
            "Unauthorized — check your API key".into()
        } else {
            format!("API error: HTTP {status}")
        };
        let _ = event_tx.send(AgentEvent::Error(err_msg.clone()));
        return Err(err_msg);
    }

    let mut dump = if std::env::var("CLAUDINIO_DEBUG_DUMP").is_ok() {
        let dump_path = std::path::Path::new("/tmp").join(format!("claudinio_api_dump_{session_id}.txt"));
        std::fs::File::create(&dump_path)
            .map(|f| std::io::BufWriter::new(f))
            .ok()
    } else {
        None
    };
    if let Some(ref mut f) = dump {
        use std::io::Write;
        let _ = writeln!(f, "--- API RAW DUMP session={session_id} ---");
    }

    let mut stream = response.bytes_stream();
    let mut buf = String::new();

    let mut current_event = String::new();
    let mut current_data = String::new();

    let mut text_deltas: Vec<String> = Vec::new();
    let mut thinking_text: String = String::new();
    let mut tool_uses: Vec<Value> = Vec::new();
    let mut tool_inputs: std::collections::HashMap<usize, String> = std::collections::HashMap::new();
    let mut stop_reason: Option<String> = None;
    let mut usage: Option<Usage> = None;

    loop {
        // Check interrupt before processing each chunk
        if interrupt.load(Ordering::SeqCst) {
            // Drop the stream to cancel the HTTP request eagerly
            drop(stream);
            return Ok(StreamOutput {
                text_deltas,
                tool_uses,
                stop_reason: Some("interrupted".into()),
                usage: None,
                interrupted: true,
            });
        }

        let chunk_result = match tokio::time::timeout(STREAM_IDLE_TIMEOUT, stream.next()).await {
            Ok(Some(r)) => r,
            Ok(None) => break,
            Err(_) => return Err("stream error: no data received for 90s, connection stalled".into()),
        };

        let chunk = chunk_result.map_err(|e| format!("stream error: {e}"))?;
        let chunk_str = String::from_utf8_lossy(&chunk);
        if let Some(ref mut f) = dump {
            use std::io::Write;
            let _ = writeln!(f, "[CHUNK {} bytes]", chunk.len());
            let _ = writeln!(f, "{}", chunk_str);
        }
        buf.push_str(&chunk_str);

        loop {
            let newline = match buf.find('\n') {
                Some(pos) => pos,
                None => break,
            };
            let line = buf[..newline].trim_end_matches('\r').to_string();
            buf = buf[newline + 1..].to_string();

            if line.is_empty() {
                if !current_event.is_empty() {
                    if let Some(ref mut f) = dump {
                        use std::io::Write;
                        let _ = writeln!(f, "[EVENT] {} | DATA: {}", current_event, current_data);
                    }
                    process_line(
                        &current_event,
                        &current_data,
                        event_tx,
                        session_id,
                        assistant_text,
                        &mut thinking_text,
                        &mut text_deltas,
                        &mut tool_uses,
                        &mut tool_inputs,
                        &mut stop_reason,
                        &mut usage,
                    )?;
                    current_event.clear();
                    current_data.clear();
                }
                continue;
            }

            if let Some(data) = line.strip_prefix("event: ") {
                current_event = data.to_string();
            } else if let Some(data) = line.strip_prefix("data: ") {
                current_data = data.to_string();
            } else if let Some(ref mut f) = dump {
                use std::io::Write;
                let _ = writeln!(f, "[RAW] {}", line);
            }
        }
    }

    if !current_event.is_empty() {
        process_line(
            &current_event,
            &current_data,
            event_tx,
            session_id,
            assistant_text,
            &mut thinking_text,
            &mut text_deltas,
            &mut tool_uses,
            &mut tool_inputs,
            &mut stop_reason,
            &mut usage,
        )?;
    }

    if !buf.is_empty() {
        if let Ok(full) = serde_json::from_str::<Value>(&buf) {
            if let Some(blocks) = full.get("content").and_then(|c| c.as_array()) {
                for block in blocks {
                    if block.get("type").and_then(|t| t.as_str()) == Some("tool_use") {
                        if let Some(input) = block.get("input") {
                            if !input.is_null() {
                                let id = block.get("id").and_then(|i| i.as_str()).unwrap_or("");
                                let _name = block.get("name").and_then(|n| n.as_str()).unwrap_or("");
                                if let Some(existing) = tool_uses.iter_mut().find(|t| {
                                    t.get("id").and_then(|i| i.as_str()) == Some(id)
                                }) {
                                    if let Some(obj) = existing.as_object_mut() {
                                        obj.insert("input".into(), input.clone());
                                    }
                                } else {
                                    tool_uses.push(block.clone());
                                }
                            }
                        }
                    }
                }
            }
            if let Some(reason) = full.get("stop_reason").and_then(|r| r.as_str()) {
                stop_reason = Some(reason.to_string());
            }
            if let Some(u) = full.get("usage") {
                merge_usage(&mut usage, u);
            }
        }
    }

    if !buf.is_empty() {
        if let Some(ref mut f) = dump {
            use std::io::Write;
            let _ = writeln!(f, "--- REMAINING BUF ---\n{}", buf);
        }
    }

    if let Some(ref mut f) = dump {
        use std::io::Write;
        let _ = writeln!(f, "--- END --- tool_uses={} text_deltas={}", tool_uses.len(), text_deltas.len());
    }

    // Blocks still in tool_inputs never got their content_block_stop — the
    // stream was cut mid-input (e.g. max_tokens). Salvage what parses, drop
    // the rest so a half-written tool call never executes.
    for (idx, accumulated) in tool_inputs.drain() {
        match serde_json::from_str::<Value>(&accumulated) {
            Ok(parsed) => {
                if let Some(tool) = tool_uses.iter_mut().find(|t| {
                    t.get("_index").and_then(|i| i.as_u64()).map(|i| i as usize) == Some(idx)
                }) {
                    if let Some(obj) = tool.as_object_mut() {
                        obj.insert("input".into(), parsed);
                    }
                }
            }
            Err(_) if !accumulated.is_empty() => {
                tool_uses.retain(|t| {
                    t.get("_index").and_then(|i| i.as_u64()).map(|i| i as usize) != Some(idx)
                });
            }
            Err(_) => {}
        }
    }

    Ok(StreamOutput {
        text_deltas,
        tool_uses,
        stop_reason,
        usage,
        interrupted: false,
    })
}

#[allow(clippy::too_many_arguments)]
fn process_line(
    event_type: &str,
    data: &str,
    event_tx: &Channel<AgentEvent>,
    _session_id: &str,
    assistant_text: &mut String,
    thinking_text: &mut String,
    text_deltas: &mut Vec<String>,
    tool_uses: &mut Vec<Value>,
    tool_inputs: &mut std::collections::HashMap<usize, String>,
    stop_reason: &mut Option<String>,
    usage: &mut Option<Usage>,
) -> Result<(), String> {
    let value: Value = serde_json::from_str(data)
        .map_err(|e| format!("json parse: {e} data: {data:.100}"))?;

    let index = value.get("index").and_then(|v| v.as_u64()).map(|i| i as usize);

    match event_type {
        "content_block_start" => {
            if let Some(block) = value.get("content_block") {
                if block.get("type").and_then(|t| t.as_str()) == Some("tool_use") {
                    let mut tool = block.clone();
                    if let Some(idx) = index {
                        if let Some(obj) = tool.as_object_mut() {
                            obj.insert("_index".into(), serde_json::json!(idx));
                        }
                        tool_inputs.insert(idx, String::new());
                    }
                    tool_uses.push(tool);
                }
            }
        }
        "content_block_delta" => {
            if let Some(delta) = value.get("delta") {
                match delta.get("type").and_then(|t| t.as_str()) {
                    Some("text_delta") => {
                        if let Some(text) = delta.get("text").and_then(|t| t.as_str()) {
                            if !text.is_empty() {
                                text_deltas.push(text.to_string());
                                assistant_text.push_str(text);
                            }
                        }
                    }
                    Some("thinking_delta") => {
                        if let Some(thinking) = delta.get("thinking").and_then(|t| t.as_str()) {
                            if !thinking.is_empty() {
                                thinking_text.push_str(thinking);
                                let _ = event_tx.send(AgentEvent::Thinking(thinking_text.clone()));
                            }
                        }
                    }
                    Some("input_json_delta") => {
                        if let Some(idx) = index {
                            if let Some(partial) = delta.get("partial_json") {
                                let fragment = match partial {
                                    Value::String(s) => s.clone(),
                                    other => serde_json::to_string(other).unwrap_or_default(),
                                };
                                tool_inputs.entry(idx).or_default().push_str(&fragment);
                            }
                        }
                    }
                    _ => {}
                }
            }
        }
        "content_block_stop" => {
            if let Some(idx) = index {
                if let Some(accumulated) = tool_inputs.remove(&idx) {
                    match serde_json::from_str::<Value>(&accumulated) {
                        Ok(parsed) => {
                            if let Some(tool) = tool_uses.iter_mut().find(|t| {
                                t.get("_index").and_then(|i| i.as_u64()).map(|i| i as usize) == Some(idx)
                            }) {
                                if let Some(obj) = tool.as_object_mut() {
                                    obj.insert("input".into(), parsed);
                                }
                            }
                        }
                        // Empty accumulation is a no-arg tool: keep the {}
                        // input from content_block_start. Non-empty JSON that
                        // fails to parse means the stream was cut mid-input
                        // (max_tokens) — drop the block instead of running the
                        // tool with a bogus empty input.
                        Err(_) if !accumulated.is_empty() => {
                            tool_uses.retain(|t| {
                                t.get("_index").and_then(|i| i.as_u64()).map(|i| i as usize)
                                    != Some(idx)
                            });
                        }
                        Err(_) => {}
                    }
                }
            }
        }
        "message_delta" => {
            if let Some(delta) = value.get("delta") {
                if let Some(reason) = delta.get("stop_reason").and_then(|r| r.as_str()) {
                    *stop_reason = Some(reason.to_string());
                }
            }
            if let Some(u) = value.get("usage") {
                merge_usage(usage, u);
            }
        }
        "message_start" => {
            if let Some(u) = value.pointer("/message/usage") {
                merge_usage(usage, u);
            }
        }
        "ping" => {}
        _ => {}
    }

    Ok(())
}

impl fmt::Display for StreamOutput {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        let text: String = self.text_deltas.join("");
        write!(f, "{text}")
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use serde_json::json;
    use tauri::ipc::InvokeResponseBody;

    #[test]
    fn test_usage_merged_across_message_start_and_delta_anthropic_style() {
        // Anthropic: input_tokens in message_start, output_tokens in message_delta
        let mut usage: Option<Usage> = None;
        merge_usage(
            &mut usage,
            &json!({"input_tokens": 1200, "output_tokens": 0, "cache_read_input_tokens": 300}),
        );
        merge_usage(&mut usage, &json!({"output_tokens": 64}));
        let u = usage.unwrap();
        assert_eq!(u.input_tokens, 1200);
        assert_eq!(u.output_tokens, 64);
        assert_eq!(u.cache_read_input_tokens, 300);
        assert_eq!(u.cost, None);
    }

    #[test]
    fn test_usage_merged_claudinio_style_delta_only() {
        // claudin.io: zeros in message_start, full usage in message_delta
        let mut usage: Option<Usage> = None;
        merge_usage(
            &mut usage,
            &json!({"input_tokens": 0, "output_tokens": 0, "cache_read_input_tokens": 0}),
        );
        merge_usage(&mut usage, &json!({"input_tokens": 15, "output_tokens": 64}));
        let u = usage.unwrap();
        assert_eq!(u.input_tokens, 15);
        assert_eq!(u.output_tokens, 64);
        assert_eq!(u.cache_read_input_tokens, 0);
    }

    #[test]
    fn test_process_line_populates_usage_from_message_start() {
        let chan = Channel::new(|_: InvokeResponseBody| Ok(()));
        let mut text_deltas = Vec::new();
        let mut tool_uses = Vec::new();
        let mut tool_inputs = std::collections::HashMap::new();
        let mut stop_reason: Option<String> = None;
        let mut usage: Option<Usage> = None;
        let mut assistant_text = String::new();
        let mut thinking_text = String::new();

        process_line(
            "message_start",
            r#"{"type":"message_start","message":{"role":"assistant","usage":{"input_tokens":500,"output_tokens":0}}}"#,
            &chan, "s1", &mut assistant_text, &mut thinking_text,
            &mut text_deltas, &mut tool_uses, &mut tool_inputs,
            &mut stop_reason, &mut usage,
        ).unwrap();
        process_line(
            "message_delta",
            r#"{"type":"message_delta","delta":{"stop_reason":"end_turn"},"usage":{"output_tokens":42}}"#,
            &chan, "s1", &mut assistant_text, &mut thinking_text,
            &mut text_deltas, &mut tool_uses, &mut tool_inputs,
            &mut stop_reason, &mut usage,
        ).unwrap();

        let u = usage.unwrap();
        assert_eq!(u.input_tokens, 500);
        assert_eq!(u.output_tokens, 42);
        assert_eq!(stop_reason.as_deref(), Some("end_turn"));
    }

    #[test]
    fn test_input_json_delta_accumulates_tool_args() {
        let chan = Channel::new(|_: InvokeResponseBody| Ok(()));

        let mut text_deltas = Vec::new();
        let mut tool_uses = Vec::new();
        let mut tool_inputs = std::collections::HashMap::new();
        let mut stop_reason: Option<String> = None;
        let mut usage: Option<Usage> = None;
        let mut assistant_text = String::new();
        let mut thinking_text = String::new();
        // Step 1: content_block_start for tool_use with empty input placeholder
        process_line(
            "content_block_start",
            r#"{"type":"content_block_start","index":0,"content_block":{"type":"tool_use","id":"toolu_abc","name":"list_dir","input":{}}}"#,
    &chan, "s1", &mut assistant_text, &mut thinking_text,
        &mut text_deltas, &mut tool_uses, &mut tool_inputs,
            &mut stop_reason, &mut usage,
        ).unwrap();

        assert_eq!(tool_uses.len(), 1);
        assert_eq!(tool_uses[0]["name"], "list_dir");
        assert_eq!(tool_uses[0]["input"], json!({}));

        // Step 2: first input_json_delta fragment
        process_line(
            "content_block_delta",
            r#"{"type":"content_block_delta","index":0,"delta":{"type":"input_json_delta","partial_json":"{\"path\":"}}"#,
    &chan, "s1", &mut assistant_text, &mut thinking_text,
        &mut text_deltas, &mut tool_uses, &mut tool_inputs,
            &mut stop_reason, &mut usage,
        ).unwrap();

        // Step 3: second fragment
        process_line(
            "content_block_delta",
            r#"{"type":"content_block_delta","index":0,"delta":{"type":"input_json_delta","partial_json":" \"/home/user/project\""}}"#,
    &chan, "s1", &mut assistant_text, &mut thinking_text,
        &mut text_deltas, &mut tool_uses, &mut tool_inputs,
            &mut stop_reason, &mut usage,
        ).unwrap();

        // Step 4: third fragment (closing brace)
        process_line(
            "content_block_delta",
            r#"{"type":"content_block_delta","index":0,"delta":{"type":"input_json_delta","partial_json":"}"}}"#,
    &chan, "s1", &mut assistant_text, &mut thinking_text,
        &mut text_deltas, &mut tool_uses, &mut tool_inputs,
            &mut stop_reason, &mut usage,
        ).unwrap();

        // Step 5: content_block_stop — should finalize the input
        process_line(
            "content_block_stop",
            r#"{"type":"content_block_stop","index":0}"#,
    &chan, "s1", &mut assistant_text, &mut thinking_text,
        &mut text_deltas, &mut tool_uses, &mut tool_inputs,
            &mut stop_reason, &mut usage,
        ).unwrap();

        // The tool_use must now have the complete parsed input
        assert_eq!(
            tool_uses[0]["input"],
            json!({"path": "/home/user/project"})
        );
    }

    #[test]
    fn test_truncated_tool_input_drops_block() {
        let chan = Channel::new(|_: InvokeResponseBody| Ok(()));

        let mut text_deltas = Vec::new();
        let mut tool_uses = Vec::new();
        let mut tool_inputs = std::collections::HashMap::new();
        let mut stop_reason: Option<String> = None;
        let mut usage: Option<Usage> = None;
        let mut assistant_text = String::new();
        let mut thinking_text = String::new();

        process_line(
            "content_block_start",
            r#"{"type":"content_block_start","index":0,"content_block":{"type":"tool_use","id":"toolu_cut","name":"edit_file","input":{}}}"#,
            &chan, "s1", &mut assistant_text, &mut thinking_text,
            &mut text_deltas, &mut tool_uses, &mut tool_inputs,
            &mut stop_reason, &mut usage,
        ).unwrap();

        // Stream hits max_tokens mid-input: the JSON never closes.
        process_line(
            "content_block_delta",
            r#"{"type":"content_block_delta","index":0,"delta":{"type":"input_json_delta","partial_json":"{\"path\": \"/workspace/Icon.tsx\", \"content\": \"const icons = "}}"#,
            &chan, "s1", &mut assistant_text, &mut thinking_text,
            &mut text_deltas, &mut tool_uses, &mut tool_inputs,
            &mut stop_reason, &mut usage,
        ).unwrap();

        process_line(
            "content_block_stop",
            r#"{"type":"content_block_stop","index":0}"#,
            &chan, "s1", &mut assistant_text, &mut thinking_text,
            &mut text_deltas, &mut tool_uses, &mut tool_inputs,
            &mut stop_reason, &mut usage,
        ).unwrap();

        // The half-written tool call must not survive to execution.
        assert!(tool_uses.is_empty());
    }

    #[test]
    fn test_tool_use_with_complete_input_in_start_keeps_it() {
        let chan = Channel::new(|_: InvokeResponseBody| Ok(()));

        let mut text_deltas = Vec::new();
        let mut tool_uses = Vec::new();
        let mut tool_inputs = std::collections::HashMap::new();
        let mut stop_reason: Option<String> = None;
        let mut usage: Option<Usage> = None;
        let mut assistant_text = String::new();
        let mut thinking_text = String::new();

        // Some APIs send the complete input directly in content_block_start
        process_line(
            "content_block_start",
            r#"{"type":"content_block_start","index":0,"content_block":{"type":"tool_use","id":"toolu_def","name":"read_file","input":{"path":"/workspace/main.rs"}}}"#,
    &chan, "s1", &mut assistant_text, &mut thinking_text,
        &mut text_deltas, &mut tool_uses, &mut tool_inputs,
            &mut stop_reason, &mut usage,
        ).unwrap();

        // content_block_stop with no input_json_delta in between
        process_line(
            "content_block_stop",
            r#"{"type":"content_block_stop","index":0}"#,
    &chan, "s1", &mut assistant_text, &mut thinking_text,
        &mut text_deltas, &mut tool_uses, &mut tool_inputs,
            &mut stop_reason, &mut usage,
        ).unwrap();

        // Input should still be the original complete value
        assert_eq!(
            tool_uses[0]["input"],
            json!({"path": "/workspace/main.rs"})
        );
    }

    #[test]
    fn test_tool_use_args_deserialize_successfully() {
        let chan = Channel::new(|_: InvokeResponseBody| Ok(()));

        let mut text_deltas = Vec::new();
        let mut tool_uses = Vec::new();
        let mut tool_inputs = std::collections::HashMap::new();
        let mut stop_reason: Option<String> = None;
        let mut usage: Option<Usage> = None;
        let mut assistant_text = String::new();
        let mut thinking_text = String::new();

        // Full streaming sequence
        process_line(
            "content_block_start",
            r#"{"type":"content_block_start","index":0,"content_block":{"type":"tool_use","id":"toolu_ghi","name":"read_file","input":{}}}"#,
    &chan, "s1", &mut assistant_text, &mut thinking_text,
        &mut text_deltas, &mut tool_uses, &mut tool_inputs,
            &mut stop_reason, &mut usage,
        ).unwrap();

        process_line(
            "content_block_delta",
            r#"{"type":"content_block_delta","index":0,"delta":{"type":"input_json_delta","partial_json":"{\"path\":"}}"#,
    &chan, "s1", &mut assistant_text, &mut thinking_text,
        &mut text_deltas, &mut tool_uses, &mut tool_inputs,
            &mut stop_reason, &mut usage,
        ).unwrap();

        process_line(
            "content_block_delta",
            r#"{"type":"content_block_delta","index":0,"delta":{"type":"input_json_delta","partial_json":"\"src/main.ts\""}}"#,
    &chan, "s1", &mut assistant_text, &mut thinking_text,
        &mut text_deltas, &mut tool_uses, &mut tool_inputs,
            &mut stop_reason, &mut usage,
        ).unwrap();

        process_line(
            "content_block_delta",
            r#"{"type":"content_block_delta","index":0,"delta":{"type":"input_json_delta","partial_json":"}"}}"#,
    &chan, "s1", &mut assistant_text, &mut thinking_text,
        &mut text_deltas, &mut tool_uses, &mut tool_inputs,
            &mut stop_reason, &mut usage,
        ).unwrap();

        process_line(
            "content_block_stop",
            r#"{"type":"content_block_stop","index":0}"#,
    &chan, "s1", &mut assistant_text, &mut thinking_text,
        &mut text_deltas, &mut tool_uses, &mut tool_inputs,
            &mut stop_reason, &mut usage,
        ).unwrap();

        // Now simulate what session.rs does: deserialize the input
        let input = tool_uses[0].get("input").unwrap();
        let path = input.get("path").and_then(|v| v.as_str()).unwrap();
        assert_eq!(path, "src/main.ts");
    }

    #[test]
    fn test_mixed_text_and_tool_stream() {
        let chan = Channel::new(|_: InvokeResponseBody| Ok(()));

        let mut text_deltas = Vec::new();
        let mut tool_uses = Vec::new();
        let mut tool_inputs = std::collections::HashMap::new();
        let mut stop_reason: Option<String> = None;
        let mut usage: Option<Usage> = None;
        let mut assistant_text = String::new();
        let mut thinking_text = String::new();

        // Text block at index 0
        process_line(
            "content_block_start",
            r#"{"type":"content_block_start","index":0,"content_block":{"type":"text","text":"Let me look at "}}"#,
    &chan, "s1", &mut assistant_text, &mut thinking_text,
        &mut text_deltas, &mut tool_uses, &mut tool_inputs,
            &mut stop_reason, &mut usage,
        ).unwrap();

        // Tool at index 1
        process_line(
            "content_block_start",
            r#"{"type":"content_block_start","index":1,"content_block":{"type":"tool_use","id":"toolu_xyz","name":"list_dir","input":{}}}"#,
    &chan, "s1", &mut assistant_text, &mut thinking_text,
        &mut text_deltas, &mut tool_uses, &mut tool_inputs,
            &mut stop_reason, &mut usage,
        ).unwrap();

        // Text delta
        process_line(
            "content_block_delta",
            r#"{"type":"content_block_delta","index":0,"delta":{"type":"text_delta","text":"the source"}}"#,
    &chan, "s1", &mut assistant_text, &mut thinking_text,
        &mut text_deltas, &mut tool_uses, &mut tool_inputs,
            &mut stop_reason, &mut usage,
        ).unwrap();

        // Tool input delta
        process_line(
            "content_block_delta",
            r#"{"type":"content_block_delta","index":1,"delta":{"type":"input_json_delta","partial_json":"{\"path\":"}}"#,
    &chan, "s1", &mut assistant_text, &mut thinking_text,
        &mut text_deltas, &mut tool_uses, &mut tool_inputs,
            &mut stop_reason, &mut usage,
        ).unwrap();

        // Text block stops (index 0) — should NOT interfere with tool at index 1
        process_line(
            "content_block_stop",
            r#"{"type":"content_block_stop","index":0}"#,
    &chan, "s1", &mut assistant_text, &mut thinking_text,
        &mut text_deltas, &mut tool_uses, &mut tool_inputs,
            &mut stop_reason, &mut usage,
        ).unwrap();

        // More tool input
        process_line(
            "content_block_delta",
            r#"{"type":"content_block_delta","index":1,"delta":{"type":"input_json_delta","partial_json":"\"/src\""}}"#,
    &chan, "s1", &mut assistant_text, &mut thinking_text,
        &mut text_deltas, &mut tool_uses, &mut tool_inputs,
            &mut stop_reason, &mut usage,
        ).unwrap();

        // More tool input
        process_line(
            "content_block_delta",
            r#"{"type":"content_block_delta","index":1,"delta":{"type":"input_json_delta","partial_json":"}"}}"#,
    &chan, "s1", &mut assistant_text, &mut thinking_text,
        &mut text_deltas, &mut tool_uses, &mut tool_inputs,
            &mut stop_reason, &mut usage,
        ).unwrap();

        // Tool stops
        process_line(
            "content_block_stop",
            r#"{"type":"content_block_stop","index":1}"#,
    &chan, "s1", &mut assistant_text, &mut thinking_text,
        &mut text_deltas, &mut tool_uses, &mut tool_inputs,
            &mut stop_reason, &mut usage,
        ).unwrap();

        // Text: content_block_start for text isn't accumulated, only deltas are
        assert_eq!(assistant_text, "the source");

        // Tool should have complete input
        assert_eq!(tool_uses.len(), 1);
        assert_eq!(
            tool_uses[0]["input"],
            json!({"path": "/src"})
        );

        // Text block stop at index 0 must NOT clear tool input at index 1
        assert!(tool_inputs.get(&1).is_none(), "tool_inputs should be empty after content_block_stop");
    }
}
