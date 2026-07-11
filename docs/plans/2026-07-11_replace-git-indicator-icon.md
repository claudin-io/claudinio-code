# Replace GitIndicator Icon: git-branch → diff (codicon:diff-single)

## 1. Context / Problem Statement

**CONFIRMED by user**: Replace the `git-branch` icon in the Git indicator badge with the `codicon:diff-single` icon from https://icones.js.org/collection/all?s=diff&icon=codicon:diff-single.

**Key finding (Prova Real)**: The `codicon:diff-single` SVG path data is **identical** to the `diff` icon already defined in the project at `src/components/Icon.tsx:150-155`. Therefore, no new icon needs to be created — we simply reuse the existing `diff` icon.

**Current state**: `GitIndicator.tsx:93` uses `<Icon name="git-branch" class="h-3.5 w-3.5" />`.

**Target state**: `GitIndicator.tsx:93` uses `<Icon name="diff" class="h-3.5 w-3.5" />`.

## 2. Goal (Definition of Done)

- The Git indicator badge in the status bar shows the `diff` icon (file with +/- indicator) instead of the `git-branch` icon (circles with curve).
- Nothing else changes — badge text, click behavior, modal, and all other icons remain unchanged.

## 3. Key Findings (Prova Real)

| Finding | Method | Proof |
|---|---|---|
| `codicon:diff-single` SVG path is identical to existing `diff` icon | Fetched `https://api.iconify.design/codicon/diff-single.svg` and compared to `Icon.tsx:150-155` `diff` entry | Both have same 4-path structure: file outline + inner + plus-badge + minus-badge. ViewBox is 16×16 in both. |
| Current icon is `git-branch` at `GitIndicator.tsx:93` | Read the file and found `<Icon name="git-branch" class="h-3.5 w-3.5" />` | grep across codebase confirms `git-branch` is only used in this one location |
| `diff` icon already has `viewBox: "0 0 16 16"` in `VIEWBOX` map | Read `Icon.tsx:165` (`diff: "0 0 16 16"`) | The viewBox override exists and is correct for this codicon glyph |
| `diff` is already used in `GitChangesModal.tsx:185` | Read the component | The icon renders correctly in the modal as a filled icon |

## 4. Authoritative Inputs

| Input | Source | Value |
|---|---|---|
| Icon name to use | User (via interview) | `diff` (existing icon, confirmed identical to `codicon:diff-single`) |
| Target file | Codebase | `src/components/GitIndicator.tsx` |
| Target line | Codebase | Line 93: `<Icon name="git-branch" class="h-3.5 w-3.5" />` |

## 5. Changes (Steps)

### Change 1: Replace icon name in GitIndicator.tsx

| Field | Value |
|---|---|
| **Target** | `src/components/GitIndicator.tsx`, line 93 |
| **Mutation** | Change `name="git-branch"` to `name="diff"` |
| **Why** | User requested replacement with `codicon:diff-single`, which is identical to the existing `diff` icon |
| **Does NOT change** | CSS classes (`h-3.5 w-3.5`), all other props, surrounding JSX, click handler, label, all other components |
| **Idempotent** | Yes — running again is a no-op |

### What must NOT change (verify-only)

- `Icon.tsx` — no new icon definitions needed
- `GitChangesModal.tsx` — already uses `diff`, unchanged
- `CommitPushModal.tsx` — uses `git-commit`, unchanged
- All other `<Icon>` usages — untouched
- i18n strings — unchanged
- `git-branch` icon definition in `Icon.tsx` — kept available for potential future use

## 6. Verification Plan

1. **Build check**: Run `pnpm build` (or `pnpm vite build`) — must succeed with no TypeScript errors.
2. **Test suite**: Run `pnpm test` (or `pnpm vitest run`) — all existing tests must pass.
3. **Visual verification**: Since this is a visual change visible in the app status bar, after building, the `diff` icon (file with +/-) should appear instead of the `git-branch` icon (circles with curve). This requires running the app via `pnpm tauri dev`.
4. **No regressions**: Verify `GitChangesModal` still shows the `diff` icon correctly (it uses the same icon in its header).
5. **Code diff**: `git diff` should show exactly one line changed in `src/components/GitIndicator.tsx`.

## 7. Tasks Summary

- 1 task: Replace `git-branch` → `diff` in `GitIndicator.tsx` line 93, then verify build + tests pass.


## Implementation Log — 2026-07-11 11:33
**Summary:** Replace git-branch icon with diff (codicon:diff-single) in GitIndicator badge
**Changed files:** M src/components/CommitPushModal.tsx, M src/components/GitChangesModal.tsx, M src/components/GitIndicator.tsx, M src/components/Icon.tsx, ?? docs/plans/2026-07-11_replace-git-indicator-icon.md
**Commits:** _(git unavailable or none)_
**Journal:** 1. Key finding: codicon:diff-single from the user's URL is IDENTICAL to the existing 'diff' icon already defined in Icon.tsx:150-155 (same 4-path structure, same 16x16 viewBox). No new icon definition needed. 2. Only one line changed: GitIndicator.tsx:93 — name="git-branch" → name="diff". 3. Verified: 384 tests pass (23 files), production build succeeds without warnings. 4. The git-branch icon definition remains in Icon.tsx for any future use — it was only replaced at the usage site.

**Task journal:**
- Replace git-branch icon with diff in GitIndicator: Found that codicon:diff-single SVG is identical to the existing 'diff' icon in Icon.tsx:150-155; No new icon needed — just reused the existing 'diff' icon; Changed name="git-branch" → name="diff" on line 93 of GitIndicator.tsx; 384 tests pass, build succeeds
