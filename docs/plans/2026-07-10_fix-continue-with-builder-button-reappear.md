# Fix: "Continue with Builder" Button Not Appearing After Re-entering Brain Mode

## Context / Problem Statement

When a user follows this flow:
1. User clicks Brain toggle → Brain runs → `write_plan` is called → "Continue with Builder" button appears ✅
2. User clicks "Continue with Builder" → switches to Builder → Builder finishes
3. User clicks Brain toggle again → THE BUTTON DOES NOT APPEAR until the user sends a new message and the Brain session runs again and calls `write_plan`

**Root cause:** The frontend signal `hasPlanBeenWritten` (ChatPanel.tsx:482) is set to `true` ONLY when a `write_plan` tool result event fires during a Brain session (line 1055). It is unconditionally reset to `false` on every `switchMode` call (line 488). When the user re-enters Brain mode manually, the plan file still exists on disk, but the signal is `false`, so the button stays hidden — even though there IS a valid plan ready to execute.

The backend function `latest_plan_file()` (finalize_plan.rs:163-179) already exists and can check for plan files on disk. The fix is to call it when the user manually switches to Brain mode.

## Goal (Definition of Done)

When the user manually switches to Brain mode (human origin), if a plan `.md` file already exists on disk for that workspace, the "Continue with Builder" button appears immediately — no need to run Brain again.

## Key Findings (Prova Real)

1. **`hasPlanBeenWritten` signal reset on every mode switch** — `ChatPanel.tsx:488`: `setHasPlanBeenWritten(false)` runs unconditionally in `switchMode`, even when valid plans exist on disk
2. **Plan detection logic already exists** — `finalize_plan.rs:163-179`: `latest_plan_file()` scans the plans directory for the newest `.md` file. This is already used in the backend golden-loop verification.
3. **Plans directory resolution already exists** — `write_plan.rs:40-49`: `plans_dir()` resolves the plan directory path, respecting custom `plan_save_path` from config
4. **No backend command exposes plan existence to frontend** — Currently there is no IPC command that the frontend can call to check if a plan exists. The `get_config` command returns `planSavePath` but does not check for actual plan files.
5. **Button visibility condition** — `ChatPanel.tsx:1599`: `mode === "brain" && modeOrigin === "human" && status === "done" && hasPlanBeenWritten`. All 4 must be true.
6. **Golden loop is separate** — Golden loop brain switches have `origin=Agent`, so `modeOrigin() === "human"` is false, and the button never shows during golden loops. No conflict.

## Authoritative Inputs

| Input | Source | Value |
|-------|--------|-------|
| Plan directory resolution | `write_plan.rs:40-49` | `plans_dir(workspace_root, plan_save_path)` |
| `plan_save_path` config field | `provider.rs:84` | `pub plan_save_path: Option<String>` |
| `AppState` config field | `state.rs` | `pub config: Arc<Mutex<AgentConfig>>` |
| `AppState::workspace()` method | `state.rs` | Returns `WorkspaceState` for a given workspace root path |
| Command registration | `lib.rs:15` | `generate_handler![...]` in the tauri builder |

## Changes (Steps)

### 1. Add `check_plan_exists` Tauri command — `src-tauri/src/commands/agent.rs`

**Target:** `src-tauri/src/commands/agent.rs` — add a new `#[tauri::command]` function near the existing `get_session_mode` / `set_session_mode` commands.

**Mutation:** Add a new command:
```rust
/// Check whether a plan file (.md) exists on disk for the workspace.
/// Used by the frontend to decide whether to show the "Continue with Builder" button
/// when the user manually switches to Brain mode.
#[tauri::command]
pub async fn check_plan_exists(
    workspace: String,
    state: State<'_, AppState>,
) -> Result<bool, String> {
    let ws = state.workspace(&workspace).await?;
    let workspace_root = ws.root.to_string_lossy().to_string();

    let cfg = state.config.lock().await;
    let plan_save_path = cfg.plan_save_path.clone();
    // Also merge workspace-level config if present
    let ws_config = crate::agent::provider::read_workspace_config(&workspace_root);
    let effective_plan_save_path = ws_config
        .as_ref()
        .and_then(|w| w.plan_save_path.as_deref())
        .or(plan_save_path.as_deref())
        .map(|s| s.to_string());
    drop(cfg);

    let dir = crate::agent::tools::write_plan::plans_dir(
        &workspace_root,
        effective_plan_save_path.as_deref(),
    );

    if !dir.exists() {
        return Ok(false);
    }

    let has_md = std::fs::read_dir(&dir)
        .map(|entries| {
            entries.flatten().any(|entry| {
                entry.path()
                    .extension()
                    .and_then(|e| e.to_str())
                    == Some("md")
            })
        })
        .unwrap_or(false);

    Ok(has_md)
}
```

**Why:** Provides a lightweight IPC call for the frontend to check plan existence without constructing a full `ToolContext`. Reuses existing `plans_dir()` and respects workspace-level `plan_save_path` override.

**Constraints:** Read-only, no side effects. Safe to call at any time.

### 2. Register new command — `src-tauri/src/lib.rs`

**Target:** `src-tauri/src/lib.rs` — `generate_handler!` macro list.

**Mutation:** Add `commands::agent::check_plan_exists` to the handler list.

**Why:** Required for the command to be callable from the frontend via `invoke`.

### 3. Add IPC wrapper — `src/lib/ipc.ts`

**Target:** `src/lib/ipc.ts` — near the existing `setSessionMode` / `getSessionMode` functions.

**Mutation:** Add:
```ts
export function checkPlanExists(workspace: string): Promise<boolean> {
  return invoke<boolean>("check_plan_exists", { workspace });
}
```

**Why:** Type-safe IPC call from SolidJS frontend to Rust backend.

### 4. Call `checkPlanExists` on human Brain mode switch — `src/components/ChatPanel.tsx`

**Target:** `src/components/ChatPanel.tsx` — `switchMode` function (lines 486-503).

**Mutation:** After `setActiveSessionId(result.sessionId)` (line 498), add:
```tsx
// If switching to brain mode and a plan already exists on disk, show the button immediately
if (m === "brain") {
  const planExists = await checkPlanExists(props.workspace);
  if (planExists) setHasPlanBeenWritten(true);
}
```

**Why:** This is the core fix. When the user manually clicks the Brain toggle, we check the filesystem for an existing plan. If one exists, we show the button without waiting for a new Brain session.

**Constraints:** Must be inside the existing `try` block, after `setSessionMode` succeeds. The `catch` block already handles backend-unavailable gracefully.

### 5. Add import — `src/components/ChatPanel.tsx`

**Target:** `src/components/ChatPanel.tsx` — import section at top.

**Mutation:** Add `checkPlanExists` to the existing import from `"../lib/ipc"`.

**Why:** The new function needs to be imported to be used.

## Verification Plan

### Dry-run / unit test

1. **Run existing tests:** `cd /Users/victortavernari/claudinio_code && pnpm test` — verify no regressions (ChatPanel does not have component tests currently, but Tauri command tests and task tests should pass).
2. **Rust build:** `cd src-tauri && cargo check` — verify the new command compiles without errors.

### Integration / end-to-end

3. **Manual test — fresh plan flow (regression check):**
   - Open workspace without a plan
   - Click Brain toggle → type a message → wait for Brain to finish and call `write_plan`
   - Verify "Continue with Builder" button appears ✅

4. **Manual test — the bug fix (the critical path):**
   - Open workspace WITH an existing plan in `.claudinio/plans/` (or `docs/plans/`)
   - Click Brain toggle → verify "Continue with Builder" button appears IMMEDIATELY (no need to send a message) ✅
   - Click the button → verify it switches to Builder and sends "Execute the plan" ✅

5. **Manual test — custom plan_save_path:**
   - Configure workspace with `plan_save_path: "docs/plans"` in `.claudinio/config.json`
   - Create a plan in `docs/plans/`
   - Click Brain toggle → verify button appears ✅

6. **Manual test — no plan exists:**
   - Open a fresh workspace with no plan files
   - Click Brain toggle → verify button does NOT appear (correct behavior) ✅

7. **Manual test — golden loop non-interference:**
   - Send a message with `<goal>` tags → golden tasks created
   - Verify golden loop still works (Brain↔Builder auto-flipping) ✅
   - Verify "Continue with Builder" button does NOT appear during golden loop (origin=Agent, not human) ✅

### Edge cases

8. **Plans directory doesn't exist:** `check_plan_exists` returns `false`, button stays hidden — correct.
9. **Re-entering Brain after Builder completes:** This is the exact bug scenario — button must appear.
10. **Rapid mode toggle:** Toggling Brain→Builder→Brain quickly — each `switchMode` call is async, the last one wins. The `hasPlanBeenWritten` signal is set correctly by the last call.

## Tasks Summary

| Task | Description |
|------|-------------|
| 1 | Add `check_plan_exists` Tauri command in `commands/agent.rs` |
| 2 | Register the command in `lib.rs` |
| 3 | Add IPC wrapper in `ipc.ts` |
| 4 | Call it in `switchMode` in `ChatPanel.tsx` and add import |
| 5 | Run tests + cargo check to verify |

## Implementation Log — 2026-07-10 23:58
**Summary:** Fix: 'Continue with Builder' button now appears immediately when re-entering Brain mode if a plan file already exists on disk
**Changed files:** M src-tauri/src/commands/agent.rs, M src-tauri/src/lib.rs, M src/components/ChatPanel.tsx, M src/lib/ipc.ts, ?? docs/plans/2026-07-09_deploy-tag-0-1-1.md, ?? docs/plans/2026-07-10_fix-continue-with-builder-button-reappear.md, ?? docs/plans/2026-07-10_steering-attachments.md
**Commits:** _(git unavailable or none)_
**Journal:** **Root cause:** hasPlanBeenWritten signal was only set by write_plan tool result events. When user re-entered Brain mode via toggle, switchMode() unconditionally reset it to false. The plan file on disk was never re-checked.

**Solution:** Added a new Tauri command check_plan_exists that reuses the existing write_plan::plans_dir() to find the plans directory (respecting both global and workspace-level plan_save_path), then checks if any .md files exist. The frontend calls this in switchMode() right after setSessionMode succeeds — if switching to Brain and a plan exists, hasPlanBeenWritten is set to true immediately, making the button appear.

**Files changed:**
- src-tauri/src/commands/agent.rs: new check_plan_exists command (50 lines)
- src-tauri/src/lib.rs: registered the command in generate_handler!
- src/lib/ipc.ts: added checkPlanExists wrapper
- src/components/ChatPanel.tsx: imported checkPlanExists, added call in switchMode

**Key decisions:** Used workspace-level config merge (.claudinio.json plan_save_path) for consistency with existing config resolution pattern in get_config. The command is read-only and safe to call any time. No changes needed to the golden loop or mode origin logic since golden loops use origin=Agent (button condition requires origin=human).

**Task journal:**
- Add check_plan_exists Tauri command: Added check_plan_exists command after get_session_mode
- Register new command in lib.rs: Added right after get_session_mode in the handler list
- Add IPC wrapper in ipc.ts: Added checkPlanExists after getSessionMode
- Call checkPlanExists in switchMode in ChatPanel.tsx: Imported checkPlanExists, added logic in switchMode after setSessionMode succeeds
- Test: run tests + cargo check: 384 tests passed, 23 test files. Rust cargo check OK.
