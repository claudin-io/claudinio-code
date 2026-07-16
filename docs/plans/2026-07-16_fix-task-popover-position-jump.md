# Fix: Task Popover Position Jump on Chat Streaming

## Context

When the chat is streaming text, the task popover (floating card on the right sidebar) jumps out of position — sometimes to the opposite side of the screen. The card returns to the correct position as soon as the user moves the mouse.

The root cause is a **3-second polling cycle** in `TasksPanel.tsx`:

1. Every 3s, `load()` calls `getTasks(workspace)` via IPC → receives new `TaskItem[]` objects
2. `<For each={tasks()}>` in SolidJS detects **new object references**, unmounts old `<button>` elements and creates new ones
3. `hoveredElement` signal still holds a reference to the **old, removed DOM element**
4. The Popover's `createEffect` re-runs (because SolidJS re-creates the triggerRef arrow function on render), reads `triggerRef()` → old element → `getBoundingClientRect()` → returns `{0, 0, 0, 0}`
5. `computePosition` with zero rect causes the popover to render at `{margin, margin}` (clamped)
6. Moving the mouse fires `onMouseEnter` → captures the **new** current button → position corrects

## Solution Design

### Approach: Freeze rect coordinates at hover time

**Replace** the `hoveredElement` signal (live DOM reference) with a `hoveredRect` signal (captured coordinates at mouseEnter):

- **Before:** `hoveredElement` stores `e.currentTarget` (HTMLButtonElement) → passed as `triggerRef={() => hoveredElement()}` to Popover → Popover calls `.getBoundingClientRect()` in its `createEffect` every time it re-runs
- **After:** `hoveredRect` stores `{ top, left, width, height }` captured once from `e.currentTarget.getBoundingClientRect()` at mouseEnter → passed as `position={hoveredRect()}` to Popover → coordinates are frozen, polling does not affect them

### Why this works

- `hoveredRect()` returns the same object reference (signal value unchanged) → SolidJS does not re-render Popover when parent re-renders due to polling
- Popover's `createEffect` already has an `if (props.position)` branch that uses explicit coordinates
- Window resize still triggers `posVersion` → Popover re-positions using captured coordinates
- Moving the mouse to a different task → new `hoveredRect` is captured → Popover repositions correctly

### Non-goals

- No changes to `Popover.tsx` itself (it already supports `position` prop)
- No changes to the polling mechanism (3s interval is fine — the problem was only the live element reference)
- No changes to `TasksBody` (inline chat task rendering — confirmed not affected)
- No changes to the task cycle/dismiss logic

## Risks

### Resize while popover is open

If the user resizes the window while the popover is open, `posVersion` increments and the popove re-positions using the captured `hoveredRect`. The rect coordinates are viewport-relative at capture time, so after a resize the trigger button may have moved. However:
- The popover's clamp logic keeps it within viewport bounds
- The next mouseEnter on any task captures fresh coordinates
- The popover auto-closes after 150ms of no hover anyway
- **Impact: low** — cosmetic only during resize, self-corrects instantly

### Scroll while popover is open

Same underlying mechanism — captured rect may no longer match the trigger position after sidebar scroll. In practice:
- The TasksPanel sidebar is a small scrollable area (~40px wide × ~200px tall)
- Moving the mouse to scroll resets `hoveredId`/`hoveredRect`
- **Impact: very low** — extremely brief, auto-corrects

## Low-Level Design

### Files to change

| File | Change |
|---|---|
| `src/components/TasksPanel.tsx` | Replace `hoveredElement` signal with `hoveredRect` signal; update mouseEnter handler; update Popover props |

### Change details

#### 1. `hoveredElement → hoveredRect` signal (TasksPanel.tsx, ~line 8)

**Before:**
```ts
const [hoveredElement, setHoveredElement] = createSignal<HTMLElement | null>(null);
```

**After:**
```ts
const [hoveredRect, setHoveredRect] = createSignal<{ top: number; left: number; width: number; height: number } | null>(null);
```

#### 2. Update `onMouseEnter` handler (~line 110)

**Before:**
```tsx
onMouseEnter={(e) => {
  cancelClose();
  setHoveredElement(e.currentTarget);
  setHoveredId(task.id);
}}
```

**After:**
```tsx
onMouseEnter={(e) => {
  cancelClose();
  const rect = e.currentTarget.getBoundingClientRect();
  setHoveredRect({ top: rect.top, left: rect.left, width: rect.width, height: rect.height });
  setHoveredId(task.id);
}}
```

#### 3. Update `scheduleClose` (~line 96)

Also reset `hoveredRect` to null when closing:

```ts
const scheduleClose = () => {
  if (closeTimer) clearTimeout(closeTimer);
  closeTimer = setTimeout(() => {
    setHoveredId(null);
    setHoveredRect(null);
  }, 150);
};
```

#### 4. Switch Popover from `triggerRef` to `position` (~line 140)

**Before:**
```tsx
<Popover
  open={hoveredTask() !== null}
  onClose={() => {}}
  triggerRef={() => hoveredElement()}
  anchorPoint={{x:1,y:0}}
  originPoint={{x:0,y:0}}
  showBackdrop={false}
>
```

**After:**
```tsx
<Popover
  open={hoveredTask() !== null}
  onClose={() => {}}
  position={hoveredRect() ?? undefined}
  anchorPoint={{x:1,y:0}}
  originPoint={{x:0,y:0}}
  showBackdrop={false}
>
```

The `Popover.position` prop type expects `{ top: number; left: number; width?: number; height?: number }` — our `hoveredRect` matches this signature exactly (all 4 fields from `DOMRect`).

### Why this cannot break anything else

- The `hoveredRect` signal is only used in two places: the `onMouseEnter` handler (write) and the `<Popover position={...}>` prop (read). No other component or function references it.
- `hoveredElement` signal is deleted entirely — its only consumer was the Popover's `triggerRef` prop.
- `hoveredTask()` remains unchanged (it only depends on `hoveredId`, not on `hoveredRect`).

### Verification plan

1. **Build check:** `pnpm tsc --noEmit` or `pnpm build` — must pass with no TypeScript errors
2. **Visual check:** Open app → hover over any task dot on the right sidebar → popover appears at correct position
3. **Polling check:** Keep mouse still while chat is streaming → popover must NOT jump position
4. **Mouse move check:** Move mouse to a different task dot → popover repositions correctly (new coordinates captured)
5. **Resize check:** With popover open, resize window → popover should still be visible (clamp logic)
6. **Close check:** Move mouse away → popover closes after 150ms delay

## Tasks summary

1. **Replace `hoveredElement` with `hoveredRect` signal** — change signal type and declaration in TasksPanel.tsx
2. **Update mouseEnter handler** — capture `getBoundingClientRect()` and store coordinates instead of element ref
3. **Update scheduleClose** — also reset `hoveredRect` to null
4. **Switch Popover from `triggerRef` to `position` prop** — pass `position={hoveredRect()}` with same anchor/origin
5. **Build verification** — run build to confirm no TS errors
