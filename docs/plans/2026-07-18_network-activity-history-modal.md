# Network Activity History Modal

## Context

The user wants to click on the existing `NetworkIndicator` globe icon and see a modal showing all past network requests with their stats. Currently the indicator only shows **active** operations in a hover tooltip. There is no history — once a `NetGuard` drops, the operation is gone forever.

**User request:** "Quando clicar no network activity, gostaria de ver uma modal com todos os requests que sairam e suas stats, guarde no maximo 100 ultimas linhas para serem apresentada por projeto."

## Solution Design

### Storage

- **CSV file** at `dirs::config_dir()/claudinio-code/network-log.csv`
- Format: `workspace,timestamp,source,detail,duration_ms,bytes,status_code`
- Backend (Rust) appends one row per completed request when the `NetGuard` is dropped.
- The frontend reads the last 100 lines **filtered by workspace** via a Tauri command.

### New Fields on NetGuard

- `set_status(u16)` — call site sets the HTTP status code after the response arrives. Optional: if never called, the column stays empty.
- Workspace captured at `begin()` time from a global `set_current_workspace()`, stored in `NetOp`.

### Reading the History

- New Tauri command: `get_network_log(workspace: String) -> Vec<LogEntry>`
- Reads the CSV, filters by workspace, returns the last 100 rows (LIFO — newest first).

### UI: Modal

- **Trigger:** clicking the `NetworkIndicator` globe button.
- **Layout:** centered modal (~640×500px), following the `TextEditorModal` overlay pattern.
- **Visual:** timeline with colored dots per row:
  - 🟢 green = 2xx
  - 🟡 yellow = 3xx/4xx
  - 🔴 red = 5xx
  - ⚪ gray = no status code
- **Content per row:** source name, detail, duration, bytes, status code badge.
- **Behavior:** snapshot on open — no auto-refresh. Closes on Escape or backdrop click.
- **Empty state:** "No requests logged for this workspace."

### Per-Project Filtering

- Workspace root path stored in CSV. `get_network_log` filters by it.
- Global `set_current_workspace(String)` called by the `open_workspace` command.
- `NetGuard::begin()` reads the global, stores it in the `NetOp`, writes it at `drop()`.

### Scope

- **In:** CSV append on `NetGuard::drop()`, `set_status()` method, `get_network_log` command, `NetworkActivityModal` component, wiring workspace to `NetworkIndicator`, status codes for LLM and Auth paths.
- **Out:** status code plumbing to all 10+ call sites (only LLM + Auth initially). No auto-refresh. No export/clear/delete buttons. No rotation/cleanup of CSV.

### Non-goals

- No UI to clear/download the CSV.
- No frontend HTTP tracking (there is none).
- No request/response body logging.
- No per-request timing breakdown (DNS, TLS, TTFB).
- No auto-refresh/polling.

## Risks

- **CSV concurrent writes:** multiple NetGuards may drop simultaneously. Mitigation: use a `std::sync::Mutex` around file append. Writes are tiny (~150 bytes each).
- **CSV grows unbounded:** over months of heavy use it could reach tens of MBs. Acceptable for now — only the last 100 rows are shown. Future: rotation at ~10MB.
- **Workspace global race:** if the user switches workspaces while a request is in-flight, the request logs under the new workspace. Mitigation: capture the workspace at `begin()` time, not at `drop()` time. Stored in `NetOp`.
- **CSV on Windows:** `dirs::config_dir()` returns `C:\Users\<user>\AppData\Roaming` which may have permission issues. Mitigation: use `dirs::data_dir()` if `config_dir()` fails (same pattern as existing code).

## Low-Level Design

### Files to touch

| File | Change |
|------|--------|
| `src-tauri/Cargo.toml` | Add `csv = "1"` dependency |
| `src-tauri/src/net_activity.rs` | Add workspace storage, status tracking, CSV writer, `set_status()`, `set_current_workspace()` |
| `src-tauri/src/commands/mod.rs` | Add `pub mod network_log;` |
| `src-tauri/src/commands/network_log.rs` | **New file** — `get_network_log` Tauri command |
| `src-tauri/src/commands/open_workspace.rs` (or `agent.rs`) | Call `set_current_workspace()` when workspace is opened |
| `src-tauri/src/lib.rs` | Register `set_current_workspace` and `get_network_log` commands |
| `src-tauri/src/agent/provider.rs` | Add `guard.set_status(code)` for LLM calls |
| `src-tauri/src/commands/auth.rs` | Add `guard.set_status(code)` for auth calls |
| `src/lib/ipc.ts` | Add `getNetworkLog()` and `setCurrentWorkspace()` |
| `src/lib/networkActivity.ts` | Add `LogEntry` interface |
| `src/components/NetworkActivityModal.tsx` | **New file** — timeline modal component |
| `src/components/NetworkIndicator.tsx` | Add `onClick` and `workspace` props |
| `src/components/ChatPanel.tsx` | Wire `workspace`, modal state, render `<NetworkActivityModal>` |
| `src/lib/locales/en-US.ts` | Add `net.modal.*` keys |
| `src/lib/locales/pt-BR.ts` | Add `net.modal.*` keys (Portuguese) |

### Detailed Changes

#### 1. `src-tauri/Cargo.toml` — add csv crate

```toml
csv = "1"
```

#### 2. `src-tauri/src/net_activity.rs` — major changes

**Add to imports:**
```rust
use std::fs::OpenOptions;
use std::io::Write;
use csv::Writer;
```

**Add workspace global:**
```rust
static CURRENT_WORKSPACE: OnceLock<String> = OnceLock::new();

pub fn set_current_workspace(workspace: String) {
    let _ = CURRENT_WORKSPACE.set(workspace);
}
```

**Extend `NetOp` struct:**
```rust
struct NetOp {
    id: u64,
    source: NetSource,
    detail: String,
    started: Instant,
    bytes: u64,
    workspace: String,       // NEW — captured at begin()
    status_code: Option<u16>, // NEW — set via set_status()
}
```

**Extend `NetOpView` (frontend snapshot):**
```rust
pub struct NetOpView {
    pub id: u64,
    pub source: NetSource,
    pub detail: String,
    pub elapsed_ms: u64,
    pub bytes: u64,
    pub status_code: Option<u16>, // NEW
}
```

**Update `emit_snapshot()`** to include `status_code` in the view.

**Update `NetGuard::begin()`** to accept workspace from the global:
```rust
pub fn begin(source: NetSource, detail: impl Into<String>) -> Self {
    let id = NEXT_ID.fetch_add(1, Ordering::Relaxed);
    let workspace = CURRENT_WORKSPACE.get().cloned().unwrap_or_default();
    if let Ok(mut ops) = tracker().ops.lock() {
        ops.push(NetOp {
            id,
            source,
            detail: detail.into(),
            started: Instant::now(),
            bytes: 0,
            workspace,
            status_code: None,
        });
    }
    emit_snapshot();
    NetGuard { id }
}
```

**Add `set_status()` method:**
```rust
impl NetGuard {
    pub fn set_status(&self, code: u16) {
        if let Ok(mut ops) = tracker().ops.lock() {
            if let Some(op) = ops.iter_mut().find(|op| op.id == self.id) {
                op.status_code = Some(code);
            }
        }
    }
}
```

**CSV write helper:**
```rust
static CSV_MUTEX: std::sync::Mutex<()> = std::sync::Mutex::new(());

fn csv_path() -> std::path::PathBuf {
    // Follow existing pattern: dirs::config_dir() + "claudinio-code"
    dirs::config_dir()
        .unwrap_or_else(|| std::path::PathBuf::from("."))
        .join("claudinio-code")
        .join("network-log.csv")
}

fn append_csv_row(op: &NetOp) {
    let path = csv_path();
    // Ensure directory exists
    if let Some(parent) = path.parent() {
        let _ = std::fs::create_dir_all(parent);
    }
    let _lock = CSV_MUTEX.lock().unwrap();
    let file_exists = path.exists();
    let file = OpenOptions::new()
        .create(true)
        .append(true)
        .open(&path);
    if let Ok(file) = file {
        let mut wtr = Writer::from_writer(file);
        // Write header if new file
        if !file_exists {
            let _ = wtr.write_record(&[
                "workspace", "timestamp", "source", "detail",
                "duration_ms", "bytes", "status_code",
            ]);
        }
        let _ = wtr.write_record(&[
            &op.workspace,
            &chrono::Utc::now().to_rfc3339(),
            &format!("{:?}", op.source), // snake_case variant name
            &op.detail,
            &op.started.elapsed().as_millis().to_string(),
            &op.bytes.to_string(),
            &op.status_code.map(|c| c.to_string()).unwrap_or_default(),
        ]);
        let _ = wtr.flush();
    }
}
```

Wait — `chrono` may not be a dependency. Let me check... Actually I can use `std::time::SystemTime` or just use a simple timestamp. Let me check what's available. The `dirs` crate IS already a dependency. But `chrono` may not be. Let me use a simpler approach:

```rust
use std::time::{SystemTime, UNIX_EPOCH};

fn unix_timestamp() -> u64 {
    SystemTime::now()
        .duration_since(UNIX_EPOCH)
        .unwrap_or_default()
        .as_secs()
}
```

**Update `NetGuard::drop()`** to write CSV before removing:
```rust
impl Drop for NetGuard {
    fn drop(&mut self) {
        if let Ok(mut ops) = tracker().ops.lock() {
            if let Some(op) = ops.iter().find(|op| op.id == self.id) {
                append_csv_row(op);
            }
            ops.retain(|op| op.id != self.id);
        }
        emit_snapshot();
    }
}
```

#### 3. `src-tauri/src/commands/network_log.rs` — NEW FILE

```rust
use serde::Serialize;

#[derive(Serialize)]
#[serde(rename_all = "camelCase")]
pub struct LogEntry {
    pub workspace: String,
    pub timestamp: String,  // unix seconds as string for simplicity
    pub source: String,
    pub detail: String,
    pub duration_ms: u64,
    pub bytes: u64,
    pub status_code: Option<u16>,
}

#[tauri::command]
pub fn get_network_log(workspace: String) -> Result<Vec<LogEntry>, String> {
    let path = crate::net_activity::csv_path(); // make csv_path pub(crate)
    if !path.exists() {
        return Ok(vec![]);
    }
    let mut rdr = csv::ReaderBuilder::new()
        .has_headers(true)
        .from_path(&path)
        .map_err(|e| e.to_string())?;

    let mut entries: Vec<LogEntry> = Vec::new();
    for result in rdr.records() {
        let record = result.map_err(|e| e.to_string())?;
        if record.get(0).unwrap_or("") != workspace { continue; }
        entries.push(LogEntry {
            workspace: record.get(0).unwrap_or("").to_string(),
            timestamp: record.get(1).unwrap_or("").to_string(),
            source: record.get(2).unwrap_or("").to_string(),
            detail: record.get(3).unwrap_or("").to_string(),
            duration_ms: record.get(4).unwrap_or("0").parse().unwrap_or(0),
            bytes: record.get(5).unwrap_or("0").parse().unwrap_or(0),
            status_code: record.get(6).and_then(|s| s.parse().ok()),
        });
    }
    // Return last 100, newest first
    entries.reverse();
    entries.truncate(100);
    Ok(entries)
}
```

#### 4. `src-tauri/src/commands/mod.rs`

Add: `pub mod network_log;`

#### 5. `src-tauri/src/lib.rs`

In `generate_handler![...]`:
```rust
commands::network_log::get_network_log,
```

Also register `set_current_workspace`:
```rust
#[tauri::command]
fn set_current_workspace(workspace: String) {
    crate::net_activity::set_current_workspace(workspace);
}
```

Or, simpler: call `set_current_workspace` from the existing `open_workspace` command (in `commands/agent.rs` or wherever `open_workspace` is defined). Let me check...

Actually, looking at the IPC list, there's `openWorkspace(path)` that takes a Channel. We can add the call there. The Rust `open_workspace` command in `commands/agent.rs` (or wherever) already receives the path.

#### 6. Status code wiring — `src-tauri/src/agent/provider.rs`

For the LLM call sites, after getting the response:
```rust
// After the reqwest response:
let status = resp.status().as_u16();
guard.set_status(status);
```

Affected locations (all 3 provider.rs NetGuard sites):
- `LlmStream` (~line 795) — after getting the streaming response
- `LlmClassify` (~line 577) — after getting the response
- `LlmOneShot` (~line 654) — after getting the response

#### 7. Status code wiring — `src-tauri/src/commands/auth.rs`

Both auth paths:
- `Auth` (~line 183) — after login exchange response
- `Auth` (~line 257) — after API key validation response

#### 8. `src/lib/ipc.ts` — add IPC wrapper

```typescript
export interface LogEntry {
  workspace: string;
  timestamp: string;
  source: string;
  detail: string;
  durationMs: number;
  bytes: number;
  statusCode?: number;
}

export function getNetworkLog(workspace: string): Promise<LogEntry[]> {
  return invoke<LogEntry[]>("get_network_log", { workspace });
}
```

#### 9. `src/lib/networkActivity.ts` — minor update

The `NetOp` interface already exists. We also export the `LogEntry` type (or re-export from ipc.ts).

No changes needed to signals — `activeOps` works as-is. The `LogEntry` type is for the modal.

#### 10. `src/components/NetworkActivityModal.tsx` — NEW FILE

Following `TextEditorModal` pattern:
- `fixed inset-0 z-50 flex items-center justify-center bg-black/40` backdrop
- Panel: `w-[640px] max-h-[500px] flex-col rounded-xl bg-surface-0 shadow-2xl`
- Header: title + close button
- Body: scrollable timeline list
- Each row: dot (color-coded) + source name + detail + duration + bytes + status badge
- Empty state when no entries

Component signature:
```tsx
const NetworkActivityModal: Component<{
  workspace: string;
  onClose: () => void;
}> = (props) => {
  const [entries, setEntries] = createSignal<LogEntry[]>([]);
  const [loading, setLoading] = createSignal(true);

  onMount(() => {
    getNetworkLog(props.workspace).then((data) => {
      setEntries(data);
      setLoading(false);
    });
  });

  // Escape key listener
  // ...

  return (
    <div class="fixed inset-0 z-50 flex items-center justify-center bg-black/40"
         onClick={(e) => { if (e.target === e.currentTarget) props.onClose(); }}>
      <div class="flex w-[640px] max-h-[500px] flex-col rounded-xl bg-surface-0 shadow-2xl">
        {/* Header */}
        <div class="flex items-center justify-between border-b border-border-subtle px-5 py-3">
          <span class="font-semibold text-ink">{t("net.modal.title")}</span>
          <button onClick={props.onClose} class="rounded-md p-1 hover:bg-surface-2">
            <Icon name="x" class="h-4 w-4 text-ink-faint" />
          </button>
        </div>
        {/* Body - scrollable */}
        <div class="flex-1 overflow-y-auto p-4">
          <Show when={!loading()} fallback={<p>Loading...</p>}>
            <Show when={entries().length > 0}
                  fallback={<p class="text-ink-muted text-sm">{t("net.modal.empty")}</p>}>
              <div class="space-y-0">
                <For each={entries()}>
                  {(entry) => (
                    <div class="flex items-start gap-3 py-2 border-l-2 border-border-subtle pl-3">
                      {/* Status dot */}
                      <div class={statusDotClass(entry.statusCode)} />
                      <div class="flex-1 min-w-0">
                        <div class="flex items-center gap-2">
                          <span class="text-xs font-medium text-ink">{sourceLabel(entry.source)}</span>
                          <Show when={entry.statusCode}>
                            <span class={statusBadgeClass(entry.statusCode)}>{entry.statusCode}</span>
                          </Show>
                        </div>
                        <Show when={entry.detail}>
                          <p class="text-[11px] text-ink-muted truncate">{entry.detail}</p>
                        </Show>
                        <div class="flex gap-3 text-[10px] text-ink-faint mt-0.5">
                          <span>{formatDuration(entry.durationMs)}</span>
                          <Show when={entry.bytes > 0}>
                            <span>{formatBytes(entry.bytes)}</span>
                          </Show>
                        </div>
                      </div>
                    </div>
                  )}
                </For>
              </div>
            </Show>
          </Show>
        </div>
      </div>
    </div>
  );
};
```

Status dot CSS classes (Tailwind):
- 2xx: `bg-green-500` (or `bg-emerald-500`)
- 3xx/4xx: `bg-yellow-500` (or `bg-amber-500`)
- 5xx: `bg-red-500`
- none: `bg-gray-400`

#### 11. `src/components/NetworkIndicator.tsx` — add onClick + workspace

- Add `workspace` and `onClick` props
- The `<button>` gets `onClick={props.onClick}`

```tsx
export const NetworkIndicator: Component<{
  placement?: 'top' | 'bottom';
  workspace: string;
  onClick: () => void;
}> = (props) => {
  // ...
  <button onClick={props.onClick} ...>
```

#### 12. `src/components/ChatPanel.tsx` — wire modal

```tsx
// New state
const [showNetModal, setShowNetModal] = createSignal(false);

// Replace <NetworkIndicator /> with:
<NetworkIndicator workspace={props.workspace} onClick={() => setShowNetModal(true)} />

// At the end of the component, after other modals:
<Show when={showNetModal()}>
  <NetworkActivityModal workspace={props.workspace} onClose={() => setShowNetModal(false)} />
</Show>
```

Both render sites (line 1865 and 3249) need updating.

#### 13. Locale keys — `en-US.ts` and `pt-BR.ts`

New keys:
```
"net.modal.title": "Network Log",
"net.modal.empty": "No requests logged for this workspace.",
```

Portuguese:
```
"net.modal.title": "Registro de Rede",
"net.modal.empty": "Nenhum request registrado para este projeto.",
```

### CSV Format

```
workspace,timestamp,source,detail,duration_ms,bytes,status_code
/Users/victor/project,1711929600,LlmStream,gpt-4o,1234,5678,200
/Users/victor/project,1711929601,Auth,login exchange,234,
```

- `source` is the `Debug` representation (snake_case): `LlmStream`, `Auth`, etc.
- `timestamp` is Unix seconds.
- `status_code` is empty when not set.

The frontend maps `source` via the existing `t("net.source.{source}")` locale keys (already lowercase — `LlmStream` → `llm_stream` matches the key).

Actually wait — the Rust `Debug` output for `NetSource::LlmStream` will be `"LlmStream"` (PascalCase), but the locale keys use `snake_case`. We need to use the `Serialize` representation (which IS snake_case thanks to `#[serde(rename_all = "snake_case")]`). So in the CSV we should write the serde name, not the Debug name.

Fix: write `op.source` to CSV using serde serialization. Easiest: `serde_json::to_string(&op.source)` or just use a match. Actually, simplest: add a method to `NetSource` that returns the snake_case string.

Let me just use the serde representation. We can call `serde_json::to_string(&op.source).unwrap().trim_matches('"')` ... no, that's ugly. Let me add a helper:

```rust
impl NetSource {
    fn as_str(&self) -> &'static str {
        match self {
            NetSource::LlmStream => "llm_stream",
            NetSource::LlmClassify => "llm_classify",
            // ... etc
        }
    }
}
```

Actually, even simpler — just use the fact that serde already serializes it as snake_case. We can use a unit variant to string:

```rust
fn source_to_str(source: NetSource) -> &'static str {
    // The serde rename_all = "snake_case" gives us these exact strings:
    match source {
        NetSource::LlmStream => "llm_stream",
        NetSource::LlmClassify => "llm_classify",
        NetSource::LlmOneShot => "llm_one_shot",
        NetSource::ListModels => "list_models",
        NetSource::Auth => "auth",
        NetSource::SkillsIndex => "skills_index",
        NetSource::SkillFetch => "skill_fetch",
        NetSource::EmbeddingModelDownload => "embedding_model_download",
        NetSource::WebSearch => "web_search",
        NetSource::Mcp => "mcp",
    }
}
```

This is explicit and works perfectly with the existing locale keys.

### Data Flow

```
[open_workspace command]    [LLM/Auth request completes]
        │                              │
        ▼                              ▼
set_current_workspace(path)    guard.set_status(code)
        │                              │
        │                        NetGuard::drop()
        │                              │
        │                        append_csv_row() ───► network-log.csv
        │
        ▼
 ┌──────────────────┐
 │  User clicks     │
 │  NetworkIndicator │
 └──────┬───────────┘
        │
        ▼
 showNetModal = true
        │
        ▼
 NetworkActivityModal.onMount()
        │
        ▼
 invoke("get_network_log", { workspace })
        │
        ▼
  Read CSV → filter by workspace → last 100 → reverse → return JSON
        │
        ▼
  Render timeline
```

### Existing patterns reused

| Pattern | Source | Used in |
|---------|--------|---------|
| Modal overlay + centering | `TextEditorModal.tsx` | `NetworkActivityModal.tsx` |
| Global `OnceLock<T>` for app-wide state | `net_activity.rs` (Tracker), `askpass.rs` (APP) | `CURRENT_WORKSPACE` |
| `#[tauri::command]` + `invoke<T>()` | All IPC functions | `get_network_log` |
| `dirs::config_dir()` for app data | `provider.rs`, `persist.rs`, `skills.rs` | CSV path |
| `Mutex<()>` for file synchronization | N/A (new pattern for this codebase) | CSV writer |
| `t(key, ...args)` with `{0}` substitution | `grill-me.ts` | Modal locale strings |
| Design tokens (`bg-surface-0`, `text-ink`, etc.) | `TextEditorModal.tsx` | Modal panel |

## Tasks Summary

1. Add `csv` crate to Cargo.toml
2. Extend `net_activity.rs`: workspace global, status field, `set_status()`, `set_current_workspace()`, CSV writer, source-to-str helper
3. Create `commands/network_log.rs` with `get_network_log` command
4. Register command in `commands/mod.rs` and `lib.rs`
5. Wire `set_current_workspace()` into the `open_workspace` command
6. Wire `guard.set_status()` in `provider.rs` (3 sites) and `auth.rs` (2 sites)
7. Add IPC wrappers in `ipc.ts`
8. Add locale keys in `en-US.ts` and `pt-BR.ts`
9. Create `NetworkActivityModal.tsx` component
10. Update `NetworkIndicator.tsx` with `onClick` and `workspace` props
11. Wire modal state + rendering in `ChatPanel.tsx`


## Implementation Log — 2026-07-18 10:58
**Summary:** Network activity history modal: clickable globe icon shows last 100 requests per workspace with status-coded timeline
**Changed files:** M src-tauri/Cargo.lock, M src-tauri/Cargo.toml, M src-tauri/src/agent/provider.rs, M src-tauri/src/commands/auth.rs, M src-tauri/src/commands/code_intel.rs, M src-tauri/src/commands/mod.rs, M src-tauri/src/lib.rs, M src-tauri/src/net_activity.rs, M src/components/ChatPanel.tsx, M src/components/NetworkIndicator.tsx, M src/lib/ipc.ts, M src/lib/locales/en-US.ts, M src/lib/locales/pt-BR.ts, ?? docs/plans/2026-07-18_network-activity-history-modal.md, ?? src-tauri/src/commands/network_log.rs, ?? src/components/NetworkActivityModal.tsx
**Commits:** _(git unavailable or none)_
**Journal:** Implementation went smoothly. Key findings:

1. **chrono with clock feature already available** — no need to add a new dependency or use raw Unix timestamps. Used `chrono::Utc::now().to_rfc3339()` for human-readable timestamps in CSV.

2. **ThinkingBar couldn't take onClick** — the second NetworkIndicator inside the ThinkingBar sub-component doesn't have access to workspace/state. Made `workspace` and `onClick` props optional with `?.()` safe call, keeping the thinking-bar indicator as status-only (no click trigger). The clickable globe is in the header bar.

3. **dirs::config_dir() works without explicit `use dirs`** — just using the qualified path in net_activity.rs compiles fine since dirs is already a dependency.

4. **NetGuard::set_status() calls were inserted carefully** — LlmStream waits until after the status check to create the guard (it only creates the guard for successful streams), so set_status is called right after the guard is created. LlmClassify and LlmOneShot set status right before the `!response.status().is_success()` check.

5. **CSV format uses serde snake_case names** — source_to_str() maps each NetSource variant to the exact strings used in the frontend locale keys (net.source.llm_stream, etc.), ensuring the modal displays translated labels correctly.

6. **open_workspace sets CURRENT_WORKSPACE first** — before the early return for already-open workspaces, so switching between workspaces correctly updates the log filter.

**Task journal:**
- CSV logging infrastructure (Rust backend): cargo check passes clean. 5 pre-existing warnings only. All 12 sub-changes implemented.
- Tauri command get_network_log + wiring: Created network_log.rs with LogEntry struct + get_network_log command. Registered in mod.rs and lib.rs. Wired set_current_workspace in code_intel.rs. cargo check passes clean.
- Status code wiring (provider.rs + auth.rs): All 5 set_status sites wired: auth.rs login exchange, auth.rs API key validation, provider.rs LlmClassify, LlmOneShot, LlmStream. cargo check passes clean, set_status warning gone.
- Frontend: types, IPC, locale: Added LogEntry interface and getNetworkLog() to ipc.ts. Added net.modal.title/empty to both en-US and pt-BR locale files.
- NetworkActivityModal component: Created NetworkActivityModal.tsx with full timeline UI: status dot coloring (green/yellow/red/gray), source names via i18n, status badges, detail/duration/bytes display, Escape + backdrop close, loading and empty states.
- Wire NetworkIndicator + Modal in ChatPanel: Updated NetworkIndicator with optional workspace? and onClick? props. Wired header bar indicator with modal trigger. ThinkingBar indicator kept as status-only (no click). Imported NetworkActivityModal, added showNetModal signal, added <Show> block for the modal after ContentViewerModal. Fixed optional props to maintain backward compat.
