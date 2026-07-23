// Prevents additional console window on Windows in release, DO NOT REMOVE!!
#![cfg_attr(not(debug_assertions), windows_subsystem = "windows")]

fn main() {
    // HF tokenizers otherwise rayon-parallelizes encode_batch across every
    // logical core, saturating the machine during indexing; the ONNX session
    // is already capped to 2 intra-op threads (see embeddings.rs). Must be set
    // before any thread spawns so the rayon pool never initializes parallel.
    //
    // SAFETY: `set_var` races with a concurrent `getenv` in any other thread.
    // This is the first statement of `main`, before Tauri or tokio start
    // anything, so this process is still single-threaded here.
    unsafe { std::env::set_var("TOKENIZERS_PARALLELISM", "false") };
    claudinio_code_lib::run()
}
