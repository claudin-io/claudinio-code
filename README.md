<p align="center">
  <img src="docs/assets/logo.png" alt="Claudinio Code" width="128">
</p>

<h1 align="center">Claudinio Code</h1>

<p align="center">
  <strong>A native desktop harness for AI coding agents.</strong><br>
  Real code intelligence, a visible reasoning timeline, and approval gates on everything that touches your machine.
</p>

<p align="center">
  <a href="LICENSE"><img alt="License: MIT" src="https://img.shields.io/badge/license-MIT-blue.svg"></a>
  <a href="https://github.com/claudin-io/claudinio-code/actions/workflows/ci.yml"><img alt="CI" src="https://github.com/claudin-io/claudinio-code/actions/workflows/ci.yml/badge.svg"></a>
  <a href="https://github.com/claudin-io/claudinio-code-releases/releases/latest"><img alt="Release" src="https://img.shields.io/github/v/release/claudin-io/claudinio-code-releases?label=download"></a>
  <img alt="Platforms" src="https://img.shields.io/badge/platform-macOS%20%7C%20Windows%20%7C%20Linux-lightgrey">
</p>

<p align="center">
  <a href="README.md">English</a> ·
  <a href="README.pt-BR.md">Português</a>
</p>

---

Claudinio Code is a desktop application — not a terminal wrapper, not a browser
tab, not an extension bolted onto someone else's editor. It runs an agent loop
in Rust, indexes your workspace with tree-sitter and local ONNX embeddings, and
renders every thought, tool call and subagent in a timeline you can inspect.

It talks to any Anthropic-compatible Messages API. [claudin.io](https://claudin.io)
is the default; Anthropic, OpenRouter, DeepSeek and anything else in the
[models.dev](https://models.dev) catalog work too.

## Install

Download the installer for your platform from
[**Releases**](https://github.com/claudin-io/claudinio-code-releases/releases/latest):

| Platform | Artifact |
|---|---|
| macOS (Apple Silicon) | `Claudinio-Code-macOS-arm64-*.dmg` |
| Windows (x64) | `Claudinio-Code-Windows-x64-*.exe` / `*.msi` |
| Windows (ARM64) | `Claudinio-Code-Windows-arm64-*.exe` |
| Linux (x64) | `Claudinio-Code-Linux-x64-*.AppImage` / `*.deb` |
| Linux (ARM64) | `Claudinio-Code-Linux-arm64-*.AppImage` / `*.deb` |

The app updates itself: release artifacts are signed and the built-in updater
verifies the signature before installing.

Building from source is covered in [CONTRIBUTING.md](CONTRIBUTING.md).

## Quickstart

1. **Open a folder.** Indexing starts immediately — keyword search is available
   right away, semantic ranking joins as soon as embeddings finish.
2. **Connect a provider.** Settings → API key and base URL (defaults to
   `https://api.claudin.io`), or sign in to OpenRouter via OAuth.
3. **Describe the task.** The agent plans, calls tools and streams its reasoning
   into the timeline.
4. **Approve what matters.** Shell commands and file edits stop for your
   approval; edits show a Monaco diff before anything is written.

## What makes it different

### Brain and Builder

Two modes with different tool surfaces. **Brain** explores read-only, interviews
you about requirements and writes a plan. **Builder** executes it — writing code,
running commands, verifying. The handoff between them starts a fresh session
seeded with the plan, so execution never inherits a context window full of
exploration.

### It actually reads your codebase

Not just grep:

- **Tree-sitter indexing** across **77 languages** — symbols, signatures, doc
  comments and call relations, in SQLite.
- **Hybrid search** — BM25 keyword matching over code, docs and paths, fused
  with MiniLM semantic embeddings via reciprocal rank fusion. Finds a message
  queue by describing what it does, and finds `TOKENIZERS_PARALLELISM` by its
  exact spelling.
- **Embeddings run locally.** `all-MiniLM-L6-v2` ships inside the installer as
  an ONNX resource. Your code is never sent anywhere to be indexed.
- **LSP** — go-to-definition, find-references and hover through
  `typescript-language-server` and `rust-analyzer`.
- **Live reindexing** — a file watcher keeps the index in sync as you work.

### Parallel subagents

The main agent delegates to parallel subagents (4 by default, configurable),
each with a fresh context and its own timeline you can open and read. Modes are
`explore` (read-only) or `code`.

### Nothing is a black box

Every assistant turn renders as a collapsible timeline — phases, thinking, tool
calls, results, subagents, token counts and cost. Sessions persist as JSONL in
`.claudinio/sessions/`, so you can reopen and continue any conversation.

### Steering, not just interrupting

Type while the agent is working and your message is queued and injected as
guidance mid-reasoning. `Esc` interrupts outright.

### Extensible

- **Skills** — drop a `SKILL.md` into `.agents/skills/`, `.claudinio/skills/` or
  `.claude/skills/` and the agent discovers it.
- **MCP** — connect Model Context Protocol servers over stdio or HTTP.

The interface and the agent are English-only. The system prompts are written and
tuned in English, and the agent asks you to write in English too — a localized
shell around an English-speaking agent was worse than being straightforward
about it.

## The tools the model can call

| Tool | Approval | What it does |
|---|---|---|
| `read_file` | auto | Read a text file inside the workspace |
| `list_dir` | auto | List a directory, honouring `.gitignore` |
| `grep` | auto | Regex search (ripgrep) |
| `code_search` | auto | Full-text symbol search (SQLite FTS5) |
| `semantic_search` | auto | Hybrid BM25 + embedding search |
| `symbol_lookup` | auto | Exact symbol lookup |
| `file_outline` | auto | Symbols defined in a file |
| `go_to_definition` | auto | LSP definition |
| `find_references` | auto | LSP references |
| `web_search` | auto | Search the web |
| `ask_user` | auto | Ask you a question, with options |
| `tasks_get` / `tasks_set` | auto | Read and update the task list |
| `write_plan` / `finalize_plan` | auto | Author and close out a plan document |
| `enter_plan_mode` / `exit_plan_mode` | auto | Switch between Brain and Builder |
| `spawn_agents` | auto | Launch parallel subagents |
| **`edit_file`** | **requires approval** | Propose an edit, shown as a diff |
| **`bash`** | **requires approval** | Run a shell command (read-only commands are allowlisted) |

File tools are confined to the opened workspace by a path guard that rejects
traversal. A denylist blocks known-destructive commands outright. See
[SECURITY.md](SECURITY.md) for the full threat model, including what the
project explicitly does *not* defend against.

## Architecture

Full write-up in [docs/ARCHITECTURE.md](docs/ARCHITECTURE.md) — layering rules,
trust boundaries, and the shapes that are deliberate.

```
┌──────────────────────────────────────────────────────────┐
│  Frontend — SolidJS + TypeScript + Tailwind              │
│  chat timeline · approvals · Monaco diffs · file tree    │
└───────────────────────────┬──────────────────────────────┘
                            │ Tauri IPC
┌───────────────────────────┴──────────────────────────────┐
│  Backend — Rust / Tauri v2                               │
│                                                          │
│  agent/        loop, providers (SSE), tools, subagents,  │
│                permissions, skills, MCP, persistence     │
│  code_intel/   tree-sitter → SQLite FTS5 + ONNX vectors  │
│  lsp/          typescript-language-server, rust-analyzer │
│  commands/     IPC surface exposed to the frontend       │
└──────────────────────────────────────────────────────────┘
```

A message flows: IPC `send_message` → session loop → provider SSE stream →
`AgentEvent`s over a channel → timeline renders live → approval gates resolve
over oneshot channels → tool results feed the next round → `Done`, persisted to
JSONL.

## Built with

[Tauri v2](https://v2.tauri.app) ·
[Rust](https://www.rust-lang.org) ·
[SolidJS](https://www.solidjs.com) ·
[TypeScript](https://www.typescriptlang.org) ·
[Tailwind CSS v4](https://tailwindcss.com) ·
[Monaco Editor](https://microsoft.github.io/monaco-editor/) ·
[tree-sitter](https://tree-sitter.github.io/tree-sitter/) ·
[SQLite / rusqlite](https://github.com/rusqlite/rusqlite) ·
[ONNX Runtime](https://github.com/pykeio/ort) ·
[tokio](https://tokio.rs)

## Contributing

Issues and pull requests are welcome — start with
[CONTRIBUTING.md](CONTRIBUTING.md) for the dev setup and the checks CI runs.
Security issues go through [SECURITY.md](SECURITY.md), never a public issue.
Participation is governed by the [Code of Conduct](CODE_OF_CONDUCT.md).

## License

[MIT](LICENSE) © Victor Carvalho Tavernari
