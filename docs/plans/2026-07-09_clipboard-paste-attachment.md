# Clipboard Paste Attachment

## 1. Context / Problem Statement

Users expect Cmd+V / Ctrl+V in the chat input to attach images/files from the clipboard — just like dragging files or clicking the paperclip button. Currently:

- **No paste handler exists** in the textarea (`ChatPanel.tsx`)
- **No clipboard reading capability** anywhere in the app
- **No temp file utility** to persist clipboard blobs to disk
- **No toast/notification component** for user feedback
- Attachments only work via paperclip button (file picker → disk paths → `addAttachment(path)`) and drag-drop (Tauri native events → disk paths)

## 2. Goal (Definition of Done)

Pressing Cmd+V / Ctrl+V in the chat textarea when the system clipboard contains a non-text item (image, PDF, any file) attaches it to the message (same as paperclip/drag-drop). Text-only clipboard contents pass through unmodified (normal paste behavior). A small pill briefly appears near the input confirming the attachment.

## 3. Key Findings (Prova Real)

| Finding | Source |
|---|---|
| `addAttachment(filePath)` exists at `ChatPanel.tsx:530-540`, calls `readAttachment(path)` IPC → `src-tauri/src/commands/fs.rs:22` | File exploration |
| Attachments signal: `createSignal<{name,path,mediaType,size}[]>([])` at line 443 | File exploration |
| Textarea has `onInput`, `onKeyDown` handlers but **no `onPaste`** | `ChatPanel.tsx:1736-1860` |
| Paperclip button: calls `pickFiles()` → iterates paths → `addAttachment(f)` each | `ChatPanel.tsx:1697-1706` |
| Drag-drop: `getCurrentWindow().onDragDropEvent()` → iterates `payload.paths` → `addAttachment(filePath)` | `ChatPanel.tsx:744-757` |
| `readAttachment` Rust command reads file from disk, maps extension → MIME type, base64-encodes, returns `{name, mediaType, data, size}` | `src-tauri/src/commands/fs.rs:22-65` |
| `tempfile` crate NOT in Cargo.toml — needs to be added | `src-tauri/Cargo.toml` |
| No toast/snackbar/notification component exists | search of `src/components/` |
| No clipboard plugin installed | `Cargo.toml`, `package.json` |
| SolidJS used — signals, `<Show>`, `<For>`, `<Portal>` | imports in `ChatPanel.tsx:1` |
| Tauri v2 with `#[tauri::command]` pattern | `src-tauri/src/lib.rs` |
| `inputRef` is `let inputRef: HTMLTextAreaElement | undefined` at line 819 | Textarea exploration |

## 4. Authoritative Inputs

| Input | Value | Source |
|---|---|---|
| Handle types | Both images AND files (any non-text clipboard data) | User decision |
| Image→temp conversion | New Tauri Rust backend command | User decision |
| Clipboard reading | `onPaste` DOM event (no plugin needed — `clipboardData.items` for blobs, `clipboardData.files` for OS files) | Design inference |
| Text-only behavior | Default paste (unchanged) | User decision |
| Feedback | Small pill/badge near input, auto-dismisses after 2s | User decision |

## 5. Changes (Steps)

### Backend

**5.1** Add `tempfile` crate to `src-tauri/Cargo.toml` dependencies
- Change: add `tempfile = "3"` to `[dependencies]`
- Reason: needed to create temp files for clipboard blob data
- Constraint: use the system temp directory (`std::env::temp_dir()`)

**5.2** New file `src-tauri/src/commands/clipboard.rs`
- Implement `#[tauri::command] write_clipboard_blob(data: String, name: String, media_type: String) -> Result<WriteClipboardBlobResult, String>`
- Logic: base64-decode the `data`, write to a temp file with appropriate extension (from media_type), return `{path, name, media_type, size}`
- Returns the same shape as `read_attachment` result for seamless integration with existing `addAttachment` flow
- Media type → extension mapping: `image/png` → `.png`, `image/jpeg` → `.jpg`, `image/gif` → `.gif`, `image/webp` → `.webp`, `image/bmp` → `.bmp`, `application/pdf` → `.pdf`, default → `.bin`
- Constraint: clean up is caller's responsibility (temp files auto-cleaned by OS eventually)

**5.3** Register new command in `src-tauri/src/lib.rs`
- Add `commands::clipboard::write_clipboard_blob` to `generate_handler![]`
- Add `mod clipboard;` to `commands/mod.rs`

**5.4** New result type: `WriteClipboardBlobResult` (in clipboard.rs or shared types)
```rust
#[derive(Serialize)]
#[serde(rename_all = "camelCase")]
pub struct WriteClipboardBlobResult {
    pub path: String,
    pub name: String,
    pub media_type: String,
    pub size: usize,
}
```

### Frontend

**5.5** New IPC function in `src/lib/ipc.ts`
- Add `writeClipboardBlob(data: string, name: string, mediaType: string): Promise<WriteClipboardBlobResult>`
- Calls `invoke("write_clipboard_blob", { data, name, mediaType })`

**5.6** New component `src/components/ToastPill.tsx`
- Props: `message: string`, `visible: boolean`, `onDismiss: () => void`
- Renders a small fixed-position pill near bottom-center of chat panel
- Styling: `bg-surface-2 border border-accent/30 rounded-lg px-3 py-1.5 text-xs text-ink shadow-lg`
- Auto-dismisses after 2 seconds via `setTimeout` in `onMount`
- Fade-in/fade-out via CSS transition (opacity + transform)
- SolidJS component using `createSignal`, `onMount`, `onCleanup`

**5.7** Add `onPaste` handler to textarea in `ChatPanel.tsx` (~line 1846)
- Add `onPaste={handlePaste}` attribute on `<textarea>`
- Handler logic:

```typescript
const handlePaste = async (e: ClipboardEvent) => {
  const items = e.clipboardData?.items;
  if (!items) return;

  let handled = false;

  // Phase 1: Check for image blobs in clipboard items
  for (let i = 0; i < items.length; i++) {
    const item = items[i];
    if (item.kind === "file" && item.type.startsWith("image/")) {
      const blob = item.getAsFile();
      if (!blob) continue;

      const reader = new FileReader();
      const base64Data = await new Promise<string>((resolve) => {
        reader.onload = () => {
          const result = reader.result as string;
          // Strip data:image/...;base64, prefix
          const comma = result.indexOf(",");
          resolve(comma >= 0 ? result.slice(comma + 1) : result);
        };
        reader.readAsDataURL(blob);
      });

      const name = `clipboard-${Date.now()}.${item.type.split("/")[1] || "png"}`;
      const result = await writeClipboardBlob(base64Data, name, item.type);
      await addAttachment(result.path);
      handled = true;
      break; // One image at a time for simplicity
    }
  }

  // Phase 2: Check for file objects (copied from OS file manager)
  if (!handled && e.clipboardData.files.length > 0) {
    for (let i = 0; i < e.clipboardData.files.length; i++) {
      const file = e.clipboardData.files[i];
      // Tauri webview may expose path on File objects
      const filePath = (file as any).path as string | undefined;
      if (filePath) {
        await addAttachment(filePath);
        handled = true;
      }
      // If no path, fall through — don't prevent default
    }
  }

  // Phase 3: If we handled any attachment, prevent default text paste
  if (handled) {
    e.preventDefault();
    showToast(`📎 ${t("chat.toast.fileAttached")}`); // or similar i18n key
  }
};
```

**5.8** Add toast state management to `ChatPanel.tsx`
- New signal: `const [toastMessage, setToastMessage] = createSignal<string | null>(null)`
- Helper: `const showToast = (msg: string) => { setToastMessage(msg); }`
- Helper: `const dismissToast = () => setToastMessage(null)`
- Render `<ToastPill message={toastMessage()} visible={!!toastMessage()} onDismiss={dismissToast} />` near input area

**5.9** Add i18n keys (optional — can use hardcoded English initially)
- Key: `chat.toast.fileAttached` → "File attached"
- In `src/lib/grill-me.ts` (or wherever locales are defined)

## 6. Verification Plan

### Dry-run
- Build check: `cargo check` in `src-tauri/`
- Frontend check: `pnpm tsc --noEmit` (type-check)

### Application
1. Build the app with Tauri: `pnpm tauri build` (or `pnpm tauri dev`)
2. Open a workspace and verify chat panel loads normally

### End-to-end
3. **Image paste test**: Copy an image (e.g., screenshot with Cmd+Shift+4 on macOS), paste into chat input with Cmd+V
   - Expect: Toast pill appears "File attached", attachment pill appears below input
   - Expect: No text pasted into textarea
4. **File paste test**: Copy a file from Finder/Explorer, paste into chat input with Cmd+V
   - Expect: Same behavior — attachment pill appears, no text pasted
5. **Text paste test**: Copy plain text, paste into chat input with Cmd+V
   - Expect: Normal text paste behavior (uninterrupted)
6. **Send message with pasted attachment**: After pasting an image, type some text and send
   - Expect: Message sent with attachment included (same as paperclip flow)

### Regression
7. Paperclip button still works — click paperclip, pick files, attach
8. Drag-and-drop still works — drag file into chat window
9. `@`-mention popover still works — type `@` in textarea
10. Normal message send/receive cycle works

### Edge / No-op safety
11. Paste when disabled (compacting / awaiting approval) — paste should be ignored or disabled per existing `disabled` attribute
12. Rapid sequential pastes — each creates separate attachment pills
13. Very large clipboard images — should work (temp file writing handles any size)
14. Empty clipboard — nothing happens


## Implementation Log — 2026-07-09 22:40
**Summary:** Implement clipboard paste attachment: new Rust write_clipboard_blob cmd, ToastPill component, onPaste handler, i18n
**Changed files:** M src-tauri/Cargo.lock, M src-tauri/Cargo.toml, M src-tauri/src/commands/mod.rs, M src-tauri/src/lib.rs, M src/components/ChatPanel.tsx, M src/lib/ipc.ts, M src/lib/locales/en-US.ts, M src/lib/locales/pt-BR.ts, ?? docs/plans/2026-07-09_clipboard-paste-attachment.md, ?? src-tauri/src/commands/clipboard.rs, ?? src/components/ToastPill.tsx
**Commits:** _(git unavailable or none)_
**Journal:** ## Implementation Summary

### Backend (Rust)
- **`src-tauri/Cargo.toml`**: Added `tempfile = "3"` dependency for writing clipboard blobs to temporary files.
- **`src-tauri/src/commands/clipboard.rs`** (new): `write_clipboard_blob` command that base64-decodes incoming data, determines file extension from media type (image/png→.png, image/jpeg→.jpg, etc.), writes to a system temp file, and returns the path/name/media_type/size — same shape as `read_attachment` for seamless integration.
- **`src-tauri/src/commands/mod.rs`**: Added `pub mod clipboard;`.
- **`src-tauri/src/lib.rs`**: Registered `commands::clipboard::write_clipboard_blob` in invoke_handler.

### Frontend (TypeScript/SolidJS)
- **`src/lib/ipc.ts`**: Added `WriteClipboardBlobResult` interface and `writeClipboardBlob()` function.
- **`src/components/ToastPill.tsx`** (new): SolidJS component — fixed-position pill near input area, auto-dismisses after 2s, fade transitions via CSS.
- **`src/components/ChatPanel.tsx`**: Major additions —
  - `onPaste` handler with 3 phases: image blobs (FileReader → writeClipboardBlob → addAttachment), OS file copies (`.path` property → addAttachment), preventDefault + showToast.
  - Toast signal state management (`toastMessage`, `showToast`, `dismissToast`).
  - ToastPill rendering near input area.
- **`src/lib/locales/en-US.ts`** & **`pt-BR.ts`**: Added `chat.toast.fileAttached` key.

### Build Verification
- `cargo check`: ✅ Passed (no errors)
- `tsc --noEmit`: ✅ No new TypeScript errors (all existing errors are pre-existing test infra issues)

### Key Decisions Made During Implementation
1. Used `onPaste` DOM API directly (no clipboard plugin needed) — `clipboardData.items` handles image blobs natively, `clipboardData.files` handles OS file manager copies.
2. ToastPill uses a simplistic `onMount`/`onCleanup` pattern — the parent conditional rendering (`toastMessage()` truthy → mount, falsy → unmount) naturally re-triggers the timer on each new paste.
3. Phase 1 reads only one image per paste for simplicity — users paste one screenshot at a time typically.
4. Removed unused `Path` import from clipboard.rs (cleanup caught during review).

**Task journal:**
- Add tempfile crate to Cargo.toml dependencies: Added `tempfile = "3"` right after `diffy = "0.4"` in Cargo.toml dependencies.
- Create clipboard.rs with write_clipboard_blob command: Created clipboard.rs with write_clipboard_blob command. Accepts base64 data, decodes, writes to temp file with extension from media_type, returns path/name/media_type/size.
- Register clipboard command in lib.rs and commands/mod.rs: Added `pub mod clipboard;` to commands/mod.rs. Added `commands::clipboard::write_clipboard_blob` to invoke_handler in lib.rs.
- Add writeClipboardBlob IPC function to ipc.ts: Added `WriteClipboardBlobResult` interface. Added `writeClipboardBlob(data, name, mediaType)` function calling invoke.
- Create ToastPill component: Created ToastPill.tsx with SolidJS. Fixed-position pill near bottom-center, auto-dismisses after 2s via setTimeout. CSS transitions for fade in/out.
- Add onPaste handler and toast state to ChatPanel.tsx: Added handlePaste async function: Phase 1 — reads image blobs via FileReader, writes to temp, then addAttachment. Phase 2 — checks clipboard files for .path (OS file manager). Phase 3 — preventDefault + showToast. Added toastMessage signal, ToastPill rendering, onPaste on textarea.
- Add i18n key for toast message: Added `chat.toast.fileAttached` key to en-US.ts (`"File attached"`) and pt-BR.ts (`"Arquivo anexado"`).
- Verify: cargo check passes: cargo check: Finished dev profile in 1.63s, no errors.
- Verify: tsc type-check passes: All TypeScript errors pre-existing (test infra). None from our changes — grep for ToastPill, clipboard, writeClipboardBlob returned zero errors.
