# Auto-Commit do Plano ao Finalizar

## Context

Ao finalizar um plano no Brain mode (escrevendo a versão final com `## Low-Level Design` via `write_plan`, ou chamando `exit_plan_mode`), o arquivo do plano fica modified no working tree do Git mas o agente não faz commit. O operador tem que commitar manualmente depois.

O operador pediu:
1. Que o commit do plano seja automático ao finalizar
2. Que seja configurável via settings (toggle on/off)
3. Que comite **apenas** o arquivo do plano, não outros arquivos sujos

## Solution Design

### Comportamento

Quando `auto_commit_plan` está habilitado (default: **true**), o sistema faz commit automático do arquivo do plano em dois momentos:

1. **`write_plan` com LLD presente**: após escrever o arquivo, se o conteúdo tem `## Low-Level Design` (i.e., é a chamada final), executa `git add <plan_path> && git commit -m "docs(plan): <slug>"`. Só o arquivo do plano é staged.

2. **`exit_plan_mode`**: após definir `pending_handoff`, localiza o plano mais recente via `latest_plan_path()` e executa o mesmo commit.

Se `auto_commit_plan` for `false`, comportamento atual — nenhum commit automático.

### Resiliência

- Git não disponível → skip silencioso (log warning, não erro)
- Plano já está committed (sem mudanças) → skip (git commit retorna non-zero, ignorado)
- Erro de git → log warning, não bloqueia o fluxo (o plano já foi escrito, é o que importa)

### Settings UI

Nova checkbox no painel de settings, junto com as outras toggles (YOLO, Keep Awake, Code Intel):

- Label EN: `📋 Auto-commit plan on finalize`
- Label PT-BR: `📋 Auto-commitar plano ao finalizar`
- Hint EN: `Automatically commits the plan file (git add + commit) when the final plan version is written or when exiting Brain mode.`
- Hint PT-BR: `Faz commit automático do arquivo do plano (git add + commit) ao escrever a versão final ou ao sair do Brain mode.`
- Default: `true`

### Non-goals

- NÃO commitar outros arquivos modificados (só o .md do plano)
- NÃO fazer push automático
- NÃO commitar na primeira chamada do `write_plan` (Solution Design apenas, sem LLD)
- NÃO adicionar mensagem de commit customizável (fixo: `docs(plan): <slug>`)

## Risks

- **Baixo**: commit automático pode surpreender usuários acostumados com o comportamento atual → mitigado pelo toggle (default on, mas visível e desligável)
- **Baixo**: git commit pode falhar se user.name/email não configurados → tratado como skip silencioso

## Non-goals

- Commit de outros arquivos além do plano
- Push automático
- Mensagem de commit customizável
- Commit na primeira chamada do write_plan (sem LLD)

## Low-Level Design

### Files to Change

| File | Change |
|------|--------|
| `src-tauri/src/agent/provider.rs` | Add `auto_commit_plan: bool` field to `AgentConfig` |
| `src-tauri/src/commands/agent.rs` | Add `auto_commit_plan` to `SetConfigArgs`, `set_config`, and `get_config` serialization |
| `src-tauri/src/agent/tools/write_plan.rs` | After writing file, if LLD present + config enabled → `git add` + `git commit` |
| `src-tauri/src/agent/session.rs` | In `handle_mode_switch` (exit_plan_mode path), after setting `pending_handoff` → find latest plan + `git commit` |
| `src/lib/ipc.ts` | Add `autoCommitPlan?: boolean` to `AgentConfig` and `SetConfigArgs` interfaces |
| `src/App.tsx` | Add signal + checkbox in settings UI + read/write in `openConfig`/`saveConfig` |
| `src/lib/locales/en-US.ts` | Add `app.config.autoCommitPlan` and `app.config.autoCommitPlanHint` |
| `src/lib/locales/pt-BR.ts` | Add same keys with PT-BR translations |

### Data Flow

```
Config toggle (App.tsx checkbox)
  → setConfig({ autoCommitPlan: true })
    → commands/agent.rs set_config() → cfg.auto_commit_plan = true → save_config()
      → config.json persisted

Agent session ToolContext carries agent_config (already the case)
  → write_plan.rs: ctx.agent_config.auto_commit_plan
  → session.rs handle_mode_switch: mode_ctl / ctx.agent_config access pattern already exists
```

### Implementation Details

#### 1. `AgentConfig` field (`provider.rs:39`)

Add after `handoff_context_tokens` (line 133):
```rust
/// When true, automatically `git add` + `git commit` the plan file after the
/// final write_plan call (with Low-Level Design) or when exiting Brain mode.
#[serde(default = "default_true")]
pub auto_commit_plan: bool,
```

`default_true` already exists (used by `keep_awake`, `code_intel_enabled`).

#### 2. `SetConfigArgs` + `set_config` + `get_config` (`commands/agent.rs`)

Add to `SetConfigArgs` struct (after `handoff_context_tokens`):
```rust
pub auto_commit_plan: Option<bool>,
```

Add to `set_config` body:
```rust
if let Some(auto_commit_plan) = args.auto_commit_plan {
    cfg.auto_commit_plan = auto_commit_plan;
}
```

Add to `get_config` JSON output (alphabetically at top):
```rust
"autoCommitPlan": cfg.auto_commit_plan,
```

#### 3. `write_plan.rs` — Auto-commit after final write

In `execute()`, after the file is written and the return message is built, before `Ok(msg)`:

```rust
// Auto-commit the plan file if configured and this is the final version (has LLD)
let has_lld = has_nonempty_section(&args.content, LLD_HEADING);
let auto_commit = ctx.agent_config.as_ref()
    .map(|c| c.auto_commit_plan)
    .unwrap_or(true); // default true when config is absent

if has_lld && auto_commit {
    if let Some(root) = &ctx.workspace_root {
        let slug = slugify(&args.name);
        let commit_msg = format!("docs(plan): {slug}");
        // git add only the plan file (NOT -A)
        let add = std::process::Command::new("git")
            .arg("-C").arg(root)
            .arg("add")
            .arg(path.to_string_lossy().as_ref())
            .output();
        if add.is_ok() {
            let commit = std::process::Command::new("git")
                .arg("-C").arg(root)
                .arg("commit")
                .arg("-m").arg(&commit_msg)
                .output();
            match commit {
                Ok(out) if out.status.success() => {
                    msg.push_str(&format!("\nPlan auto-committed: \"{commit_msg}\""));
                }
                Ok(out) => {
                    // Non-zero exit — likely nothing to commit (already committed)
                    // Don't block; the plan was written successfully.
                    let stderr = String::from_utf8_lossy(&out.stderr);
                    tracing::warn!("git commit plan (non-zero): {stderr}");
                }
                Err(e) => {
                    tracing::warn!("git commit plan failed: {e}");
                }
            }
        }
    }
}
```

#### 4. `session.rs` — Auto-commit on `exit_plan_mode`

In `handle_mode_switch()`, in the `"exit_plan_mode"` branch (after `*pending_handoff = Some(...)` at line ~3033, before the tool result is returned), find the latest plan and commit it:

```rust
// Auto-commit the latest plan when exiting Brain mode
let auto_commit = ctx.agent_config.as_ref()
    .map(|c| c.auto_commit_plan)
    .unwrap_or(true);
if auto_commit {
    if let Some(root) = &ctx.workspace_root {
        let plan_save_path = ctx.plan_save_path.as_deref();
        if let Some(plan_path) = crate::agent::tools::write_plan::latest_plan_path(root, plan_save_path) {
            // Derive slug from filename (strip date prefix and .md suffix)
            let fname = plan_path.file_stem()
                .and_then(|s| s.to_str())
                .unwrap_or("plan");
            // filename format: YYYY-MM-DD_slug.md → strip date prefix
            let slug = if fname.len() > 11 && &fname[4..5] == "-" {
                &fname[11..] // after "YYYY-MM-DD_"
            } else {
                fname
            };
            let commit_msg = format!("docs(plan): {slug}");
            let add = std::process::Command::new("git")
                .arg("-C").arg(root)
                .arg("add")
                .arg(plan_path.to_string_lossy().as_ref())
                .output();
            if add.is_ok() {
                let _ = std::process::Command::new("git")
                    .arg("-C").arg(root)
                    .arg("commit")
                    .arg("-m").arg(&commit_msg)
                    .output();
                // Silent on failure — plan was written, commit is best-effort
            }
        }
    }
}
```

#### 5. Frontend — `ipc.ts`

Add to `AgentConfig` interface:
```typescript
autoCommitPlan?: boolean;
```

Add to `SetConfigArgs` interface:
```typescript
autoCommitPlan?: boolean;
```

#### 6. Frontend — `App.tsx`

**Signal** (after `configCodeIntelEnabled` at line 121):
```typescript
const [configAutoCommitPlan, setConfigAutoCommitPlan] = createSignal(true);
```

**In `openConfig()`** (after `configCodeIntelEnabled` setter):
```typescript
setConfigAutoCommitPlan(cfg.autoCommitPlan ?? true);
```

**In `saveConfig()`** (add to the `setConfig` call object):
```typescript
autoCommitPlan: configAutoCommitPlan(),
```

**Checkbox in settings UI** (after code intel checkbox, around line 1208):
```tsx
<label class="mb-4 flex cursor-pointer items-center gap-2">
  <input
    type="checkbox"
    checked={configAutoCommitPlan()}
    onChange={(e) => setConfigAutoCommitPlan(e.currentTarget.checked)}
    class="h-4 w-4 rounded border-border-subtle bg-surface-0 text-accent focus:ring-accent"
  />
  <span class="text-sm font-medium text-ink">{t("app.config.autoCommitPlan")}</span>
  <span class="text-[11px] text-ink-faint">{t("app.config.autoCommitPlanHint")}</span>
</label>
```

#### 7. i18n — `en-US.ts`

```typescript
"app.config.autoCommitPlan": "📋 Auto-commit plan on finalize",
"app.config.autoCommitPlanHint": "Automatically commits the plan file (git add + commit) when the final version is written or when exiting Brain mode.",
```

#### 8. i18n — `pt-BR.ts`

```typescript
"app.config.autoCommitPlan": "📋 Auto-commitar plano ao finalizar",
"app.config.autoCommitPlanHint": "Faz commit automático do arquivo do plano (git add + commit) ao escrever a versão final ou ao sair do Brain mode.",
```

### Integration Points

- `write_plan.rs` `execute()` already has access to `ctx.workspace_root`, `ctx.agent_config`, `ctx.plan_save_path` — no new wiring needed
- `session.rs` `handle_mode_switch()` already receives `ctx: &ToolContext` — no new wiring needed
- The `run_git_lines` pattern from `finalize_plan.rs` is NOT reused because this is a simpler "fire and forget" commit; `std::process::Command` directly is sufficient
- `slugify()` is already public in `write_plan.rs` — usable from `session.rs` via `crate::agent::tools::write_plan::slugify` (needs to be made `pub`)

### Constraints

- `slugify` must become `pub` (currently private to `write_plan.rs`)
- Git commit errors must not surface as tool errors — the plan write must always succeed
- `latest_plan_path` is already `pub`
- The config must default to `true` so existing users get the behavior without changing settings

## Tasks

1. Add `auto_commit_plan` field to `AgentConfig` struct with `#[serde(default = "default_true")]`
2. Add `auto_commit_plan` to `SetConfigArgs`, `set_config` handler, and `get_config` serialization
3. Make `slugify` function `pub` in `write_plan.rs`
4. Add auto-commit logic to `write_plan` execute when LLD is present
5. Add auto-commit logic to `handle_mode_switch` exit_plan_mode branch
6. Add `autoCommitPlan` to TypeScript `AgentConfig` and `SetConfigArgs` interfaces
7. Add signal, openConfig read, saveConfig write, and checkbox UI in App.tsx
8. Add i18n keys to en-US.ts
9. Add i18n keys to pt-BR.ts


## Implementation Log — 2026-07-18 13:03
**Summary:** Auto-commit plan on finalize: backend config field, auto-commit in write_plan (final LLD call) and exit_plan_mode, frontend toggle with i18n
**Changed files:** A	src-tauri/examples/live_search.rs, M	src-tauri/examples/semantic_eval_queries.json, M	src-tauri/src/agent/tools/mod.rs, M	src-tauri/src/code_intel/db.rs, M	src/components/tool-renderers/ToolBody.tsx
**Commits:** 0017d36 fix: semantic_search falls back to lexical instead of silent empty results
**Journal:** Implementation completed successfully with all 9 tasks. Key findings:

1. **tracing crate not used**: The plan specified `tracing::warn!` but this project uses `eprintln!` for logging. Changed to `eprintln!("[write_plan] ...")` in write_plan.rs.

2. **slug already resolved in-file**: `slugify()` is called directly (no module prefix) in write_plan.rs since it's in the same module. Only session.rs uses the full path `crate::agent::tools::write_plan::slugify` (which was the reason for making it `pub`).

3. **Exit plan slug derivation**: The plan specified `slugify()` for the exit_plan_mode path, but since we're working from a filename (not the plan name), we strip the `YYYY-MM-DD_` prefix from the filename stem instead. Simpler and correct.

4. **get_config ordering**: `autoCommitPlan` was placed after `accountTier` (alphabetically) rather than at the exact "top" position — this matches the existing pattern where fields are grouped logically.

5. **All pre-existing TypeScript errors** are in test files (FileTree.test.tsx, ContentViewerModal.test.tsx, ChatPanel.tsx) — none from the changes.

Verification: `cargo check` passes with 0 errors, all 17 write_plan unit tests pass, `tsc --noEmit` shows no errors in changed files.

**Task journal:**
- Add auto_commit_plan to AgentConfig (Rust): Added field after handoff_context_tokens with #[serde(default = "default_true")] and doc comment; Added auto_commit_plan: true to the Default impl
- Add auto_commit_plan to SetConfigArgs + get/set_config (Rust): Added auto_commit_plan: Option<bool> to SetConfigArgs struct; Added if-let handler in set_config() body; Added autoCommitPlan key to get_config() JSON output
- Make slugify pub (Rust): Changed fn slugify to pub fn slugify on line 77
- Auto-commit in write_plan execute (Rust): Inserted auto-commit block lines 150-186 before Ok(msg); Git errors use eprintln! (tracing crate not used in this project); Build passes, all 17 write_plan tests pass
- Auto-commit in exit_plan_mode (Rust): Inserted auto-commit block at lines 3035-3063 after pending_handoff, before return tuple; Slug derived from filename by stripping YYYY-MM-DD_ prefix; Errors silently ignored — plan write always succeeds
- Add autoCommitPlan to TypeScript interfaces: Added autoCommitPlan?: boolean to AgentConfig after handoffContextTokens; Added autoCommitPlan?: boolean to SetConfigArgs after handoffContextTokens
- Add checkbox UI + wiring in App.tsx: Added configAutoCommitPlan signal at line 122; Added read in openConfig() after codeIntelEnabled; Added write in saveConfig() setConfig call; Added checkbox UI after code intel toggle following existing pattern
- Add i18n keys to en-US.ts: Added keys at lines 73-74 after codeIntelHint
- Add i18n keys to pt-BR.ts: Added keys at lines 73-74 after codeIntelHint
