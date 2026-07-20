//! `claudinio search` — busca híbrida (BM25 + vetorial com RRF) no índice.

use crate::model;
use claudinio_core::code_intel::db::IndexDb;

pub async fn run(query: String, path: Option<String>, limit: i64) -> anyhow::Result<()> {
    let ws = model::resolve_workspace(path)?;
    let db_path = model::index_db_path(&ws);
    if !db_path.exists() {
        anyhow::bail!("Índice não encontrado para {}. Rode `claudinio index` antes.", ws.display());
    }
    let db = IndexDb::open(&db_path).map_err(anyhow::Error::msg)?;

    // Perna vetorial é opcional: se o modelo não carregar, degrada para BM25.
    let query_vec: Option<Vec<f32>> = match model::load_embedder(&ws).await {
        Ok(emb) => emb.lock().ok().and_then(|mut e| e.encode_query(&query).ok()),
        Err(e) => {
            eprintln!("(embeddings indisponíveis, usando só BM25: {e})");
            None
        }
    };

    let results = db
        .search_hybrid(&query, query_vec.as_deref(), limit as usize)
        .map_err(anyhow::Error::msg)?;

    if results.is_empty() {
        println!("Nenhum resultado.");
        return Ok(());
    }
    for r in results {
        println!(
            "{}:{}  [{}]  {} {}  ({:.3})",
            r.file_path, r.start_line, r.match_type, r.kind, r.name, r.score
        );
        if let Some(sig) = &r.signature {
            let sig = sig.trim();
            if !sig.is_empty() {
                println!("      {sig}");
            }
        }
    }
    Ok(())
}
