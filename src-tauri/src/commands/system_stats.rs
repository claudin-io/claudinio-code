use serde::Serialize;
use sysinfo::{Pid, ProcessRefreshKind, ProcessesToUpdate, RefreshKind, System};
use tauri::{AppHandle, Emitter};
use tokio::time::{Duration, interval};

static EVENT: &str = "system-stats";

#[derive(Clone, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct SystemStatsPayload {
    pub cpu_percent: f32,
    pub memory_rss_bytes: u64,
}

/// Start a background task that polls the process CPU% + memory RSS every 5s
/// and emits `system-stats` events to the frontend.
pub fn start_poller(app: AppHandle) {
    tauri::async_runtime::spawn(async move {
        let mut system = System::new_with_specifics(
            RefreshKind::nothing().with_processes(ProcessRefreshKind::everything()),
        );
        let pid = Pid::from(std::process::id() as usize);
        // sysinfo reports CPU summed across cores (a 12-thread machine can
        // read "535%"); normalize to a 0-100% whole-machine share.
        let logical_cores = std::thread::available_parallelism()
            .map(|n| n.get())
            .unwrap_or(1) as f32;

        let mut ticker = interval(Duration::from_secs(5));
        loop {
            ticker.tick().await;

            // Refresh only our process — cheap
            system.refresh_processes_specifics(
                ProcessesToUpdate::Some(&[pid]),
                false,
                ProcessRefreshKind::everything(),
            );

            if let Some(process) = system.process(pid) {
                let payload = SystemStatsPayload {
                    cpu_percent: process.cpu_usage() / logical_cores,
                    // sysinfo::Process::memory() returns bytes on all platforms
                    memory_rss_bytes: process.memory(),
                };
                let _ = app.emit(EVENT, payload);
            }
        }
    });
}
