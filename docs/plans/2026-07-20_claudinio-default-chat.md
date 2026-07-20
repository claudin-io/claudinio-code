# Plan: `claudinio` puro abre o chat por default

## Context

O usuário pediu, em PT-BR: "gostaria que ao chamar claudinio, abrisse o chat por default e nao precisar de claudinio chat".

Hoje o binário `claudinio` SEM subcomando já abre a TUI de chat: `cli/src/main.rs:88` faz `match cli.command.unwrap_or(Command::Chat { path: None })`. A funcionalidade foi implementada em `bf00d97 feat(cli): add claudinio CLI + minimalist ratatui TUI over the shared core`, mas a documentação do pacote (`npm/claudinio/README.md:14`) ainda diz `npx claudinio chat  # TUI interativa` como caminho canônico, sem mencionar que rodar `claudinio` puro já é equivalente.

Decisão confirmada pelo usuário: **só documentar e nada mudar no código**.

## Goal (Definition of Done)

1. `docs/plans/2026-07-20_claudinio-default-chat.md` descreve o estado real (código já faz o default) e marcar como implementado.
2. `npm/claudinio/README.md` chama `npx claudinio` (sem subcomando) como o atalho canônico e deixa `claudinio chat` documentado como forma explícita.
3. Sem diffs de código em `cli/`, `core/`, `src-tauri/`.

## Key Findings (Real Proof)

- **Default já implementado** — `cli/src/main.rs:88`: `match cli.command.unwrap_or(Command::Chat { path: None })` (lido verbatim). Subcommand é `Option<Command>`, `Option` é desfeito pelo `unwrap_or`.
- **Plano antigo já cobre o design** — `docs/plans/2026-07-20_claudinio-default-chat.md` tem Solution Design + Low-Level Design consistentes com o código atual. Reaproveitar.
- **Branch atual** — `feat/cli-tui` (git status confirmado). Working tree com modificações não relacionadas ao escopo deste pedido, mas nenhuma conflita.
- **Único ponto de divergência com a verdade** — `npm/claudinio/README.md:14` ainda não documenta o atalho sem subcomando; esse é o entregável.

## Authoritative Inputs

- `cli/src/main.rs:88` — fonte canônica do comportamento default.
- `npm/claudinio/README.md:14` — alvo da mudança de documentação.
- Decisão do usuário ("Só documentar e nada mudar") registrada nesta sessão.

## Changes (Steps)

### 1. Atualizar `npm/claudinio/README.md` — bloco "Uso"

- **Target:** `npm/claudinio/README.md` linhas 8–16 (bloco `## Uso`).
- **Mutation:** colocar `npx claudinio` (sem subcomando) como primeiro exemplo com label `# TUI interativa (default)`; manter `npx claudinio chat` como forma explícita logo abaixo.
- **Why:** usuário pediu que rodar `claudinio` sem subcomando abra o chat. README é onde o usuário lê isso.
- **Constraints:** copy literal preservada (não-traduzir termos técnicos que já estão em inglês). Nenhuma alteração em outros blocos.
- **Wiring sketch:** nenhum código; apenas Markdown. Sem hook, sem import, sem teste.
- **Não tocar:** comportamento do `cli` (decidido pelo usuário: nada mudar).

### 2. Marcar o plano como satisfeito

- **Target:** `docs/plans/2026-07-20_claudinio-default-chat.md`.
- **Mutation:** adicionar nota curta ao topo "Implementação verificada em `cli/src/main.rs:88` (commit `bf00d97`); único follow-up é README (verificado após este patch)."
- **Why:** rastreabilidade — quem ler o plano depois sabe que já foi entregue.
- **Constraints:** uma nota curta, sem refazer o plano.

## Verification Plan

- `cd /Users/victortavernari/claudinio_code && git diff -- npm/claudinio/README.md docs/plans/2026-07-20_claudinio-default-chat.md` → mostra exatamente os 2 arquivos de doc, nenhum de código.
- `git diff --stat -- cli core src-tauri src` → vazio (nenhum diff de código).
- Inspeção visual do trecho do README modificado: a linha `npx claudinio chat        # TUI interativa` passa a ter antes uma linha tipo `npx claudinio               # TUI interativa (default)`.
- `cat cli/src/main.rs | grep -n "unwrap_or(Command::Chat"` → confirma que o default permanece (read-only check).

## Risks

- **Muito baixo.** Apenas Markdown, sem build, sem runtime.
- Risco zero de quebrar o default já existente (não estamos tocando o código).

## Non-goals

- Não alterar `cli/src/main.rs` nem nenhum Rust.
- Não adicionar `--path` global (decidido pelo usuário no plano original e mantido).
- Não traduzir a doc para outras línguas.
- Não tocar em outros planos em `docs/plans/`.

## Low-Level Design

**Arquivo 1 — `npm/claudinio/README.md` (única mudança visível ao usuário):**

Estado atual (linhas 8–16):

```md
## Uso

\`\`\`bash
npx claudinio auth login
npx claudinio index .
npx claudinio search "hybrid retrieval"
npx claudinio run -m brain "explique o módulo X"
npx claudinio chat        # TUI interativa
\`\`\`
```

Estado alvo (mesmo bloco, `claudinio` puro promovido para primeiro exemplo):

```md
## Uso

\`\`\`bash
npx claudinio             # TUI interativa (default)
npx claudinio auth login
npx claudinio index .
npx claudinio search "hybrid retrieval"
npx claudinio run -m brain "explique o módulo X"
npx claudinio chat        # TUI interativa (forma explícita)
\`\`\`
```

Sem links, sem imagens, sem novos blocos. Mantém o resto do README intacto.

**Arquivo 2 — `docs/plans/2026-07-20_claudinio-default-chat.md` (apenas anotação):**

Acrescentar uma linha curta no topo (acima de `## Context`) do tipo `> Implementado em \`bf00d97\` (ver `cli/src/main.rs:88\`). Follow-up: documentação no README do npm.`.

**Hook no runtime:** nenhum. Não há código a tocar. O default na CLI já está em `cli/src/main.rs:88` (`unwrap_or(Command::Chat { path: None })`) e é o ground-truth.

## Tasks summary

1. Atualizar `npm/claudinio/README.md` (bloco `## Uso`) para promover `npx claudinio` sem subcomando como atalho default da TUI.
2. Anotar `docs/plans/2026-07-20_claudinio-default-chat.md` como implementado.
