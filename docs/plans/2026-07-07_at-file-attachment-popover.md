# @ File Attachment Popover — Solution Design

## 1. Context / Problem Statement

**Request:** When the user types `@` in the chat input textarea, a popover should appear near the caret showing a fuzzy-searchable list of all project files and folders. On selection, the chosen file/folder path is inserted as inline text (e.g. `@src/components/ChatPanel.tsx`).

**Current state:** The chat input (`ChatPanel.tsx:759-800`) is a plain `<textarea>` with zero `@` handling. No autocomplete, no mention system. File browsing exists as a lazy tree (`FileTree.tsx`) using one-level `listDir` IPC, but no recursive flat list exists.

**Environment:** Solid.js + TypeScript frontend, Tauri v2 + Rust backend, Tailwind CSS v4. Monaco editor is present but used only for `DiffViewer`.

**All decisions below were CONFIRMED by user interview.**

| Decision | Choice |
|---|---|
| @ behavior | `@` stays as text; selection inserts inline path |
| Data source | Recursive walk of all project files, client-side |
| Fuzzy algorithm | `fuse.js` |
| Popover position | Near caret (VS Code-style dropdown) |
| Scope | Files AND folders |
| Trigger timing | Immediately on `@`, filter as user types |
| Item display | Relative path only, no icons/metadata |

## 2. Goal (Definition of Done)

Typing `@` in the chat textarea opens a keyboard-navigable popover dropdown positioned at the caret. The popover shows all workspace files/folders, filters them via fuse.js fuzzy search as the user types, and on selection inserts the relative path inline (e.g. `@src/components/ChatPanel.tsx`) at the `@` cursor position. Pressing Esc or deleting the `@` closes the popover.

## 3. Key Findings (Prova Real)

- **Finding:** The Tauri backend has no recursive file walker. `commands/fs.rs:list_dir` is depth-1 only.
  - *Method:* Read `src-tauri/src/commands/fs.rs` via subagent exploration.
  - *Proof:* Subagent report confirmed `list_dir` signature `(path: String) -> Vec<DirEntry>` with one-level listing.

- **Finding:** Solid.js `Portal` is already used in `TasksPanel.tsx:120` for a floating popover — same pattern we need.
  - *Method:* Read `TasksPanel.tsx` via subagent exploration.
  - *Proof:* `import { Portal } from "solid-js/web"` at top of file; popover rendered at `position: fixed; right: 48px; z-index: 9999`.

- **Finding:** Existing modals use `onMount`/`onCleanup` for ESC key listeners and click-outside handling. We will follow the same pattern.
  - *Method:* Inspected `ContextWarning.tsx` and `ChatPanel.tsx` SubagentModal.
  - *Proof:* `document.addEventListener("keydown", handleKey)` in `onMount`, removed in `onCleanup`.

- **Finding:** The `ChatPanel` textarea has `inputRef` (line 397) giving us DOM access for caret positioning.
  - *Method:* Subagent read `ChatPanel.tsx`.
  - *Proof:* `let inputRef!: HTMLTextAreaElement;` at line 397.

- **Finding:** `pnpm` is the package manager. `fuse.js` needs to be added.
  - *Method:* Read `package.json`.
  - *Proof:* `"packageManager": "pnpm@11.9.0+..."` and `pnpm-lock.yaml` present.

- **Finding:** The app has an `Icon` component (`Icon.tsx`) with ~30+ SVG icons. No file-type-specific icons exist, but this is not needed per user decision.
  - *Method:* Subagent read `Icon.tsx`.
  - *Proof:* File contains inline SVG path data for paperclip, brain, send, etc.

## 4. Authoritative Inputs

| Input | Source | Status |
|---|---|---|
| `fuse.js` for fuzzy search | User decision, confirmed | Will be added as dependency |
| Recursive file list via Tauri command | User decision, confirmed | New backend command needed |
| Popover near caret | User decision, confirmed | Mirror-div positioning technique |
| Include folders | User decision, confirmed | — |
| Open immediately on `@` | User decision, confirmed | — |
| Relative path display only | User decision, confirmed | — |

## 5. Changes (Steps)

### Step 1: Rust Backend — Add `walk_dir` Tauri command

- **Target:** `src-tauri/src/commands/fs.rs` + register in `src-tauri/src/lib.rs`
- **Mutation:** Add a new `#[tauri::command] fn walk_dir(root: String) -> Vec<WalkEntry>` that recursively walks the directory tree respecting `.gitignore`, returning a flat list of relative paths with `is_dir` flag.
- **Why:** The frontend needs a complete file list to feed fuse.js. The existing `list_dir` is depth-1 only.
- **Constraints:**
  - Use the `ignore` crate (already present via `list_dir`'s gitignore support) or `walkdir`.
  - Respect `.gitignore` (use `ignore::WalkBuilder`).
  - Skip hidden files/dirs (`.` prefix) by default, matching existing `list_dir` behavior.
  - Return relative paths from `root`.
  - Max depth: entire tree (no artificial limit, but respect `.gitignore` for `node_modules`, `target`, etc.).
  - Idempotent: pure read-only operation.

### Step 2: Frontend — Add `fuse.js` dependency

- **Target:** `package.json`, run `pnpm add fuse.js`
- **Mutation:** Add `fuse.js` and its type definitions.
- **Why:** User selected fuse.js for fuzzy search.
- **Constraints:** Use `pnpm` (the project's package manager).

### Step 3: Frontend — IPC bridge for `walk_dir`

- **Target:** `src/lib/ipc.ts`
- **Mutation:** Add a `walkDirectory(path: string): Promise<WalkEntry[]>` function that calls `invoke("walk_dir", { root: path })`. Define `WalkEntry` type: `{ path: string; is_dir: boolean }`.
- **Why:** Bridges the new backend command to the frontend type-safely.

### Step 4: Frontend — File index state management

- **Target:** `src/lib/fileIndex.ts` (new file)
- **Mutation:** Create a shared reactive store/signal for the flat file list. Export:
  - `fileIndex: Signal<string[]>` — the flat list of paths (files + folders).
  - `setFileIndex(index: string[])` — setter.
  - `loadFileIndex(workspacePath: string): Promise<void>` — calls `walkDirectory` and populates the signal.
- **Why:** Multiple components (App, ChatPanel) need access to the file list. Keeps it decoupled.
- **Do NOT change:** The `workspaceStatus.ts` store is separate; don't mix concerns.

### Step 5: Frontend — `FileMentionPopover` component

- **Target:** `src/components/FileMentionPopover.tsx` (new file)
- **Mutation:** A Solid.js component that:
  - **Props:** `query: string`, `fileList: string[]`, `position: { top: number; left: number }`, `onSelect: (path: string) => void`, `onClose: () => void`.
  - **Fuse.js initialization:** `createMemo` that builds a `Fuse` instance from `fileList` with config: `{ keys: [], threshold: 0.4, distance: 100, includeScore: true }`. When `query` is empty, show all items; when non-empty, run `.search(query)`.
  - **Rendering:** Portaled dropdown (`Portal` from `solid-js/web`) at `position`, with `max-h-[240px] overflow-y-auto`, styled like existing modals (bg-surface, border-border-subtle, rounded-lg, shadow-lg). Max 20 results visible.
  - **Keyboard:** ↑/↓ to navigate highlighted index, Enter to select, Esc to close.
  - **Mouse:** Click to select, hover to highlight.
  - **Item format:** Just relative path string (e.g. `src/components/ChatPanel.tsx`), no icons.
  - **Lifecycle:** `onMount` adds global key listeners; `onCleanup` removes them.
- **Why:** Encapsulates all popover logic — search, rendering, keyboard nav, selection.
- **Constraints:** Follow existing patterns: `Portal` + `position: fixed` (like `TasksPanel`), ESC/click-outside (like `ContextWarning`), `z-50`.

### Step 6: Frontend — @ detection and caret positioning in `ChatPanel`

- **Target:** `src/components/ChatPanel.tsx`
- **Mutation:**
  - Add a `mentionQuery` signal (`""` when not active, the text after `@` when active).
  - Add a `mentionPosition` signal (`null` when closed, `{ top, left }` when open).
  - Replace the plain `onInput={(e) => setInput(e.currentTarget.value)}` (line 775) with a handler that:
    1. Gets `textarea.value`, `textarea.selectionStart`.
    2. Scans backwards from `selectionStart` to find the nearest unclosed `@` (a `@` that is not preceded by a non-whitespace, i.e., at word boundary or start of input, and no space between `@` and cursor).
    3. If found: extract query (text between `@` and cursor), compute caret pixel position (mirror div technique), set `mentionQuery` and `mentionPosition`.
    4. If not found: close popover (set `mentionQuery` to `""`).
  - **Caret positioning helper:** Create a hidden `<div>` (mirror) with identical font/size/padding/width as the textarea. Copy text from start to caret position into it. Append a `<span>` marker. Use `marker.getBoundingClientRect()` relative to textarea's rect to compute `top`/`left` for the popover.
  - Render `<FileMentionPopover>` conditionally when `mentionQuery` is not `""`.
  - On selection: replace the `@query` portion of the input with `@selectedPath`, update `input` signal, close popover, focus textarea at end of inserted path.
  - On Esc: close popover (already handled inside popover).
- **Why:** This is the core integration — detecting `@`, managing popover state, and inserting selected paths.
- **Do NOT change:** The `handleKeyDown` for Enter (send) — when popover is open, Enter should select, not send. Modify `handleKeyDown`: if popover is open, Enter should be swallowed by the popover (don't call `send()`), and the popover's own key handler fires first.

### Step 7: Frontend — Wire file index loading into `App.tsx`

- **Target:** `src/App.tsx`
- **Mutation:** After `openWorkspace` succeeds, call `loadFileIndex(workspacePath)` to populate the flat file list. Pass the `fileIndex` signal down to `ChatPanel`.
- **Why:** The file list must be available when a workspace is active.
- **Constraints:** Only load when a workspace is actually open (not on EmptyState screen).

## 6. Verification Plan

### Dry-run & Build
1. `pnpm build` — must compile with zero errors.
2. `cargo build` in `src-tauri/` — must compile with zero errors.

### Backend: `walk_dir` command
3. Run the app, open a workspace. Call `walk_dir` from dev tools console (or a quick test script). Verify:
   - Returns a flat list of paths.
   - All paths are relative to workspace root.
   - `.gitignore` is respected (`node_modules/` excluded, etc.).
   - Hidden files (`.git/`, `.DS_Store`) excluded.
   - Both files and directories present.
   - `is_dir` flag is correct.

### Popover visual & UX
4. Type `@` in chat input → popover appears immediately below the caret.
5. Popover shows all workspace files/folders when query is empty.
6. Type `@Chat` → list filters to paths matching "Chat" fuzzily.
7. Press ↑/↓ → highlight moves. Press Enter → path inserted as `@path`.
8. Press Esc → popover closes, `@` text remains.
9. Delete the `@` → popover closes.
10. Click item with mouse → path inserted, popover closes.
11. When popover is open, pressing Enter does NOT send the message.
12. When popover is closed, pressing Enter sends the message as before (regression).

### Integration
13. With a workspace open, the file list loads. Close and re-open workspace → list reloads correctly.
14. Empty workspace (no files) → `@` opens empty popover with "No files" message (graceful).

### Regression
15. Normal chat flow (send message, receive response, steering) works unchanged.
16. Paperclip file attachment still works.
17. Drag-and-drop still works.
18. Mode toggle (brain/builder) still works.

## 7. Risks

- **Large workspace performance:** A project with 50k+ files could make fuse.js initialization slow. Mitigation: debounce fuse instance creation, skip `node_modules` and `.git` via `.gitignore` (already respected).
- **Mirror div accuracy:** Caret positioning with mirror div is reliable but must match textarea styling exactly (font, padding, line-height, word-wrap). Mitigation: copy computed styles from textarea to mirror div.
- **fuse.js bundle size:** ~5KB gzip. Acceptable given user confirmation.

---

## Tasks Summary

1. **Rust: `walk_dir` command** — recursive directory walker in `commands/fs.rs`
2. **Install fuse.js** — `pnpm add fuse.js`
3. **IPC bridge** — `walkDirectory()` in `ipc.ts` + `WalkEntry` type
4. **File index state** — `fileIndex.ts` shared signal
5. **FileMentionPopover component** — portaled dropdown with fuse.js + keyboard nav
6. **ChatPanel @ integration** — detection, caret positioning, popover wiring, Enter override
7. **App.tsx wiring** — load file index on workspace open, pass to ChatPanel
