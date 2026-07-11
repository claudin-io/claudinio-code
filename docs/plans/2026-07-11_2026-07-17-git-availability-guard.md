# Plan: Hide GitIndicator when git is not available

## Context

A Windows user crashes on first run because git is not found. The `GitIndicator` component calls `git_status` and `git_branch` unconditionally on mount (and every 5s thereafter). When git is not installed:

1. `run_git()` in `commands/git.rs` tries `std::process::Command::new("git")` — on Windows this can trigger OS-level "Windows cannot find git" dialogs, or in some environments block the process creation call path, causing the webview to freeze ("Not Responding").
2. The `GitIndicator` button is rendered unconditionally — even when git is absent, the button appears (dimmed/empty) but still shows up in the UI, confusing the user.
3. There is zero git-availability detection anywhere in the frontend or backend.

## Solution Design

### 1. New backend command: `check_git_available`

**File:** `src-tauri/src/commands/git.rs`

Add a new synchronous Tauri command:

```rust
#[tauri::command]
pub fn check_git_available() -> bool {
    std::process::Command::new("git")
        .arg("--version")
        .output()
        .map(|o| o.status.success())
        .unwrap_or(false)
}
```

This is intentionally kept synchronous and simple — it runs a single short-lived process and returns `bool`. Tauri dispatches sync commands on a thread pool, so it won't block the UI.

### 2. Register the command

**File:** `src-tauri/src/lib.rs`

Add `commands::git::check_git_available` to `generate_handler![]`.

### 3. Frontend IPC binding

**File:** `src/lib/ipc.ts`

Add:

```ts
export function checkGitAvailable(): Promise<boolean> {
  return invoke<boolean>("check_git_available");
}
```

### 4. Conditionally render GitIndicator

**File:** `src/components/GitIndicator.tsx`

- Add a `gitAvailable` signal, initialized to `null` (loading state).
- On mount, call `checkGitAvailable()` once and store result.
- Return `null` (render nothing) when `gitAvailable` is `false`.
- Keep existing 5s polling of `gitStatus`/`gitBranch` — but only when `gitAvailable` is `true`.
- Clean up intervals properly in all branches.

### 5. Existing `run_git` already handles error gracefully

No change needed to `run_git()`. When git is not found, `Command::new("git")` fails with `io::Error`, and `git_status`/`git_branch` return `Err`. The frontend already catches these. The fix is to not call them at all when git is absent.

## Risks

- **Race condition**: If git is installed but the `check_git_available` command runs before the PATH is fully resolved (e.g., slow shell init), it might report false negative. Mitigation: `Command::new("git")` uses the current process PATH — if git is in the user's system PATH, it will be found on any platform. On Windows, this is always the system PATH as resolved by the installer.
- **Minimal blast radius**: The change is additive (new command) + conditional rendering. Existing behavior is untouched when git is present.

## Tasks summary

| # | Task | File(s) | Description |
|---|------|---------|-------------|
| 1 | Add `check_git_available` command | `src-tauri/src/commands/git.rs` | Add Tauri command that runs `git --version` and returns bool |
| 2 | Register command | `src-tauri/src/lib.rs` | Add to `generate_handler![]` |
| 3 | Add IPC binding | `src/lib/ipc.ts` | Add `checkGitAvailable()` function |
| 4 | Conditional GitIndicator | `src/components/GitIndicator.tsx` | Check availability on mount, render null if unavailable |


## Implementation Log — 2026-07-11 22:26
**Summary:** Add git availability guard — GitIndicator auto-hides when git is not installed
**Changed files:** A	docs/plans/2026-07-11_2025-07-14-anti-stall-harness.md, M	src-tauri/src/agent/persist.rs, M	src-tauri/src/agent/session.rs, M	src/components/ChatPanel.tsx
**Commits:** 38d0468 feat: add anti-stall harness with Brain progress guard, per-round compaction, and tool-result truncation
**Journal:** Key decisions:
1. check_git_available is a simple sync Tauri command running `git --version` — it's intentionally synchronous because Tauri dispatches sync commands on its thread pool, so a single process spawn won't block the UI for more than a few ms.
2. The GitIndicator component now calls checkGitAvailable() on mount (once). If false, the entire button is hidden via `<Show when={gitAvailable() === true}>` — no button, no tooltip, no icon. This is the cleanest UX: users without git simply don't see git UI at all.
3. Polling (5s for status, 30s for branch) was refactored into a createEffect that only runs when gitAvailable is confirmed true. onCleanup inside the effect handles interval teardown automatically when the component unmounts or the workspace changes.
4. No changes to run_git() were needed — it already returns Err gracefully when git isn't found. The fix is simply to never call those commands when git is absent.
5. Pre-existing test failures (test_read_file_large_truncated, theme.ts TS errors) are unrelated to this change.

**Task journal:**
- Add check_git_available command in Rust backend: Added check_git_available() at end of git.rs — runs `git --version` via std::process::Command and returns bool
- Register check_git_available in Tauri handler: Added commands::git::check_git_available to generate_handler![]
- Add IPC binding for checkGitAvailable: Added checkGitAvailable() IPC function next to gitBranch in ipc.ts
- Conditionally render GitIndicator based on git availability: User confirmed: remove button from UI entirely when git not found (render null); Refactored polling into createEffect that only runs when gitAvailable() === true — intervals are cleaned up automatically via onCleanup inside the effect; Wrapped button in <Show when={gitAvailable() === true}> to render nothing when git is missing
