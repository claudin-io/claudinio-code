# Fix: Compression before/after values + Read file bubble filename

## Context
Two UI bugs: (1) compression status shows identical before/after token values, (2) Read file chat bubble is missing the file path.

## Solution Design
**Fix 1 (session.rs)**: In the per-round compaction re-check trigger, `history` is rebuilt from compacted records *before* computing `__compact_done__` args, so both args are computed on the same post-compaction history. Save pre-compaction estimate into a local variable before calling `compact_history()`.

**Fix 2 (toolPresentation.ts)**: `toolSummary()` reads only `args.path`. The Rust backend accepts both `path` and `file_path` (serde alias). If the LLM sends `file_path`, the frontend gets `args.path = undefined`. Add fallback: `args.path ?? args.file_path ?? ""`.

## Risks
Low risk — one-line changes, no API contract changes.

## Non-goals
Not changing token estimation algorithm, not changing backend serialization, not updating TypeScript type definitions.

## Low-Level Design

**Change 1 — session.rs lines 1270-1317**: Add `let pre_tokens = estimate_tokens(history, &system, &tools);` before the `compact_history` call, use it as `args[0]` in `__compact_done__` instead of recomputing `estimate_tokens` on the already-rebuilt history.

**Change 2 — toolPresentation.ts line 97**: `String(args.path ?? "")` becomes `String(args.path ?? args.file_path ?? "")`.

**Verification**: cargo check + tsc. First trigger point (lines 1160-1163) already computes correctly — no regression.


## Implementation Log — 2026-07-14 12:17
**Summary:** Fix compression before/after values (session.rs) and Read file bubble missing filename (toolPresentation.ts)
**Changed files:** M src-tauri/src/agent/session.rs, M src/components/tool-renderers/toolPresentation.ts, ?? docs/plans/2026-07-14_fix-compression-values-and-read-bubble.md
**Commits:** _(git unavailable or none)_
**Journal:** Two small but impactful UI bugs fixed:

**Bug 1 — Compression showing identical before/after values**
Root cause: In the per-round compaction re-check trigger (session.rs ~line 1314), the `__compact_done__` event's `args[0]` (before value) was computed by calling `estimate_tokens()` AFTER `history` had already been rebuilt from compacted records. This meant both args were the same post-compaction value. Fix: saved `pre_tokens` into a variable before the `compact_history()` call, and used it as `args[0]`. The first trigger (after user message) already had this pattern with `estimated` — so we just replicated it for the second trigger.

**Bug 2 — "Read file" bubble missing the file path**
Root cause: The frontend `toolSummary()` only read `args.path`, but the Rust backend's `ReadFileArgs` uses `#[serde(alias = "file_path")]` and forwards the LLM's raw args to the frontend. Some LLMs emit `file_path` instead of `path`, causing `args.path = undefined` and an empty summary. Fix: added fallback `args.path ?? args.file_path ?? ""` matching the backend's alias flexibility.

Both fixes are one-line changes with no API contract or data flow changes. Verified: cargo check passes, tsc --noEmit has only pre-existing test errors.

**Task journal:**
- Fix compression before/after values in session.rs: Added `let pre_tokens = estimate_tokens(...);` before the threshold check.; Replaced `estimate_tokens(history, ...)/1000` with `pre_tokens/1000` in both `__compact_start__:` and `__compact_done__:` events.; The `__compact_done__` now correctly shows pre-compaction vs post-compaction (new_ctx) values instead of two identical values.
- Fix Read file bubble missing filename in toolPresentation.ts: Changed `String(args.path ?? "")` to `String(args.path ?? args.file_path ?? "")` — now handles both `path` and `file_path` keys that the LLM may emit.; This matches the Rust backend's `#[serde(alias = "file_path")]` flexibility.
- Verify both fixes compile and have no regressions: cargo check: ✅ passed clean.; tsc --noEmit: only pre-existing errors in test files (ContentViewerModal.test.tsx, subagentTimeline.test.ts) — none related to our change.; First trigger (lines ~1160-1163) confirmed: still uses `estimated / 1000` as before value — no regression.
