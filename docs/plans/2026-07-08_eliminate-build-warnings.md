# Solution Design: Eliminate all build warnings and type errors

## Context

The project has **7 warnings/errors** across the TypeScript frontend and Rust backend:

| # | File | Type | Root Cause |
|---|------|------|------------|
| 1 | `src/App.tsx:6` | TS6133 | `isBusy` imported from `./lib/workspaceStatus` but never read |
| 2 | `src/App.tsx:291` | TS6133 | `isMac` function declared but never called |
| 3 | `src/components/ChatPanel.tsx:34` | TS6133 | `AttachmentData` type imported but never used |
| 4 | `src/components/TasksPanel.tsx:47` | TS2769 | `next[t.status]` returns `string`, but `TaskItem.status` is the union `"todo" \| "doing" \| "done"` |
| 5 | Vite build | Warning | `@tauri-apps/plugin-dialog` imported both statically (`ipc.ts`) and dynamically (`ChatPanel.tsx`) |
| 6 | Vite build | Warning | Monaco editor chunks > 500KB (inherent size, not fixable) |
| 7 | `src-tauri/src/agent/session.rs:812` | `dead_code` | `cost_for` function never called |

**User decisions confirmed:**
- `cost_for` → **remove** (not `#[allow(dead_code)]`)
- Chunk size warnings → **suppress** via `chunkSizeWarningLimit` in `vite.config.ts`

## Solution Design

### 1. Remove unused `isBusy` import (`App.tsx`)
- **Target:** `src/App.tsx`, line 6 import statement
- **Mutation:** Remove `isBusy` from the destructured import from `./lib/workspaceStatus`
- **Why:** TS6133 — declared but never read

### 2. Remove unused `isMac` function (`App.tsx`)
- **Target:** `src/App.tsx`, line 291
- **Mutation:** Delete the `const isMac = () => ...` line
- **Why:** TS6133 — declared but never read

### 3. Remove unused `AttachmentData` type import (`ChatPanel.tsx`)
- **Target:** `src/components/ChatPanel.tsx`, line 34
- **Mutation:** Remove `type AttachmentData,` from the import from `../lib/ipc`
- **Why:** TS6133 — declared but never read. The component uses inline types for attachment objects, not `AttachmentData`.

### 4. Fix status type mismatch (`TasksPanel.tsx`)
- **Target:** `src/components/TasksPanel.tsx`, lines 42–43
- **Mutation:** Change the `next` record to have the explicit type `Record<string, TaskItem["status"]>`:
  ```ts
  const next: Record<string, TaskItem["status"]> = {
    todo: "doing",
    doing: "done",
    done: "todo",
  };
  ```
- **Why:** TS2769/TS2345 — `next[t.status]` returns `string`, but `TaskItem` expects `"todo" | "doing" | "done"`

### 5. Fix dynamic+static import conflict (`plugin-dialog`)
- **Target:** `src/lib/ipc.ts` and `src/components/ChatPanel.tsx`
- **Mutation:**
  - Add `pickFiles()` function to `src/lib/ipc.ts` (uses the existing static import of `open`):
    ```ts
    export async function pickFiles(): Promise<string[]> {
      const selected = await open({ multiple: true });
      if (!selected) return [];
      return Array.isArray(selected) ? selected : [selected];
    }
    ```
  - In `ChatPanel.tsx`, replace the inline `await import("@tauri-apps/plugin-dialog")` call with `await pickFiles()`
  - Remove the dynamic import block
- **Why:** Vite warns when a module is both statically and dynamically imported — it prevents proper chunking. Consolidating to static-only resolves this.

### 6. Suppress chunk size warnings (`vite.config.ts`)
- **Target:** `vite.config.ts`
- **Mutation:** Add `build.chunkSizeWarningLimit` (e.g., `2000` for 2000KB):
  ```ts
  build: {
    chunkSizeWarningLimit: 2000, // Monaco editor chunks are inherently large
  }
  ```
- **Why:** Monaco editor produces ~5MB chunks; this is expected and unavoidable

### 7. Remove dead Rust function (`session.rs`)
- **Target:** `src-tauri/src/agent/session.rs`, lines ~812–817
- **Mutation:** Remove the entire `cost_for` function and the comment above it
- **Why:** `#[warn(dead_code)]` — function has zero call sites. User chose removal over `#[allow]`.

## Verification Plan

1. **`tsc --noEmit`** — must exit 0 with zero errors
2. **`pnpm run build`** (vite build) — must exit 0 with zero warnings
3. **`cargo check`** in `src-tauri/` — must exit 0 with zero warnings

## Risks

- **Low risk on all changes** — these are all dead code removal, type narrowing, and warning suppression. No behavioral changes.
- The `pickFiles()` extraction slightly changes the file picker call path, but the behavior is identical.
