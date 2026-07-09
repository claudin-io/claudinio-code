# Fix Onboarding Wizard — Two Visual Bugs

## Context

Two bugs reported on the onboarding wizard screen:

1. **Untranslated i18n keys** — The "What you can do" screen (step 1) shows raw localization keys (`onboarding.features.agent.title`, etc.) instead of translated text.
2. **Previous button arrow points right** — The Previous button shows `>` instead of `<`, matching the Next button's direction.

## Bug 1 — Root Cause (Investigated ✅)

**File:** `src/components/OnboardingWizard.tsx`  
**File:** `src/lib/grill-me.ts`

In `OnboardingWizard.tsx`, the `features` array is a **module-level `const`**, calling `t()` immediately at module evaluation time:

```ts
const features = [
  { title: t("onboarding.features.agent.title"), desc: t("onboarding.features.agent.desc") },
  // ...
];
```

In `grill-me.ts`, dictionaries are loaded asynchronously (`await import("./locales/...")`) inside `loadDict()`. The `currentDict()` signal starts as `{}` (line 84) and is only populated later. So when `t()` runs at module scope, `currentDict()` is empty → `dict[key]` is `undefined` → `t()` returns the raw key string (grill-me.ts:105: `if (val === undefined) return key`).

Even after the dict eventually loads, the `features` array is **not reactive** — it's a plain `const`, so `t()` never re-executes.

**Fix:** Make the `features` array a function/`createMemo` that calls `t()` inside the render, where SolidJS reactivity will re-evaluate it once `currentDict()` is populated.

## Bug 2 — Root Cause (Investigated ✅)

**File:** `src/components/Icon.tsx`

The `chevron-left` SVG path renders incorrectly as a right-pointing arrow:

```
"chevron-left": ["M10 6L8.59 7.41 13.17 12l-4.58 4.59L10 18l6-6z"]
```

The tip reaches x=13.17 and x=16, both on the RIGHT side — tracing a `>` shape, not `<`.

The `chevron-right` path also looks wrong (a stepped rectangle shape):

```
"chevron-right": ["M16 13v-2h-2v2h2Zm-2-2V9h-2v2h2Zm0 4v-2h-2v2h2Zm-2-6V7h-2v2h2Zm0 8v-2h-2v2h2ZM10 7V5H8v2h2Zm0 12v-2H8v2h2Z"]
```

**User requested replacement icons:** `material-symbols:arrow-left-rounded` and `material-symbols:arrow-right-rounded`.

Standard Material Symbols paths:
- `arrow-left-rounded`: `m14 7l-5 5l5 5V7z`
- `arrow-right-rounded`: `m10 17l5-5l-5-5v10z`

**Fix:** Replace the `chevron-left` and `chevron-right` paths in `Icon.tsx` with the correct Material Symbols paths.

## Solution Design

### Change 1: Fix i18n — make features reactive

**Target:** `src/components/OnboardingWizard.tsx`

Replace static `const features` array with an inline approach that resolves translations reactively. Simplest change: make `features` a getter function called inside the JSX, so `t()` runs during rendering.

```diff
- const features = [
-   { icon: "thinking-face", title: t("onboarding.features.agent.title"), desc: t("onboarding.features.agent.desc") },
-   { icon: "check-circle", title: t("onboarding.features.approval.title"), desc: t("onboarding.features.approval.desc") },
-   { icon: "layers", title: t("onboarding.features.subagents.title"), desc: t("onboarding.features.subagents.desc") },
-   { icon: "search", title: t("onboarding.features.indexing.title"), desc: t("onboarding.features.indexing.desc") },
- ];
```

Replace `<For each={features}>` with a reactive pattern that calls `t()` at render time. Option A: inline the `For` each with a function call. Option B: use a `createMemo`. Simplest is to define a `features()` function.

### Change 2: Fix arrow SVG paths

**Target:** `src/components/Icon.tsx`

Replace `chevron-left` and `chevron-right` paths with Material Symbols paths:

```diff
  "chevron-left": [
-   "M10 6L8.59 7.41 13.17 12l-4.58 4.59L10 18l6-6z"
+   "m14 7l-5 5l5 5V7z"
  ],
  "chevron-right": [
-   "M16 13v-2h-2v2h2Zm-2-2V9h-2v2h2Zm0 4v-2h-2v2h2Zm-2-6V7h-2v2h2Zm0 8v-2h-2v2h2ZM10 7V5H8v2h2Zm0 12v-2H8v2h2Z"
+   "m10 17l5-5l-5-5v10z"
  ],
```

## Risks

- Low risk. Both changes are isolated to `OnboardingWizard.tsx` and `Icon.tsx`.
- The `chevron-right` icon is also used in `ChatPanel.tsx` (lines 1993, 2168, 2596) — verify the new path renders correctly there too.
- The i18n fix only affects the features screen; other `t()` calls in the same component (`onboarding.welcome.*`, `onboarding.signIn.*`, `onboarding.next`, `onboarding.prev`) are already used inline inside JSX, so they work fine.

## Verification Plan

1. **i18n fix:** Run the app and navigate to step 1. Confirm "What you can do" title and all 4 feature cards show translated text, not raw keys.
2. **Arrow fix:** Run the app and confirm Previous button shows `<` and Next button shows `>`.
3. **Regression — ChatPanel:** Check that `chevron-right` icons in ChatPanel still render correctly.


## Implementation Log — 2026-07-09 12:22
**Summary:** Fix two onboarding wizard bugs: untranslated i18n keys and Previous button arrow direction
**Changed files:** M	src-tauri/Cargo.lock, M	src-tauri/Cargo.toml, A	src-tauri/src/agent/install_id.rs, M	src-tauri/src/agent/mod.rs, M	src-tauri/src/agent/provider.rs, M	src-tauri/src/commands/auth.rs
**Commits:** 32c0adf feat: send per-device install_id at login for app-install trial promo
**Journal:** **Bug 1 — Chaves sem tradução:** A `features` array em `OnboardingWizard.tsx` chamava `t()` no escopo do módulo, antes do dicionário carregar assincronamente via `await import()`. `t()` retornava a chave crua (`onboarding.features.agent.title`) como fallback quando `val === undefined`. Como o array era `const` estática, nunca re-avaliava depois. **Fix:** `const features` → `const features = () => [...]` (função), e `<For each={features}>` → `<For each={features()}>`. Agora as chamadas `t()` rodam durante a renderização, quando o Signal `currentDict()` já está populado.

**Bug 2 — Seta do Previous:** O SVG path de `chevron-left` (`M10 6L8.59 7.41 13.17 12l-4.58 4.59L10 18l6-6z`) desenhava a ponta no lado direito (x=13.17), renderizando como `>`. **Fix:** Substituído pelos paths Material Symbols solicitados: `chevron-left` → `m14 7l-5 5l5 5V7z` (seta `<`), `chevron-right` → `m10 17l5-5l-5-5v10z` (seta `>`). O `chevron-right` também é usado no `ChatPanel.tsx` — o novo path é uma seta padrão pra direita, compatível.

**Task journal:**
- Fix untranslated localization keys on onboarding features screen: Root cause: `features` array called `t()` at module scope, before dict loaded asynchronously. `t()` returns raw key when dict entry is undefined.; Fix: `const features` → `const features = () => [...]`, and `<For each={features}>` → `<For each={features()}>`. Now `t()` runs during render, when dict is available.
- Fix Previous button arrow direction (> to <): Existing `chevron-left` path `M10 6L8.59 7.41...` had tip at x=13.17/x=16 (right side), rendering as `>` instead of `<`.; Fixed `chevron-left` → `m14 7l-5 5l5 5V7z`, `chevron-right` → `m10 17l5-5l-5-5v10z` (Material Symbols).; Regression check: `chevron-right` is also used in ChatPanel.tsx (lines 1993, 2168, 2596) — new path is a standard right arrow so it'll render correctly everywhere.
