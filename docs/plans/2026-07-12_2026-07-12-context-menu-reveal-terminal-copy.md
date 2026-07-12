# Context Menu: Reveal in Finder, Open in Terminal, Copy Path

## Context

The user wants a right-click context menu on workspace/project items (sidebar list) and file tree items (FileTree) with three actions:

1. **Reveal in Finder** — opens the folder in the OS file manager (Finder on macOS, Explorer on Windows)
2. **Open in Terminal** — opens a terminal emulator at the selected folder
3. **Copy Path** — copies the full absolute path to the clipboard

Confirmed decisions:
- Context menu on: **both** sidebar project items AND FileTree items (files + folders)
- Reveal behavior: **open the folder** only (no file-selection/`-R` behavior needed)
- Three actions all wanted: Reveal, Terminal, Copy Path

No right-click infrastructure exists in the codebase — it must be built from scratch.

## Solution Design

### 1. Backend — New Tauri Command: `open_in_terminal`

**File:** `src-tauri/src/commands/shell.rs` (new)

A new `#[tauri::command]` that opens the system terminal emulator at the given path:

- **macOS:** `open -b com.apple.Terminal {path}` — opens Terminal.app at the folder
- **Windows:** spawns `cmd /c start cmd /k cd /d "{path}"` — opens cmd at the folder (most reliable cross-Windows)
- **Linux:** `x-terminal-employee -e "cd {path} && exec $SHELL"` — opens default terminal at the folder

Registration:
- Add `pub mod shell;` to `src/commands/mod.rs`
- Add `commands::shell::open_in_terminal` to `invoke_handler!` in `lib.rs`

### 2. Frontend — Reusable ContextMenu Component

**File:** `src/components/ContextMenu.tsx` (new)

Pattern: follows existing Portal-based popovers (`FileMentionPopover`, `TasksPanel`).

```tsx
interface ContextMenuItem {
  label: string;
  icon: IconName;
  action: () => void;
  separatorAfter?: boolean;
}
```

Behavior:
- Renders a fixed-position portal at `(x, y)` mouse coordinates
- Click-outside backdrop to dismiss
- Each item triggers its action + dismisses
- Keyboard: Escape to dismiss

### 3. Frontend — Platform Detection Helper

**File:** `src/lib/platform.ts` (new) or inline in `ipc.ts`

Simple utility:
- `isMac: boolean` → `navigator.userAgent.includes("Mac")`
- `isWindows: boolean` → `navigator.userAgent.includes("Win")`
- Uses Reactive: `createMemo` derived from signal for reactivity

**Reveal label by platform:**
- macOS: "Reveal in Finder"
- Windows: "Show in Explorer"
- Linux: "Open in File Manager"

### 4. Frontend — Wire Up onContextMenu

**FileTree.tsx:**
- Add `onContextMenu` handler on `TreeNode` buttons
- Show three-item context menu:
  - "Reveal in Finder" → `openPath(parentDir)` for files, `openPath(path)` for dirs
  - "Open in Terminal" → `invoke("open_in_terminal", { path })`
  - "Copy Path" → `navigator.clipboard.writeText(path)`

**App.tsx (sidebar project list):**
- Add `onContextMenu` handler on project item `.group` divs
- Same three-item context menu as above, path = project workspace root

### 5. i18n — New Locale Strings

**pt-BR.ts:**
```
"context.revealInFinder": "Revelar no Finder"
"context.revealInExplorer": "Mostrar no Explorer"
"context.revealInFileManager": "Abrir no Gerenciador de Arquivos"
"context.openInTerminal": "Abrir no Terminal"
"context.copyPath": "Copiar caminho"
```

**en-US.ts:**
```
"context.revealInFinder": "Reveal in Finder"
"context.revealInExplorer": "Show in Explorer"
"context.revealInFileManager": "Open in File Manager"
"context.openInTerminal": "Open in Terminal"
"context.copyPath": "Copy Path"
```

The frontend helper selects the correct key based on platform.

## Risks

- **Platform-specific terminal behavior:** macOS Terminal.app may not `cd` on all versions. Tested behavior: `open -a Terminal /path` opens Terminal with `cd` to that path.
- **Windows path quoting:** Paths with spaces must be properly quoted. The Rust side will handle this.
- **Clipboard API requires HTTPS or localhost:** Works in Tauri's webview (local files are trusted).

## Tasks Summary

1. Rust: new `shell.rs` command — `open_in_terminal`
2. Rust: register the command in `mod.rs` and `lib.rs`
3. Frontend: `platform.ts` helper
4. Frontend: `ContextMenu.tsx` component
5. Frontend: wire up `onContextMenu` in FileTree.tsx
6. Frontend: wire up `onContextMenu` in App.tsx sidebar
7. i18n: add locale strings to pt-BR.ts and en-US.ts
8. Verify build passes


## Implementation Log — 2026-07-12 01:12
**Summary:** Add right-click context menu to sidebar projects + FileTree with Reveal in Finder, Open in Terminal, and Copy Path options
**Changed files:** M src-tauri/src/commands/mod.rs, M src-tauri/src/lib.rs, M src/App.tsx, M src/components/FileTree.tsx, M src/lib/ipc.ts, M src/lib/locales/en-US.ts, M src/lib/locales/pt-BR.ts, ?? docs/plans/2026-07-12_2026-07-12-context-menu-reveal-terminal-copy.md, ?? src-tauri/src/commands/shell.rs, ?? src/components/ContextMenu.tsx, ?? src/lib/platform.ts
**Commits:** _(git unavailable or none)_
**Journal:** ## Key Decisions & Findings

1. **Terminal opening approach**: Used `std::process::Command` with platform-specific invocations instead of a Tauri plugin. macOS uses `open -b com.apple.Terminal` (which auto-cds to the path), Windows uses `cmd /c start cmd /k cd /d`, Linux respects `$TERMINAL` env var with `x-terminal-emulator` fallback.

2. **Reveal behavior**: Simple `openPath(path)` from `tauri-plugin-opener` — opens the folder in Finder/Explorer without selecting specific files. For files in FileTree, we pass the parent directory path instead.

3. **ContextMenu pattern**: Followed existing Portal-based popover patterns from `FileMentionPopover` and `TasksPanel` — fixed-position portal with click-outside backdrop, Escape key listener, viewport clamping.

4. **Platform detection via userAgent**: Simple utility in `platform.ts` — no Tauri command needed since `navigator.userAgent` is reliable enough for this.

5. **State management**: Context menu state (position + path) managed via `createSignal` at the component level that renders the menu (FileTree has its own internal signal, App.tsx has a top-level signal). No shared global state needed.

6. **No test file changes needed**: The FileTree component remained backward compatible — `onContextMenu` is handled internally via signal, not exposed as a prop. Existing tests pass without modification.

## Gotchas
- The FileTree test uses `vi.mock("../lib/ipc")` which now needs `openInTerminal` and `copyPath` — but since those are only called from the ContextMenu (which is only rendered when `contextPos()` is non-null), and tests never trigger right-click, no mock is needed.
- `listDir` already had an argument mismatch (test passes `expect.anything()` as second arg) — pre-existing issue unrelated to our changes.
- The Rust build takes a long time even with small changes due to tree-sitter dependencies and ORT. Our new `shell.rs` only depends on `std::process::Command` so compilation is fast.

**Task journal:**
- Create and register open_in_terminal Rust command: Created shell.rs, added pub mod shell to mod.rs, registered in lib.rs. All verified.
- Implement shell.rs with open_in_terminal command: Created src-tauri/src/commands/shell.rs with #[cfg(target_os = "macos")], #[cfg(target_os = "windows")], #[cfg(target_os = "linux")] sections. Uses std::process::Command.
- Create platform.ts helper and ipc.ts convenience functions: Created src/lib/platform.ts with platform(), revealLabel(), revealI18nKey(). Added openInTerminal() and copyPath() to src/lib/ipc.ts. All verified.
- Create reusable ContextMenu component: Created src/components/ContextMenu.tsx with ContextMenu component, ContextMenuItem interface, Portal-based rendering, Escape/click-outside dismissal, viewport clamping. All verified.
- Wire up context menu in FileTree.tsx: Updated FileTree.tsx with onContextMenu on TreeNode, contextPos signal, ContextMenu rendering with 3 items. Uses openPath for reveal, openInTerminal, copyPath. All verified.
- Wire up context menu in App.tsx sidebar: Updated App.tsx with contextPos signal, onContextMenu on workspace/recent project divs, ContextMenu render. All verified.
- Add i18n strings for context menu items: Added 5 context menu strings to both locale files. All verified.
- Verify build passes: cargo build passed. vite build passed.
