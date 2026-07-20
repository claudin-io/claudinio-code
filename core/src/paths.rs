//! Resolução de caminhos machine-local, compartilhada por app e CLI.
//!
//! Índice e cache de modelo vivem FORA do workspace (SQLite/WAL não funciona
//! bem em drives de rede, e o modelo ~23MB não deve ser rebaixado por projeto).
//! É essencial que app e CLI derivem os MESMOS caminhos, senão indexariam para
//! arquivos diferentes — por isso a lógica é única aqui.

use crate::code_intel::embeddings;
use std::path::{Path, PathBuf};

/// Bundle identifier do app (Tauri usa isto para `app_data_dir`). O CLI compõe
/// `dirs::data_dir()/APP_IDENTIFIER`, batendo com o diretório do app.
pub const APP_IDENTIFIER: &str = "io.claudin.code";

/// Diretório de dados do app para processos sem Tauri (CLI). Espelha o
/// `app_data_dir()` do Tauri: `<data_dir da plataforma>/io.claudin.code`.
pub fn default_app_data_dir() -> PathBuf {
    dirs::data_dir()
        .unwrap_or_else(|| std::env::temp_dir().join("claudinio-code"))
        .join(APP_IDENTIFIER)
}

/// DB de índice machine-local de um workspace (nunca dentro do workspace).
pub fn index_db_path(app_data_dir: &Path, workspace_root: &Path) -> PathBuf {
    let stem: String = workspace_root
        .file_name()
        .map(|n| n.to_string_lossy().to_string())
        .unwrap_or_else(|| "workspace".into())
        .chars()
        .map(|c| if c.is_ascii_alphanumeric() || c == '-' || c == '_' { c } else { '_' })
        .take(40)
        .collect();
    let hash = xxhash_rust::xxh3::xxh3_64(workspace_root.to_string_lossy().as_bytes());
    app_data_dir.join("indexes").join(format!("{stem}-{hash:016x}.db"))
}

/// Diretório de cache do modelo de embeddings (app data, não por workspace).
pub fn model_cache_dir(app_data_dir: &Path) -> PathBuf {
    app_data_dir
        .join("models")
        .join(embeddings::model_cache_dirname())
}

/// Resolve o diretório do modelo já presente em disco, na ordem: diretórios
/// extras fornecidos pelo frontend (resource dir do bundle, `CARGO_MANIFEST_DIR`
/// em dev), o cache de app data, e por fim o cache legado por-workspace
/// (pré-0.1.13, read-only). Retorna `None` se nenhum tiver o arquivo do modelo.
pub fn resolve_model_dir(
    app_data_dir: &Path,
    workspace_root: &Path,
    extra_dirs: &[PathBuf],
) -> Option<PathBuf> {
    let subdir = format!("models/{}", embeddings::model_cache_dirname());
    let model_file = embeddings::model_filename();

    for base in extra_dirs {
        let p = base.join(&subdir);
        if p.join(model_file).exists() {
            return Some(p);
        }
    }
    let cache = model_cache_dir(app_data_dir);
    if cache.join(model_file).exists() {
        return Some(cache);
    }
    let legacy = workspace_root
        .join(".claudinio/models")
        .join(embeddings::model_cache_dirname());
    if legacy.join(model_file).exists() {
        return Some(legacy);
    }
    None
}
