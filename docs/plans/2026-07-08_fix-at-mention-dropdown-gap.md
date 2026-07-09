# Plan: Fix @ mention dropdown floating gap

## 1. Context / Problem Statement

When the `@` file mention autocomplete has few results (1-3 files), the dropdown floats with a visible gap above the input instead of sitting snugly against it.

**Root cause**: `FileMentionPopover` uses a hardcoded `POPOVER_ESTIMATED_HEIGHT = 260` in its `top`-based position calculation. When results are few (actual popover height ~80px), the popover is still placed 260px above the caret — creating a ~180px gap.

## 2. Goal (Definition of Done)

The `@` file mention dropdown always sits directly above the input caret (4px gap) regardless of how many results are shown. No floating gap.

## 3. Key Findings (Prova Real)

- **Finding**: Position calculator uses `POPOVER_ESTIMATED_HEIGHT = 260` for `top` positioning.
  - **Method**: `grep -n "POPOVER_ESTIMATED_HEIGHT" src/components/ChatPanel.tsx` → line 1630
  - **Proof**: `const POPOVER_ESTIMATED_HEIGHT = 260;` at `ChatPanel.tsx:1630`

- **Finding**: `FileMentionPopover` uses `top` CSS, not `bottom`.
  - **Method**: `grep -n "top:" src/components/FileMentionPopover.tsx` → line 90
  - **Proof**: `top: ${props.position.top}px` at `FileMentionPopover.tsx:90`

- **Finding**: `TagMentionPopover` and `SkillMentionPopover` already use `bottom` positioning with no issues.
  - **Method**: Read `TagMentionPopover.tsx:91-92`, `SkillMentionPopover.tsx` style section
  - **Proof**: `bottom: ${props.bottom}px` — this pattern works for the same UI context.

- **Finding**: Position signal type is `{ top: number; left: number; height: number }`.
  - **Method**: `grep -n "mentionPosition" src/components/ChatPanel.tsx` → line 440
  - **Proof**: `createSignal<{ top: number; left: number; height: number } | null>(null)` at `ChatPanel.tsx:440`

- **Finding**: Test fixture uses old shape.
  - **Method**: `grep -n "defaultPosition" src/components/FileMentionPopover.test.tsx` → line 14
  - **Proof**: `const defaultPosition = { top: 100, left: 200, height: 20 };` at `FileMentionPopover.test.tsx:14`

## 4. Authoritative Inputs

| Input | Value | Source |
|-------|-------|--------|
| Bottom formula | `window.innerHeight - pos.top + 4` | Existing pattern in `TagMentionPopover` / `SkillMentionPopover` position calc (`ChatPanel.tsx:1675`) |
| FileMentionPopover props shape | `{ bottom: number; left: number }` | Pattern from `TagMentionPopoverProps` (`TagMentionPopover.tsx:21`) |

## 5. Changes (Steps)

### Change 1: Update `FileMentionPopover` interface and CSS
- **Target**: `src/components/FileMentionPopover.tsx`
- **Mutation**: 
  - Line 7: Change `position: { top: number; left: number; height: number }` → `position: { bottom: number; left: number }`
  - Line 90: Change `top: ${props.position.top}px` → `bottom: ${props.position.bottom}px`
- **Why**: Switch from fixed-top positioning to natural bottom-anchored positioning, allowing the popover to grow upward to match its actual height.
- **Constraints**: No change to component behavior, styling, or rendering besides the positioning axis.

### Change 2: Update position signal type in `ChatPanel.tsx`
- **Target**: `src/components/ChatPanel.tsx`, line 440
- **Mutation**: Change `createSignal<{ top: number; left: number; height: number } | null>(null)` → `createSignal<{ bottom: number; left: number } | null>(null)`
- **Why**: Match the new `FileMentionPopover` props shape.

### Change 3: Update position calculation in `ChatPanel.tsx`
- **Target**: `src/components/ChatPanel.tsx`, lines 1628-1650
- **Mutation**: 
  - Remove `POPOVER_ESTIMATED_HEIGHT = 260` constant
  - Remove the `if (top available) / else (show below)` flip logic
  - Replace with single `bottom = window.innerHeight - pos.top + 4` formula
  - Change `setMentionPosition({ top, left, height: pos.height })` → `setMentionPosition({ bottom, left })`
- **Why**: Align with the established `bottom`-based pattern used by tag/skill popovers. The input is always at the bottom of the viewport so no flip logic is needed.
- **Constraints**: Must not affect the tag/skill popover position calculation.

### Change 4: Update test fixture
- **Target**: `src/components/FileMentionPopover.test.tsx`, line 14
- **Mutation**: Change `const defaultPosition = { top: 100, left: 200, height: 20 }` → `const defaultPosition = { bottom: 100, left: 200 }`
- **Why**: Match the new interface shape so tests compile and pass.

## 6. Verification Plan

| Step | Command | Expected Result |
|------|---------|-----------------|
| Tests pass | `npm test -- --run FileMentionPopover` | All FileMentionPopover tests pass (13 tests) |
| Type check | `npx tsc --noEmit` (or equivalent) | No type errors |
| Build | `npm run build` or `cargo tauri build` | Build succeeds |
| Visual | Open app, type `@` in chat input with workspace that has files | Popover appears directly above caret (4px gap), no floating gap when results are few |

## 7. Risks

- **Low risk**: `FileMentionPopover` is only used in one place (`ChatPanel.tsx:1768`). All references are statically checkable.
- **No risk**: `TagMentionPopover` and `SkillMentionPopover` are NOT modified — only `FileMentionPopover` changes.
