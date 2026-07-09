# Plan: Fix ToastPill Auto-Dismiss Bug

## Context / Problem Statement

The "File attached" toast (`ToastPill`) appears when a file is attached but never auto-dismisses after 2 seconds. The toast remains visible indefinitely.

## Root Cause

`ToastPill` uses Solid's `onMount()` lifecycle hook to schedule the 2-second dismiss timeout. However:
- `onMount` runs **exactly once** — when the component is first inserted into the DOM.
- The `ToastPill` is rendered **always** in `ChatPanel` (never conditionally mounted), only hidden via CSS `opacity-0` when `message` is `null`.
- When `message` transitions from `null` (hidden) → `"File attached"` (visible), the component **does not remount**, so `onMount` does not fire again.
- Result: **the dismiss timeout is never scheduled**, and the toast stays visible forever.

The code itself acknowledges this deficiency with a comment:
```
// Using a createEffect would be better, but we keep it simple
// The parent ChatPanel re-mounts this on message change via key or conditional
```

The parent's assumption ("re-mounts this on message change") is incorrect — there is no `key` binding on `<ToastPill>`, so Solid reuses the same component instance.

## Solution Design

Replace `onMount` + manual `scheduleDismiss` call with Solid's `createEffect`, which automatically re-runs whenever `props.message` changes.

### Changes

**File: `/Users/victortavernari/claudinio_code/src/components/ToastPill.tsx`**

1. **Import `createEffect`** alongside `onMount`, `onCleanup`.
2. **Replace `onMount` block** with a `createEffect` that:
   - Clears any existing timeout.
   - If `props.message` is truthy, schedules a new 2s timeout to call `props.onDismiss()`.
   - If `props.message` is null/falsy, does nothing (already hidden).
3. **Remove unused `scheduleDismiss` function** (its logic moves into the effect).
4. **`onCleanup` stays** — it clears the timeout on unmount.

### Key behaviors preserved:
- Timeout is **re-scheduled** every time `message` changes from one truthy value to another.
- Timeout is **cancelled** if `message` becomes null before 2s (e.g. user dismisses manually).
- Timer still fires `onDismiss` exactly once per toast appearance.

## Risks

- **Low.** This is a minimal, targeted change — replacing one Solid primitive (`onMount`) with another (`createEffect`). No behavioral changes beyond the fix.
- `createEffect` runs synchronously after the first render (so it fires on mount too, same as `onMount`), and then on every reactive dependency change.

## Verification Plan

1. **Build check:** `cargo tauri build` or at minimum check that TypeScript compiles.
2. **Manual test:** Attach a file → confirm "File attached" toast appears → confirm it auto-dismisses after ~2 seconds.
3. **Edge case:** Set `message` → quickly dismiss manually (`onDismiss`) → confirm no stray timeout callback fires after.

## Task Summary

| # | Task | File |
|---|------|------|
| 1 | Refactor ToastPill: replace onMount with createEffect for auto-dismiss scheduling | `src/components/ToastPill.tsx` |


## Implementation Log — 2026-07-09 23:07
**Summary:** Fix ToastPill auto-dismiss: replace onMount with createEffect for reactive 2s timeout scheduling
**Changed files:** M	src-tauri/src/agent/provider.rs, M	src-tauri/src/agent/session.rs, M	src-tauri/src/agent/subagent.rs, M	src-tauri/src/agent/tools/mod.rs
**Commits:** 78f933b revert: restore English AI prompts and tool descriptions
**Journal:** Fixed the "File attached" toast never auto-dismissing. Root cause: ToastPill used onMount to schedule a 2s dismiss timeout, but onMount only fires once when the component first mounts. Since ChatPanel renders <ToastPill> unconditionally (no key, always in DOM), when message transitions from null→"File attached" the component doesn't remount and onMount never re-fires. Fix: replaced onMount + scheduleDismiss with createEffect, which reactively tracks props.message and schedules the 2s timeout every time the message becomes truthy. The import was switched from { onMount, onCleanup } to { createEffect, onCleanup }. The stale comment block acknowledging the deficiency was also removed.

**Task journal:**
- Fix ToastPill: replace onMount with createEffect for auto-dismiss: Root cause: onMount runs once on mount, but the parent keeps <ToastPill> always rendered (no key, never conditional) — onMount never re-fires when message goes null→"File attached"; Fix: imported createEffect, removed onMount and scheduleDismiss, replaced with createEffect that clears previous timeout and schedules a new 2s timeout when props.message is truthy; Removed the stale comment block that acknowledged the deficiency; Verified — no TypeScript errors in ToastPill.tsx
