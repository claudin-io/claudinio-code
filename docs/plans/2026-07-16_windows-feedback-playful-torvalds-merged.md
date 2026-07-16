# Address Windows user feedback: context menu, idle memory, "Open in IDE" (+ `--goto` support)

## Context

A Windows user reported three issues:
1. **Right-click shows a Chrome-like context menu** ("send this tab to other devices") — Tauri uses the system WebView2 (Chromium-based) on Windows, and the default context menu was never suppressed.
2. **~9.3GB memory at idle** with many "Git for Windows" / "Console Window Host" child processes. Root cause: LSP servers (`typescript-language-server` + `rust-analyzer`) spawn unconditionally on every `open_workspace`, rust-analyzer alone holds 1–4GB and shells out to git. Additional `CREATE_NO_WINDOW` gaps in MCP stdio, `rg` spawn, and LSP client spawns cause conhost pileup.
3. **Feature request:** buttons to open the project/file in VS Code or Cursor.

The original plan covered all three. This merged plan adds `--goto file:line` support to Item 3 (per user decision), cleans up a duplicated `CREATE_NO_WINDOW` constant in `shell.rs`, and adds minor file cleanup.

## Solution Design

### Item 1 — Suppress default WebView2 context menu
Add a `contextmenu` event listener in `src/index.tsx` that calls `preventDefault()` on right-click, except on editable elements and selected text. Gated to production builds only so dev tools remain usable.

### Item 2 — Idle memory + stray Windows processes

**2a. Conditional LSP spawn (biggest win):** Gate LSP server startup on project markers: `start_tsserver` only if `package.json`/`tsconfig.json`/`jsconfig.json` exists at workspace root; `start_rust_analyzer` only if `Cargo.toml` exists at workspace root.

**2b. Apply `no_window` to remaining spawns:** Add `procutil::no_window` / `procutil::no_window_tokio` to LSP client spawn, MCP stdio spawn, and `rg` spawn. Also clean up `shell.rs` which duplicates the `CREATE_NO_WINDOW` constant instead of importing from `procutil`.

**2c. Settings toggle `code_intel_enabled`:** Add a boolean config field that skips ALL LSP startup and ALL indexing/embedding when disabled. When toggled off, the user must re-open the workspace for it to take effect — no hot-reload complexity. No persistent status indicator needed (user knows they turned it off).

**2d. Deduplicate git_branch polling:** `App.tsx` and `GitIndicator.tsx` both poll `git_branch` every 30s. Keep the App.tsx poll, pass branch as prop into `GitIndicator`, delete GitIndicator's own `git_branch` interval.

### Item 3 — "Open in IDE" (VS Code / Cursor) + `--goto file:line`

**Detection:** New `detect_ides` Tauri command checks for `code` and `cursor` on PATH (using login-shell PATH helper). On macOS also checks `/Applications/`. Returns list of detected IDE IDs.

**Open:** New `open_in_ide` Tauri command that takes `path`, `ide`, and optional `goto_line`. Per-platform:
- macOS without goto: `open -a "Visual Studio Code" <path>` / `open -a "Cursor" <path>`
- With goto (all platforms): `code --goto <file>:<line>:<col>` / `cursor --goto <file>:<line>:<col>`
- Windows without goto: `cmd /c code <path>` with `no_window`
- Linux without goto: `Command::new("code"/"cursor")` with login PATH

**Config:** `AgentConfig.preferred_ide: Option<String>` ("vscode" | "cursor"). Settings modal dropdown populated from `detect_ides()`, shown only when at least one IDE detected.

**Frontend — Header:** Icon button before settings gear, opens `activeWorkspace()` in `preferred_ide ?? availableIdes()[0]`. Shown only when IDEs detected.

**Frontend — App-level context menu:** Per-IDE items ("Open in VS Code" / "Open in Cursor") alongside existing "Reveal in Finder", "Open in Terminal", "Copy Path".

**Frontend — FileTree context menu:** Per-IDE items opening the specific **file**. When the file is currently open in an editor and cursor position is known, pass line number for `--goto`.

**Frontend — `--goto` cursor tracking:** Track `activeEditorCursor: {path: string, line: number} | null` signal in App.tsx. `FileEditorModal` and `ContentViewerModal` get an `onCursorLineChange` callback prop that fires from Monaco's `onDidChangeCursorPosition`. When building FileTree context menu items, if the right-clicked file matches `activeEditorCursor.path`, pass the line to `open_in_ide`.

## Risks

| Risk | Mitigation |
|------|-----------|
| **2a — false negative on root markers:** User has a monorepo where `Cargo.toml` is in a subdirectory, not root. rust-analyzer won't start automatically. | Acceptable trade-off. User can still open the subdirectory as a workspace. LSP servers already initialize with workspace-root URI. |
| **2c — toggle requires workspace reopen:** UX friction if user toggles mid-session expecting instant effect. | Acceptable. "Reopen workspace for changes to take effect" note in the setting label. |
| **3 — `code`/`cursor` CLI not in PATH on macOS:** macOS users who installed via Homebrew may not have the CLI. `detect_ides` checks both PATH and `/Applications/`. Without goto, `open -a` works even without CLI. With goto, CLI is required — fall back to opening without goto. |
| **3 — Windows `cmd /c code`:** `code.cmd` is a batch file, can't be spawned directly. `cmd /c` resolves it. Tested pattern from `shell.rs`. |
| **3 — macOS `open -a` with `--goto`:** `open -a` doesn't support `--goto`. Must use `code`/`cursor` CLI for goto. |

## Non-goals

- LSP idle-shutdown timer (deferred — nice-to-have but not the reported symptom)
- Per-query full-table embedding optimization in `db.rs:582` (deferred — transient memory churn, not idle memory)
- Windows-specific `--goto` smoke test (manual QA on next release)
- No `code_intel_enabled` hot-reload (requires workspace reopen — simpler and safer)
- No persistent status indicator for disabled code intel (user decision)
- No `open -a` with `--goto` (must use CLI for goto — design decision)

## Low-Level Design

### Files to touch

| File | Change | Item |
|------|--------|------|
| `src/index.tsx` | Add contextmenu event listener | 1 |
| `src-tauri/src/lsp/manager.rs` | Gate LSP spawns on root markers | 2a |
| `src-tauri/src/lsp/client.rs` | Add `no_window` to `Command` spawn | 2b |
| `src-tauri/src/agent/mcp.rs` | Add `no_window_tokio` to stdio `Command` | 2b |
| `src-tauri/src/agent/tools/grep.rs` | Add `no_window` to `rg` spawn | 2b |
| `src-tauri/src/commands/shell.rs` | Replace local `CREATE_NO_WINDOW` with `procutil::no_window` | 2b |
| `src-tauri/src/agent/provider.rs` | Add `code_intel_enabled` field to `AgentConfig` | 2c |
| `src-tauri/src/commands/code_intel.rs` | Gate LSP + indexing on `code_intel_enabled` | 2c |
| `src/App.tsx` | Add `code_intel_enabled` checkbox in settings, git dedup, IDE UI, cursor tracking | 2c,2d,3 |
| `src/components/GitIndicator.tsx` | Remove own `git_branch` interval, accept `branch` prop | 2d |
| `src/lib/ipc.ts` | Add `detectIdes()` and `openInIde()` wrappers | 3 |
| `src/lib/locales/en-US.ts` | Add new locale keys | 2c,3 |
| `src/lib/locales/pt-BR.ts` | Add new locale keys | 2c,3 |
| `src/components/FileTree.tsx` | Add IDE items to context menu, pass `availableIdes` + `activeEditorCursor` props | 3 |
| `src/components/FileEditorModal.tsx` | Add `onCursorLineChange` prop, fire from Monaco cursor event | 3 |
| `src/components/ContentViewerModal.tsx` | Add `onCursorLineChange` prop, fire from Monaco cursor event | 3 |
| `src-tauri/src/commands/ide.rs` | **New file:** `detect_ides` and `open_in_ide` commands | 3 |
| `src-tauri/src/commands/mod.rs` | Register `ide` module | 3 |
| `src-tauri/src/lib.rs` | Register `detect_ides` and `open_in_ide` in `generate_handler!` | 3 |

### Detailed implementation

---

#### Item 1 — Suppress default WebView2 context menu

**File:** `src/index.tsx` (currently 7 lines)

Add BEFORE the `render()` call:
```tsx
if (!import.meta.env.DEV) {
  document.addEventListener("contextmenu", (e) => {
    const t = e.target as HTMLElement | null;
    if (t?.closest("input, textarea, [contenteditable='true'], [contenteditable='']")) return;
    if (window.getSelection()?.toString()) return;
    e.preventDefault();
  });
}
```

**Rationale:** Editable elements and text selections keep native behavior (copy/paste/spellcheck). Existing custom context menus (App.tsx:1508, FileTree.tsx:92) already call `preventDefault()` and are unaffected.

---

#### Item 2a — Conditional LSP spawn

**File:** `src-tauri/src/lsp/manager.rs`, function `start_for_workspace` (lines 23–30)

Current code unconditionally calls both:
```rust
pub fn start_for_workspace(&mut self, workspace_root: &str) -> Result<(), String> {
    self.start_tsserver(workspace_root)?;
    self.start_rust_analyzer(workspace_root)?;
    Ok(())
}
```

New code:
```rust
pub fn start_for_workspace(&mut self, workspace_root: &str) -> Result<(), String> {
    use std::path::Path;
    let root = Path::new(workspace_root);
    // TypeScript: only if a JS/TS project marker exists at root
    if root.join("package.json").exists()
        || root.join("tsconfig.json").exists()
        || root.join("jsconfig.json").exists()
    {
        self.start_tsserver(workspace_root)?;
    }
    // Rust: only if Cargo.toml exists at root
    if root.join("Cargo.toml").exists() {
        self.start_rust_analyzer(workspace_root)?;
    }
    Ok(())
}
```

**Rationale:** Root-only detection is correct — servers are initialized with workspace-root URI. If a monorepo has subdirectories with their own markers, the user opens that subdirectory as a workspace.

---

#### Item 2b — `no_window` on remaining spawns + cleanup

**File A:** `src-tauri/src/lsp/client.rs`, lines 35–41

Current:
```rust
let mut process = Command::new(server_path)
    .stdin(Stdio::piped())
    .stdout(Stdio::piped())
    .stderr(Stdio::null())
    .spawn()
    .map_err(|e| format!("spawn {server_path}: {e}"))?;
```

New — add `use crate::commands::procutil;` at top and restructure:
```rust
let mut cmd = Command::new(server_path);
cmd.stdin(Stdio::piped())
    .stdout(Stdio::piped())
    .stderr(Stdio::null());
procutil::no_window(&mut cmd);
let mut process = cmd.spawn()
    .map_err(|e| format!("spawn {server_path}: {e}"))?;
```

**File B:** `src-tauri/src/agent/mcp.rs`, line 177 (inside `.configure(|c| { ... })` closure)

Current:
```rust
let cmd = tokio::process::Command::new(command).configure(|c| {
    c.args(args);
    for (k, v) in env {
        c.env(k, v);
    }
    if let Some(dir) = &workdir {
        c.current_dir(dir);
    }
});
```

New — add `use crate::commands::procutil;` and inside the closure add:
```rust
let cmd = tokio::process::Command::new(command).configure(|c| {
    c.args(args);
    for (k, v) in env {
        c.env(k, v);
    }
    if let Some(dir) = &workdir {
        c.current_dir(dir);
    }
    procutil::no_window_tokio(c);
});
```

**File C:** `src-tauri/src/agent/tools/grep.rs`, line 21

Current:
```rust
let mut cmd = Command::new("rg");
```

New — add `use crate::commands::procutil;` and after setting all args, before `.output()`:
```rust
let mut cmd = Command::new("rg");
// ... existing .arg() calls ...
procutil::no_window(&mut cmd);
```

**File D:** `src-tauri/src/commands/shell.rs` — cleanup duplicated constant

Current Windows block:
```rust
#[cfg(target_os = "windows")]
{
    use std::os::windows::process::CommandExt;
    const CREATE_NO_WINDOW: u32 = 0x08000000;
    Command::new("cmd")
        .args(["/c", "start", "cmd", "/k", "cd", "/d", &path])
        .creation_flags(CREATE_NO_WINDOW)
        .spawn()
        .map_err(|e| format!("Failed to open Terminal: {e}"))?;
}
```

New — use `procutil::no_window`:
```rust
#[cfg(target_os = "windows")]
{
    let mut cmd = Command::new("cmd");
    cmd.args(["/c", "start", "cmd", "/k", "cd", "/d", &path]);
    crate::commands::procutil::no_window(&mut cmd);
    cmd.spawn()
        .map_err(|e| format!("Failed to open Terminal: {e}"))?;
}
```

Also remove the now-unused `use std::os::windows::process::CommandExt;` import from the Windows block.

---

#### Item 2c — Settings toggle `code_intel_enabled`

**File A:** `src-tauri/src/agent/provider.rs` — `AgentConfig` struct (~line 39)

Add before `keep_awake` (near line 99):
```rust
#[serde(default = "default_true")]
pub code_intel_enabled: bool,
```

And in the `Default` impl (line 126+):
```rust
code_intel_enabled: true,
```

**File B:** `src-tauri/src/commands/code_intel.rs` — `open_workspace` command

After the workspace is created but BEFORE LSP startup (around line 289–292):
```rust
// Gate LSP + indexing on config
let config = /* load config */;
if config.code_intel_enabled {
    let mut lsp = ws.lsp_manager.lock().await;
    let _ = lsp.start_for_workspace(&path);
    // ... embedding/indexing code (already gated by if/else flow) ...
}
```

For simplicity: wrap the entire LSP start + embedding generation block inside `if config.code_intel_enabled { ... }`. The `FileWatcher` should also be skipped when disabled (it triggers reindexing).

**File C:** `src/App.tsx` — Settings modal (~line 1059, near `keep_awake`)

Add checkbox row following the exact `keep_awake` pattern:
```tsx
<label class="mb-2 flex cursor-pointer items-center gap-2">
  <input
    type="checkbox"
    checked={configCodeIntelEnabled()}
    onChange={(e) => setConfigCodeIntelEnabled(e.currentTarget.checked)}
    class="h-4 w-4 rounded border-border-subtle bg-surface-0 text-accent focus:ring-accent"
  />
  <span class="text-sm font-medium text-ink">{t("app.config.codeIntelEnabled")}</span>
  <span class="text-[11px] text-ink-faint">{t("app.config.codeIntelEnabledHint")}</span>
</label>
```

Need a new signal `configCodeIntelEnabled` and setter `setConfigCodeIntelEnabled` following the pattern of `configKeepAwake`/`setConfigKeepAwake`. These already exist as a pattern: signal declared alongside other config signals, setter calls `setConfig()` with the new value.

**File D:** `src/lib/locales/en-US.ts` — add keys:
```ts
"app.config.codeIntelEnabled": "Code Intelligence (LSP + embeddings)",
"app.config.codeIntelEnabledHint": "Disable to save memory on large projects. Requires workspace reopen.",
```

**File E:** `src/lib/locales/pt-BR.ts` — add keys:
```ts
"app.config.codeIntelEnabled": "Code Intelligence (LSP + embeddings)",
"app.config.codeIntelEnabledHint": "Desative para economizar memória em projetos grandes. Requer reabrir o workspace.",
```

---

#### Item 2d — Deduplicate git_branch polling

**File A:** `src/App.tsx` — add `branch` prop to `GitIndicator`

The `App.tsx` already polls `git_branch` at line 170 and stores it in `gitBranchName` signal (line 147). The `GitIndicator` component is rendered with this signal already available — just need to plumb it differently.

Current `App.tsx` usage of `GitIndicator` — find where it's rendered and change from:
```tsx
<GitIndicator workspace={activeWorkspace()} />
```
To:
```tsx
<GitIndicator workspace={activeWorkspace()} branch={gitBranchName()} />
```

**File B:** `src/components/GitIndicator.tsx`

Remove:
- The `branchInFlight` ref (or variable)
- The `refreshBranch` function
- The `branchIntervalId` interval (line ~60, `setInterval(refreshBranch, 30000)`)
- The `gitBranchName` signal (if local)
- The `git_branch` import if no longer needed
- Clean up the `onCleanup` to only clear the `statusIntervalId`

Add `branch` to props:
```tsx
props: { workspace: () => string | null; branch: () => string }
```

Where the branch was previously displayed from the local signal, use `props.branch()` instead.

---

#### Item 3 — "Open in IDE" + `--goto`

##### Rust backend (new file)

**File: `src-tauri/src/commands/ide.rs`** (new)

```rust
use std::path::Path;
use std::process::Command;

/// Returns list of detected IDE ids: ["vscode", "cursor"]
#[tauri::command]
pub async fn detect_ides() -> Vec<String> {
    let mut ides: Vec<String> = Vec::new();

    // Check CLI on PATH using login-shell PATH (same helper grep.rs uses)
    let login_path = crate::agent::tools::bash::login_path();

    // vscode
    if which_in_path("code", &login_path).is_some()
        || cfg!(target_os = "macos") && Path::new("/Applications/Visual Studio Code.app").exists()
    {
        ides.push("vscode".to_string());
    }

    // cursor
    if which_in_path("cursor", &login_path).is_some()
        || cfg!(target_os = "macos") && Path::new("/Applications/Cursor.app").exists()
    {
        ides.push("cursor".to_string());
    }

    ides
}

/// Open a path (file or folder) in the specified IDE, optionally at a line number
#[tauri::command]
pub async fn open_in_ide(path: String, ide: String, goto_line: Option<u32>) -> Result<(), String> {
    match ide.as_str() {
        "vscode" => open_ide("code", "Visual Studio Code", &path, goto_line),
        "cursor" => open_ide("cursor", "Cursor", &path, goto_line),
        other => Err(format!("Unknown IDE: {other}")),
    }
}

fn open_ide(cli: &str, app_name: &str, path: &str, goto_line: Option<u32>) -> Result<(), String> {
    if let Some(line) = goto_line {
        // --goto requires the CLI on all platforms
        let mut cmd = Command::new(cli);
        cmd.arg("--goto").arg(format!("{path}:{line}:1"));
        crate::commands::procutil::no_window(&mut cmd);
        cmd.spawn()
            .map_err(|e| format!("Failed to open {app_name} with goto: {e}"))?;
    } else {
        #[cfg(target_os = "macos")]
        {
            Command::new("open")
                .args(["-a", app_name, path])
                .spawn()
                .map_err(|e| format!("Failed to open {app_name}: {e}"))?;
        }
        #[cfg(target_os = "windows")]
        {
            let mut cmd = Command::new("cmd");
            cmd.args(["/c", cli, path]);
            crate::commands::procutil::no_window(&mut cmd);
            cmd.spawn()
                .map_err(|e| format!("Failed to open {app_name}: {e}"))?;
        }
        #[cfg(target_os = "linux")]
        {
            let login_path = crate::agent::tools::bash::login_path();
            let mut cmd = Command::new(cli);
            cmd.env("PATH", &login_path).arg(path);
            crate::commands::procutil::no_window(&mut cmd);
            cmd.spawn()
                .map_err(|e| format!("Failed to open {app_name}: {e}"))?;
        }
    }
    Ok(())
}

fn which_in_path(binary: &str, login_path: &str) -> Option<String> {
    let full_path = format!("{login_path}:/usr/local/bin:/usr/bin:/bin");
    std::env::split_paths(&full_path)
        .find(|dir| dir.join(binary).exists())
        .map(|d| d.join(binary).to_string_lossy().to_string())
}
```

**Module registration:**

**File:** `src-tauri/src/commands/mod.rs` — add:
```rust
pub mod ide;
```

**File:** `src-tauri/src/lib.rs` — in `generate_handler!`, add near `commands::shell::open_in_terminal` (line ~78):
```rust
commands::ide::detect_ides,
commands::ide::open_in_ide,
```

##### Config

**File:** `src-tauri/src/agent/provider.rs` — `AgentConfig` struct, add:
```rust
#[serde(default)]
pub preferred_ide: Option<String>,
```

And in `Default` impl:
```rust
preferred_ide: None,
```

##### Frontend — IPC wrappers

**File:** `src/lib/ipc.ts` — add near `openInTerminal` (line 37):
```ts
export function detectIdes(): Promise<string[]> {
  return invoke<string[]>("detect_ides");
}

export function openInIde(path: string, ide: string, gotoLine?: number): Promise<void> {
  return invoke<void>("open_in_ide", { path, ide, gotoLine });
}
```

##### Frontend — `--goto` cursor tracking

**File:** `src/App.tsx`
- New signal: `const [activeEditorCursor, setActiveEditorCursor] = createSignal<{path: string, line: number} | null>(null);`
- Handler: `const handleCursorLineChange = (path: string, line: number) => setActiveEditorCursor({path, line});`
- Pass to both modals:
  - `<FileEditorModal ... onCursorLineChange={(line) => handleCursorLineChange(editorFilePath()!, line)} />`
  - `<ContentViewerModal ... onCursorLineChange={(line) => handleCursorLineChange(viewerFilePath, line)} />`

**File:** `src/components/FileEditorModal.tsx`
- Add to props: `onCursorLineChange?: (line: number) => void`
- In Monaco `onMount` callback (where `editor` is available):
  ```ts
  if (onCursorLineChange) {
    editor.onDidChangeCursorPosition((e) => {
      onCursorLineChange(e.position.lineNumber);
    });
  }
  ```
- Fire initial cursor position on mount: `onCursorLineChange?.(editor.getPosition()?.lineNumber ?? 1)`

**File:** `src/components/ContentViewerModal.tsx` — identical changes as FileEditorModal.

##### Frontend — Settings modal

**File:** `src/App.tsx` — settings modal (~line 704, after theme picker)
```tsx
<Show when={availableIdes().length > 0}>
  <label class="mb-4 block">
    <span class="mb-1 block text-sm font-medium text-ink">{t("app.config.preferredIde")}</span>
    <select
      value={configPreferredIde() ?? availableIdes()[0]}
      onChange={(e) => setConfigPreferredIde(e.currentTarget.value || null)}
      class="w-full appearance-none rounded-md border border-border-subtle bg-surface-0 p-2 text-sm text-ink focus:border-accent focus:outline-none focus:ring-1 focus:ring-accent"
    >
      <For each={availableIdes()}>
        {(ide) => <option value={ide}>{ideLabel(ide)}</option>}
      </For>
    </select>
  </label>
</Show>
```

Need signals: `availableIdes` (populated on mount via `detectIdes()`), `configPreferredIde`, `setConfigPreferredIde`.

Helper: `ideLabel(ide: string) => ide === 'vscode' ? 'VS Code' : 'Cursor'`.

##### Frontend — Header button

**File:** `src/App.tsx` — header right cluster (~line 650), insert BEFORE the settings gear:
```tsx
<Show when={availableIdes().length > 0}>
  <button
    onClick={() => {
      const ide = configPreferredIde() ?? availableIdes()[0];
      const ws = activeWorkspace();
      if (ws) openInIde(ws, ide).catch(console.error);
    }}
    class="flex h-7 w-7 items-center justify-center rounded-md text-ink-muted hover:bg-surface-2 hover:text-ink"
    title={t("app.openInIde", { ide: ideLabel(configPreferredIde() ?? availableIdes()[0]) })}
  >
    <Icon name="external-link" />
  </button>
</Show>
```

##### Frontend — App-level context menu

**File:** `src/App.tsx` — lines 1508–1533, the `ContextMenu` items array

Add per-IDE items AFTER "Reveal in Finder" and BEFORE "Open in Terminal":
```tsx
...(availableIdes().map(ide => ({
  label: `Open in ${ideLabel(ide)}`,
  icon: 'external-link' as IconName,
  action: () => openInIde(pos().path, ide).catch(console.error),
}))),
```

##### Frontend — FileTree context menu

**File:** `src/components/FileTree.tsx` — lines 92–119

Add IDE items. When the right-clicked file matches `activeEditorCursor().path`, use `--goto`:
```tsx
...(props.availableIdes?.() ?? []).map(ide => {
  const gotoLine = props.activeEditorCursor?.()?.path === pos().path
    ? props.activeEditorCursor?.()?.line
    : undefined;
  return {
    label: `Open in ${ideLabel(ide)}`,
    icon: 'external-link' as IconName,
    action: () => openInIde(pos().path, ide, gotoLine).catch(console.error),
  };
}),
```

New FileTree props:
- `availableIdes?: () => string[]`
- `activeEditorCursor?: () => {path: string, line: number} | null`

Pass these from App.tsx at the FileTree render site (~line 1359).

##### Locale keys

**File:** `src/lib/locales/en-US.ts`:
```ts
"app.openInIde": "Open in {ide}",
"app.config.preferredIde": "Preferred IDE",
```

**File:** `src/lib/locales/pt-BR.ts`:
```ts
"app.openInIde": "Abrir no {ide}",
"app.config.preferredIde": "IDE preferido",
```

---

### Verification Plan

1. **Item 1 — macOS build:** `npm run tauri build` → right-click on chat background produces no menu; textarea right-click keeps native menu; selected text keeps native menu.
2. **Item 2a — LSP gating:** Open a JS-only workspace → `ps aux | grep rust-analyzer` shows nothing; `ps aux | grep typescript-language-server` shows tsserver. Open Rust project → rust-analyzer starts.
3. **Item 2b — `no_window`:** `cargo check` passes on macOS (no-ops). Windows confirmation deferred to user smoke test.
4. **Item 2c — Toggle off:** Toggle code_intel_enabled OFF in settings, save, reopen workspace → no LSP processes. Hover/goto-definition show graceful error. Toggle back ON, reopen → LSP starts.
5. **Item 2d — Git polling dedup:** Open workspace with git → `gitBranchName` appears in header. Only 1 network call per 30s to `git_branch` (vs 2 before).
6. **Item 3 — IDE detection:** On machine without VS Code/Cursor → header button hidden, context menu has no IDE items. On machine with VS Code → button appears, dropdown shows VS Code.
7. **Item 3 — Open folder:** Click header button → VS Code/Cursor opens at workspace root.
8. **Item 3 — Open file with goto:** Open a file in the editor, right-click in FileTree → "Open in VS Code" with `--goto` passes line number. `code --goto <file>:<line>:1` executes.
9. **Item 3 — Open file without goto:** Right-click a file NOT currently open → "Open in VS Code" opens without `--goto`.
10. **Regression:** `npm run test`, `cargo test` pass. Existing terminal opening, file reveal, and copy path context menu items still work.

### Tasks summary

1. Item 1: Add `contextmenu` suppression to `src/index.tsx`
2. Item 2a: Gate LSP spawns on project root markers in `manager.rs`
3. Item 2b: Add `no_window` to LSP client, MCP stdio, `rg` spawn; clean up `shell.rs`
4. Item 2c: Add `code_intel_enabled` to config, gate in `code_intel.rs`, add settings checkbox + locale
5. Item 2d: Deduplicate `git_branch` polling between App.tsx and GitIndicator
6. Item 3a: Create `src-tauri/src/commands/ide.rs`, register module + commands
7. Item 3b: Add `preferred_ide` to config, add IPC wrappers, add settings dropdown + locale
8. Item 3c: Add header IDE button, App-level context menu IDE items
9. Item 3d: Add `--goto` cursor tracking to FileEditorModal + ContentViewerModal
10. Item 3e: Add FileTree context menu IDE items with `--goto` support
11. Verify: run tests + manual checks


## Implementation Log — 2026-07-16 10:56
**Summary:** Windows user feedback: context menu suppression, no_window, conditional LSP, IDE detection + context menu items with --goto, cursor tracking in modals, cleanup of ~62 duplicate buttons
**Changed files:** M src-tauri/src/agent/mcp.rs, M src-tauri/src/agent/provider.rs, M src-tauri/src/agent/tools/grep.rs, M src-tauri/src/commands/agent.rs, M src-tauri/src/commands/code_intel.rs, M src-tauri/src/commands/mod.rs, M src-tauri/src/commands/shell.rs, M src-tauri/src/lib.rs, M src-tauri/src/lsp/client.rs, M src-tauri/src/lsp/manager.rs, M src/App.tsx, M src/components/ContentViewerModal.test.tsx, M src/components/ContentViewerModal.tsx, M src/components/FileEditorModal.test.tsx, M src/components/FileEditorModal.tsx, M src/components/FileTree.tsx, M src/components/GitIndicator.test.tsx, M src/components/GitIndicator.tsx, M src/index.tsx, M src/lib/ipc.ts, M src/lib/locales/en-US.ts, M src/lib/locales/pt-BR.ts, ?? docs/2026-07-15_reddit-post.md, ?? docs/plans/2026-07-15_reddit-post-r-claudinio.md, ?? docs/plans/2026-07-16_windows-feedback-playful-torvalds-merged.md, ?? src-tauri/src/commands/ide.rs
**Commits:** _(git unavailable or none)_
**Journal:** Key findings from implementation:

1. **Runaway agent duplication**: The previous session's agent copy-pasted ~62 identical "Open in IDE" button blocks throughout App.tsx — inside the settings modal, sidebar loops, update banner, and even mid-span. Cleaned up with a Python regex script removing all but the legitimate header button.

2. **Monaco mock gap**: Added `onDidChangeCursorPosition` mock to FileEditorModal.test.tsx and ContentViewerModal.test.tsx — the real code now uses cursor tracking but the mocks didn't provide it, causing test failures.

3. **GitIndicator test syntax error**: The previous session left a malformed test with an empty body that broke babel parsing. Fixed by adding the missing test implementation.

4. **Cursor tracking propagation pattern**: ContentViewerModal (read-only) and FileEditorModal (editable) both now expose `onCursorLineChange` via the Monaco `onDidChangeCursorPosition` event. The pattern is consistent — same prop name, same listener wiring.

5. **Context menu IDE items**: Both the app-level (workspace project list) and FileTree context menus now show per-IDE items (VS Code / Cursor) with `--goto path:line` when `activeEditorCursor()` points to the same file path as the context menu target.

6. **FileTree prop passing**: `availableIdes` is passed as a plain value (`availableIdes()`), while `activeEditorCursor` is passed as the signal itself (accessor), so FileTree reads it reactively on each render.

**Task journal:**
- Item 1: Suppress default WebView2 context menu: Added contextmenu event listener to src/index.tsx
- Item 2a: Conditional LSP spawn based on project root markers: Edited start_for_workspace in manager.rs
- Item 2b: Apply no_window to LSP, MCP, rg spawns: Added no_window to LSP client.rs, MCP rs, grep rs, shell.rs
- Item 2c: Settings toggle code_intel_enabled: Rust backend done. Frontend settings checkbox + IDE dropdown done.
- Item 2d: Deduplicate git_branch polling: Removed gitBranch from GitIndicator. Tests updated.
- Item 3a: Create IDE backend: ide.rs created, registered in mod.rs and lib.rs
- Item 3b: IDE config + IPC + settings dropdown: IPC types, settings dropdown, locale keys all done.
- Item 3c: Header IDE button + App context menu IDE items: Removed 62 spurious duplicate 'Open in IDE' buttons. Added per-IDE items to app-level context menu with --goto support.
- Item 3d: Cursor tracking in ContentViewerModal: Added onCursorLineChange to ContentViewerModalProps + Monaco cursor listener.
- Item 3e: FileTree context menu IDE items with --goto: Added availableIdes and activeEditorCursor props to FileTree. Added per-IDE context menu items with --goto when cursor matches file.
- Verification: cargo check + npm test: cargo check: ok. cargo test: 225 passed. npm test: 35 files, 639 passed.
