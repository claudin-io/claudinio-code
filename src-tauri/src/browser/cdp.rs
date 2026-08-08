//! A minimal Chrome DevTools Protocol client.
//!
//! CDP is JSON-RPC over one WebSocket: requests carry an `id` and come back
//! with the same `id`; anything without an `id` is an event. A reader task
//! demultiplexes the two — replies resolve a pending `oneshot`, events go out on
//! a broadcast channel that the console and network buffers subscribe to.
//!
//! Only the ~15 methods the browser tools need are used, which is why this is
//! hand-rolled rather than pulling in a full CDP binding crate.

use futures::{SinkExt, StreamExt};
use serde_json::{Value, json};
use std::collections::HashMap;
use std::sync::Arc;
use std::sync::atomic::{AtomicBool, AtomicI64, Ordering};
use std::time::Duration;
use tokio::sync::{Mutex, broadcast, mpsc, oneshot};
use tokio_tungstenite::tungstenite::Message;

pub const DEFAULT_CALL_TIMEOUT: Duration = Duration::from_secs(30);
/// Screenshots are the one call that is genuinely slow, especially full-page.
pub const SCREENSHOT_TIMEOUT: Duration = Duration::from_secs(60);

/// Frame and message caps.
///
/// tungstenite defaults to 16 MiB per frame, and a full-page screenshot comes
/// back as one base64 string that exceeds that — the connection would be torn
/// down mid-capture with an opaque protocol error. This is the single most
/// common way a hand-written CDP client breaks.
const MAX_MESSAGE_BYTES: usize = 512 << 20;

const EVENT_CHANNEL_CAPACITY: usize = 1024;

/// In-flight calls, keyed by CDP request id.
type PendingCalls = Arc<Mutex<HashMap<i64, oneshot::Sender<Result<Value, String>>>>>;

#[derive(Clone, Debug)]
pub struct CdpEvent {
    pub session_id: Option<String>,
    pub method: String,
    pub params: Value,
}

pub struct CdpConnection {
    next_id: AtomicI64,
    outgoing: mpsc::UnboundedSender<Message>,
    pending: PendingCalls,
    events: broadcast::Sender<CdpEvent>,
    closed: Arc<AtomicBool>,
}

impl CdpConnection {
    /// Connect to a `ws://127.0.0.1:PORT/devtools/...` endpoint.
    pub async fn connect(ws_url: &str) -> Result<Arc<Self>, String> {
        let config = tokio_tungstenite::tungstenite::protocol::WebSocketConfig::default()
            .max_message_size(Some(MAX_MESSAGE_BYTES))
            .max_frame_size(Some(MAX_MESSAGE_BYTES));
        let (stream, _) = tokio_tungstenite::connect_async_with_config(ws_url, Some(config), false)
            .await
            .map_err(|e| format!("connect to devtools at {ws_url}: {e}"))?;

        let (mut sink, mut source) = stream.split();
        let (outgoing, mut outbox) = mpsc::unbounded_channel::<Message>();
        let (events, _) = broadcast::channel(EVENT_CHANNEL_CAPACITY);
        let pending: PendingCalls = Arc::new(Mutex::new(HashMap::new()));
        let closed = Arc::new(AtomicBool::new(false));

        tokio::spawn(async move {
            while let Some(msg) = outbox.recv().await {
                if sink.send(msg).await.is_err() {
                    break;
                }
            }
        });

        let reader_pending = pending.clone();
        let reader_events = events.clone();
        let reader_closed = closed.clone();
        tokio::spawn(async move {
            while let Some(Ok(msg)) = source.next().await {
                let text = match msg {
                    Message::Text(t) => t.to_string(),
                    Message::Binary(b) => String::from_utf8_lossy(&b).into_owned(),
                    Message::Close(_) => break,
                    _ => continue,
                };
                let Ok(value) = serde_json::from_str::<Value>(&text) else {
                    continue;
                };

                if let Some(id) = value.get("id").and_then(Value::as_i64) {
                    if let Some(tx) = reader_pending.lock().await.remove(&id) {
                        let _ = tx.send(match value.get("error") {
                            Some(err) => Err(cdp_error_message(err)),
                            None => Ok(value.get("result").cloned().unwrap_or(Value::Null)),
                        });
                    }
                } else if let Some(method) = value.get("method").and_then(Value::as_str) {
                    // A send error just means nobody is subscribed yet.
                    let _ = reader_events.send(CdpEvent {
                        session_id: value
                            .get("sessionId")
                            .and_then(Value::as_str)
                            .map(str::to_string),
                        method: method.to_string(),
                        params: value.get("params").cloned().unwrap_or(Value::Null),
                    });
                }
            }
            // The socket is gone: fail every in-flight call rather than letting
            // callers sit on their timeouts.
            reader_closed.store(true, Ordering::SeqCst);
            for (_, tx) in reader_pending.lock().await.drain() {
                let _ = tx.send(Err("browser disconnected".into()));
            }
        });

        Ok(Arc::new(CdpConnection {
            next_id: AtomicI64::new(1),
            outgoing,
            pending,
            events,
            closed,
        }))
    }

    pub fn is_closed(&self) -> bool {
        self.closed.load(Ordering::SeqCst)
    }

    pub fn subscribe(&self) -> broadcast::Receiver<CdpEvent> {
        self.events.subscribe()
    }

    pub async fn call(
        &self,
        session_id: Option<&str>,
        method: &str,
        params: Value,
    ) -> Result<Value, String> {
        self.call_with_timeout(session_id, method, params, DEFAULT_CALL_TIMEOUT)
            .await
    }

    pub async fn call_with_timeout(
        &self,
        session_id: Option<&str>,
        method: &str,
        params: Value,
        timeout: Duration,
    ) -> Result<Value, String> {
        if self.is_closed() {
            return Err("browser disconnected".into());
        }
        let id = self.next_id.fetch_add(1, Ordering::SeqCst);
        let mut message = json!({ "id": id, "method": method, "params": params });
        if let Some(sid) = session_id {
            message["sessionId"] = json!(sid);
        }

        let (tx, rx) = oneshot::channel();
        self.pending.lock().await.insert(id, tx);
        if self
            .outgoing
            .send(Message::Text(message.to_string().into()))
            .is_err()
        {
            self.pending.lock().await.remove(&id);
            return Err("browser disconnected".into());
        }

        match tokio::time::timeout(timeout, rx).await {
            Ok(Ok(result)) => result.map_err(|e| format!("{method}: {e}")),
            Ok(Err(_)) => Err(format!("{method}: connection closed")),
            Err(_) => {
                self.pending.lock().await.remove(&id);
                Err(format!("{method}: timed out after {timeout:?}"))
            }
        }
    }
}

/// CDP errors are `{code, message, data?}`; `data` usually holds the part that
/// actually says what went wrong.
fn cdp_error_message(err: &Value) -> String {
    let message = err
        .get("message")
        .and_then(Value::as_str)
        .unwrap_or("unknown error");
    match err.get("data").and_then(Value::as_str) {
        Some(data) if !data.is_empty() => format!("{message} ({data})"),
        _ => message.to_string(),
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn error_messages_include_the_data_field() {
        let err =
            json!({"code": -32000, "message": "Could not find node", "data": "for selector .x"});
        assert_eq!(
            cdp_error_message(&err),
            "Could not find node (for selector .x)"
        );
    }

    #[test]
    fn error_messages_survive_a_missing_data_field() {
        assert_eq!(
            cdp_error_message(&json!({"message": "boom"})),
            "boom".to_string()
        );
        assert_eq!(cdp_error_message(&json!({})), "unknown error".to_string());
    }

    /// The 16 MiB tungstenite default silently kills the connection on a
    /// full-page screenshot, so the override must stay well above it.
    #[test]
    fn message_cap_is_far_above_a_full_page_screenshot() {
        const { assert!(MAX_MESSAGE_BYTES > 64 << 20) };
    }
}
