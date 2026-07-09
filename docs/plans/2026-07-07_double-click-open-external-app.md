# Solution Design: Double-click to open files in external apps

## Context / Problem Statement

When browsing files in the "Browse Files" panel (`FileTree` component), clicking a file currently only highlights it visually (sets `selectedFile` signal). There is no way to open files in external applications like VSCode, Finder, Preview, etc. from within the file browser.

**User request:** Double-clicking a file in the file tree should open it in the OS default application.

**What the user CONFIRMED:**
- Open with OS default app (not extension-to-app mapping)
- Single-click preserves current highlight behavior
- Double-click triggers the external open
- Silent failure if OS cannot open (no toast/error feedback)

## Solution Design

### Overview

The `@tauri-apps/plugin-opener` plugin is **already fully installed** (npm dep, Cargo dep, Rust `init()`, capability `"opener:default"` granted). It exposes `openPath(path: string): Promise<void>` which opens a file path with the system's default application. We just need to wire it to the double-click event in `FileTree`.

### Changes

#### 1. `src/lib/ipc.ts` — Add a thin wrapper (optional but clean)

Add a simple `openExternal` function:

```typescript
import { openPath } from "@tauri-apps/plugin-opener";

function openExternal(path: string): void {
  openPath(path).catch(() => {});
}
```

**Why:** Keeps all IPC/Tauri calls in one module, and the `.catch(() => {})` provides silent error handling as requested.

**Alternative (more ponytail):** Import `openPath` directly in `App.tsx`. Slightly fewer files touched but less consistent with the project's pattern of having all IPC in `ipc.ts`. I'll go with the wrapper in `ipc.ts` since it matches the existing pattern (all Tauri plugin imports are in `ipc.ts`).

#### 2. `src/components/FileTree.tsx` — Add double-click handler

**FileTree component** (`export const FileTree`):
- Add new prop: `onOpenExternal: (path: string) => void`
- Pass it through to `TreeNode`

**TreeNode component** (`const TreeNode`):
- Add new prop: `onOpenExternal: (path: string) => void`
- Add `onDblClick` handler to the `<button>` element: calls `props.onOpenExternal(props.entry.path)` **only for files** (not directories)
- Pass `onOpenExternal` to child `TreeNode` in the `<For>` loop

**Why double-click on the `<button>`:** The entire row is already a `<button>` element. Adding an `onDblClick` handler here is the minimal change. Browsers handle the distinction between single-click and double-click natively.

#### 3. `src/App.tsx` — Wire the handler

- Import `openExternal` from `"./lib/ipc"` (or add to the existing import)
- Create a handler: `const handleOpenExternal = (path: string) => { openExternal(path); };`
- Pass `onOpenExternal={handleOpenExternal}` to `<FileTree>`

### What does NOT change

- No new npm dependencies or Cargo crates
- No capability/permission changes
- No changes to directory click behavior
- No visual changes to the file tree at all
- The `selectedFile` signal and highlight behavior remain untouched
- The `onOpenFile` / `setSelectedFile` wire stays as-is

## Risks

- **Low risk.** The change is purely additive (new prop, new handler). Existing single-click behavior is unchanged.
- Double-click timing is handled by the browser — the first click of a double-click will still trigger `onClick` (highlight), then the second triggers `onDblClick` (open external). This is expected and desirable behavior.
- `openPath` is async but fire-and-forget — we don't await it, so double-click feels instant.

## Tasks Summary

1. Add `openExternal` wrapper in `src/lib/ipc.ts`
2. Add `onOpenExternal` prop to `TreeNode` and `FileTree` components, wire `onDblClick`
3. Import and wire the handler in `App.tsx`
4. Test: open the app, browse files, double-click a `.tsx` file → opens in VSCode (if default)
