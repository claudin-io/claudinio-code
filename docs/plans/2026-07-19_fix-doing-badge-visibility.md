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
