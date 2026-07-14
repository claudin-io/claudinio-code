# Plano: Corrigir custo de subagents não incluído no total da sessão

## Context / Problem

Na imagem fornecida pelo usuário, a soma dos custos individuais dos 4 subagents é **$0.0826**, mas o total exibido no rodapé é **$0.0613** — uma diferença de **~$0.0213**.

### Root Cause

O fluxo de dados tem um furo:

1. **`subagent.rs` — `run_spawn_agents()`** (linha ~233) retorna `(ContentBlock, u32, u32)` — blocos, tokens_in, tokens_out. O custo acumulado de cada subagent (`result.cost`) é enviado via `AgentEvent::SubagentDone` para o frontend renderizar, mas **não é retornado** no tuple.

2. **`session.rs` — caller** (linha ~1783):
```rust
let (block, sub_in, sub_out) = subagent::run_spawn_agents(...).await;
total_in += sub_in;
total_out += sub_out;
// ⚠️ sub_cost NUNCA é capturado ou adicionado a run_cost_*
```

3. **Consequência**: `roll_cost()` usa `run_cost_input/output/cache` que contêm apenas o custo do modelo principal. O custo dos subagents é perdido do `cumul_cost`, que alimenta `SessionStats.cumulativeCost` → `ContextFooter`.

**Casos específicos**:
- **Provider reporta custo real** (`run_cost_*` não-None): o custo subagent é completamente omitido.
- **Provider não reporta custo** (fallback para estimativa `cost_breakdown_for`): o `total_in`/`total_out` inclui tokens dos subagents, então a estimativa é acidentalmente maior — mascara parcialmente o bug.

## Solução

Três mudanças pontuais:

### 1. `run_spawn_agents` retorna custo total

`subagent.rs` — `run_spawn_agents()`:
- Adicionar `let mut total_cost: f64 = 0.0;` no escopo da função.
- No loop, após `total_in += result.in_tok; total_out += result.out_tok;` adicionar `total_cost += result.cost;`.
- Mudar o retorno de `(ContentBlock, u32, u32)` para `(ContentBlock, u32, u32, f64)`.

Essa função tem 3 `return` statements (early exits para erro/parse), todos retornando custo 0.0, e 1 return final que deve retornar o `total_cost` acumulado.

### 2. Caller captura e acumula o custo

`session.rs` — linha ~1783:
```rust
let (block, sub_in, sub_out, sub_cost) = subagent::run_spawn_agents(...).await;
total_in += sub_in;
total_out += sub_out;
// Adiciona custo dos subagents ao run_cost_input (entra na conta do cumulativeCost)
```

Como `run_cost_*` são Option<f64> que tracking breakdown por input/output/cache, a forma mais limpa de incluir subagent cost é:
- Adicionar um novo parâmetro `subagent_cost: f64` a `roll_cost()`.
- Dentro de `roll_cost`, adicionar `subagent_cost` diretamente ao `*cumul_cost` (sem tocar em `cumul_cost_input/output/cache` — o breakdown continua refletindo apenas o modelo principal).

```rust
fn roll_cost(..., subagent_cost: f64, ...) {
    ...
    *cumul_cost = Some(cumul_cost.unwrap_or(0.0) + ci + co + cc + subagent_cost);
    ...
}
```

### 3. Atualizar `roll_cost` signature e todas as call sites (4)

Em `session.rs`, `roll_cost()` recebe o novo parâmetro `subagent_cost: f64`. Atualizar todas as 4 chamadas:

| Linha | Contexto | Valor |
|-------|----------|-------|
| ~1365 | Done normal | `subagent_cost` |
| ~1434 | Done com turno vazio (early exit) | `subagent_cost` (0.0 se não houve subagents) |
| ~1682 | Interrupt | `subagent_cost` |
| ~1931 | Fim do golden loop | `subagent_cost` |

Em todos os casos, o caller tem a variável `subagent_cost` disponível, inicializada como `0.0` e atualizada apenas quando `spawn_agents` é chamado. Isso garante idempotência — se não houve subagents no round, custo 0.0.

---

## Arquivos Alterados

| Arquivo | Mudança |
|---------|---------|
| `src-tauri/src/agent/subagent.rs` | `run_spawn_agents` retorna `(ContentBlock, u32, u32, f64)`; acumula `total_cost` |
| `src-tauri/src/agent/session.rs` | `roll_cost` recebe `subagent_cost: f64`; caller captura e passa o custo |
| `src-tauri/src/agent/subagent.rs` | `run_subagent` já retorna `SubagentResult { cost }` — sem mudança |

## Verificação

1. **Build:** `cargo build` compila sem erros — testa que todas as call sites foram atualizadas.
2. **Testes:** `cargo test` — verificar que `session_stats_without_cost`, `cost_for`, e `cumulative_stats` ainda passam (nenhum testa subagent cost diretamente, mas confirmar que não quebramos nada).
3. **End-to-end:** Rodar uma sessão com múltiplos `spawn_agents`, verificar no frontend que a soma dos custos individuais dos subagents + custo do modelo principal bate com o total exibido no rodapé.

## Riscos

- **Baixo**: Mudanças restritas a 2 arquivos, tipos bem definidos, 4 call sites todas no mesmo arquivo.
- **Nenhum risk de regressão de custos existentes**: `subagent_cost` é 0.0 por default — sessões sem subagents têm comportamento idêntico.


## Implementation Log — 2026-07-14 06:14
**Summary:** Subagent costs now included in cumulative total — cost data flows end-to-end from subagent results through roll_cost to the frontend
**Changed files:** M src-tauri/src/agent/session.rs, M src-tauri/src/agent/subagent.rs, ?? docs/plans/2026-07-14_subagent-cost-missing-from-total.md
**Commits:** _(git unavailable or none)_
**Journal:** Key findings & decisions:
- Subagent cost was already correctly tracked in `SubagentResult.cost` and sent to the frontend via `AgentEvent::SubagentDone` — the fix was purely about propagating it back to the cost-rolling function.
- `run_spawn_agents` returns were updated: function signature to `(ContentBlock, u32, u32, f64)`, `total_cost` accumulator in the main loop, all 3 early returns return `0.0` (no subagents ran).
- `roll_cost` got a new `subagent_cost: f64` parameter between the breakdown args and the mutable accumulators. It's added to `cumul_cost` but NOT to per-category breakdowns (input/output/cache) since those are token-estimate-based categories.
- The caller `run_workflow_with_profile` now declares `let mut subagent_cost: f64 = 0.0` alongside the other accumulators, captures `sub_cost` from `run_spawn_agents`'s 4th return value, and passes it to all 5 `roll_cost()` call sites.
- All 204 tests pass; no new tests needed since this is a structural propagation fix, not new behavior.

**Task journal:**
- run_spawn_agents retorna total_cost: subagent.rs: run_spawn_agents agora retorna (ContentBlock, u32, u32, f64) — total_cost é acumulado no loop e 0.0 nos early returns
- roll_cost recebe subagent_cost: session.rs: roll_cost agora aceita subagent_cost: f64, adicionado ao cumul_cost
- Caller captura e propaga subagent_cost nas 4 call sites de roll_cost: session.rs: subagent_cost capturado no caller de run_spawn_agents; passado para todas as 5 call sites de roll_cost
- Build e testes passam: cargo build: OK sem erros. cargo test: 204 passed, 0 failed, 3 ignored (live API tests).
