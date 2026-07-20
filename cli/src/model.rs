//! Helpers de workspace/índice/embedder compartilhados pelos subcomandos.

use claudinio_core::code_intel::embeddings::{self, SharedEmbedder};
use claudinio_core::paths;
use std::path::{Path, PathBuf};

/// Resolve o diretório do workspace (arg ou diretório atual), canonizado.
pub fn resolve_workspace(path: Option<String>) -> anyhow::Result<PathBuf> {
    let p = match path {
        Some(p) => PathBuf::from(p),
        None => std::env::current_dir()?,
    };
    Ok(p.canonicalize().unwrap_or(p))
}

/// Caminho do DB de índice machine-local do workspace (compartilhado com o app).
pub fn index_db_path(workspace: &Path) -> PathBuf {
    paths::index_db_path(&paths::default_app_data_dir(), workspace)
}

/// Diretórios extra onde procurar o modelo em dev (repositório local).
fn dev_model_dirs() -> Vec<PathBuf> {
    let mut v = Vec::new();
    // cli/ → ../core e ../src-tauri contêm `models/` durante o desenvolvimento.
    if let Some(root) = Path::new(env!("CARGO_MANIFEST_DIR")).parent() {
        v.push(root.join("core"));
        v.push(root.join("src-tauri"));
    }
    v
}

/// Carrega o embedder ONNX, baixando o modelo no primeiro uso se necessário.
pub async fn load_embedder(workspace: &Path) -> anyhow::Result<SharedEmbedder> {
    let data_dir = paths::default_app_data_dir();
    if let Some(dir) = paths::resolve_model_dir(&data_dir, workspace, &dev_model_dirs()) {
        return embeddings::load_shared(&dir).map_err(anyhow::Error::msg);
    }
    let cache = paths::model_cache_dir(&data_dir);
    eprintln!(
        "Baixando modelo de embeddings (~23MB) para {}…",
        cache.display()
    );
    embeddings::ensure_model_downloaded(&cache)
        .await
        .map_err(anyhow::Error::msg)?;
    embeddings::load_shared(&cache).map_err(anyhow::Error::msg)
}
