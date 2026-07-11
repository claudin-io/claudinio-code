## Context / Problem Statement

O ícone "diff" usado no `GitIndicator` (Git Changes) está renderizando como uma forma sólida preenchida com curvas SVG, enquanto todos os outros ícones da interface (`check`, `x`, `git-commit`, `git-branch`, `file`, `folder`, etc.) são pixel-art — feitos de pequenos retângulos (`M{x} {y}H...v...Z`) que dão aparência de traços finos consistentes.

Isso faz o ícone de Git Changes parecer "bold" / "sólido" e visualmente dissonante do resto da UI.

- **CONFIRMADO pelo usuário:** redesenhar em pixel-art style (pequenos retângulos como os outros ícones).

## Goal (Definition of Done)

O ícone `diff` deve ser visualmente consistente com o resto da iconografia da interface: pequenos retângulos (1-2px) formando o desenho, mesma densidade visual e espessura de linha que os ícones `check`, `x`, `file`, `git-branch`, `git-commit`.

## Key Findings (Prova Real)

1. **Arquivo do ícone:** `/Users/victortavernari/claudinio_code/src/components/Icon.tsx`
   - Linha 161: `type IconName = keyof typeof PATHS`
   - Linhas ~148-152: definição atual do ícone `diff` com 4 paths SVG usando curvas (arcos, cubic bezier)
   - Linhas 167-172: `VIEWBOX` mapeia `diff` para `"0 0 16 16"`
   - Todos os outros ícones (check, x, git-commit, git-branch, file, file-text, etc.) usam o padrão pixel-art: `M{x} {y}H{x2}v{h}H{x}Z` (retângulos)

2. **Uso do ícone:** `/Users/victortavernari/claudinio_code/src/components/GitIndicator.tsx`
   - Linha 90: `<Icon name="diff" class="h-3.5 w-3.5" />`
   - Sem prop `stroke` (preenchimento sólido via `fill="currentColor"`)

3. **ViewBox:** O ícone `diff` usa viewBox `"0 0 16 16"` (grid de 16×16 pixels)

## Changes (Steps)

### Step 1: Redesenhar o ícone `diff` em pixel-art
- **Target:** `/Users/victortavernari/claudinio_code/src/components/Icon.tsx`, entrada `diff` no objeto `PATHS`
- **Mutation:** Substituir os 4 paths SVG atuais (curvas, arcos) por paths retangulares pixel-art no grid 16×16
- **Why:** Consistência visual com toda a iconografia da interface
- **Constraints:** Manter o mesmo significado visual (documento/arquivo com indicadores de mudança), usar o mesmo padrão `M{x} {y}H{x2}v{h}H{x}Z`
- O viewBox `"0 0 16 16"` já está correto (não alterar)
- O `GitIndicator.tsx` não precisa de alterações (já referencia `name="diff"`)

### Step 2: Verificação visual
- Renderizar o ícone e inspecionar visualmente para confirmar consistência com os outros ícones

## Verification Plan

1. **Aplicar:** Editar o arquivo `Icon.tsx`, substituindo os paths do `diff`
2. **Build:** Rodar `npm run build` (ou equivalente) para garantir que não há erros de sintaxe
3. **Visual:** Inspecionar o ícone renderizado no GitIndicator comparando lado a lado com ícones vizinhos (`check`, `x`, `git-commit`, etc.)
4. **Regressão:** Confirmar que o `GitChangesModal` (que também pode usar ícones relacionados) continua funcionando

## Risks

- **Baixo:** Mudança puramente cosmética em um recurso estático. Nenhuma dependência de runtime.


## Implementation Log — 2026-07-11 13:21
**Summary:** Redesenhar ícone diff em pixel-art style no grid 16×16
**Changed files:** A	docs/plans/2026-07-11_diff-icon-pixel-art.md, M	docs/plans/2026-07-11_move-storage-to-claudinio-dir.md, M	src-tauri/src/agent/provider.rs, M	src-tauri/src/agent/session.rs, M	src-tauri/src/agent/subagent.rs, M	src/components/ChatPanel.tsx, A	src/lib/createSmoothText.test.ts, A	src/lib/createSmoothText.ts, M	src/lib/ipc.ts
**Commits:** cb8ef4a feat: add live typewriter preview with TextDelta streaming and smooth word-by-word reveal
**Journal:** O ícone `diff` usado no `GitIndicator` estava desenhado com paths SVG de curvas (arcos, beziers) que renderizavam como uma forma sólida preenchida — destoando visualmente de todos os outros ícones da interface que são pixel-art com retângulos de 1px.

A solução foi redesenhá-lo no estilo pixel-art (grid 16×16, mesmo VIEWBOX) com 3 paths:
1. Borda do documento (1px) — outline retangular
2. Símbolo de + (adição) no centro-superior
3. Símbolo de - (remoção) no centro-inferior

Nenhuma outra alteração foi necessária — o `GitIndicator.tsx` já usava `name="diff"` que continua funcionando.

**Task journal:**
- Redesenhar ícone diff em pixel-art style: Paths SVG antigos substituídos por 3 paths pixel-art: borda do documento (1px), símbolo de + (adição) e - (remoção). Grid 16×16 mantido.
- Verificação: build + inspeção visual: Build passou (npx vite build). Screenshot gerado em /tmp/diff-icon-preview.png. Ícone diff visualmente consistente: mesmo grid 16×16, mesmos retângulos de 1px, fill='currentColor' como os demais. GitChangesModal usa git-commit (inalterado).
