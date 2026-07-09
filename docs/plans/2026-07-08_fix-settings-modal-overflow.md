# Fix Settings Modal Overflow

## 1. Context / Problem Statement
The Settings modal (`src/App.tsx` lines 316-498) has no height constraint and no internal scrolling. On shorter screens (laptops, small monitors), the content overflows past the viewport bottom, cutting off the "YOLO Blacklist" field and the Cancel/Save buttons. **User confirmed** the Simple approach: add `max-h-[90vh] overflow-y-auto` to the card.

## 2. Goal (Definition of Done)
Settings modal card is constrained to 90% of viewport height and scrolls internally when content exceeds that limit. All fields remain accessible on any screen height.

## 3. Key Findings (Prova Real)
- **Card div** at `src/App.tsx:318`: `<div class="w-[400px] rounded-lg bg-surface-1 p-5 shadow-modal">` — missing `max-h-*` and `overflow-y-auto`
- **Overlay** (`fixed inset-0 flex items-center justify-center`) already centers vertically — no change needed here
- **App.css** — no modal-specific height rules exist; no collateral to adjust

## 4. Authoritative Inputs
- Card location: `src/App.tsx`, line ~318 (the `<div>` right inside `<Show when={showConfig()}>`)
- Fix per user: `max-h-[90vh] overflow-y-auto` (Recommended Simple approach)

## 5. Changes (Steps)

| # | Target | Mutation | Why |
|---|--------|----------|-----|
| 1 | `src/App.tsx` line ~318, the card `<div>` className | Add `max-h-[90vh] overflow-y-auto` to existing classes | Constrains modal to 90vh, enables scroll for overflow |

**Classes before:** `w-[400px] rounded-lg bg-surface-1 p-5 shadow-modal`
**Classes after:** `w-[400px] max-h-[90vh] overflow-y-auto rounded-lg bg-surface-1 p-5 shadow-modal`

**No other files affected.** No new dependencies. No structural changes.

## 6. Verification Plan
1. **Checkout** current branch is not `main` (or create a feature branch)
2. **Apply** the one-line change to `src/App.tsx`
3. **Build check** — `cargo check` or the project's build command passes
4. **Visual** — open Settings on a small window (~768px height), confirm:
   - Modal stays within viewport (no cutoff)
   - Vertical scrollbar appears and works
   - Content at bottom (Save/Cancel) is reachable by scrolling
   - On large screens, no scrollbar appears (modal fits)
5. **Regression** — open/close Settings, verify overlay click-to-dismiss works, save settings works

## 7. Tasks Summary
1. Apply one-line CSS fix: `max-h-[90vh] overflow-y-auto` on the settings card div
2. Verify build passes and settings modal scrolls correctly
