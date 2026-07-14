# Plan: Hide bash terminal window on Windows

## Context

On Windows, every time the Claudinio Code agent executes a bash/shell command, a black `cmd.exe` console window flashes on screen. This is distracting and looks unpolished.

## Solution Design

Add the `CREATE_NO_WINDOW` flag to the bash tool's process spawning on Windows. The utility already exists in `src-tauri/src/commands/procutil.rs` and is already used by `git.rs` and `finalize_plan.rs`. The bash tool simply never received this guard.

**User experience:** Commands run invisibly (stdout/stderr still captured and rendered in the chat panel), no console window flash.

## Risks

- **None.** The flag is already battle-tested across git and finalize_plan commands. The bash tool uses the same `tokio::process::Command` API that `no_window_tokio` targets.

## Non-goals

- Hiding console windows for grep, MCP, or LSP processes (separate future improvements).

## Low-Level Design

### Change: Add `no_window_tokio` to bash tool

**Target file:** `src-tauri/src/agent/tools/bash.rs`

**Why:** The bash tool spawns `cmd.exe` on Windows without `CREATE_NO_WINDOW`, causing a console flash every time a bash command runs.

**Pattern:** Follow the exact same approach as `src-tauri/src/commands/git.rs`:
1. Import the helper function: `use crate::commands::procutil::no_window_tokio;`
2. Call it before `spawn()`: `no_window_tokio(&mut child);`

**Exact changes (2 insertions, 0 deletions):**

1. **Add import** — after existing `use crate::agent::tools::ToolContext;` (line 9), add:
   ```rust
   use crate::commands::procutil::no_window_tokio;
   ```

2. **Apply to Command** — after `.kill_on_drop(true)` (line 135) and before `.spawn()` (line 136), add:
   ```rust
   no_window_tokio(&mut child);
   ```

**Resulting diff:**
```diff
 use crate::agent::tools::ToolContext;
+use crate::commands::procutil::no_window_tokio;

 // ...
 
 let mut child = Command::new(shell)
     .arg(shell_flag)
     .arg(&args.command)
     .env("PATH", login_path)
     .current_dir(args.workdir.as_deref().unwrap_or("."))
     .stdin(std::process::Stdio::piped())
     .stdout(std::process::Stdio::piped())
     .stderr(std::process::Stdio::piped())
     .kill_on_drop(true)
+    no_window_tokio(&mut child);
     .spawn()
     .map_err(|e| format!("failed to spawn command: {e}"))?;
```

**Note:** `no_window_tokio` is a no-op on non-Windows (`#[cfg(not(target_os = "windows"))]`), so this change is safe on macOS/Linux.

### Verification

1. **Build check:** `cargo build` in `src-tauri/` — must compile without errors.
2. **Windows smoke test:** On a Windows machine, trigger any bash command via the agent (e.g., `echo hello`). Verify no console window flashes appear.


## Implementation Log — 2026-07-14 16:19
**Summary:** Hide bash tool console window flash on Windows using CREATE_NO_WINDOW flag
**Changed files:** M src-tauri/src/agent/tools/bash.rs, ?? docs/plans/2026-07-14_deploy-v0-1-7.md, ?? docs/plans/2026-07-14_hide-bash-window-windows.md
**Commits:** _(git unavailable or none)_
**Journal:** Two-line fix: imported `no_window_tokio` from `procutil.rs` and applied it to the `tokio::process::Command` builder before spawn. Had to split the chained builder into separate statements (build + apply flag + spawn) since `no_window_tokio` takes a `&mut` ref between builder calls. The original chained `.spawn()` caused a `mut` ownership issue — fixed by declaring `let mut child` on the spawn result. Build passes cleanly. On Windows, this will suppress the `cmd.exe` console flash; on macOS/Linux it's a compile-time no-op.

**Task journal:**
- Add no_window_tokio to bash tool: Added `use crate::commands::procutil::no_window_tokio;` import at line 10; Restructured builder to allow `no_window_tokio(&mut child)` call before `.spawn()` — split the chained builder into separate statements; Uses the same `no_window_tokio` helper that git.rs already uses (cross-platform, no-op on non-Windows)
- Build check: Build passed cleanly on macOS (dev profile); The `no_window_tokio` function is a no-op on non-Windows, so macOS/Linux behavior is unchanged; On Windows, `CREATE_NO_WINDOW` (0x08000000) will suppress the console window flash
