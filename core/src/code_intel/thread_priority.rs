/// RAII guard that drops the current thread to below-normal priority for the
/// duration of CPU-heavy indexing/embedding work, so it never competes with
/// the WebView UI thread on low-core Windows machines.
///
/// Deliberately `THREAD_PRIORITY_BELOW_NORMAL` and not background mode
/// (`THREAD_MODE_BACKGROUND_BEGIN`): background mode also demotes I/O and
/// memory priority to "very low", which makes scans over network drives
/// pathologically slow.
///
/// The guard restores normal priority on drop. That matters on Tokio
/// `spawn_blocking` threads, which are pooled and reused by unrelated work
/// (agent bash, git) that must not inherit the demotion.
///
/// No-op on non-Windows: macOS/Linux schedulers already keep a 2-thread ONNX
/// load from starving the UI.
pub struct BackgroundPriority(());

impl BackgroundPriority {
    #[cfg(windows)]
    pub fn begin() -> Self {
        use windows_sys::Win32::System::Threading::{
            GetCurrentThread, SetThreadPriority, THREAD_PRIORITY_BELOW_NORMAL,
        };
        unsafe {
            SetThreadPriority(GetCurrentThread(), THREAD_PRIORITY_BELOW_NORMAL);
        }
        Self(())
    }

    #[cfg(not(windows))]
    pub fn begin() -> Self {
        Self(())
    }
}

impl Drop for BackgroundPriority {
    fn drop(&mut self) {
        #[cfg(windows)]
        unsafe {
            use windows_sys::Win32::System::Threading::{
                GetCurrentThread, SetThreadPriority, THREAD_PRIORITY_NORMAL,
            };
            SetThreadPriority(GetCurrentThread(), THREAD_PRIORITY_NORMAL);
        }
    }
}
