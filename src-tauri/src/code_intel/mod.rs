pub mod db;
pub mod embeddings;
pub mod fallback;
pub mod indexer;
pub mod parser;
pub mod text;
pub mod thread_priority;
pub mod watcher;

/// Only one workspace indexes at a time. Restoring several workspaces at
/// startup used to launch parallel scans + embedding runs that together pegged
/// every core (and hammered slow/network drives). Lives here rather than with
/// the IPC command that first acquires it, so the file watcher can respect the
/// same limit without `code_intel` depending on `commands`.
pub static INDEX_SEMAPHORE: tokio::sync::Semaphore = tokio::sync::Semaphore::const_new(1);
