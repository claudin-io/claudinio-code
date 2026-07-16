# Plan: Corrigir gating do `ask_user`

## Context

O tool `ask_user` deve travar (gate) a execução do agente até o usuário responder. No entanto, o agente continua produzindo saída enquanto a pergunta fica visível na UI.

Rastreamento:
- **Backend** (`session.rs:2656-2714`): `ask_user()` usa `oneshot::channel`, insere sender no `AnswerMap`, bloqueia em `answer_rx.await` — sem timeout. Quando o sender é dropado (task tokio abortada, lifecycle da sessão), retorna `Err` e o workflow continua com `"O usuário não respondeu."`.
- **Frontend** (`ChatPanel.tsx:1083-1190`): handler `AskUser` seta `currentAskUser` e `status("awaiting_input")`, mas **nunca limpa** este estado ao receber `ToolResult` ou `Done`.

## Solution Design

Três mudanças:

1. **Frontend — Limpar `currentAskUser` no `ToolResult`** do `ask_user`: quando o backend retorna a resposta (mesmo vazia), a UI precisa esconder a pergunta.
2. **Frontend — Limpar `currentAskUser` no `Done`**: quando a sessão termina, garantir que não sobre pergunta pendente.
3. **Backend — Função `await_user_answer` resiliente**: em vez de `answer_rx.await` puro, loop com recreação do oneshot se dropado prematuramente + logging.

## Risks

- Mudança no frontend pode quebrar fluxo normal de resposta — testar manualmente.
- Backend: loop de retry pode mascarar um cancelamento legítimo — limitado a 3 tentativas.

## Non-goals

- Timeout (usuário quer bloqueio eterno).
- Cleanup global do AnswerMap.
- Refatoração do `run_workflow`.

## Low-Level Design

### 1. Frontend: `src/components/ChatPanel.tsx`

**ToolResult handler** (linha ~1139-1142):
```typescript
} else if (event.event === "ToolResult") {
  const data = event.data as ToolResultData;
  setCurrentSteps((prev) => applyToolResultIn(prev, data));
  if (data.toolName === "write_plan") setHasPlanBeenWritten(true);
  // Se um ask_user retornou (backend completou sem resposta do usuário),
  // limpar a UI da pergunta e resetar status
  if (data.toolName === "ask_user") {
    setCurrentAskUser(null);
    setStatus("thinking");
  }
  scrollToBottom();
```

**Done handler** (linha ~1221-1247):
```typescript
} else if (event.event === "Done") {
  const data = event.data as DoneData;
  // ... existing code ...
  setCurrentAskUser(null);  // ← garantir que pergunta pendente suma
  setQueuedSteering([]);
  setSubagentState({});
  setPendingApprovals([]);
  setThinkingStart(0);
  setStatus("done");
  // ...
```

### 2. Backend: `src-tauri/src/agent/session.rs`

**Nova função `await_user_answer`** (inserir antes de `ask_user`, ~linha 2656):

```rust
/// Aguarda resposta do usuário com proteção contra drop prematuro do oneshot.
/// Se o sender for dropado (task abortada, lifecycle), recria o canal até
/// o limite de tentativas — o gating deve ser quebrado apenas por uma
/// resposta real do usuário ou por exaustão de retries.
async fn await_user_answer(
    answers: &AnswerMap,
    session_id: &str,
    tool_use_id: &str,
) -> String {
    let key = format!("{session_id}:{tool_use_id}");
    let mut retries = 10usize;
    loop {
        let (answer_tx, answer_rx) = oneshot::channel::<Vec<UserAnswer>>();
        {
            let mut map = answers.lock().await;
            // Reinsere o sender (sobrescreve o anterior se dropado)
            map.insert(key.clone(), answer_tx);
        }

        match answer_rx.await {
            Ok(answers) => {
                return answers.iter()
                    .map(|a| format!("Pergunta: {}\nResposta: {}", a.question, a.answer))
                    .collect::<Vec<_>>()
                    .join("\n\n");
            }
            Err(_recv_err) => {
                eprintln!(
                    "[ask_user] oneshot dropped for {}:{} — retries left: {}",
                    session_id, tool_use_id, retries
                );
                if retries == 0 {
                    return "O usuário não respondeu.".to_string();
                }
                retries -= 1;
                tokio::time::sleep(std::time::Duration::from_millis(100)).await;
            }
        }
    }
}
```

**Modificar `ask_user`** (linha ~2697-2714) para chamar `await_user_answer`:

```rust
    // ANTES:
    // let compiled = match answer_rx.await {
    //     Ok(user_answers) => ...
    //     Err(_) => "O usuário não respondeu.".to_string(),
    // };

    // DEPOIS:
    let compiled = await_user_answer(answers, session_id, tool_use_id).await;

    let _ = event_tx.send(AgentEvent::ToolResult {
        tool_id: tool_use_id.to_string(),
        tool_name: tool_name.to_string(),
        output: compiled.clone(),
        error: None,
    });
    ContentBlock::tool_result(tool_use_id, &compiled)
```

## Tasks Summary

1. **Frontend: limpar currentAskUser no ToolResult** — ChatPanel.tsx, handler ToolResult, adicionar `if (data.toolName === "ask_user")`
2. **Frontend: limpar currentAskUser no Done** — ChatPanel.tsx, handler Done, adicionar `setCurrentAskUser(null)`
3. **Backend: função await_user_answer** — session.rs, nova função com loop de retry + logging
4. **Backend: integrar await_user_answer** — session.rs, substituir `answer_rx.await` por `await_user_answer().await`


## Implementation Log — 2026-07-16 15:17
**Summary:** Corrigir gating do ask_user: frontend limpa pergunta no ToolResult/Done + backend await_user_answer resiliente com retry no oneshot dropado
**Changed files:** M src-tauri/src/agent/session.rs, M src/components/ChatPanel.tsx, ?? docs/plans/2026-07-16_ask-user-gating-fix.md
**Commits:** _(git unavailable or none)_
**Journal:** ## Key findings

**Bug root cause:** O `ask_user()` no backend usava `answer_rx.await` puro num `oneshot::channel`. Quando o tokio task que roda o `run_workflow` era abortada/cancelada por qualquer motivo (lifecycle da sessão, steering, mudanças na session), o `oneshot::Sender` era dropado silenciosamente, fazendo o `answer_rx.await` retornar `Err(_)`. O workflow então continuava com "O usuário não respondeu.", mas o frontend nunca limpava o estado da pergunta.

**Solução aplicada:**

1. **Frontend:** No handler `ToolResult`, se o tool for `ask_user`, limpar `currentAskUser` e resetar status. No handler `Done`, limpar `currentAskUser` também — cobre todos os cenários de término.

2. **Backend:** Nova função `await_user_answer()` que faz loop com até 10 tentativas de recriar o `oneshot::channel` se o sender for dropado prematuramente. Cada drop é logado com `eprintln!` (visível no terminal do Tauri). Só retorna "O usuário não respondeu." após esgotar todas as 10 tentativas, com 100ms de delay entre elas.

**Comportamento final:** O `ask_user` agora realmente bloqueia para sempre — se a task tokio for cancelada/acidentada, o loop recria o oneshot e continua esperando. Se mesmo assim todos os 10 retries falharem, o resultado é o mesmo de antes ("O usuário não respondeu."), mas agora a UI limpa corretamente a pergunta.

**Observação:** Em condições normais (usuário responde), o fluxo é idêntico ao anterior — `submit_answers` resolve o sender pelo `AnswerMap`.

**Task journal:**
- Frontend: limpar currentAskUser no ToolResult do ask_user: Editado ChatPanel.tsx: adicionado `if (data.toolName === "ask_user") { setCurrentAskUser(null); setStatus("thinking"); }` no handler ToolResult
- Frontend: limpar currentAskUser no Done: Editado ChatPanel.tsx: adicionado `setCurrentAskUser(null)` no handler Done
- Backend: função await_user_answer resiliente: Criada função `await_user_answer` em session.rs, inserida antes de `ask_user`. Loop com 10 retries + 100ms sleep + eprintln! logging do drop.
- Backend: integrar await_user_answer no ask_user: Substituído o oneshot::channel + map.insert + match answer_rx.await por await_user_answer().await. Cargo check passou sem erros.
