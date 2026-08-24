# Architecture

How Claudinio Code is put together, and which of its shapes are load-bearing.
Read this before a change that crosses a module boundary; `CONTRIBUTING.md`
covers the build and the checks.

## The layers

```
┌──────────────────────────────────────────────────────────────┐
│  src/  — SolidJS frontend                                    │
│  chat timeline · approvals · Monaco diffs · file tree        │
└──────────────────────────┬───────────────────────────────────┘
                           │  Tauri IPC (invoke + Channel events)
┌──────────────────────────┴───────────────────────────────────┐
│  src-tauri/src/                                              │
│                                                              │
│    commands/     the IPC surface — thin adapters             │
│         │                                                    │
│         ▼                                                    │
│    agent/        loop, providers, tools, subagents,          │
│                  permissions, skills, MCP, persistence       │
│    code_intel/   tree-sitter → SQLite FTS5 + ONNX vectors    │
│    lsp/          language-server client                      │
│    workspace_path.rs, procutil.rs, http.rs, askpass.rs       │
└──────────────────────────────────────────────────────────────┘
```

**The dependency rule: `commands/` depends on the core; the core never depends
on `commands/`.** A `#[tauri::command]` should unwrap its arguments, call into
`agent/`, `code_intel/` or `lsp/`, and map the result — nothing more. Anything
with logic worth testing belongs below it, where it can be tested without a
Tauri runtime.

This is enforced, not just documented: `architecture_tests` in `lib.rs` fails
the build if `agent/`, `code_intel/` or `lsp/` import `crate::commands`. Three
things violated it historically and are worth knowing about, because they are
the shapes that tempt you back:

- a platform helper with no IPC in it (`procutil.rs`) filed under `commands/`
  because a command was its first caller;
- JSONL persistence for tasks living in `commands/tasks.rs` next to the commands
  that expose it (now in `agent/persist.rs`, which already owns the store);
- a global `INDEX_SEMAPHORE` in `commands/code_intel.rs` that the file watcher
  had to reach back for (now in `code_intel/mod.rs`).

## What one message does

```
send_message (commands/agent.rs)
   └─ agent/session.rs :: run_workflow
        ├─ system_prompt(mode, profile) + api_tools()
        ├─ compaction / context handoff if the window is close to full
        └─ loop, per round:
             ├─ provider::stream_message   (SSE → AgentEvent over a Channel)
             ├─ no tool calls? → judge_terminal_turn, then done
             └─ tool calls  → permissions → [approval gate] → tools::execute
                                └─ results appended, next round
```

Every step is persisted as a line of JSONL under `.claudinio/sessions/`. That
file is the source of truth, not memory: it is re-read on each message, so a
session survives a restart and the UI can rebuild the whole timeline from it
(`src/lib/chatRecords.ts` does that translation).

`AgentEvent`s stream to the frontend over a Tauri `Channel` while the run is in
flight. Approvals travel the other way through oneshot channels keyed by
`session_id:tool_use_id`.

### Brain and Builder

Two modes over the same loop, differing in tool surface and system prompt.
Brain is read-only and must produce a plan; Builder executes. The handoff
(`agent/transition.rs`) starts a *new* session seeded with the plan rather than
continuing the old one, so execution never inherits a context window full of
exploration. The same machinery handles the token-threshold handoff.

## Trust boundaries

Three, and they are the parts to be careful with.

**1. What the model may touch.** `agent/tools/mod.rs::validate_path` gates every
file tool; `agent/permissions.rs` gates `bash` with an allowlist/denylist and an
approval prompt. `edit_file` always shows a diff first.

**2. What the webview may touch.** The chat renders untrusted content, so treat
the frontend as a place where attacker-influenced script could run. The
filesystem commands in `commands/fs.rs` therefore enforce workspace containment
themselves — they do not rely on the frontend asking nicely. Both guards call
`workspace_path.rs` so they cannot drift apart. Writes outside the workspace go
through `export_file`, which opens the save dialog *in Rust* so the destination
is never an IPC argument.

**3. What becomes DOM.** `src/lib/markdown.ts` is the only markdown→HTML path.
It sanitizes with an allowlist before anything reaches `innerHTML`, because
`marked` deliberately passes raw HTML through and Tauri always exposes
`window.__TAURI_INTERNALS__.invoke` to whatever runs in the page. The CSP in
`tauri.conf.json` (no `script-src 'unsafe-inline'`) is the second layer.

**4. What comes back from a web page.** The browser tools put third-party
content into the model's context, and a page can contain text written to
manipulate it. `browser/page.rs::wrap_untrusted` labels every such payload as
data and neutralizes a forged closing tag; the system prompt tells the model to
treat it as evidence, never instructions. That is mitigation, not a guarantee —
the load-bearing part is that the browser tools cannot execute anything, and
that `bash` and `edit_file` still require approval.

`SECURITY.md` states what each boundary guarantees and what is explicitly out of
scope.

## Browser

`browser/` drives a dedicated Chromium over the DevTools Protocol so the agent
can see and operate a running page. It knows nothing about the agent; the
adapter is `agent/tools/browser.rs`.

- `provision.rs` — downloads a pinned Chrome for Testing build, verified by
  sha256 against a table generated by `scripts/pin_chromium.py`. Never the
  user's own Chrome profile: it holds live session cookies, and an agent with
  those plus one hostile page is credential exfiltration. Even when configured
  to use an installed browser, a fresh process is launched against our own
  `--user-data-dir`.
- `launch.rs` — spawn and teardown. `--remote-debugging-port=0` plus the
  `DevToolsActivePort` file, and a profile directory hashed per workspace, so
  two open workspaces cannot collide (sharing one profile makes the second
  Chromium hand off its URL and exit, which presents as an unexplained launch
  timeout). A PID file lets the next run reap a browser orphaned by an app
  crash, and only when the live process's executable still matches ours.
- `cdp.rs` — one WebSocket, JSON-RPC demultiplexed by `id`, events fanned out on
  a broadcast channel. The frame size cap is raised well above tungstenite's
  16 MiB default, which a full-page screenshot exceeds in a single message.
- `page.rs` — the attached tab: navigation, interaction, and the event pump that
  fills the buffers. Clearing a field uses the element's own `select()` rather
  than a synthesized Cmd/Ctrl+A, because that shortcut is handled by the
  browser's UI layer and never reaches the renderer.
- `buffers.rs` — bounded console and network history with a read cursor, filled
  independently of tool calls so nothing logged during a page load is lost.
- `screenshot.rs` — the four capture modes. `Emulation.setDeviceMetricsOverride`
  pins `deviceScaleFactor: 1` so CSS pixels equal output pixels and the geometry
  is reproducible across machines; `DOM.getBoxModel` quads are viewport-relative
  and get shifted into document coordinates, which is what `clip` expects once
  `captureBeyondViewport` is on.

The download is never triggered by a tool call — it is ~190 MB of the user's
bandwidth, so the tool fails with an actionable message and the user starts it
from Settings.

## Local models

`llama/` runs models on the user's own machine. It knows nothing about the
agent: the agent reaches it through the ordinary provider path, because
`llama-server --jinja` speaks the same OpenAI-compatible
`/v1/chat/completions` that `agent/provider/openai.rs` already speaks to remote
providers. A local model is just the id `local/<key>`.

- `provision.rs` — downloads a pinned `llama-server` from the upstream GitHub
  release, verified by sha256 against a table generated by
  `scripts/pin_llama.py` (the release API publishes sizes but no checksums).
  The archive's single top-level directory is flattened away so the binary and
  its `libggml*` siblings end up in one directory, which is what `@loader_path`
  and `$ORIGIN` need. CUDA is deliberately not offered: it needs a second
  ~390 MB runtime archive unpacked alongside, and Vulkan already covers NVIDIA.
- `supervisor.rs` — one server process per loaded model, in a global registry
  rather than app state, because `provider::stream_message` reaches inference
  with `&AgentConfig` alone. Each server binds loopback on a probed port, gets a
  random per-process api-key, and runs with `--no-webui`. That key is a trust
  boundary, not hygiene: without it any local process — including a page the
  agent's own browser visits — could POST to the port for free inference and a
  prompt-injection oracle. Readiness is `/health` on a deadline derived from the
  file size, since a 20 GB model off a cold page cache genuinely takes minutes.
- `catalog.rs` / `hf.rs` — GGUF weights and where they come from. Unlike every
  other pinned artifact in the app, the checksum is not committed: the Hugging
  Face tree API returns each LFS object's `oid`, which *is* its sha256, so
  integrity is verified against upstream live. The catalog entry is written only
  after every shard verifies, which makes it the ready marker.
- `Engine` (in `mod.rs`) — llama.cpp or MLX. MLX is Apple Silicon only and is
  the default there, because it generates measurably faster on the same weights
  (277 → 353 tok/s on an M2 Max with Qwen3-0.6B 4-bit). Every read goes through
  `LocalPrefs::effective_engine`, which falls back to llama.cpp when the
  configured engine is not available: `config.json` syncs between machines, and
  "mlx" arriving on a Windows box should degrade rather than fail every request.
  The engine also decides the model format — GGUF is one file, an MLX model is a
  whole repository — so it selects which catalog applies.
- `hardware.rs` — RAM/VRAM detection behind the admission check. A model larger
  than memory does not fail with a dialog; it swaps the machine to a standstill,
  so both the download button and the server start refuse it.

The MLX engine is a sidecar of our own, `claudinio-mlx`, built on Apple's
`mlx-swift` and released from its own repository so the Metal build (which needs
`xcodebuild` and a separately downloaded Metal toolchain) never enters this
repo's release path. It is pinned by version and sha256 like every other
artifact: being ours says nothing about the network it arrives over. It exists
rather than Apple's Python `mlx_lm.server` for three reasons — no interpreter to
provision, an api-key that `mlx_lm.server` simply does not have, and weights
loaded from a directory this app has already verified. Tool-call parsing is
*not* reimplemented; it comes from `mlx-swift-lm`'s per-family parsers, which is
the part that fails silently when rewritten.

Neither runtime nor any model is ever fetched by a tool call — one is tens of
megabytes and the other tens of gigabytes of the user's bandwidth.

## Code intelligence

`code_intel/` builds and serves the index; nothing in it knows about the agent.

- `parser.rs` — tree-sitter across 77 grammars → symbols, signatures, doc
  comments, call relations.
- `db.rs` — SQLite. Symbols and an FTS5 table (`chunk_fts`) for BM25, plus
  embedding rows.
- `embeddings.rs` — `all-MiniLM-L6-v2` through ONNX Runtime, in-process. Code is
  never sent anywhere to be indexed. The model ships as a Tauri resource.
- Search is **hybrid**: BM25 and vector results fused by reciprocal rank fusion.
  Keyword matching finds an exact identifier; the vector side finds a thing by
  description. Neither alone was good enough.
- `watcher.rs` — debounced file watching keeps the index live, sharing
  `INDEX_SEMAPHORE` so a background reindex cannot pile onto a foreground one.

## Frontend

SolidJS, no store library — signals and props.

- `App.tsx` owns workspace-level state (open workspaces, config, modals).
- `components/ChatPanel.tsx` owns one conversation: the event stream, the
  timeline, the composer.
- `components/chat/TimelineRows.tsx` holds the leaf renderers — one component
  per row type. Presentational, props in, no session state.
- `lib/chatRecords.ts` is the pure `SessionRecord[] → ChatMessage[]` translation
  plus the small formatters. Pure so it is directly testable; it used to live
  inside ChatPanel and be *copied* into the test file, which meant the tests
  passed against code that did not ship.
- `lib/ipc.ts` is the single typed wrapper over `invoke`. Nothing else calls
  `invoke` directly.

The frontend makes no network requests of its own. Every provider call happens
in Rust, which is why the CSP can keep `connect-src` closed.

## Things that look odd and are deliberate

- **English only, everywhere.** No translation layer. The system prompts are
  written and tuned in English and the agent asks users to write in English;
  a localized shell around that was worse than being direct. See the README.
- **`docs/plans/`** holds one design document per feature, written before the
  code. They are historical decision records, not living specs — see the README
  in that directory.
- **`src-tauri` is the Cargo workspace root.** There is no manifest at the
  repository root.
- **Portuguese test fixtures** in `agent/session.rs` are verbatim captures of
  sessions that broke something. Translating them would destroy the regression.
- **`ContentBlock` is `#[serde(untagged)]` and its variant order matters.**
  Deserialization takes the first structural match, and the same enum is both
  the wire format and the session JSONL format, so reordering it silently
  changes how archived sessions reload. `ToolResultContent` is untagged for the
  same reason: it lets a `tool_result` carry either a bare string (what every
  tool wrote before images existed) or an array of blocks, with no migration.
- **Images in session files live in `media/`, not in the JSONL.** A screenshot
  is ~200 KB of base64 and the session file is re-read on every message, so
  `persist.rs` swaps oversized payloads for a content-addressed reference on
  write and reads them back on load. A reference whose file is gone degrades to
  a text note rather than failing the request.
