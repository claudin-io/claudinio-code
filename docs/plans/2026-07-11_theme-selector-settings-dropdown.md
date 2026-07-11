# Patch: Theme selector — header toggle → settings dropdown

## Context
O usuário não gostou do toggle cíclico no header. Quer um dropdown estilo "select" dentro do modal de Settings, igual ao seletor de idioma.

## Changes

### 1. `src/App.tsx` — 3 alterações

**a) Import line (line 10):**
- Antes: `import { theme, preference, cycleTheme } from "./lib/theme";`
- Depois: `import { preference, setThemePreference } from "./lib/theme";`

**b) Header (remover linhas 452-458):**
- Remover o botão `<button onClick={cycleTheme}...>...</button>` inteiro

**c) Settings modal — adicionar dropdown após seletor de idioma:**
```tsx
{/* Theme selector */}
<label class="mb-1 block text-xs text-ink-muted">{t("app.config.theme")}</label>
<select
  value={preference()}
  onChange={(e) => setThemePreference(e.currentTarget.value as any)}
  class="mb-4 w-full appearance-none rounded-md border border-border-subtle bg-surface-0 p-2 text-sm text-ink focus:border-accent focus:outline-none focus:ring-1 focus:ring-accent"
>
  <option value="system">🖥️ {t("theme.system")}</option>
  <option value="dark">🌙 {t("theme.dark")}</option>
  <option value="light">☀️ {t("theme.light")}</option>
  <option value="sepia">📖 {t("theme.sepia")}</option>
</select>
```

### 2. `src/lib/locales/pt-BR.ts`
- Adicionar: `"app.config.theme": "Tema"`

### 3. `src/lib/locales/en-US.ts`
- Adicionar: `"app.config.theme": "Theme"`

## Verification
- `pnpm vitest run` — todos os testes passando
- `pnpm build` — build bem-sucedido


## Implementation Log — 2026-07-11 18:21
**Summary:** Move theme selector from header toggle to Settings modal dropdown
**Changed files:** M index.html, M src/App.css, M src/App.tsx, M src/components/ContentViewerModal.tsx, M src/components/DiffViewer.tsx, M src/components/FileEditorModal.tsx, M src/components/Icon.tsx, M src/lib/locales/en-US.ts, M src/lib/locales/pt-BR.ts, M src/lib/monacoThemes.test.ts, M src/lib/monacoThemes.ts, M src/lib/theme.test.ts, M src/lib/theme.ts, ?? .commandcode/, ?? docs/plans/2026-07-11_theme-selector-4-temas.md, ?? docs/plans/2026-07-11_theme-selector-settings-dropdown.md
**Commits:** _(git unavailable or none)_
**Journal:** Removed the header toggle button and added a <select> dropdown inside the Settings modal, right after the language selector. Same visual style as the existing language dropdown (rounded border, bg-surface-0, focus ring). Options show emoji + label: 🖥️ Sistema, 🌙 Escuro, ☀️ Claro, 📖 Sépia. Uses setThemePreference() on change. Import updated from cycleTheme to setThemePreference.

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
