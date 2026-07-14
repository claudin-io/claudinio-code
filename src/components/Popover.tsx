import {
  createEffect,
  createSignal,
  onCleanup,
  onMount,
  Show,
  type Component,
  type JSX,
} from "solid-js";
import { Portal } from "solid-js/web";

// ── Types ──────────────────────────────────────────────────────────

export interface AnchorPoint {
  /** 0 = left edge, 1 = right edge */
  x: number;
  /** 0 = top edge, 1 = bottom edge */
  y: number;
}

export interface PopoverProps {
  /** Whether the popover is visible */
  open: boolean;
  /** Called when the popover should close (Escape, backdrop click) */
  onClose: () => void;
  /**
   * Element anchor: the popover positions relative to this element's
   * bounding rect. Overridden by `position` if both are set.
   * Can be a direct HTMLElement ref or a lazy accessor (e.g. `() => myRef`).
   */
  triggerRef?: HTMLElement | (() => HTMLElement | undefined | null);
  /** Point on the trigger element (default: {x:0, y:1} = bottom-left) */
  anchorPoint?: AnchorPoint;
  /** Point on the popover itself (default: {x:0, y:0} = top-left) */
  originPoint?: AnchorPoint;
  /** Minimum distance from viewport edges in px (default: 8) */
  margin?: number;
  /** Show a full-screen backdrop that catches outside clicks (default: true) */
  showBackdrop?: boolean;
  /**
   * Override position with explicit viewport coordinates.
   * When set, triggerRef is ignored.
   */
  position?: { top: number; left: number; width?: number; height?: number };
  /** Extra class(es) applied to the popover container */
  class?: string;
  children: JSX.Element;
}

// ── Positioning engine ─────────────────────────────────────────────

type Rect = { top: number; left: number; width: number; height: number };

export function computePosition(
  trigger: Rect,
  popover: { width: number; height: number },
  anchorPoint: AnchorPoint,
  originPoint: AnchorPoint,
  margin: number,
): { top: number; left: number } {
  // 1. Global anchor point on the trigger
  const anchorX = trigger.left + trigger.width * anchorPoint.x;
  const anchorY = trigger.top + trigger.height * anchorPoint.y;

  // 2. Initial position
  let left = anchorX - popover.width * originPoint.x;
  let top = anchorY - popover.height * originPoint.y;

  const vw = window.innerWidth;
  const vh = window.innerHeight;

  // 3. Overflow check per axis
  const overflowRight = left + popover.width > vw - margin;
  const overflowLeft = left < margin;
  const overflowBottom = top + popover.height > vh - margin;
  const overflowTop = top < margin;

  // 4. Flip: try the opposite origin on any overflowing axis
  if (overflowRight || overflowLeft) {
    left = anchorX - popover.width * (1 - originPoint.x);
  }
  if (overflowBottom || overflowTop) {
    top = anchorY - popover.height * (1 - originPoint.y);
  }

  // 5. Clamp as final fallback so the popover stays within the viewport
  left = Math.max(margin, Math.min(left, vw - popover.width - margin));
  top = Math.max(margin, Math.min(top, vh - popover.height - margin));

  return { top, left };
}

// ── Component ──────────────────────────────────────────────────────

export const Popover: Component<PopoverProps> = (props) => {
  const anchorPt = () => props.anchorPoint ?? { x: 0, y: 1 };
  const originPt = () => props.originPoint ?? { x: 0, y: 0 };
  const margin = () => props.margin ?? 8;

  const [popoverWidth, setPopoverWidth] = createSignal(0);
  const [popoverHeight, setPopoverHeight] = createSignal(0);
  const [ready, setReady] = createSignal(false);
  const [position, setPosition] = createSignal({ top: 0, left: 0 });
  const [popoverEl, setPopoverEl] = createSignal<HTMLDivElement | undefined>();

  // ── ResizeObserver: measure popover dimensions ─────────────────
  // Must use a signal-based ref (setPopoverEl) so the createEffect
  // reactively triggers when the DOM element appears. A plain `let`
  // popoverRef would NOT re-run the effect — the ref callback in JSX
  // fires during DOM creation but is invisible to SolidJS reactivity.
  // Without this, a Popover rendered with open=false first and then
  // toggled to true would never get its observer.
  createEffect(() => {
    const el = popoverEl();
    if (!el || !props.open) return;

    const ro = new ResizeObserver((entries) => {
      for (const entry of entries) {
        const { width, height } = entry.contentRect;
        setPopoverWidth(width);
        setPopoverHeight(height);
        if (!ready()) setReady(true);
      }
    });
    ro.observe(el);

    onCleanup(() => {
      ro.disconnect();
    });
  });

  // ── Window resize trigger ─────────────────────────────────────
  const [resizeVersion, setResizeVersion] = createSignal(0);
  onMount(() => {
    const onResize = () => setResizeVersion((v) => v + 1);
    window.addEventListener("resize", onResize);
    onCleanup(() => window.removeEventListener("resize", onResize));
  });

  // ── Recalculate position ──────────────────────────────────────
  createEffect(() => {
    resizeVersion(); // track resize changes
    const pw = popoverWidth();
    const ph = popoverHeight();

    if (!props.open || pw === 0 || ph === 0) return;

    let triggerRect: Rect | undefined;

    if (props.position) {
      // position is an explicit 0-area trigger rect (caret / mouse coords).
      // width defaults to 0 so anchorPoint math anchors at the exact point.
      triggerRect = {
        top: props.position.top,
        left: props.position.left,
        width: props.position.width ?? 0,
        height: props.position.height ?? 0,
      };
    } else if (props.triggerRef) {
      const el =
        typeof props.triggerRef === "function"
          ? props.triggerRef()
          : props.triggerRef;
      if (el) {
        const r = el.getBoundingClientRect();
        triggerRect = { top: r.top, left: r.left, width: r.width, height: r.height };
      }
    }

    if (!triggerRect) return;

    const pos = computePosition(triggerRect, { width: pw, height: ph }, anchorPt(), originPt(), margin());

    // Only update if the position actually changed — prevents infinite loops
    setPosition((prev) => {
      if (prev.top === pos.top && prev.left === pos.left) return prev;
      return pos;
    });
  });

  // ── Escape key dismiss ────────────────────────────────────────
  onMount(() => {
    const onKeyDown = (e: KeyboardEvent) => {
      if (e.key === "Escape" && props.open) {
        e.preventDefault();
        e.stopPropagation();
        props.onClose();
      }
    };
    document.addEventListener("keydown", onKeyDown);
    onCleanup(() => document.removeEventListener("keydown", onKeyDown));
  });

  // ── Track open changes to reset ready state ───────────────────
  createEffect(() => {
    if (!props.open) {
      setReady(false);
    }
  });

  // ── Render ────────────────────────────────────────────────────
  return (
    <Show when={props.open}>
      <Portal>
        {/* Backdrop — catches outside clicks */}
        <Show when={props.showBackdrop ?? true}>
          <div class="fixed inset-0 z-40" onClick={props.onClose} />
        </Show>

        {/* Popover panel */}
        <div
          ref={setPopoverEl}
          class={props.class ?? ""}
          style={{
            position: "fixed",
            top: `${position().top}px`,
            left: `${position().left}px`,
            "z-index": 50,
            visibility: ready() ? "visible" : "hidden",
          }}
        >
          {props.children}
        </div>
      </Portal>
    </Show>
  );
};
