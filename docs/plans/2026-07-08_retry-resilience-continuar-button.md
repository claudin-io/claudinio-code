# Solution Design: Retry Resilience with "Continuar" Button

## Context / Problem Statement

The session `239e5d3d` hit an `API error: HTTP 502 Bad Gateway` from the LLM provider right after a `git commit`. The app has a retry mechanism in `stream_message_with_retry`, but:

1. **Backoff is wrong**: Was changed to `[60s, 120s, 180s, 300s]` — too aggressive (starts at 1 minute). Should be `[2s, 5s, 15s, 30s, 60s, 120s, 180s, 300s]` — 8 tiers, starting with fast retries for transient hiccups, scaling to 5 minutes for serious outages.
2. **Error leaks to chat during retry**: `provider.rs:369` sends `AgentEvent::Error` IMMEDIATELY on a non-200 response, even though `stream_message_with_retry` will retry it. The user sees an error message, then a few seconds later the retry succeeds — confusing.
3. **No "Continuar" button after exhaustion**: When all retries fail, the chat ends in an "error" state with no way to resume. The user must re-type their message.

**What the user CONFIRMED:**
- 8-tier backoff: 2s, 5s, 15s, 30s, 60s, 120s, 180s, 300s
- Silent during retries — no error in chat
- After exhaustion: error + "Continuar" button that sends `[system] continue from where you stopped`
- The "Continuar" approach uses existing `sendMessage` mechanism (session resumes from JSONL history)

## Goal (Definition of Done)

1. On transient API errors (502, 429, stream errors), the retry engine silently backoffs through 8 tiers.
2. Only after ALL 8 retries fail does an error appear in chat — with a clickable "Continuar" button.
3. "Continuar" sends a silent system message and resumes the session exactly where it stopped.
4. Build passes (Rust + Vite), existing tests pass.

## Key Findings (Prova Real)

- **`session.rs:818`** — `is_retryable_error` already correctly identifies 429 and 5xx codes → no change needed
- **`session.rs:842`** — `BACKOFFS_MS` manually changed to `[60_000, 120_000, 180_000, 300_000]` during the session → needs to be `[2_000, 5_000, 15_000, 30_000, 60_000, 120_000, 180_000, 300_000]`
- **`provider.rs:369`** — `AgentEvent::Error(err_msg.clone())` is sent on every non-200 status → must be removed so errors don't leak to chat during retry
- **`commands/agent.rs:291`** — `AgentEvent::Error(e)` fires only after `run_workflow` returns `Err` → this is correct, fires after ALL retries exhausted
- **`ChatPanel.tsx:1071-1076`** — On `Error` event, appends error text to messages and sets status to `"error"` → needs to show "Continuar" button instead of plain text
- **`ChatPanel.tsx` send function** — `sendMessage` already loads active session from JSONL, so sending `[system] continue from where you stopped` will naturally resume the session

## Changes (Steps)

### Step 1: Remove premature error event in `provider.rs`
- **Target**: `src-tauri/src/agent/provider.rs:369`
- **Mutation**: Delete the line `let _ = event_tx.send(AgentEvent::Error(err_msg.clone()));`
- **Why**: This error fires on EVERY non-200 response, immediately leaking to chat even though the retry loop in `session.rs` will attempt again. The error should only surface after the retry loop fully exhausts.
- **Constraints**: Keep the `return Err(err_msg)` — the error string still needs to propagate to the retry loop.

### Step 2: Fix backoff sequence in `session.rs`
- **Target**: `src-tauri/src/agent/session.rs:842`
- **Mutation**: Change `BACKOFFS_MS` from `[60_000, 120_000, 180_000, 300_000]` to `[2_000, 5_000, 15_000, 30_000, 60_000, 120_000, 180_000, 300_000]`
- **Why**: Start with fast retries (2s) for transient hiccups, scale gradually to 5min for serious outages. Total retry window: ~12.5 minutes.
- **Constraints**: Keep the existing loop logic unchanged — it already correctly counts attempts and checks `is_retryable_error`.

### Step 3: Add "Continuar" UI in `ChatPanel.tsx`
- **Target**: `src/components/ChatPanel.tsx:1071-1076` (the `Error` event handler)
- **Mutation 3a**: Instead of appending error text to messages, set a new signal `retryableError` with the error message.
- **Mutation 3b**: In the JSX, when `retryableError` is set, render an error bar below the input area with the error text and a "Continuar" button.
- **Mutation 3c**: "Continuar" handler calls `sendMessage(workspace, "[system] continue from where you stopped", [], handleEvent, mode)` to resume the session.
- **Mutation 3d**: Also handle the case where `status` changes to `"done"` — clear `retryableError`.
- **Why**: Gives the user a one-click way to resume after a transient outage without re-typing their message.

### Step 4: Add `retryableError` signal declaration
- **Target**: `src/components/ChatPanel.tsx` (near the other signals, ~line 100)
- **Mutation**: Add `const [retryableError, setRetryableError] = createSignal<string | null>(null);`
- **Why**: Need a signal to track whether to show the "Continuar" bar.

### Step 5: Wire up `"done"` event to clear retryable error
- **Target**: `src/components/ChatPanel.tsx` (in `handleEvent`, near the `"Done"` handler)
- **Mutation**: Add `setRetryableError(null);` when a `"Done"` or `"TextStep"` event fires (any progress means the retry succeeded).
- **Why**: If a retry succeeds after some attempts, the error bar should disappear.

## Verification Plan

1. **Rust build**: `cargo build --manifest-path src-tauri/Cargo.toml` — must pass
2. **Frontend build**: `npx vite build` — must pass
3. **Existing tests**: `npx vitest run` — all tests must pass
4. **Manual test scenario** (for user): Trigger a 502 by temporarily using a bad endpoint, verify:
   - No error appears in chat during retries
   - After ~12min, error bar with "Continuar" appears
   - Clicking "Continuar" resumes the session
5. **Commit**: New branch, atomic commit with all changes

## Risks

- **Subagents** (`subagent.rs:294`) call `provider::stream_message` directly without going through `stream_message_with_retry` — they get no retry benefit from this change. Excluded from scope (would require separate work).
- The "Continuar" button sends `[system]` message which lands in the JSONL history — this is intentional (it's a legitimate conversation turn) but could cause slight confusion if the user inspects the session file.

## Tasks Summary

| # | Task | Files |
|---|------|-------|
| 1 | Remove premature `AgentEvent::Error` in `provider.rs` | `src-tauri/src/agent/provider.rs` |
| 2 | Fix `BACKOFFS_MS` to 8-tier sequence in `session.rs` | `src-tauri/src/agent/session.rs` |
| 3 | Add `retryableError` signal + "Continuar" error bar UI in `ChatPanel.tsx` | `src/components/ChatPanel.tsx` |
| 4 | Build & verify | Rust + Vite + vitest |
