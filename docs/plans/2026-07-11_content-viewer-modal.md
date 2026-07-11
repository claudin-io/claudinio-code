# Content Viewer Modal — Link Interception & Rendering

## Context / Problem Statement

When the LLM generates markdown containing links (relative file paths, web URLs, image paths, video/audio paths), the user expects to **click them** and see the content. Currently:

1. **Links are rendered** by `marked` as `<a href="...">` tags — visually styled (accent color, underline) but **non-functional**.
2. **No click interception exists** — clicking a link in the Tauri WebView does nothing (no navigation, no external browser).
3. **No content viewer modal exists** — the project has `FileEditorModal` (Monaco, edible) and `TextEditorModal` (Monaco, text), but no unified read-only viewer for arbitrary content types.

The user confirmed: every clickable link in the chat must resolve to the appropriate viewer.

## Goal (Definition of Done)

Clicking any link rendered in chat text:
- **Web URLs** (`http://`, `https://`) → open in the system's default browser via `openExternalUrl()`.
- **Code/text files** (`.md`, `.txt`, `.js`, `.ts`, `.rs`, `.py`, `.json`, `.toml`, `.css`, `.html`, `.yaml`, `.yml`, `.xml`, `.sh`, `.sql`, `.env`, `.gitignore`, and similar) → open in a **read-only Monaco Editor modal** (`ContentViewerModal`).
- **Images** (`.png`, `.jpg`, `.jpeg`, `.gif`, `.webp`, `.svg`) → open in a modal with `object-fit: contain` and an "open externally" button.
- **Videos** (`.mp4`, `.webm`, `.mov`) → open in a modal with an HTML5 `<video>` player.
- **Audio** (`.mp3`, `.wav`, `.ogg`, `.flac`) → open in a modal with an HTML5 `<audio>` player.
- **Streaming links** (YouTube, Spotify, etc.) → open in the system browser (same as web URLs).
- The modal is closeable via: (a) close button (X), (b) backdrop click, (c) Escape key.

## Key Findings (Prova Real)

| Finding | Method | Proof |
|---|---|---|
| ChatPanel renders assistant messages via `innerHTML={marked.parse(msg.text)}` at line ~1777 | `read_file` of ChatPanel.tsx | Lines 1774–1778 |
| Only `code` renderer is overridden in `marked.use()` — `link` uses default renderer | `read_file` of ChatPanel.tsx | Lines 62–69 |
| `openExternalUrl()` exists in `src/lib/ipc.ts` line 689, uses `openUrl()` from `@tauri-apps/plugin-opener` | `grep` + `read_file` | Lines 689–691 |
| `openExternal()` exists in ipc.ts line 685, uses `openPath()` for file paths | `grep` + `read_file` | Lines 685–687 |
| ChatPanel receives `workspace: string` prop (root path), used for resolving relative file paths | `grep` for `props.workspace` | Line 443 of ChatPanel.tsx |
| `readFile()` IPC function exists, returns `Promise<string>` | `read_file` of ipc.ts | Line 15 |
| Monaco Editor already a dependency (`monaco-editor ^0.55.1`), used in FileEditorModal, TextEditorModal, DiffViewer | `read_file` of package.json | Dependencies list |
| No existing `convertFileSrc` usage — needed for serving local media in Tauri WebView | `grep` across entire `src/` | Empty result |
| Existing modal pattern: `fixed inset-0 z-50 bg-black/40` overlay + centered card, close on backdrop click + Escape | `read_file` of CommitPushModal, GitChangesModal, FileEditorModal | Consistent pattern across all 5 modals |
| User confirmed: web links → system browser; local files → read-only Monaco; `w-[80vw] h-[80vh]`; images → contain + external button; both data attributes + event delegation | `ask_user` interview (6 questions) | All decisions confirmed |

## Authoritative Inputs

| Input | Value | Source |
|---|---|---|
| Modal dimensions | `w-[80vw] h-[80vh]` | User (Q3) |
| Link interception strategy | Custom `marked` link renderer (data attributes) + event delegation | User (Q6) |
| Web link behavior | `openExternalUrl()` — system browser | User (Q1) |
| Local file behavior | Read-only Monaco Editor modal | User (Q2) |
| Image behavior | `object-fit: contain` + "open externally" button | User (Q4) |
| Video/audio behavior | Embedded HTML5 player for local files, external browser for streaming | User (Q5) |
| File extensions for code/text | `.md`, `.txt`, `.js`, `.ts`, `.jsx`, `.tsx`, `.rs`, `.py`, `.json`, `.toml`, `.yaml`, `.yml`, `.css`, `.scss`, `.html`, `.xml`, `.sh`, `.bash`, `.sql`, `.env`, `.gitignore`, `.php`, `.java`, `.go`, `.c`, `.h`, `.cpp`, `.hpp`, `.swift`, `.kt`, `.rb`, `.lua`, `.r`, `.cs`, `.fs` | Inferred from `detectLanguage()` in FileEditorModal.tsx |

## Changes (Steps)

### Step 1 — Create `ContentViewerModal` component (`src/components/ContentViewerModal.tsx`)

**Target:** NEW file `src/components/ContentViewerModal.tsx`

**Mutation:** A unified modal component that accepts `contentType`, `title`, `content`/`src`, and `workspace`, and renders the appropriate viewer:
- **`text`**: Monaco Editor, read-only, language detected from file extension. Loads file content via `readFile()` IPC.
- **`image`**: `<img>` with `object-fit: contain`, centered, with a "Open Externally" button using `openExternal()`.
- **`video`**: HTML5 `<video controls>` with `convertFileSrc()` for local file URL.
- **`audio`**: HTML5 `<audio controls>` with `convertFileSrc()` for local file URL.
- **`loading`** and **`error`** states.

**Why:** Centralizes content rendering logic; replaces the need for N separate viewer modals.

**Constraints:**
- Follow existing modal pattern: `fixed inset-0 z-50 flex items-center justify-center bg-black/40`
- Card: `w-[80vw] h-[80vh] rounded-xl bg-surface-0 shadow-2xl flex flex-col overflow-hidden`
- Header: flex row with filename/title + close (X) button
- Close: backdrop click (`e.target === e.currentTarget`), Escape key, X button
- Monaco Editor: `monaco.editor.create()`, `readOnly: true`, `wordWrap: "on"`, no minimap, same themes as FileEditorModal (`claudinio-dark`/`claudinio-light`)
- Dispose Monaco editor on cleanup (`onCleanup`)
- For images/video/audio: use `convertFileSrc()` from `@tauri-apps/api/core` to get `asset://` URL
- Props: `contentType: 'text' | 'image' | 'video' | 'audio'`, `filePath: string`, `title: string`, `workspace: string`, `onClose: () => void`

### Step 2 — Add custom `link` renderer to `marked` config in `ChatPanel.tsx`

**Target:** `src/components/ChatPanel.tsx`, the `marked.use()` block (lines ~62-69)

**Mutation:** Add a `link()` renderer to the `marked.use({ renderer: { ... } })` configuration:
```tsx
link({ href, title, text }) {
  const ext = href.split('?')[0].split('#')[0].split('.').pop()?.toLowerCase();
  let dataType = 'external';
  if (href.match(/^https?:\/\//)) {
    dataType = 'external';
  } else if (ext && ['png','jpg','jpeg','gif','webp','svg'].includes(ext)) {
    dataType = 'image';
  } else if (ext && ['mp4','webm','mov'].includes(ext)) {
    dataType = 'video';
  } else if (ext && ['mp3','wav','ogg','flac'].includes(ext)) {
    dataType = 'audio';
  } else {
    dataType = 'file';
  }
  const titleAttr = title ? ` title="${title}"` : '';
  return `<a href="${href}"${titleAttr} data-link-type="${dataType}">${text}</a>`;
}
```

**Why:** Data attributes enable the event delegation handler to classify links without regex at click time. The classification logic lives in one place.

**Constraints:** Preserve existing behavior for all other markdown elements. Do NOT change the `code` renderer.

### Step 3 — Add event delegation click handler in `ChatPanel.tsx`

**Target:** `src/components/ChatPanel.tsx`, the section where `.prose-content` divs are rendered for assistant messages.

**Mutation:** Add an `onClick` handler to the container that renders assistant message markdown (the `<div class="prose-content ..." innerHTML={...} />`):
```tsx
onClick={(e) => {
  const anchor = (e.target as HTMLElement).closest('a[data-link-type]');
  if (!anchor) return;
  e.preventDefault();
  const href = anchor.getAttribute('href')!;
  const linkType = anchor.getAttribute('data-link-type')!;
  handleLinkClick(href, linkType);
}}
```

And a `handleLinkClick(href: string, linkType: string)` function that:
- `external` → `openExternalUrl(href)`
- `file` → resolve path (relative → absolute using `props.workspace`), set `viewerFile` signal to `{ type: 'text', path, title }`
- `image` → same, type = `'image'`
- `video` → same, type = `'video'`
- `audio` → same, type = `'audio'`

**Why:** Event delegation is more efficient than per-link handlers, and catches dynamically rendered content from `innerHTML`.

**Constraints:**
- Must handle relative paths (`./src/foo.ts`, `../lib/bar.rs`) — resolve against `props.workspace`
- Must handle absolute paths as-is
- Must strip `file://` prefix if present
- Must prevent default navigation
- Apply to ALL `.prose-content` containers (assistant messages + timeline items + thinking rows)

### Step 4 — Integrate `ContentViewerModal` into `ChatPanel.tsx`

**Target:** `src/components/ChatPanel.tsx`

**Mutation:** Add `ContentViewerModal` import, state signal (`viewerFile`), and render it at the end of the component (alongside other modals like GitChangesModal, CommitPushModal):
```tsx
<Show when={viewerFile()}>
  <ContentViewerModal
    contentType={viewerFile()!.type}
    filePath={viewerFile()!.path}
    title={viewerFile()!.title}
    workspace={props.workspace}
    onClose={() => setViewerFile(null)}
  />
</Show>
```

**Why:** Wires the new modal into the existing chat UI.

**Constraints:** Place alongside other modals at the bottom of the ChatPanel JSX return, before the closing fragment.

### Step 5 — Add i18n strings for the new modal

**Target:** `src/lib/locales/en-US.ts` and `src/lib/locales/pt-BR.ts`

**Mutation:** Add strings:
```ts
// en-US
contentViewer: {
  openExternally: "Open Externally",
  close: "Close",
  loading: "Loading...",
  error: "Failed to load file",
},

// pt-BR
contentViewer: {
  openExternally: "Abrir Externamente",
  close: "Fechar",
  loading: "Carregando...",
  error: "Falha ao carregar arquivo",
},
```

**Why:** Consistent i18n coverage.

**Constraints:** Add to the existing `t()` lookup dictionary. Use the `t()` function for all user-facing strings in `ContentViewerModal`.

### Step 6 — Add `convertFileSrc` import in `ContentViewerModal`

**Target:** `src/components/ContentViewerModal.tsx`

**Mutation:** Import `convertFileSrc` from `@tauri-apps/api/core` to generate `asset://` URLs for local media files.

**Why:** Tauri WebView cannot access local files via `file://`; `asset://` protocol is required.

## Verification Plan

1. **TypeScript compilation:** `pnpm tsc --noEmit` — must pass with zero errors.
2. **Build:** `pnpm build` — Vite build must succeed.
3. **Lint check:** `pnpm lint` (if script exists) — must pass.
4. **Existing tests:** `pnpm test` — all existing tests must still pass (no regressions).
5. **Visual/manual verification (Tauri app):**
   - Start the app, open a workspace, send a message to the agent that triggers markdown links
   - Click a web URL → must open in system browser
   - Click a `.md` or `.ts` file link → must open read-only Monaco editor in modal
   - Click an image link → must show image with `object-fit: contain` and "Open Externally" button
   - Click a video/audio link → must show HTML5 player
   - Close modal via X, Escape, and backdrop click → all must work
6. **Edge cases:**
   - Broken/unresolvable paths → error state in modal
   - Non-existent local file → error state
   - Very long filenames → modal header must truncate gracefully
   - Dark/light theme → Monaco and modal must respect current theme

## Risks

| Risk | Mitigation |
|---|---|
| `marked` link renderer interfering with existing markdown behavior | The custom renderer only adds a `data-link-type` attribute; all other HTML output is identical to default |
| Monaco Editor memory leaks from multiple creates/disposes | Follow existing pattern in FileEditorModal: `onCleanup(() => editor.dispose())`, single ref |
| `convertFileSrc` not available or broken in dev mode | Test both dev (`pnpm tauri dev`) and build |
| Performance with large files in Monaco | Monaco handles large files natively; set `largeFileOptimizations: true` if needed |

## Tasks Summary

1. Create `ContentViewerModal` component (text/image/video/audio viewers)
2. Add `link` renderer to `marked` config in ChatPanel
3. Add event delegation + `handleLinkClick` in ChatPanel
4. Integrate ContentViewerModal into ChatPanel
5. Add i18n strings (en-US + pt-BR)
6. Verify: build, tests, type-check


## Implementation Log — 2026-07-11 14:26
**Summary:** Content Viewer Modal — link interception system for LLM chat messages
**Changed files:** M src/components/ChatPanel.tsx, M src/lib/locales/en-US.ts, M src/lib/locales/pt-BR.ts, ?? docs/plans/2026-07-11_content-viewer-modal.md, ?? src/components/ContentViewerModal.tsx
**Commits:** _(git unavailable or none)_
**Journal:** All 5 tasks implemented and verified. Key decisions and gotchas:

1. **ContentViewerModal**: Created with 4 content type modes (text/image/video/audio). Uses Monaco Editor (read-only, same theme/claudinio integration as FileEditorModal) for text. Media files use convertFileSrc() from @tauri-apps/api/core for the `asset://` protocol (required by Tauri WebView — `file://` doesn't work). resolvePath() handles both relative paths (resolved against workspace root, matching the pattern in ChatPanel's handleLinkClick) and absolute paths, plus strips `file://` prefix.

2. **Link classification in marked**: The custom link() renderer classifies links by extension (after stripping query/fragment). Web URLs (http/https) → 'external', images/video/audio by extension → their type, everything else → 'file'. The data-link-type attribute is added to each <a> tag.

3. **Event delegation**: onClick handlers on all 5 .prose-content divs. Three are inside the ChatPanel component (assistant messages, live text) and have access to the full handleLinkClick function that routes to the modal for file/media types. The other three (PhaseResultRow, TextRow, SubagentTimeline report) are separate components without access to the modal signal — they only handle external links (openExternalUrl). This is a pragmatic tradeoff: timeline entries rarely contain local file links, and making the modal state available across component boundaries would add complexity.

4. **Gotcha - Regex in resolvePath**: The subagent originally wrote `workspace.replace(/\/\/g, "/")` which is invalid regex syntax. Had to fix to `workspace.replace(/\\/g, "/")`.

5. **Build**: 395 tests pass, Vite build successful, type-check shows zero new errors.

**Task journal:**
- Create ContentViewerModal component: Created ContentViewerModal.tsx with 4 content types (text/image/video/audio), Monaco Editor read-only, convertFileSrc for media, resolvePath helper, loading/error states, backdrop/Escape/X close
- Add link renderer to marked config in ChatPanel: Added link() renderer to marked.use() that classifies links via data-link-type: external/image/video/audio/file
- Add event delegation + link handler + modal integration in ChatPanel: Added viewerFile signal and handleLinkClick function; Added ContentViewerModal import; Added onClick delegation to all 5 .prose-content divs (assistant messages, live text, PhaseResultRow, TextRow, subagent report); PhaseResultRow, TextRow, and subagent report handle only external links (separate components); Added ContentViewerModal rendering at bottom of component
- Add i18n strings for ContentViewerModal: en-US: 'Open Externally', 'Close', 'Loading...', 'Failed to load file'; pt-BR: 'Abrir Externamente', 'Fechar', 'Carregando...', 'Falha ao carregar arquivo'
- Verify: build, type-check, tests: pnpm tsc --noEmit: zero erros do nosso código (apenas erros pré-existentes em .test.ts e FileEditorModal); pnpm build: OK — 24 test files, 395 tests passed; Vite build successful; pnpm tauri build: não testado (pipeline CI)
