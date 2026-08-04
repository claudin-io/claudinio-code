//! Run the quality harness against a real project, outside the app.
//!
//! The gate normally only runs inside an agent session, which makes it awkward
//! to inspect: you cannot see what the harness would detect, run, or block on
//! without driving a whole conversation. This binary is the same code path with
//! the session peeled off — point it at any folder and it prints the project it
//! detected, the commands it would run, and the verdict it would reach.
//!
//! Usage:
//!   cargo run --example quality_check -- <workspace_root>
//!   cargo run --example quality_check -- <workspace_root> tests
//!   cargo run --example quality_check -- <workspace_root> tests,coverage
//!   cargo run --example quality_check -- <workspace_root> --detect-only
//!
//! Layers default to whatever the workspace enforces. `--detect-only` prints
//! the detected stacks and their commands without executing anything, which is
//! the fast way to check that detection got your project right.

use claudinio_code_lib::quality::{self, Layer, QualityConfig};
use std::path::PathBuf;

#[tokio::main]
async fn main() {
    let mut args = std::env::args().skip(1);
    let Some(root) = args.next() else {
        eprintln!("usage: quality_check <workspace_root> [layers|--detect-only]");
        std::process::exit(2);
    };
    let root = PathBuf::from(root)
        .canonicalize()
        .unwrap_or_else(|e| panic!("cannot resolve workspace: {e}"));
    let second = args.next();
    let detect_only = second.as_deref() == Some("--detect-only");

    let cfg = QualityConfig::load(&root);
    println!("workspace: {}", root.display());
    println!(
        "enforced:  {}",
        if cfg.enforced_layers.is_empty() {
            "nothing (observation mode — reports, never blocks)".to_string()
        } else {
            cfg.enforced_layers
                .iter()
                .map(|l| l.as_str())
                .collect::<Vec<_>>()
                .join(", ")
        }
    );
    println!(
        "threshold: {:.1}% of changed lines",
        cfg.diff_coverage_threshold
    );

    let profile = quality::profile::detect(&root, &cfg);
    if profile.stacks.is_empty() {
        println!(
            "\nNo test-capable project detected. Set quality.test_cmd in .claudinio.json \
             to tell the harness how to run this project's tests."
        );
        std::process::exit(1);
    }
    println!("\n── detected stacks ──");
    for s in &profile.stacks {
        println!("{} at {}", s.name, s.root.display());
        println!("  tests:    {}", s.test_cmd);
        println!(
            "  coverage: {}",
            s.coverage_cmd.as_deref().unwrap_or("(none known)")
        );
    }

    let base = quality::evidence::git_head(&root);
    match quality::diff::changed_lines(&root, base.as_deref()) {
        Some(changed) => println!(
            "\nchanged since {}: {} line(s) across {} file(s)",
            base.as_deref()
                .map(|c| &c[..7.min(c.len())])
                .unwrap_or("HEAD"),
            quality::diff::total_lines(&changed),
            changed.len()
        ),
        // Not a failure, but it does mean the diff-scoped layers cannot run.
        None => println!(
            "\nno git repository here, so coverage and mutation cannot be scoped to this \
             run's changes and will report as unavailable"
        ),
    }

    if detect_only {
        println!("\n--detect-only: nothing was executed.");
        return;
    }

    let layers: Vec<Layer> = match second {
        Some(list) => list
            .split(',')
            .filter(|s| !s.trim().is_empty())
            .map(|name| Layer::parse(name).unwrap_or_else(|| panic!("unknown layer '{name}'")))
            .collect(),
        None => cfg.finish_line_layers(),
    };
    println!(
        "\n── running: {} ──",
        layers
            .iter()
            .map(|l| l.as_str())
            .collect::<Vec<_>>()
            .join(", ")
    );

    match quality::run_layers(&root, &cfg, &layers, base.as_deref(), None).await {
        Ok(report) => {
            print!("\n{}", report.summary_text());
            let detail = report.failure_detail(4_000);
            if !detail.is_empty() {
                println!("\n{detail}");
            }
            for l in &report.layers {
                if let Some(log) = &l.log_path {
                    println!("log ({}, {}): {log}", l.layer.as_str(), l.stack);
                }
            }
            println!("\ndigest: {}", report.digest);
            println!("(evidence is valid only while the digest matches — any edit invalidates it)");
            // Same convention as a test runner: non-zero when the gate blocks.
            if !report.verdict.pass {
                std::process::exit(1);
            }
        }
        Err(e) => {
            eprintln!("\nharness could not run: {e}");
            std::process::exit(2);
        }
    }
}
