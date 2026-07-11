# Plan: Replace prompt enhancer icon & remove from inline input

## Context / Problem Statement

The user wants two changes to the "prompt enhancer" feature:

1. **Replace the icon** from the custom `magic-rabbit` icon to `material-symbols:magic-button-outline` (https://icones.js.org/collection/all?s=magic&icon=material-symbols:magic-button-outline) — in the only remaining location: the TextEditorModal.
2. **Remove the enhance button entirely from the inline ChatPanel toolbar** — the user confirmed it should be removed, not just hidden. The button at ChatPanel line ~1890-1908 should be deleted, keeping only the enhance button inside the TextEditorModal.

## Goal (Definition of Done)

- The inline ChatPanel input toolbar no longer has an enhance button.
- The TextEditorModal enhance button uses the new `magic-button-outline` icon (material-symbols).
- The old `magic-rabbit` icon definition is cleaned up from `Icon.tsx`.
- The project builds and TypeScript type-checks cleanly.

## Key Findings (Prova Real)

- **Icon system:** Custom inline SVG in `src/components/Icon.tsx`. All icons are defined in a `PATHS: Record<string, string[]>` object with optional `VIEWBOX` overrides. No external icon packages. _(source: `src/components/Icon.tsx`)_

- **Current enhance icon:** `"magic-rabbit"` — a custom 32×32 bunny icon defined at `Icon.tsx:88-91` with viewBox override at line 168. Only used in two places:
  - `src/components/ChatPanel.tsx` line 1905: `<Icon name="magic-rabbit" class="h-4 w-4" />`
  - `src/components/TextEditorModal.tsx` line 77: `<Icon name="magic-rabbit" class="h-4 w-4" />`

- **ChatPanel inline enhance button:** lines ~1890-1908 — a `<button>` wrapping `<Icon name="magic-rabbit">`. It calls `enhanceHandler(text)`, then `setInput(enhanced)`, then `setShowEditor(true)`. This entire button block must be removed.

- **TextEditorModal enhance button:** lines ~71-81 — a `<button>` inside a `<Show when={props.onEnhance}>` block. This is the button that will keep working but with the new icon name.

- **Target icon:** `material-symbols:magic-button-outline`. Must be fetched from `https://api.iconify.design/material-symbols/magic-button-outline.svg`, its `<path d="...">` values extracted, and registered in `PATHS`.

## Authoritative Inputs

| Input | Value | Source |
|-------|-------|--------|
| New icon ID | `material-symbols:magic-button-outline` | User |
| SVG URL | `https://api.iconify.design/material-symbols/magic-button-outline.svg` | Iconify API |
| Icon name to register | `"magic-button-outline"` | Derived (material-symbols prefix stripped) |

## Changes (Steps)

### 1. Fetch SVG and register new icon in `src/components/Icon.tsx`

- **Target:** `src/components/Icon.tsx`
- **Mutation:** Fetch the SVG from the Iconify API, extract all `<path d="...">` values, and add a new entry `"magic-button-outline"` to the `PATHS` object. Material Symbols icons use 24×24 viewBox by default, so no `VIEWBOX` override is needed.
- **Why:** This is how icons are added in this project — no external packages, all SVG paths inlined.
- **Constraints:** Preserve existing formatting (one array element per path string). Place alphabetically among existing icons.

### 2. Remove old `magic-rabbit` from `src/components/Icon.tsx`

- **Target:** `src/components/Icon.tsx`
- **Mutation:** Remove the `"magic-rabbit"` entry from `PATHS` (lines 88-91) and its viewBox override from `VIEWBOX` (line ~168).
- **Why:** Dead code cleanup — `magic-rabbit` will no longer be referenced anywhere.
- **Constraints:** TypeScript will catch any remaining references via the `IconName` type union.

### 3. Update TextEditorModal to use new icon name

- **Target:** `src/components/TextEditorModal.tsx`
- **Mutation:** Line 77: change `<Icon name="magic-rabbit" class="h-4 w-4" />` to `<Icon name="magic-button-outline" class="h-4 w-4" />`.
- **Why:** The user wants the material-symbols icon instead of the custom rabbit.
- **Constraints:** No other changes to the enhance button logic.

### 4. Remove enhance button from ChatPanel inline toolbar

- **Target:** `src/components/ChatPanel.tsx`
- **Mutation:** Delete the entire enhance `<button>` block (including the `<Show>` wrapper for loading state), approximately lines 1890-1908. This is the `<button onClick={async () => { ... }}>` with `<Icon name="magic-rabbit">` / `<Icon name="loader">`.
- **Why:** User confirmed removal of the inline enhance button. It will only exist in the TextEditorModal now.
- **Constraints:** Do not touch adjacent buttons (notebook-pen, paperclip, mode toggles). Clean up any leftover whitespace/gaps.

### 5. Verify TypeScript compilation

- **Target:** full project
- **Mutation:** Run `pnpm typecheck` (or equivalent TS check) to confirm no broken imports or missing icon references.
- **Why:** Removing `magic-rabbit` from PATHS changes the `IconName` type — any remaining references will be caught.

## Verification Plan

1. **Type-check:** Run `npx tsc --noEmit` — must exit 0.
2. **Grep for `magic-rabbit`:** `grep -r "magic-rabbit" src/` — must return empty (except possibly in git history, not in working tree).
3. **Grep for `magic-button-outline`:** Must find the definition in `Icon.tsx` and the usage in `TextEditorModal.tsx`.
4. **Visual check (manual):** The user should open the app and confirm:
   - The inline input toolbar no longer shows an enhance button.
   - Clicking the notebook-pen icon opens the TextEditorModal.
   - The TextEditorModal toolbar shows the new material-symbols magic button icon.
   - Clicking it triggers enhancement.

## Risks

- **Low risk.** Changes are scoped to icon data and a single button removal. No backend or IPC changes.
- If the SVG fetch fails, the Builder can manually extract path data from visiting `https://icones.js.org/collection/all?s=magic&icon=material-symbols:magic-button-outline` in a browser.

## Tasks Summary

1. Fetch material-symbols:magic-button-outline SVG and register in Icon.tsx
2. Remove old magic-rabbit icon from Icon.tsx
3. Update TextEditorModal.tsx to use new icon name
4. Remove enhance button from ChatPanel.tsx inline toolbar
5. Verify TypeScript compilation and grep for stale references


## Implementation Log — 2026-07-11 12:43
**Summary:** Replace prompt enhancer icon with material-symbols:magic-button-outline and remove inline button from ChatPanel toolbar
**Changed files:** M src/components/ChatPanel.tsx, M src/components/Icon.tsx, M src/components/TextEditorModal.tsx, ?? docs/plans/2026-07-11_replace-prompt-enhancer-icon.md
**Commits:** _(git unavailable or none)_
**Journal:** ## Implementation Summary

**1. Icon replacement (`Icon.tsx`):**
- Fetched the `material-symbols:magic-button-outline` SVG from Iconify API — it uses standard 24×24 viewBox, single path with 4 segments.
- Replaced the `magic-rabbit` entry (2 paths, custom 32×32 viewBox) with `magic-button-outline` (1 path).
- Removed the `magic-rabbit` viewBox override from VIEWBOX. No viewBox override needed for `magic-button-outline` since it uses the default 24×24.

**2. TextEditorModal icon update:**
- Changed `<Icon name="magic-rabbit">` to `<Icon name="magic-button-outline">` in the modal header (the only remaining enhance button location).

**3. ChatPanel inline enhance button removal:**
- Deleted the entire enhance `<button>` block (onClick with enhanceHandler → setInput → setShowEditor, plus the loading spinner Show fallback).
- The toolbar now starts directly with the notebook-pen (editor) button.
- Also fixed an unused variable warning: `isEnhancing` was no longer read, so destructured as `[, setIsEnhancing]`.

**4. Verification:**
- `npx tsc --noEmit`: zero errors in our 3 changed files.
- `grep -r 'magic-rabbit' src/`: empty — full cleanup.
- All pre-existing TS errors (test files, vitest globals, Monaco Editor types, unused vars) are untouched and unrelated.

**Key decision:** The inline button both enhanced AND opened the editor. After removal, the user still opens the editor via the notebook-pen button and enhances from within the modal.

**Task journal:**
- Register new icon and remove old magic-rabbit from Icon.tsx: Added 'magic-button-outline' to PATHS between 'layers' and 'external-link'; Removed 'magic-rabbit' entry from PATHS (2 path strings); Removed 'magic-rabbit: 0 0 32 32' from VIEWBOX
- Update TextEditorModal enhance button icon: Updated icon ref in TextEditorModal.tsx line ~77
- Remove enhance button from ChatPanel inline toolbar: Removed the inline enhance <button> block (onClick → enhanceHandler + setInput + setShowEditor); Toolbar now starts directly with notebook-pen button; Also fixed unused isEnhancing signal → replaced with [, setIsEnhancing] to avoid TS6133 error
- Verify TypeScript compilation and stale reference cleanup: npx tsc --noEmit: zero errors in our changed files (ChatPanel.tsx, TextEditorModal.tsx, Icon.tsx); grep -r 'magic-rabbit' src/: empty — 0 references; grep -r 'magic-button-outline' src/: Icon.tsx:88 (definition), TextEditorModal.tsx:77 (usage)
