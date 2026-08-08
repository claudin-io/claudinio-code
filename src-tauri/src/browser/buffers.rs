//! What the page logged and what it fetched.
//!
//! Fed by a task draining the CDP event broadcast, independently of whether a
//! tool call is in flight — otherwise everything logged during a page load,
//! which is most of what matters, would be gone by the time the agent asks.
//!
//! `apply_event` is a plain method over `(method, params)` so the whole
//! translation layer is testable against captured CDP JSON, with no browser.

use serde_json::Value;
use std::collections::{HashMap, VecDeque};

const CONSOLE_CAP: usize = 500;
const NETWORK_CAP: usize = 300;
/// Per-entry text cap. A single `console.log` of a large object should not be
/// able to crowd out the other 499 entries.
const TEXT_CAP: usize = 2_000;
/// Query strings carry tokens; there is no reason to keep an unbounded URL.
const URL_CAP: usize = 512;

#[derive(Debug, Clone, PartialEq)]
pub struct ConsoleEntry {
    pub seq: u64,
    pub level: String,
    pub text: String,
    /// "console" | "exception" | "browser" | "navigation"
    pub source: String,
    pub url: Option<String>,
    pub line: Option<u32>,
}

#[derive(Debug, Clone, PartialEq)]
pub struct NetEntry {
    pub seq: u64,
    pub request_id: String,
    pub method: String,
    pub url: String,
    pub resource_type: String,
    pub status: Option<u64>,
    pub mime: Option<String>,
    pub size_bytes: Option<u64>,
    pub duration_ms: Option<u64>,
    pub error: Option<String>,
    pub from_cache: bool,
    /// CDP monotonic timestamp of the request, used to compute the duration.
    start_ts: f64,
}

#[derive(Default)]
pub struct Buffers {
    console: VecDeque<ConsoleEntry>,
    network: VecDeque<NetEntry>,
    /// Requests seen but not yet finished, keyed by CDP requestId.
    inflight: HashMap<String, NetEntry>,
    next_seq: u64,
    console_cursor: u64,
    network_cursor: u64,
}

impl Buffers {
    fn seq(&mut self) -> u64 {
        self.next_seq += 1;
        self.next_seq
    }

    fn push_console(&mut self, entry: ConsoleEntry) {
        if self.console.len() >= CONSOLE_CAP {
            self.console.pop_front();
        }
        self.console.push_back(entry);
    }

    fn finish_request(&mut self, request_id: &str, mutate: impl FnOnce(&mut NetEntry)) {
        let Some(mut entry) = self.inflight.remove(request_id) else {
            return;
        };
        mutate(&mut entry);
        if self.network.len() >= NETWORK_CAP {
            self.network.pop_front();
        }
        self.network.push_back(entry);
    }

    /// Translate one CDP event into buffer state. Unknown methods are ignored.
    pub fn apply_event(&mut self, method: &str, params: &Value) {
        match method {
            "Runtime.consoleAPICalled" => {
                let level = normalize_level(params.get("type").and_then(Value::as_str));
                let args = params
                    .get("args")
                    .and_then(Value::as_array)
                    .map(|a| {
                        a.iter()
                            .map(render_remote_object)
                            .collect::<Vec<_>>()
                            .join(" ")
                    })
                    .unwrap_or_default();
                let (url, line) = top_frame(params.pointer("/stackTrace/callFrames"));
                let seq = self.seq();
                self.push_console(ConsoleEntry {
                    seq,
                    level,
                    text: truncate(&args, TEXT_CAP),
                    source: "console".into(),
                    url,
                    line,
                });
            }
            // Uncaught errors never reach consoleAPICalled — this is where a
            // thrown exception shows up, and it is usually the thing the user
            // actually wants to see.
            "Runtime.exceptionThrown" => {
                let details = params.get("exceptionDetails");
                let text = details
                    .and_then(|d| d.pointer("/exception/description").and_then(Value::as_str))
                    .or_else(|| details.and_then(|d| d.get("text").and_then(Value::as_str)))
                    .unwrap_or("uncaught exception")
                    .to_string();
                let url = details
                    .and_then(|d| d.get("url").and_then(Value::as_str))
                    .map(|s| truncate(s, URL_CAP));
                let line = details
                    .and_then(|d| d.get("lineNumber").and_then(Value::as_u64))
                    .map(|n| n as u32 + 1);
                let seq = self.seq();
                self.push_console(ConsoleEntry {
                    seq,
                    level: "error".into(),
                    text: truncate(&text, TEXT_CAP),
                    source: "exception".into(),
                    url,
                    line,
                });
            }
            // Browser-level messages: failed subresources, CSP violations,
            // deprecations. Skip `console-api`, which duplicates the above.
            "Log.entryAdded" => {
                let entry = params.get("entry");
                let source = entry.and_then(|e| e.get("source").and_then(Value::as_str));
                if source == Some("console-api") {
                    return;
                }
                let text = entry
                    .and_then(|e| e.get("text").and_then(Value::as_str))
                    .unwrap_or_default()
                    .to_string();
                if text.is_empty() {
                    return;
                }
                let level =
                    normalize_level(entry.and_then(|e| e.get("level").and_then(Value::as_str)));
                let url = entry
                    .and_then(|e| e.get("url").and_then(Value::as_str))
                    .map(|s| truncate(s, URL_CAP));
                let line = entry
                    .and_then(|e| e.get("lineNumber").and_then(Value::as_u64))
                    .map(|n| n as u32);
                let seq = self.seq();
                self.push_console(ConsoleEntry {
                    seq,
                    level,
                    text: truncate(&text, TEXT_CAP),
                    source: source.unwrap_or("browser").to_string(),
                    url,
                    line,
                });
            }
            // A navigation marker rather than a buffer reset: the error from
            // just before a redirect is often the whole story.
            "Page.frameNavigated" => {
                if params.pointer("/frame/parentId").is_some() {
                    return; // subframe
                }
                let url = params
                    .pointer("/frame/url")
                    .and_then(Value::as_str)
                    .unwrap_or_default()
                    .to_string();
                let seq = self.seq();
                self.push_console(ConsoleEntry {
                    seq,
                    level: "info".into(),
                    text: format!("--- navigated to {} ---", truncate(&url, URL_CAP)),
                    source: "navigation".into(),
                    url: None,
                    line: None,
                });
            }
            "Network.requestWillBeSent" => {
                let Some(request_id) = params.get("requestId").and_then(Value::as_str) else {
                    return;
                };
                let seq = self.seq();
                let entry = NetEntry {
                    seq,
                    request_id: request_id.to_string(),
                    method: params
                        .pointer("/request/method")
                        .and_then(Value::as_str)
                        .unwrap_or("GET")
                        .to_string(),
                    url: truncate(
                        params
                            .pointer("/request/url")
                            .and_then(Value::as_str)
                            .unwrap_or_default(),
                        URL_CAP,
                    ),
                    resource_type: params
                        .get("type")
                        .and_then(Value::as_str)
                        .unwrap_or("Other")
                        .to_string(),
                    status: None,
                    mime: None,
                    size_bytes: None,
                    duration_ms: None,
                    error: None,
                    from_cache: false,
                    start_ts: params
                        .get("timestamp")
                        .and_then(Value::as_f64)
                        .unwrap_or(0.0),
                };
                self.inflight.insert(request_id.to_string(), entry);
            }
            "Network.responseReceived" => {
                let Some(request_id) = params.get("requestId").and_then(Value::as_str) else {
                    return;
                };
                if let Some(entry) = self.inflight.get_mut(request_id) {
                    entry.status = params.pointer("/response/status").and_then(Value::as_u64);
                    entry.mime = params
                        .pointer("/response/mimeType")
                        .and_then(Value::as_str)
                        .map(str::to_string);
                    entry.from_cache = params
                        .pointer("/response/fromDiskCache")
                        .and_then(Value::as_bool)
                        .unwrap_or(false);
                }
            }
            // The real transferred size arrives here, not on responseReceived.
            "Network.loadingFinished" => {
                let Some(request_id) = params.get("requestId").and_then(Value::as_str) else {
                    return;
                };
                let size = params.get("encodedDataLength").and_then(Value::as_f64);
                let end = params.get("timestamp").and_then(Value::as_f64);
                self.finish_request(request_id, |entry| {
                    entry.size_bytes = size.map(|s| s as u64);
                    entry.duration_ms = duration_ms(entry.start_ts, end);
                });
            }
            "Network.loadingFailed" => {
                let Some(request_id) = params.get("requestId").and_then(Value::as_str) else {
                    return;
                };
                let error = params
                    .get("errorText")
                    .and_then(Value::as_str)
                    .unwrap_or("request failed")
                    .to_string();
                let canceled = params
                    .get("canceled")
                    .and_then(Value::as_bool)
                    .unwrap_or(false);
                let end = params.get("timestamp").and_then(Value::as_f64);
                self.finish_request(request_id, |entry| {
                    entry.error = Some(if canceled {
                        format!("{error} (canceled)")
                    } else {
                        error
                    });
                    entry.duration_ms = duration_ms(entry.start_ts, end);
                });
            }
            _ => {}
        }
    }

    /// Console entries, newest last. `only_new` advances a cursor so a polling
    /// agent gets each line once — the difference between a cheap loop and
    /// re-reading 500 lines every turn.
    pub fn console(
        &mut self,
        only_new: bool,
        min_level: Option<&str>,
        limit: usize,
    ) -> Vec<ConsoleEntry> {
        let cursor = self.console_cursor;
        let mut out: Vec<ConsoleEntry> = self
            .console
            .iter()
            .filter(|e| !only_new || e.seq > cursor)
            .filter(|e| min_level.is_none_or(|m| level_rank(&e.level) >= level_rank(m)))
            .cloned()
            .collect();
        // Advance past everything buffered, not just what passed the filter:
        // a level filter must not make the next unfiltered read replay old
        // lines it already logically consumed.
        if let Some(last) = self.console.back() {
            self.console_cursor = last.seq;
        }
        if out.len() > limit {
            out.drain(..out.len() - limit);
        }
        out
    }

    pub fn network(&mut self, only_new: bool, filter: Option<&str>, limit: usize) -> Vec<NetEntry> {
        let cursor = self.network_cursor;
        let mut out: Vec<NetEntry> = self
            .network
            .iter()
            .filter(|e| !only_new || e.seq > cursor)
            .filter(|e| match filter {
                None => true,
                Some("failed") => e.error.is_some() || e.status.is_some_and(|s| s >= 400),
                Some(needle) => e.url.contains(needle),
            })
            .cloned()
            .collect();
        if let Some(last) = self.network.back() {
            self.network_cursor = last.seq;
        }
        if out.len() > limit {
            out.drain(..out.len() - limit);
        }
        out
    }

    pub fn is_empty(&self) -> bool {
        self.console.is_empty() && self.network.is_empty()
    }
}

fn duration_ms(start: f64, end: Option<f64>) -> Option<u64> {
    // CDP network timestamps are monotonic seconds from an arbitrary epoch.
    let end = end?;
    if start <= 0.0 || end < start {
        return None;
    }
    Some(((end - start) * 1000.0) as u64)
}

fn normalize_level(raw: Option<&str>) -> String {
    match raw.unwrap_or("log") {
        "error" | "assert" => "error",
        "warning" | "warn" => "warning",
        "info" => "info",
        "debug" | "verbose" | "trace" => "debug",
        _ => "log",
    }
    .to_string()
}

fn level_rank(level: &str) -> u8 {
    match level {
        "error" => 4,
        "warning" => 3,
        "info" => 2,
        "log" => 1,
        _ => 0,
    }
}

fn truncate(s: &str, n: usize) -> String {
    if s.chars().count() <= n {
        return s.to_string();
    }
    let cut: String = s.chars().take(n).collect();
    format!("{cut}…")
}

/// Render a CDP `RemoteObject` without evaluating anything in the page.
///
/// Chrome sends primitives inline, a structured `preview` for objects, and a
/// `description` for errors and functions — enough to reconstruct a readable
/// line from the event alone.
fn render_remote_object(obj: &Value) -> String {
    if let Some(v) = obj.get("value") {
        return match v {
            Value::String(s) => s.clone(),
            other => other.to_string(),
        };
    }
    if let Some(u) = obj.get("unserializableValue").and_then(Value::as_str) {
        return u.to_string(); // NaN, Infinity, -0, bigints
    }
    if let Some(props) = obj.pointer("/preview/properties").and_then(Value::as_array) {
        let subtype = obj.pointer("/preview/subtype").and_then(Value::as_str);
        let rendered: Vec<String> = props
            .iter()
            .map(|p| {
                let name = p.get("name").and_then(Value::as_str).unwrap_or("");
                let value = p.get("value").and_then(Value::as_str).unwrap_or("");
                if subtype == Some("array") {
                    value.to_string()
                } else {
                    format!("{name}: {value}")
                }
            })
            .collect();
        let overflow = obj
            .pointer("/preview/overflow")
            .and_then(Value::as_bool)
            .unwrap_or(false);
        let body = rendered.join(", ");
        let body = if overflow {
            format!("{body}, …")
        } else {
            body
        };
        return if subtype == Some("array") {
            format!("[{body}]")
        } else {
            format!("{{{body}}}")
        };
    }
    if let Some(d) = obj.get("description").and_then(Value::as_str) {
        return d.to_string();
    }
    format!(
        "[{}]",
        obj.get("type").and_then(Value::as_str).unwrap_or("unknown")
    )
}

/// The frame that produced a console call, if the stack was captured.
fn top_frame(frames: Option<&Value>) -> (Option<String>, Option<u32>) {
    let Some(frame) = frames.and_then(Value::as_array).and_then(|a| a.first()) else {
        return (None, None);
    };
    (
        frame
            .get("url")
            .and_then(Value::as_str)
            .filter(|s| !s.is_empty())
            .map(|s| truncate(s, URL_CAP)),
        // CDP line numbers are 0-based; every UI that shows them is 1-based.
        frame
            .get("lineNumber")
            .and_then(Value::as_u64)
            .map(|n| n as u32 + 1),
    )
}

#[cfg(test)]
mod tests {
    use super::*;
    use serde_json::json;

    fn buffers() -> Buffers {
        Buffers::default()
    }

    #[test]
    fn console_calls_render_their_arguments() {
        let mut b = buffers();
        b.apply_event(
            "Runtime.consoleAPICalled",
            &json!({
                "type": "error",
                "args": [{"type": "string", "value": "boom"}, {"type": "number", "value": 42}],
                "stackTrace": {"callFrames": [{"url": "http://x/app.js", "lineNumber": 9}]}
            }),
        );
        let out = b.console(false, None, 50);
        assert_eq!(out.len(), 1);
        assert_eq!(out[0].level, "error");
        assert_eq!(out[0].text, "boom 42");
        assert_eq!(out[0].url.as_deref(), Some("http://x/app.js"));
        assert_eq!(out[0].line, Some(10), "line numbers are reported 1-based");
    }

    #[test]
    fn uncaught_exceptions_become_error_entries() {
        let mut b = buffers();
        b.apply_event(
            "Runtime.exceptionThrown",
            &json!({"exceptionDetails": {
                "text": "Uncaught",
                "exception": {"description": "TypeError: x is not a function\n    at app.js:3"},
                "url": "http://x/app.js",
                "lineNumber": 2
            }}),
        );
        let out = b.console(false, None, 50);
        assert_eq!(out[0].level, "error");
        assert_eq!(out[0].source, "exception");
        assert!(out[0].text.starts_with("TypeError"));
    }

    /// Log.entryAdded also carries console.* messages; keeping them would show
    /// every console call twice.
    #[test]
    fn browser_log_skips_what_the_runtime_already_reported() {
        let mut b = buffers();
        b.apply_event(
            "Log.entryAdded",
            &json!({"entry": {"source": "console-api", "level": "error", "text": "dup"}}),
        );
        b.apply_event(
            "Log.entryAdded",
            &json!({"entry": {"source": "network", "level": "error",
                              "text": "Failed to load resource: 404", "url": "http://x/a.png"}}),
        );
        let out = b.console(false, None, 50);
        assert_eq!(out.len(), 1);
        assert_eq!(out[0].source, "network");
    }

    #[test]
    fn a_full_request_collapses_into_one_entry_with_status_and_size() {
        let mut b = buffers();
        b.apply_event(
            "Network.requestWillBeSent",
            &json!({"requestId": "1", "timestamp": 100.0, "type": "XHR",
                    "request": {"method": "POST", "url": "http://x/api/save"}}),
        );
        b.apply_event(
            "Network.responseReceived",
            &json!({"requestId": "1", "response": {"status": 500, "mimeType": "application/json"}}),
        );
        b.apply_event(
            "Network.loadingFinished",
            &json!({"requestId": "1", "timestamp": 100.25, "encodedDataLength": 1234.0}),
        );

        let out = b.network(false, None, 50);
        assert_eq!(out.len(), 1, "the three events must collapse into one row");
        assert_eq!(out[0].method, "POST");
        assert_eq!(out[0].status, Some(500));
        assert_eq!(out[0].size_bytes, Some(1234));
        assert_eq!(out[0].duration_ms, Some(250));
        assert_eq!(out[0].resource_type, "XHR");
    }

    #[test]
    fn a_failed_request_records_why() {
        let mut b = buffers();
        b.apply_event(
            "Network.requestWillBeSent",
            &json!({"requestId": "2", "timestamp": 1.0,
                    "request": {"method": "GET", "url": "http://x/missing"}}),
        );
        b.apply_event(
            "Network.loadingFailed",
            &json!({"requestId": "2", "timestamp": 1.5,
                    "errorText": "net::ERR_CONNECTION_REFUSED"}),
        );
        let out = b.network(false, None, 50);
        assert_eq!(out[0].error.as_deref(), Some("net::ERR_CONNECTION_REFUSED"));
    }

    #[test]
    fn an_unfinished_request_is_not_reported_yet() {
        let mut b = buffers();
        b.apply_event(
            "Network.requestWillBeSent",
            &json!({"requestId": "3", "timestamp": 1.0, "request": {"method": "GET", "url": "http://x/slow"}}),
        );
        assert!(b.network(false, None, 50).is_empty());
    }

    #[test]
    fn the_failed_filter_catches_both_error_status_and_transport_failure() {
        let mut b = buffers();
        for (id, status) in [("a", 200u64), ("b", 404)] {
            b.apply_event(
                "Network.requestWillBeSent",
                &json!({"requestId": id, "timestamp": 1.0, "request": {"method": "GET", "url": format!("http://x/{id}")}}),
            );
            b.apply_event(
                "Network.responseReceived",
                &json!({"requestId": id, "response": {"status": status}}),
            );
            b.apply_event(
                "Network.loadingFinished",
                &json!({"requestId": id, "timestamp": 1.1, "encodedDataLength": 1.0}),
            );
        }
        let out = b.network(false, Some("failed"), 50);
        assert_eq!(out.len(), 1);
        assert_eq!(out[0].status, Some(404));
    }

    /// The property that makes a polling agent cheap: read twice, see it once.
    #[test]
    fn only_new_advances_a_cursor() {
        let mut b = buffers();
        b.apply_event(
            "Runtime.consoleAPICalled",
            &json!({"type": "log", "args": [{"type": "string", "value": "first"}]}),
        );
        assert_eq!(b.console(true, None, 50).len(), 1);
        assert!(b.console(true, None, 50).is_empty());

        b.apply_event(
            "Runtime.consoleAPICalled",
            &json!({"type": "log", "args": [{"type": "string", "value": "second"}]}),
        );
        let out = b.console(true, None, 50);
        assert_eq!(out.len(), 1);
        assert_eq!(out[0].text, "second");
        // only_new = false still replays everything buffered.
        assert_eq!(b.console(false, None, 50).len(), 2);
    }

    /// A level filter must not leave older low-level lines to reappear on the
    /// next unfiltered read.
    #[test]
    fn a_level_filter_still_consumes_the_lines_it_skipped() {
        let mut b = buffers();
        b.apply_event(
            "Runtime.consoleAPICalled",
            &json!({"type": "log", "args": [{"type": "string", "value": "chatter"}]}),
        );
        b.apply_event(
            "Runtime.consoleAPICalled",
            &json!({"type": "error", "args": [{"type": "string", "value": "boom"}]}),
        );
        let errors = b.console(true, Some("error"), 50);
        assert_eq!(errors.len(), 1);
        assert!(b.console(true, None, 50).is_empty());
    }

    #[test]
    fn navigation_marks_the_log_instead_of_clearing_it() {
        let mut b = buffers();
        b.apply_event(
            "Runtime.consoleAPICalled",
            &json!({"type": "error", "args": [{"type": "string", "value": "before"}]}),
        );
        b.apply_event(
            "Page.frameNavigated",
            &json!({"frame": {"url": "http://x/next"}}),
        );
        let out = b.console(false, None, 50);
        assert_eq!(out.len(), 2, "the pre-navigation error must survive");
        assert!(out[1].text.contains("navigated to http://x/next"));
    }

    #[test]
    fn subframe_navigation_is_ignored() {
        let mut b = buffers();
        b.apply_event(
            "Page.frameNavigated",
            &json!({"frame": {"url": "http://x/ad", "parentId": "root"}}),
        );
        assert!(b.console(false, None, 50).is_empty());
    }

    #[test]
    fn the_limit_keeps_the_newest_entries() {
        let mut b = buffers();
        for i in 0..10 {
            b.apply_event(
                "Runtime.consoleAPICalled",
                &json!({"type": "log", "args": [{"type": "number", "value": i}]}),
            );
        }
        let out = b.console(false, None, 3);
        assert_eq!(out.len(), 3);
        assert_eq!(out[2].text, "9", "newest entry must be last");
        assert_eq!(out[0].text, "7");
    }

    #[test]
    fn the_console_ring_drops_the_oldest_past_capacity() {
        let mut b = buffers();
        for i in 0..(CONSOLE_CAP + 10) {
            b.apply_event(
                "Runtime.consoleAPICalled",
                &json!({"type": "log", "args": [{"type": "number", "value": i}]}),
            );
        }
        let out = b.console(false, None, usize::MAX);
        assert_eq!(out.len(), CONSOLE_CAP);
        assert_eq!(out[0].text, "10");
    }

    #[test]
    fn remote_objects_render_across_their_shapes() {
        let cases = [
            (json!({"type": "string", "value": "hi"}), "hi"),
            (json!({"type": "number", "value": 3.5}), "3.5"),
            (json!({"type": "boolean", "value": true}), "true"),
            (
                json!({"type": "number", "unserializableValue": "NaN"}),
                "NaN",
            ),
            (
                json!({"type": "object", "description": "Error: boom\n  at x"}),
                "Error: boom\n  at x",
            ),
            (json!({"type": "function"}), "[function]"),
            (
                json!({"type": "object", "preview": {"properties": [
                    {"name": "a", "value": "1"}, {"name": "b", "value": "x"}]}}),
                "{a: 1, b: x}",
            ),
            (
                json!({"type": "object", "preview": {"subtype": "array", "overflow": true,
                       "properties": [{"name": "0", "value": "1"}]}}),
                "[1, …]",
            ),
        ];
        for (input, expected) in cases {
            assert_eq!(render_remote_object(&input), expected, "for {input}");
        }
    }

    #[test]
    fn long_text_is_truncated_with_an_ellipsis() {
        let mut b = buffers();
        let huge = "x".repeat(TEXT_CAP * 2);
        b.apply_event(
            "Runtime.consoleAPICalled",
            &json!({"type": "log", "args": [{"type": "string", "value": huge}]}),
        );
        let out = b.console(false, None, 50);
        assert!(out[0].text.chars().count() <= TEXT_CAP + 1);
        assert!(out[0].text.ends_with('…'));
    }

    /// Truncation is by characters, not bytes: slicing a multi-byte string on
    /// a byte boundary panics.
    #[test]
    fn truncation_does_not_split_multibyte_characters() {
        let s = "ação".repeat(1000);
        let out = truncate(&s, 10);
        assert_eq!(out.chars().count(), 11);
    }

    #[test]
    fn unknown_events_are_ignored() {
        let mut b = buffers();
        b.apply_event("Some.futureEvent", &json!({"whatever": true}));
        assert!(b.is_empty());
    }
}
