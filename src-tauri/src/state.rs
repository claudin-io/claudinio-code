use crate::agent::provider::AgentConfig;
use crate::agent::session::{AnswerMap, ApprovalMap, ModeCtl, ModeOrigin, SessionMode, SteeringCtl};
use crate::agent::skills::SkillManager;
use crate::code_intel::db::IndexDb;
use crate::code_intel::embeddings::SharedEmbedder;
use crate::lsp::manager::LspManager;
use std::collections::HashMap;
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

/// Everything scoped to one open workspace (folder). Each open folder gets its
/// own entry so multiple workspaces can run agents in parallel without
/// clobbering each other's index, LSP servers, or active session.
pub struct WorkspaceState {
    pub root: PathBuf,
    pub index_db: Arc<IndexDb>,
    pub skills_manager: Arc<Mutex<SkillManager>>,
    /// One LSP manager per workspace: `LspManager` keys servers by language,
    /// so a shared manager would answer workspace B from workspace A's root.
    pub lsp_manager: Arc<Mutex<LspManager>>,
    pub _watcher: Mutex<Option<crate::code_intel::watcher::FileWatcher>>,
    pub watcher_warning: Mutex<Option<String>>,
    pub active_session: Mutex<Option<SessionHandle>>,
}

pub struct AppState {
    pub config: Mutex<AgentConfig>,
    pub approvals: ApprovalMap,
    pub answers: AnswerMap,
    pub workspaces: Mutex<HashMap<PathBuf, Arc<WorkspaceState>>>,
    /// Steering controllers keyed by session id, so interrupt/steer target the
    /// right run when multiple workspaces execute in parallel. Entries are
    /// removed when the run's workflow task finishes. Arc so the workflow task
    /// can clean up its own entry after the Tauri state borrow ends.
    pub steering: Arc<Mutex<HashMap<String, Arc<SteeringCtl>>>>,
    /// Mode controllers keyed by session id: the current Brain/Builder
    /// state shared between the UI toggle and a running workflow. Initialized
    /// lazily from the session's JSONL (last Mode record).
    pub modes: Arc<Mutex<HashMap<String, Arc<ModeCtl>>>>,
    pub embedding_model: Arc<Mutex<Option<SharedEmbedder>>>,
}

impl AppState {
    pub fn new() -> Self {
        Self {
            config: Mutex::new(crate::agent::provider::load_config()),
            approvals: Arc::new(Mutex::new(std::collections::HashMap::new())),
            answers: Arc::new(Mutex::new(std::collections::HashMap::new())),
            workspaces: Mutex::new(HashMap::new()),
            steering: Arc::new(Mutex::new(HashMap::new())),
            modes: Arc::new(Mutex::new(HashMap::new())),
            embedding_model: Arc::new(Mutex::new(None)),
        }
    }

    pub async fn workspace(&self, path: &str) -> Result<Arc<WorkspaceState>, String> {
        let map = self.workspaces.lock().await;
        map.get(std::path::Path::new(path))
            .cloned()
            .ok_or_else(|| format!("workspace not open: {path}"))
    }

    pub async fn steering_for(&self, session_id: &str) -> Arc<SteeringCtl> {
        let mut map = self.steering.lock().await;
        map.entry(session_id.to_string())
            .or_insert_with(|| Arc::new(SteeringCtl::new()))
            .clone()
    }

    pub async fn remove_steering(&self, session_id: &str) {
        let mut map = self.steering.lock().await;
        map.remove(session_id);
    }

    pub fn steering_map(&self) -> Arc<Mutex<HashMap<String, Arc<SteeringCtl>>>> {
        self.steering.clone()
    }

    /// The mode controller for a session, created on first access from the
    /// session's persisted Mode records (default: Builder set by Human).
    pub async fn mode_for(&self, session_id: &str, store_path: &std::path::Path) -> Arc<ModeCtl> {
        let mut map = self.modes.lock().await;
        map.entry(session_id.to_string())
            .or_insert_with(|| {
                let (mode, origin) = crate::agent::persist::load_records(store_path)
                    .ok()
                    .and_then(|recs| crate::agent::persist::last_mode(&recs))
                    .and_then(|(m, o)| {
                        SessionMode::parse(&m).map(|m| {
                            (
                                m,
                                if o == "agent" { ModeOrigin::Agent } else { ModeOrigin::Human },
                            )
                        })
                    })
                    .unwrap_or((SessionMode::Builder, ModeOrigin::Human));
                Arc::new(ModeCtl::new(mode, origin))
            })
            .clone()
    }
}
