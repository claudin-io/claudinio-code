## Context

The `/new` TUI command creates a new session but leaves stale state: `app.attachments` (pending file paths from the previous session), `app.running` (still `true` if a turn was in progress), and `app.status` (still `Working`). This causes confusing behavior — attachments from the old chat persist, and the UI appears busy.

## Solution Design

`new_session()` in `cli/src/tui/app.rs` must also clear these fields, exactly as the user confirmed:

1. `app.attachments.clear()`
2. `app.running = false`
3. `app.status = Status::Idle`

## Risks

None. These are local state resets on a struct that `new_session` already fully owns.

## Non-goals

- Not clearing the editor text or overlay.
- Not touching the web frontend.

## Low-Level Design

### File: `cli/src/tui/app.rs`

#### Function `new_session` (~line 1030)

Add 3 lines just before the final `app.commit_notice(...)` call:

```rust
app.attachments.clear();
app.running = false;
app.status = Status::Idle;
```

`Status` is already imported (line 12: `use super::transcript::{Status, SubLive, ToolCard};`). Both `running: bool` and `status: Status` are declared on the `App` struct (lines 76-77). No import or struct changes needed.

## Tasks summary

1. Edit `new_session` in `cli/src/tui/app.rs` — add `attachments.clear()`, `running = false`, `status = Status::Idle`.
2. `cargo build -p cli` to verify.


## Implementation Log — 2026-07-21 14:24
**Summary:** Add attachments.clear(), running=false, status=Idle to new_session()
**Changed files:** A	docs/plans/2026-07-21_tui-new-clears-attachments-status.md
**Commits:** 2ba5e12 docs(plan): tui-new-clears-attachments-status
**Journal:** Straightforward 3-line addition in `new_session()`. The tricky part was locating the exact insertion point — right before `commit_notice`, after `question = None`. The existing function already cleared in_tok/out_tok, tools, subagents, tasks, and question, but was missing attachments (Vec<String>), running (bool), and status (Status). These three fields are what made `/new` feel broken: stale attachments persisted from the old session, the spinner kept running, and the status bar stayed yellow/Working. All three are now reset before the "── new session ──" notice is committed. No imports or struct changes needed — `Status` was already imported and `attachments`/`running`/`status` are plain fields on `App`.

**Task journal:**
- Limpar attachments, running e status em new_session(): Adicionadas 3 linhas em new_session(): app.attachments.clear(), app.running = false, app.status = Status::Idle — entre app.question = None e app.commit_notice().
- Verificar compilação: Compilou sem erros. Warnings não relacionados no claudinio-core (imports não usados).
