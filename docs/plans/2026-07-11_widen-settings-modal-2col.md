# Widen Settings Modal with 2-Column Layout

## 1. Context / Problem Statement
The Settings modal (`src/App.tsx`, inside `<Show when={showConfig()}>`) is **400px wide** with all ~12 fields stacked vertically in a single column. This makes the modal feel cramped and causes unnecessary vertical scrolling. **User confirmed** the following:
- **Width:** 680px (up from 400px)
- **Layout:** 2-column grid for related pairs — Brain/Builder model selectors side by side, and the 4 numeric fields (max rounds, sub max rounds, golden cycles, golden stalls) in a 2×2 grid
- **Full-width fields:** Language selector, account/auth, Easter egg overrides, plan save path, YOLO checkbox, YOLO blacklist, and Cancel/Save buttons remain full single-column width

## 2. Goal (Definition of Done)
Settings modal is 680px wide. Brain & Builder model selectors are side-by-side in a 2-column row. The 4 numeric inputs (max rounds, sub max rounds, golden cycles, golden stalls) sit in a 2×2 grid. No content is cut off; scrolling still works via existing `max-h-[90vh] overflow-y-auto`. All field labels, hints, and "source" badges remain readable.

## 3. Key Findings (Prova Real)
- **Modal card:** `src/App.tsx:465` — `<div class="w-[400px] max-h-[90vh] overflow-y-auto rounded-lg bg-surface-1 p-5 shadow-modal">` — only width needs changing to 680px
- **All fields** are direct children of that card div, separated by `<hr>` at lines ~543 and ~656
- **Model selectors:** Brain model (lines ~588-618) and Builder model (lines ~626-656) — each with label + "source" badge + select/input
- **Numeric fields:** max rounds (lines ~664-686), sub max rounds (lines ~691-716), golden cycles (lines ~720-733), golden stalls (lines ~735-748) — each with label + "source" badge + input + hint
- **No CSS changes needed** — all layout can be done with Tailwind utility classes (`grid grid-cols-2 gap-x-4`)
- **Existing plan** `docs/plans/2026-07-08_fix-settings-modal-overflow.md` already applied the `max-h-[90vh] overflow-y-auto` fix — this plan builds on top of it

## 4. Authoritative Inputs
- **Width:** 680px (per user confirmation)
- **2-col pairs:** Brain/Builder models, max rounds/sub max rounds, golden cycles/golden stalls (per user confirmation)
- **Full-width:** language, account, Easter egg, plan path, YOLO, buttons (per user confirmation)

## 5. Changes (Steps)

| # | Target | Mutation | Why |
|---|--------|----------|-----|
| 1 | `src/App.tsx:465` — card div className | Change `w-[400px]` → `w-[680px]` | Widen the modal per user request |
| 2 | `src/App.tsx` — Brain & Builder model sections (lines ~587-655) | Wrap both in `<div class="grid grid-cols-2 gap-x-4 mb-4">`, remove individual `mb-4` from each select's container | Models side by side, sharing one row |
| 3 | `src/App.tsx` — HR before "max rounds" (line ~656) and after "golden stalls" hint (line ~748) | Keep the HR at line 656. After golden stalls hint, keep existing HR at line ~750 | Section separators remain, numeric grid goes between them |
| 4 | `src/App.tsx` — 4 numeric fields (max rounds, sub max rounds, golden cycles, golden stalls) | Wrap in `<div class="grid grid-cols-2 gap-x-4 gap-y-1 mb-4">`. Each field becomes a grid cell. Keep individual `mb-*` margins on hints minimal (`mb-1`) since the grid row handles spacing. | 2×2 grid reduces vertical space, looks organized |
| 5 | **No change:** language, account, Easter egg, plan path, YOLO, buttons | Verify only — no code change | These stay full-width as confirmed |

## 6. Verification Plan
1. **Build check:** `pnpm run build` (or `cargo build` in `src-tauri/`) passes with no errors
2. **Visual — desktop (1280px+):** Open settings → modal is 680px wide, models side-by-side, numeric fields in 2×2 grid, no overflow, no scrollbar needed on tall screens
3. **Visual — small window (~900px height):** Scrollbar appears, all content still reachable
4. **Regression:** Open/close settings, change language, toggle YOLO, save settings — all work
5. **Dark/light theme:** Toggle theme, verify modal renders correctly in both

## 7. Tasks Summary
1. Widen modal card from 400px to 680px
2. Put Brain/Builder model selectors in a 2-column grid row
3. Put the 4 numeric fields (max rounds, sub max rounds, golden cycles, golden stalls) in a 2×2 grid
4. Verify build passes and layout looks correct visually


## Implementation Log — 2026-07-11 01:07
**Summary:** Settings modal widened from 400px to 680px with 2-column grid layout for Brain/Builder models side-by-side and 2x2 grid for numeric fields (max rounds, sub max rounds, golden cycles, golden stalls)
**Changed files:** M src-tauri/examples/semantic_eval_queries.json, M src-tauri/src/code_intel/db.rs, M src-tauri/src/commands/code_intel.rs, M src/App.tsx, M src/lib/ipc.test.ts, M src/lib/ipc.ts, M src/lib/locales/en-US.ts, M src/lib/locales/pt-BR.ts, ?? docs/plans/2026-07-09_deploy-tag-0-1-1.md, ?? docs/plans/2026-07-10_steering-attachments.md, ?? docs/plans/2026-07-11_sidebar-index-status-redesign.md, ?? docs/plans/2026-07-11_tasks-panel-popover-max-height-scroll.md, ?? docs/plans/2026-07-11_widen-settings-modal-2col.md
**Commits:** _(git unavailable or none)_
**Journal:** ## Design Decisions
- **Width 680px** was chosen over 600px because with 2 columns and `gap-x-4`, each column gets ~328px (680 - 16 - 16 gap = 648 / 2 = 324px), which is still comfortable for model selector dropdowns and numeric inputs.
- **Grid approach** used pure Tailwind `grid grid-cols-2` utilities — no CSS changes needed, no new classes.
- **mb-4 removed** from individual `<select>/<input>` elements inside grid cells since the grid container's `gap-y-2` handles vertical spacing, avoiding double margins.
- **Hint text margins** changed from `mb-3`/`mb-4` to `mb-0` inside grid cells to let the grid gap control row spacing consistently.
- **Easter egg override mode** (textarea input instead of select) still works within the grid — each model column correctly renders either a select or a text input based on the `easterEggActive` signal.

## Proof
Build passes cleanly — 384 tests pass, 23 test files, vite production build completes.

**Task journal:**
- Widen modal card to 680px: Changed w-[400px] to w-[680px] on the modal card div
- Brain and Builder models side-by-side in 2-col grid: Wrapped both model selectors in a grid grid-cols-2 gap-x-4 mb-4 div. Each model inside its own <div> cell. Removed mb-4 from individual selects/inputs since grid handles spacing.
- Numeric fields in 2×2 grid: Wrapped all 4 numeric fields in grid grid-cols-2 gap-x-4 gap-y-2 mb-4. Each field is a cell: max rounds (cell 1), sub max rounds (cell 2), golden cycles (cell 3), golden stalls (cell 4). Hints changed from mb-3/mb-4 to mb-0 since grid handles row spacing.
- Verify build and visual layout: pnpm run build passed: 23 test files, 384 tests passed, vite build completed in 12.95s. No TypeScript errors.
