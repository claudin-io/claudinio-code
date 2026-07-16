# Fix Virtual Scroll Breaking Chat Rendering

## Context

**Commit que introduziu o bug**: `a147be1` — "feat: add workspace buffer, memory optimization plan, and refine agent tools/persistence"

**Problema**: Três sintomas com a mesma causa raiz:
1. **Input "some"** durante conversas — a área de chat fica vazia (mensagens não renderizam), dando impressão de que o input desapareceu
2. **Após pause (ESC), tudo some** — só restam os botões "Read Plan" e "Continue with Builder"
3. **Histórico não abre** — clicar em "History" e selecionar uma sessão não mostra as mensagens

**Causa raiz**: O virtual scroll (`createVirtualizer`) é criado com `count: messages().length` como snapshot único. O TanStack Virtual não rastreia reativamente mudanças no `count` quando passado como valor plano — `messages()` é avaliado uma única vez na criação do componente (normalmente `0`), e o virtualizer nunca é notificado quando o array de mensagens cresce ou é substituído.

**Evidência**: O `git diff HEAD~1 -- src/components/ChatPanel.tsx` mostra a mudança de `<For each={messages()}>` para `<For each={virtualizer.getVirtualItems()}>`. A versão antiga era reativa porque lia diretamente o sinal SolidJS. A nova versão lê `virtualItems` de um store que só é populado com base no `count` estático.

**Confirmado pelo usuário**: "quebrou duas coisas nesses ultimos trabalhos, basicamente meu input não apareceu e depois quando dei pause, sumiu tudo e ficou só os botões de trabalho". A sessão `e808ced5-1a26-4250-bb99-d338c2ebc334.jsonl` mostra o fluxo completo: sessão nova, mensagem enviada, agente trabalhando, sessão finalizada — mas sem renderização visível.

## Solution Design

**Correção**: Adicionar um `createEffect` que chama `virtualizer.setOptions({ count: messages().length })` sempre que `messages()` mudar. Isso mantém o virtualizer reativo sem recriá-lo a cada mudança.

**O que NÃO muda**: Toda a infraestrutura de `workspaceBuffer`, `pushEvent`, `drainBuffer` permanece intacta — esses funcionam corretamente e são necessários para buffering entre workspaces.

## Risks

- **Baixo**: `setOptions` é parte da API pública do TanStack Virtual e é usado internamente pelo `createComputed` do wrapper Solid. Chamá-lo em um `createEffect` externo é seguro.
- **Performance**: `setOptions` dispara `_willUpdate()` que recalcula os items virtuais. Para arrays grandes de mensagens, isso faz trabalho extra. Mitigação: o `overscan: 5` e `estimateSize` já limitam o cálculo ao viewport.

## Non-goals

- Não vamos remover o virtual scroll — ele é necessário para performance com conversas longas
- Não vamos mudar a estratégia de buffering (`workspaceBuffer`)
- Não vamos alterar a lógica de interrupt/pause

## Low-Level Design

### Arquivo: `src/components/ChatPanel.tsx`

**Ponto de inserção**: Após a criação do `virtualizer` (linha ~1050), antes do `addOrUpdateToolIn`.

**Mudança**: Adicionar um `createEffect`:

```tsx
// Keep virtualizer count in sync with messages signal
createEffect(() => {
  virtualizer.setOptions({ count: messages().length });
});
```

**Por que funciona**: 
- `createEffect` rastreia `messages().length` como dependência reativa
- Quando `messages()` muda (novo send, reopenSession, Done promovido, etc.), o efeito dispara
- `virtualizer.setOptions({ count })` faz merge do novo count e chama `_willUpdate()` internamente, recalculando `virtualItems` e `totalSize`
- O `<For each={virtualizer.getVirtualItems()}>` e `min-height: virtualizer.getTotalSize()` já são reativos (o proxy do solid-virtual retorna signals para `getVirtualItems` e `getTotalSize`)

### Arquivos NÃO alterados:
- `src/lib/workspaceBuffer.ts` — funciona corretamente
- `src-tauri/` — nenhum código Rust afetado
- `node_modules/@tanstack/solid-virtual` — biblioteca externa, não modificamos

## Verification Plan

1. **Build**: `pnpm run build` deve passar (vitest + vite build)
2. **Testes**: `pnpm test` deve passar (testes existentes do ChatPanel)
3. **Cenário 1 — Nova conversa**: Abrir workspace, digitar mensagem, enviar. Verificar que a mensagem do usuário aparece no chat e o agente responde visivelmente.
4. **Cenário 2 — Interrupt (ESC)**: Durante o agente trabalhando, pressionar ESC. Verificar que o status muda para "done" e a mensagem "Pausado pelo usuário" aparece. Em Brain mode com plano escrito, os botões "Read Plan" e "Continue with Builder" devem aparecer JUNTO com as mensagens, não sozinhos.
5. **Cenário 3 — Histórico**: Clicar em "History", selecionar uma sessão anterior. Verificar que as mensagens da sessão carregam e são visíveis.
6. **Cenário 4 — Nova sessão**: Clicar "New", confirmar. Verificar que o chat limpa corretamente e uma nova conversa pode ser iniciada.

## Tasks Summary

1. `fix-virtualizer-count` — Adicionar `createEffect` para manter `count` do virtualizer em sync com `messages().length` em `src/components/ChatPanel.tsx`
2. `verify-fix` — Rodar `pnpm test` e `pnpm run build` para garantir que nada quebrou


## Implementation Log — 2026-07-16 19:44
**Summary:** Added createEffect to sync virtualizer count with SolidJS messages signal — one line fix
**Changed files:** M src/components/ChatPanel.tsx, ?? docs/plans/2026-07-16_fix-virtual-scroll-chat-rendering.md
**Commits:** _(git unavailable or none)_
**Journal:** Root cause: TanStack Virtual's `createVirtualizer` takes a snapshot of options at creation time. `count: messages().length` was evaluated once (typically 0) and never updated when the SolidJS signal changed. The `<For each={virtualizer.getVirtualItems()}>` therefore always rendered an empty array.

Fix: A single `createEffect(() => { virtualizer.setOptions({ count: messages().length }); })` bridges the gap — SolidJS reactivity drives virtualizer re-renders. This fixes all three symptoms (disappearing input, pause clearing chat, history not loading) because they all share the same underlying issue: messages were being added to the signal but never rendered.

Gotcha: The TanStack wrapper's `createComputed` sets up an `onChange` handler that reconciles the store, but it does NOT re-read the top-level options like `count` — that's only read once at construction time. The `setOptions` call in our `createEffect` triggers `_willUpdate()` which recalculates the virtual item range.

**Task journal:**
- Add createEffect to sync virtualizer count with messages: Added createEffect(() => { virtualizer.setOptions({ count: messages().length }); }) right after the virtualizer creation block. This bridges the gap between SolidJS reactive signals and TanStack Virtual's non-reactive count option — without it, count stayed frozen at 0 and no messages ever rendered.
- Run tests and build to verify nothing broke: pnpm test: 35 test files, 639 tests all passed. pnpm run build: vitest 639 passed, vite build successful (5.1MB main bundle). No regressions.
