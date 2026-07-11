# Plano: Widget de Pergunta no Commit & Push

## Contexto / Problema

O `CommitPushModal` é usado para executar o fluxo de "Commit & Push" via agente. Ele já trata eventos `ToolCall`, `ToolResult`, `Thinking`, `TextStep`, `Done` e `Error` — mas **não trata o evento `AskUser`**.

Quando o agente faz uma pergunta (ex: "Você está na branch main com mudanças não commitadas. Devo criar um novo branch ou commitar direto na main?"), o evento `{ event: "AskUser", data: {...} }` enviado pelo backend (`session.rs:2477`) é **silenciosamente ignorado** pelo modal. O usuário vê o JSON bruto do `ask_user` como se fosse um `ToolCall` comum — sem poder responder.

O componente `QuestionCard` já existe dentro de `ChatPanel.tsx` (linhas 3190–3330) com suporte a radio/checkbox, opção "Outra resposta", campo de texto livre, e submit via `submitAnswers()`.

## Solução

Extrair `QuestionCard` para um arquivo próprio e reutilizá-lo dentro do `CommitPushModal`.

## Mudanças

### 1. Extrair `QuestionCard` para `src/components/QuestionCard.tsx`

- Mover a definição completa de `QuestionCard` (componente + interface `QuestionDraft`) de `ChatPanel.tsx:3184-3330` para novo arquivo.
- Manter imports necessários (`Component`, `createSignal`, `For`, `Show`, `Icon`, `t`, `AskUserData`, `UserAnswer`).
- **Não** mudar lógica interna — é um refactor puro.

### 2. Importar `QuestionCard` em `ChatPanel.tsx`

- Remover o código inline do `QuestionCard`.
- Importar de `./QuestionCard`.
- `handleAnswers`, `handleEvent` (caso `"AskUser"`) e `currentAskUser` permanecem no `ChatPanel.tsx`.

### 3. Adicionar suporte a `AskUser` no `CommitPushModal.tsx`

- Importar: `QuestionCard`, `submitAnswers`, `AskUserData`, `UserAnswer` (já vem de `ipc.ts`).
- Adicionar estado: `const [currentAskUser, setCurrentAskUser] = createSignal<AskUserData | null>(null)`.
- No `handleEvent`, adicionar caso `"AskUser"`:
  ```ts
  case "AskUser":
    setCurrentAskUser(event.data as AskUserData);
    break;
  ```
- Adicionar `handleAnswers`:
  ```ts
  const handleAnswers = async (answers: UserAnswer[]) => {
    const ask = currentAskUser();
    if (!ask) return;
    try {
      await submitAnswers(ask.sessionId, ask.toolId, answers);
      setCurrentAskUser(null);
    } catch (e) {
      console.error("Failed to submit answers:", e);
    }
  };
  ```
- No JSX, renderizar o `<QuestionCard>` quando houver pergunta ativa:
  ```tsx
  <Show when={currentAskUser()}>
    <div class="px-5 pb-3">
      <QuestionCard ask={currentAskUser()!} onSubmit={handleAnswers} />
    </div>
  </Show>
  ```
  - Colocar **acima** da timeline ou **entre** a timeline e o footer, para ficar visível.

## Riscos

- `QuestionCard` atualmente acessa `t("chat.question.needsAnswer")` etc. — esses locales já existem e são carregados globalmente, sem risco.
- `submitAnswers` precisa do `sessionId`. O `CommitPushModal` já armazena `sessionId` (variável local, linha `let sessionId: string | null = null`). O evento `AskUserData` também carrega `sessionId`, então podemos usar de qualquer fonte.
- O `handleEvent` do `CommitPushModal` é callback passado para `commitAndPush` — está recebendo o evento real do backend. `AskUser` é um dos tipos de evento suportados pelo canal.

## Verificação

1. **Build:** `pnpm tauri build` ou `pnpm dev` sem erros de TypeScript.
2. **Visual:** Abrir o modal de Commit & Push quando houver mudanças. Se o agente chamar `ask_user`, o widget de pergunta (radio/checkbox + "Outra resposta" + submit) deve aparecer dentro do modal.
3. **Funcional:** Responder à pergunta e confirmar que o fluxo continua (agente recebe a resposta e prossegue).
4. **Regressão:** O `ChatPanel` continua funcionando normalmente com `QuestionCard` (perguntas no chat normal).

## Tasks

1. Extrair QuestionCard para src/components/QuestionCard.tsx
2. Atualizar ChatPanel.tsx para importar de QuestionCard
3. Adicionar AskUser handler no CommitPushModal.tsx


## Implementation Log — 2026-07-11 22:40
**Summary:** Add AskUser question widget support in CommitPushModal
**Changed files:** M src-tauri/src/commands/git.rs, M src-tauri/src/lib.rs, M src/components/ChatPanel.tsx, M src/components/CommitPushModal.tsx, M src/components/GitIndicator.tsx, M src/lib/ipc.ts, ?? docs/plans/2026-07-11_2026-07-17-git-availability-guard.md, ?? docs/plans/2026-07-11_ask-user-widget-commit-push.md, ?? src/components/QuestionCard.tsx
**Commits:** _(git unavailable or none)_
**Journal:** ## Key findings & decisions

**Problem:** CommitPushModal ignored the `AskUser` event entirely — when the agent asked a question (e.g. "create branch vs commit to main"), the raw JSON was displayed as a tool call log line with no interactive widget.

**Solution:** Extracted QuestionCard (radio/checkbox + "Other answer" + free-text input + submit button) from ChatPanel.tsx into its own file `src/components/QuestionCard.tsx`, then reused it in CommitPushModal.

**Gotchas:**
- The initial edit_file on ChatPanel.tsx to remove the inline QuestionCard only caught the *first* occurrence — but there was a *second* duplicate copy of QuestionCard (with its own imports) further down the file. Had to remove it separately.
- A leftover `import { createSignal, For, Show, type Component } from "solid-js"` was left orphaned between the end of the first QuestionCard and the second one. Removed that too.
- QuestionCard was already well-structured as a standalone component with no ChatPanel-specific dependencies — extraction was clean.

**Files changed:**
- `src/components/QuestionCard.tsx` (NEW) — extracted component
- `src/components/ChatPanel.tsx` — replaced inline QuestionCard with import
- `src/components/CommitPushModal.tsx` — added AskUser handler + QuestionCard rendering

**Verification:** `npx tsc --noEmit` shows zero new errors. The 46 pre-existing errors are all in unrelated test files and other components.

**Task journal:**
- Extrair QuestionCard para src/components/QuestionCard.tsx: QuestionCard.tsx criado com sucesso.; Import em ChatPanel.tsx atualizado.; Removido código inline duplicado do ChatPanel.tsx.
- Atualizar ChatPanel.tsx para importar QuestionCard: Import adicionado na linha 64.; JSX usage na linha 2004 mantido.; Nenhum resquício de QuestionCard/QuestionDraft no ChatPanel.tsx (confirmado via grep).
- Adicionar suporte a AskUser no CommitPushModal.tsx: Import de QuestionCard, submitAnswers, AskUserData, UserAnswer adicionado.; Estado currentAskUser adicionado.; Case 'AskUser' no handleEvent - seta currentAskUser.; handleAnswers criado - chama submitAnswers.; QuestionCard renderizado no topo da timeline quando currentAskUser() for truthy.; Build limpo - zero novos erros de TS (46 erros pré-existentes não relacionados).
