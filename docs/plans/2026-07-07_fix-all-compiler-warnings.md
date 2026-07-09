# Solution Design: Fix All Compiler Warnings

## Context

The `cargo build --release` produces 16 warnings across these categories:

### Trivial (5 warnings) — single-line fixes:
1. **Unused import** `std::path::Path` in `code_intel/indexer.rs:4` — `Path` type never explicitly used in this file (all paths are `&str`)
2. **Duplicate pattern** `"toml"` in `commands/agent.rs:192` and `:195` — same string appears twice in a `matches!` alternation
3. **Unnecessary `mut`** on `message` in `commands/agent.rs:164` — the variable is never mutated (it shadows an immutable binding)
4. **Unused variable `name`** in `agent/provider.rs:450` — assigned from `block.get("name")` but never read
5. **Unused variable `session_id`** in `agent/provider.rs:523` — parameter never referenced in `process_line()` body

### Dead code (11 warnings) — confirmed zero callers across entire codebase:

**A. Session compaction helpers** (`persist.rs`):
- `last_compacted_summary` ~L295 — 0 callers
- `records_since_compacted` ~L304 — 0 callers (only used by tests)
- `records_before_compacted` ~L316 — 0 callers (only used by tests)
- `compact_boundary` ~L324 — only called by the 3 dead functions above
- NOTE: `SessionRecord::Compacted` variant is **NOT dead** — used by `session.rs:112`, `persist.rs:183,189`, `persist.rs:425`, and the frontend. Keep it.

**B. `ContentBlock::get_text`** (`provider.rs:211`) — 0 callers

**C. Skills dead methods** (`skills.rs`):
- `reload_body` ~L329 — 0 callers
- `len` ~L349 — 0 callers
- `frontmatter_cache` field ~L191 — never read outside `scan()` which populates it
- `REMOTE_REGISTRY_BASE` constant ~L357 — dead, `REMOTE_INDEX_URL` is the one actually used

**D. LSP client dead methods** (`lsp/client.rs`):
- `did_open` ~L178 — only called from dead `LspManager::did_open`
- `did_change` ~L192 — only called from dead `LspManager::did_change`
- `did_close` ~L202 — only called from dead `LspManager::did_close`
- `shutdown` ~L260 — 0 callers (Drop impl handles cleanup)

**E. LSP manager dead code** (`lsp/manager.rs`):
- `detect_language` ~L79 — 0 callers
- `lsp_key` ~L91 — only called by dead methods
- `get_uri` ~L100 — only called by dead methods
- `get_language_id` ~L104 — 0 callers
- `with_server` ~L118 — 0 callers
- `did_open` ~L127 — 0 callers
- `did_change` ~L141 — 0 callers
- `did_close` ~L154 — 0 callers
- `language` field in `LspServerInstance` ~L14 — set but never read
- `use std::path::Path` import ~L3 — becomes dead after removing all methods that use it

## Solution Design

### Step 1: Fix trivial warnings (5 files, ~5 lines changed)
- Remove `use std::path::Path;` from `indexer.rs:4`
- Remove duplicate `"toml"` from `agent.rs:196`
- Remove `mut` from `let mut message` on `agent.rs:164`
- Prefix `name` with underscore: `let _name =` in `provider.rs:450`
- Prefix `session_id` with underscore: `_session_id` in `provider.rs:523`

### Step 2: Remove `get_text` from `ContentBlock` (`provider.rs`)
- Delete lines ~211-216 (the entire `get_text` method)

### Step 3: Remove dead skills code (`skills.rs`)
- Delete `frontmatter_cache` field from `SkillManager` struct
- Remove the field from `new()` constructor initialization
- Delete `reload_body` method
- Delete `len` method
- Delete `REMOTE_REGISTRY_BASE` constant

### Step 4: Remove dead LSP client methods (`lsp/client.rs`)
- Delete `did_open` method
- Delete `did_change` method
- Delete `did_close` method
- Delete `shutdown` method

### Step 5: Remove dead LSP manager code (`lsp/manager.rs`)
- Remove `use std::path::Path;` import (becomes unused after deletions)
- Remove `language: String,` field from `LspServerInstance`
- Update `start_tsserver` construction to not set `language`
- Update `start_rust_analyzer` construction to not set `language`
- Delete `detect_language` method
- Delete `lsp_key` method (only used by now-dead methods)
- Delete `get_uri` method (only used by now-dead methods)
- Delete `get_language_id` method
- Delete `with_server` method
- Delete `did_open` method
- Delete `did_change` method
- Delete `did_close` method

### Step 6: Remove dead persist functions + their tests (`persist.rs`)
- Delete `last_compacted_summary` function
- Delete `records_since_compacted` function
- Delete `records_before_compacted` function
- Delete `compact_boundary` function
- Delete related test cases: `records_before_and_since_respect_tail`, `records_after_and_before_compacted`, `records_after_and_before_without_compacted`
- Also delete `records_before_and_since_compacted_basic` if it exists — determine after reading full test module

### Verification
- `cargo build --release` produces **0 warnings**
- All existing tests pass: `cargo test --lib`
- `cargo check` also clean

## Risks

- **Low risk**: All dead code has been confirmed with zero callers via codebase-wide search. The `Compacted` variant of `SessionRecord` is explicitly preserved (it's actively used).
- **Test cleanup**: Tests that assert dead functions will also be removed. No other tests should be affected.
- **No functional change**: All removals are dead code — the application behavior is unchanged.

## Tasks Summary

| # | Task | Files |
|---|------|-------|
| T1 | Fix trivial warnings (import, dup pattern, mut, unused vars) | `indexer.rs`, `agent.rs`, `provider.rs` |
| T2 | Remove `get_text` from ContentBlock | `provider.rs` |
| T3 | Remove dead skills code (frontmatter_cache, reload_body, len, REMOTE_REGISTRY_BASE) | `skills.rs` |
| T4 | Remove dead LSP client methods (did_open, did_change, did_close, shutdown) | `lsp/client.rs` |
| T5 | Remove dead LSP manager code (8 methods + language field + import) | `lsp/manager.rs` |
| T6 | Remove dead persist functions + their tests | `persist.rs` |
| T7 | Verify: `cargo build --release` and `cargo test --lib` produce 0 warnings | All |
