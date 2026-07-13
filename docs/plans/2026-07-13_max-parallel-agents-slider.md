# Setting: Max Parallel Subagents — Configurable Slider

## Context

Hoje `MAX_PARALLEL_AGENTS: usize = 4` é hardcoded em `subagent.rs:11`. O usuário quer controlar esse valor via Settings (1–8), com um slider, seguindo o mesmo pipeline dos numéricos existentes (`maxRounds`, `subMaxRounds`, `maxGoldenCycles`).

**Decisões confirmadas:**
- UI: slider `<input type="range" min="1" max="8">` com rótulos "slower"/"faster"
- Workspace override via `.claudinio.json`: **Sim** (somando ao whitelist do `merge_workspace_config`)
- Descrição dinâmica da tool: **Sim** (`get_defs(max_parallel)` → `api_tools(config)` → call sites)
- Default efetivo: 4 (quando `None`)
- Clamp: `1..=8`

## Solution Design

### Pipeline completo

```
App.tsx (slider) → setConfig({maxParallelAgents: n})
  → SetConfigArgs (Option<Option<usize>>, camelCase)
    → set_config (merge com clamp 1..=8)
      → AgentConfig.max_parallel_agents: Option<usize> (None = default 4)
        → effective_max_parallel() helper
          → run_spawn_agents (validação usa valor efetivo)
        → get_defs(max_parallel) (formata descrição dinâmica)
          → api_tools() / subagent_defs()
```

### Mudanças (8 arquivos)

1. **`src-tauri/src/agent/provider.rs`** — `AgentConfig`: novo campo `max_parallel_agents: Option<usize>`, Default = None, merge_workspace_config whitelist
2. **`src-tauri/src/agent/subagent.rs`** — helper `effective_max_parallel()`, atualizar validação, atualizar teste
3. **`src-tauri/src/agent/tools/mod.rs`** — `get_defs(max_parallel: usize)` com descrição formatada
4. **`src-tauri/src/agent/session.rs`** — `api_tools()` recebe `&AgentConfig`, passa `max_parallel` para `get_defs()`
5. **`src-tauri/src/agent/subagent.rs`** — `subagent_defs()` e `api_tools()` passam `max_parallel` (dummy 4, pois spawn_agents é filtrado)
6. **`src-tauri/src/commands/agent.rs`** — `SetConfigArgs` + `maxParallelAgents`, `set_config` merge + clamp, `get_config` serialização
7. **`src/lib/ipc.ts`** — `AgentConfig.maxParallelAgents?` e `SetConfigArgs.maxParallelAgents?`
8. **`src/App.tsx`** — signal, openConfig, saveConfig, slider UI com badges workspace/local
9. **`src/lib/locales/en-US.ts` e `pt-BR.ts`** — novas chaves

## Risks

- Baixo risco: pipeline idêntico ao de `maxRounds`/`subMaxRounds`, já testado
- Tool description dinâmica: modificar assinatura de `api_tools()` afeta 4 call sites + 1 test helper — precisa compilar todos

## Verification

1. `cargo check` e `cargo test` no workspace `src-tauri`
2. Slider aparece com default 4, arrasta para 8, salva, reabre → persiste
3. Com limite = 1, spawn_agents com 2+ specs rejeita; com limite = 8, passa
4. Badge workspace/local quando `.claudinio.json` define `max_parallel_agents`


## Implementation Log — 2026-07-13 02:14
**Summary:** Configurable parallel subagents slider (1-8) via Settings UI + workspace override via .claudinio.json
**Changed files:** M src-tauri/src/agent/provider.rs, M src-tauri/src/agent/session.rs, M src-tauri/src/agent/subagent.rs, M src-tauri/src/agent/tools/mod.rs, M src-tauri/src/commands/agent.rs, M src/App.tsx, M src/lib/ipc.ts, M src/lib/locales/en-US.ts, M src/lib/locales/pt-BR.ts, ?? docs/plans/2026-07-13_fix-left-align-chat-text.md, ?? docs/plans/2026-07-13_max-parallel-agents-slider.md
**Commits:** _(git unavailable or none)_
**Journal:** ## Findings & Key Decisions

1. **Pipeline reused smoothly**: The existing `setConfig → AgentConfig` numeric pipeline (`maxRounds`, `subMaxRounds`, etc.) was a perfect template. The double-Option pattern (`Option<Option<usize>>` in SetConfigArgs) and `#[serde(default)]` worked without surprises.

2. **Tool description dynamism required plumbing**: Making `get_defs()` accept `max_parallel: usize` meant threading `&AgentConfig` through `api_tools()` in session.rs (4 call sites + 1 test). The GitSync profile path in `api_tools` also needs `get_defs(maxp)` — done. Subagents filter out spawn_agents so they pass `MAX_PARALLEL_AGENTS` (4) as a dummy.

3. **Clamp at the boundary, not at use**: Decided to clamp in `set_config` (1..=8) so the stored value is always valid, AND in `effective_max_parallel()` as a defense-in-depth measure. The `unwrap_or(4).clamp(1, 8)` chain handles stale config.json files gracefully.

4. **Slider vs number input**: The plan specified `<input type="range">` — used exactly that, with `step="1"`, the current value displayed next to the label, and "slower"/"faster" labels on each end. Workspace-config badge/disable follows the same pattern as maxRounds/subMaxRounds.

5. **Workspace override key**: In `.claudinio.json` the key is `max_parallel_agents` (snake_case, matching the Rust struct field), following the convention of the other workspace-config overrides.

## Gotchas
- The `spawn_agents` ToolDef also has `input_schema.maxItems: 4` — had to make that dynamic too, otherwise the JSON Schema would still advertise 4 even when the real limit changed.
- `subagent_defs` is called from tests with `MAX_PARALLEL_AGENTS` — updated all 4 test call sites.
- `api_tools()` in session.rs has a GitSync early-return path that also calls `get_defs()` — caught it during the edit.

**Task journal:**
- provider.rs: Add max_parallel_agents to AgentConfig: Added field to struct after install_fallback_seed; Added None to Default impl; Added to merge_workspace_config doc comment; Added merge block after sub_max_rounds
- subagent.rs: Add effective_max_parallel helper + update validation + test: Added MAX_PARALLEL_AGENTS_CAP = 8; Added effective_max_parallel() helper; Updated run_spawn_agents validation to use effective_max_parallel(config); Updated test_max_parallel_constants with CAP and 4 scenarios
- tools/mod.rs: get_defs(max_parallel) dynamic description: Changed get_defs() -> get_defs(max_parallel: usize); Formatted description and input_schema maxItems dynamically
- session.rs: Pass config to api_tools() for dynamic description: Added config: &AgentConfig param to api_tools(); Uses effective_max_parallel(config) to call get_defs(); Updated all 3 call sites in run_workflow_with_profile; Updated test call site with AgentConfig::default()
- subagent.rs: Update subagent_defs/api_tools for new get_defs signature: Changed subagent_defs signature to accept max_parallel; api_tools passes MAX_PARALLEL_AGENTS (4) as dummy; Updated all 4 test call sites
- commands/agent.rs: SetConfigArgs, set_config, get_config: Added max_parallel_agents: Option<Option<usize>> to SetConfigArgs; Added merge in set_config with clamp(1,8); Added maxParallelAgents to get_config json! macro
- ipc.ts: Add maxParallelAgents to TypeScript types: Added maxParallelAgents?: number | null to AgentConfig; Added maxParallelAgents?: number | null to SetConfigArgs
- App.tsx: Signal, openConfig, saveConfig, slider UI: Added createSignal<number>(4) for slider; Added setConfigMaxParallelAgents(cfg.maxParallelAgents ?? 4) to openConfig; Added maxParallelAgents: configMaxParallelAgents() to saveConfig; Added slider UI with range input 1-8, badges, slower/faster labels
- locales: en-US.ts + pt-BR.ts new keys: Added en-US keys: maxParallelAgents, maxParallelAgentsHint, slower, faster; Added pt-BR keys: maxParallelAgents, maxParallelAgentsHint, slower, faster
- VERIFY: cargo check, cargo test, and manual smoke test: cargo check: OK (4.19s, no errors); cargo test: OK (204/204 passed, 0 failed, 3 ignored); npx tsc --noEmit: OK (no errors)


## Implementation Log — 2026-07-13 02:19
**Summary:** Fix duplicate slider and add visible slider track styling
**Changed files:** M src-tauri/src/agent/provider.rs, M src-tauri/src/agent/session.rs, M src-tauri/src/agent/subagent.rs, M src-tauri/src/agent/tools/mod.rs, M src-tauri/src/commands/agent.rs, M src/App.css, M src/App.tsx, M src/lib/ipc.ts, M src/lib/locales/en-US.ts, M src/lib/locales/pt-BR.ts, ?? docs/plans/2026-07-13_fix-left-align-chat-text.md, ?? docs/plans/2026-07-13_max-parallel-agents-slider.md
**Commits:** _(git unavailable or none)_
**Journal:** Two bugs from the initial implementation were fixed:

1. **Duplicate slider**: The maxParallelAgents slider was inserted in two places in App.tsx — once at ~line 819 (after the model selector, before the 2-column grid) and once at ~line 944 (after the grid). The second occurrence was likely added during separate edit passes. Removed the duplicate at lines ~944-~977. Confirmed only one slider block and one `<input type="range">` remains via grep.

2. **Invisible slider track**: The range input had `appearance-none` but no custom `::-webkit-slider-runnable-track` or `::-moz-range-track` styling. In Tauri's WebView with Tailwind's appearance reset, only the thumb was visible with no guiding line. Added:
   - 4px rounded track with `--border-subtle` color
   - 14px accent-colored thumb with a `--surface-0` border ring (2px) and subtle shadow
   - `::-webkit-slider-thumb` properly centered via `margin-top: -5px`
   - Disabled state styling that dims the track and grays out the thumb
   All scoped to `input[type="range"]` so it doesn't affect other inputs.

**Task journal:**
- Remove the duplicate maxParallelAgents slider block: Removed the second occurrence (was at line ~944 after the 2-column grid). The first occurrence at line ~819 (after model selector, before the grid) is the one that stays.
- Add CSS track styling to the range slider for visibility: Added full track styling: 4px rounded track with border-subtle color, 14px accent-colored thumb with surface-0 border ring, and disabled state styling.
- Verify both fixes with tsc: npx tsc --noEmit: OK (no errors); grep confirms single slider block and single range input in App.tsx


## Implementation Log — 2026-07-13 03:00
**Summary:** Successfully deployed v0.1.6: version bumped, validated (625 tests, clean build), tagged, pushed, and released via GitHub Actions with all 3 platform artifacts
**Changed files:** A	.agents/skills/deploy/SKILL.md, M	package.json, M	src-tauri/Cargo.lock, M	src-tauri/Cargo.toml, M	src-tauri/tauri.conf.json
**Commits:** c8cbf6e feat: add deploy skill for claudinio-code release pipeline, 5a0ac93 chore: bump version to 0.1.6
**Journal:** ## Deploy v0.1.6 - Results

**Build outcome:** All 3 platforms passed on first attempt. macOS ARM completed first (~15min), Linux x64 second (~25min), Windows last (~35min). No failures.

**Key decisions:**
- Created deploy skill at `.agents/skills/deploy/SKILL.md` documenting the full release pipeline: which 3 files to bump (`package.json`, `Cargo.toml`, `tauri.conf.json`), the gotcha about `Cargo.lock` needing regeneration, pre-deploy validation (625 tests + build), and how to force-redeploy by deleting/recreating the tag.
- The APP_VERSION UI constant is injected via Vite's `define` from `package.json`, so no frontend code changes needed for version bumps.

**Gotchas discovered:**
1. Three files need version bumps, not just one: package.json, Cargo.toml, tauri.conf.json
2. Cargo.lock auto-updates on cargo check/build, but must be committed — otherwise CI's `--frozen-lockfile` fails
3. Tag must be exact `vX.Y.Z` format — pre-release tags like `v0.1.6-rc1` do NOT trigger the workflow
4. Cannot re-use a tag for a new release — must delete and recreate or bump to next version
5. The deploy skill should live under `.agents/skills/` not `.claude/skills/` in the project

**Task journal:**
- Investigate project state, version, and CI/CD pipeline: Current version is 0.1.5 in package.json and src-tauri/Cargo.toml; Release workflow at .github/workflows/release.yml triggers on tag push v[0-9]+.[0-9]+.[0-9]+; Branch: main, clean working tree, up-to-date with origin/main; GitHub remote: git@github.com:claudin-io/claudinio-code.git; 14 commits since v0.1.5 tag
- Update version and push tag v0.1.6: Versions bumped: package.json, Cargo.toml, tauri.conf.json (and Cargo.lock auto-synced); Pre-deploy validation passed: 625 tests, build in 13.66s; Reviewed APP_VERSION injection via Vite define from package.json; Commit 5a0ac93: chore: bump version to 0.1.6; Tag v0.1.6 pushed, triggered GitHub Actions release.yml; Linux build: success (incl. all tree-sitter grammars); macOS ARM build: success; Windows x64 build: success; Create Release job: success; 7 assets published: Windows (.msi + .exe), macOS (.dmg), Linux (.AppImage + .deb + .tar.gz x2); Deploy skill created at .agents/skills/deploy/SKILL.md


## Implementation Log — 2026-07-13 21:08
**Summary:** Fix 'unable to open database file' error when opening a new workspace — ensure .claudinio/ directory exists before opening SQLite index database
**Changed files:** M docs/plans/2026-07-13_max-parallel-agents-slider.md, M src-tauri/src/code_intel/db.rs
**Commits:** _(git unavailable or none)_
**Journal:** Root cause: IndexDb::open() in code_intel/db.rs called Connection::open(db_path) without ensuring the parent directory (.claudinio/) existed first. When opening a new workspace that never had a .claudinio/ directory, SQLite returned "unable to open database file" because it couldn't create the file inside a non-existent directory. The sessions_dir() function in persist.rs already did create_dir_all() — this was an inconsistent gap.

Fix: Added a `create_dir_all(parent)` call in IndexDb::open() before the SQLite Connection::open(). This is idempotent (create_dir_all is a no-op if the dir exists) and follows the same pattern already used in sessions_dir().

**Task journal:**
- Fix 'unable to open database file' error — ensure .claudinio/ dir exists: IndexDb::open() em src-tauri/src/code_intel/db.rs não garantia que o diretório pai (.claudinio/) existisse antes de abrir o SQLite; SQLite retorna 'unable to open database file' quando o diretório pai não existe — erro reproduzia ao abrir um workspace novo pela primeira vez; Adicionado create_dir_all() para o parent dir antes do Connection::open(); 29/29 testes passam, build completo sem erros
