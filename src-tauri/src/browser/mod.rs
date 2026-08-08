//! A dedicated Chromium the agent can drive over the DevTools Protocol.
//!
//! Layout mirrors the MCP client: a handle lives on `WorkspaceState` for the
//! workspace's whole lifetime (not per session), so the page keeps its login,
//! scroll position and SPA route across chat turns instead of relaunching a
//! browser every message.
//!
//! `provision` gets the binary onto disk, `launch` starts and stops the
//! process. The CDP transport and the page API build on top of these.

pub mod buffers;
pub mod cdp;
pub mod launch;
pub mod page;
pub mod provision;
pub mod screenshot;

use serde::{Deserialize, Serialize};
use std::path::PathBuf;
use std::sync::Arc;
use tokio::sync::Mutex;

/// User-facing browser settings, persisted as part of `AgentConfig`.
#[derive(Debug, Clone, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct BrowserPrefs {
    /// Whether the browser tools are offered to the model at all. When false
    /// they are dropped from the tool list, so they cost no prompt tokens.
    #[serde(default = "default_true")]
    pub enabled: bool,
    /// Headed by default: watching the agent navigate is most of the value
    /// when debugging a UI.
    #[serde(default)]
    pub headless: bool,
    /// Use this executable instead of the managed download.
    #[serde(default)]
    pub chrome_path: Option<String>,
    #[serde(default = "default_viewport_width")]
    pub viewport_width: u32,
    #[serde(default = "default_viewport_height")]
    pub viewport_height: u32,
}

fn default_true() -> bool {
    true
}
fn default_viewport_width() -> u32 {
    1280
}
fn default_viewport_height() -> u32 {
    800
}

impl Default for BrowserPrefs {
    fn default() -> Self {
        BrowserPrefs {
            enabled: default_true(),
            headless: false,
            chrome_path: None,
            viewport_width: default_viewport_width(),
            viewport_height: default_viewport_height(),
        }
    }
}

impl BrowserPrefs {
    /// Clamp to something a browser can actually render. Values arrive from
    /// settings JSON that a user may hand-edit.
    pub fn clamped_viewport(&self) -> (u32, u32) {
        (
            self.viewport_width.clamp(320, 3840),
            self.viewport_height.clamp(240, 2160),
        )
    }

    /// The executable to launch, and whether it still needs downloading.
    pub fn resolve_exe(&self) -> Result<ExeResolution, String> {
        if let Some(p) = self.chrome_path.as_deref().filter(|p| !p.trim().is_empty()) {
            let path = PathBuf::from(p);
            return if path.exists() {
                Ok(ExeResolution::Ready(path))
            } else {
                Err(format!("configured browser not found at {p}"))
            };
        }
        if provision::is_installed() {
            return Ok(ExeResolution::Ready(provision::managed_exe()?));
        }
        Ok(ExeResolution::NeedsDownload)
    }
}

pub enum ExeResolution {
    Ready(PathBuf),
    NeedsDownload,
}

/// What the settings screen shows about the managed install.
#[derive(Debug, Clone, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct BrowserStatus {
    pub installed: bool,
    pub version: &'static str,
    pub exe_path: Option<String>,
    pub download_size: u64,
    /// A Chrome-family browser already on this machine, offered as an
    /// alternative to the download.
    pub system_chrome: Option<String>,
    pub supported: bool,
}

pub fn status() -> BrowserStatus {
    let platform = provision::host_platform();
    BrowserStatus {
        installed: provision::is_installed(),
        version: provision::CHROMIUM_VERSION,
        exe_path: provision::managed_exe()
            .ok()
            .map(|p| p.display().to_string()),
        download_size: platform
            .and_then(provision::build_for)
            .map(|b| b.size)
            .unwrap_or(0),
        system_chrome: provision::detect_system_chrome().map(|p| p.display().to_string()),
        supported: platform.is_some(),
    }
}

/// The message shown when a browser tool runs before anything is installed.
///
/// Downloading ~190 MB over someone's connection is not something a tool call
/// gets to do on its own, so the tool fails with an actionable message and the
/// user starts the download from settings.
pub fn not_installed_message() -> String {
    let s = status();
    if !s.supported {
        return format!(
            "The browser tools have no Chromium build for {}-{}.",
            std::env::consts::OS,
            std::env::consts::ARCH
        );
    }
    format!(
        "The browser is not set up yet. It needs a one-time {} MB download of \
         Chromium {}. Ask the user to open Settings → Browser and start it{}.",
        s.download_size / 1_000_000,
        s.version,
        match s.system_chrome {
            Some(p) => format!(", or to point the setting at their installed browser at {p}"),
            None => String::new(),
        }
    )
}

/// A launched browser plus the endpoint to talk to it.
pub struct BrowserManager {
    launched: Mutex<Option<launch::LaunchedBrowser>>,
    ws_url: String,
    conn: Mutex<Option<Arc<cdp::CdpConnection>>>,
    page: Mutex<Option<Arc<page::PageSession>>>,
    viewport: (u32, u32),
}

impl BrowserManager {
    pub async fn start(
        exe: PathBuf,
        user_data_dir: PathBuf,
        prefs: &BrowserPrefs,
    ) -> Result<Self, String> {
        let viewport = prefs.clamped_viewport();
        let launched = launch::launch(launch::LaunchOptions {
            exe,
            user_data_dir,
            headless: prefs.headless,
            viewport_width: viewport.0,
            viewport_height: viewport.1,
        })
        .await?;
        Ok(BrowserManager {
            ws_url: launched.ws_url.clone(),
            launched: Mutex::new(Some(launched)),
            conn: Mutex::new(None),
            page: Mutex::new(None),
            viewport,
        })
    }

    /// The CDP connection, dialled on first use.
    pub async fn connection(&self) -> Result<Arc<cdp::CdpConnection>, String> {
        let mut guard = self.conn.lock().await;
        if let Some(c) = guard.as_ref()
            && !c.is_closed()
        {
            return Ok(c.clone());
        }
        let conn = cdp::CdpConnection::connect(&self.ws_url).await?;
        *guard = Some(conn.clone());
        Ok(conn)
    }

    /// The tab the tools operate on, opened on first use.
    ///
    /// One tab per workspace on purpose: the tools address "the page", and
    /// juggling several would need a tab-handle concept in every schema for no
    /// benefit to the task at hand.
    pub async fn page(&self) -> Result<Arc<page::PageSession>, String> {
        let conn = self.connection().await?;
        let mut guard = self.page.lock().await;
        if let Some(p) = guard.as_ref()
            && !p.conn.is_closed()
        {
            return Ok(p.clone());
        }
        let page = Arc::new(page::PageSession::open(conn, self.viewport).await?);
        *guard = Some(page.clone());
        Ok(page)
    }

    pub fn ws_url(&self) -> &str {
        &self.ws_url
    }

    /// False once the process is gone — which happens routinely in headed mode,
    /// because the user can just close the window.
    pub async fn is_alive(&self) -> bool {
        let mut guard = self.launched.lock().await;
        match guard.as_mut() {
            Some(l) => matches!(l.child.try_wait(), Ok(None)),
            None => false,
        }
    }

    pub async fn shutdown(&self) {
        // Ask Chromium to close itself first: it tears down its own renderer
        // children, which killing the parent process does not.
        if let Some(conn) = self.conn.lock().await.take() {
            let _ = tokio::time::timeout(
                std::time::Duration::from_secs(3),
                conn.call(None, "Browser.close", serde_json::json!({})),
            )
            .await;
        }
        *self.page.lock().await = None;
        let taken = self.launched.lock().await.take();
        if let Some(l) = taken {
            launch::shutdown(l).await;
        }
    }
}

/// Lazy, per-workspace handle.
///
/// Nothing is downloaded or launched until a tool actually asks for it: opening
/// a folder must never cost a browser launch, let alone a download.
pub struct BrowserHandle {
    inner: Mutex<Option<Arc<BrowserManager>>>,
    workspace_root: PathBuf,
}

impl BrowserHandle {
    pub fn new(workspace_root: PathBuf) -> Self {
        BrowserHandle {
            inner: Mutex::new(None),
            workspace_root,
        }
    }

    /// Return the running browser, launching it if needed.
    ///
    /// The mutex is held across the launch so two concurrent tool calls cannot
    /// race into starting two browsers on the same profile.
    // The only caller is the browser tool, which lands in the next slice; the
    // settings "Test" button goes through `BrowserManager::start` directly
    // because it deliberately uses a throwaway profile.
    #[allow(dead_code)]
    pub async fn ensure(&self, prefs: &BrowserPrefs) -> Result<Arc<BrowserManager>, String> {
        let mut guard = self.inner.lock().await;
        if let Some(m) = guard.as_ref() {
            if m.is_alive().await {
                return Ok(m.clone());
            }
            // Closed by the user, or crashed: reap it and start over.
            m.shutdown().await;
            *guard = None;
        }

        let exe = match prefs.resolve_exe()? {
            ExeResolution::Ready(p) => p,
            ExeResolution::NeedsDownload => return Err(not_installed_message()),
        };
        let profile = launch::profile_dir_for(&provision::browser_dir()?, &self.workspace_root);
        let manager = Arc::new(BrowserManager::start(exe, profile, prefs).await?);
        *guard = Some(manager.clone());
        Ok(manager)
    }

    pub async fn shutdown(&self) {
        let taken = self.inner.lock().await.take();
        if let Some(m) = taken {
            m.shutdown().await;
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn viewport_is_clamped_to_something_renderable() {
        let prefs = BrowserPrefs {
            viewport_width: 0,
            viewport_height: 99_999,
            ..Default::default()
        };
        assert_eq!(prefs.clamped_viewport(), (320, 2160));
    }

    #[test]
    fn defaults_are_enabled_and_headed() {
        let prefs = BrowserPrefs::default();
        assert!(prefs.enabled);
        assert!(!prefs.headless, "headed is the default");
        assert_eq!(prefs.clamped_viewport(), (1280, 800));
    }

    #[test]
    fn a_configured_path_that_does_not_exist_is_an_error_not_a_download() {
        let prefs = BrowserPrefs {
            chrome_path: Some("/nope/does/not/exist".into()),
            ..Default::default()
        };
        assert!(prefs.resolve_exe().is_err());
    }

    #[test]
    fn a_blank_configured_path_falls_back_to_the_managed_install() {
        let prefs = BrowserPrefs {
            chrome_path: Some("   ".into()),
            ..Default::default()
        };
        // Either resolution is fine depending on whether this machine has the
        // managed build; what matters is that a blank string is not treated as
        // a configured path.
        assert!(prefs.resolve_exe().is_ok());
    }

    #[test]
    fn the_not_installed_message_tells_the_user_what_to_do() {
        let msg = not_installed_message();
        assert!(msg.contains("Settings") || msg.contains("no Chromium build"));
    }
}
