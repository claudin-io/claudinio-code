# Mover nome do branch do GitIndicator para o header principal (App.tsx)

## Context
No layout atual, o `GitIndicator` (dentro do `ChatPanel`) renderiza o nome do branch (`main`) como um `<span>` abaixo do botão de changes. O header principal em `App.tsx` mostra o caminho do workspace (`{activeWorkspace()}`) seguido do botão de settings (engrenagem).

O usuário considera feio o branch estar ali e quer movê-lo para o header principal, abaixo do path do workspace, ao lado da engrenagem.

## Solution Design

### Mudanças

1. **`src/components/GitIndicator.tsx`** — Remover o `<Show when={branch_()}>` que renderiza o `<span>` com o nome do branch abaixo do botão de changes. O branch continua sendo obtido via polling (para uso interno, tooltip etc), mas não é mais mostrado como texto visível ali.

2. **`src/App.tsx`** — Adicionar um signal `gitBranchName` e lógica para buscar o branch do workspace ativo (usando `gitBranch()` já existente em `src/lib/ipc.ts`), com polling a cada 30s. Renderizar o nome do branch como um `<span>` abaixo do workspace path, ao lado do botão settings.

### Detalhes de layout no header do App.tsx
Estrutura atual do header (App.tsx ~linha 553):
```html
<header class="flex h-16 shrink-0 items-center border-b border-border-subtle bg-surface-1 pl-2 pr-3">
  ... logo ...
  <div class="ml-auto flex items-center gap-3">
    <span class="max-w-[280px] truncate font-mono text-[12px] text-ink-faint" ...>
      {activeWorkspace()}
    </span>
    <button onClick={openConfig} ...>
      <Icon name="settings" />
    </button>
  </div>
</header>
```

Novo layout:
```html
<div class="ml-auto flex items-center gap-3">
  <div class="flex flex-col items-end">
    <span class="max-w-[280px] truncate font-mono text-[12px] text-ink-faint" ...>
      {activeWorkspace()}
    </span>
    <Show when={gitBranchName()}>
      <span class="text-[10px] leading-none text-ink-faint">{gitBranchName()}</span>
    </Show>
  </div>
  <button onClick={openConfig} ...>
    <Icon name="settings" />
  </button>
</div>
```

### Lógica do branch em App.tsx
- Importar `gitBranch` e `checkGitAvailable` de `../lib/ipc`
- Criar `const [gitBranchName, setGitBranchName] = createSignal("")`
- Criar `const [gitAvailable, setGitAvailable] = createSignal<boolean | null>(null)`
- `checkGitAvailable().then(setGitAvailable)` no mount
- `createEffect` que quando `gitAvailable() === true` e `activeWorkspace()` existir, chama `refreshGitBranch()` e faz polling a cada 30s
- `refreshGitBranch = async () => { const b = await gitBranch(ws); setGitBranchName(b); }`

Obs: o `activeWorkspace()` só está disponível dentro do componente `App`, e o path dele não muda enquanto um workspace está aberto (muda quando o usuário troca de projeto). A lógica de polling é similar à do GitIndicator.

## Risks
- Baixo: mudança puramente estética/estrutural, sem alteração de comportamento
- GitIndicator continua funcionando normalmente, só perde a exibição do texto do branch

## Tasks summary
1. `move-branch-1` — Remover `<Show when={branch_()}>` do GitIndicator.tsx
2. `move-branch-2` — Adicionar lógica de branch signal + polling no App.tsx
3. `move-branch-3` — Renderizar branch no header do App.tsx ao lado do settings


## Implementation Log — 2026-07-12 01:57
**Summary:** Move branch name from GitIndicator to main App header below workspace path
**Changed files:** M docs/plans/2026-07-12_align-themes-to-claudinio-design-system.md, M src/App.tsx, M src/components/GitIndicator.tsx, ?? docs/plans/2026-07-12_2026-07-20-move-branch-to-header.md
**Commits:** _(git unavailable or none)_
**Journal:** 3 arquivos alterados: GitIndicator.tsx (removeu o texto do branch do sidebar do ChatPanel), App.tsx (adicionou import gitBranch/checkGitAvailable, signals gitBranchName/gitAvailable, polling a cada 30s, e renderiza o branch no header principal). O branch 'main' agora aparece abaixo do caminho do workspace no header do App, ao lado do botão de configurações, alinhado à direita — exatamente como pedido. Nenhum erro novo de compilação.

**Task journal:**
- Remover branch text do GitIndicator.tsx: Removido o `<Show when={branch_()}>` com `<span>` do JSX do GitIndicator. O `div.flex-col.items-end` wrapper também foi removido — agora só o button direto dentro do Show.
- Adicionar lógica de branch signal + polling no App.tsx: Import de `gitBranch, checkGitAvailable` adicionado na linha 15 de App.tsx; signals `gitBranchName` e `gitAvailable` + polling a cada 30s com `createEffect` + `onCleanup` adicionados após `configOverrideApiKey`.
- Renderizar branch no header do App.tsx ao lado do settings: Workspace path agora está dentro de `div.flex-col.items-end` com o branch abaixo como `<Show when={gitBranchName()}>`; botão settings permanece ao lado, no mesmo `div.ml-auto.flex.items-center.gap-3`.; tsc build: só erros pré-existentes, nenhum relacionado às nossas alterações.


## Implementation Log — 2026-07-12 01:58
**Summary:** Add git-branch icon next to branch name in header
**Changed files:** M docs/plans/2026-07-12_align-themes-to-claudinio-design-system.md, M src/App.tsx, M src/components/GitIndicator.tsx, ?? docs/plans/2026-07-12_2026-07-20-move-branch-to-header.md
**Commits:** _(git unavailable or none)_
**Journal:** After the initial 3 tasks, the user requested a git-branch icon to be placed alongside the branch name in the header. The icon `git-branch` already existed in Icon.tsx (from pixelarticons), so no new icon was needed. Only a small change to App.tsx: the branch `<span>` now wraps both the icon and the text in a `flex items-center gap-1` container. Result: `🔀 main` appears below the workspace path, next to the settings gear.

**Task journal:**
- Remover branch text do GitIndicator.tsx: Removido o `<Show when={branch_()}>` com `<span>` do JSX do GitIndicator. O `div.flex-col.items-end` wrapper também foi removido — agora só o button direto dentro do Show.
- Adicionar lógica de branch signal + polling no App.tsx: Import de `gitBranch, checkGitAvailable` adicionado na linha 15 de App.tsx; signals `gitBranchName` e `gitAvailable` + polling a cada 30s com `createEffect` + `onCleanup` adicionados após `configOverrideApiKey`.
- Renderizar branch no header do App.tsx ao lado do settings: Workspace path agora está dentro de `div.flex-col.items-end` com o branch abaixo como `<Show when={gitBranchName()}>`; botão settings permanece ao lado, no mesmo `div.ml-auto.flex.items-center.gap-3`.; tsc build: só erros pré-existentes, nenhum relacionado às nossas alterações.


## Implementation Log — 2026-07-12 01:59
**Summary:** Replace git-branch icon with exact pixelarticons SVG from user
**Changed files:** M docs/plans/2026-07-12_align-themes-to-claudinio-design-system.md, M src/App.tsx, M src/components/GitIndicator.tsx, M src/components/Icon.tsx, ?? docs/plans/2026-07-12_2026-07-20-move-branch-to-header.md
**Commits:** _(git unavailable or none)_
**Journal:** User provided a specific pixelarticons SVG for the git-branch icon. Replaced the existing 4-path definition in Icon.tsx with the exact single-path d-string from the user's SVG. The icon now renders as a single `<path>` with combined absolute/relative moves matching the pixelarticons spec. Verified: tsc passes with no new errors.

**Task journal:**
- Remover branch text do GitIndicator.tsx: Removido o `<Show when={branch_()}>` com `<span>` do JSX do GitIndicator. O `div.flex-col.items-end` wrapper também foi removido — agora só o button direto dentro do Show.
- Adicionar lógica de branch signal + polling no App.tsx: Import de `gitBranch, checkGitAvailable` adicionado na linha 15 de App.tsx; signals `gitBranchName` e `gitAvailable` + polling a cada 30s com `createEffect` + `onCleanup` adicionados após `configOverrideApiKey`.
- Renderizar branch no header do App.tsx ao lado do settings: Workspace path agora está dentro de `div.flex-col.items-end` com o branch abaixo como `<Show when={gitBranchName()}>`; botão settings permanece ao lado, no mesmo `div.ml-auto.flex.items-center.gap-3`.; tsc build: só erros pré-existentes, nenhum relacionado às nossas alterações.
