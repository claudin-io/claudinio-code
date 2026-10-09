//! Code intelligence is the `claudinio-code-intel` crate, the same code the
//! code-intel MCP plugin ships (github.com/claudin-io/code-intel). This module
//! only re-exports it, so the app keeps addressing it as `crate::code_intel::…`.
//! Tauri stays out of it: `commands::code_intel` turns the crate's progress and
//! watch callbacks into `index-progress` events.

pub use claudinio_code_intel::{
    INDEX_SEMAPHORE, db, download, embeddings, fallback, indexer, parser, text, thread_priority,
    watcher,
};
