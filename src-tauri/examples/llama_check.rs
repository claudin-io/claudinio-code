//! End-to-end check of the local inference path, without the GUI.
//!
//! Installs the pinned `llama-server` if needed, downloads a small GGUF from
//! the Hub, starts a server for it and puts one tool-calling request through
//! `/v1/chat/completions` — the sequence the settings "Test" button runs, plus
//! the part that actually matters for the agent loop. Not a unit test on
//! purpose: it pulls ~430 MB and starts a real process.
//!
//! Usage:
//!   cargo run --example llama_check
//!   cargo run --example llama_check -- --repo unsloth/Qwen3-8B-GGUF --quant Q4_K_M
//!   cargo run --example llama_check -- --keep   (leaves the model installed)

use claudinio_code_lib::llama::{Engine, LocalPrefs, catalog, hardware, hf, provision, supervisor};
use std::sync::Arc;

const DEFAULT_REPO: &str = "unsloth/Qwen3-0.6B-GGUF";
const DEFAULT_QUANT: &str = "Q4_K_M";

fn engine_label(use_mlx: bool) -> &'static str {
    if use_mlx { "MLX" } else { "llama.cpp" }
}

fn arg(args: &[String], name: &str) -> Option<String> {
    let i = args.iter().position(|a| a == name)?;
    args.get(i + 1).cloned()
}

#[tokio::main]
async fn main() {
    let args: Vec<String> = std::env::args().collect();
    let repo = arg(&args, "--repo").unwrap_or_else(|| DEFAULT_REPO.to_string());
    let quant = arg(&args, "--quant").unwrap_or_else(|| DEFAULT_QUANT.to_string());
    let keep = args.iter().any(|a| a == "--keep");

    // `--mlx` exercises the Apple Silicon engine end to end: download the
    // pinned sidecar, install an MLX repo, serve it, and put a tool call
    // through it.
    let use_mlx = args.iter().any(|a| a == "--mlx");
    let prefs = LocalPrefs {
        engine: if use_mlx {
            Engine::Mlx
        } else {
            Engine::Llamacpp
        },
        ..LocalPrefs::default()
    };
    let hw = hardware::detect();
    println!(
        "hardware:   {:.0} GB RAM, {} cores, unified={}, gpu={:?}",
        hw.total_ram_bytes as f64 / 1e9,
        hw.logical_cores,
        hw.unified_memory,
        hw.gpu_name
    );
    println!(
        "budget:     {:.1} GB usable for weights",
        hardware::usable_model_bytes(&hw) as f64 / 1e9
    );

    let status = claudinio_code_lib::llama::status(&prefs);
    println!("pinned:     {} ({:?})", status.build, status.target);
    println!("installed:  {}", status.server_installed);
    if !status.supported {
        eprintln!("no llama.cpp build for this platform");
        std::process::exit(1);
    }

    if use_mlx && !status.mlx_installed {
        println!(
            "downloading the MLX runtime {} ({} MB)…",
            status.mlx_version,
            status.mlx_download_size / 1_000_000
        );
        let progress: claudinio_code_lib::download::ProgressFn =
            Arc::new(|done, total| print!("\r  mlx: {done} / {total} bytes    "));
        match provision::ensure_mlx_installed(Some(&progress)).await {
            Ok(exe) => println!("\nmlx runtime: {}", exe.display()),
            Err(e) => {
                eprintln!("\nmlx install failed: {e}");
                std::process::exit(1);
            }
        }
    }

    if !use_mlx && !status.server_installed {
        println!(
            "downloading the runtime ({} MB)…",
            status.download_size / 1_000_000
        );
        let progress: claudinio_code_lib::download::ProgressFn =
            Arc::new(|done, total| print!("\r  runtime: {} / {} bytes    ", done, total));
        match provision::ensure_installed(prefs.backend, Some(&progress)).await {
            Ok(exe) => println!("\nruntime:    {}", exe.display()),
            Err(e) => {
                eprintln!("\nruntime install failed: {e}");
                std::process::exit(1);
            }
        }
    }

    // The MLX engine needs an MLX repo; the GGUF default would not load.
    let repo = if use_mlx && repo == DEFAULT_REPO {
        "mlx-community/Qwen3-0.6B-4bit".to_string()
    } else {
        repo
    };
    println!("resolving {repo} {quant}…");
    let detail = match hf::repo_detail(&repo).await {
        Ok(d) => d,
        Err(e) => {
            eprintln!("hub lookup failed: {e}");
            std::process::exit(1);
        }
    };
    let options = hf::group_quants(&detail.files);
    println!(
        "quants:     {}",
        options
            .iter()
            .map(|o| format!(
                "{} ({:.1} GB, {} shard(s))",
                o.quant,
                o.total_bytes as f64 / 1e9,
                o.shards
            ))
            .collect::<Vec<_>>()
            .join(", ")
    );
    // An MLX repo publishes exactly one quantization, named in the repo itself,
    // so there is no GGUF-style quant list to look up.
    let fallback = hf::QuantOption {
        quant: hf::mlx_quant(&repo),
        files: Vec::new(),
        total_bytes: 0,
        shards: 0,
    };
    let option = match options
        .iter()
        .find(|o| o.quant.eq_ignore_ascii_case(&quant))
    {
        Some(o) => o,
        None if hf::is_mlx_repo(&detail.files) => &fallback,
        None => {
            eprintln!("{repo} has no {quant}");
            std::process::exit(1);
        }
    };
    println!(
        "fit:        {:?}",
        hardware::fit_verdict(option.total_bytes, &hw)
    );

    let has_template = detail
        .gguf
        .as_ref()
        .is_some_and(|g| g.chat_template.is_some());
    println!(
        "template:   {}",
        if hf::is_mlx_repo(&detail.files) {
            true
        } else {
            has_template
        }
    );
    let ctx = detail.gguf.as_ref().and_then(|g| g.context_length);
    let arch = detail.gguf.as_ref().and_then(|g| g.architecture.clone());
    let spec = if hf::is_mlx_repo(&detail.files) {
        catalog::mlx_spec(&detail.repo, &detail.files, ctx, true, arch)
    } else {
        catalog::spec_from_quant(&detail.repo, ctx, has_template, arch, option)
    };

    let progress: catalog::InstallProgressFn = Arc::new(|p: catalog::InstallProgress| {
        print!(
            "\r  {} / {}: {:.0}%    ",
            p.file_index,
            p.file_count,
            100.0 * p.overall_done as f64 / p.overall_total.max(1) as f64
        );
    });
    let model = match catalog::install(spec, Some(&progress)).await {
        Ok(m) => {
            println!("\nmodel:      {} ({} bytes)", m.display_name, m.total_bytes);
            m
        }
        Err(e) => {
            eprintln!("\nmodel download failed: {e}");
            std::process::exit(1);
        }
    };

    println!("starting the {} server…", engine_label(use_mlx));
    let started = std::time::Instant::now();
    let endpoint = match supervisor::ensure_serving(&model.key, &prefs, 0).await {
        Ok(e) => e,
        Err(e) => {
            eprintln!("start failed: {e}");
            std::process::exit(1);
        }
    };
    println!(
        "endpoint:   {} (ready in {:.1}s)",
        endpoint.base_url,
        started.elapsed().as_secs_f32()
    );

    // Unauthenticated inference must be refused: the port is on loopback, which
    // any local process — and any page the agent's own browser visits — can
    // reach. (`/v1/models` and `/health` stay open by llama-server's design;
    // they expose an opaque alias and a boolean, not compute.)
    let client = reqwest::Client::new();
    let unauth = client
        .post(format!("{}/chat/completions", endpoint.base_url))
        .json(&serde_json::json!({
            "model": model.key,
            "messages": [{"role": "user", "content": "hi"}],
            "max_tokens": 1
        }))
        .send()
        .await;
    match unauth {
        Ok(r) => {
            let status = r.status();
            println!("no-auth chat: HTTP {status} (expect 401)");
            if status != reqwest::StatusCode::UNAUTHORIZED {
                eprintln!("SECURITY: the local server accepted an unauthenticated request");
            }
        }
        Err(e) => println!("no-auth chat failed: {e}"),
    }

    // The request that matters: does this model emit a tool call?
    let body = serde_json::json!({
        "model": model.key,
        "max_tokens": 256,
        "messages": [
            {"role": "user", "content": "What is in the file src/main.rs? Use the read_file tool."}
        ],
        "tools": [{
            "type": "function",
            "function": {
                "name": "read_file",
                "description": "Read a file from the workspace",
                "parameters": {
                    "type": "object",
                    "properties": {"path": {"type": "string"}},
                    "required": ["path"]
                }
            }
        }]
    });
    let resp = client
        .post(format!("{}/chat/completions", endpoint.base_url))
        .bearer_auth(&endpoint.api_key)
        .json(&body)
        .send()
        .await;
    match resp {
        Ok(r) => {
            let status = r.status();
            let json: serde_json::Value = r.json().await.unwrap_or_default();
            let choice = &json["choices"][0]["message"];
            println!("chat:       HTTP {status}");
            println!("finish:     {}", json["choices"][0]["finish_reason"]);
            if let Some(calls) = choice["tool_calls"].as_array() {
                println!(
                    "tool_calls: {}",
                    calls
                        .iter()
                        .map(|c| format!(
                            "{}({})",
                            c["function"]["name"].as_str().unwrap_or("?"),
                            c["function"]["arguments"].as_str().unwrap_or("")
                        ))
                        .collect::<Vec<_>>()
                        .join(", ")
                );
            } else {
                println!(
                    "content:    {:?}",
                    choice["content"]
                        .as_str()
                        .unwrap_or("")
                        .chars()
                        .take(200)
                        .collect::<String>()
                );
                println!("NOTE: no tool_calls — this model may not drive the agent loop.");
            }
        }
        Err(e) => eprintln!("chat request failed: {e}"),
    }

    println!("running:    {:?}", supervisor::running().await);
    supervisor::stop(&model.key).await;
    println!("stopped.");

    if !keep {
        let _ = catalog::remove(&model.key);
        println!("removed the test model (pass --keep to retain it).");
    }
}
