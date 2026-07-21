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
