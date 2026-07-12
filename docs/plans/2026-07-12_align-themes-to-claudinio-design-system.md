# Align Light & Dark Themes to Claudinio Design System

## Context / Problem Statement

The current theming system (3 themes: dark, light, sepia) uses warm brown/beige tones that don't match the Claudinio Design System. The DS defines a single dark mode with cool violet-toned OKLCH colors (hue ~280), but no light or sepia variants. This plan updates all three themes to derive from the DS, keeps existing token names, and adds missing DS tokens.

### CONFIRMED (via interview):
- Dark mode → DS dark tokens exactly
- Light mode → derived from DS (same hue, inverted lightness scale)
- Sepia → kept, derived from DS base (warm hue shift)
- Token names → kept (`--surface-0`, `--ink`, etc.), values updated from DS
- Radii → updated to DS values (6/8/12/16px)
- New tokens → added (spacing, accent variants, warning, subtles, glow, card-highlight)
- No light mode spec provided — derived algorithmically

### INFERRED:
- OKLCH color format used for all new values (DS standard)
- `--accent` mapped to DS `--accent-strong` (logo blue #5C60E6) since current `--accent` already IS that color
- `--accent-hover` mapped to DS `--accent-hover` (lighter than accent-strong in OKLCH)
- Shadows updated to DS format (OKLCH notation)

## Goal (Definition of Done)

All three themes (dark, light, sepia) use OKLCH colors derived from the Claudinio Design System:
- Dark mode: exact DS token values
- Light mode: same hue, inverted lightness/chroma scale, passing ≥4.5:1 AA contrast
- Sepia mode: warm-hue variant preserving character
- Monaco editor themes match the CSS tokens
- All new DS tokens available for future use
- Build passes, no visual regressions in theme switching

## Key Findings (Prova Real)

| Finding | Method | Proof |
|---|---|---|
| All theme tokens in `src/App.css` lines 4–65 | `read_file` | 3 blocks: `:root` (dark), `[data-theme="light"]`, `[data-theme="sepia"]` |
| Tailwind v4 bridge in `@theme inline` block, lines 67–90 | `read_file` | Maps CSS vars → Tailwind utilities like `bg-surface-1` |
| Monaco themes in `src/lib/monacoThemes.ts` | `read_file` | 3 custom themes matching CSS tokens by hardcoded hex |
| Theme switching via `data-theme` attribute on `<html>` | `read_file` on `src/lib/theme.ts` | No React context — SolidJS signal + `document.documentElement.dataset.theme` |
| Anti-flash script in `index.html` lines 3–18 | subagent report | Reads localStorage, sets `data-theme` before first paint |
| No `tailwind.config.*` (Tailwind v4, CSS-driven) | subagent report | `@tailwindcss/vite` plugin, `@theme inline` in CSS |
| Fonts: Inter + JetBrains Mono (both locally served woff2) | subagent report | `src/assets/fonts/fonts.css` |
| Current `--accent: #5C60E6` = DS `--accent-strong` (same hex) | comparison | Both are the logo blue |
| Current `--accent-ink: #fff` = DS `--accent-text: oklch(0.99 0 0)` ≈ white | comparison | Essentially identical |

## Authoritative Inputs

| Input | Source | Value |
|---|---|---|
| DS token values (dark mode) | User-provided CSS block | Full OKLCH palette (see Solution Design) |
| Light mode derivation algorithm | User agreement | Invert lightness, same hue ~280, adjusted chroma |
| Sepia mode derivation | User agreement | Shift to warm hue, keep character |
| Token naming convention | User agreement | Keep: surface-0/1/2/3, ink/ink-muted/ink-faint, border-subtle/strong, accent, accent-hover, accent-ink |
| Border radii | User agreement | sm=6px, md=8px, lg=12px, new xl=16px |
| New tokens to add | User agreement | All DS tokens: spacing, accent variants, warning, subtles, glow, card-highlight |
| Spacing scale | User-provided DS | --space-1:4px, --space-2:8px, --space-3:12px, --space-4:16px, --space-5:20px, --space-6:24px, --space-8:32px, --space-10:40px, --space-12:48px, --space-16:64px |

## Changes (Steps)

### Step 1: Update Dark Mode Tokens (`src/App.css`, `:root` block, lines 4–26)

**Mutation:** Replace all token values with DS OKLCH equivalents. Add new tokens.

| Token | Old Value | New Value (from DS) |
|---|---|---|
| `--surface-0` | `#141210` | `oklch(0.145 0.015 280)` |
| `--surface-1` | `#1c1917` | `oklch(0.17 0.015 280)` |
| `--surface-2` | `#26221e` | `oklch(0.185 0.018 280)` |
| `--surface-3` | `#302b26` | `oklch(0.23 0.02 280)` |
| `--border-subtle` | `#35302a` | `oklch(0.28 0.02 280)` |
| `--border-strong` | `#4a433b` | `oklch(0.33 0.02 280)` |
| `--ink` | `#ece8e3` | `oklch(0.95 0.01 280)` |
| `--ink-muted` | `#9c948a` | `oklch(0.78 0.015 280)` |
| `--ink-faint` | `#6e675f` | `oklch(0.65 0.02 280)` |
| `--accent` | `#5C60E6` | `oklch(0.562 0.199 276.6)` (DS `--accent-strong`) |
| `--accent-hover` | `#6B6FF0` | `oklch(0.59 0.2 277)` |
| `--accent-ink` | `#fff` | `oklch(0.99 0 0)` |
| `--success` | `#7fb069` | `oklch(0.72 0.17 155)` |
| `--danger` | `#e5735f` | `oklch(0.68 0.19 25)` |
| `--shadow-sm` | `0 1px 2px rgba(0,0,0,0.4)` | `0 1px 2px oklch(0 0 0 / 0.5)` |
| `--shadow-md` | `0 4px 12px rgba(0,0,0,0.5)` | `0 4px 16px oklch(0 0 0 / 0.5)` |
| `--shadow-modal` | `0 8px 32px rgba(0,0,0,0.6)` | `0 12px 40px oklch(0 0 0 / 0.6)` |
| `--radius-sm` | `4px` | `6px` |
| `--radius-md` | `6px` | `8px` |
| `--radius-lg` | `10px` | `12px` |

**New tokens to add:**
```
--radius-xl: 16px;
--accent-subtle: oklch(0.62 0.19 277 / 0.14);
--accent-glow: oklch(0.62 0.19 277 / 0.35);
--success-subtle: oklch(0.72 0.17 155 / 0.14);
--warning: oklch(0.78 0.15 85);
--warning-subtle: oklch(0.78 0.15 85 / 0.14);
--danger-subtle: oklch(0.68 0.19 25 / 0.14);
--glow-accent: 0 0 24px var(--accent-glow);
--card-highlight: inset 0 1px 0 oklch(1 0 0 / 0.05);
--space-1: 4px;
--space-2: 8px;
--space-3: 12px;
--space-4: 16px;
--space-5: 20px;
--space-6: 24px;
--space-8: 32px;
--space-10: 40px;
--space-12: 48px;
--space-16: 64px;
```

**Why:** Exact alignment with Claudinio DS. OKLCH provides perceptually uniform color manipulation.

### Step 2: Create Light Mode Tokens (`src/App.css`, `[data-theme="light"]` block, lines 28–47)

**Mutation:** Replace entire light block with DS-derived light values (hue ~280, inverted lightness).

**Light mode derived values:**
```
--surface-0: oklch(0.98 0.003 280);       /* lightest bg, near-white with violet hint */
--surface-1: oklch(0.95 0.006 280);
--surface-2: oklch(0.91 0.01 280);
--surface-3: oklch(0.86 0.015 280);
--border-subtle: oklch(0.80 0.02 280);
--border-strong: oklch(0.70 0.025 280);
--ink: oklch(0.18 0.02 280);               /* dark text, AA on surface-0 (≥10:1) */
--ink-muted: oklch(0.38 0.02 280);
--ink-faint: oklch(0.52 0.02 280);
--accent: oklch(0.50 0.20 277);            /* slightly darker for light bg contrast */
--accent-hover: oklch(0.46 0.195 277);
--accent-ink: oklch(0.99 0 0);             /* white */
--accent-subtle: oklch(0.50 0.20 277 / 0.12);
--accent-glow: oklch(0.50 0.20 277 / 0.25);
--success: oklch(0.58 0.17 155);
--success-subtle: oklch(0.58 0.17 155 / 0.12);
--warning: oklch(0.65 0.16 85);
--warning-subtle: oklch(0.65 0.16 85 / 0.12);
--danger: oklch(0.52 0.20 25);
--danger-subtle: oklch(0.52 0.20 25 / 0.12);
--shadow-sm: 0 1px 2px oklch(0 0 0 / 0.08);
--shadow-md: 0 4px 16px oklch(0 0 0 / 0.10);
--shadow-modal: 0 12px 40px oklch(0 0 0 / 0.12);
--glow-accent: 0 0 24px var(--accent-glow);
--card-highlight: inset 0 1px 0 oklch(0 0 0 / 0.04);
color-scheme: light;
```

Note: radii and spacing inherit from `:root` (not redefined). New tokens (`warning`, `subtle` variants, etc.) are defined for both themes.

**Why:** Light mode matching the same design language. Perceptually uniform OKLCH ensures consistent hue perception at different lightness levels.

### Step 3: Update Sepia Theme Tokens (`src/App.css`, `[data-theme="sepia"]` block, lines 49–65)

**Mutation:** Replace sepia values with warm-hue OKLCH equivalents (~85–90 hue), derived from DS structure.

**Sepia derived values:**
```
--surface-0: oklch(0.96 0.015 90);
--surface-1: oklch(0.92 0.02 90);
--surface-2: oklch(0.87 0.025 88);
--surface-3: oklch(0.81 0.03 85);
--border-subtle: oklch(0.76 0.035 85);
--border-strong: oklch(0.66 0.04 82);
--ink: oklch(0.22 0.03 80);
--ink-muted: oklch(0.42 0.03 82);
--ink-faint: oklch(0.56 0.03 82);
--accent: oklch(0.55 0.16 65);             /* warm orange-amber */
--accent-hover: oklch(0.50 0.15 65);
--accent-ink: oklch(0.99 0 0);
--accent-subtle: oklch(0.55 0.16 65 / 0.12);
--accent-glow: oklch(0.55 0.16 65 / 0.25);
--success: oklch(0.58 0.17 145);
--success-subtle: oklch(0.58 0.17 145 / 0.12);
--warning: oklch(0.65 0.16 80);
--warning-subtle: oklch(0.65 0.16 80 / 0.12);
--danger: oklch(0.52 0.20 25);
--danger-subtle: oklch(0.52 0.20 25 / 0.12);
--shadow-sm: 0 1px 2px oklch(0 0 0 / 0.08);
--shadow-md: 0 4px 16px oklch(0 0 0 / 0.10);
--shadow-modal: 0 12px 40px oklch(0 0 0 / 0.12);
--glow-accent: 0 0 24px var(--accent-glow);
--card-highlight: inset 0 1px 0 oklch(0 0 0 / 0.04);
color-scheme: light;
```

**Why:** Sepia charm preserved with OKLCH precision. Warm hue (~85–90) vs cool DS hue (~280).

### Step 4: Update Tailwind v4 Bridge (`src/App.css`, `@theme inline` block, lines 67–90)

**Mutation:** Add new color/radius/shadow token bridges.

**New Tailwind utility mappings:**
```
--color-accent-subtle: var(--accent-subtle);
--color-accent-glow: var(--accent-glow);
--color-success-subtle: var(--success-subtle);
--color-warning: var(--warning);
--color-warning-subtle: var(--warning-subtle);
--color-danger-subtle: var(--danger-subtle);
--radius-xl: var(--radius-xl);
--shadow-lg: none;  /* not needed, --shadow-modal covers it */
```

**Why:** Keep Tailwind utilities in sync with CSS custom properties so `bg-accent-subtle`, `text-warning`, `rounded-xl` etc. resolve correctly.

**Constraints:** Don't change existing mappings — only add new ones.

### Step 5: Update Monaco Themes (`src/lib/monacoThemes.ts`)

**Mutation:** Replace all hex color references in all three Monaco themes with values matching the new CSS tokens.

Dark Monaco theme colors (OKLCH converted to hex for Monaco):
- Background: `#141210` → needs conversion from `oklch(0.145 0.015 280)` → approximately `#191927` or similar cool dark
- Actually, let me compute: oklch(0.145 0.015 280) is a very dark, slightly cool color. Approx hex: `#1b1a24`

Light Monaco theme: match Step 2 light values
Sepia Monaco theme: match Step 3 sepia values

**Important:** Monaco requires hex colors (#RRGGBB). Convert OKLCH values to their closest hex equivalents. I'll compute these during implementation using a conversion script.

**Why:** Monaco editor must visually match the CSS theme tokens.

**Constraints:** Monaco `defineTheme` only runs once (guarded by `defined` flag).

### Step 6: Update Tests

**Target:** `src/lib/theme.test.ts` and `src/lib/monacoThemes.test.ts`

**Why:** Tests may reference old color values.

**Constraints:** Only update color assertions — don't change test logic.

### Step 7: Build & Visual Verification

Run `npm run build` (or equivalent). Verify:
1. No build errors
2. All themes cycle correctly (dark → light → sepia → system)
3. Monaco editor matches each theme
4. No color flashes on theme switch

## NOT Changed (explicit non-targets)

- `index.html` anti-flash script — no color values to update
- `src/lib/theme.ts` — logic unchanged, only CSS token values change
- Component files — they reference token names that remain the same
- Tailwind v4 configuration — no `tailwind.config.*` file exists
- Font loading (`src/assets/fonts/fonts.css`) — fonts already match DS

## Risks

| Risk | Mitigation |
|---|---|
| OKLCH→hex conversion inaccuracies for Monaco | Use a conversion script to compute precise hex from OKLCH values |
| Light mode contrast too low | Verify all text/surface pairs ≥4.5:1 using APCA or WCAG calculator |
| Perceptual mismatch between OKLCH in CSS vs hex in Monaco | Accept small rounding differences; Monaco doesn't support OKLCH |
| Build breaks from OKLCH syntax | Tailwind v4 + modern Vite support OKLCH; verify one token first |

## Verification Plan

1. **Build:** `npm run build` — must exit 0
2. **Lint:** `npm run lint` (if exists) — must pass
3. **Tests:** `npm test` — must pass
4. **Theme cycling:** Manually cycle themes in Tauri app — no visual glitches
5. **Monaco matching:** Open a file in editor, cycle themes — editor colors match UI
6. **Contrast check:** Run contrast checker on light mode text/background pairs
7. **Regression:** Verify sepia mode still looks warm/paper-like

## Tasks Summary

1. Update `:root` dark mode tokens + add new DS tokens
2. Create light mode tokens derived from DS (same hue, inverted scale)
3. Update sepia mode tokens to warm-hue OKLCH
4. Add new token bridges to Tailwind `@theme inline` block
5. Update Monaco themes (dark/light/sepia) with OKLCH→hex converted colors
6. Update tests if they reference old color values
7. Build, test, and verify all three themes


## Implementation Log — 2026-07-12 01:49
**Summary:** Align dark/light/sepia themes to Claudinio Design System (OKLCH tokens, updated radii, new DS tokens, Monaco hex conversion)
**Changed files:** M src/App.css, M src/lib/monacoThemes.ts, ?? docs/plans/2026-07-12_align-themes-to-claudinio-design-system.md
**Commits:** _(git unavailable or none)_
**Journal:** Key decisions and findings:

1. **Accent contrast fix**: The DS has two accent variants -- `--accent` at oklch(0.62 0.19 277) (AA-safe as text, 5.16:1 on dark bg) and `--accent-strong` at oklch(0.562 0.199 276.6) (the logo blue #5C60E6, for fills with white text). Mapped `--accent` to the text-safe variant, added `--accent-strong` as a new token. This was not obvious from the DS CSS block alone.

2. **Light mode derivation**: No light spec existed — derived from the dark DS by inverting lightness scale (same hue ~280, lower chroma for surfaces). All contrast pairs verified ≥4.5:1 AA via OKLCH luminance computation. Light mode got a cool violet-toned palette consistent with the brand.

3. **Monaco OKLCH→hex conversion**: Monaco doesn't support OKLCH, so each color was computed via Python (OKLab → linear sRGB → sRGB transfer → 8-bit hex with optional alpha in #RRGGBBAA format). Cursor colors use the `--accent` variant.

4. **Radii updated**: sm=6px, md=8px, lg=12px, new xl=16px to match DS.

5. **New DS tokens added**: Full spacing scale, warning colors, subtle/semantic variants for all status colors, glow effects, card highlight — available for future use via both CSS vars and Tailwind utilities.

6. **No test changes needed**: Tests only verify logic/call count — no color assertions.

**Task journal:**
- Update :root dark mode tokens to DS values: Replaced all :root token values with exact DS OKLCH values; Added 10 new spacing tokens (--space-1 through --space-16); Added new accent/semantic tokens: --accent-glow, --accent-subtle, --success-subtle, --warning, --warning-subtle, --danger-subtle, --glow-accent, --card-highlight; Added --ink-subtle: oklch(0.58 0.02 280) (DS --text-muted equivalent); Updated radii: sm=6px, md=8px, lg=12px, new xl=16px; Updated shadows to OKLCH notation
- Create light mode tokens derived from DS: Replaced light block with DS-derived values (hue ~280, inverted scale); All new tokens included for light
- Update sepia theme tokens to warm-hue OKLCH: Replaced sepia block with warm-hue OKLCH (~85-90 hue); All new tokens included for sepia
- Add new token bridges to Tailwind @theme inline: Added: --color-accent-subtle, --color-accent-glow, --color-ink-subtle, --color-success-subtle, --color-warning, --color-warning-subtle, --color-danger-subtle, --radius-xl
- Update Monaco editor themes to match new tokens: Dark: bg #090910, fg #edeef5, cursor #6d74f5, gutter #090910; Light: bg #f8f8fa, fg #10111a, cursor #4d4bd1, gutter #f8f8fa; Sepia: bg #f6f2e7, fg #22190a, cursor #ae5700, gutter #f6f2e7; All values computed from OKLCH via Python conversion script
- Update tests referencing old color values: No color assertions found — tests only verify logic/call count/idempotency; No changes needed
- Build, test, and verify all themes: Build: ✓ passed (vite v6.4.3); Tests: 402/402 passed across 24 test files; Contrast verification (verified via Python OKLCH luminance):;   - ink on surface-0 (light): 17.78:1 ✓;   - ink-muted on surface-0 (light): 9.48:1 ✓;   - ink-faint on surface-0 (light): 5.22:1 ✓;   - accent on surface-0 (light): 6.09:1 ✓;   - ink-subtle on surface-0 (light): 4.59:1 ✓;   - accent on surface-0 (dark): 5.16:1 ✓;   - ink on surface-0 (dark): 17.10:1 ✓; All text/surface pairs ≥4.5:1 AA — verified
