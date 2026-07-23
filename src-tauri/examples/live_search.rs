//! Runs hybrid queries against an existing (live) index.db instead of
//! reindexing, to diagnose stale-embedding / calibration issues in place.
//!
//! Usage:
//!   cargo run --example live_search -- <model_dir> <index_db> <query>...
//!   cargo run --example live_search -- --no-vector <index_db> <query>...
//!     (BM25-only, exercising the pending-embeddings degradation path)

use claudinio_code_lib::code_intel::db::IndexDb;
use claudinio_code_lib::code_intel::embeddings::CodeEmbedder;
use std::path::Path;

fn main() {
    let args: Vec<String> = std::env::args().collect();
    if args.len() < 4 {
        eprintln!("usage: live_search <model_dir>|--no-vector <index_db> <query>...");
        std::process::exit(1);
    }
    let mut embedder = if args[1] == "--no-vector" {
        None
    } else {
        Some(CodeEmbedder::load(Path::new(&args[1])).expect("load model"))
    };
    let db = IndexDb::open(Path::new(&args[2])).expect("open db");

    for query in &args[3..] {
        let qvec = embedder
            .as_mut()
            .map(|e| e.encode_query(query).expect("encode query"));
        let results = db
            .search_hybrid(query, qvec.as_deref(), 10)
            .expect("search");
        println!("query: {query} -> {} results", results.len());
        for r in results.iter().take(5) {
            println!(
                "  {:.3} [{}]  {}  {}",
                r.score, r.match_type, r.file_path, r.name
            );
        }
    }
}
