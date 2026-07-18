# Network Indicator — Remover número, manter apenas ícone

## Context
O NetworkIndicator na status bar mostra um ícone globe + um número de conexões ativas + tooltip com detalhes. O usuário quer simplificar: remover o número, deixar apenas o ícone piscando quando houver atividade de rede.

## Solution Design
- **O que muda:** Remover o `<span>` com `ops().length` dentro do `<button>` do `NetworkIndicator`.
- **O que fica igual:** Ícone `globe` com classe `animate-pulse` condicional (`active()`), tooltip de hover com detalhes, cores accent/ink-faint conforme atividade.
- **UX:** Quando há atividade → ícone acende (accent) e pulsa. Quando não há → ícone fica apagado (ink-faint) e estático. Tooltip revela detalhes no hover.

## Risks
- Nenhum. Mudança puramente de remoção de markup, sem alteração de lógica ou estado.

## Non-goals
- Não alterar animação do ícone (já usa `animate-pulse`)
- Não alterar tooltip
- Não alterar backend/net_activity.rs

## Low-Level Design

### Arquivos tocados
1. **`src/components/NetworkIndicator.tsx`** — remover `<Show when={active()}><span>{ops().length}</span></Show>` (linhas 42-44 atuais).

### Padrão existente
O componente já condiciona o pulso do ícone via `active()`:
```tsx
<Icon name="globe" class={"h-3.5 w-3.5" + (active() ? " animate-pulse" : "")} />
```
A cor do botão também alterna entre `text-accent` e `text-ink-faint` conforme `active()`. Nenhuma lógica nova necessária.

### Mudança exata
Apenas deletar o bloco:
```tsx
<Show when={active()}>
  <span>{ops().length}</span>
</Show>
```
do arquivo `src/components/NetworkIndicator.tsx`.

Nenhuma dependência, import, ou estado afetado.

## Tasks summary
1. Remover o `<span>` com número de conexões ativas do NetworkIndicator


## Implementation Log — 2026-07-18 08:43
**Summary:** Removeu o número de conexões ativas do NetworkIndicator — apenas o ícone globe permanece visível na status bar
**Changed files:** M docs/plans/2026-07-18_system-stats-indicator.md, M src/components/NetworkIndicator.tsx, ?? docs/plans/2026-07-18_network-indicator-remove-number.md
**Commits:** _(git unavailable or none)_
**Journal:** Remoção limpa de 3 linhas no NetworkIndicator. O botão agora tem apenas o ícone globe — quando há atividade, ícone acende (text-accent) e pulsa (animate-pulse); quando inativo, fica apagado (text-ink-faint) e estático. Tooltip com detalhes das conexões ativas preservado intacto no hover. Nenhuma alteração de lógica, estado ou backend. tsc --noEmit passou sem erros.

**Task journal:**
- Remover número do NetworkIndicator: Removed 3 lines (<Show>, <span>, </Show>) from button body. Button now contains only globe Icon. All other markup, signals (active, hovered, ops), and tooltip remain intact. tsc --noEmit passes with 0 errors.
