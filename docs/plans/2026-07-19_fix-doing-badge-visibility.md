# Fix "Doing" badge visibility in light themes

## Context
The task status badge for "doing" uses `bg-white/15 text-white` which renders white text on a semi-transparent white background. This only works against dark backgrounds. In light themes (beige/light card backgrounds), the badge is nearly invisible.

**User confirmed:** amber-500 color (`bg-amber-500/15 text-amber-500`), consistent with other in-progress states in the app (e.g. CommitPushModal "interrupted", golden task badge).

## Solution Design
Change the "doing" badge classes in `TasksPanel.tsx` from `bg-white/15 text-white` to `bg-amber-500/15 text-amber-500`.

Also fix the dot indicator on line 144 which uses `bg-white` for the "doing" dot — change to `bg-amber-500`.

## Risks
- Low risk. Only visual CSS class change, no behavior impact.

## Non-goals
- Not modifying any other status colors (done, todo remain unchanged).
- Not touching other components' badges.

## Low-Level Design

**File:** `src/components/TasksPanel.tsx`

**Change 1 — Badge (line 196):**
- From: `"bg-white/15 text-white": task.status === "doing",`
- To: `"bg-amber-500/15 text-amber-500": task.status === "doing",`

**Change 2 — Dot indicator (line 66):**
- From: `if (s === "doing") return "bg-white";`
- To: `if (s === "doing") return "bg-amber-500";`

**Change 3 — Dot in summary (line 144):**
- From: `class="inline-block h-2 w-2 rounded-full bg-white"`
- To: `class="inline-block h-2 w-2 rounded-full bg-amber-500"`

**Pattern reference:** The golden task badge on line 172 already uses `bg-amber-500/10 text-amber-500`. The interruption badge in CommitPushModal uses `bg-amber-500/15 text-amber-500`. This fix aligns the "doing" badge with that convention.

## Tasks
1. Fix "doing" badge class on line 196
2. Fix "doing" dot color function on line 66
3. Fix "doing" dot in summary on line 144
4. Verify visually in light theme


## Implementation Log — 2026-07-19 09:45
**Summary:** Fix "Doing" badge invisible in light themes — changed white to amber-500
**Changed files:** A	docs/plans/2026-07-19_fix-doing-badge-visibility.md
**Commits:** 7ac0c07 docs(plan): fix-doing-badge-visibility
**Journal:** 3 one-line CSS class changes in TasksPanel.tsx, swapping "doing" status indicators from white to amber-500. White-on-transparent-white was invisible in light themes. Amber-500 matches existing conventions (golden task badge, CommitPushModal interrupted state). TypeScript check confirms no new errors — only pre-existing issues in unrelated files.

**Task journal:**
- Fix doing status badge class: Changed line 196: bg-white/15 text-white → bg-amber-500/15 text-amber-500
- Fix doing dot color function: Changed line 66: bg-white → bg-amber-500 in dotColor function
- Fix doing dot in summary area: Changed line 144: bg-white → bg-amber-500 in summary legend dot
- Verify badge visibility in light theme: All 3 lines verified in source. TypeScript check shows pre-existing errors in other files — no new errors from our change. Amber-500 on amber-500/15 provides strong contrast on both light and dark themes.
