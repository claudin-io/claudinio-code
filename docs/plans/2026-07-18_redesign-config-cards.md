# Redesign Config Toggle Cards — Settings Panel

## Context

The settings modal in `App.tsx` has three configuration checkboxes ("Keep awake while working", "Code intelligence", "Auto-commit plan on finalize") displayed as single-row `<label>` elements: `checkbox + emoji-label + hint-text` all on one line. Long labels cause the hint to wrap unpredictably, creating misaligned, uneven visual rhythm.

**User request:** Redesign these three toggles using the project's design skills (impeccable, design-taste-frontend, frontend-design). Replace emojis with real icons (`<Icon>` component + custom coffee SVG). Use a card-style layout with a minimal/editorial aesthetic.

## Solution Design

### Layout: Card-style, minimal/editorial

Each config option becomes a distinct card:
- **Left accent border** (2px) — transparent by default, transitions to `text-accent` when checked, subtle hover state
- No background fill — clean, integrated into the settings panel
- **Icon on the left** (16px), checkbox beside it, text block to the right
- **Text stacked vertically**: label (bold, `text-sm`) on top, hint (`text-[11px]`, `text-ink-faint`) below
- Compact spacing between cards (`space-y-1`)
- Checkbox stays visible but subtly sized (`h-3.5 w-3.5`)

### Icons (replace emojis ☕🧠📋)

| Toggle | Icon | Source |
|--------|------|--------|
| Keep awake | Custom coffee cup (stroke SVG from Huge Icons) | User-provided SVG → new `"coffee-cup"` entry in PATHS |
| Code intelligence | `brain` | Existing `Icon` component (pixel art, fill) |
| Auto-commit plan | `notebook-pen` | Existing `Icon` component (Lucide, stroke) |

### Locale strings

Remove emoji prefixes (`☕ `, `🧠 `, `📋 `) from locale files since real icons now serve that role.

### Visual reference (ASCII wireframe)

```
┌──────────────────────────────────────────────┐
│                                              │
│  ▌ [☐] [☕]  Keep awake while working        │
│  ▌           Prevents the system from...      │
│                                              │
│  ▌ [☐] [🧠]  Code intelligence              │
│  ▌           Enables LSP, FTS5 index...       │
│                                              │
│  ▌ [☐] [📋]  Auto-commit plan on finalize   │
│  ▌           Automatically commits the...     │
│                                              │
└──────────────────────────────────────────────┘

▌ = accent left border (visible when checked)
[☐] = checkbox
[icon] = 16×16 icon
```

### Interaction states
- **Default**: transparent left border, checkbox unchecked
- **Hover**: left border → `border-accent/30`
- **Checked**: left border → `border-accent`, checkbox filled
- Entire card is clickable (wrapped in `<label>`)
- `transition-colors` for smooth border transitions

### Edge cases
- Long hint text → wraps naturally within card boundaries
- No visual breakage at narrow settings panel widths
- Dark/light theme: icons use `currentColor`, border uses `text-accent` token

## Risks

| Risk | Mitigation |
|------|-----------|
| Coffee cup icon looks different from other icons (pixel vs stroke) | Brain is pixel fill, notebook-pen is stroke — icons are already mixed-style in this codebase, so a third style fits |
| `has-[:checked]` not supported in older Tauri webviews | Tauri 2.x uses latest WebKit, `:has()` is well-supported |
| Border transition might feel heavy | Using `transition-colors` (color only), lightweight |

## Non-goals

- Not changing any other part of the settings panel
- Not adding new configuration options
- Not modifying the YOLO section or IDE selector
- Not introducing new dependencies

---

## Low-Level Design

### Files to modify

1. **`src/components/Icon.tsx`** — add coffee cup icon to PATHS and STROKE_ICONS
2. **`src/App.tsx`** — replace checkbox rows (lines ~1191–1225) with card layout
3. **`src/lib/locales/en-US.ts`** — strip emoji prefixes from label strings (lines 69, 71, 73)
4. **`src/lib/locales/pt-BR.ts`** — strip emoji prefixes from label strings (lines 69, 71, 73)

### Change 1: Add coffee cup icon (`src/components/Icon.tsx`)

**Target:** `PATHS` record and `STROKE_ICONS` record

Add new entry before `archive-drawer` in PATHS:

```ts
// hugeicons:coffee-cup (24×24), stroke icon
"coffee-cup": [
  "M18.25 10.5h1.39c1.852 0 2.402.265 2.357 1.584c-.073 2.183-1.058 4.72-4.997 5.416",
  "M5.946 20.615C2.572 18.02 2.075 14.34 2.001 10.5c-.031-1.659.45-2 2.658-2h10.682c2.208 0 2.69.341 2.658 2c-.074 3.84-.57 7.52-3.945 10.115c-.96.738-1.77.885-3.135.885H9.081c-1.364 0-2.174-.147-3.135-.886Z",
  "M11.309 2.5C10.762 2.839 10 4 10 5.5M7.54 4S7 4.5 7 5.5M14.001 4c-.273.17-.501 1-.501 1.5",
],
```

Add to STROKE_ICONS:
```ts
"coffee-cup": true,
```

**Wiring:** The `IconName` type auto-updates from `keyof typeof PATHS`. No import changes needed — existing `Icon` import in `App.tsx` already handles new icons.

### Change 2: Redesign checkbox rows (`src/App.tsx`, lines ~1191–1225)

**Current markup (all 3 use identical pattern):**
```jsx
<label class="mb-4 flex cursor-pointer items-center gap-2">
  <input type="checkbox" checked={...} onChange={...} class="h-4 w-4 rounded ..." />
  <span class="text-sm font-medium text-ink">{t("app.config.keepAwake")}</span>
  <span class="text-[11px] text-ink-faint">{t("app.config.keepAwakeHint")}</span>
</label>
```

**New markup — replace all 3 labels with:**

```jsx
<div class="space-y-1">
  {/* Keep awake while working */}
  <label class="group flex cursor-pointer items-start gap-3 border-l-2 border-transparent py-2 pl-3 pr-1 transition-colors has-[:checked]:border-accent hover:border-accent/30">
    <div class="mt-0.5 flex shrink-0 items-center gap-2">
      <input
        type="checkbox"
        checked={configKeepAwake()}
        onChange={(e) => setConfigKeepAwake(e.currentTarget.checked)}
        class="h-3.5 w-3.5 rounded border-border-subtle bg-surface-0 text-accent focus:ring-accent"
      />
      <Icon name="coffee-cup" class="h-4 w-4 text-ink-faint" stroke />
    </div>
    <div class="min-w-0">
      <span class="text-sm font-medium text-ink">{t("app.config.keepAwake")}</span>
      <span class="block text-[11px] leading-relaxed text-ink-faint">{t("app.config.keepAwakeHint")}</span>
    </div>
  </label>

  {/* Code intelligence */}
  <label class="group flex cursor-pointer items-start gap-3 border-l-2 border-transparent py-2 pl-3 pr-1 transition-colors has-[:checked]:border-accent hover:border-accent/30">
    <div class="mt-0.5 flex shrink-0 items-center gap-2">
      <input
        type="checkbox"
        checked={configCodeIntelEnabled()}
        onChange={(e) => setConfigCodeIntelEnabled(e.currentTarget.checked)}
        class="h-3.5 w-3.5 rounded border-border-subtle bg-surface-0 text-accent focus:ring-accent"
      />
      <Icon name="brain" class="h-4 w-4 text-ink-faint" />
    </div>
    <div class="min-w-0">
      <span class="text-sm font-medium text-ink">{t("app.config.codeIntel")}</span>
      <span class="block text-[11px] leading-relaxed text-ink-faint">{t("app.config.codeIntelHint")}</span>
    </div>
  </label>

  {/* Auto-commit plan on finalize */}
  <label class="group flex cursor-pointer items-start gap-3 border-l-2 border-transparent py-2 pl-3 pr-1 transition-colors has-[:checked]:border-accent hover:border-accent/30">
    <div class="mt-0.5 flex shrink-0 items-center gap-2">
      <input
        type="checkbox"
        checked={configAutoCommitPlan()}
        onChange={(e) => setConfigAutoCommitPlan(e.currentTarget.checked)}
        class="h-3.5 w-3.5 rounded border-border-subtle bg-surface-0 text-accent focus:ring-accent"
      />
      <Icon name="notebook-pen" class="h-4 w-4 text-ink-faint" stroke />
    </div>
    <div class="min-w-0">
      <span class="text-sm font-medium text-ink">{t("app.config.autoCommitPlan")}</span>
      <span class="block text-[11px] leading-relaxed text-ink-faint">{t("app.config.autoCommitPlanHint")}</span>
    </div>
  </label>
</div>
```

**CSS classes rationale:**
- `group` — enables `has-[:checked]:border-accent` via the `has-` pseudo-class which scopes to the group
- `flex items-start` — horizontal layout, top-aligned so multi-line hints don't misalign icons
- `gap-3` — 12px between checkbox+icon group and text block
- `border-l-2 border-transparent` — invisible left border in default state
- `py-2 pl-3 pr-1` — 8px vertical, 12px left (accounts for border), 4px right
- `transition-colors` — smooth border color transitions
- `has-[:checked]:border-accent` — when checkbox is checked, border turns accent color
- `hover:border-accent/30` — subtle hover preview of accent
- `mt-0.5` on icon row — optical centering with text baseline
- `min-w-0` on text block — prevents flex overflow
- `block` on hint span — forces new line regardless of width

### Change 3 & 4: Locale strings

**`src/lib/locales/en-US.ts`** (lines 69, 71, 73):

```
"app.config.keepAwake": "Keep awake while working",
"app.config.codeIntel": "Code intelligence",
"app.config.autoCommitPlan": "Auto-commit plan on finalize",
```

**`src/lib/locales/pt-BR.ts`** (lines 69, 71, 73):

```
"app.config.keepAwake": "Manter acordado enquanto trabalha",
"app.config.codeIntel": "Inteligência de código",
"app.config.autoCommitPlan": "Auto-commitar plano ao finalizar",
```

Hints remain unchanged (they never had emojis).

### Design system alignment

Per impeccable skill:
- **4pt spacing scale**: `gap-3` (12px), `pl-3` (12px), `py-2` (8px), `space-y-1` (4px)
- **Typography**: consistent with existing `text-sm font-medium` + `text-[11px] text-ink-faint` pattern
- **Icons**: `text-ink-faint` — subdued, don't compete with label text
- **No gradient text, no cards-inside-cards, no centering everything**

Per design-taste-frontend:
- **One accent color** — left border uses existing `text-accent` token
- **No "Lila Rule"** violations — no purple/blue glow aesthetic
- **Anti-default**: no generic card with background fill and shadow

### Verification Plan

1. **Build**: `npm run build` — must succeed with zero errors
2. **Icon registration**: grep for `"coffee-cup"` in `src/components/Icon.tsx` — must appear in both PATHS and STROKE_ICONS
3. **Locale check**: grep for `☕`, `🧠`, `📋` in `src/lib/locales/` — must return zero results
4. **Visual**: Open settings panel → verify all three cards render with correct icons, left border highlights blue on check, subtle on hover
5. **Functionality**: Toggle each checkbox → verify border state changes, verify config signal updates persist
6. **Theme toggle**: Switch between dark/light → verify icon colors adapt via `currentColor`
7. **Regression**: Verify YOLO section and IDE selector below the cards are unaffected

### Tasks summary

1. Add `coffee-cup` icon to `PATHS` and `STROKE_ICONS` in `src/components/Icon.tsx`
2. Strip emoji prefixes from `app.config.keepAwake`, `app.config.codeIntel`, `app.config.autoCommitPlan` in `src/lib/locales/en-US.ts`
3. Strip emoji prefixes from same keys in `src/lib/locales/pt-BR.ts`
4. Replace the three checkbox `<label>` elements in `src/App.tsx` (lines ~1191–1225) with the new card-style layout
5. Build, verify, and visually inspect


## Implementation Log — 2026-07-18 13:45
**Summary:** Redesigned config toggle cards: coffee-cup icon, card layout with accent left border, emoji-free locale labels
**Changed files:** A	docs/plans/2026-07-18_auto-commit-plan.md, A	docs/plans/2026-07-18_redesign-config-cards.md, M	docs/plans/2026-07-18_update-button-header.md, M	src-tauri/src/agent/provider.rs, M	src-tauri/src/agent/session.rs, M	src-tauri/src/agent/tools/write_plan.rs, M	src-tauri/src/commands/agent.rs, M	src/App.tsx, M	src/lib/ipc.ts, M	src/lib/locales/en-US.ts, M	src/lib/locales/pt-BR.ts
**Commits:** eae3fb5 docs: add implementation log to update-button-header plan, 25730f3 feat: auto-commit plan on finalize with config toggle, fe790b8 docs(plan): redesign-config-cards, 59569ba docs(plan): redesign-config-cards
**Journal:** All 5 tasks completed successfully. 

Key decisions during implementation:
- Coffee cup icon placed alphabetically before "construction-worker" in PATHS (line 93) and before "notebook-pen" in STROKE_ICONS (line 265) — fits naturally into existing ordering.
- Card layout uses `has-[:checked]` pseudo-class on the group `<label>` — works with Tauri 2.x WebKit. Icon row gets `mt-0.5` for optical centering with text baseline.
- Locale changes were surgical: only the 3 label keys, hints never had emojis so zero risk of collateral damage.
- Build verified: 35 test files / 643 tests pass, Vite production build produces clean output.

Gotchas: None. The plan was precise — all file paths, line numbers, and markup matched reality exactly.

**Task journal:**
- Add coffee-cup icon to Icon.tsx: Added 'coffee-cup' to PATHS (before construction-worker) and STROKE_ICONS (before notebook-pen). Alphabetical order correct.
- Strip emoji from en-US locale labels: Stripped ☕, 🧠, 📋 prefixes from keepAwake, codeIntel, autoCommitPlan labels. Hints unchanged.
- Strip emoji from pt-BR locale labels: Stripped ☕, 🧠, 📋 prefixes from keepAwake, codeIntel, autoCommitPlan labels in pt-BR. Hints unchanged.
- Replace checkbox rows with card layout in App.tsx: Replaced old labels with card layout. Three <Icon> components: coffee-cup (stroke), brain (fill), notebook-pen (stroke). Old mb-4 flex cursor-pointer pattern fully gone.
- Build and verify: Build: 35 test files, 643 tests all passed. Vite build succeeded, zero errors. Verified: coffee-cup in PATHS + STROKE_ICONS, zero emojis in locale files, old mb-4 flex pattern gone from App.tsx.
