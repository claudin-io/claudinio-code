# Plan: Fix "Continue with Builder" Button — Icon + Visibility

## 1. Context / Problem Statement

Two bugs were identified (user-provided screenshots):

1. **Icon doesn't appear** in the "Continue with Builder" button. The button uses a raw UnoCSS/iconify class `i-lucide:construction-worker` instead of the project's `<Icon>` component. The project uses `<Icon name="construction-worker" />` everywhere else (e.g., the mode toggle buttons), which resolves SVG paths from `PATHS` in `Icon.tsx`. The raw `i-lucide:*` class likely isn't installed/registered.

2. **Button appears when it shouldn't.** The visibility condition is `mode() === "brain" && modeOrigin() === "human" && status() === "done"`. This is too broad — it shows the button whenever the user is in Brain mode with `done` status, even when no plan was written. The user observed: after the plan was implemented by Builder, switching back to Brain mode (via the toggle) incorrectly shows the button again. The button should only appear after `write_plan` has been called during the current Brain session, and before the plan has been acted on.

## 2. Goal (Definition of Done)

- Bug 1: The "Continue with Builder" button renders the `construction-worker` icon correctly, using `<Icon name="construction-worker" />`.
- Bug 2: The button ONLY shows when ALL of: (a) mode is "brain", (b) mode origin is "human", (c) status is "done", AND (d) a `write_plan` tool was executed and returned a result during this Brain session.
- Button does NOT show: after plan was already implemented by Builder and user toggles back to Brain; when user enters Brain without writing a plan.

## 3. Key Findings (Prova Real)

| # | Finding | Method | Proof |
|---|---------|--------|-------|
| 1 | Button uses `i-lucide:construction-worker` class (raw UnoCSS) | `grep "i-lucide:" src/` | Only one result: `src/components/ChatPanel.tsx:1516` |
| 2 | Toggle buttons use `<Icon name="construction-worker" />` successfully | `read_file` of `ChatPanel.tsx:1720-1727` | Line 1726: `<Icon name="construction-worker" class="h-4 w-4" />` |
| 3 | `Icon` component resolves names from `PATHS` map; `PATHS["construction-worker"]` exists | `code_search "construction-worker"` in `Icon.tsx` | `Icon.tsx:79` — SVG path data |
| 4 | Visibility condition is `mode() === "brain" && modeOrigin() === "human" && status() === "done"` | `read_file` of `ChatPanel.tsx:1509` | Line 1509 |
| 5 | `continueWithBuilder` function at line 492 | `grep "continueWithBuilder"` | `ChatPanel.tsx:492-518` |
| 6 | `handleEvent` processes `ToolResult` events at line 979, matching by `toolId` | `read_file` of `ChatPanel.tsx:975-978` | Lines 975-978 (estimated) |
| 7 | No existing signal tracks whether `write_plan` was called | `grep "write_plan" src/components/ChatPanel.tsx` | No results |
| 8 | `ToolResultData` interface has `toolId`, `toolName`, `output`, `error` | `read_file` of `ipc.ts:194-200` | `ToolResultData` has `toolName: string` |
| 9 | `switchMode` function at line 475 — switches mode and adds modeChange to timeline | `read_file` of `ChatPanel.tsx:475-490` | Line 475 |

## 4. Authoritative Inputs

| Input | Value | Source |
|-------|-------|--------|
| Icon to use | `construction-worker` (same as mode toggle) | Existing code at ChatPanel.tsx:1726 and confirmed by user |
| Button visibility logic | Show only after `write_plan` was executed | Confirmed by user answer to alignment question |
| `hasPlanBeenWritten` signal | `createSignal(false)`, set true on `write_plan` ToolResult, reset on `continueWithBuilder` click and on `switchMode` call | Confirmed by user |

## 5. Changes (Steps)

### Step 1 — Replace icon class with `<Icon>` component
- **Target:** `/Users/victortavernari/claudinio_code/src/components/ChatPanel.tsx` line ~1516
- **Mutation:** Replace `<span class="inline-flex h-4 w-4 items-center justify-center"><span class="i-lucide:construction-worker h-4 w-4" /></span>` with `<Icon name="construction-worker" class="h-4 w-4" />`
- **Why:** The `i-lucide:*` class doesn't render; the project's `<Icon>` component resolves SVG paths correctly.
- **Constraints:** Keep the same sizing (`h-4 w-4`). Remove the wrapper `<span>` since `<Icon>` returns an `<svg>` directly.

### Step 2 — Add `hasPlanBeenWritten` signal
- **Target:** `/Users/victortavernari/claudinio_code/src/components/ChatPanel.tsx`, near existing signal declarations (around line ~472)
- **Mutation:** Add `const [hasPlanBeenWritten, setHasPlanBeenWritten] = createSignal(false);`
- **Why:** Need to track whether `write_plan` was executed during current Brain session.

### Step 3 — Set `hasPlanBeenWritten = true` on `write_plan` ToolResult
- **Target:** `/Users/victortavernari/claudinio_code/src/components/ChatPanel.tsx`, inside `handleEvent`, in the `ToolResult` branch (~line 977-978)
- **Mutation:** After applying the tool result, add: `if (data.toolName === "write_plan") setHasPlanBeenWritten(true);`
- **Why:** This is the trigger — the agent has written a plan, so we should show the button.

### Step 4 — Reset `hasPlanBeenWritten` on `continueWithBuilder` click
- **Target:** `/Users/victortavernari/claudinio_code/src/components/ChatPanel.tsx`, inside `continueWithBuilder` function (~line 493)
- **Mutation:** Add `setHasPlanBeenWritten(false);` at the start of the function (before `switchMode`).
- **Why:** The plan is being acted on, so the button should not reappear.

### Step 5 — Reset `hasPlanBeenWritten` on mode switch
- **Target:** `/Users/victortavernari/claudinio_code/src/components/ChatPanel.tsx`, inside `switchMode` function (~line 476)
- **Mutation:** Add `setHasPlanBeenWritten(false);` right before `if (m === mode()) return;` or right after the early return check.
- **Why:** When user manually toggles mode, the plan context is no longer relevant.

### Step 6 — Update button visibility condition
- **Target:** `/Users/victortavernari/claudinio_code/src/components/ChatPanel.tsx` line 1509
- **Mutation:** Change from:
  ```
  <Show when={mode() === "brain" && modeOrigin() === "human" && status() === "done"}>
  ```
  to:
  ```
  <Show when={mode() === "brain" && modeOrigin() === "human" && status() === "done" && hasPlanBeenWritten()}>
  ```
- **Why:** Now the button only shows when a plan was actually written.

### No changes to:
- `Icon.tsx` — the SVG path already exists
- `ipc.ts` — no interface changes needed
- `session.rs` (backend) — tracking `write_plan` is purely a frontend UI concern
- `switchMode` behavior — the mode switch logic itself is correct

## 6. Verification Plan

1. **Build check:** Run `npm run build` or `npx tsc --noEmit` — no new type errors.
2. **Test suite:** Run existing tests — all 73 tests should still pass.
3. **Bug 1 visual verification:** Start the app, enter Brain mode, trigger a `write_plan`, confirm the button renders with the icon visible.
4. **Bug 2 — Positive case:** After `write_plan` is executed in Brain mode, the button appears.
5. **Bug 2 — Negative case A:** Switch to Brain mode without writing a plan — button should NOT appear.
6. **Bug 2 — Negative case B:** After implementing the plan (clicking the button), switch back to Brain mode — button should NOT appear.
7. **Bug 2 — Reset on mode toggle:** Write a plan (button visible), manually toggle to Builder and back to Brain — button should NOT appear.

## 7. Tasks Summary

1. Fix icon: replace `i-lucide` class with `<Icon>` component
2. Add `hasPlanBeenWritten` signal
3. Wire signal in `ToolResult` handler, `continueWithBuilder`, and `switchMode`
4. Update button visibility condition
5. Verify all changes