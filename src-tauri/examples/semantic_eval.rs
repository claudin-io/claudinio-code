//! Calibration eval for semantic_search: indexes a workspace, runs real
//! queries extracted from past sessions, and reports rank of the expected
//! files plus score distributions. Used to compare embedding models and
//! tune MIN_SEMANTIC_SCORE.
//!
//! Usage:
//!   cargo run --example semantic_eval -- <model_dir> <workspace_root> <queries.json>

use claudinio_code_lib::code_intel::db::IndexDb;
use claudinio_code_lib::code_intel::embeddings::CodeEmbedder;
use claudinio_code_lib::code_intel::indexer;
use std::path::Path;

#[derive(serde::Deserialize)]
struct EvalSet {
    positive: Vec<PositiveCase>,
    negative: Vec<NegativeCase>,
}

#[derive(serde::Deserialize)]
struct PositiveCase {
    query: String,
    expected: Vec<String>,
}

#[derive(serde::Deserialize)]
struct NegativeCase {
    query: String,
}

fn main() {
    let args: Vec<String> = std::env::args().collect();
    if args.len() != 4 {
        eprintln!("usage: semantic_eval <model_dir> <workspace_root> <queries.json>");
        std::process::exit(1);
    }
    let (model_dir, root, queries_path) = (&args[1], &args[2], &args[3]);

    let eval: EvalSet = serde_json::from_str(
        &std::fs::read_to_string(queries_path).expect("read queries.json"),
    )
    .expect("parse queries.json");

    let mut embedder = CodeEmbedder::load(Path::new(model_dir)).expect("load model");

    let db_path = std::env::temp_dir().join(format!("semantic_eval_{}.db", std::process::id()));
    let _ = std::fs::remove_file(&db_path);
    let db = IndexDb::open(&db_path).expect("open db");

    eprintln!("indexing {root} ...");
    let t = std::time::Instant::now();
    let (files, symbols) =
        indexer::scan_workspace(&db, root, None, Some(&mut embedder), None, None).expect("scan");
    eprintln!("indexed {files} files, {symbols} symbols in {:.1}s", t.elapsed().as_secs_f32());

    let mut top1 = 0usize;
    let mut top3 = 0usize;
    let mut anywhere = 0usize;
    let mut relevant_scores: Vec<f32> = Vec::new();
    let mut irrelevant_scores: Vec<f32> = Vec::new();

    println!("\n=== POSITIVE QUERIES ===");
    for case in &eval.positive {
        let qvec = embedder.encode_query(&case.query).expect("encode query");
        let results = db.search_by_embedding(&case.query, &qvec, 15).expect("search");

        // unique-file rank of the first expected basename
        let mut seen_files: Vec<String> = Vec::new();
        let mut rank: Option<usize> = None;
        for r in &results {
            let base = Path::new(&r.file_path)
                .file_name()
                .map(|s| s.to_string_lossy().to_string())
                .unwrap_or_default();
            if !seen_files.contains(&base) {
                seen_files.push(base.clone());
            }
            let file_rank = seen_files.iter().position(|f| f == &base).unwrap() + 1;
            // A hit is either the expected file itself, or a doc section whose
            // title names it (e.g. "6. Frontend: `src/App.tsx` — Settings
            // modal") — the agent follows that pointer just the same.
            let is_hit = case.expected.iter().any(|e| e == &base)
                || (r.kind == "doc_section" && case.expected.iter().any(|e| r.name.contains(e.as_str())));
            if is_hit {
                if rank.is_none() {
                    rank = Some(file_rank);
                }
                relevant_scores.push(r.score);
            } else {
                irrelevant_scores.push(r.score);
            }
        }
        match rank {
            Some(1) => top1 += 1,
            _ => {}
        }
        if matches!(rank, Some(r) if r <= 3) {
            top3 += 1;
        }
        if rank.is_some() {
            anywhere += 1;
        }
        let top: Vec<String> = results
            .iter()
            .take(3)
            .map(|r| {
                format!(
                    "{}:{} ({:.3})",
                    Path::new(&r.file_path).file_name().unwrap_or_default().to_string_lossy(),
                    r.name,
                    r.score
                )
            })
            .collect();
        println!(
            "[rank={}] {:60} -> {}",
            rank.map(|r| r.to_string()).unwrap_or_else(|| "MISS".into()),
            truncate(&case.query, 58),
            top.join(" | ")
        );
    }

    println!("\n=== NEGATIVE QUERIES (expect empty) ===");
    let mut neg_empty = 0usize;
    for case in &eval.negative {
        let qvec = embedder.encode_query(&case.query).expect("encode query");
        let results = db.search_by_embedding(&case.query, &qvec, 15).expect("search");
        if results.is_empty() {
            neg_empty += 1;
        }
        println!(
            "[{}] {:60} -> {} results (top score {})",
            if results.is_empty() { "OK  " } else { "LEAK" },
            truncate(&case.query, 58),
            results.len(),
            results.first().map(|r| format!("{:.3}", r.score)).unwrap_or_else(|| "-".into())
        );
    }

    let n = eval.positive.len();
    println!("\n=== SUMMARY ===");
    println!("positive queries: {n}");
    println!("  top-1 (unique file): {top1} ({}%)", 100 * top1 / n.max(1));
    println!("  top-3 (unique file): {top3} ({}%)", 100 * top3 / n.max(1));
    println!("  anywhere in top-15:  {anywhere} ({}%)", 100 * anywhere / n.max(1));
    println!("negative queries empty: {neg_empty}/{}", eval.negative.len());
    print_dist("relevant score dist  ", &mut relevant_scores);
    print_dist("irrelevant score dist", &mut irrelevant_scores);

    let _ = std::fs::remove_file(&db_path);
}

fn print_dist(label: &str, scores: &mut Vec<f32>) {
    if scores.is_empty() {
        println!("{label}: (none)");
        return;
    }
    scores.sort_by(|a, b| a.partial_cmp(b).unwrap());
    let p = |q: f32| scores[((scores.len() - 1) as f32 * q) as usize];
    println!(
        "{label}: n={:3} min={:.3} p25={:.3} p50={:.3} p75={:.3} max={:.3}",
        scores.len(),
        scores[0],
        p(0.25),
        p(0.5),
        p(0.75),
        scores[scores.len() - 1]
    );
}

fn truncate(s: &str, n: usize) -> String {
    if s.len() <= n {
        s.to_string()
    } else {
        format!("{}…", &s[..s.char_indices().nth(n).map(|(i, _)| i).unwrap_or(s.len())])
    }
}
