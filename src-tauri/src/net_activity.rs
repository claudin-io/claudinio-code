//! Global network-activity tracker behind the status-bar network indicator.
//!
//! Every outbound request/stream registers a [`NetGuard`] (RAII) so the UI can
//! show what is using the network and why — including agent runs the user may
//! have forgotten about. The tracker emits the full list of active operations
//! on the `network-activity` Tauri event whenever it changes.

use serde::Serialize;
use std::sync::atomic::{AtomicU64, Ordering};
use std::sync::{Mutex, OnceLock};
use std::time::Instant;
use tauri::Emitter;

pub const EVENT_NAME: &str = "network-activity";
/// Minimum interval between byte-count re-emits for a streaming op.
const BYTES_EMIT_INTERVAL_MS: u128 = 1000;

/// What kind of subsystem opened the connection. The frontend maps each
/// variant to a localized name + explanation (`net.*` locale keys).
#[derive(Clone, Copy, Debug, PartialEq, Eq, Serialize)]
#[serde(rename_all = "snake_case")]
pub enum NetSource {
    LlmStream,
    LlmClassify,
    LlmOneShot,
    ListModels,
    Auth,
    SkillsIndex,
    SkillFetch,
    EmbeddingModelDownload,
    WebSearch,
    Mcp,
}

struct NetOp {
    id: u64,
    source: NetSource,
    detail: String,
    started: Instant,
    bytes: u64,
}

/// Serialized snapshot of one active operation, sent to the frontend.
#[derive(Clone, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct NetOpView {
    pub id: u64,
    pub source: NetSource,
    pub detail: String,
    pub elapsed_ms: u64,
    pub bytes: u64,
}

#[derive(Default)]
struct Tracker {
    app: Mutex<Option<tauri::AppHandle>>,
    ops: Mutex<Vec<NetOp>>,
    last_bytes_emit: Mutex<Option<Instant>>,
}

static NEXT_ID: AtomicU64 = AtomicU64::new(1);

fn tracker() -> &'static Tracker {
    static TRACKER: OnceLock<Tracker> = OnceLock::new();
    TRACKER.get_or_init(Tracker::default)
}

/// Store the AppHandle once at startup (`.setup()` in lib.rs) so guards can
/// emit events without threading an AppHandle through every network call.
pub fn set_app_handle(handle: tauri::AppHandle) {
    if let Ok(mut app) = tracker().app.lock() {
        *app = Some(handle);
    }
}

fn emit_snapshot() {
    let t = tracker();
    let views: Vec<NetOpView> = match t.ops.lock() {
        Ok(ops) => ops
            .iter()
            .map(|op| NetOpView {
                id: op.id,
                source: op.source,
                detail: op.detail.clone(),
                elapsed_ms: op.started.elapsed().as_millis() as u64,
                bytes: op.bytes,
            })
            .collect(),
        Err(_) => return,
    };
    if let Ok(app) = t.app.lock() {
        if let Some(app) = app.as_ref() {
            let _ = app.emit(EVENT_NAME, &views);
        }
    }
}

/// RAII handle for one network operation: registering emits the updated op
/// list, dropping (any exit path — success, error, interrupt) removes the op
/// and emits again.
pub struct NetGuard {
    id: u64,
}

impl NetGuard {
    pub fn begin(source: NetSource, detail: impl Into<String>) -> Self {
        let id = NEXT_ID.fetch_add(1, Ordering::Relaxed);
        if let Ok(mut ops) = tracker().ops.lock() {
            ops.push(NetOp {
                id,
                source,
                detail: detail.into(),
                started: Instant::now(),
                bytes: 0,
            });
        }
        emit_snapshot();
        NetGuard { id }
    }

    /// Record received bytes for a streaming op. Re-emits at most once per
    /// second so a fast SSE stream doesn't flood the UI with events.
    pub fn add_bytes(&self, n: u64) {
        let t = tracker();
        if let Ok(mut ops) = t.ops.lock() {
            if let Some(op) = ops.iter_mut().find(|op| op.id == self.id) {
                op.bytes += n;
            }
        }
        let should_emit = match t.last_bytes_emit.lock() {
            Ok(mut last) => {
                let due = last.map_or(true, |l| l.elapsed().as_millis() >= BYTES_EMIT_INTERVAL_MS);
                if due {
                    *last = Some(Instant::now());
                }
                due
            }
            Err(_) => false,
        };
        if should_emit {
            emit_snapshot();
        }
    }
}

impl Drop for NetGuard {
    fn drop(&mut self) {
        if let Ok(mut ops) = tracker().ops.lock() {
            ops.retain(|op| op.id != self.id);
        }
        emit_snapshot();
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn guard_registers_and_unregisters() {
        let count = || tracker().ops.lock().unwrap().len();
        let before = count();
        let g = NetGuard::begin(NetSource::WebSearch, "test");
        assert_eq!(count(), before + 1);
        g.add_bytes(42);
        assert_eq!(
            tracker()
                .ops
                .lock()
                .unwrap()
                .iter()
                .find(|op| op.id == g.id)
                .unwrap()
                .bytes,
            42
        );
        drop(g);
        assert_eq!(count(), before);
    }
}
