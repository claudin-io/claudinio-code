# Update Button in Header (next to logo)

## Context
The app already has an auto-update system (`checkForUpdate()` → `@tauri-apps/plugin-updater`) and a fixed bottom-right banner showing "New version X available" with "Update now" and "Later" buttons. The user wants an additional, more prominent "Update to vX.Y.Z" button right next to the logo pill in the header — similar to the reference image — that triggers the install directly on click.

## Solution Design

### UI Placement
A new button inserted immediately after the logo/version pill (`<span class="rounded-full border border-accent/30 ...">`) in the `<header>` of `src/App.tsx`. The button is a rounded rectangular badge with `bg-warning` (yellow-amber — an existing theme token used by zero components today, defined in all 16 themes) and black text for maximum contrast.

### States
1. **Hidden** — when `updateInfo()` is `null` (no update available). Renders nothing.
2. **Idle** — when `updateInfo()` is set and `updateProgress()` is `null`. Shows `"Update to v{newVersion}"` text. Clicks call `installUpdate()`.
3. **Installing** — when `updateProgress() !== null`. Shows a spinner icon (`loader` icon with `animate-spin`) + `"Installing…"` text. Button is still visible but non-interactive (`pointer-events-none` / `cursor-default`).

### Color
- **Background**: `bg-warning` — the `--warning` CSS custom property, an amber/yellow hue (~85°) defined in every theme variant in `App.css` with appropriate lightness/chroma per theme.
- **Text**: `text-black` — maximum contrast against the yellow-amber background across all 16 themes.

### Coexistence with Existing Banner
The bottom-right banner (`<Show when={updateInfo() && !updateBannerDismissed()}>`) remains unchanged. Both the header button and the banner react to the same signals (`updateInfo`, `updateProgress`, `updateInstallError`). The button does NOT dismiss the banner — they coexist. If the user clicks the header button, install begins, both the button and banner show progress.

### Locale
New i18n keys:
- `update.updateTo`: `"Update to v{0}"` (en), `"Atualizar para v{0}"` (pt-BR)
- `update.installing`: `"Installing…"` (en), `"Instalando…"` (pt-BR)

### Sizing & Layout
- `rounded-md`, `px-2.5 py-1`, `text-xs`, `font-medium`
- Fits inline between the logo pill and the `ml-auto` right-side controls

## Risks
- **Low risk**: Piggybacks on existing signals. The only new code is a `<Show>` + `<button>` in the header.
- **Color contrast**: `bg-warning` varies across themes but is always a yellow-amber hue — black text guarantees readability.

## Non-goals
- Not replacing the bottom-right banner
- Not changing the update check logic
- Not modifying the config panel's update section
- No new dependencies or components — inline in App.tsx

## Low-Level Design

### Files to Modify

| File | Change |
|------|--------|
| `src/App.tsx` | Add update button in header (line ~689), right after logo pill `</span>` |
| `src/lib/locales/en-US.ts` | Add `update.updateTo` and `update.installing` keys |
| `src/lib/locales/pt-BR.ts` | Add `update.updateTo` and `update.installing` keys |

### Data Flow

```
updateInfo() signal (line 141)
  ├── null → button hidden, banner hidden
  └── { version, currentVersion, ... } → button visible, banner visible

updateProgress() signal (line 145)
  ├── null → button idle (clickable, shows "Update to vX.Y.Z")
  └── number → button installing (spinner + "Installing…", non-clickable)

installUpdate() (line 313)
  ├── sets updateProgress(0)
  ├── calls info.install(callback)
  └── on error → sets updateInstallError()
```

### Button Code (insert at line 689 in App.tsx)

Insert right after the closing `</span>` of the logo pill (currently line 689):

```tsx
<Show when={updateInfo()}>
  <button
    onClick={() => void installUpdate()}
    disabled={updateProgress() !== null}
    class="rounded-md bg-warning px-2.5 py-1 text-xs font-medium text-black transition-opacity hover:opacity-90 disabled:cursor-default disabled:opacity-80"
  >
    <Show
      when={updateProgress() !== null}
      fallback={<>{t("update.updateTo", updateInfo()!.version)}</>}
    >
      <span class="flex items-center gap-1.5">
        <Icon name="loader" class="h-3 w-3 animate-spin" />
        {t("update.installing")}
      </span>
    </Show>
  </button>
</Show>
```

### Locale Insertions

**en-US.ts** — after existing `update.*` keys (after line 62 `"update.error"`):
```ts
"update.updateTo": "Update to v{0}",
"update.installing": "Installing…",
```

**pt-BR.ts** — same position:
```ts
"update.updateTo": "Atualizar para v{0}",
"update.installing": "Instalando…",
```

### Existing Imports Already Available in App.tsx
- `Show` from solid-js (line 1) ✓
- `Icon` from `./components/Icon` (line 17) ✓
- `t` from `./lib/grill-me` (line 10) ✓
- `updateInfo`, `updateProgress`, `installUpdate` — all local signals/functions ✓

### The `loader` Icon
Already defined in `src/components/Icon.tsx` (line ~72). Animated with `animate-spin` class. The `animate-spin` utility exists in Tailwind v4.

### Tailwind Classes Used
- `bg-warning` → maps to `--color-warning` → `var(--warning)` in `@theme inline` (App.css line 505)
- `text-black` → standard Tailwind
- `rounded-md` → maps to `--radius-md`
- `transition-opacity`, `hover:opacity-90`, `disabled:opacity-80`, `disabled:cursor-default`
- `animate-spin` → built-in Tailwind

### Verification
1. Build check: `npm run build` (or equivalent) — should compile without errors
2. Visual: the button only renders when `updateInfo()` is set (requires an actual update from the Tauri endpoint, or can be tested by temporarily hardcoding `updateInfo` signal)
3. The bottom-right banner must still work independently
4. All 16 themes — `bg-warning text-black` must be readable in every theme

## Tasks Summary
1. Add `update.updateTo` and `update.installing` locale keys to `en-US.ts`
2. Add `update.updateTo` and `update.installing` locale keys to `pt-BR.ts`
3. Insert the update button `<Show>` block in App.tsx header, after the logo pill closing tag
4. Build and verify compilation
