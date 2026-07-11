# Solution Design: Theme Selector — 4 Temas (System, Dark, Light, Sepia)

## Context / Problem Statement

### Current State
- O projeto tem 2 temas visuais: **Dark** (default) e **Light**, definidos via CSS custom properties em `src/App.css`.
- A seleção de tema é **exclusivamente automática**: segue `prefers-color-scheme` do OS (`src/lib/theme.ts`). Não há override do usuário.
- O settings modal (`App.tsx`) tem seletor de idioma, API key, modelos, etc., mas **não tem seletor de tema** NEM toggle rápido.
- O `index.html` tem um script anti-flash que seta `data-theme` baseado apenas no OS.
- **2 bugs relacionados a tema**: `FileEditorModal` sempre usa `claudinio-dark` (hardcoded) e `ContentViewerModal` usa `classList.contains("dark")` em vez de `dataset.theme`.
- Monaco Editor tem temas `claudinio-dark` e `claudinio-light` em `src/lib/monacoThemes.ts`.

### O que o usuário CONFIRMOU (via entrevista)
1. Manter Dark + Light existentes, **adicionar Sepia/Warm** como terceiro tema.
2. Seletor com **4 opções**: System (default, segue OS), Dark, Light, Sepia.
3. UI: **toggle rápido no header** que cicla entre as 4 opções ao clicar (System → Dark → Light → Sepia → System...).
4. Opção "System" alterna automaticamente entre Dark e Light conforme o OS.

## Goal (Definition of Done)

Um toggle no header principal permite ao usuário escolher entre 4 temas: System (default), Dark, Light, e Sepia. A escolha persiste em `localStorage`. O tema "System" segue o OS dinamicamente. O tema "Sepia" aplica uma paleta quente (tons de papel envelhecido). O `index.html` respeita a escolha salva para evitar flash. Os bugs do `FileEditorModal` e `ContentViewerModal` são corrigidos para reagir ao tema corretamente.

## Key Findings (Prova Real)

| # | Finding | Method | Proof |
|---|---------|--------|-------|
| 1 | Tema é 100% OS-driven, sem persistência do usuário | `src/lib/theme.ts:1-14` — `createThemeSignal()` lê `window.matchMedia("(prefers-color-scheme: light)")`, sem `localStorage` | Arquivo lido; `localStorage` não é referenciado |
| 2 | CSS vars para dark e light em `App.css:5-48` via `[data-theme="dark"]` e `[data-theme="light"]` | `read_file src/App.css` | Seletores `[data-theme="dark"]` e `[data-theme="light"]` contêm todas as tokens |
| 3 | `FileEditorModal.tsx:112`: tema hardcoded `"claudinio-dark"` — ignora tema atual | `grep "claudinio-dark" src/components/FileEditorModal.tsx` | Linha: `theme: "claudinio-dark"` |
| 4 | `ContentViewerModal.tsx:68-75`: usa `classList.contains("dark")` mas projeto usa `dataset.theme` | `read_file` do componente | Checa `document.documentElement.classList.contains("dark")` — nunca será true |
| 5 | `index.html:2-11`: script anti-flash só consulta `prefers-color-scheme`, ignora preferência salva | `read_file index.html` | Script inline sem acesso a `localStorage` |
| 6 | Sistema de ícones é pixel art customizado via `Icon.tsx` com paths SVG inline | `read_file src/components/Icon.tsx` | `PATHS` record com paths SVG manuais |
| 7 | i18n (`grill-me.ts`) persiste idioma via `localStorage` (`claudinio_locale`) — padrão a seguir | `read_file src/lib/grill-me.ts` | `localStorage.getItem/setItem("claudinio_locale", ...)` |
| 8 | Header em `App.tsx:438-458` — botão settings é o único ícone no header direito | `read_file App.tsx:435-460` | `<Icon name="settings" />` ao lado do path do workspace |
| 9 | Monaco themes em `monacoThemes.ts` — `claudinio-dark` e `claudinio-light`, com guard `let defined` | `read_file src/lib/monacoThemes.ts` | Dois temas definidos, idempotentes |

## Authoritative Inputs

| Input | Source | Value |
|-------|--------|-------|
| 4 opções de tema | Usuário (confirmado) | `"system"`, `"dark"`, `"light"`, `"sepia"` |
| Comportamento do toggle | Usuário (confirmado) | Ciclar na ordem: System → Dark → Light → Sepia → System |
| Estilo do 3º tema | Usuário (confirmado) | Sepia/Warm — tons quentes, papel envelhecido |
| Tema default | Usuário (confirmado) | System (segue OS) |
| Ícones necessários | Derivado do design | Sol (light), Lua (dark), Monitor (system), Livro/Café (sepia) — pixel art style |
| `localStorage` key | Convenção do projeto (`claudinio_` prefix) | `claudinio_theme` |
| Paleta Sepia | Design decision (a confirmar visualmente) | Ver tabela abaixo |

### Paleta Sepia (design decision)

| Token | Valor | Descrição |
|-------|-------|-----------|
| `--surface-0` | `#f4ecd8` | Fundo base — papel quente |
| `--surface-1` | `#efe6cc` | Fundo de cards |
| `--surface-2` | `#e8ddbc` | Fundo de hover |
| `--surface-3` | `#dfd2a8` | Fundo de pressed |
| `--border-subtle` | `#d4c594` | Bordas sutis |
| `--border-strong` | `#b8a56c` | Bordas fortes |
| `--ink` | `#4a3728` | Texto principal — marrom escuro |
| `--ink-muted` | `#8a7560` | Texto secundário |
| `--ink-faint` | `#b8a58c` | Texto terciário |
| `--accent` | `#c77d3a` | Cor de destaque — âmbar |
| `--accent-hover` | `#d98d4a` | Destaque hover |
| `--accent-ink` | `#ffffff` | Texto sobre destaque |
| `--success` | `#6b8c42` | Verde suave |
| `--danger` | `#c2553a` | Vermelho terracota |
| `color-scheme` | `light` | Força scrollbars claras |

## Changes (Steps)

### Step 1 — Adicionar ícones de tema ao `Icon.tsx`
- **Target:** `src/components/Icon.tsx`
- **Mutation:** Adicionar 4 novos ícones pixel art no record `PATHS`: `"sun"`, `"moon"`, `"monitor"`, `"book-open"`. Adicionar nomes ao type `IconName`.
- **Why:** O toggle no header precisa de ícones para representar cada tema atual.
- **Constraints:** Seguir o estilo pixel art dos ícones existentes (paths SVG preenchidos, viewBox 24×24). Cada ícone deve ser reconhecível em 24×24.

### Step 2 — Adicionar strings i18n
- **Target:** `src/lib/locales/pt-BR.ts` e `src/lib/locales/en-US.ts`
- **Mutation:** Adicionar chaves `"theme.system"`, `"theme.dark"`, `"theme.light"`, `"theme.sepia"` com os labels traduzidos.
- **Why:** O tooltip do toggle precisa mostrar o nome do tema no idioma atual.
- **Constraints:** Seguir o formato existente do dict.

### Step 3 — Refatorar `theme.ts` para suportar preferência + localStorage
- **Target:** `src/lib/theme.ts`
- **Mutation:** Reescrever `createThemeSignal()`:
  - Ler `localStorage.getItem("claudinio_theme")` como `ThemePreference = "system" | "dark" | "light" | "sepia"`.
  - Default `"system"` se não setado.
  - Se preference é `"system"`, seguir `prefers-color-scheme` (dark/light).
  - Se preference é `"dark"`, `"light"`, ou `"sepia"`, usar esse valor diretamente.
  - Expor: `themePreference` (signal com a preferência), `resolvedTheme` (signal com dark/light/sepia), `setThemePreference` (função que salva em localStorage + atualiza signals), `cycleTheme` (função que cicla).
  - `resolvedTheme` deve setar `document.documentElement.dataset.theme`.
- **Why:** O comportamento atual só segue OS; precisamos de preferência do usuário com persistência.
- **Constraints:** Manter retrocompatibilidade com código que importa `theme` (reativo). O `DiffViewer.tsx` usa `theme()` — deve continuar funcionando.

### Step 4 — Adicionar CSS do tema Sepia
- **Target:** `src/App.css`
- **Mutation:** Adicionar bloco `[data-theme="sepia"]` com todas as CSS custom properties da paleta Sepia (após o bloco `[data-theme="light"]`). Adicionar tokens Sepia ao bloco `@theme inline`.
- **Why:** Sem CSS, o tema Sepia não tem efeito visual.
- **Constraints:** Espelhar exatamente a estrutura dos blocos dark/light.

### Step 5 — Adicionar tema Sepia ao Monaco
- **Target:** `src/lib/monacoThemes.ts`
- **Mutation:** Adicionar `"claudinio-sepia"` baseado em `vs` (light), com cores da paleta Sepia.
- **Why:** Monaco Editor precisa de um tema separado para o fundo e cores de sintaxe no modo Sepia.
- **Constraints:** Seguir o mesmo padrão de `claudinio-dark` e `claudinio-light`.

### Step 6 — Atualizar `index.html` anti-flash script
- **Target:** `index.html`
- **Mutation:** Modificar o script inline para ler `localStorage.getItem("claudinio_theme")` e resolver o `data-theme` correto antes do primeiro paint.
- **Why:** Evita flash de tema errado ao carregar a página.
- **Constraints:** Deve ser síncrono e inline (sem imports). Se a preferência for `"system"`, manter o comportamento atual de `matchMedia`.

### Step 7 — Adicionar toggle de tema no header de `App.tsx`
- **Target:** `src/App.tsx`
- **Mutation:** 
  - Importar `themePreference`, `resolvedTheme`, `cycleTheme` de `./lib/theme`.
  - Adicionar botão de toggle no header (entre o path do workspace e o botão settings) que chama `cycleTheme()`.
  - O ícone do botão muda conforme `resolvedTheme()`: sun para light, moon para dark, monitor para system, book-open para sepia.
  - Tooltip mostra nome do tema atual via `t()`.
- **Why:** Interface para o usuário trocar de tema.
- **Constraints:** Manter o layout do header intacto. Botão deve ter o mesmo estilo visual do botão settings existente.

### Step 8 — Corrigir `FileEditorModal` para usar tema resolvido
- **Target:** `src/components/FileEditorModal.tsx`
- **Mutation:** Substituir `theme: "claudinio-dark"` hardcoded por valor dinâmico baseado em `resolvedTheme()`.
- **Why:** Bug existente — editor sempre usa tema dark.
- **Constraints:** Mapear: dark → `claudinio-dark`, light → `claudinio-light`, sepia → `claudinio-sepia`.

### Step 9 — Corrigir `ContentViewerModal` para usar dataset.theme
- **Target:** `src/components/ContentViewerModal.tsx`
- **Mutation:** Substituir `document.documentElement.classList.contains("dark")` por `document.documentElement.dataset.theme === "dark"` e adicionar suporte a sepia.
- **Why:** Bug existente — usa API errada (`classList` em vez de `dataset.theme`).
- **Constraints:** Mapear: dark → `claudinio-dark`, light → `claudinio-light`, sepia → `claudinio-sepia`.

### Step 10 — Atualizar `DiffViewer` para suportar tema Sepia
- **Target:** `src/components/DiffViewer.tsx`
- **Mutation:** Adicionar `claudinio-sepia` ao mapeamento de temas no `createEffect` que chama `monaco.editor.setTheme()`.
- **Why:** DiffViewer já reage a `theme()`, mas só mapeia dark/light.
- **Constraints:** dark → `claudinio-dark`, light → `claudinio-light`, sepia → `claudinio-sepia`.

### Step 11 — Atualizar testes
- **Target:** `src/lib/theme.test.ts`, `src/lib/monacoThemes.test.ts`
- **Mutation:** Atualizar testes existentes para cobrir o novo comportamento (preferência, ciclo, localStorage, sepia).
- **Why:** Garantir que as mudanças não quebram nada e são testadas.
- **Constraints:** Manter o padrão de testes existente (vitest + jsdom).

## O que NÃO muda

- Estrutura do settings modal — sem adição de seletor de tema lá (usuário escolheu apenas toggle no header).
- Tokens CSS de dark e light — permanecem idênticos.
- Comportamento de `prefers-color-scheme` para o modo System — permanece idêntico.
- Outros componentes que usam classes Tailwind — funcionam automaticamente via CSS vars.

## Verification Plan

### Verificações automatizadas
1. **Testes unitários**: Rodar `pnpm vitest run` — todos os testes devem passar, incluindo os novos/atualizados.
2. **Build**: Rodar `pnpm build` — sem erros de TypeScript ou Vite.

### Verificações visuais
3. **Renderizar cada tema em 1280×800**: Screenshot do app com cada um dos 4 temas aplicados.
4. **Verificar tema Sepia**: Confirmar que tons são quentes, texto legível, contraste adequado.
5. **Verificar toggle no header**: Confirmar que clicar cicla e o ícone muda.
6. **Verificar flash prevention**: Recarregar com tema Sepia salvo — confirmar que não há flash de dark.
7. **Verificar Monaco Editor**: Abrir FileEditorModal em cada tema — confirmar que o editor segue o tema.
8. **Verificar DiffViewer**: Abrir diff em cada tema — confirmar que segue o tema.

### Verificações de regressão
9. **System mode**: Com preferência "system", alternar OS dark/light — confirmar que o app segue.
10. **localStorage**: Mudar tema, recarregar — confirmar que a escolha persiste.

## Tasks Summary

| # | Task | Arquivos |
|---|------|----------|
| 1 | Adicionar ícones de tema (sun, moon, monitor, book-open) | `Icon.tsx` |
| 2 | Adicionar strings i18n para temas | `pt-BR.ts`, `en-US.ts` |
| 3 | Refatorar `theme.ts` com preferência + localStorage + ciclo | `theme.ts` |
| 4 | Adicionar CSS do tema Sepia | `App.css` |
| 5 | Adicionar tema Sepia ao Monaco (`claudinio-sepia`) | `monacoThemes.ts` |
| 6 | Atualizar `index.html` anti-flash script | `index.html` |
| 7 | Adicionar toggle de tema no header | `App.tsx` |
| 8 | Corrigir `FileEditorModal` (hardcoded dark) | `FileEditorModal.tsx` |
| 9 | Corrigir `ContentViewerModal` (classList vs dataset) | `ContentViewerModal.tsx` |
| 10 | Atualizar `DiffViewer` para suporte Sepia | `DiffViewer.tsx` |
| 11 | Atualizar testes | `theme.test.ts`, `monacoThemes.test.ts` |


## Implementation Log — 2026-07-11 16:57
**Summary:** Add 4-theme selector (System, Dark, Light, Sepia) with header toggle and localStorage persistence
**Changed files:** M index.html, M src/App.css, M src/App.tsx, M src/components/ContentViewerModal.tsx, M src/components/DiffViewer.tsx, M src/components/FileEditorModal.tsx, M src/components/Icon.tsx, M src/lib/locales/en-US.ts, M src/lib/locales/pt-BR.ts, M src/lib/monacoThemes.test.ts, M src/lib/monacoThemes.ts, M src/lib/theme.test.ts, M src/lib/theme.ts, ?? .commandcode/, ?? docs/plans/2026-07-11_theme-selector-4-temas.md
**Commits:** _(git unavailable or none)_
**Journal:** Key findings and decisions from implementation:

1. **theme.ts architecture**: Followed the grill-me.ts pattern (lazy init via createRoot + plain exported functions). The `theme()` function is backward-compatible as a signal-like accessor. Exported `preference()`, `resolvedTheme()`, `setThemePreference()`, `cycleTheme()` as the new public API. Added `__resetState()` for testing.

2. **Interop with existing code**: DiffViewer.tsx already used `import { theme } from "../lib/theme"` and called `theme()` inside createEffect. The new `theme()` function is a plain function that internally calls `initState().resolvedTheme()`, and SolidJS tracks the inner createMemo through function call — so reactive behavior is preserved without changes to consumers.

3. **Bug fixes discovered**: FileEditorModal.tsx had hardcoded `"claudinio-dark"`. ContentViewerModal.tsx used `classList.contains("dark")` instead of `dataset.theme`. Both were fixed as part of this change.

4. **CSS approach**: Tailwind v4's `@theme inline {}` block already maps tokens via var() references (e.g. `--color-surface-0: var(--surface-0)`), so adding the `[data-theme="sepia"]` block is sufficient — all Tailwind classes like `bg-surface-0`, `text-ink`, `border-accent` automatically resolve dynamically. No @theme changes were needed.

5. **Testing**: The persistent nature of createRoot required adding __resetState() to clear the state cache between tests. The matchMedia spy test was removed since the end-to-end change-event test proves the listener works.

6. **Sepia palette**: Warm earth tones based on #f4ecd8 background (paper-like) with #4a3728 text (dark brown), accent #c77d3a (amber/orange). Feels like reading an old book.

7. **Monaco sepia theme**: Defined with base 'vs', with background matching the CSS sepia surface-0. Colors are warm-toned but ensure contrast for code readability.

**Task journal:**
- Add theme icons (sun, moon, monitor, book-open) to Icon.tsx: Added 4 new pixel-art SVG icons to PATHS: sun (circle+8 rays), moon (crescent with stair-step pattern), monitor (screen with stand), book-open (open book with text lines). Auto-typed via keyof typeof PATHS.
- Add theme i18n strings to locale files: Added theme keys under // ── Theme section in both locale files.
- Refactor theme.ts with preference, localStorage, and cycle: Rewrote theme.ts with preference/setThemePreference/cycleTheme/resolvedTheme/theme exports as functions (same pattern as grill-me.ts). Uses createMemo for resolved theme that follows prefers-color-scheme in system mode. Persists to localStorage key 'claudinio_theme'. Backward compatible — theme() still works.
- Add Sepia CSS custom properties and @theme tokens: Added [data-theme='sepia'] CSS block with warm palette. No @theme changes needed—existing Tailwind tokens use var() references dynamically.
- Add claudinio-sepia Monaco Editor theme: Added claudinio-sepia Monaco theme with sepia warm palette colors.
- Update index.html anti-flash script for theme preference: Updated index.html anti-flash script: reads claudinio_theme from localStorage (wrapped in try/catch), validates against valid themes, falls back to system dark/light detection.
- Add theme toggle button to App.tsx header: Added theme toggle button in App.tsx header between workspace path and settings gear. Uses preference() for the icon lookup and tooltip (maps system→monitor, dark→moon, light→sun, sepia→book-open).
- Fix FileEditorModal hardcoded dark theme: Imported { theme } from ../lib/theme. Replaced hardcoded 'claudinio-dark' with dynamic IIFE that maps resolved theme to Monaco theme name.
- Fix ContentViewerModal classList→dataset.theme bug: Replaced classList.contains('dark') with reactive theme() signal from ../lib/theme. Now correctly maps sepia→claudinio-sepia, dark→claudinio-dark, light→claudinio-light.
- Update DiffViewer for Sepia theme support: Updated both the initial createDiffEditor theme and the reactive setTheme effect with three-way conditional: dark→claudinio-dark, sepia→claudinio-sepia, fallback→claudinio-light.
- Update theme and Monaco theme tests: Updated theme.test.ts with 13 tests (old: 6, new: cycleTheme, preference, setThemePreference, localStorage persistence, sepia override, etc.). Updated monacoThemes.test.ts for 3 themes. 402/402 tests passing, build succeeds.
