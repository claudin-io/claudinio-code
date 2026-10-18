//! The headless eval runner. Everything is in `claudinio_code_lib::eval`;
//! this is the thin `main` so the runner is an example, not a second binary
//! the Tauri bundler would have to know about.
//!
//!   cargo run --example claudinio-eval -- validate ../eval/tasks
//!   cargo run --example claudinio-eval -- run ../eval/tasks --model <id> \
//!       --base-url <url> --api-key-env <ENV> [--protocol anthropic|openai]
//!   cargo run --example claudinio-eval -- import-swebench <instances.jsonl> <out-dir>
//!
//! See `eval/README.md`.

#[tokio::main]
async fn main() {
    let args: Vec<String> = std::env::args().skip(1).collect();
    std::process::exit(claudinio_code_lib::eval::cli(args).await);
}
