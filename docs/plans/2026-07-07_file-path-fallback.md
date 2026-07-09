# Plan: `file_path` ↔ `path` fallback for tool argument deserialization

## 1. Context / Problem Statement

Four file-operation tools (`read_file`, `edit_file`, `list_dir`, `grep`) use serde structs with a `path` field. Three LSP-related tools (`file_outline`, `go_to_definition`, `find_references`) parse arguments ad-hoc and expect `file_path`. The JSON schemas exposed to the LLM are consistent per-tool, but some LLM fine-tunes confuse the two naming conventions and send the wrong key.

**Concrete symptom:** When the LLM sends `{"file_path": "/path/to/file"}` to `read_file`, `edit_file`, or `list_dir`, serde rejects it with:

```
invalid args: missing field `path`
```

The same can happen in reverse if the LLM sends `{"path": "..."}` to `file_outline`/`go_to_definition`/`find_references`.

**CONFIRMED by user:** Bidirectional fallback — each tool should accept both `path` and `file_path`. When both are present, the tool's canonical field takes precedence.

## 2. Goal (Definition of Done)

Every tool that accepts a file path argument (`read_file`, `edit_file`, `list_dir`, `grep`, `file_outline`, `go_to_definition`, `find_references`) accepts **both** `path` and `file_path` in the incoming JSON. The behavior is identical regardless of which key the LLM sends.

## 3. Key Findings (Prova Real)

| Finding | Method | Proof |
|---------|--------|-------|
| `ReadFileArgs.path` is the field (line 9) | `cat -n src-tauri/src/agent/tools/read_file.rs` | `pub path: String` at line 9 |
| `EditFileArgs.path` is the field (line 6) | `cat -n src-tauri/src/agent/tools/edit_file.rs` | `pub path: String` at line 6 |
| `ListDirArgs.path` is the field (line 6) | `cat -n src-tauri/src/agent/tools/list_dir.rs` | `pub path: String` at line 6 |
| `GrepArgs.path` is the field (line 8) | `cat -n src-tauri/src/agent/tools/grep.rs` | `pub path: Option<String>` at line 8 |
| `file_outline` reads `file_path` inline (line 434) | `cat -n src-tauri/src/agent/tools/mod.rs` | `args.get("file_path").and_then(...)` at line 434 |
| `go_to_definition` reads `file_path` inline (line 440) | same source | `args.get("file_path")` at line 440 |
| `find_references` reads `file_path` inline (line 458) | same source | `args.get("file_path")` at line 458 |
| JSON schemas are consistent internally (4 tools = `path`, 3 tools = `file_path`) | `get_defs()` in mod.rs lines 79-319 | `"path"` in schemas for read_file/list_dir/grep/edit_file; `"file_path"` for file_outline/go_to_definition/find_references |
| No fallback exists anywhere | Grepped entire codebase | No normalization/pre-processing step exists |

## 4. Changes (Steps)

### 4.1 — `src-tauri/src/agent/tools/read_file.rs`

**Target:** `ReadFileArgs` struct, line 9.

**Mutation:** Add `#[serde(alias = "file_path")]` to the `path` field:

```rust
#[derive(Deserialize)]
pub struct ReadFileArgs {
    #[serde(alias = "file_path")]
    pub path: String,
    pub start_line: Option<usize>,
    pub end_line: Option<usize>,
}
```

**Why:** serde's `alias` attribute lets the struct accept both `path` (primary) and `file_path` (fallback) with zero runtime overhead and no extra code.

**Constraints:** No other logic changes. `serde` is already imported.

### 4.2 — `src-tauri/src/agent/tools/edit_file.rs`

**Target:** `EditFileArgs` struct, line 6.

**Mutation:** Add `#[serde(alias = "file_path")]`:

```rust
#[derive(Deserialize)]
pub struct EditFileArgs {
    #[serde(alias = "file_path")]
    pub path: String,
    pub old_string: String,
    pub new_string: String,
}
```

### 4.3 — `src-tauri/src/agent/tools/list_dir.rs`

**Target:** `ListDirArgs` struct, line 6.

**Mutation:** Add `#[serde(alias = "file_path")]`:

```rust
#[derive(Deserialize)]
pub struct ListDirArgs {
    #[serde(alias = "file_path")]
    pub path: String,
}
```

### 4.4 — `src-tauri/src/agent/tools/grep.rs`

**Target:** `GrepArgs` struct, line 8.

**Mutation:** Add `#[serde(alias = "file_path")]`:

```rust
#[derive(Deserialize)]
pub struct GrepArgs {
    pub pattern: String,
    #[serde(alias = "file_path")]
    pub path: Option<String>,
}
```

**Note:** `path` is `Option<String>` here — the alias works identically for `Option` fields.

### 4.5 — `src-tauri/src/agent/tools/mod.rs` — Inline tools (reverse direction)

**Target:** The `execute()` function, three inline parsers:

- Line 434 (`file_outline`): `args.get("file_path")`
- Line 440 (`go_to_definition`): `args.get("file_path")`
- Line 458 (`find_references`): `args.get("file_path")`

**Mutation:** Replace each `args.get("file_path")` with a helper that tries `file_path` first, then falls back to `path`. Simplest approach: a local closure or inline `or_else`:

```rust
// Helper: accept both "file_path" and "path" as fallback
let fp = args.get("file_path")
    .or_else(|| args.get("path"))
    .and_then(|v| v.as_str())
    .ok_or("missing file_path")?;
```

Apply this pattern to all three: `file_outline` (line 434), `go_to_definition` (line 440), `find_references` (line 458).

**Why:** These three tools do NOT use serde structs — they parse `serde_json::Value` ad-hoc. So they need an explicit fallback, not a serde alias.

### 4.6 — Unit tests

**Target:** `mod.rs` test module (lines 733+).

**Mutation:** Add two tests:
1. **`test_read_file_accepts_file_path_fallback`** — sends `{"file_path": "..."}` to `read_file` and asserts success.
2. **`test_file_outline_accepts_path_fallback`** — sends `{"path": "..."}` to `file_outline` (via execute) and asserts it doesn't fail with "missing file_path".

**Constraints:** Must create a temporary file so the tool has something to read/outline.

### What does NOT change

- JSON schemas in `get_defs()` — these remain unchanged. The primary/contractual field name stays as-is; the fallback is a server-side tolerance, not a schema change.
- `write_plan`, `bash`, `tasks`, `web_search`, `code_search`, `symbol_lookup`, `semantic_search` — no path field at all, no change needed.
- `LspPositionArgs.file_path` in `src-tauri/src/commands/lsp.rs` — this is a Tauri command, not an agent tool; out of scope.

## 5. Verification Plan

### 5.1 — Compile check

```bash
cd src-tauri && cargo check 2>&1
```

Expected: zero errors. The `alias` attribute is a standard serde feature.

### 5.2 — serde alias: read_file accepts `file_path`

```bash
cd src-tauri && cargo test test_read_file_accepts_file_path_fallback -- --nocapture
```

Expected: test passes. Sends `{"file_path": "..."}` and asserts `read_file` succeeds.

### 5.3 — ad-hoc fallback: file_outline accepts `path`

```bash
cd src-tauri && cargo test test_file_outline_accepts_path_fallback -- --nocapture
```

Expected: test passes.

### 5.4 — Existing tests unchanged (regression)

```bash
cd src-tauri && cargo test --lib agent::tools
```

Expected: all existing tests pass with zero failures.

### 5.5 — Manual smoke test (real LLM interaction)

Ask the LLM to `read_file` — the LLM naturally mixes conventions; confirm no more `missing field 'path'` errors in real usage.

## 6. Risks

| Risk | Likelihood | Mitigation |
|------|-----------|------------|
| Both `path` and `file_path` sent with different values | Low (LLMs send one or the other, not both) | serde prioritizes the struct field name (`path`); `file_path` is ignored when `path` is present. For inline parsers, `file_path` is tried first — same behavior: the canonical key wins. |
| JSON schema changes cascade into LLM confusion | N/A | No schema changes; schemas remain clean with one canonical field each. |
| Performance impact | Zero | `alias` is compile-time; `or_else` is one extra pointer chase, negligible. |

## 7. Tasks Summary

1. Add `#[serde(alias = "file_path")]` to `ReadFileArgs.path`
2. Add `#[serde(alias = "file_path")]` to `EditFileArgs.path`
3. Add `#[serde(alias = "file_path")]` to `ListDirArgs.path`
4. Add `#[serde(alias = "file_path")]` to `GrepArgs.path`
5. Add `path` fallback in `file_outline` inline parser
6. Add `path` fallback in `go_to_definition` inline parser
7. Add `path` fallback in `find_references` inline parser
8. Add unit tests for both directions
9. Run full test suite to verify no regressions
