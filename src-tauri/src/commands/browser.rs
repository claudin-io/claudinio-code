//! IPC surface for the browser: install, inspect and remove the managed
//! Chromium. Driving pages is the agent's job, not the UI's.

use crate::browser::{self, BrowserStatus};
use crate::download::ProgressFn;
use crate::state::AppState;
use serde::Serialize;
use std::sync::Arc;
use tauri::{Emitter, State};

#[derive(Clone, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct BrowserInstallProgress {
    pub downloaded_bytes: u64,
    pub total_bytes: u64,
    /// "download" | "extract"
    pub phase: &'static str,
}

#[tauri::command]
pub fn browser_status() -> BrowserStatus {
    browser::status()
}

/// Download and extract the pinned Chromium.
///
/// Only ever reached from an explicit click in settings: a tool call must not
/// spend ~190 MB of someone's bandwidth on its own.
#[tauri::command]
pub async fn browser_install(app: tauri::AppHandle) -> Result<BrowserStatus, String> {
    let emitter = app.clone();
    let on_progress: ProgressFn = Arc::new(move |downloaded, total| {
        let _ = emitter.emit(
            "browser-install-progress",
            BrowserInstallProgress {
                downloaded_bytes: downloaded,
                total_bytes: total,
                phase: "download",
            },
        );
    });

    browser::provision::ensure_installed(Some(&on_progress)).await?;

    let _ = app.emit(
        "browser-install-progress",
        BrowserInstallProgress {
            downloaded_bytes: 0,
            total_bytes: 0,
            phase: "extract",
        },
    );
    Ok(browser::status())
}

/// Remove the managed install. Any running browser is stopped first, since on
/// Windows the executable cannot be deleted while it is mapped.
#[tauri::command]
pub async fn browser_uninstall(state: State<'_, AppState>) -> Result<BrowserStatus, String> {
    let handles: Vec<_> = {
        let workspaces = state.workspaces.lock().await;
        workspaces.values().map(|ws| ws.browser.clone()).collect()
    };
    for h in handles {
        h.shutdown().await;
    }
    browser::provision::uninstall()?;
    Ok(browser::status())
}

/// Launch the configured browser against a throwaway profile, confirm it
/// reports a debugging endpoint, then shut it back down.
///
/// The counterpart of `mcp_test_server`: it answers "is this actually going to
/// work" before the user hits it mid-task. A missing shared library or a broken
/// download surfaces here with Chromium's own stderr attached, instead of as a
/// mystery timeout on the first tool call.
#[tauri::command]
pub async fn browser_test(state: State<'_, AppState>) -> Result<String, String> {
    let prefs = { state.config.lock().await.browser.clone() };
    let exe = match prefs.resolve_exe()? {
        browser::ExeResolution::Ready(p) => p,
        browser::ExeResolution::NeedsDownload => return Err(browser::not_installed_message()),
    };

    // A dedicated profile, so testing never disturbs a workspace's live session.
    let profile = browser::provision::browser_dir()?.join("profile-selftest");
    let manager = browser::BrowserManager::start(exe.clone(), profile, &prefs).await?;
    let endpoint = manager.ws_url().to_string();
    let alive = manager.is_alive().await;
    manager.shutdown().await;

    if !alive {
        return Err("the browser exited immediately after launching".into());
    }
    Ok(format!(
        "{} launched and answered on {endpoint}",
        exe.display()
    ))
}

/// Stop the browser for every open workspace without removing the install.
#[tauri::command]
pub async fn browser_close(state: State<'_, AppState>) -> Result<(), String> {
    let handles: Vec<_> = {
        let workspaces = state.workspaces.lock().await;
        workspaces.values().map(|ws| ws.browser.clone()).collect()
    };
    for h in handles {
        h.shutdown().await;
    }
    Ok(())
}
