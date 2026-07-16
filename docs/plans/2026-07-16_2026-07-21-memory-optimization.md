# Memory Optimization: Frontend Virtualization + Backend Caching

## Context

A Windows x64 user reported ~9.3GB memory usage in Claudinio Code. The Task Manager shows a single child process consuming 9,304 MB (the WebView/Rust backend process). Investigation across 5 parallel subagents found three categories of memory issues:

1. **Frontend (SolidJS WebView):** Messages array grows unbounded, no virtual scrolling (all messages in DOM), all workspaces kept mounted with `display: none`, `highlight.js` full language registry loaded
2. **Backend Rust — Session I/O:** `load_records()` called 11+ times per workflow round, each re-reading and deserializing the full JSONL file (~5-50 MB) into a `Vec<SessionRecord>`
3. **Backend Rust — Code Intel:** `load_all_embeddings()` loads the entire embedding table into RAM on every `search_by_embedding()` call

## Solution Design

### Frontend Optimizations

#### 1. Virtual Scrolling with @tanstack/virtual (CRITICAL)
- Install `@tanstack/virtual` (SolidJS bindings built-in)
- Replace `<For each={messages()}>` with a virtualized list
- Only render ~10 messages above + ~10 below the visible viewport
- Estimated memory reduction: DOM nodes drop from N (all messages) to ~30 (visible window)

#### 2. Inactive Workspace Message Buffering (CRITICAL)
- Unmount the ChatPanel for non-active workspaces using `<Show>` instead of `display: none`
- Replace background workspace's direct message signal with a bounded `AgentEvent` buffer (FIFO, max ~50 events)
- When the user switches back, replay the buffer and then reload full history from the JSONL via `load_session()`
- Estimated memory reduction: only 1 ChatPanel fully mounted at a time instead of N workspaces

#### 3. highlight.js Lazy Loading (MEDIUM)
- Replace `import hljs from "highlight.js"` (loads ALL languages) with dynamic imports
- Only load the specific language when rendering a code block
- For unknown languages, use a minimal fallback (no highlight or load only common langs)
- Estimated memory reduction: ~2-4 MB (highlight.js full registry + language data)

### Backend Rust Optimizations

#### 4. In-Memory LRU Cache for `load_records()` (CRITICAL)
- Add a `LruCache<PathBuf, (Vec<SessionRecord>, Instant)>` to `AppState` in `state.rs`
- Cache TTL of ~1 second — enough to survive the 11+ calls per round but short enough to pick up new appends
- The hot path in `session.rs` calls `load_records()` 11 times; the cache reduces this to 1 file read per round + 10 cache hits
- Create a thin `load_records_cached(path, cache)` function accessible from the workflow
- Estimated memory reduction: eliminates 10 redundant deserializations per round, saving ~50-500 MB of temporary heap allocations

#### 5. Paginated Embedding Scan in `search_by_embedding()` (CRITICAL)
- Replace `load_all_embeddings()` full-table-load with a paginated cursor
- Load 2000 rows at a time with `LIMIT ? OFFSET ?`, score them, keep a top-K min-heap
- This bounds heap allocation to `PAGE_SIZE * row_size` (~2000 * 1.6KB = ~3.2MB) regardless of total symbol count
- Estimated memory reduction: from N*1.6KB (potentially hundreds of MB) to ~3.2MB peak per search

## Risks

1. **@tanstack/virtual SolidJS compatibility:** SolidJS's fine-grained reactivity can conflict with virtual list libraries. Mitigation: use `@tanstack/solid-virtual` (explicit SolidJS support, not the plain `@tanstack/virtual`). Key area to test: row height estimation for variable-height messages (code blocks, images). Use `estimateSize` with a minimum and `measure()` on image load.

2. **Message buffer on workspace switch race condition:** If a background workspace receives events during the brief window between unmount and App.tsx taking over the listener, events could be lost. Mitigation: the Channel (Tauri IPC) listeners must be owned by App.tsx, not by ChatPanel. The panel registers/unregisters callbacks on mount/cleanup; the Channel stays alive.

3. **Embedding pagination scoring correctness:** Without a vector index, paginating by `LIMIT/OFFSET` means we only see the top-K of each page, not the global top-K across ALL rows. However, since the current code already loads EVERY row and sorts full-table, pagination changes semantics only if a well-scoring symbol is concentrated in a late page and page boundaries cut it. Mitigation: use a large page size (5000) so for most workspaces it's a single page. For monorepos, the per-row payload is small enough (~200 bytes for `(id, chunk_start, chunk_end, embedding)` stripped of SymbolRecord metadata) that a 5000-page is only ~1MB.

4. **Cache staleness of records_cache:** The 1-second TTL cache might return stale records if a different thread appends to the JSONL between consecutive reads. Mitigation: the workflow loop is sequential — all appends happen via the same task. After each append (`persist::append`), the cache entry for that path should be invalidated. The cache is just an optimization; stale reads at worst miss a recent append (caught on next TTL expiry).

5. **highlight.js async rendering creates visible flash:** Switching from synchronous `marked.parse` to async can cause a flash of unstyled text. Mitigation: render a placeholder `<code>` block immediately, then upgrade it when the highlight module loads. Or use `marked.parse` synchronously with a pre-loaded common-languages set.

## Non-goals

- NOT rewriting the model provider (provider.rs is fine — streaming is well-managed)
- NOT changing the session JSONL compaction strategy (already works; file rotation is out of scope)
- NOT replacing SQLite embeddings with FAISS/sqlite-vec (pagination achieves the same memory goal without new dependencies)
- NOT changing LSP manager architecture (max 2 servers per workspace, cleaned up on drop)
- NOT adding a full WebWorker system for markdown rendering (out of scope for this optimization pass)

## Low-Level Design

### 1. Virtual Scrolling — ChatPanel.tsx

**Target file:** `src/components/ChatPanel.tsx`

**New dependency:** `@tanstack/solid-virtual` in `package.json`

**Changes:**

1. Import `createVirtualizer`:
```typescript
import { createVirtualizer } from '@tanstack/solid-virtual';
```

2. Create virtualizer instance after messages signal:
```typescript
const parentRef = createRef<HTMLDivElement>();
const virtualizer = createVirtualizer({
  count: () => messages().length,
  getScrollElement: () => parentRef.current,
  estimateSize: () => 80, // conservative estimate; rows can be taller
  overscan: 5,
});

// Auto-scroll when new messages arrive while already at bottom
const prevLen = createRef(0);
createEffect(() => {
  const len = messages().length;
  if (len > prevLen.current) {
    prevLen.current = len;
    // Only auto-scroll if user hasn't scrolled up
    const el = parentRef.current;
    if (el && el.scrollHeight - el.scrollTop - el.clientHeight < 200) {
      virtualizer.scrollToIndex(len - 1);
    }
  }
});
```

3. Replace the render area (around line 1854). The existing `<div class="flex flex-1 flex-col overflow-y-auto">` with `ref={scrollContainerRef}` → change to use `parentRef` and virtualized rendering:
```tsx
<div ref={parentRef} class="flex-1 overflow-y-auto">
  <div style={{ height: `${virtualizer.getTotalSize()}px`, width: '100%', position: 'relative' }}>
    <For each={virtualizer.getVirtualItems()}>
      {(virtualRow) => {
        const msg = messages()[virtualRow.index];
        return (
          <div
            style={{
              position: 'absolute',
              top: 0,
              left: 0,
              width: '100%',
              height: `${virtualRow.size}px`,
              transform: `translateY(${virtualRow.start}px)`,
            }}
          >
            <div class="mb-6">
              {/* existing message rendering JSX — exactly the same as current code */}
            </div>
          </div>
        );
      }}
    </For>
  </div>
</div>
```

4. Update `handleScroll` — the virtualizer manages its own scroll state. Remove the old scroll handling logic or let it coexist. The existing `handleScroll` at line ~1850 does nothing critical (just scroll position tracking). Simplify to only call `virtualizer.getVirtualItems()` which is reactive.

5. Update the live text / TimelineSteps area (after the `<For>`, around line 1960) — this live area renders outside the virtualizer. Keep it as-is appended after the virtualized block (virtualizer sees `count = messages().length` and the live area renders as a separate div below the virtualized container).

**Integration points:**
- `scrollContainerRef` (line ~1850) — replaced by `parentRef`
- `handleScroll` — simplified, coexists with virtualizer
- Auto-scroll effect — new `createEffect` watching `messages().length`

### 2. Inactive Workspace Buffer — App.tsx + ChatPanel.tsx

**Target files:** `src/App.tsx`, `src/components/ChatPanel.tsx`, new `src/lib/workspaceBuffer.ts`

**New module — `src/lib/workspaceBuffer.ts`:**
```typescript
import type { AgentEvent } from './ipc';

const MAX_BUFFER = 100;
const buffers = new Map<string, AgentEvent[]>();

export function getBuffer(workspace: string): AgentEvent[] {
  let buf = buffers.get(workspace);
  if (!buf) {
    buf = [];
    buffers.set(workspace, buf);
  }
  return buf;
}

export function pushEvent(workspace: string, event: AgentEvent): void {
  const buf = getBuffer(workspace);
  buf.push(event);
  if (buf.length > MAX_BUFFER) {
    buf.splice(0, buf.length - MAX_BUFFER);
  }
}

export function drainBuffer(workspace: string): AgentEvent[] {
  const events = buffers.get(workspace) ?? [];
  buffers.delete(workspace);
  return events;
}
```

**Changes in `App.tsx` (around line 1580):**
The key insight: the Tauri IPC `Channel` listener must NOT live inside ChatPanel. ChatPanel currently creates the listener (via `sendMessage` return value or similar). We need to:

a) Keep the Channel listener at the App level, keyed by workspace path
b) Route events to the active ChatPanel's `setMessages` directly, or to the buffer if inactive

**Simplest approach (recommended):**
- In `App.tsx`, maintain two shared refs: `activeDispatch: Map<string, (event: AgentEvent) => void>`
- Each ChatPanel registers its event handler on mount via a callback prop: `onRegisterDispatch(ws: string, fn: (e: AgentEvent) => void)`
- When a workspace is inactive, App.tsx calls `pushEvent(ws, event)` instead
- When activated, ChatPanel receives buffered events on mount and processes them before attaching to live dispatch

**Changes in `ChatPanel.tsx`:**
- On `onMount`: call `drainBuffer(props.workspace)` and replay events into the message/step signals
- Register the dispatch handler via `props.onRegisterDispatch`
- On `onCleanup`: unregister (so App.tsx falls back to buffering)

### 3. Lazy highlight.js — ChatPanel.tsx + ToolBody.tsx

**Target files:** `src/components/ChatPanel.tsx`, `src/components/tool-renderers/ToolBody.tsx`

**Changes:**
1. Remove `import hljs from "highlight.js"` from both files
2. Create `src/lib/markdownRenderer.ts`:
```typescript
import { marked } from 'marked';

// Pre-load the most common languages
const COMMON_LANGS = ['typescript', 'javascript', 'rust', 'python', 'json', 'bash', 'html', 'css', 'markdown'];

const hljsCache = new Map<string, any>();

async function highlightCode(code: string, lang: string | undefined): Promise<string> {
  if (!lang) return escapeHtml(code);
  
  // Try to get the highlighter for this language
  if (!hljsCache.has(lang)) {
    try {
      const path = lang === 'ts' ? 'typescript' : lang === 'js' ? 'javascript' : lang;
      const mod = await import(`highlight.js/lib/languages/${path}`);
      hljsCache.set(lang, mod.default);
    } catch {
      // Language not found — escape and return
      return escapeHtml(code);
    }
  }
  const hl = hljsCache.get(lang);
  if (!hl) return escapeHtml(code);
  return hl.highlight(code, { language: lang }).value;
}

function escapeHtml(text: string): string {
  return text.replace(/&/g, '&amp;').replace(/</g, '&lt;').replace(/>/g, '&gt;');
}
```

3. Configure `marked`:
```typescript
const renderer = new marked.Renderer();
renderer.code = ({ text, lang }) => {
  // Return a placeholder div with the code data; the actual highlighting
  // happens async but for now we use the synchronous cached version
  const safeLang = lang || '';
  return `<pre><code class="hljs ${safeLang ? `language-${safeLang}` : ''}">${escapeHtml(text)}</code></pre>`;
};
marked.use({ renderer });
```

4. In ChatPanel.tsx, switch from `innerHTML={marked.parse(text, { async: false }) as string}` to using the async version. Since SolidJS doesn't easily support async rendering in `innerHTML`, the simplest approach: keep the synchronous renderer (it still escapes correctly) but only load COMMON_LANGS synchronously. The full `hljs` import is what's expensive, and removing it saves 2-4MB regardless of syntax highlighting.

5. **Simplest possible change (recommended):** Instead of dynamic imports, replace `import hljs from "highlight.js"` with:
```typescript
import typescript from 'highlight.js/lib/languages/typescript';
import javascript from 'highlight.js/lib/languages/javascript';
import rust from 'highlight.js/lib/languages/rust';
import python from 'highlight.js/lib/languages/python';
import json from 'highlight.js/lib/languages/json';
import bash from 'highlight.js/lib/languages/bash';
import html from 'highlight.js/lib/languages/xml'; // HTML uses 'xml' in highlight.js
import css from 'highlight.js/lib/languages/css';
import markdown from 'highlight.js/lib/languages/markdown';
```
Then register only these and use `highlight` instead of `highlightAuto`. This loads ~small fraction of the full registry and eliminates the auto-detect penalty.

### 4. LRU Cache for `load_records()` — persist.rs + state.rs + session.rs

**Target files:**
- `src-tauri/Cargo.toml` — add `lru = "0.12"`
- `src-tauri/src/agent/persist.rs` — add cached wrapper
- `src-tauri/src/state.rs` — add cache to `AppState`
- `src-tauri/src/agent/session.rs` — use cached version in workflow loop

**Detailed changes:**

**`Cargo.toml`:** Add `lru = "0.12"` to `[dependencies]`

**`persist.rs`:**
```rust
use std::time::Instant;
use lru::LruCache;
use std::num::NonZeroUsize;

pub fn load_records_cached(
    path: &Path,
    cache: &Mutex<LruCache<PathBuf, (Vec<SessionRecord>, Instant)>>
) -> Result<Vec<SessionRecord>, String> {
    let mut cache = cache.lock().unwrap();
    if let Some((records, cached_at)) = cache.get(path) {
        if cached_at.elapsed() < std::time::Duration::from_millis(800) {
            return Ok(records.clone());
        }
    }
    let records = load_records(path)?;
    cache.put(path.to_path_buf(), (records.clone(), Instant::now()));
    Ok(records)
}

/// Invalidate the cache entry for a path after a write.
pub fn invalidate_cache(
    path: &Path,
    cache: &Mutex<LruCache<PathBuf, (Vec<SessionRecord>, Instant)>>
) {
    let mut cache = cache.lock().unwrap();
    cache.pop(path);
}
```

**`state.rs`:**
Add field to `AppState`:
```rust
use lru::LruCache;
use std::num::NonZeroUsize;

// Inside AppState:
pub records_cache: Mutex<LruCache<PathBuf, (Vec<SessionRecord>, Instant)>>,

// Inside AppState::new():
records_cache: Mutex::new(LruCache::new(NonZeroUsize::new(64).unwrap())),
```

**`session.rs`:**
- The function `run_workflow_with_profile` receives `ctx: &ToolContext`. Add `records_cache` to `ToolContext` in `tools/mod.rs`
- Change all 11+ call sites from `load_records(&store.path)` to `load_records_cached(&store.path, &ctx.records_cache)`, except:
  - Right AFTER a `store.append()` call, call `invalidate_cache(&store.path, &ctx.records_cache)` to ensure fresh reads
- The initial load at `session.rs:138` (pre-flight) should always call the uncached version to get the absolute latest state

**`tools/mod.rs`:** Add `records_cache: Arc<Mutex<LruCache<...>>>` to `ToolContext`:
```rust
use lru::LruCache;
use std::num::NonZeroUsize;

pub struct ToolContext {
    // existing fields...
    pub records_cache: Arc<Mutex<LruCache<PathBuf, (Vec<SessionRecord>, Instant)>>>,
}
```

**`commands/agent.rs`:**
- When constructing `ToolContext`, pass `state.records_cache` clone
- The `load_session()` command always loads fresh from disk (it's called on navigation, not in the hot loop) — no caching needed

### 5. Paginated Embedding Scan — db.rs

**Target file:** `src-tauri/src/code_intel/db.rs`

**Changes:**

1. Rename existing `load_all_embeddings` → make it private, add a paginated variant:
```rust
const EMBEDDING_PAGE_SIZE: i64 = 2000;

/// Load a page of embeddings. Returns empty vec when no more rows.
pub fn load_embeddings_page(&self, page_size: i64, offset: i64) -> Result<Vec<(SymbolRecord, i64, i64, Vec<f32>)>, String> {
    let conn = self.conn.lock().map_err(|e| e.to_string())?;
    let mut stmt = conn
        .prepare(
            "SELECT s.id, s.file_id, s.name, s.kind, s.signature,
                    s.start_line, s.start_col, s.end_line, s.end_col, f.path,
                    e.start_line, e.end_line, e.embedding
             FROM symbols s
             JOIN files f ON f.id = s.file_id
             JOIN symbol_embeddings e ON e.symbol_id = s.id
             ORDER BY s.id, e.start_line
             LIMIT ? OFFSET ?",
        )
        .map_err(|e| format!("prepare: {e}"))?;
    // ... same deserialization as load_all_embeddings ...
    let results = stmt
        .query_map(rusqlite::params![page_size, offset], |row| { /* same */ })?
        .filter_map(|r| r.ok())
        .collect();
    Ok(results)
}
```

2. Rewrite `search_by_embedding`:
```rust
pub fn search_by_embedding(
    &self,
    query_text: &str,
    query_vec: &[f32],
    limit: usize,
) -> Result<Vec<SemanticSearchResult>, String> {
    let tokens = tokenize_query(query_text);
    let page_size: i64 = EMBEDDING_PAGE_SIZE;
    let mut offset: i64 = 0;
    let mut best_per_symbol: HashMap<i64, SemanticSearchResult> = HashMap::new();

    loop {
        let page = self.load_embeddings_page(page_size, offset)?;
        if page.is_empty() {
            break;
        }
        for (sym, chunk_start, chunk_end, emb) in page {
            if emb.len() != query_vec.len() { continue; }
            // same scoring logic as current code...
        }
        offset += page_size;
    }

    // same dedup + truncation as current code...
}
```

3. Keep existing tests — they use `search_by_embedding` externally, which still has the same interface. The tests insert small numbers of rows so they'll complete in one page.

## Tasks Summary

1. **golden-frontend-virtual-scroll** — Install `@tanstack/solid-virtual`, replace `<For each={messages()}>` with virtualized rendering in ChatPanel.tsx. Update scroll handling and auto-scroll logic.
2. **golden-frontend-workspace-buffer** — Create `src/lib/workspaceBuffer.ts`, refactor `App.tsx` to route events to buffer when workspace inactive, update ChatPanel mount/cleanup.
3. **golden-frontend-lazy-highlight** — Replace full `import hljs from "highlight.js"` with selective language imports in ChatPanel.tsx and ToolBody.tsx.
4. **golden-backend-records-cache** — Add `lru` dependency, add cache to `AppState`, modify `persist.rs` with cached wrapper, update `ToolContext`, change 11 load_records call sites in `session.rs` to use cache.
5. **golden-backend-embedding-pagination** — Rewrite `load_all_embeddings` as paginated `load_embeddings_page`, update `search_by_embedding` with pagination loop in `db.rs`.

Each task references concrete file paths, symbols, and values from this Low-Level Design.


## Implementation Log — 2026-07-16 16:49
**Summary:** Workspace event buffer: unmount inactive ChatPanels via <Show>, buffer AgentEvents in-flight for replayed on re-activation
**Changed files:** M package.json, M pnpm-lock.yaml, M src-tauri/Cargo.lock, M src-tauri/Cargo.toml, M src-tauri/src/agent/persist.rs, M src-tauri/src/agent/session.rs, M src-tauri/src/agent/tools/bash.rs, M src-tauri/src/agent/tools/finalize_plan.rs, M src-tauri/src/agent/tools/mod.rs, M src-tauri/src/agent/tools/tasks.rs, M src-tauri/src/agent/tools/write_plan.rs, M src-tauri/src/code_intel/db.rs, M src-tauri/src/commands/agent.rs, M src-tauri/src/state.rs, M src/App.tsx, M src/components/ChatPanel.tsx, M src/components/tool-renderers/ToolBody.tsx, ?? docs/plans/2026-07-16_2026-07-21-memory-optimization.md, ?? src/lib/workspaceBuffer.ts
**Commits:** _(git unavailable or none)_
**Journal:** ## Workspace Event Buffer

**Key design:** The workspace buffer solves a fundamental problem with `<Show>` unmounting. When a ChatPanel is unmounted (no longer the active workspace), its `handleEvent` callback that was passed to `sendMessage()` (via Tauri's `Channel`) would otherwise start throwing when it tries to call `setMessages()` or `setCurrentSteps()` on a destroyed component. 

**Solution:** Three coordinated changes:
1. **`src/lib/workspaceBuffer.ts`**: A module-level `Map<string, AgentEvent[]>` with `pushEvent()` (FIFO cap 100), `drainBuffer()`, and helpers. Follows the same pattern as the existing `workspaceStatus.ts` (module-level store + exported functions).

2. **`App.tsx`**: Replaced `display:none` div wrappers with `<Show when={activeWorkspace() === ws}>` for both ChatPanel and TasksPanel. This allows inactive components to fully unmount, freeing their DOM memory and component state.

3. **`ChatPanel.tsx`**:
   - Added `if (!props.isActive()) { pushEvent(props.workspace, event); return; }` as the first guard in `handleEvent` — when a workspace becomes inactive mid-send, subsequent events from the still-running IPC `Channel` get buffered instead of calling setters on a destroyed component.
   - Added buffer draining in `onMount` — when the panel re-mounts (user switches back), it drains any buffered events, replays them through `handleEvent` to restore the live state, and scrolls to bottom.
   - `onMount` and `onCleanup` patterns (ESC listener, drag-drop, global keydown) already had `if (!props.isActive()) return;` guards — no changes needed there.

**Gotcha:** The `sendMessage()` call in `ipc.ts` creates a `new Channel<AgentEvent>()` and assigns `channel.onmessage = onEvent`. The `Channel` object lives on the Tauri IPC layer — even if the component unmounts, the channel stays alive and fires callbacks. The buffer guard catches these and stores them safely.

**Verification:** `npx tsc --noEmit` — 0 new errors (only pre-existing ones in test files and the `_text` unused variable).

**Task journal:**
- Unmount inactive workspaces, buffer AgentEvents: Created src/lib/workspaceBuffer.ts with getBuffer, pushEvent, drainBuffer, hasBufferedEvents, clearBuffer — module-level Map<string, AgentEvent[]> with MAX_BUFFER=100 FIFO cap; App.tsx: Replaced display:none div wrappers with <Show when={activeWorkspace()===ws}> for both ChatPanel and TasksPanel — inactive panels now unmount fully, freeing DOM memory; ChatPanel.tsx: Added `if (!props.isActive()) { pushEvent(props.workspace, event); return; }` as first guard in handleEvent — events from still-running IPC calls get buffered; ChatPanel.tsx: Added buffer replay in onMount — on remount, drains buffered events and replays them through handleEvent, then scrolls to bottom; Updated stale comment in App.tsx; Verified: npx tsc --noEmit shows 0 new errors (only pre-existing ones)
- Virtual Scroll with @tanstack/solid-virtual in ChatPanel.tsx: Added createVirtualizer import from @tanstack/solid-virtual (v3); Added virtualizer instance after scrollToBottom; count uses messages().length (v3 expects number, not getter); estimateSize heuristic: 80-500px based on text length, steps, and role; Replaced static <For each={messages()}> with virtualized rendering using virtualizer.getVirtualItems() with absolute positioning (transform: translateY); Updated scrollToBottom to use virtualizer.scrollToIndex with align:'end' with scrollIntoView fallback; Container uses position:relative + min-height from virtualizer.getTotalSize(); Overscan set to 5 items; Verified: npx tsc --noEmit shows 0 new errors (only pre-existing _text warning)
- Lazy highlight.js imports: Removed full import hljs from highlight.js; Replaced with core + selective languages; Eliminated highlightAuto overhead; Applied to ChatPanel.tsx and ToolBody.tsx; Verified with npx tsc --noEmit
- LRU cache for load_records(): Added lru crate, load_records_cached with 800ms TTL; Propagated through ToolContext across 6 files; cargo check — 0 errors, 0 warnings
- Paginated embedding scan in db.rs: Added EMBEDDING_PAGE_SIZE=2000, load_embeddings_page(); Rewrote search_by_embedding() with paginated loop; 2 tests pass unchanged
