# Solution Design: Show Subagent Input (Goal) and Summary (Report) in UI

## Context / Problem Statement

When a subagent is spawned via `spawn_agents`, the user sees a `SubagentRow` in the chat timeline showing the subagent name, mode, and status. The user can click to open a `SubagentModal` showing the subagent's internal steps (thinking, tool calls). However, **two critical pieces of information are missing**:

1. **The input (goal)** — what the parent agent told the subagent to do. The `goal` field exists in `SubagentStartedData` and is stored in `SubagentTimelineState.goal`, but it is **never rendered** in either `SubagentRow` or `SubagentModal`.

2. **The output (report/summary)** — what the subagent produced as its final answer. The Rust `SubagentResult.report` field contains the final summary, but the `AgentEvent::SubagentDone` variant **does not include a `report` field** — it only sends `subagentId`, `status`, `rounds`, `inputTokens`, `outputTokens`. The report is silently discarded.

### Root Cause

- **Rust** (`src-tauri/src/agent/session.rs` lines ~476-483): `AgentEvent::SubagentDone` is missing a `report: String` field.
- **Rust** (`src-tauri/src/agent/subagent.rs` lines ~206-211): When `SubagentDone` is sent, `result.report` is not forwarded.
- **TypeScript** (`src/lib/ipc.ts` lines ~80-86): `SubagentDoneData` interface has no `report` field.
- **TypeScript** (`src/components/ChatPanel.tsx`): `SubagentTimelineState` lacks `report` field; `SubagentRow` and `SubagentModal` never render `goal` or `report`.

## Goal (Definition of Done)

The subagent's **goal** (input) and **report** (output) are visible to the user:
- **Inline (SubagentRow)**: truncated `goal` (first ~80 chars) and truncated `report` (first ~120 chars) shown as dim text below the name/status.
- **Detail (SubagentModal)**: full `goal` in a labeled section at the top, full `report` rendered as markdown at the bottom.
- **TDD**: vitest tests prove that `SubagentTimelineState` accepts `goal` and `report` fields and that the rendering logic extracts them.
- **Backward compatible**: the Rust `SubagentDone` adds `report` as a new optional-seeming field; TypeScript `SubagentDoneData` makes `report` optional (`report?: string`) so old compiled frontends don't break.

## Key Findings (Prova Real)

| Finding | Method | Proof |
|---------|--------|-------|
| `SubagentDone` Rust variant has no `report` field | `read_file` on `session.rs` lines 474-483 | Fields: `subagent_id`, `status`, `rounds`, `input_tokens`, `output_tokens` — no `report` |
| `SubagentResult.report` exists but is discarded | `read_file` on `subagent.rs` lines 28-35 and 206-211 | `result.report` is composed into combined string for tool_result, but NOT sent in `SubagentDone` event |
| `SubagentStartedData` has `goal` but `SubagentTimelineState` has it too, just never rendered | `read_file` on `ipc.ts` lines 74-79 and `ChatPanel.tsx` `SubagentTimelineState` interface | `goal: string` exists in both types but `SubagentRow` only renders `name`, `mode`, `status` |
| `SubagentTimelineState` lacks `report` field entirely | `read_file` on `ChatPanel.tsx` `SubagentTimelineState` interface | Fields: `id`, `name`, `goal`, `mode`, `status`, `rounds`, `inputTokens`, `outputTokens`, `steps` — no `report` |
| No existing test files in the project | `ls` on project root and src | Zero `.test.*` or `.spec.*` files found |
| `vitest`, `@testing-library/dom`, `jsdom` already in `devDependencies` | `read_file` on `package.json` | Versions present: vitest ^4.1.9, @testing-library/dom ^10.4.1, jsdom ^29.1.1 |
| vite.config.ts has no vitest config | `read_file` on `vite.config.ts` | No `test` block in defineConfig |

## Authoritative Inputs

| Input | Source | Status |
|-------|--------|--------|
| User wants goal + report in both Row (inline truncated) and Modal (full) | User confirmation | CONFIRMED |
| Use vitest + @testing-library/dom (already in package.json) | User confirmation | CONFIRMED |
| Report should render as markdown in Modal | User confirmation | CONFIRMED |
| `marked` is already imported in ChatPanel.tsx for markdown rendering | Code inspection | ACTIVE |
| `SubagentSpec` fields: name, goal, mode, expected_output | `subagent.rs` lines 16-21 | ACTIVE |
| `find_remote_skills` / `list_skills` export patterns in `ipc.ts` | `ipc.ts` | REFERENCE |

## Changes (Steps)

### 1. Rust: Add `report` field to `AgentEvent::SubagentDone`

- **Target**: `src-tauri/src/agent/session.rs`, `AgentEvent::SubagentDone` variant (lines ~476-483)
- **Mutation**: Add `report: String` field with `#[serde(rename = "report")]`
- **Why**: The report is produced by `SubagentResult` but never sent to frontend. Adding it to the event bridges the gap.
- **Constraints**: Serde rename to camelCase for TypeScript compatibility.

### 2. Rust: Forward `result.report` in `run_spawn_agents`

- **Target**: `src-tauri/src/agent/subagent.rs`, `run_spawn_agents` function (lines ~206-211)
- **Mutation**: In the `SubagentDone` send call, add `report: result.report.clone()` (or move)
- **Why**: The report was only used in the combined string for the parent's tool_result; now it's also sent to the frontend.
- **Constraints**: The combined string for the parent tool_result is kept as-is (it already includes the report text). The new field is additional, not a replacement.

### 3. TypeScript: Add `report` to `SubagentDoneData` interface

- **Target**: `src/lib/ipc.ts`, `SubagentDoneData` interface (lines ~80-86)
- **Mutation**: Add `report?: string;` (optional for backward compat)
- **Why**: Type-safe reception of the new field from the backend.

### 4. TypeScript: Add `report` to `SubagentTimelineState`

- **Target**: `src/components/ChatPanel.tsx`, `SubagentTimelineState` interface
- **Mutation**: Add `report?: string;` field
- **Why**: Store the report when SubagentDone arrives.

### 5. TypeScript: Capture `d.report` in `SubagentDone` handler

- **Target**: `src/components/ChatPanel.tsx`, `handleEvent` function, `SubagentDone` branch (~lines 590-610)
- **Mutation**: Add `report: d.report` to the state update object
- **Why**: So the report is stored in `subagentState` and available for rendering.

### 6. TypeScript: Update `SubagentRow` to show goal + report inline

- **Target**: `src/components/ChatPanel.tsx`, `SubagentRow` component
- **Mutation**: Below the status line, add two truncated text lines: `goal` (80 chars) and `report` (120 chars), styled as dim text with a left border accent.
- **Why**: User can see input/output at a glance without opening the modal.
- **Constraints**: Only show when fields are non-empty; truncate with ellipsis.

### 7. TypeScript: Update `SubagentModal` to show goal + report

- **Target**: `src/components/ChatPanel.tsx`, `SubagentModal` component
- **Mutation**: Add a "Goal" section at top (dim background, monospaced, full text) and a "Report" section at bottom (rendered as markdown via `marked.parse`).
- **Why**: Full detail view when user clicks the subagent row.
- **Constraints**: Only show when fields are non-empty; match existing modal styling patterns.

### 8. Setup vitest configuration

- **Target**: `vite.config.ts`
- **Mutation**: Add `test` block with jsdom environment, globals enabled, include pattern for `src/**/*.test.{ts,tsx}`
- **Why**: Enable `vitest` CLI to find and run tests.
- **Constraints**: Must not break the existing Vite config for Tauri dev/build.

### 9. Write TDD tests

- **Target**: New file `src/components/ChatPanel.test.ts`
- **Mutation**: Tests for:
  - `SubagentTimelineState` type compatibility (report field accepted)
  - `SubagentRow` rendered output contains goal/report when present
  - `SubagentModal` rendered output contains goal/report sections when present
  - Goal/report not shown when absent (backward compat)
- **Why**: Regression safety and TDD discipline.
- **Constraints**: Use vitest + @testing-library/dom; test pure rendering functions where possible, avoid full DOM rendering complexity for SolidJS components if too complex.

### 10. Add localization keys for new labels

- **Target**: `src/lib/locales/pt-BR.ts` and `src/lib/locales/en-US.ts`
- **Mutation**: Add keys: `chat.subagent.goal` ("Objetivo" / "Goal"), `chat.subagent.report` ("Relatório" / "Report")
- **Why**: i18n consistency.

## Verification Plan

### Dry-run (pre-build)
1. `cargo check` in `src-tauri/` — confirms Rust code compiles.
2. `pnpm exec tsc --noEmit` — confirms TypeScript types align.

### Apply
1. `cargo build` — confirms full Rust compilation.
2. `pnpm build` — confirms Vite build works with new vitest config.

### Test
1. `pnpm exec vitest run` — all new tests pass; zero regressions.

### End-to-end (manual)
1. Open a workspace in Claudinio Code.
2. Ask the agent to spawn subagents (e.g. "investigate the codebase").
3. Observe `SubagentRow` in timeline: goal and report truncated text visible below name/status.
4. Click to open `SubagentModal`: full goal in labeled section at top; full report rendered as markdown at bottom.
5. Verify backward compat: subagent without report (old backend) still renders without errors.

### Seam / wiring proof
1. `grep "report"` on `AgentEvent::SubagentDone` in session.rs — field present.
2. `grep "report: result.report"` in subagent.rs — forwarding present.
3. `grep "report?: string"` in ipc.ts — TypeScript interface updated.
4. `grep "d.report"` in ChatPanel.tsx — capture point present.

### Regression
1. Existing subagent tests in Rust (`cargo test -p claudinio-code`) still pass.
2. Subagent modal still shows steps timeline correctly.
3. Token counts still display correctly in SubagentRow.

## Risks

| Risk | Mitigation |
|------|-----------|
| SolidJS component testing with @testing-library/dom can be tricky without `solid-testing-library` | Test pure logic functions first; if SolidJS rendering tests are too complex, extract pure helper functions and test those |
| Adding `report` field to Rust event could break deserialization in old frontend | Make `report` optional in TypeScript (`report?: string`) |
| vitest config might conflict with Vite's Tauri-specific settings | Use `mergeConfig` pattern; test in isolation with `vitest` command |

## Tasks Summary

1. Add `report` to Rust `AgentEvent::SubagentDone` + forward in `run_spawn_agents`
2. Add `report` to TypeScript `SubagentDoneData` + `SubagentTimelineState`
3. Capture `report` in `SubagentDone` handler + update `SubagentRow` inline display
4. Update `SubagentModal` to show goal + report sections
5. Add localization keys for goal/report labels
6. Setup vitest config in `vite.config.ts`
7. Write TDD tests for subagent display logic
8. Verify: `cargo check`, `pnpm tsc --noEmit`, `pnpm exec vitest run`
