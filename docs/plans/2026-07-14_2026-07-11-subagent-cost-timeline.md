# Plan: Show per-worker cost in timeline

## Context

The timeline currently shows token counts (`35k → 3.9k`) for each subagent worker, but does **not** show the dollar cost. The cumulative session cost (including workers) is already shown in the `ContextFooter` and is correctly calculated — the gap is that individual workers display no cost, so the user can't see which workers are expensive.

Confirmed decisions:
- **Format**: Show a single total dollar value alongside tokens, e.g. `35k → 3.9k · $0.04` (Question 1 → A)
- **Total**: The session total in `ContextFooter` already aggregates worker costs correctly — no footer breakdown needed (Question 2 → A)

## Solution Design

### 1. Backend: Calculate & propagate cost in SubagentDone (Rust)

**Files**: `src-tauri/src/agent/session.rs` + `src-tauri/src/agent/subagent.rs`

#### 1a. Export cost helpers from `session.rs`
Make `cost_or_estimate` and `model_pricing` **`pub(crate)`** so `subagent.rs` can call them.

#### 1b. Add `cost` to `SubagentResult`
```rust
pub struct SubagentResult {
    pub status: &'static str,
    pub report: String,
    pub rounds: u32,
    pub in_tok: u32,
    pub out_tok: u32,
    pub cost: f64,  // NEW
}
```

#### 1c. Accumulate cost in `run_subagent`
After each round, if `stream_output.usage.cost` is `Some`, add it to `total_cost`. If `None`, fall back to `cost_breakdown_for()` using the model and tokens.

```rust
// Inside the round loop, after accumulating in/out tokens:
if let Some(u) = &stream_output.usage {
    total_in += u.input_tokens;
    total_out += u.output_tokens;
    // Accumulate cost — prefer provider-reported, fall back to estimate
    if let Some(c) = u.cost {
        total_cost += c;
    }
}
// After the loop, if total_cost is 0 and we have tokens, estimate:
if total_cost == 0.0 && (total_in > 0 || total_out > 0) {
    let est = cost_breakdown_for(&config.builder_model, total_in, 0, total_out);
    total_cost = est.input + est.output;
}
```

#### 1d. Add `cost` to `SubagentDone` event in `AgentEvent` enum
```rust
SubagentDone {
    #[serde(rename = "subagentId")]
    subagent_id: String,
    status: String,
    rounds: u32,
    #[serde(rename = "inputTokens")]
    input_tokens: u32,
    #[serde(rename = "outputTokens")]
    output_tokens: u32,
    #[serde(rename = "report")]
    report: String,
    #[serde(rename = "cost")]
    cost: f64,  // NEW
}
```

#### 1e. Pass `cost` through `run_spawn_agents`
When constructing `SubagentDone`, add `cost: result.cost`.

### 2. Frontend: Display cost in SubagentRow (TypeScript + SolidJS)

#### 2a. Update IPC types (`src/lib/ipc.ts`)
- Add `cost: number` to `SubagentDoneData`
- Add `cost?: number` to `AgentEvent["data"]` for `SubagentDone`

#### 2b. Update subagentTimeline helpers (`src/lib/subagentTimeline.ts`)
- Add `cost: number` to `SubagentNode` and `SubagentDoneInput`
- Update `applySubagentDone` to pass through `cost`

#### 2c. Update SubagentTimelineState in ChatPanel (`src/components/ChatPanel.tsx`)
- Add `cost: number` to `SubagentTimelineState` interface
- Initialize `cost: 0` in `SubagentStarted` handler
- The `SubagentDone` handler already calls `applySubagentDone` which will carry the cost

#### 2d. Display cost in SubagentRow
Near the existing token display (`formatTokens(props.subagent.inputTokens)→{formatTokens(props.subagent.outputTokens)}`), add the cost:

```tsx
<Show when={props.subagent.inputTokens > 0}>
  <span class="font-mono text-[10px] text-ink-faint">
    {formatTokens(props.subagent.inputTokens)}→{formatTokens(props.subagent.outputTokens)}
    <Show when={props.subagent.cost > 0}>
      <span class="text-ink-faint"> · ${props.subagent.cost.toFixed(4)}</span>
    </Show>
  </span>
</Show>
```

Format: Use `toFixed(4)` for small values (sub-cent precision), or dynamically choose precision — e.g. `cost < 0.01 ? toFixed(6) : cost < 1 ? toFixed(4) : toFixed(2)`.

### 3. Locale strings
The cost display is self-contained in the component markup — no new locale strings needed (the `$` symbol is universal and the format is inline).

## Data Flow (Updated)

```
run_subagent()
  ├── total_in / total_out (tokens) — existing
  ├── total_cost (NEW) — accumulated from provider or estimated
  └── returns SubagentResult { ..., cost: total_cost }

run_spawn_agents()
  └── sends AgentEvent::SubagentDone { ..., cost }  → Frontend
                                                   ↓
applySubagentDone() → SubagentNode.cost           ↓
SubagentRow renders: "{in}→{out} · ${cost}"       ↓
                                                   ↓
SessionStats (existing) → ContextFooter            ↓
  Cumulative total ALREADY includes worker costs    ↓
  (since tokens flow back to main loop → roll_cost)
```

## Risks

1. **Cache read tokens**: `run_subagent` doesn't track `cache_read_input_tokens`. The cost estimate ignores cache, which is fine — cache cost is typically ~15–30% of input cost and the provider-reported cost (when available) includes it. If the provider doesn't report cost and cache hits are significant, the estimate will be slightly low. Acceptable for a display estimate.
2. **Provider doesn't report cost**: Fallback uses `cost_breakdown_for()` which estimates from tokens × pricing. The same fallback is used in the main loop — consistent.
3. **Very small costs**: A worker with <100 tokens costs ~$0.00005. Use appropriate decimal precision: show `$0.0001` for tiny costs, `$0.0123` for typical ones.

## Verification

1. **Build check**: `cargo build` passes with new fields
2. **Unit test**: `run_subagent` returns a `SubagentResult` with `cost > 0`
3. **Frontend check**: Run the app, spawn subagents, verify each `SubagentRow` shows `{in}→{out} · ${cost}` with plausible values
4. **Total check**: Sum of all per-worker costs + main turn costs ≤ total shown in ContextFooter (should be close)


## Implementation Log — 2026-07-14 05:48
**Summary:** Show per-worker cost in timeline alongside existing token display
**Changed files:** M src-tauri/src/agent/session.rs, M src-tauri/src/agent/subagent.rs, M src/components/ChatPanel.test.ts, M src/components/ChatPanel.tsx, M src/lib/ipc.ts, M src/lib/subagentTimeline.ts, ?? docs/plans/2026-07-14_2026-07-11-subagent-cost-timeline.md
**Commits:** _(git unavailable or none)_
**Journal:** ## Implementation Log

### Changes Made Across 6 Files

**Rust Backend (3 files):**
- `src-tauri/src/agent/session.rs`: Made `Pricing`, `CostBreakdown`, `model_pricing`, `cost_breakdown_for`, `cost_or_estimate` all `pub(crate)`. Made `CostBreakdown` fields `pub(crate)`. Added `cost: f64` to `AgentEvent::SubagentDone`. Updated the round-trip test to include `cost`.
- `src-tauri/src/agent/subagent.rs`: Added `cost: f64` to `SubagentResult`. Accumulated `total_cost` per round in `run_subagent` (preferring provider-reported `usage.cost`, falling back to `cost_breakdown_for()` estimate when cost is 0 and tokens > 0). Updated all 4 return sites (failed/interrupted/completed/max_rounds) with fallback logic. Passed `result.cost` to the `SubagentDone` event in `run_spawn_agents`.

**TypeScript Frontend (3 files):**
- `src/lib/ipc.ts`: Added `cost: number` to `SubagentDoneData` interface.
- `src/lib/subagentTimeline.ts`: Added `cost: number` to `SubagentNode` and `SubagentDoneInput`. Updated `applySubagentDone` to propagate cost.
- `src/components/ChatPanel.tsx`: Added `cost: number` to `SubagentTimelineState`. Initialized `cost: 0` in both `subagentState` and `currentSteps` snapshots. Updated `SubagentRow` to display cost with adaptive precision.

### Key Design Decision
When the provider doesn't report cost (common with local/DIY proxies), the subagent estimates it using the same `cost_breakdown_for()` function the main loop uses — ensuring consistency regardless of whether the cost comes from the provider's middleware or our fallback calculation.

### Tests
- All 204 Rust tests pass (including the round-trip serde test with the new `cost` field)
- All 25 frontend tests pass (updated all `applySubagentDone` calls to include `cost`)

**Task journal:**
- Export cost helpers from session.rs: Made Pricing, CostBreakdown, model_pricing, cost_breakdown_for, cost_or_estimate all pub(crate)
- Add cost to SubagentResult and SubagentDone in Rust: Added cost: f64 to SubagentResult; Added total_cost accumulation in run_subagent (prefer provider cost, fallback to cost_breakdown_for); Updated all 4 return sites (failed/interrupted/completed/max_rounds) with cost; Updated SubagentDone event in run_spawn_agents to pass result.cost
- Add cost field to AgentEvent::SubagentDone: Added cost: f64 to AgentEvent::SubagentDone + updated existing test
- Update SubagentDoneData TypeScript type with cost field: Added cost: number to SubagentDoneData
- Update subagentTimeline.ts helpers with cost field: Added cost to SubagentNode, SubagentDoneInput, and applySubagentDone propagation
- Update SubagentTimelineState with cost field: Added cost to SubagentTimelineState, initialized as 0 in SubagentStarted handler and the inline currentSteps snapshot
- Display cost in SubagentRow component: Updated SubagentRow to show cost with adaptive precision
- Build and verify everything compiles: cargo build: ok; cargo test --lib: 204 passed, 0 failed; vitest: 25 tests passed
