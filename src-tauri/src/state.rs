use crate::agent::provider::AgentConfig;
use crate::agent::session::{AnswerMap, ApprovalMap, SteeringCtl};
use crate::code_intel::db::IndexDb;
use crate::code_intel::embeddings::SharedEmbedder;
use crate::lsp::manager::LspManager;
use std::path::PathBuf;
use std::sync::Arc;
use tokio::sync::Mutex;

/// The conversation the user is currently in. The JSONL file at `store_path` is
/// the source of truth for its history — it is reloaded on each message, so a
/// session accumulates across turns and survives app restarts.
#[derive(Clone)]
pub struct SessionHandle {
    pub id: String,
    pub store_path: PathBuf,
}

pub struct AppState {
    pub config: Mutex<AgentConfig>,
    pub approvals: ApprovalMap,
    pub answers: AnswerMap,
    pub index_db: Mutex<Option<Arc<IndexDb>>>,
    pub workspace_root: Mutex<Option<PathBuf>>,
    pub _watcher: Mutex<Option<crate::code_intel::watcher::FileWatcher>>,
    pub lsp_manager: Arc<Mutex<LspManager>>,
    pub active_session: Mutex<Option<SessionHandle>>,
    pub steering: Arc<SteeringCtl>,
    pub embedding_model: Mutex<Option<SharedEmbedder>>,
}

impl AppState {
    pub fn new() -> Self {
        Self {
            config: Mutex::new(crate::agent::provider::load_config()),
            approvals: Arc::new(Mutex::new(std::collections::HashMap::new())),
            answers: Arc::new(Mutex::new(std::collections::HashMap::new())),
            index_db: Mutex::new(None),
            workspace_root: Mutex::new(None),
            _watcher: Mutex::new(None),
            lsp_manager: Arc::new(Mutex::new(LspManager::new())),
            active_session: Mutex::new(None),
            steering: Arc::new(SteeringCtl::new()),
            embedding_model: Mutex::new(None),
        }
    }
}
