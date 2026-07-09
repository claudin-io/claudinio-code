# Plan: Change pensador icon to codicon:thinking

## Context
The user wants to replace the pixel-art `thinking-face` icon used for the Pensador mode button with the `codicon:thinking` icon from [icones.js.org](https://icones.js.org/collection/all?s=think&icon=codicon:thinking).

## Current State
- The pensador icon is defined as `"thinking-face"` in `src/components/Icon.tsx` inside the `PATHS` object (line 66-68), with pixel-art SVG path data.
- The icon is referenced in `src/components/ChatPanel.tsx` at two locations:
  - **Line 1295**: Mode toggle button (`<Icon name="thinking-face" class="h-4 w-4" />`)
  - **Line 1652**: Mode change timeline label (`name={step.modeChange!.mode === "pensador" ? "thinking-face" : "construction-worker"}`)
- There are also two comment/string references in `Icon.tsx` (line 65) and the `IconName` type automatically picks up the key.

## Solution Design

### Approach
**Minimal approach**: Replace the SVG path data for the existing `"thinking-face"` key in `PATHS` with the paths from `codicon:thinking`. This avoids renaming and touching all references.

**Rationale**: Only 1 file (Icon.tsx) needs a data change. Zero reference updates needed in ChatPanel.tsx. The `IconName` type is derived from `keyof typeof PATHS` so it auto-updates.

### Files to change
| File | Change |
|------|--------|
| `src/components/Icon.tsx` | Replace the SVG path array for `"thinking-face"` with the `codicon:thinking` paths. Also update the comment above it. |

### Steps
1. **Fetch `codicon:thinking` SVG data** from `https://api.iconify.design/codicon/thinking.json` — the response contains `{ body: "...", width: 16/24 }` with the SVG path data.
2. **Parse the SVG body** — extract the `<path d="..." />` elements (or the single `d` attribute) from the icon data.
3. **Update `Icon.tsx`** — replace the paths array for the `"thinking-face"` key with the extracted codicon path(s). Update the comment from `// pixel take on fluent-emoji-high-contrast:thinking-face` to `// codicon:thinking`.
4. **Build verification** — run `cargo tauri dev` or `npm run build` to confirm no TypeScript errors.

### Risks
- None. This is a pure data replacement — the icon name stays the same, the `IconName` type is unaffected, and all existing references continue to work.

### Tasks
1. Fetch codicon:thinking SVG data (curl the Iconify API)
2. Update Icon.tsx with the new SVG paths
3. Verify the build compiles
