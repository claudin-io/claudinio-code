# Plan: TasksPanel Popover — max-height + scroll

## Context / Problem Statement
O popover de detalhes das tasks (card que aparece ao passar o mouse sobre os dots coloridos na barra direita) não tem limite de altura. Quando uma task tem descrição longa ou muitos itens de journal, o card cresce verticalmente e ultrapassa o viewport, cortando o conteúdo que fica fora da tela — o usuário não consegue ver nem scrollar.

**O usuário confirmou**: apenas `max-h-[50vh] + overflow-y-auto` já resolve; não é necessário reposicionar o card dinamicamente.

## Goal (Definition of Done)
O popover de task detalhada tem `max-h-[50vh]` com scroll vertical (`overflow-y-auto`), garantindo que nunca ultrapasse metade da viewport e que o conteúdo excedente seja acessível via scroll.

## Key Findings (Prova Real)
- **Arquivo alvo**: `src/components/TasksPanel.tsx`, linha ~148 — o `<div>` do popover dentro do `<Portal>`.
- **Classes atuais do div**: `"w-64 rounded-lg bg-surface-1 p-3"` — sem restrição de altura nem scroll.
- **Projeto usa Tailwind CSS** (`@import "tailwindcss"` em `App.css`), então as classes utilitárias `max-h-[50vh]` e `overflow-y-auto` estão disponíveis nativamente.
- **Não há estilo customizado de scrollbar no projeto** — o scroll padrão do browser é suficiente e consistente com o resto da UI.

## Changes (Steps)

### 1. Adicionar `max-h-[50vh] overflow-y-auto` ao popover em TasksPanel.tsx
- **Target**: `src/components/TasksPanel.tsx`, linha ~148, o `<div>` do popover
- **Mutation**: Adicionar as classes `max-h-[50vh] overflow-y-auto` ao className
- **Why**: Limita o card a 50% da altura da viewport e permite scroll vertical quando o conteúdo excede
- **Constraints**: Apenas adição de classes Tailwind; nenhuma mudança de lógica, estrutura ou estilos

## Verification Plan
1. **Visual**: Renderizar o popover com uma task que tenha descrição longa + vários journal entries → confirmar que o card não ultrapassa 50vh e que o scroll funciona
2. **Regressão**: Tasks com descrições curtas devem continuar com a mesma aparência (sem scrollbar visível, altura natural)
3. **Testes**: Rodar `pnpm test -- TasksPanel` para garantir que os testes existentes passam

## Risks
- Nenhum. Mudança puramente aditiva de classes CSS utilitárias.


## Implementation Log — 2026-07-11 00:51
**Summary:** fix(tasks-panel): limit popover height to 50vh with scroll
**Changed files:** M	src/components/TasksPanel.tsx
**Commits:** 1b1bc68 fix(tasks-panel): limit popover height to 50vh with scroll
**Journal:** Popover de detalhes das tasks agora com max-h-[50vh] + overflow-y-auto. A mudança foi minimalista — apenas 2 classes Tailwind adicionadas ao className do popover div. O card cresce naturalmente até 50% da viewport e a partir daí o conteúdo excedente fica acessível via scroll. Nenhuma lógica alterada, nenhum estilo customizado necessário.

**Task journal:**
- Add max-h-[50vh] overflow-y-auto to TasksPanel popover: Added max-h-[50vh] overflow-y-auto to the popover div in TasksPanel.tsx line ~148.
- Verify: tests pass + build success: 384/384 tests passed (23 test files); Build completo com Vite passou sem erros; Alteração minimalista: +2 classes Tailwind, sem mudanças estruturais
