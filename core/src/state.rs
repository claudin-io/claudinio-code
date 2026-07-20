use crate::agent::persist::SessionRecord;
use crate::agent::provider::AgentConfig;
use crate::agent::session::{AnswerMap, ApprovalMap, ModeCtl, ModeOrigin, SessionMode, SteeringCtl};
use crate::agent::skills::SkillManager;
use crate::code_intel::db::IndexDb;
use crate::code_intel::embeddings::SharedEmbedder;
use crate::code_intel::indexer::IndexProgress;
use crate::lsp::manager::LspManager;
use lru::LruCache;
use std::collections::HashMap;
use std::num::NonZeroUsize;
use std::path::PathBuf;
use std::sync::Arc;
use std::time::Instant;
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
    /// Machine-local path of the index SQLite file (under app data, never
    /// inside the workspace — see `commands::code_intel::index_db_path`).
    pub index_db_path: PathBuf,
    pub skills_manager: Arc<Mutex<SkillManager>>,
    /// One LSP manager per workspace: `LspManager` keys servers by language,
    /// so a shared manager would answer workspace B from workspace A's root.
    pub lsp_manager: Arc<Mutex<LspManager>>,
    pub _watcher: Mutex<Option<crate::code_intel::watcher::FileWatcher>>,
    pub watcher_warning: Mutex<Option<String>>,
    pub active_session: Mutex<Option<SessionHandle>>,
    /// Connected MCP servers for this workspace. Lives for the workspace's
    /// whole lifetime (not per-session) so stdio servers aren't respawned on
    /// every chat turn. `None` until the first `ensure_mcp_connected` call.
    pub mcp: Mutex<Option<Arc<crate::agent::mcp::McpManager>>>,
    /// Fingerprint of the `mcp_servers` config used for the current `mcp`
    /// connection, so a config change triggers a reconnect.
    pub mcp_fingerprint: Mutex<Option<String>>,
    /// Tracks indexing progress so tools can report status during the initial
    /// scan. `Some(progress)` = indexing in progress; `None` = indexing
    /// complete (or never started).
    pub index_progress: Arc<std::sync::Mutex<Option<IndexProgress>>>,
}

impl WorkspaceState {
    /// Abre um workspace para uso headless (CLI): abre o índice já existente e
    /// inicializa os managers vazios (watcher desligado, MCP conectado sob
    /// demanda). O app Tauri monta o `WorkspaceState` com o pipeline completo de
    /// `open_workspace` (scan + embeddings + watcher); aqui assumimos que o
    /// índice já foi criado por `claudinio index`.
    pub fn open(root: PathBuf, index_db_path: PathBuf) -> Result<Self, String> {
        let index_db = Arc::new(IndexDb::open(&index_db_path)?);
        Ok(Self {
            skills_manager: Arc::new(Mutex::new(SkillManager::new(Some(root.clone())))),
            lsp_manager: Arc::new(Mutex::new(LspManager::new())),
            root,
            index_db,
            index_db_path,
            _watcher: Mutex::new(None),
            watcher_warning: Mutex::new(None),
            active_session: Mutex::new(None),
            mcp: Mutex::new(None),
            mcp_fingerprint: Mutex::new(None),
            index_progress: Arc::new(std::sync::Mutex::new(None)),
        })
    }

    /// Connect to configured MCP servers if not already connected with the
    /// current config, and return the (possibly cached) manager. Reconnects
    /// whenever `mcp_servers` changed since the last connection.
    pub async fn ensure_mcp_connected(
        &self,
        config: &AgentConfig,
    ) -> Arc<crate::agent::mcp::McpManager> {
        let fingerprint = serde_json::to_string(&config.mcp).unwrap_or_default();
        {
            let current = self.mcp.lock().await;
            let current_fp = self.mcp_fingerprint.lock().await;
            if let (Some(mgr), Some(fp)) = (current.as_ref(), current_fp.as_ref()) {
                if *fp == fingerprint {
                    return mgr.clone();
                }
            }
        }

        // Config changed (or first run): drop the old connections (if any)
        // and reconnect from scratch.
        let stale = self.mcp.lock().await.take();
        if let Some(mgr) = stale {
            mgr.shutdown().await;
        }

        let workspace_root = self.root.to_str();
        let manager = crate::agent::mcp::McpManager::connect_all(&config.mcp, workspace_root).await;
        *self.mcp.lock().await = Some(manager.clone());
        *self.mcp_fingerprint.lock().await = Some(fingerprint);
        manager
    }
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
    /// Cancel signal for a pending OAuth loopback wait (OpenRouter connect),
    /// so the UI can abort instead of sitting out the 120s callback timeout.
    pub oauth_cancel: Mutex<Option<Arc<tokio::sync::Notify>>>,
    pub embedding_model: Arc<Mutex<Option<SharedEmbedder>>>,
    pub records_cache: std::sync::Arc<std::sync::Mutex<LruCache<PathBuf, (Vec<SessionRecord>, Instant)>>>,
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
            oauth_cancel: Mutex::new(None),
            embedding_model: Arc::new(Mutex::new(None)),
            records_cache: std::sync::Arc::new(std::sync::Mutex::new(LruCache::new(NonZeroUsize::new(64).unwrap()))),
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
