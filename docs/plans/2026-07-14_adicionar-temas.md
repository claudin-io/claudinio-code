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
