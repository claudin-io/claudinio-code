//! Global network-activity tracker behind the status-bar network indicator.
//!
//! Every outbound request/stream registers a [`NetGuard`] (RAII) so the UI can
//! show what is using the network and why — including agent runs the user may
//! have forgotten about. The tracker emits the full list of active operations
//! on the `network-activity` Tauri event whenever it changes.

use serde::Serialize;
use std::fs::OpenOptions;
use csv::Writer;
use std::sync::atomic::{AtomicU64, Ordering};
use std::sync::{Mutex, OnceLock};
use std::time::Instant;

/// Observador do snapshot de atividade de rede. O app Tauri registra um que
/// emite o evento `network-activity`; o CLI pode não registrar nenhum.
pub type NetObserver = Box<dyn Fn(Vec<NetOpView>) + Send + Sync>;

/// Set by open_workspace so NetGuard::begin() captures which project sent the request.
static CURRENT_WORKSPACE: OnceLock<String> = OnceLock::new();

pub fn set_current_workspace(workspace: String) {
    let _ = CURRENT_WORKSPACE.set(workspace);
}

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
    ProviderCatalog,
    SkillsIndex,
    SkillFetch,
    EmbeddingModelDownload,
    WebSearch,
    Mcp,
}

fn source_to_str(source: NetSource) -> &'static str {
    match source {
        NetSource::LlmStream => "llm_stream",
        NetSource::LlmClassify => "llm_classify",
        NetSource::LlmOneShot => "llm_one_shot",
        NetSource::ListModels => "list_models",
        NetSource::Auth => "auth",
        NetSource::ProviderCatalog => "provider_catalog",
        NetSource::SkillsIndex => "skills_index",
        NetSource::SkillFetch => "skill_fetch",
        NetSource::EmbeddingModelDownload => "embedding_model_download",
        NetSource::WebSearch => "web_search",
        NetSource::Mcp => "mcp",
    }
}

struct NetOp {
    id: u64,
    source: NetSource,
    detail: String,
    started: Instant,
    bytes: u64,
    workspace: String,
    status_code: Option<u16>,
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
    pub status_code: Option<u16>,
}

#[derive(Default)]
struct Tracker {
    observer: Mutex<Option<NetObserver>>,
    ops: Mutex<Vec<NetOp>>,
    last_bytes_emit: Mutex<Option<Instant>>,
}

static NEXT_ID: AtomicU64 = AtomicU64::new(1);

fn tracker() -> &'static Tracker {
    static TRACKER: OnceLock<Tracker> = OnceLock::new();
    TRACKER.get_or_init(Tracker::default)
}

/// Registra o observador uma vez no startup (`.setup()` no lib.rs do app) para
/// que os guards reportem o snapshot sem carregar um handle em cada chamada.
pub fn set_observer(observer: NetObserver) {
    if let Ok(mut obs) = tracker().observer.lock() {
        *obs = Some(observer);
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
                status_code: op.status_code,
            })
            .collect(),
        Err(_) => return,
    };
    if let Ok(obs) = t.observer.lock() {
        if let Some(obs) = obs.as_ref() {
            obs(views);
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
        let workspace = CURRENT_WORKSPACE.get().cloned().unwrap_or_default();
        if let Ok(mut ops) = tracker().ops.lock() {
            ops.push(NetOp {
                id,
                source,
                detail: detail.into(),
                started: Instant::now(),
                bytes: 0,
                workspace,
                status_code: None,
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

    /// Record the HTTP status code the call site received. Optional — if never
    /// called the column stays empty in the CSV.
    pub fn set_status(&self, code: u16) {
        if let Ok(mut ops) = tracker().ops.lock() {
            if let Some(op) = ops.iter_mut().find(|op| op.id == self.id) {
                op.status_code = Some(code);
            }
        }
    }
}

static CSV_MUTEX: std::sync::Mutex<()> = std::sync::Mutex::new(());

pub fn csv_path() -> std::path::PathBuf {
    dirs::config_dir()
        .unwrap_or_else(|| std::path::PathBuf::from("."))
        .join("claudinio-code")
        .join("network-log.csv")
}

fn append_csv_row(op: &NetOp) {
    let path = csv_path();
    if let Some(parent) = path.parent() {
        let _ = std::fs::create_dir_all(parent);
    }
    let _lock = CSV_MUTEX.lock().unwrap();
    let file_exists = path.exists();
    let file = OpenOptions::new()
        .create(true)
        .append(true)
        .open(&path);
    if let Ok(file) = file {
        let mut wtr = Writer::from_writer(file);
        if !file_exists {
            let _ = wtr.write_record(&[
                "workspace", "timestamp", "source", "detail",
                "duration_ms", "bytes", "status_code",
            ]);
        }
        let _ = wtr.write_record(&[
            &op.workspace,
            &chrono::Utc::now().to_rfc3339(),
            source_to_str(op.source),
            &op.detail,
            &op.started.elapsed().as_millis().to_string(),
            &op.bytes.to_string(),
            &op.status_code.map(|c| c.to_string()).unwrap_or_default(),
        ]);
        let _ = wtr.flush();
    }
}

impl Drop for NetGuard {
    fn drop(&mut self) {
        if let Ok(mut ops) = tracker().ops.lock() {
            if let Some(op) = ops.iter().find(|op| op.id == self.id) {
                append_csv_row(op);
            }
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
