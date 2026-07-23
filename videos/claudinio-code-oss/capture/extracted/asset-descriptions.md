# Asset inventory

No website was crawled. This is a **no-capture** project: the source is the
repository itself (`claudin-io/claudinio-code`), and the app's interface is
rebuilt in HTML from its own stylesheet rather than screenshotted. See
`BRIEF.md` § Customizations.

## Real files on disk (`capture/assets/`)

| File | What it is | Where it belongs |
|---|---|---|
| `logo.png` | The Claudinio Code mark — rounded-square app icon, indigo `#5c60e6` field, 512px, transparent background | Frame 01 (open) and frame 08 (CTA close) |
| `InterVariable.woff2` | The app's UI typeface, variable weight 100–900 | Every frame that renders app UI |
| `JetBrainsMono-Regular.woff2` | The app's code/mono typeface | Code surfaces, tool names, file paths, diff |
| `JetBrainsMono-Medium.woff2` | Mono, medium weight | Emphasis inside code surfaces |

## Design system source (read, not copied)

| Source | What it provides |
|---|---|
| `src/App.css` `:root` (lines 4–48) | The oklch token scale — surfaces, ink, accent, success/warning/danger, radii, spacing, shadows |
| `src/App.css` `@theme inline` (line 486) | Tailwind v4 theme mapping (`--color-surface-0`, `--color-ink-faint`, `--font-mono`, …). Ported verbatim into each composition's `<style type="text/tailwindcss">`, which makes the app's real utility classes resolve |
| `src/components/*.tsx` | The real `class` strings reused verbatim in the UI mocks |

Hex equivalents of the oklch tokens are in `tokens.json`; `accent-strong`
converts to exactly `#5c60e6`, the value `App.css` documents as the logo blue.

## UI surfaces to build (the video's featured "assets")

Each is an HTML reconstruction using the app's own theme, not an invention.
Source component named for fidelity.

| id | Surface | Source component | Featured in |
|---|---|---|---|
| `app-shell` | Three-panel window chrome: FileTree │ Viewer │ Chat, header with workspace name, model select, status dots | `App.tsx`, `FileTree.tsx` | Reusable sub-composition, all UI frames |
| `mode-toggle` | Segmented Brain / Builder control (codicon:thinking + carbon:tool-box icons) | `ChatPanel.tsx`, `Icon.tsx` | Frame 03 |
| `plan-block` | A written plan rendered in the timeline, then the handoff to a fresh session | `ChatPanel.tsx` phase blocks | Frame 03 |
| `index-progress` | Indexing progress: file counts, symbol counts, embeddings joining | `IndexProgress` events | Frame 04 |
| `search-result` | `semantic_search` returning ranked hits, and `grep` returning an exact-spelling hit | tool-result rendering | Frame 04 |
| `timeline` | Collapsible assistant turn: phase divider, thinking, tool calls, results, token count + cost footer | `ChatPanel.tsx` | Frame 05 |
| `subagents` | Four parallel subagent cards, each with its own live timeline | `TasksPanel` / subagent blocks | Frame 05 |
| `approval-bash` | The approval card for a `bash` call — command, allow/deny, keyboard hints | `ApprovalCard` | Frame 06 |
| `approval-diff` | Monaco side-by-side diff for a proposed `edit_file`, with the approve gate | `DiffViewer.tsx` (JetBrains Mono 13px, `renderSideBySide`) | Frame 06 |
| `skills-mcp` | A `SKILL.md` discovered from `.agents/skills/`, and connected MCP servers | skills manager, MCP config | Frame 07 |
| `provider-list` | Model select showing claudin.io (default) plus OpenRouter / Anthropic / DeepSeek | `ModelSelect.tsx` | Frame 07 |

## Facts available for on-screen copy

All from `README.md` — do not round or embellish:

- MIT licensed · `github.com/claudin-io/claudinio-code`
- macOS (Apple Silicon), Windows (x64 + ARM64), Linux (x64 + ARM64)
- tree-sitter indexing across **77 languages**
- Hybrid search: BM25 over code/docs/paths fused with MiniLM embeddings via reciprocal rank fusion
- `all-MiniLM-L6-v2` ships in the installer as an ONNX resource; code is never sent anywhere to be indexed
- LSP: `typescript-language-server`, `rust-analyzer`
- Parallel subagents: **4 by default**, configurable; modes `explore` (read-only) or `code`
- `edit_file` and `bash` require approval; every other tool is auto
- Sessions persist as JSONL in `.claudinio/sessions/`
- Built with Tauri v2, Rust, SolidJS, TypeScript, Tailwind v4, Monaco, tree-sitter, SQLite, ONNX Runtime, tokio
