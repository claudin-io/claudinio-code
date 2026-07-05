use crate::agent::provider::AgentConfig;
use crate::agent::session::ApprovalMap;
use crate::code_intel::db::IndexDb;
use crate::lsp::manager::LspManager;
use std::path::PathBuf;
use std::sync::Arc;
use tokio::sync::Mutex;

pub struct AppState {
    pub config: Mutex<AgentConfig>,
    pub approvals: ApprovalMap,
    pub index_db: Mutex<Option<Arc<IndexDb>>>,
    pub workspace_root: Mutex<Option<PathBuf>>,
    pub _watcher: Mutex<Option<crate::code_intel::watcher::FileWatcher>>,
    pub lsp_manager: Arc<Mutex<LspManager>>,
}

impl AppState {
    pub fn new() -> Self {
        Self {
            config: Mutex::new(crate::agent::provider::load_config()),
            approvals: Arc::new(Mutex::new(std::collections::HashMap::new())),
            index_db: Mutex::new(None),
            workspace_root: Mutex::new(None),
            _watcher: Mutex::new(None),
            lsp_manager: Arc::new(Mutex::new(LspManager::new())),
        }
    }
}
