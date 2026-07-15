# Fix: Allow Chat Messages During Workspace Indexing

## Context

**Problem:** When opening a large project, the workspace indexing (tree-sitter parsing + FTS5 population) blocks the workspace from being registered in the state HashMap. Since `send_message` and all other commands check `state.workspace(path)` which looks up this HashMap, **any attempt to send a message during indexing fails with `"workspace not open: /path"`**. The user sees `"Failed to send: workspace not open"` in the chat.

The progress bar shows "2170/2718" while `open_workspace` blocks on `scan_workspace`. The ChatPanel UI is already visible with an enabled input — but every message fails until indexing completes.

**User's chosen approach:** Allow messages to send freely. Index-dependent tools (`code_search`, `symbol_lookup`, `file_outline`, `go_to_definition`, `find_references`, `semantic_search`) should return a clear "indexing in progress (X/Y files)" message instead of failing or returning misleading empty results. Non-index tools (`read_file`, `list_dir`, `grep`, `bash`) should work normally.

**Key architectural facts discovered:**
- `IndexDb::open()` uses SQLite WAL mode — concurrent reads during writes are safe
- All 6 index-dependent tools already degrade gracefully on empty DB (return `[]`, don't error)
- `semantic_search` already has a 5-second polling loop for the embedding model
- `open_workspace` initializes the DB at line 76 (early) but only inserts workspace into HashMap at line 258 (after scan completes)
- Embeddings (Phase 5) already run in background via `tokio::spawn` — tools handle this
- The DB connection is opened per-tool-call via `open_db()` — tools don't share the indexer's connection, so no lock contention

## Solution Design

**Insert the workspace into the state HashMap immediately after DB initialization**, before `scan_workspace` starts. Add a shared `index_progress` tracker to `WorkspaceState` so index-dependent tools can check whether indexing is still in progress and report the current progress to the agent.

### Flow after fix:

```
open_workspace called
  ├── DB init (IndexDb::open)
  ├── Build WorkspaceState (with index_progress = "indexing, 0/N")
  ├── Insert into HashMap  ← MOVED HERE (was at line 258)
  ├── spawn scan_workspace (updates shared index_progress every 10 files)
  ├── ... (model download, embeddings, watcher, LSP — same as before)
  ├── After scan completes: set index_progress = None (indexing done)
  └── Return IndexStatus

  Meanwhile:
  ├── send_message → state.workspace(path) → FOUND! → agent starts
  ├── read_file, list_dir, grep, bash → work normally
  └── code_search, semantic_search, etc. → check index_progress
       ├── If "indexing": return "Indexing in progress (X/Y files). Please wait..."
       └── If None: query DB normally
```

## Risks

- **Low risk:** The `WorkspaceState` inserted early has default values for fields that are populated later (`_watcher`, LSP not started, `active_session` = None). Since all these fields are behind `Mutex`/`Arc`, they can be updated in-place after the workspace is already in the HashMap.
- **Low risk:** `scan_workspace` takes `&IndexDb` — the DB is behind `Arc<IndexDb>` in `WorkspaceState`. The indexer borrows the same DB. SQLite WAL mode ensures concurrent reads (from tools) don't conflict with writes (from indexer). Tools open their own connections via `IndexDb::open()`, so no lock contention on the indexer's connection mutex.
- **Edge case:** If `open_workspace` fails AFTER inserting into the HashMap (e.g., model download fails), the workspace remains in the HashMap with `index_progress` stuck at "indexing". Tools would keep reporting "indexing in progress" forever. Mitigation: set `index_progress = None` in error paths.

## Non-goals

- Reindexing flow (file watcher triggers) — already works, no changes needed
- Frontend UI changes (progress bar already shows correctly)
- Embedding phase gating — already handled by `semantic_search`'s 5-second polling
- `close_workspace` during indexing — edge case, not addressed

## Low-Level Design

### Files to modify:

1. **`src-tauri/src/state.rs`** — Add `index_progress` field to `WorkspaceState`
2. **`src-tauri/src/commands/code_intel.rs`** — Move workspace insertion earlier, wire up progress tracking
3. **`src-tauri/src/code_intel/indexer.rs`** — Accept optional shared progress in `scan_workspace`
4. **`src-tauri/src/agent/tools/mod.rs`** — Add `index_progress` to `ToolContext`, add gate check

### Change 1: `WorkspaceState` — add `index_progress` field

**File:** `src-tauri/src/state.rs`

Add import:
```rust
use crate::code_intel::indexer::IndexProgress;
```

Add field to `WorkspaceState` struct:
```rust
/// Tracks indexing progress so tools can report status during initial scan.
/// `Some(progress)` = indexing in progress; `None` = indexing complete.
pub index_progress: Arc<Mutex<Option<IndexProgress>>>,
```

### Change 2: `open_workspace` — insert workspace into HashMap early

**File:** `src-tauri/src/commands/code_intel.rs`

**Step 2a:** After DB init (line ~76), build and insert a full `WorkspaceState`:

```rust
let db = Arc::new(IndexDb::open(&db_path)?);

// --- NEW: Build workspace state early so send_message works during indexing ---
let root = std::path::PathBuf::from(&path);
let index_progress: Arc<Mutex<Option<IndexProgress>>> = Arc::new(Mutex::new(Some(IndexProgress {
    status: "indexing".into(),
    files_indexed: 0,
    symbols_indexed: 0,
    total_files: 0,
    workspace: path.clone(),
})));
let lsp_manager = Arc::new(tokio::sync::Mutex::new(crate::lsp::manager::LspManager::new()));
let workspace = Arc::new(WorkspaceState {
    root: root.clone(),
    index_db: db.clone(),
    skills_manager: Arc::new(tokio::sync::Mutex::new(
        crate::agent::skills::SkillManager::new(Some(root.clone())),
    )),
    lsp_manager: lsp_manager.clone(),
    _watcher: tokio::sync::Mutex::new(None),
    watcher_warning: tokio::sync::Mutex::new(None),
    active_session: tokio::sync::Mutex::new(None),
    mcp: tokio::sync::Mutex::new(None),
    mcp_fingerprint: tokio::sync::Mutex::new(None),
    index_progress: index_progress.clone(),
});
{
    let mut map = state.workspaces.lock().await;
    map.insert(root.clone(), workspace);
}
// --- END NEW ---
```

**Step 2b:** Pass `index_progress` clone to `scan_workspace`:

```rust
let scan_handle = spawn_blocking({
    let db = Arc::clone(&db);
    let path = path.clone();
    let app_handle = app_handle.clone();
    let progress_channel = progress_channel.clone();
    let shared_progress = index_progress.clone();  // NEW
    move || {
        indexer::scan_workspace(
            db.as_ref(),
            &path,
            Some(&app_handle),
            None,
            Some(&progress_channel),
            Some(&shared_progress),  // NEW parameter
        )
    }
});
```

**Step 2c:** After scan completes, clear progress:

```rust
let (files_count, symbols_count) = scan_handle.await...;

// Mark indexing as complete
if let Ok(mut progress) = index_progress.lock() {
    *progress = None;
}
```

**Step 2d:** Remove the duplicate `WorkspaceState` construction and `map.insert` at lines 220-258 (the original location). The workspace is already inserted. Later steps that update fields (LSP start, watcher setup) now update through the already-inserted `Arc`:

```rust
// LSP: start on the already-existing manager
{
    let mut lsp = lsp_manager.lock().await;
    let _ = lsp.start_for_workspace(&path);
}

// Watcher: set in the existing Mutex
{
    let mut watcher_lock = workspace._watcher.lock().await;
    *watcher_lock = watcher;
}
// Note: need to get a reference to the workspace. Since it's in the HashMap as Arc,
// we can clone it back out:
let workspace = state.workspace(&path).await.unwrap();
```

**Step 2e:** Error path cleanup — if anything fails after workspace insertion, set `index_progress = None`:

In every `?` error return after the insertion point, add:
```rust
if let Ok(mut p) = index_progress.lock() {
    *p = None;
}
```

### Change 3: `scan_workspace` — accept shared progress

**File:** `src-tauri/src/code_intel/indexer.rs`

Add parameter to function signature:
```rust
pub fn scan_workspace(
    db: &IndexDb,
    root: &str,
    app_handle: Option<&tauri::AppHandle>,
    embedder: Option<&SharedEmbedder>,
    progress_channel: Option<&Channel<IndexProgress>>,
    shared_progress: Option<&Mutex<Option<IndexProgress>>>,  // NEW
) -> Result<(i64, i64), String> {
```

In the progress emission loop (currently at ~line 370, where `if counted % 10 == 0`), add:
```rust
if let Some(sp) = shared_progress {
    if let Ok(mut guard) = sp.lock() {
        *guard = Some(prog.clone());
    }
}
```

The `Mutex` import is already available via `std::sync::Mutex` (used elsewhere in the file).

### Change 4: `ToolContext` — add `index_progress` field

**File:** `src-tauri/src/agent/tools/mod.rs`

Add field to `ToolContext` struct:
```rust
/// Current indexing progress — tools check this to see if the workspace index is ready.
pub index_progress: Option<Arc<Mutex<Option<crate::code_intel::indexer::IndexProgress>>>>,
```

### Change 5: `send_message` — pass `index_progress` to `ToolContext`

**File:** `src-tauri/src/commands/agent.rs`

In the `ToolContext` construction (~line 267), add:
```rust
let ctx = ToolContext {
    db_path,
    lsp_manager: Some(ws.lsp_manager.clone()),
    workspace_root,
    embedding_model: state.embedding_model.clone(),
    session_store_path: Some(handle.store_path.to_string_lossy().to_string()),
    read_tracker: Arc::new(Mutex::new(ReadTracker::default())),
    interrupt: Some(steering.interrupt.clone()),
    agent_config: Some(config.clone()),
    plan_save_path: config.plan_save_path.clone(),
    base_commit,
    auto_approve_git: false,
    mcp: Some(mcp),
    mode_ctl: Some(mode_ctl.clone()),
    index_progress: Some(ws.index_progress.clone()),  // NEW
};
```

### Change 6: Add gate check for index-dependent tools

**File:** `src-tauri/src/agent/tools/mod.rs`

Add helper function:
```rust
/// Check whether the workspace index is ready. Returns an error with a
/// human-readable progress message if indexing is still in progress.
fn check_index_ready(ctx: &ToolContext) -> Result<(), String> {
    if let Some(ref progress) = ctx.index_progress {
        let guard = progress.lock().map_err(|e| format!("index progress lock: {e}"))?;
        if let Some(ref prog) = *guard {
            if prog.status == "indexing" && prog.total_files > 0 {
                return Err(format!(
                    "Workspace index is still being built: {}/{} files indexed ({} symbols). \
                     This tool requires the index to be complete. Please wait a moment and try again, \
                     or use tools that don't depend on the index (read_file, list_dir, grep, bash).",
                    prog.files_indexed, prog.total_files, prog.symbols_indexed
                ));
            }
        }
    }
    Ok(())
}
```

Add `check_index_ready(&ctx)?;` at the start of these tool handlers:
- `"code_search"` (line ~506)
- `"symbol_lookup"` (line ~513)
- `"file_outline"` (line ~519)
- `"go_to_definition"` (line ~525)
- `"find_references"` (line ~543)
- `"semantic_search"` (line ~561)

### Change 7: Handle `index_progress` in `compact_session` and other commands

Commands that access `state.workspace(path)` will now succeed during indexing. But `compact_session`, `new_session`, `list_sessions`, `load_session`, `set_session_mode`, `get_session_mode`, `close_workspace` — these don't need the index and should work fine.

`search_symbols`, `symbol_lookup`, `file_outline` commands (the non-agent versions used by the frontend) — these also call `state.workspace(path)` and will now succeed during indexing. They query the DB directly and return whatever is there (partial results). This is acceptable — the frontend will show partial @-mention autocomplete results, which is better than nothing.

No changes needed for these commands — they work correctly with partial data.

## Tasks summary

1. Add `index_progress` field to `WorkspaceState` in `state.rs`
2. Move workspace insertion earlier in `open_workspace` (after DB init, before scan)
3. Add `shared_progress` parameter to `scan_workspace` in `indexer.rs`
4. Add `index_progress` field to `ToolContext` in `tools/mod.rs`
5. Wire `index_progress` from workspace to `ToolContext` in `commands/agent.rs`
6. Add `check_index_ready` gate to all 6 index-dependent tools
7. Build and verify the changes compile


## Implementation Log — 2026-07-15 18:29
**Summary:** Allow chat messages during indexing — workspace inserted early, index-dependent tools gate with progress message
**Changed files:** M src-tauri/examples/semantic_eval.rs, M src-tauri/src/agent/tools/bash.rs, M src-tauri/src/agent/tools/finalize_plan.rs, M src-tauri/src/agent/tools/mod.rs, M src-tauri/src/agent/tools/tasks.rs, M src-tauri/src/agent/tools/write_plan.rs, M src-tauri/src/code_intel/indexer.rs, M src-tauri/src/commands/agent.rs, M src-tauri/src/commands/code_intel.rs, M src-tauri/src/state.rs, ?? docs/plans/2026-07-15_allow-messages-during-indexing.md
**Commits:** _(git unavailable or none)_
**Journal:** ## Implementation Summary

**Problem:** Messages sent during workspace indexing (the scan_workspace phase) failed with `"workspace not open"` because the workspace was only inserted into the state HashMap AFTER the scan completed. The ChatPanel was visible and enabled, but every send attempt errored.

**Solution:** Move the workspace insertion to right after DB initialization (before scan), with an `index_progress` shared state. Index-dependent tools check this progress and return a human-readable message if indexing is still in progress; non-index tools (read, list, grep, bash) work normally.

### Key Decisions & Gotchas:

1. **std::sync::Mutex vs tokio::sync::Mutex**: The `index_progress` field uses `std::sync::Mutex` instead of tokio's because the `scan_workspace` function runs inside `spawn_blocking` (blocking thread pool), where tokio mutexes can't be used. This is safe because the lock is only held briefly to read/write an `Option<IndexProgress>`.

2. **go_to_definition and find_references skip the gate**: These tools use LSP (which starts independently of the index) as the primary source, with DB as a fallback. Gating them would block valid LSP-based results, so they're not gated. The heuristic fallback silently returns empty results if the DB isn't populated — acceptable.

3. **Watcher and LSP are set retroactively**: Since the workspace is inserted early with `_watcher: None`, the watcher and LSP are configured after scan completes by mutating fields through the Arc handle (`ws._watcher.lock().await = watcher`). This works because all fields are behind Mutexes.

4. **Overwrite guard for workspace tools**: The frontend's `search_symbols`, `symbol_lookup`, `file_outline` commands (used for @-mention autocomplete) now succeed during indexing but return partial data. This is acceptable — partial results are better than no results.

5. **Reopening a workspace**: The early-return path (already open) is unchanged — it returns immediately before the early insertion logic. No risk of double-insertion.

### Files Changed:
- `src-tauri/src/state.rs` — added `index_progress` field
- `src-tauri/src/commands/code_intel.rs` — moved workspace insertion earlier
- `src-tauri/src/code_intel/indexer.rs` — added `shared_progress` parameter
- `src-tauri/src/agent/tools/mod.rs` — added `check_index_ready` gate + ToolContext field
- `src-tauri/src/commands/agent.rs` — wired index_progress into ToolContext
- `src-tauri/src/agent/tools/tasks.rs`, `bash.rs`, `finalize_plan.rs`, `write_plan.rs` — test context updates
- `examples/semantic_eval.rs` — added None arg to scan_workspace call

**Task journal:**
- Add index_progress field to WorkspaceState: Added IndexProgress import and index_progress field to WorkspaceState
- Move workspace insertion to before scan_workspace in open_workspace: Moved workspace insertion to right after DB init. Early workspace built with index_progress tracking, LSP and watcher set via handles later.
- Add shared_progress parameter to scan_workspace: Added shared_progress parameter at 4 emission points (initial, per-10-files, grand total, done)
- Add index_progress field to ToolContext: Added import and field to ToolContext struct. Used std::sync::Mutex for compatibility with blocking indexer code.
- Wire index_progress into ToolContext in send_message: Added index_progress: Some(ws.index_progress.clone()) to ToolContext construction in send_message, compact_session, and commit_and_push
- Add check_index_ready gate to index-dependent tools: Added check_index_ready helper that returns descriptive error with X/Y progress. Applied to code_search, symbol_lookup, file_outline, semantic_search. Skipped go_to_definition and find_references since LSP is index-independent.
- Build and verify compilation: Build succeeds. All 225 tests pass. Pre-existing clippy warning in bash.rs (unrelated to this change).
