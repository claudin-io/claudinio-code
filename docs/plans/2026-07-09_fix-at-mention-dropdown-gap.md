# Plan: Fix @ mention dropdown floating gap

## 1. Context / Problem Statement

When the `@` file mention autocomplete has few results (1-3 files), the dropdown floats with a visible gap above the input instead of sitting snugly against it.

**Root cause**: `FileMentionPopover` uses a hardcoded `POPOVER_ESTIMATED_HEIGHT = 260` in its `top`-based position calculation. When results are few (actual popover height ~80px), the popover is still placed 260px above the caret — creating a ~180px gap.

**Confirmed by user**: Follow the existing plan — switch from `top` to `bottom`-based positioning.

## 2. Goal (Definition of Done)

The `@` file mention dropdown always sits directly above the input caret (4px gap) regardless of how many results are shown. No floating gap.

## 3. Key Findings (Prova Real)

| Finding | Method | Proof |
|---------|--------|-------|
| Position calculator uses `POPOVER_ESTIMATED_HEIGHT = 260` for `top` positioning | Read `ChatPanel.tsx:1754` | `const POPOVER_ESTIMATED_HEIGHT = 260;` |
| `FileMentionPopover` uses `top` CSS, not `bottom` | Read `FileMentionPopover.tsx:89-92` | `top: ${props.position.top}px` |
| `TagMentionPopover` and `SkillMentionPopover` already use `bottom` positioning with no gap issues | Investigated by subagent | `bottom: ${props.bottom}px` pattern works |
| Position signal type is `{ top: number; left: number; height: number }` | Read `ChatPanel.tsx:448` | `createSignal<{ top: number; left: number; height: number } \| null>(null)` |
| Test fixture uses old shape | Read `FileMentionPopover.test.tsx:14` | `const defaultPosition = { top: 100, left: 200, height: 20 };` |

## 4. Authoritative Inputs

| Input | Value | Source |
|-------|-------|--------|
| Bottom formula | `window.innerHeight - pos.top + 4` | Existing pattern from TagMentionPopover/SkillMentionPopover position calc in `ChatPanel.tsx` |
| FileMentionPopover props shape | `{ bottom: number; left: number }` | Pattern from `TagMentionPopoverProps` (`TagMentionPopover.tsx`) |

## 5. Changes (Steps)

### Change 1: Update `FileMentionPopover` interface and CSS
- **Target**: `src/components/FileMentionPopover.tsx`
- **Mutation**: 
  - Line 7: Change `position: { top: number; left: number; height: number }` → `position: { bottom: number; left: number }`
  - Line 90: Change `top: ${props.position.top}px` → `bottom: ${props.position.bottom}px`
- **Why**: Switch from fixed-top positioning to natural bottom-anchored positioning, so the popover grows upward from its actual height.

### Change 2: Update position signal type in `ChatPanel.tsx`
- **Target**: `src/components/ChatPanel.tsx`, line 448
- **Mutation**: Change `createSignal<{ top: number; left: number; height: number } | null>(null)` → `createSignal<{ bottom: number; left: number } | null>(null)`
- **Why**: Match the new `FileMentionPopover` props shape.

### Change 3: Update position calculation in `ChatPanel.tsx`
- **Target**: `src/components/ChatPanel.tsx`, lines 1750-1769
- **Mutation**: 
  - Remove `POPOVER_ESTIMATED_HEIGHT = 260` constant
  - Remove the `if (room above) / else (show below)` flip logic
  - Replace with single `bottom = window.innerHeight - pos.top + 4` formula
  - Change `setMentionPosition({ top, left, height: pos.height })` → `setMentionPosition({ bottom, left })`
- **Why**: Align with the established `bottom`-based pattern. The input is always near the bottom of the viewport so no flip logic is needed.

### Change 4: Update test fixture
- **Target**: `src/components/FileMentionPopover.test.tsx`, line 14
- **Mutation**: Change `const defaultPosition = { top: 100, left: 200, height: 20 }` → `const defaultPosition = { bottom: 100, left: 200 }`
- **Why**: Match the new interface shape so tests compile and pass.

## 6. Verification Plan

| Step | Command | Expected Result |
|------|---------|-----------------|
| Tests pass | `npx vitest run src/components/FileMentionPopover.test.tsx` | All 13 tests pass |
| TypeScript check | `npx tsc --noEmit` | No type errors |
| Visual | Open app, type `@` in chat input with a workspace loaded | Popover appears directly above caret (4px gap), no floating gap with few results |

## 7. Risks

- **Low risk**: `FileMentionPopover` is only used in one place (`ChatPanel.tsx`). All references are statically checkable via TypeScript.
- **No risk to**: `TagMentionPopover` and `SkillMentionPopover` — they are NOT modified.
