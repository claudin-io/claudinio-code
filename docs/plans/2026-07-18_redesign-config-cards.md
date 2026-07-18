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
- Compact spacing between cards (`space-y-1` or `space-y-2`)
- Checkbox stays visible but subtly sized (3.5×3.5 `h-3.5 w-3.5`)

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

### Non-goals
- Not changing any other part of the settings panel
- Not adding new configuration options
- Not modifying the YOLO section or IDE selector
- Not introducing new dependencies

---

## Low-Level Design

### Files to modify

1. **`src/components/Icon.tsx`** — add coffee cup icon to PATHS and STROKE_ICONS
2. **`src/App.tsx`** — replace checkbox rows (lines ~1191–1225) with card layout
3. **`src/lib/locales/en-US.ts`** — strip emoji prefixes from label strings
4. **`src/lib/locales/pt-BR.ts`** — strip emoji prefixes from label strings

### Change 1: Add coffee cup icon (`src/components/Icon.tsx`)

**Target:** `PATHS` record and `STROKE_ICONS` record

Add new entry after the existing icons (before `archive-drawer` or at end of PATHS):

```ts
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

**Note:** The user-provided SVG had `stroke="#3a88fe"` hardcoded on the `<g>`. By registering as a stroke icon, the `Icon` component renders it with `stroke="currentColor"`, making it theme-aware.

### Change 2: Redesign checkbox rows (`src/App.tsx`, lines ~1191–1225)

**Current markup:**
```jsx
<label class="mb-4 flex cursor-pointer items-center gap-2">
  <input type="checkbox" ... class="h-4 w-4 ..." />
  <span class="text-sm font-medium text-ink">{t("app.config.keepAwake")}</span>
  <span class="text-[11px] text-ink-faint">{t("app.config.keepAwakeHint")}</span>
</label>
```

**New markup (replace all 3 labels):**

Wrap in a container with `space-y-1` for compact card separation.

```jsx
<div class="space-y-1">
  <!-- Keep awake card -->
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

  <!-- Code intelligence card -->
  <label class="...same...">
    <!-- Icon: brain (no stroke — it's a fill icon) -->
    <Icon name="brain" class="h-4 w-4 text-ink-faint" />
    ...
  </label>

  <!-- Auto-commit plan card -->
  <label class="...same...">
    <!-- Icon: notebook-pen (stroke) -->
    <Icon name="notebook-pen" class="h-4 w-4 text-ink-faint" stroke />
    ...
  </label>
</div>
```

**CSS classes breakdown:**
- `group` — enables group-hover for the border
- `flex items-start` — horizontal layout, top-aligned (for multi-line hints)
- `gap-3` — 12px between checkbox+icon group and text block
- `border-l-2 border-transparent` — invisible left border by default
- `py-2 pl-3 pr-1` — compact padding (8px vertical, 12px left for border offset, 4px right)
- `transition-colors` — smooth border transitions
- `has-[:checked]:border-accent` — when checkbox inside is checked, border turns accent
- `hover:border-accent/30` — subtle hover preview
- `mt-0.5` on icon row — optically centers with text baseline
- `min-w-0` on text block — prevents overflow
- `block` on hint — forces new line

### Change 3 & 4: Locale strings

**`src/lib/locales/en-US.ts`:**

```
"app.config.keepAwake": "Keep awake while working",        // was "☕ Keep awake..."
"app.config.codeIntel": "Code intelligence",                // was "🧠 Code..."
"app.config.autoCommitPlan": "Auto-commit plan on finalize" // was "📋 Auto-commit..."
```

**`src/lib/locales/pt-BR.ts`:**

```
"app.config.keepAwake": "Manter acordado enquanto trabalha",           // was "☕ Manter..."
"app.config.codeIntel": "Inteligência de código",                      // was "🧠 Inteligência..."
"app.config.autoCommitPlan": "Auto-commitar plano ao finalizar",       // was "📋 Auto-commitar..."
```

Hints remain unchanged (they never had emojis).

### Design system alignment

Per impeccable skill:
- **No side-stripe borders** rule: the `border-l-2` is subtle (2px accent) and functional (checked indicator), not decorative — it's the primary affordance. Contextually this is a state indicator, not a decorative stripe.
- **4pt spacing**: `gap-3` (12px), `pl-3` (12px), `py-2` (8px) — all on the 4pt scale
- **Typography**: consistent with existing `text-sm font-medium` + `text-[11px] text-ink-faint` pattern already used in the settings panel
- **Icons**: `text-ink-faint` so they don't compete with the label text

Per design-taste-frontend:
- **No gradient text, no purple/blue glow** — clean monochrome treatment
- **One accent color** — left border uses the existing `text-accent` token

### Risks

| Risk | Mitigation |
|------|-----------|
| Coffee cup icon looks different from other icons (pixel vs stroke) | Brain is pixel fill, notebook-pen is stroke — icons are already mixed-style in this codebase, so a third style fits |
| `has-[:checked]` not supported in older Tauri webviews | Tauri 2.x uses latest WebKit, `:has()` is well-supported |
| Border transition might feel heavy | Using `transition-colors` (color only), lightweight |

### Verification

1. **Build**: `npm run build` — must succeed
2. **Visual**: Open settings → verify all three cards render with correct icons, left border highlights on check/hover
3. **Functionality**: Toggle each checkbox → verify border state changes, verify config signal updates
4. **Locale**: Switch to pt-BR → verify labels show without emoji prefixes
5. **Dark/light theme**: Toggle theme → verify icon colors adapt via `currentColor`

### Tasks summary

1. Add `coffee-cup` icon to `PATHS` and `STROKE_ICONS` in `src/components/Icon.tsx`
2. Strip emoji prefixes from `app.config.keepAwake`, `app.config.codeIntel`, `app.config.autoCommitPlan` in `src/lib/locales/en-US.ts`
3. Strip emoji prefixes from same keys in `src/lib/locales/pt-BR.ts`
4. Replace the three checkbox `<label>` elements in `src/App.tsx` (~lines 1191–1225) with the new card-style layout
5. Build and verify visually
