# Fix: UI não atualiza na troca de modo (Tab)

## Context

When the user presses Tab in idle mode, `toggle_mode()` switches `app.mode` between `Brain` and `Builder`. The TUI must immediately reflect the change in three places:
1. Box title (input border): shows current mode name
2. Footer L2 bottom-right: shows model name (`brain_model` vs `builder_model`)
3. Footer L2 bottom-left: shows mode label (`brain`/`builder`)

The user reports NONE of these update after pressing Tab. Note from the attached screenshot: bottom-left L1 shows the directory path (`~/claudinio_code (feat/cli-tui)`) but the mode label is on L2 — the user expects L1 or L2 to show the current mode name after the switch.

## Solution Design

The fix must ensure that after `toggle_mode()` runs, the next `render::draw()` call picks up the new `app.mode`. The relevant code paths:

- **`app.rs:825-830`**: `toggle_mode()` — already sets `app.mode` and calls `mode_ctl.set()`
- **`render.rs:143`**: Box title reads `app.mode.as_str()` fresh every frame
- **`render.rs:202-212`**: `FooterInfo { mode: app.mode.as_str(), model: app.cur_model(), ... }` reads fresh every frame
- **`app.rs:294-303`**: After `handle_event()` returns, `commit_and_draw()` is always called

Two possible root causes to verify and address:
1. **Tab event not reaching `toggle_mode`**: `handle_key` has two `KeyCode::Tab` arms — one in the main handler, one in the overlay handler. If the overlay is active or Tab is consumed elsewhere, the toggle doesn't fire.
2. **`app.mode` reverting**: If `ModeChanged` event from the backend arrives after the toggle (processed in the `agent_rx` branch), it overwrites `app.mode` back.

## Risks

- Wrong diagnosis: the code appears correct on static analysis — the bug might be a build issue or runtime state corruption
- Fix could introduce a race between human toggle and backend `ModeChanged`

## Non-Goals

- No layout changes to the footer (only ensuring values update)
- No behavioral changes to Tab key beyond fixing the mode display
- No changes to `mode_ctl` or backend mode persistence

## Low-Level Design

### Investigation (empirical — run code probes)

1. Add temporary debug prints to verify `toggle_mode` fires on Tab:
   - In `handle_key` before `toggle_mode(app)` → print `"TOGGLE: before mode={:?}"`
   - At end of `toggle_mode` → print `"TOGGLE: after mode={:?}"`
2. In `render::draw`, at the point where `app.mode.as_str()` is read for the box title → print `"DRAW: mode={:?}"`
3. In `event::apply`, in `ModeChanged` handler → print `"EVENT: ModeChanged mode={mode} origin={origin}"`
4. Run the binary, press Tab, check terminal output for the trace

### Fix (depending on investigation)

**Hypothesis A — Tab not reaching toggle:**
No change needed to the toggle match; verify the overlay isn't intercepting. Fix: ensure `handle_key` is reached for Tab when idle.

**Hypothesis B — `ModeChanged` reverting:**
In `event.rs`, in the `ModeChanged` handler, add a guard: if the new mode from the backend is the same as what the user toggled to, or if origin=="human", skip the `app.mode = m` assignment (the mode was already set locally by `toggle_mode`).

```rust
AgentEvent::ModeChanged { mode, origin, .. } => {
    if let Some(m) = SessionMode::parse(&mode) {
        // Don't revert mode if it was already set by the local toggle
        // (both are in sync, the backend event is informational)
        if app.mode != m {
            app.mode = m;
        }
    }
    ...
}
```

Actually simpler: just don't guard — if the backend says the mode is what we already have, the assignment is a no-op anyway. The problem might be that the BACKEND hasn't processed the toggle yet and sends ModeChanged with the OLD mode. In that case we need: only accept ModeChanged if origin != "human".

**Hypothesis C — Draw works but ratatui viewport issue:**
`commit_and_draw` → `terminal.draw(|f| render::draw(f, app))` always re-renders everything. No-op.

### Tasks

1. Investigate: add temp debug traces to verify code paths for toggle and render
2. Apply fix based on findings (likely Hypothesis B)
3. Remove debug traces
4. Verify manually by running TUI and pressing Tab
