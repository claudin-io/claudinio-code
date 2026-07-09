# Plan: Open folder picker at workspace root in settings

## 1. Context / Problem Statement

When the user opens **Settings → Plan save path → Browse folder** (folder icon button), the native OS folder picker dialog opens at the OS-default location (usually home directory or last-visited folder). The user wants it to open **already at the workspace root**, so they can pick a subfolder relative to the workspace.

**Confirmed by user:** Yes, the folder picker should default to the workspace root when triggered from the settings "Browse folder" button.

**Inferred:** The main "Open folder" button (used to open new workspaces) should remain unchanged — it should keep opening at the OS default.

## 2. Goal (Definition of Done)

Clicking the folder icon button in **Settings → Plan save path** opens the native folder picker dialog with its initial directory set to the active workspace root. The main "Open folder" button behavior is unchanged.

## 3. Key Findings (Prova Real)

| # | Finding | Traceability |
|---|---|---|
| 1 | The folder icon button in settings calls `pickPlanPath()` | `src/App.tsx:430` — `<button onClick={pickPlanPath}>` |
| 2 | `pickPlanPath()` calls `pickFolder()` with **no arguments** | `src/App.tsx:222` — `const folder = await pickFolder();` |
| 3 | `pickFolder()` calls `open({ directory: true, multiple: false })` with **no `defaultPath`** | `src/lib/ipc.ts:20` — `const selected = await open({ directory: true, multiple: false });` |
| 4 | The Tauri `open()` API **does support** a `defaultPath` option | `@tauri-apps/plugin-dialog` — the `open()` FilteredOptions includes an optional `defaultPath` string |
| 5 | `pickPlanPath()` already has access to `activeWorkspace()` | `src/App.tsx:225` — `const ws = activeWorkspace();` (already used for relative-path conversion) |
| 6 | `openFolder()` (main "Open folder" button) also calls `pickFolder()` with no arguments | `src/App.tsx:307` — should remain unchanged |
| 7 | Tests for `pickFolder()` exist and expect no `defaultPath` | `src/lib/ipc.test.ts:1095-1115` — two test cases |

## 4. Authoritative Inputs

| Input | Value | Source |
|---|---|---|
| `pickFolder()` signature | `(): Promise<string \| null>` | `src/lib/ipc.ts:19` |
| `open()` API | `open(options: { directory, multiple, defaultPath?, ... })` | `@tauri-apps/plugin-dialog` |
| `activeWorkspace()` return type | `string \| undefined` | `src/App.tsx` (signal) |
| Approach selected by user | "Add optional defaultPath param to pickFolder()" | User confirmation |

## 5. Changes (Steps)

### Step 1: Modify `pickFolder()` to accept optional `defaultPath`

- **Target:** `src/lib/ipc.ts`, lines 19–22
- **Mutation:** Add optional `defaultPath?: string` parameter. When provided, pass it to the `open()` call.
- **Why:** Enables callers to set the initial directory; keeps the function backward-compatible (no defaultPath = OS default, same as before).
- **Constraints:** Default parameter value is `undefined` — no breaking change to `openFolder()`.

```typescript
// BEFORE
export async function pickFolder(): Promise<string | null> {
  const selected = await open({ directory: true, multiple: false });
  return typeof selected === "string" ? selected : null;
}

// AFTER
export async function pickFolder(defaultPath?: string): Promise<string | null> {
  const selected = await open({ directory: true, multiple: false, ...(defaultPath ? { defaultPath } : {}) });
  return typeof selected === "string" ? selected : null;
}
```

### Step 2: Pass workspace root from `pickPlanPath()`

- **Target:** `src/App.tsx`, line 222
- **Mutation:** Pass `activeWorkspace()` (or `undefined` when no workspace is open) as `defaultPath` to `pickFolder()`.
- **Why:** This is the call site that needs the workspace-root default behavior.
- **Constraints:** When no workspace is active (`activeWorkspace()` returns `undefined`), `pickFolder()` receives `undefined` and falls back to OS-default — same as before.

```typescript
// BEFORE
const pickPlanPath = async () => {
  const folder = await pickFolder();
  // ...

// AFTER
const pickPlanPath = async () => {
  const folder = await pickFolder(activeWorkspace());
  // ...
```

### Step 3: `openFolder()` — NO CHANGE

- **Target:** `src/App.tsx`, line 307
- **Mutation:** None. Keep calling `pickFolder()` with no arguments.
- **Why:** The main "Open folder" dialog should not change behavior.
- **Constraint:** Verify the call site is untouched after the change.

### Step 4: Update tests for `pickFolder()`

- **Target:** `src/lib/ipc.test.ts`, lines 1091–1115
- **Mutation:** Add a test case that verifies `defaultPath` is passed through to `open()` when provided.
- **Why:** The new parameter must be covered by tests.

New test case:
```typescript
it("passes defaultPath to open when provided", async () => {
  const { open } = await import("@tauri-apps/plugin-dialog");
  vi.mocked(open).mockResolvedValue("/workspace/sub");
  const { pickFolder } = await import("./ipc");
  const result = await pickFolder("/workspace");
  expect(vi.mocked(open)).toHaveBeenCalledWith({ directory: true, multiple: false, defaultPath: "/workspace" });
  expect(result).toBe("/workspace/sub");
});
```

## 6. Verification Plan

| # | Check | Method | Expected |
|---|---|---|---|
| 1 | Existing `pickFolder()` tests still pass | `npm test -- --run src/lib/ipc.test.ts` | All 3 tests pass (2 existing + 1 new) |
| 2 | New test: `defaultPath` passed through | New test case (see Step 4) | `open()` called with `defaultPath: "/workspace"` |
| 3 | No argument call still works | Existing test at line 1095 | `open()` called with `{ directory: true, multiple: false }` only |
| 4 | `openFolder()` call site unchanged | Manual code review — line 307 | Still calls `pickFolder()` with no arguments |
| 5 | TypeScript compilation | `npx tsc --noEmit` | No errors |

## 7. Risks

- **Low risk.** The change is additive (optional parameter). All existing call sites are backward-compatible. The Tauri `open()` API is documented to accept `defaultPath`.
- **Edge case:** When no workspace is active, `activeWorkspace()` returns `undefined`, and `pickFolder(undefined)` passes no `defaultPath` — same behavior as before. ✅

## 8. Tasks Summary

1. Add optional `defaultPath?: string` parameter to `pickFolder()` in `src/lib/ipc.ts`
2. Pass `activeWorkspace()` as `defaultPath` from `pickPlanPath()` in `src/App.tsx`
3. Add test case for `defaultPath` in `src/lib/ipc.test.ts`
4. Run tests and verify TypeScript compiles
