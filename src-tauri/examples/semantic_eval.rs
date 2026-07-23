//! Calibration eval for hybrid search: indexes a workspace, runs real
//! queries extracted from past sessions, and reports rank of the expected
//! files per query class plus score distributions. Used to compare embedding
//! models and tune `HybridParams` (the production values live in
//! `HybridParams::default()` — update them from a sweep run, not by hand).
//!
//! Usage:
//!   cargo run --release --example semantic_eval -- <model_dir> <workspace_root> <queries.json>
//!   cargo run --release --example semantic_eval -- <model_dir> <ws> <queries.json> --sweep
//!   cargo run --release --example semantic_eval -- <model_dir> <ws> <queries.json> --no-vector
//!
//! --sweep runs a grid over fusion parameters and prints one row per combo.
//! --no-vector skips the model entirely (BM25 leg only) — this simulates the
//! pending-embeddings window; exact-term classes must still pass.

use claudinio_code_lib::code_intel::db::{HybridParams, IndexDb};
use claudinio_code_lib::code_intel::embeddings::CodeEmbedder;
use claudinio_code_lib::code_intel::indexer;
use std::collections::BTreeMap;
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
    #[serde(default = "default_class")]
    class: String,
}

fn default_class() -> String {
    "concept".into()
}

#[derive(serde::Deserialize)]
struct NegativeCase {
    query: String,
}

#[derive(Default, Clone)]
struct ClassStats {
    n: usize,
    top1: usize,
    top3: usize,
    top15: usize,
}

struct Summary {
    per_class: BTreeMap<String, ClassStats>,
    neg_empty: usize,
    neg_total: usize,
    relevant_scores: Vec<f32>,
    irrelevant_scores: Vec<f32>,
}

impl Summary {
    fn overall(&self) -> ClassStats {
        let mut o = ClassStats::default();
        for s in self.per_class.values() {
            o.n += s.n;
            o.top1 += s.top1;
            o.top3 += s.top3;
            o.top15 += s.top15;
        }
        o
    }
}

fn main() {
    let args: Vec<String> = std::env::args().collect();
    let flags: Vec<&String> = args.iter().filter(|a| a.starts_with("--")).collect();
    let positional: Vec<&String> = args
        .iter()
        .skip(1)
        .filter(|a| !a.starts_with("--"))
        .collect();
    if positional.len() != 3 {
        eprintln!("usage: semantic_eval <model_dir> <workspace_root> <queries.json> [--sweep] [--no-vector]");
        std::process::exit(1);
    }
    let (model_dir, root, queries_path) = (positional[0], positional[1], positional[2]);
    let sweep = flags.iter().any(|f| *f == "--sweep");
    let no_vector = flags.iter().any(|f| *f == "--no-vector");

    let eval: EvalSet =
        serde_json::from_str(&std::fs::read_to_string(queries_path).expect("read queries.json"))
            .expect("parse queries.json");

    let mut embedder = if no_vector {
        None
    } else {
        Some(CodeEmbedder::load(Path::new(model_dir)).expect("load model"))
    };

    let db_path = std::env::temp_dir().join(format!("semantic_eval_{}.db", std::process::id()));
    let _ = std::fs::remove_file(&db_path);
    let db = IndexDb::open(&db_path).expect("open db");

    eprintln!("indexing {root} ...");
    let t = std::time::Instant::now();
    let (files, symbols) =
        indexer::scan_workspace(&db, root, None, embedder.as_mut(), None, None).expect("scan");
    eprintln!(
        "indexed {files} files, {symbols} symbols in {:.1}s",
        t.elapsed().as_secs_f32()
    );

    // Encode every query once — the sweep re-runs search, not the model.
    let pos_vecs: Vec<Option<Vec<f32>>> = eval
        .positive
        .iter()
        .map(|c| {
            embedder
                .as_mut()
                .map(|e| e.encode_query(&c.query).expect("encode query"))
        })
        .collect();
    let neg_vecs: Vec<Option<Vec<f32>>> = eval
        .negative
        .iter()
        .map(|c| {
            embedder
                .as_mut()
                .map(|e| e.encode_query(&c.query).expect("encode query"))
        })
        .collect();

    if sweep {
        // The decisive levers are the two gates (vector-leg cosine entry and
        // the final hybrid score) plus the BM25 weight; rrf_k trades against
        // min_hybrid_score on the same axis, so it stays at the default.
        println!("rrf_k  min_hyb  w_bm25  min_cos |  top1  top3  top15  neg  | exact-id top1");
        for rrf_k in [60.0f32] {
            for min_hybrid_score in [0.35f32, 0.40, 0.45] {
                for w_bm25 in [0.7f32, 1.0] {
                    for min_cosine_candidate in [0.35f32, 0.40, 0.45, 0.50] {
                        let params = HybridParams {
                            rrf_k,
                            w_bm25,
                            min_hybrid_score,
                            min_cosine_candidate,
                            ..HybridParams::default()
                        };
                        let s = run_eval(&db, &eval, &pos_vecs, &neg_vecs, &params, false);
                        let o = s.overall();
                        let exact = s
                            .per_class
                            .get("exact-identifier")
                            .cloned()
                            .unwrap_or_default();
                        println!(
                            "{:5} {:8} {:7} {:8} | {:4}% {:4}% {:5}% {:2}/{} | {:3}%",
                            rrf_k,
                            min_hybrid_score,
                            w_bm25,
                            min_cosine_candidate,
                            100 * o.top1 / o.n.max(1),
                            100 * o.top3 / o.n.max(1),
                            100 * o.top15 / o.n.max(1),
                            s.neg_empty,
                            s.neg_total,
                            100 * exact.top1 / exact.n.max(1),
                        );
                    }
                }
            }
        }
    } else {
        let params = HybridParams::default();
        let s = run_eval(&db, &eval, &pos_vecs, &neg_vecs, &params, true);
        let o = s.overall();
        println!(
            "\n=== SUMMARY ({}) ===",
            if no_vector { "BM25-only" } else { "hybrid" }
        );
        println!("positive queries: {}", o.n);
        println!(
            "  top-1 (unique file): {} ({}%)",
            o.top1,
            100 * o.top1 / o.n.max(1)
        );
        println!(
            "  top-3 (unique file): {} ({}%)",
            o.top3,
            100 * o.top3 / o.n.max(1)
        );
        println!(
            "  anywhere in top-15:  {} ({}%)",
            o.top15,
            100 * o.top15 / o.n.max(1)
        );
        println!("negative queries empty: {}/{}", s.neg_empty, s.neg_total);
        println!("\nper class (top1/top3/top15 of n):");
        for (class, c) in &s.per_class {
            println!(
                "  {:16} {:2}/{:2}/{:2} of {:2}  ({}% / {}% / {}%)",
                class,
                c.top1,
                c.top3,
                c.top15,
                c.n,
                100 * c.top1 / c.n.max(1),
                100 * c.top3 / c.n.max(1),
                100 * c.top15 / c.n.max(1),
            );
        }
        let mut rel = s.relevant_scores;
        let mut irr = s.irrelevant_scores;
        print_dist("relevant score dist  ", &mut rel);
        print_dist("irrelevant score dist", &mut irr);
    }

    let _ = std::fs::remove_file(&db_path);
}

fn run_eval(
    db: &IndexDb,
    eval: &EvalSet,
    pos_vecs: &[Option<Vec<f32>>],
    neg_vecs: &[Option<Vec<f32>>],
    params: &HybridParams,
    verbose: bool,
) -> Summary {
    let mut summary = Summary {
        per_class: BTreeMap::new(),
        neg_empty: 0,
        neg_total: eval.negative.len(),
        relevant_scores: Vec::new(),
        irrelevant_scores: Vec::new(),
    };

    if verbose {
        println!("\n=== POSITIVE QUERIES ===");
    }
    for (case, qvec) in eval.positive.iter().zip(pos_vecs.iter()) {
        let results = db
            .search_hybrid_with(&case.query, qvec.as_deref(), 15, params)
            .expect("search");

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
                || (r.kind == "doc_section"
                    && case.expected.iter().any(|e| r.name.contains(e.as_str())));
            if is_hit {
                if rank.is_none() {
                    rank = Some(file_rank);
                }
                summary.relevant_scores.push(r.score);
            } else {
                summary.irrelevant_scores.push(r.score);
            }
        }
        let stats = summary.per_class.entry(case.class.clone()).or_default();
        stats.n += 1;
        if rank == Some(1) {
            stats.top1 += 1;
        }
        if matches!(rank, Some(r) if r <= 3) {
            stats.top3 += 1;
        }
        if rank.is_some() {
            stats.top15 += 1;
        }
        if verbose {
            let top: Vec<String> = results
                .iter()
                .take(3)
                .map(|r| {
                    format!(
                        "{}:{} ({:.3}/{})",
                        Path::new(&r.file_path)
                            .file_name()
                            .unwrap_or_default()
                            .to_string_lossy(),
                        r.name,
                        r.score,
                        &r.match_type[..1],
                    )
                })
                .collect();
            println!(
                "[rank={}] [{}] {:52} -> {}",
                rank.map(|r| r.to_string()).unwrap_or_else(|| "MISS".into()),
                &case.class[..case.class.len().min(8)],
                truncate(&case.query, 50),
                top.join(" | ")
            );
        }
    }

    if verbose {
        println!("\n=== NEGATIVE QUERIES (expect empty) ===");
    }
    for (case, qvec) in eval.negative.iter().zip(neg_vecs.iter()) {
        let results = db
            .search_hybrid_with(&case.query, qvec.as_deref(), 15, params)
            .expect("search");
        if results.is_empty() {
            summary.neg_empty += 1;
        }
        if verbose {
            println!(
                "[{}] {:60} -> {} results (top score {})",
                if results.is_empty() { "OK  " } else { "LEAK" },
                truncate(&case.query, 58),
                results.len(),
                results
                    .first()
                    .map(|r| format!("{:.3}", r.score))
                    .unwrap_or_else(|| "-".into())
            );
        }
    }

    summary
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
        format!(
            "{}…",
            &s[..s.char_indices().nth(n).map(|(i, _)| i).unwrap_or(s.len())]
        )
    }
}
