# Multi-Language Support

## Context

Claudinio Code currently supports only 2 locales: `pt-BR` (hardcoded default) and `en-US`. The sibling project `claudinio_litellm` supports 18 locales with a proper locale resolution system (query → cookie → Accept-Language → `en-US`). Claudinio Code needs parity: all 18 languages from the litellm project, system language detection at startup, English fallback for untranslated strings, and dynamic `<html lang>` attribute.

The current i18n system is home-grown in `src/lib/grill-me.ts`: a SolidJS signal backed by `localStorage`, lazy-loaded dicts via dynamic `import()`, and a `t()` function with positional interpolation. The `<select>` picker is hardcoded in `src/App.tsx` with exactly two `<option>` tags. The Rust backend has no locale awareness for its own UI (only for indexing user projects).

## Solution Design

### 1. All 18 Locales Supported

Add the 16 missing locales to the `LocaleId` type, `loadDict()` branches, `FLAGS`/`LOCALE_LABELS` maps, and the UI picker. Supported locales (aligned with claudinio_litellm):

| Code | Language | Flag | Label |
|------|----------|------|-------|
| `en-US` | English (US) | 🇺🇸 | EN |
| `pt-BR` | Portuguese (Brazil) | 🇧🇷 | PT |
| `pt-PT` | Portuguese (Portugal) | 🇵🇹 | PT |
| `es-ES` | Spanish | 🇪🇸 | ES |
| `fr-FR` | French | 🇫🇷 | FR |
| `de-DE` | German | 🇩🇪 | DE |
| `it-IT` | Italian | 🇮🇹 | IT |
| `ru-RU` | Russian | 🇷🇺 | RU |
| `tr-TR` | Turkish | 🇹🇷 | TR |
| `ar-SA` | Arabic | 🇸🇦 | العربية |
| `hi-IN` | Hindi | 🇮🇳 | हिन्दी |
| `bn-BD` | Bengali | 🇧🇩 | বাংলা |
| `ur-PK` | Urdu | 🇵🇰 | اردو |
| `zh-CN` | Chinese (Simplified) | 🇨🇳 | 中文 |
| `ja-JP` | Japanese | 🇯🇵 | 日本語 |
| `ko-KR` | Korean | 🇰🇷 | 한국어 |
| `vi-VN` | Vietnamese | 🇻🇳 | VI |
| `id-ID` | Indonesian | 🇮🇩 | ID |

### 2. New Translation Files (Empty Dicts with en-US Fallback)

16 new locale dict files at `src/lib/locales/{code}.ts`. Each exports an empty `LocaleDict` (`{}`). The `t()` function will fall back to `en-US` values when a key is missing from the active locale. The user will provide translations later.

### 3. en-US Fallback Chain in `t()`

When `t(key)` doesn't find the key in the current locale's dict, it checks the `en-US` dict (eagerly preloaded into the dict cache at module init), then falls back to the raw key string. This means even untranslated locales show English UI instead of raw dotted keys like `"app.config.title"`.

### 4. System Language Detection at Startup

The initial locale (when no `localStorage` value exists) is resolved via:
1. `navigator.language` (browser webview — reflects OS locale in Tauri)
2. Tauri `get_os_locale` command (Rust `sys-locale` crate as fallback)
3. Hardcoded `"en-US"` default

The resolved locale is matched against supported locales (exact match, then language-prefix match), mirroring `claudinio_litellm`'s `resolve_locale()` logic.

### 5. Dynamic `<html lang>` Attribute

A SolidJS `createEffect` syncs `document.documentElement.lang` to the current `locale()` signal, so `<html lang>` always matches the user's selected language (for accessibility, SEO of web views, and CSS `:lang()` selectors).

### 6. UI Picker — Dynamic Options

The `<select>` in the config modal is refactored to render `<option>` elements dynamically from the `LOCALE_LABELS` and `FLAGS` maps, instead of hardcoding two options. The label `"Idioma / Language"` is replaced with a proper `t()` key (`"app.config.language"`).

## Risks

- **Large PR surface**: ~20+ files touched (16 new locale files + 5-6 existing files). Mitigation: new locale files are trivial boilerplate; existing file changes are surgical.
- **`navigator.language` may return generic codes** (e.g. `"pt"` instead of `"pt-BR"`). Mitigation: match by language prefix as fallback (same as claudinio_litellm).
- **`sys-locale` crate adds a Rust dependency.** Mitigation: it's a tiny, widely-used crate with no transitive dependencies of concern.
- **RTL locales (ar-SA, ur-PK)**: Structure supports it but no RTL CSS is part of this plan. Fonts may not render Arabic/Hindi/Bengali/Urdu correctly without additional CSS. Mitigation: non-goal for now — the locale infrastructure is the deliverable; RTL styling is a separate task.

## Non-goals

- Actual translations for the 16 new locales (user provides later)
- RTL CSS/layout support
- Number/date formatting via `Intl.*` APIs
- Extraction tooling for missing/unused keys
- Rust backend i18n (Tauri-side strings remain English)
- `Accept-Language` header parsing (not relevant for a desktop app)
- Cookie/query-param locale resolution (web-only patterns from litellm, not applicable to the desktop app)

---

## Low-Level Design

### Architecture Overview

The i18n system lives entirely in the frontend (SolidJS + TypeScript). The only Rust change is a new `get_os_locale` Tauri command. The flow:

```
App startup
  → createLocaleState()
    → localStorage has value? → use it
    → navigator.language? → resolveLocale(navigator.language)
    → invoke("get_os_locale")? → resolveLocale(osLocale)
    → "en-US" hard default
  → locale signal set
  → loadDict(initialLocale) → dict applied
  → ensureDictWatcher() → reactive dict reload on locale change
```

### Files to Create (16 new locale files)

Each file at `src/lib/locales/{code}.ts`, following the pattern:

```typescript
import type { LocaleDict } from "../grill-me";

const dict: LocaleDict = {};

export default dict;
```

Files: `es-ES.ts`, `fr-FR.ts`, `de-DE.ts`, `it-IT.ts`, `ru-RU.ts`, `tr-TR.ts`, `ar-SA.ts`, `hi-IN.ts`, `bn-BD.ts`, `ur-PK.ts`, `zh-CN.ts`, `ja-JP.ts`, `ko-KR.ts`, `vi-VN.ts`, `id-ID.ts`, `pt-PT.ts`

### Files to Modify

#### 1. `src-tauri/Cargo.toml`

Add under `[dependencies]`:
```toml
sys-locale = "0.3"
```

`sys-locale` 0.3 is a lightweight crate (~15KB) with zero transitive runtime deps. Returns locale strings like `"en-US"`, `"pt-BR"`, `"de-DE"`.

#### 2. `src-tauri/src/commands/locale.rs` (NEW)

```rust
/// Returns the OS locale string (e.g. "en-US", "pt-BR"), or "en-US" on failure.
#[tauri::command]
fn get_os_locale() -> String {
    sys_locale::get_locale().unwrap_or_else(|| "en-US".to_string())
}
```

#### 3. `src-tauri/src/lib.rs`

Add `mod locale;` to `commands/mod.rs` (or inline in `lib.rs`). Register in `invoke_handler`:
```rust
commands::locale::get_os_locale,
```

Also add `pub mod locale;` inside the `commands` module. Let me check how commands are structured.

Wait — `lib.rs` does `mod commands;` and the `commands` dir has individual files. Let me check if there's a `commands/mod.rs`.

Actually from the lib.rs: `mod commands;` — this resolves to `commands/mod.rs` or `commands.rs`. Let me verify. The grep shows `commands/agent.rs`, `commands/shell.rs`, etc. — so there must be a `commands/mod.rs`.

Let me check.

For the LLD I'll note: Add `src-tauri/src/commands/locale.rs` with the command, then add `pub mod locale;` to `src-tauri/src/commands/mod.rs` and register in `lib.rs` invoke_handler.

#### 4. `src/lib/ipc.ts`

Add:
```typescript
export function getOsLocale(): Promise<string> {
  return invoke<string>("get_os_locale");
}
```

#### 5. `src/lib/grill-me.ts` — Major Changes

**5a. `LocaleId` type (line 3)** — extend union:
```typescript
export type LocaleId =
  | "en-US" | "pt-BR" | "pt-PT" | "es-ES" | "fr-FR" | "de-DE"
  | "it-IT" | "ru-RU" | "tr-TR" | "ar-SA" | "hi-IN" | "bn-BD"
  | "ur-PK" | "zh-CN" | "ja-JP" | "ko-KR" | "vi-VN" | "id-ID";
```

**5b. `SUPPORTED_LOCALES` constant (NEW, before `createLocaleState`):**
```typescript
export const SUPPORTED_LOCALES: LocaleId[] = [
  "en-US", "pt-BR", "pt-PT", "es-ES", "fr-FR", "de-DE",
  "it-IT", "ru-RU", "tr-TR", "ar-SA", "hi-IN", "bn-BD",
  "ur-PK", "zh-CN", "ja-JP", "ko-KR", "vi-VN", "id-ID",
];
```

**5c. `resolveLocale(raw: string): LocaleId` function (NEW):**
```typescript
export function resolveLocale(raw: string): LocaleId {
  // Exact match
  if (SUPPORTED_LOCALES.includes(raw as LocaleId)) return raw as LocaleId;
  // Language prefix match (e.g. "pt" → "pt-BR")
  const prefix = raw.split("-")[0].toLowerCase();
  for (const loc of SUPPORTED_LOCALES) {
    if (loc.toLowerCase().startsWith(prefix)) return loc;
  }
  // Fallback
  return "en-US";
}
```

**5d. `createLocaleState()` (lines 12-17)** — change initial fallback:

Current:
```typescript
const initial: LocaleId = stored ?? "pt-BR";
```

New: The initial is `stored ?? null`. If `null`, resolve asynchronously via `navigator.language` → Tauri → `en-US`. This requires converting the initialization to be async.

However, `createSignal` needs a synchronous initial value. The pattern will be:
1. Default the signal to `"en-US"` synchronously on module init
2. Kick off an async resolution in a `createRoot` that upgrades to the detected locale if no `localStorage` value exists

Implementation strategy:

```typescript
function createLocaleState() {
  const stored = (typeof localStorage !== "undefined"
    ? localStorage.getItem(STORAGE_KEY)
    : null) as LocaleId | null;
  const initial: LocaleId = stored ?? "en-US";
  const [locale, _setLocale] = createSignal<LocaleId>(initial);

  // If no stored locale, detect from system
  if (!stored) {
    detectAndSetLocale(_setLocale);
  }

  const setLocale = (id: LocaleId) => { ... }; // unchanged
  return { locale, setLocale };
}

async function detectAndSetLocale(setter: (id: LocaleId) => void) {
  // 1. navigator.language
  if (typeof navigator !== "undefined") {
    const nav = navigator.language;
    if (nav) {
      const resolved = resolveLocale(nav);
      if (resolved !== "en-US") { setter(resolved); return; }
    }
  }
  // 2. Tauri OS locale
  try {
    const { getOsLocale } = await import("../lib/ipc");
    const os = await getOsLocale();
    if (os) { setter(resolveLocale(os)); return; }
  } catch {}
  // 3. Stay on en-US
}
```

Wait, but `getOsLocale` is in `ipc.ts` which imports from `@tauri-apps/api/core`. That might not work in SSR. Let me use dynamic import to avoid issues.

Actually, let me reconsider. The current code already handles SSR. The Tauri invoke will only work in Tauri context — in SSR/dev server it will fail, which is fine because we `catch` it. The dynamic import pattern avoids bundling issues.

**5e. `loadDict()` (lines 75-85)** — switch/case with all 18 locales:

Current if/else replaced with full chain:
```typescript
export async function loadDict(id: LocaleId): Promise<LocaleDict> {
  if (dictCache.has(id)) return dictCache.get(id)!;
  let mod: { default: LocaleDict };
  switch (id) {
    case "pt-BR":   mod = await import("./locales/pt-BR"); break;
    case "en-US":   mod = await import("./locales/en-US"); break;
    case "pt-PT":   mod = await import("./locales/pt-PT"); break;
    case "es-ES":   mod = await import("./locales/es-ES"); break;
    case "fr-FR":   mod = await import("./locales/fr-FR"); break;
    case "de-DE":   mod = await import("./locales/de-DE"); break;
    case "it-IT":   mod = await import("./locales/it-IT"); break;
    case "ru-RU":   mod = await import("./locales/ru-RU"); break;
    case "tr-TR":   mod = await import("./locales/tr-TR"); break;
    case "ar-SA":   mod = await import("./locales/ar-SA"); break;
    case "hi-IN":   mod = await import("./locales/hi-IN"); break;
    case "bn-BD":   mod = await import("./locales/bn-BD"); break;
    case "ur-PK":   mod = await import("./locales/ur-PK"); break;
    case "zh-CN":   mod = await import("./locales/zh-CN"); break;
    case "ja-JP":   mod = await import("./locales/ja-JP"); break;
    case "ko-KR":   mod = await import("./locales/ko-KR"); break;
    case "vi-VN":   mod = await import("./locales/vi-VN"); break;
    case "id-ID":   mod = await import("./locales/id-ID"); break;
  }
  dictCache.set(id, mod.default);
  return mod.default;
}
```

**5f. `t()` (lines 107-116)** — add en-US fallback on line 110:

Current:
```typescript
if (val === undefined) return key;
```

New:
```typescript
if (val === undefined) {
  // Fallback to en-US
  const enDict = dictCache.get("en-US");
  if (enDict) {
    val = enDict[key];
    if (val !== undefined) {
      // Found in en-US — continue to interpolation below
    } else {
      return key;
    }
  } else {
    return key;
  }
}
```

But we need to restructure the `t()` function slightly to handle the fallback value flowing into interpolation. Better approach:

```typescript
export function t(key: string, ...args: (string | number)[]): string {
  const dict = currentDict();
  let val = dict[key];
  if (val === undefined) {
    // Fallback to en-US dict
    const enDict = dictCache.get("en-US");
    if (enDict) val = enDict[key];
  }
  if (val === undefined) return key;
  if (typeof val === "function") return val(...args);
  let result = val;
  for (let i = 0; i < args.length; i++) {
    result = result.replace(new RegExp(`\\{${i}\\}`, "g"), String(args[i]));
  }
  return result;
}
```

**5g. Eagerly preload en-US dict** — add after `ensureDictWatcher()` call (line 105):
```typescript
// Preload en-US so t() fallback works synchronously
loadDict("en-US");
```

**5h. `FLAGS` (lines 119-122)** — extend to 18 entries:
```typescript
export const FLAGS: Record<LocaleId, string> = {
  "en-US": "🇺🇸", "pt-BR": "🇧🇷", "pt-PT": "🇵🇹",
  "es-ES": "🇪🇸", "fr-FR": "🇫🇷", "de-DE": "🇩🇪",
  "it-IT": "🇮🇹", "ru-RU": "🇷🇺", "tr-TR": "🇹🇷",
  "ar-SA": "🇸🇦", "hi-IN": "🇮🇳", "bn-BD": "🇧🇩",
  "ur-PK": "🇵🇰", "zh-CN": "🇨🇳", "ja-JP": "🇯🇵",
  "ko-KR": "🇰🇷", "vi-VN": "🇻🇳", "id-ID": "🇮🇩",
};
```

**5i. `LOCALE_LABELS` (lines 124-126)** — extend to 18 entries:
```typescript
export const LOCALE_LABELS: Record<LocaleId, string> = {
  "en-US": "EN", "pt-BR": "PT", "pt-PT": "PT",
  "es-ES": "ES", "fr-FR": "FR", "de-DE": "DE",
  "it-IT": "IT", "ru-RU": "RU", "tr-TR": "TR",
  "ar-SA": "العربية", "hi-IN": "हिन्दी", "bn-BD": "বাংলা",
  "ur-PK": "اردو", "zh-CN": "中文", "ja-JP": "日本語",
  "ko-KR": "한국어", "vi-VN": "VI", "id-ID": "ID",
};
```

**5j. Dynamic `<html lang>` (NEW)** — add after `ensureDictWatcher()`:
```typescript
// Sync <html lang> to current locale
createRoot(() => {
  createEffect(() => {
    document.documentElement.lang = getLocaleState().locale();
  });
});
```

#### 6. `src/App.tsx` — Dynamic Locale Picker

**Lines 713-722:** Replace hardcoded `<select>` with dynamic generation:

```tsx
{/* Lang selector */}
<label class="mb-1 block text-xs text-ink-muted">{t("app.config.language")}</label>
<select
  value={locale()}
  onChange={(e) => setLocale(e.currentTarget.value as LocaleId)}
  class="mb-4 w-full appearance-none rounded-md border border-border-subtle bg-surface-0 p-2 text-sm text-ink focus:border-accent focus:outline-none focus:ring-1 focus:ring-accent"
>
  <For each={SUPPORTED_LOCALES}>
    {(loc) => (
      <option value={loc}>{FLAGS[loc]} {LOCALE_LABELS[loc]}</option>
    )}
  </For>
</select>
```

Also add `SUPPORTED_LOCALES, FLAGS` to the import from `grill-me` on line 8:
```tsx
import { t, locale, setLocale, type LocaleId, SUPPORTED_LOCALES, FLAGS } from "./lib/grill-me";
```

#### 7. `src/lib/locales/en-US.ts` — Add New Translation Key

Add after existing keys:
```typescript
"app.config.language": "Language",
```

#### 8. `src/lib/locales/pt-BR.ts` — Add New Translation Key

Add after existing keys:
```typescript
"app.config.language": "Idioma",
```

#### 9. `src/lib/locales.test.ts` — Update Tests

The current tests hardcode imports of `enUS` and `ptBR`. Need to:

**9a.** Update "both exports have the same number of keys" to be more flexible — or better, change it to test that all 18 dictionaries can be loaded and have valid values.

**9b.** Add a test that new locale dicts with empty objects don't break `t()` (cover by testing fallback behavior).

**9c.** Add tests for `resolveLocale()`:
- Exact match: `resolveLocale("pt-BR")` → `"pt-BR"`
- Prefix match: `resolveLocale("pt")` → `"pt-BR"`
- No match: `resolveLocale("zz-ZZ")` → `"en-US"`

**9d.** Add tests for `SUPPORTED_LOCALES` ordering and completeness.

Simplified approach for the empty dicts: since the 16 new dicts export `{}`, the key parity test with en-US would fail. Instead, the parity test should only apply to locales that have content, or the test should be restructured to test that `t()` with en-US fallback works correctly for empty locales.

#### 10. `src/lib/grill-me.test.ts` — Update Mocks

Add `vi.mock()` calls for all 16 new locale modules (lines 23-35). Each mock returns `{ default: {} }`.

#### 11. `index.html` — Update Default `lang`

Change line 2 from:
```html
<html lang="pt-BR" data-theme="dark">
```
to:
```html
<html lang="en-US" data-theme="dark">
```

This is the initial value before the JS effect kicks in.

#### 12. `src-tauri/src/commands/mod.rs`

Add `pub mod locale;` to expose the new command module.

### Data Flow

```
User opens app
  → grill-me.ts module init
    → createLocaleState()
      → localStorage "claudinio_locale" = null (first run)
      → initial = "en-US" (synchronous fallback)
      → detectAndSetLocale() kicked off (async)
        → navigator.language = "pt-BR" → resolveLocale("pt-BR") = "pt-BR"
        → setLocale("pt-BR") → signal updated, localStorage written
    → loadDict("en-US") — eagerly preloaded for fallback
    → ensureDictWatcher() — reactive effect created
  → App renders
    → locale() = "pt-BR" (after async resolution)
    → All t() calls use pt-BR dict
    → <html lang="pt-BR"> via effect

User opens language dropdown in settings
  → <select> shows all 18 locales dynamically from SUPPORTED_LOCALES
  → User picks "fr-FR"
  → setLocale("fr-FR") → signal + localStorage updated
  → ensureDictWatcher effect fires
    → loadDict("fr-FR") → imports empty dict {}
    → t() calls: dict["app.title"] = undefined → en-US["app.title"] = "Claudinio Code"
    → UI shows English text for all untranslated keys
```

### Wiring Checklist

| Seam | Source | Target | Verification |
|------|--------|--------|-------------|
| `get_os_locale` Rust command | `src-tauri/src/commands/locale.rs` | Registered in `lib.rs` invoke_handler | `grep get_os_locale src-tauri/src/lib.rs` |
| `getOsLocale()` IPC | `src/lib/ipc.ts` | Called from `grill-me.ts` `detectAndSetLocale()` | Dynamically imported; `catch` handles missing Tauri |
| New locale dicts | `src/lib/locales/*.ts` | `loadDict()` switch/case branches | `SUPPORTED_LOCALES` matches file count |
| `t()` fallback | `grill-me.ts:110` | `dictCache.get("en-US")` | `loadDict("en-US")` preloads synchronously |
| Dynamic `<html lang>` | `createEffect` in `grill-me.ts` | `document.documentElement.lang` | Effect fires on every `locale()` change |
| Dynamic picker | `SUPPORTED_LOCALES` + `FLAGS` | `<For>` in `App.tsx:713` | Renders 18 options |
| `app.config.language` key | `en-US.ts` + `pt-BR.ts` | `t("app.config.language")` in `App.tsx:713` | Key exists in both files |

### Verification Plan

1. **TypeScript build**: `npx tsc --noEmit` — must pass with no errors (LocaleId union exhaustiveness checked in FLAGS/LOCALE_LABELS)
2. **Existing tests**: `npx vitest run` — all existing tests pass + new `resolveLocale` tests
3. **New locale files**: `ls src/lib/locales/*.ts | wc -l` → 18 files
4. **Rust build**: `cargo build -p claudinio-code` — `sys-locale` compiles, `get_os_locale` registered
5. **Manual smoke test**: Open app, check that `<html lang>` updates, language dropdown shows 18 options, selecting a new locale works and shows English fallback

---

## Tasks Summary

1. Add `sys-locale` crate to `Cargo.toml` and create `get_os_locale` Tauri command
2. Register `get_os_locale` in `lib.rs` invoke_handler and `commands/mod.rs`
3. Add `getOsLocale()` to `src/lib/ipc.ts` (TypeScript IPC binding)
4. Create 16 empty locale dict files in `src/lib/locales/`
5. Extend `grill-me.ts`: LocaleId type, SUPPORTED_LOCALES, resolveLocale(), FLAGS, LOCALE_LABELS, loadDict()
6. Add en-US fallback chain in `t()` and eager en-US preload
7. Add system language detection in `createLocaleState()` and dynamic `<html lang>` effect
8. Update `index.html` lang to `en-US`
9. Refactor `App.tsx` locale picker to dynamic `<For>` + add `"app.config.language"` key
10. Add `"app.config.language"` to `en-US.ts` and `pt-BR.ts`
11. Update `locales.test.ts` and `grill-me.test.ts` for 18 locales
12. Build verification: TypeScript + Vitest + Cargo


## Implementation Log — 2026-07-17 09:25
**Summary:** 18-locale i18n: LocaleId expanded, system detection, en-US fallback, dynamic <html lang>, dynamic picker
**Changed files:** M docs/plans/2026-07-16_post-release-cache-cleanup.md, M index.html, M src-tauri/Cargo.lock, M src-tauri/Cargo.toml, M src-tauri/src/commands/mod.rs, M src-tauri/src/lib.rs, M src/App.tsx, M src/lib/grill-me.test.ts, M src/lib/grill-me.ts, M src/lib/ipc.ts, M src/lib/locales.test.ts, M src/lib/locales/en-US.ts, M src/lib/locales/pt-BR.ts, ?? docs/plans/2026-07-17_multi-language-support.md, ?? src-tauri/src/commands/locale.rs, ?? src/lib/locales/ar-SA.ts, ?? src/lib/locales/bn-BD.ts, ?? src/lib/locales/de-DE.ts, ?? src/lib/locales/es-ES.ts, ?? src/lib/locales/fr-FR.ts, ?? src/lib/locales/hi-IN.ts, ?? src/lib/locales/id-ID.ts, ?? src/lib/locales/it-IT.ts, ?? src/lib/locales/ja-JP.ts, ?? src/lib/locales/ko-KR.ts, ?? src/lib/locales/pt-PT.ts, ?? src/lib/locales/ru-RU.ts, ?? src/lib/locales/tr-TR.ts, ?? src/lib/locales/ur-PK.ts, ?? src/lib/locales/vi-VN.ts, ?? src/lib/locales/zh-CN.ts
**Commits:** _(git unavailable or none)_
**Journal:** Key findings during implementation:

1. **`vi.stubGlobal` ordering in locales.test.ts**: Static ES imports are hoisted, so `vi.stubGlobal("localStorage", ...)` must be a direct call (not wrapped in `beforeAll`) or grill-me.ts module-init code crashes before the stub is active. Used a dynamic `import()` inside `beforeAll` to defer grill-me import until after the stub.

2. **`LOCALE_LABELS` needed in App.tsx import**: The `<For>` template uses `LOCALE_LABELS[loc]` in the option display text, so it had to be added to the import alongside `SUPPORTED_LOCALES` and `FLAGS`. The subagent caught this independently.

3. **Default locale change cascaded through 4 tests**: Changing the hardcoded default from "pt-BR" to "en-US" broke tests that relied on pt-BR being the active locale at import time. Each test needed explicit `setLocale("pt-BR")` + `flushUntil` to switch before asserting pt-BR values.

4. **`resolveLocale` empty-string edge case**: An empty Accept-Language header produces `""` which splits to `[""]` with prefix `""`. The loop over `SUPPORTED_LOCALES` finds no match (no locale code starts with `""`), so it correctly falls back to `"en-US"`. Added a test for this case.

5. **Cargo build path**: `cargo build -p claudinio-code` must run from `src-tauri/`, not the workspace root, since the root has no `Cargo.toml`.

**Task journal:**
- Multi-language support: 18 locales, system detection, en-US fallback, dynamic <html lang>: All 12 modifications + 18 new files implemented; TypeScript: no new errors from our changes (all pre-existing); Vitest: locales.test.ts 9/9, grill-me.test.ts 17/17 — all pass; Cargo: builds clean with no new warnings; 18 locale files confirmed: all 18 codes present
- Add sys-locale crate + get_os_locale Tauri command: Added sys-locale = "0.3" to Cargo.toml line 116; Created src-tauri/src/commands/locale.rs with get_os_locale command; Added pub mod locale; to commands/mod.rs line 10; Registered commands::locale::get_os_locale in lib.rs line 26
- Add getOsLocale() to src/lib/ipc.ts: Added getOsLocale() function at lines 47-50 in ipc.ts, after copyPath
- Create 16 empty locale dict files: Created all 16 locale files with empty dicts. Verified: ls src/lib/locales/*.ts | wc -l = 18 files
- Extend grill-me.ts: LocaleId, SUPPORTED_LOCALES, resolveLocale, FLAGS, LOCALE_LABELS, loadDict: Extended LocaleId to 18 locales, added SUPPORTED_LOCALES and resolveLocale(); Replaced loadDict if/else with 18-case switch, extended FLAGS and LOCALE_LABELS; Added system detection (navigator.language + getOsLocale), en-US fallback in t(), eager en-US preload, <html lang> sync effect
- Add en-US fallback in t() + eager en-US preload + system detection + dynamic <html lang>: All fallback/detection/lang changes applied as part of grill-me.ts rewrite
- Update index.html lang to en-US: Changed index.html line 2 from pt-BR to en-US
- Refactor App.tsx locale picker to dynamic + add app.config.language key: App.tsx line 8: added SUPPORTED_LOCALES, FLAGS, LOCALE_LABELS to import; App.tsx lines 711-722: replaced hardcoded select with dynamic <For> loop; en-US.ts line 88: added "app.config.language": "Language"; pt-BR.ts line 88: added "app.config.language": "Idioma"
- Update locale tests for 18 locales: locales.test.ts: added localStorage stub, 16 locale imports, empty dict tests, resolveLocale tests — 9/9 pass; grill-me.test.ts: added 16 vi.mock() calls, FLAGS/LOCALE_LABELS now count check, default expect changed to en-US, 4 tests fixed for new default — 17/17 pass
- Build verification: TypeScript + Vitest + Cargo: locales.test.ts: 9/9 pass; grill-me.test.ts: 17/17 pass; Cargo build: clean (pre-existing warnings only, no new ones); 18 locale files confirmed in src/lib/locales/; All LocaleId codes present in: type, SUPPORTED_LOCALES, loadDict switch, FLAGS, LOCALE_LABELS
