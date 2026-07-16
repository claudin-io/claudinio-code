## Context

O `SYSTEM_PROMPT` (constante em `session.rs`) contém a frase "Max 4 per call" como texto hardcoded, mas o settings já tem `max_parallel_agents` (slider 1-8) que é usado dinamicamente na definição da tool `spawn_agents` (no `maxItems` do schema e na `description`). Isto cria uma inconsistência: o modelo vê "Max 4" no prompt mas "1-8" na tool.

## Solution Design

Substituir "Max 4 per call" por "Max {n} per call" no `SYSTEM_PROMPT`, onde `{n}` é o valor real de `max_parallel_agents` das settings.

**Mudanças necessárias:**
1. A função `system_prompt()` deve receber o valor `max_parallel_agents: usize` como parâmetro adicional
2. O `SYSTEM_PROMPT` deve usar um placeholder `{max_parallel}` que é substituído na formatação final
3. Todos os call sites de `system_prompt()` devem passar o valor (extraído via `effective_max_parallel(config)`)
4. Os testes existentes devem continuar a passar (já usam `MAX_PARALLEL_AGENTS` = 4, que corresponde ao default)

## Risks

- **Baixo risco**: a tool `spawn_agents` já usa o valor dinâmico, portanto a mudança é apenas alinhar o texto do prompt
- O `GIT_SYNC_PROMPT` não menciona subagents, portanto não precisa de alteração
- O `SUBAGENT_SYSTEM_PROMPT` (subagent.rs) também não menciona máximo — sem alterações

## Non-goals

- Não alterar a lógica do `effective_max_parallel` nem os limites
- Não alterar a UI nem o slider de settings
- Não alterar o prompt dos subagents
- Não alterar a definição da tool `spawn_agents` (já está correta)

## Low-Level Design

### Arquivo: `src-tauri/src/agent/session.rs`

#### 1. Alterar a constante `SYSTEM_PROMPT` (linha ~325)

De:
```
- Use for broad/parallel tasks. Max 4 per call. Modes: 'explore' or 'code'.
```
Para:
```
- Use for broad/parallel tasks. Max {max_parallel} per call. Modes: 'explore' or 'code'.
```

#### 2. Alterar a assinatura de `system_prompt()` (linha ~389)

Adicionar parâmetro `max_parallel: usize`:

```rust
fn system_prompt(
    workspace_root: Option<&str>,
    skills_section: Option<&str>,
    plan_save_path: Option<&str>,
    mode: SessionMode,
    profile: PromptProfile,
    max_parallel: usize,
) -> String {
```

#### 3. Substituir o placeholder na formatação

No início da função `system_prompt()`, adicionar a substituição depois que `base` é construído:

```rust
let base = base.replace("{max_parallel}", &max_parallel.to_string());
```

Isto garante que o placeholder é substituído em TODOS os modos (Brain, Builder, GitSync — embora GitSync não mencione subagents, não faz mal substituir).

#### 4. Actualizar call sites em `run_workflow()` (linhas ~1123, ~1289, ~1314)

Cada call site tem acesso a `config`. Adicionar `effective_max_parallel(config)` como último argumento.

Linha ~1123:
```rust
let mut system = system_prompt(ctx.workspace_root.as_deref(), skills_section.as_deref(), ctx.plan_save_path.as_deref(), cur_mode, profile, effective_max_parallel(config));
```

Linha ~1289:
```rust
system = system_prompt(ctx.workspace_root.as_deref(), skills_section.as_deref(), ctx.plan_save_path.as_deref(), cur_mode, profile, effective_max_parallel(config));
```

Linha ~1314:
```rust
system = system_prompt(
    ctx.workspace_root.as_deref(),
    skills_section.as_deref(),
    ctx.plan_save_path.as_deref(),
    cur_mode,
    profile,
    effective_max_parallel(config),
);
```

#### 5. Actualizar todos os call sites de teste

Todos os testes em `session.rs` chamam `system_prompt(Some(ROOT), None, None, SessionMode::..., PromptProfile::...)`. Adicionar `MAX_PARALLEL_AGENTS` (que é 4, igual ao hardcoded anterior) como último argumento.

Exemplo:
```rust
let sys = system_prompt(Some(ROOT), None, None, SessionMode::Brain, PromptProfile::Standard, MAX_PARALLEL_AGENTS);
```

Isto garante que os testes não quebram — o valor 4 é idêntico ao "Max 4" que estava hardcoded.

### Arquivo: NENHUM outro ficheiro precisa de alteração

- `subagent.rs` — o `SUBAGENT_SYSTEM_PROMPT` não menciona máximo
- `tools/mod.rs` — a tool `spawn_agents` já usa o valor dinâmico corretamente
- Nenhum frontend — o slider já existe

### Data flow

```
Settings (max_parallel_agents) → effective_max_parallel(config) → system_prompt(..., max_parallel) → SYSTEM_PROMPT.replace("{max_parallel}", ...) → LLM
```

### Verificação

1. `cargo build` — compilar sem erros
2. `cargo test` — todos os testes passam (o valor 4 mantém o comportamento anterior)
3. Teste manual: alterar o slider para 8, verificar que o prompt contém "Max 8 per call"

### Tasks summary

1. Alterar SYSTEM_PROMPT placeholder e assinatura de system_prompt()
2. Actualizar call sites em run_workflow() com effective_max_parallel(config)
3. Actualizar call sites de teste com MAX_PARALLEL_AGENTS
4. Build + testes para verificar


## Implementation Log — 2026-07-16 22:40
**Summary:** Made 'Max X per call' in system prompt dynamic, reading from max_parallel_agents settings
**Changed files:** M src-tauri/src/agent/session.rs, ?? docs/plans/2026-07-16_dynamic-max-subagents-in-prompt.md
**Commits:** _(git unavailable or none)_
**Journal:** All 4 tasks completed. The change is minimal and clean: one constant edit, one signature change, one .replace() call, and mechanical argument additions at call sites. The key design decision was to use `{max_parallel}` as a simple string placeholder in SYSTEM_PROMPT rather than a more complex templating system — consistent with how `{plans_subdir}` is already handled in the mode-specific prompt blocks. The GitSync prompt is unaffected since it never mentions subagents. Build: 0 new warnings. Tests: 225 passed, 0 failed.

**Task journal:**
- Alterar SYSTEM_PROMPT placeholder e assinatura: SYSTEM_PROMPT: 'Max 4' -> 'Max {max_parallel}'; system_prompt() signature: added max_parallel: usize parameter; Added base.replace("{max_parallel}", ...) after plans_subdir resolution, before mode match
- Actualizar call sites em run_workflow(): 3 call sites updated: lines 1125, 1291, 1316 — all now pass subagent::effective_max_parallel(config)
- Actualizar call sites de teste: All 8 test call sites updated with subagent::MAX_PARALLEL_AGENTS
- Build e testes: cargo build: OK (only pre-existing warnings); cargo test --lib: 225 passed, 0 failed, 3 ignored; prompt_eval_tests: 7 passed, 0 failed, 2 ignored (live API)
