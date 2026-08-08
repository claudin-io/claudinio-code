//! End-to-end check of the browser provisioning path, without the GUI.
//!
//! Downloads the pinned Chromium if needed, launches it, confirms it answers on
//! a DevTools endpoint, and shuts it back down — the same sequence the settings
//! "Test" button runs. This is not a unit test on purpose: it pulls ~190 MB and
//! starts a real browser, neither of which belongs in `cargo test`.
//!
//! Usage:
//!   cargo run --example browser_check
//!   cargo run --example browser_check -- --headless
//!   cargo run --example browser_check -- --keep-open   (leaves it running 10s)

use claudinio_code_lib::browser::{
    BrowserManager, BrowserPrefs, ExeResolution, provision, screenshot, status,
};
use std::sync::Arc;

#[tokio::main]
async fn main() {
    let args: Vec<String> = std::env::args().collect();
    let headless = args.iter().any(|a| a == "--headless");
    let keep_open = args.iter().any(|a| a == "--keep-open");

    let s = status();
    println!("platform supported: {}", s.supported);
    println!("pinned version:     {}", s.version);
    println!("installed:          {}", s.installed);
    println!("download size:      {} MB", s.download_size / 1_000_000);
    if let Some(sc) = &s.system_chrome {
        println!("system chrome:      {sc}");
    }
    if !s.supported {
        eprintln!("no Chrome for Testing build for this platform");
        std::process::exit(1);
    }

    let prefs = BrowserPrefs {
        headless,
        ..Default::default()
    };

    let exe = match prefs.resolve_exe().expect("resolve executable") {
        ExeResolution::Ready(p) => p,
        ExeResolution::NeedsDownload => {
            println!("\ndownloading Chromium {}…", s.version);
            let progress: claudinio_code_lib::download::ProgressFn =
                Arc::new(|done: u64, total: u64| {
                    if let Some(pct) = (done * 100).checked_div(total) {
                        eprint!("\r  {pct:>3}%  {} MB", done / 1_000_000);
                    }
                });
            let exe = provision::ensure_installed(Some(&progress))
                .await
                .expect("install chromium");
            eprintln!("\r  done                    ");
            exe
        }
    };
    println!("\nexecutable: {}", exe.display());

    let profile = provision::browser_dir()
        .expect("browser dir")
        .join("profile-example");
    println!("profile:    {}", profile.display());

    println!(
        "\nlaunching ({})…",
        if headless { "headless" } else { "headed" }
    );
    let started = std::time::Instant::now();
    let manager = BrowserManager::start(exe, profile, &prefs)
        .await
        .expect("launch chromium");
    println!("  endpoint: {}", manager.ws_url());
    println!("  ready in: {:?}", started.elapsed());
    println!("  alive:    {}", manager.is_alive().await);

    // --shot <url> [selector]: navigate, capture every mode, write the images
    // out so the geometry can be eyeballed against DevTools.
    if let Some(i) = args.iter().position(|a| a == "--shot") {
        let url = args.get(i + 1).map(String::as_str).unwrap_or("about:blank");
        let selector = args.get(i + 2).map(String::as_str);

        let page = manager.page().await.expect("open page");
        println!("\nnavigating to {url}…");
        let landed = page
            .navigate(url, true, std::time::Duration::from_secs(15), None)
            .await
            .expect("navigate");
        println!("  landed on: {landed}");

        // Give async errors and in-flight requests a moment to land.
        tokio::time::sleep(std::time::Duration::from_millis(600)).await;
        {
            let mut buffers = page.buffers.lock().await;
            println!("\nconsole:");
            for e in buffers.console(false, None, 50) {
                let at = match (&e.url, e.line) {
                    (Some(u), Some(l)) => format!("  ({u}:{l})"),
                    _ => String::new(),
                };
                println!("  [{:<7}] {:<9} {}{at}", e.level, e.source, e.text);
            }
            println!("\nnetwork:");
            for e in buffers.network(false, None, 50) {
                let status = match (&e.error, e.status) {
                    (Some(err), _) => format!("FAILED {err}"),
                    (None, Some(s)) => s.to_string(),
                    (None, None) => "pending".into(),
                };
                println!(
                    "  {:<5} {:<28} {:<10} {:?}B {:?}ms  {}",
                    e.method, status, e.resource_type, e.size_bytes, e.duration_ms, e.url
                );
            }
            // Second read must be empty: this is the cursor doing its job.
            println!(
                "\nsecond only_new read: {} console, {} network entries",
                buffers.console(true, None, 50).len(),
                buffers.network(true, None, 50).len()
            );
        }

        let metrics = screenshot::layout_metrics(&page).await.expect("metrics");
        println!(
            "  content:   {:.0}x{:.0} css px",
            metrics.content_width, metrics.content_height
        );

        let mut targets: Vec<(&str, screenshot::Target)> = vec![
            ("viewport", screenshot::Target::Viewport),
            ("full_page", screenshot::Target::FullPage),
            (
                "rect",
                screenshot::Target::Rect(screenshot::Rect {
                    x: 0.0,
                    y: 0.0,
                    width: 400.0,
                    height: 300.0,
                }),
            ),
        ];
        if let Some(sel) = selector {
            targets.push(("selector", screenshot::Target::Selector(sel.to_string())));
        }

        let out_dir = std::env::temp_dir().join("claudinio-browser-shots");
        std::fs::create_dir_all(&out_dir).expect("create output dir");
        for (name, target) in targets {
            match screenshot::capture(&page, &target).await {
                Ok(cap) => {
                    let bytes = base64::Engine::decode(
                        &base64::engine::general_purpose::STANDARD,
                        &cap.image.data,
                    )
                    .expect("decode base64");
                    let path = out_dir.join(format!("{name}.jpg"));
                    std::fs::write(&path, &bytes).expect("write image");
                    println!(
                        "  {name:>10}: {}x{}  {} KB  -> {}",
                        cap.image.width,
                        cap.image.height,
                        bytes.len() / 1024,
                        path.display()
                    );
                    if let Some(note) = cap.note {
                        println!("             note: {note}");
                    }
                }
                Err(e) => println!("  {name:>10}: FAILED — {e}"),
            }
        }
    }

    if keep_open {
        println!("\nleaving it open for 10s…");
        tokio::time::sleep(std::time::Duration::from_secs(10)).await;
    }

    println!("\nshutting down…");
    manager.shutdown().await;
    println!("  alive:    {}", manager.is_alive().await);
    println!("\nOK");
}
