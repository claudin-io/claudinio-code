pub mod db;
pub mod embeddings;
pub mod fallback;
pub mod indexer;
pub mod parser;
pub mod text;
pub mod thread_priority;
pub mod watcher;

/// Serializa a indexação entre workspaces: só um scan/embedding roda por vez.
/// Vive no core porque o watcher (core) e o comando `open_workspace` (desktop)
/// compartilham o mesmo permit.
pub static INDEX_SEMAPHORE: tokio::sync::Semaphore = tokio::sync::Semaphore::const_new(1);
