# Adicionar Temas Pré-configurados

## Contexto

O Claudinio Code atualmente possui 3 temas (dark, light, sepia) gerenciados via CSS custom properties (oklch) + SolidJS signals. A seleção é feita por um `<select>` simples no modal de configurações.

O usuário solicitou mais 10 temas pré-configurados (com variantes dark/light): Dracula, Nord, Solarized, Monokai, One Dark, Catppuccin, Tokyo Night, Gruvbox, Rose Pine, Everforest — totalizando até **16 temas** (incluindo "system" e os 3 existentes).

**Decisões do usuário já confirmadas:**
- Cada variante é uma opção separada no grid (e.g. "Solarized Dark", "Solarized Light")
- Grid visual com cards de pré-visualização, 3 colunas
- "System" continua existindo como opção

## Solution Design

### 1. Expansão do sistema de temas (`src/lib/theme.ts`)

**Tipos:**
```typescript
export type ThemePreference = "system" | ThemeId;
export type ResolvedTheme = ThemeId;
export type ThemeId = 
  | "claudinio" | "claudinio-light" | "claudinio-sepia"
  | "dracula" | "nord" | "solarized-dark" | "solarized-light"
  | "monokai" | "one-dark" | "catppuccin" | "tokyo-night"
  | "gruvbox-dark" | "gruvbox-light" | "rose-pine" | "everforest";
```

**Metadata map** — cada tema tem:
- `labelKey`: chave de tradução (e.g. `"theme.dracula"`)
- `category`: `"dark" | "light"` — para grouping
- `previewColors`: array de 4-5 cores hex/oklch para o card de preview
- `isSystemDefault`: booleano para fallback do system

**Resolução de "system":**
- OS dark → `"claudinio"` (default dark)
- OS light → `"claudinio-light"` (default light)
- Backward compatible com o comportamento atual

### 2. CSS variables — 12 novos blocos `[data-theme="..."]` em `src/App.css`

Cada novo tema terá um bloco CSS completo com todas as variáveis do design system, usando `oklch`:

| Tema | Caráter | Hue base |
|------|---------|----------|
| dracula | Dark roxo-rosado | ~325 |
| nord | Dark azul-acinzentado | ~220 |
| solarized-dark | Dark marrom-esverdeado | ~45 |
| solarized-light | Light marrom-esverdeado | ~45 |
| monokai | Dark amarelo-rosado | ~50 |
| one-dark | Dark azul-ardósia | ~230 |
| catppuccin | Dark rosado-suave | ~350 |
| tokyo-night | Dark azul-noturno | ~240 |
| gruvbox-dark | Dark âmbar-escuro | ~40 |
| gruvbox-light | Light âmbar | ~40 |
| rose-pine | Dark rosado-pinho | ~340 |
| everforest | Dark verde-musgo | ~160 |

Cada bloco define: `surface-0/1/2/3`, `border-subtle/strong`, `ink/subtle/muted/faint`, `accent/strong/hover/ink/subtle/glow`, `success/warning/danger` (com subtis), `shadow-*`, `glow-accent`, `card-highlight`, `color-scheme`.

### 3. Monaco editor themes (`src/lib/monacoThemes.ts`)

Adicionar 12 novos temas Monaco (nomes: `claudinio-dracula`, `claudinio-nord`, etc.) com cores extraídas dos esquemas originais: background, foreground, lineHighlight, selection, cursor, gutter, diff, scrollbar.

### 4. Theme selector UI — Grid visual de cards

Substituir o `<select>` por uma grade de cards de tema:

**Layout:** 3 colunas, grid responsivo, dentro do modal de configurações
**Cada card:**
- Borda sutil, cantos arredondados
- Preview visual: ~5 bolinhas coloridas dispostas horizontalmente (surface mais escura, surface mais clara, ink, accent, success)
- Nome do tema (traduzido via i18n)
- Estado "selecionado": borda acentuada + ícone de check
- Estado "ativo atualmente": indicador sutil (mesmo que o modal esteja fechado, mostra qual está em uso)

**Card "System":**
- Primeiro card, sempre visível
- Preview mostrando gradiente dark→light (ou ícone de monitor)
- Label "Sistema" / "System"
- Badge mostrando o tema atualmente resolvido (e.g. "→ Claudinio")

**Implementação:** Componente inline no App.tsx ou extraído para `src/components/ThemePicker.tsx`.

### 5. i18n — Novas chaves de tradução

**en-US.ts:**
- `"theme.dracula": "Dracula"`, `"theme.nord": "Nord"`, etc.

**pt-BR.ts:**
- `"theme.dracula": "Drácula"`, `"theme.nord": "Nord"`, etc.
- Nomes próprios/nomes de marca mantêm-se em inglês ou traduzidos conforme contexto.

### 6. Atualização dos consumidores

- `DiffViewer.tsx` — já reativo via `createEffect` ✅ (só precisa do mapeamento correto `themeId → monacoThemeName`)
- `ContentViewerModal.tsx` — já reativo ✅
- `FileEditorModal.tsx` — NÃO é reativo (tema definido só na inicialização). Será atualizado para reagir ao `theme()` via `createEffect`.

### 7. Testes

- `theme.test.ts` — expandir para testar novas preferências, resolução e persistência
- `monacoThemes.test.ts` — verificar definição de todos os temas
- `FileEditorModal.test.tsx` — atualizar mocks de tema

## Riscos

1. **Tamanho do App.css:** 12 novos blocos de ~20 variáveis cada = ~240 linhas adicionais. Gerenciável. Se ficar grande demais, extrair para `src/themes.css`.
2. **Definição de cores oklch:** Precisa de cuidado para manter contraste AA em cada tema (especialmente os claros). Todos os valores serão calibrados manualmente para garantir legibilidade.
3. **Monaco syntax highlighting:** Com `base: "vs-dark"` ou `"vs"` + `inherit: true`, o Monaco já herda a coloração sintática correta. Temas com `base` diferente do que o resolved theme espera podem precisar de override de regras de tokenização.
4. **Card preview:** As cores de preview mostradas no card precisam ser hardcoded (não dá para computar do CSS dinamicamente sem renderizar o tema). Manteremos um array de 4-5 cores representativas por tema.

## Tasks (sumário)

1. **Expandir types e metadata em `theme.ts`** — ThemePreference, ResolvedTheme, ThemeId, themeMetadata map, resolver para "system"
2. **Gerar CSS variables** — 12 novos blocos `[data-theme="..."]` no App.css
3. **Adicionar Monaco themes** — 12 novos temas em `monacoThemes.ts`
4. **Criar ThemePicker grid component** — Substituir `<select>` por grid de cards visuais
5. **Atualizar i18n** — en-US.ts e pt-BR.ts com nomes dos novos temas
6. **Corrigir FileEditorModal** — Adicionar `createEffect` para tema reativo
7. **Atualizar testes** — theme.test.ts, monacoThemes.test.ts, mocks nos componentes


## Implementation Log — 2026-07-14 10:57
**Summary:** Adicionados 12 novos temas pré-configurados (Dracula, Nord, Solarized, Monokai, One Dark, Catppuccin, Tokyo Night, Gruvbox, Rose Pine, Everforest) com seletor visual grid, CSS oklch, Monaco themes, i18n, e 645/645 testes passando.
**Changed files:** A	docs/plans/2026-07-14_adicionar-temas.md, A	docs/plans/2026-07-14_thinking-bar-refactor.md, M	src-tauri/src/agent/session.rs, M	src-tauri/src/agent/tools/bash.rs, M	src-tauri/src/agent/tools/finalize_plan.rs, M	src-tauri/src/agent/tools/mod.rs, M	src-tauri/src/agent/tools/tasks.rs, M	src-tauri/src/agent/tools/write_plan.rs, M	src-tauri/src/commands/agent.rs, M	src/App.css, M	src/components/ChatPanel.tsx, M	src/test-setup.ts
**Commits:** 88029b4 feat: add mandatory Low-Level Design step to Brain mode workflow, 4472016 feat: add theme support and thinking bar refactor documentation
**Journal:** ## Key decisions and gotchas

1. **Legacy theme migration**: Os valores antigos `"dark"`, `"light"`, `"sepia"` armazenados no localStorage são migrados automaticamente para `"claudinio"`, `"claudinio-light"`, `"claudinio-sepia"`. Isso garante que usuários existentes não percam a preferência de tema.

2. **getMonacoTheme bug fix critical**: A função `getMonacoTheme` precisa tratar os temas já prefixados com `"claudinio-"` (ex: `claudinio-light`, `claudinio-sepia`) para não gerar duplo prefixo como `claudinio-claudinio-light`. A implementação verifica `t.startsWith("claudinio-")` primeiro.

3. **Signal vs valor**: O ThemePicker usa `preference()` e `resolvedTheme()` diretamente no JSX (chamando como função) em vez de capturar numa variável. Isso porque `preference` e `resolvedTheme` exportados de `theme.ts` já são funções getter (não signals diretos), então precisam ser invocados com `()`.

4. **Preview swatches**: As previewColors no themeMetadata foram calibradas manualmente com valores oklch representativos de cada tema. O card "System" usa um gradiente metade dark/metade light para representar que ele delega ao OS.

5. **4 colunas no grid**: O usuário pediu grid de 4 cards por linha (mudou de 3 para 4 durante a implementação).

6. **Mock de getMonacoTheme nos testes**: Todos os 3 arquivos de teste que mockam monacoThemes (DiffViewer, ContentViewerModal, FileEditorModal) precisaram da mesma adaptação — tratar `claudinio-` prefixados e `claudinio` (default dark) separadamente.

## Files changed
- src/lib/theme.ts — types, metadata, legacy migration, resolvePreference
- src/App.css — 12 novos blocos [data-theme="..."]
- src/lib/monacoThemes.ts — 12 temas Monaco + getMonacoTheme utility
- src/lib/locales/en-US.ts — 15 novas chaves
- src/lib/locales/pt-BR.ts — 15 novas chaves com traduções
- src/components/ThemePicker.tsx — novo componente de grid visual
- src/App.tsx — ThemePicker integrado no modal de settings
- src/components/FileEditorModal.tsx — createEffect reativo + getMonacoTheme
- src/components/DiffViewer.tsx — mapping atualizado para getMonacoTheme
- src/components/ContentViewerModal.tsx — mapping atualizado para getMonacoTheme
- src/lib/theme.test.ts — 21 testes (legacy migration + novos ThemeIds + metadata)
- src/lib/monacoThemes.test.ts — 5 testes (15 temas + getMonacoTheme)
- src/components/ThemePicker.test.tsx — 6 testes (render + click)
- src/components/FileEditorModal.test.tsx — mock atualizado
- src/components/DiffViewer.test.tsx — mock atualizado
- src/components/ContentViewerModal.test.tsx — mock atualizado

**Task journal:**
- Expandir types e metadata em theme.ts: Types expandidos com ThemeId (15 variantes), themeMetadata com previewColors, legacy migration
- Adicionar CSS variables para os 12 novos temas em App.css: 12 novos blocos adicionados
- Adicionar Monaco editor themes em monacoThemes.ts: 15 temas total, getMonacoTheme() exportado
- Adicionar i18n keys para nomes dos temas: 15 novas keys cada
- Criar ThemePicker com grid visual de cards: ThemePicker com System card + 15 theme cards
- Integrar ThemePicker no modal de configurações: select substituído
- Tornar componentes Monaco reativos: getMonacoTheme() compartilhado
- Atualizar testes: 645/645 tests passing
