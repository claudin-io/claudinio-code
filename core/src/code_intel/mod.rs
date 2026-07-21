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

/// Lista arquivos (não diretórios) do workspace, relativos a `root`,
/// respeitando `.gitignore` e pulando ocultos, ordenados. Cap em `max`. Usado
/// pelo `@`-mention do CLI/TUI (o app usa `walk_dir` do src-tauri — mesma
/// semântica: `hidden(true)`, `git_ignore(true)`, `git_global(true)`).
pub fn list_files(root: &str, max: usize) -> Vec<String> {
    let dir = std::path::Path::new(root);
    if !dir.is_dir() {
        return Vec::new();
    }
    let walker = ignore::WalkBuilder::new(dir)
        .hidden(true)
        .git_ignore(true)
        .git_global(true)
        .build();
    let mut files: Vec<String> = walker
        .filter_map(|e| e.ok())
        .filter(|e| e.depth() > 0 && e.file_type().map(|t| t.is_file()).unwrap_or(false))
        .filter_map(|e| {
            e.path()
                .strip_prefix(root)
                .ok()
                .map(|p| p.to_string_lossy().into_owned())
        })
        .take(max)
        .collect();
    files.sort();
    files
}
