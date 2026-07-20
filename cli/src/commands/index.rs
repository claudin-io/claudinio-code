//! `claudinio index` — indexa símbolos + embeddings de um workspace no mesmo DB
//! machine-local que o app usa.

use crate::model;
use claudinio_core::code_intel::db::IndexDb;
use claudinio_core::code_intel::indexer::{self, IndexProgress, ProgressSink};

/// Reporta progresso numa linha de status no stderr.
struct StderrProgress;

impl ProgressSink for StderrProgress {
    fn emit(&self, p: IndexProgress) {
        eprint!(
            "\r  {:<10} {}/{} arquivos · {} símbolos          ",
            p.status, p.files_indexed, p.total_files, p.symbols_indexed
        );
    }
}

pub async fn run(path: Option<String>) -> anyhow::Result<()> {
    let ws = model::resolve_workspace(path)?;
    let root = ws.to_string_lossy().to_string();
    let db_path = model::index_db_path(&ws);
    if let Some(parent) = db_path.parent() {
        std::fs::create_dir_all(parent).ok();
    }
    let db = IndexDb::open(&db_path).map_err(anyhow::Error::msg)?;

    println!("Indexando {root}");
    let progress = StderrProgress;

    // Fase 1: símbolos (tree-sitter). Sem embedder — os embeddings vêm depois.
    let (files, symbols) =
        indexer::scan_workspace(&db, &root, Some(&progress), None, None).map_err(anyhow::Error::msg)?;
    eprintln!();
    println!("Símbolos: {files} arquivos, {symbols} símbolos.");

    // Fase 2: embeddings (ONNX/MiniLM), baixando o modelo no primeiro uso.
    let embedder = model::load_embedder(&ws).await?;
    let (emb_files, embeddings) =
        indexer::generate_all_embeddings(&db, &embedder, Some(&progress), &root)
            .map_err(anyhow::Error::msg)?;
    eprintln!();
    println!("Embeddings: {embeddings} gerados ({emb_files} arquivos processados).");
    Ok(())
}
