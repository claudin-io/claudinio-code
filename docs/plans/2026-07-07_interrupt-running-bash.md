# Interromper scripts bash em execução ao pausar

## Context

**Problema**: Quando o usuário pressiona "pausar" ou ESC, o `SteeringCtl.interrupt` é setado para `true`. Isso interrompe corretamente:
- A stream HTTP da LLM (`provider::stream_message` — checagem por chunk)
- As próximas chamadas de tool (loop em `run_workflow` e `run_subagent`)

Mas **NÃO interrompe** um script bash que já está em execução. O `bash::execute()` faz `child.wait_with_output()` bloqueante dentro de `tokio::time::timeout()`, sem nenhuma checagem de interrupt durante a espera. O processo filho só é morto se der timeout.

**Impacto**: Scripts longos (`npm install`, `cargo build`, testes, etc.) continuam rodando mesmo depois do usuário pedir para parar, consumindo recursos e dando a impressão de que o pause não funciona.

## Solution Design

### Alterações

1. **`SteeringCtl.interrupt`**: Mudar de `AtomicBool` para `Arc<AtomicBool>`, permitindo compartilhar o flag com o `ToolContext` sem mudar a API pública (Deref mantém `.load()` e `.store()` funcionando).

    - **Target**: `src-tauri/src/agent/session.rs` — campo `interrupt` em `SteeringCtl` e método `clear()`
    - **Mutation**: `pub interrupt: AtomicBool` → `pub interrupt: Arc<AtomicBool>`, inicializado com `Arc::new(AtomicBool::new(false))`
    - **Why**: `Arc` permite clonar a referência para `ToolContext` sem lifetime issues. O `Arc<AtomicBool>` implementa `Deref<Target=AtomicBool>`, então todas as chamadas existentes (`.load()`, `.store()`, `.swap()`) continuam funcionando identicamente.
    - **Constraints**: Não quebrar `interrupt_session` (commands/agent.rs:440) nem `provider::stream_message`.

2. **`ToolContext`**: Adicionar campo `interrupt: Option<Arc<AtomicBool>>`.

    - **Target**: `src-tauri/src/agent/tools/mod.rs` — struct `ToolContext`
    - **Mutation**: Adicionar `pub interrupt: Option<Arc<AtomicBool>>`
    - **Why**: `ToolContext` já é injetado em toda tool via `tools::execute()`. É o canal natural para o sinal de interrupt chegar ao `bash::execute()`.
    - **Constraints**: Campo é `Option` — ferramentas que não precisam (ou testes) passam `None`.

3. **`bash::execute()`**: Substituir `tokio::time::timeout` + `wait_with_output` por `tokio::select!` que monitora simultaneamente: (a) fim do processo, (b) timeout, (c) flag de interrupt a cada ~200ms.

    - **Target**: `src-tauri/src/agent/tools/bash.rs` — função `execute()`
    - **Mutation**: 
      - Assinatura muda para `pub async fn execute(args: BashArgs, ctx: &ToolContext) -> Result<String, String>`
      - Lógica interna: spawnar child, depois `tokio::select!` entre 3 branches:
        ```rust
        tokio::select! {
            output = child.wait_with_output() => { /* process normally */ }
            _ = tokio::time::sleep(Duration::from_secs(timeout_secs)) => { /* kill, return timeout error */ }
            _ = interrupt_poll(interrupt) => { child.kill().await; return Err("Interrupted by user") }
        }
        ```
      - `interrupt_poll` é um loop que dorme 200ms e checa o flag, quebrando quando `true`
    - **Why**: Só um `select!` pode dar ao usuário controle sobre um processo bloqueante. O `kill_on_drop(true)` já garante que se a task for cancelada o child morre, mas como usamos `select!`, fazemos `.kill()` explícito e aguardamos para garantir.
    - **Constraints**: 
      - Idempotente: `child.kill()` é seguro chamar mais de uma vez
      - Testes existentes precisam ser atualizados (novo parâmetro `ctx`)
      - Quando `interrupt` é `None`, comportamento atual é preservado

4. **`tools::execute()` dispatch**: Passar `ctx` para `bash::execute()`.

    - **Target**: `src-tauri/src/agent/tools/mod.rs` — match arm `"bash"`
    - **Mutation**: `bash::execute(a).await?` → `bash::execute(a, ctx).await?`
    - **Why**: `ctx` já está disponível no escopo, só não era passado.

5. **`send_message`**: Popular `ctx.interrupt` com clone do `SteeringCtl.interrupt`.

    - **Target**: `src-tauri/src/commands/agent.rs` — construção do `ToolContext`
    - **Mutation**: Adicionar `interrupt: Some(steering.interrupt.clone())` ao struct literal
    - **Why**: Conecta o sinal de interrupt do usuário ao ToolContext que vai para as tools.

6. **`compact_session`**: Idem — popular `ctx.interrupt`.

    - **Target**: `src-tauri/src/commands/agent.rs` — construção do `ToolContext` no `compact_session`
    - **Why**: Consistência. Compaction não roda bash, mas se um dia rodar, já está correto.

7. **Testes**: Atualizar `test_ctx()` e testes em `bash.rs`.

    - **Target**: `src-tauri/src/agent/tools/mod.rs` — `test_ctx()`, e `src-tauri/src/agent/tools/bash.rs` — `fn run()` helper
    - **Mutation**: `test_ctx()` ganha `interrupt: None`; `run()` cria um `ToolContext` mínimo com `interrupt: None`
    - **Why**: Compilação e backward compat dos testes.

### O que NÃO muda

- `provider::stream_message` — já checa interrupt por chunk
- `run_workflow` / `run_subagent` — já checam interrupt entre tools
- `SteeringCtl.clear()` — mantém `self.interrupt.store(false, ...)`, que funciona via Deref
- `interrupt_session` command — mantém `ctl.interrupt.store(true, ...)`, idem
- Subagents — compartilham o mesmo `SteeringCtl` do pai, então a correção no bash os cobre automaticamente

## Risks

- **Baixo risco**: `Arc<AtomicBool>` via Deref é transparente. Nenhum código existente quebra.
- **Baixo risco**: `tokio::select!` com polling de 200ms pode adicionar latência de até 200ms entre o pause e o kill. Aceitável.
- **Nenhum risco de race**: `AtomicBool` com `Ordering::SeqCst` já é a estratégia existente.

## Tasks

1. Mudar `SteeringCtl.interrupt` para `Arc<AtomicBool>` e ajustar `new()` e `clear()`
2. Adicionar campo `interrupt: Option<Arc<AtomicBool>>` ao `ToolContext`
3. Refatorar `bash::execute()` com `tokio::select!` + interrupt polling
4. Atualizar dispatch em `tools::execute()` para passar `ctx` ao bash
5. Popular `ctx.interrupt` em `send_message` e `compact_session`
6. Atualizar testes (`test_ctx()`, `bash::run()` helper)
7. Compilar e rodar testes para verificar
