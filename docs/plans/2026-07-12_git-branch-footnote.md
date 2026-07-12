# Git Branch Footnote no ChatPanel Header

## Context
O `GitIndicator` no ChatPanel já obtém (via `gitBranch()`) e faz polling do nome do branch a cada 30s, mas apenas o mostra na `tooltip` do botão. O utilizador quer que o branch seja visível como uma mini-legenda read-only por baixo do botão (ou do label "0 changes" / "X changes"), no header do ChatPanel.

## Solution Design
1. **`GitIndicator.tsx`** — o componente já tem o signal `branch_()`. Basta adicionar um `<span>` com o nome do branch por baixo do botão existente.
2. O layout fica: botão com icon + label (ex: "3 changes (+5 −2)") na mesma linha, e por baixo o nome do branch em texto pequeno e muted.
3. Só renderiza quando gitAvailable é true e branch não é vazio.

### Estrutura do HTML (antes vs depois)

**Antes:**
```tsx
<button onClick={...} title={tooltip()} class={btnClass()}>
  <Icon name="diff" class="h-3.5 w-3.5" />
  <span>{label()}</span>
</button>
```

**Depois:**
```tsx
<div class="flex flex-col items-end">
  <button onClick={...} title={tooltip()} class={btnClass()}>
    <Icon name="diff" class="h-3.5 w-3.5" />
    <span>{label()}</span>
  </button>
  <Show when={branch_()}>
    <span class="text-[10px] leading-none text-ink-faint">{branch_()}</span>
  </Show>
</div>
```

### Ficheiros a alterar
- `src/components/GitIndicator.tsx` — adicionar footnote com o branch
- Nenhuma alteração a i18n, IPC, ou backend (branch já existe e é populado)

## Risks
- Risco baixo. A alteração é puramente no template JSX do `GitIndicator`.
- O branch pode ser vazio ou null → já tratamos com `<Show when={branch_()}>`.

## Tasks
1. `git-branch-footnote-1` — Alterar `GitIndicator.tsx` para renderizar o branch name como footnote por baixo do botão
2. `git-branch-footnote-2` — Verificar que o projeto compila sem erros


## Implementation Log — 2026-07-12 01:21
**Summary:** Git branch footnote added below the GitIndicator in ChatPanel header
**Changed files:** M src/components/GitIndicator.tsx, ?? docs/plans/2026-07-12_git-branch-footnote.md
**Commits:** _(git unavailable or none)_
**Journal:** The change was minimal and clean: wrapped the existing `<button>` in a `<div class="flex flex-col items-end">` and added a `<Show when={branch_()}>` rendering the branch name as a small muted footnote beneath the git indicator label. No new signals, no new IPC calls, no new i18n keys — the branch signal was already being polled every 30s, it just wasn't rendered visually. Build passed with 0 errors across 402 tests and 1451 modules.

**Task journal:**
- Adicionar footnote do branch ao GitIndicator: Plan file: docs/plans/2026-07-12_git-branch-footnote.md; GitIndicator.tsx já tem o signal `branch_()` populado por polling a cada 30s — só falta renderizá-lo no JSX.; O branch já está na tooltip do botão; agora passa a ficar visível permanentemente como footnote.; Ficheiro alterado com sucesso — JSX do return atualizado: div flex-col + Show when branch_() com texto small muted.
- Verificar compilação: pnpm build OK: 402 tests passed, 1451 modules transformed, 92 output chunks gerados. Zero erros.
