# Contributing to Claudinio Code

Thanks for taking the time. This document covers what you need to get a build
running and what a mergeable change looks like.

## Before you write code

- **Bugs** — open an issue with the reproduction. If you already have the fix,
  open the PR and link the issue.
- **Features** — open an issue first and describe the problem before the
  solution. The agent loop, permission model and indexing pipeline have opinions
  baked into them; a short discussion saves you from rewriting a large PR.
- **Small stuff** — typos, broken links, obvious one-line fixes: just send the PR.

## Development setup

### Prerequisites

| Tool | Version |
|---|---|
| [Node.js](https://nodejs.org) | 18+ |
| [pnpm](https://pnpm.io) | 9+ |
| [Rust](https://rustup.rs) | stable, 2024 edition |
| [Python](https://python.org) | 3.9+ (only for the embedding-model fetch script) |

**Linux** also needs the Tauri system dependencies:

```bash
sudo apt-get install -y libwebkit2gtk-4.1-dev libayatana-appindicator3-dev \
  librsvg2-dev patchelf libxdo-dev libssl-dev xdg-utils
```

### Running

```bash
git clone https://github.com/claudin-io/claudinio-code.git
cd claudinio-code
pnpm install
pnpm tauri dev
```

The first `cargo` build compiles 77 tree-sitter grammars and a statically linked
ONNX Runtime. Expect it to take a while; incremental builds are fast.

### Building a release bundle

```bash
# once — downloads all-MiniLM-L6-v2 (sha256-pinned) into src-tauri/models/
python3 scripts/fetch_embedding_model.py
pnpm tauri build
```

The model is bundled as a Tauri resource so semantic search works fully offline.

## Checks your PR must pass

Run these locally before pushing — CI runs the same set:

```bash
pnpm test                          # vitest (frontend)
pnpm exec tsc --noEmit             # typecheck
cargo fmt --all --check            # from src-tauri/
cargo clippy --all-targets -- -D warnings
cargo test
```

The tree is warning-clean. A PR that adds warnings will fail CI.

## Codebase map

[docs/ARCHITECTURE.md](docs/ARCHITECTURE.md) is the longer version, including the
layering rule that CI enforces.

```
src/                        SolidJS frontend
  components/               UI (chat timeline, settings, diff viewer, file tree)
    chat/TimelineRows.tsx   leaf renderers, one per timeline row type
    settings/               the settings panel's sub-panels
  lib/chatRecords.ts        SessionRecord[] -> timeline model (pure, tested)
  lib/markdown.ts           the only markdown -> HTML path; sanitizes
  lib/ipc.ts                the only place that calls Tauri `invoke`
src-tauri/src/
  agent/                    agent loop, providers, tools, subagents, skills, MCP
    tools/                  the tool implementations the model calls
    permissions.rs          allowlist / denylist / approval policy
    persist.rs              session JSONL store
  code_intel/               tree-sitter indexing, SQLite FTS5, ONNX embeddings
  lsp/                      language server client
  commands/                 Tauri IPC surface — thin adapters only
  workspace_path.rs         workspace containment, shared by tools and IPC
docs/ARCHITECTURE.md        layering, message lifecycle, trust boundaries
docs/plans/                 one design doc per feature (see below)
```

### Plan documents

Non-trivial features get a plan in `docs/plans/YYYY-MM-DD_slug.md` before the
code: the problem, the decisions and their rationale, the files touched. This is
how the project keeps its reasoning reviewable — read a few before writing your
first one. Bug fixes and small changes do not need one.

## Conventions

- **Commits** follow [Conventional Commits](https://www.conventionalcommits.org):
  `feat:`, `fix:`, `chore:`, `docs:`, `refactor:`, `test:`, with an optional
  scope — `feat(search): hybrid BM25+semantic retrieval`.
- **Tests** live next to the code: `#[cfg(test)]` modules in Rust, `*.test.ts`
  in the frontend. New behaviour needs a test; bug fixes need a regression test.
- **Rust** is formatted by `rustfmt` and linted by `clippy` at
  `-D warnings`. Prefer returning `Result<_, String>` in tool and command code
  to match the surrounding style.
- **Comments** explain *why*, not *what*. The codebase is deliberately light on
  narration and heavy on rationale where a decision is non-obvious.
- **English only**, in the UI and in the code. User-facing strings are plain
  English literals — there is no translation layer, by design (see the README).
  Comments and identifiers are English too.
- **Markdown rendering** goes through `renderMarkdown` in `src/lib/markdown.ts`,
  never `marked.parse` directly. That function owns the sanitize pass that keeps
  injected HTML in model output from executing — see SECURITY.md.

## Touching sensitive areas

Some changes get extra scrutiny — flag them explicitly in your PR description:

- `agent/permissions.rs` — anything that widens the bash allowlist or narrows
  the denylist.
- `workspace_path.rs` — the containment rule behind both `validate_path` (agent
  tools) and the `commands/fs.rs` guards.
- `commands/fs.rs` — the filesystem surface the webview can reach.
- `src/lib/markdown.ts`, `app.security.csp` in `tauri.conf.json` — the two
  layers that stop injected HTML in model output from executing.
- `agent/app_sign.rs`, `commands/auth.rs` — request signing and credentials.
- `.github/workflows/release.yml` — the signing and publishing pipeline.

If you found a security bug, do not open a PR that quietly fixes it. See
[SECURITY.md](SECURITY.md).

## Pull requests

- Branch from `main`, keep the PR focused on one thing.
- Describe what changed and why; screenshots or a short clip for UI changes.
- Link the issue it closes.
- Expect review comments on rationale, not just style.

By contributing you agree that your contributions are licensed under the
[MIT License](LICENSE).
