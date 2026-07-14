# Fix: total cost in footer doesn't include subagent cost

## Context

No session stats footer, the displayed total cost (e.g. `$0.0753`) is lower than the sum of individual subagent costs shown in chat rows (`$0.0527 + $0.0462 + $0.0406 = $0.1395`). Subagent **tokens** are correctly counted in the total (via `total_in`/`total_out`), but subagent **cost** is accumulated in a separate `subagent_cost` variable that never reaches the live `SessionStats` event.

Two omissions in `src-tauri/src/agent/session.rs`:

### Problem 1: Live `SessionStats` emits wrong `cumulative_cost` (~line 1410)

The live stats event (sent after each streaming round) computes:

```rust
cumulative_cost: Some(live_cost_input + live_cost_output + live_cost_cache),
```

Where:
- `live_cost_input = cumul_cost_input.unwrap_or(0.0) + round_ci`
- `live_cost_output = cumul_cost_output.unwrap_or(0.0) + round_co`
- `live_cost_cache = cumul_cost_cache.unwrap_or(0.0) + round_cc`

None of these include `subagent_cost`. The `roll_cost` function correctly adds `subagent_cost` to the blended `cumul_cost`, but the live stats ignore the blended value and recompute from the breakdowns which have NO subagent cost.

### Problem 2: Terminal paths don't emit final `SessionStats`

After `roll_cost` + `write_status` (5 terminal paths), the code sends only `AgentEvent::Done`. No `AgentEvent::SessionStats` follows, so the frontend footer never sees the corrected `cumul_cost` (which now includes subagent cost via `roll_cost`). The value only self-corrects on session reload (History -> re-open), because `getSessionStats` reads the persisted `total_cost` from `write_status`, which does include the blended subagent cost.

## Solution Design

All changes in `src-tauri/src/agent/session.rs`. No frontend changes needed.

### Fix 1: Live `SessionStats` cumulative_cost formula (line 1410)

Replace the recomputed-from-breakdowns formula with the blended value:

```rust
cumulative_cost: Some(cumul_cost.unwrap_or(0.0) + round_ci + round_co + round_cc + subagent_cost),
```

The `cost_input`/`cost_output`/`cost_cache_read` breakdown fields stay unchanged.

### Fix 2: Extract `emit_final_stats` closure and call on all 5 terminal paths

Define a local closure after the variable declarations (~line 1211):

```rust
let emit_final_stats = |cumul_in: u64, cumul_out: u64, cumul_cost: Option<f64>,
                        cumul_cost_input: Option<f64>, cumul_cost_output: Option<f64>,
                        cumul_cost_cache: Option<f64>, last_context: u64|
{
    let _ = event_tx.send(AgentEvent::SessionStats {
        input_tokens: cumul_in as u32,
        output_tokens: cumul_out as u32,
        cumulative_cost: cumul_cost,
        cost_input: cumul_cost_input,
        cost_output: cumul_cost_output,
        cost_cache_read: cumul_cost_cache,
        context_tokens: last_context,
        max_context_tokens: MAX_CONTEXT_TOKENS,
        compact_threshold: COMPACT_THRESHOLD,
    });
};
```

Called after each `write_status(...)` and before `AgentEvent::Done` in all 5 terminal paths.

## Risks

- Low risk — the formula change only affects the live stats value sent to the frontend.
- The `cumulative_cost` may now be slightly higher than `cost_input + cost_output + cost_cache_read` in the tooltip — this is correct since subagent costs don't have a per-category breakdown.

## Non-goals

- No frontend changes
- No changes to persisted cost data or schema
- No changes to `roll_cost` or `write_status` functions
- No changes to `SessionStats` struct/event schema

## Low-Level Design

All changes are in a single file: `/Users/victortavernari/claudinio_code/src-tauri/src/agent/session.rs`. The function `run_workflow_with_profile` (which spans ~lines 1180-2090) contains all the existing code that needs modification. No other files or functions are touched.

Three atomic changes are needed: fix the formula on line 1410, add a closure after line 1211, and insert the closure call at 5 terminal sites.

### Variables relevant to the fix

All declared ~lines 1197-1214 in `run_workflow_with_profile`:
- `cumul_in: u64` — persisted total input tokens (cumul.0)
- `cumul_out: u64` — persisted total output tokens (cumul.1)
- `cumul_cost: Option<f64>` — blended cumulative cost, includes historical subagent via `roll_cost` (cumul.2)
- `cumul_cost_input: Option<f64>` — per-category input cost breakdown (cumul.3)
- `cumul_cost_output: Option<f64>` — per-category output cost breakdown (cumul.4)
- `cumul_cost_cache: Option<f64>` — per-category cache read cost breakdown (cumul.5)
- `subagent_cost: f64` — accumulates this run's subagent cost from `run_spawn_agents` (line 1884)
- `last_context: u64` — context tokens for current round, updated per-stream
- `event_tx: &Channel<AgentEvent>` — Tauri IPC channel, available throughout

### Data flow for subagent cost

1. Tool execution loop at ~line 1884: `subagent_cost += sub_cost;` — accumulates cost from `run_spawn_agents`
2. `roll_cost(... subagent_cost ...)` on terminal paths blends `subagent_cost` into `cumul_cost`
3. `write_status(... cumul_cost ...)` persists to `SessionRecord::Status { total_cost }`
4. `AgentEvent::SessionStats` — drives footer display; currently emitted live at line 1407 and post-compaction at 1149/1303, but never after terminal `roll_cost`
5. `AgentEvent::Done` — ends the turn, carries no cost data (frontend only reads cost from SessionStats)

### Change 1: Fix live SessionStats cumulative_cost (line 1410)

**Location:** Around line 1410, in the live stats emission block (lines 1399-1417).

**Current code:**
```rust
cumulative_cost: Some(live_cost_input + live_cost_output + live_cost_cache),
```

**Replacement:**
```rust
cumulative_cost: Some(cumul_cost.unwrap_or(0.0) + round_ci + round_co + round_cc + subagent_cost),
```

`round_ci`, `round_co`, `round_cc` are the current round's cost breakdowns computed at line 1400-1403 from `cost_or_estimate`. This formula matches how `roll_cost` works: persisted blended total + this round's costs + this run's subagent cost.

### Change 2: Add `emit_final_stats` closure (after ~line 1211)

**Location:** After `let mut subagent_cost: f64 = 0.0;` (the last variable declaration), before the `while` loop that starts the main run loop.

**Code to insert:**
```rust
let emit_final_stats = |cumul_in: u64, cumul_out: u64, cumul_cost: Option<f64>,
                        cumul_cost_input: Option<f64>, cumul_cost_output: Option<f64>,
                        cumul_cost_cache: Option<f64>, last_context: u64|
{
    let _ = event_tx.send(AgentEvent::SessionStats {
        input_tokens: cumul_in as u32,
        output_tokens: cumul_out as u32,
        cumulative_cost: cumul_cost,
        cost_input: cumul_cost_input,
        cost_output: cumul_cost_output,
        cost_cache_read: cumul_cost_cache,
        context_tokens: last_context,
        max_context_tokens: MAX_CONTEXT_TOKENS,
        compact_threshold: COMPACT_THRESHOLD,
    });
};
```

### Change 3: Call `emit_final_stats` at all 5 terminal paths

Each insertion point is AFTER `write_status(...)` and BEFORE `event_tx.send(AgentEvent::Done { ... })`. All calls use the same argument pattern with already-rolled cumulative values.

**Path A — Interrupt mid-stream (~line 1455):**
```rust
            emit_final_stats(cumul_in, cumul_out, cumul_cost,
                cumul_cost_input, cumul_cost_output, cumul_cost_cache, last_context);
```

**Path B — max_tokens (~line 1525):**
```rust
            emit_final_stats(cumul_in, cumul_out, cumul_cost,
                cumul_cost_input, cumul_cost_output, cumul_cost_cache, last_context);
```

**Path C — Normal Done (~line 1774):**
```rust
            emit_final_stats(cumul_in, cumul_out, cumul_cost,
                cumul_cost_input, cumul_cost_output, cumul_cost_cache, last_context);
```

**Path D — Interrupt post-tools (~line 2026):**
```rust
            emit_final_stats(cumul_in, cumul_out, cumul_cost,
                cumul_cost_input, cumul_cost_output, cumul_cost_cache, last_context);
```

**Path E — max_rounds (~line 2061):**
```rust
        emit_final_stats(cumul_in, cumul_out, cumul_cost,
            cumul_cost_input, cumul_cost_output, cumul_cost_cache, last_context);
```

### Verification

1. `cargo check` in `src-tauri/` — no compilation errors.
2. Run app, trigger `spawn_agents` (ask an exploration question), confirm:
   - While streaming: footer `$` >= sum of subagent costs shown in chat rows.
   - After turn ends (Done, with subagents spawned): footer value matches value after session reload (History -> re-open).

## Tasks summary

1. **Fix live SessionStats formula** — Change `cumulative_cost` line 1410 from breakdown-sum to blended formula including `subagent_cost`.
2. **Add emit_final_stats closure** — Insert closure after variable declarations.
3. **Insert call at terminal paths** — Call `emit_final_stats` in all 5 terminal paths.
4. **Verify** — `cargo check` + end-to-end test.


## Implementation Log — 2026-07-14 12:41
**Summary:** Fix footer total cost to include subagent cost: fix live formula + emit SessionStats on terminal paths
**Changed files:** M src-tauri/src/agent/session.rs, ?? docs/plans/2026-07-14_fix-subagent-cost-footer.md
**Commits:** _(git unavailable or none)_
**Journal:** Two co-dependent bugs were fixed in `run_workflow_with_profile` in `session.rs`:

**Bug 1 — Live footer cost ignoring subagents:** The `SessionStats` event emitted after each streaming round used a cumulative_cost formula derived from per-category breakdown variables (`live_cost_input + live_cost_output + live_cost_cache`). These breakdowns never include subagent cost (which is tracked in a separate `subagent_cost` variable and only blended into the total via `roll_cost` at terminal paths). Fix: switched to the blended formula `cumul_cost.unwrap_or(0.0) + round_ci + round_co + round_cc + subagent_cost`, matching how `roll_cost` accumulates.

**Bug 2 — No final SessionStats after terminal paths:** The 5 terminal paths (interrupt mid-stream, max_tokens, normal Done, interrupt post-tools, max_rounds) all call `roll_cost` + `write_status` (which correctly persists the blended cumulative cost including subagent cost), but then only emit `AgentEvent::Done`. No `AgentEvent::SessionStats` follows, so the frontend footer never sees the corrected value until session reload from History. Fix: extracted a local closure `emit_final_stats` that sends `SessionStats` with the already-rolled cumulative values, and called it in all 5 terminal paths just before the `Done` event.

The closure captures `event_tx` by reference and accepts the 7 rolled values as parameters. The same `MAX_CONTEXT_TOKENS` and `COMPACT_THRESHOLD` constants used in the existing live stats emission are reused.

The `edit_file` for Path A (interrupt mid-stream) also inadvertently matched and replaced Path D (interrupt post-tools) since both have `stop_reason: "interrupted"` — this was the correct outcome since both needed the same insertion, just at different locations. Verified: all 5 call sites are present in the final diff.

No frontend changes were needed. The `cargo check` passes cleanly."""

**Task journal:**
- Fix live SessionStats cumulative_cost formula: Changed line 1410 from `cumulative_cost: Some(live_cost_input + live_cost_output + live_cost_cache),` to `cumulative_cost: Some(cumul_cost.unwrap_or(0.0) + round_ci + round_co + round_cc + subagent_cost),`
- Add emit_final_stats closure: Inserted `emit_final_stats` closure after `let mut subagent_cost: f64 = 0.0;` at line 1212, capturing `event_tx` by reference
- Insert emit_final_stats call at all 5 terminal paths: Path A - Interrupt mid-stream (line 1473); Path B - max_tokens (line 1545); Path C - Normal Done (line 1796); Path D - Interrupt post-tools (line 2050); Path E - max_rounds (line 2087)
