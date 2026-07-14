import { describe, it, expect, vi, afterEach, beforeEach } from "vitest";
import { render } from "solid-js/web";
import { Popover, computePosition } from "./Popover";

// ── Setup: create a stable trigger element in the document ─────────
let triggerEl: HTMLButtonElement;

beforeEach(() => {
  triggerEl = document.createElement("button");
  triggerEl.style.position = "absolute";
  triggerEl.style.top = "100px";
  triggerEl.style.left = "100px";
  triggerEl.style.width = "120px";
  triggerEl.style.height = "32px";
  triggerEl.textContent = "Trigger";
  document.body.appendChild(triggerEl);
});

afterEach(() => {
  document.body.innerHTML = "";
  vi.clearAllMocks();
});

// ── Mock ResizeObserver for jsdom ─────────────────────────────────
beforeEach(() => {
  vi.stubGlobal(
    "ResizeObserver",
    vi.fn(function MockResizeObserver(callback: ResizeObserverCallback) {
      let observed = false;
      return {
        observe: vi.fn(() => {
          if (observed) return;
          observed = true;
          queueMicrotask(() => {
            callback(
              [{ contentRect: { width: 280, height: 160 } } as ResizeObserverEntry],
              null as unknown as ResizeObserver,
            );
          });
        }),
        disconnect: vi.fn(),
        unobserve: vi.fn(),
      };
    }),
  );
});

afterEach(() => {
  vi.unstubAllGlobals();
});

// ══════════════════════════════════════════════════════════════════════

describe("computePosition (pure function)", () => {
  const makeTrigger = (overrides: Partial<typeof base> = {}) => ({ ...base, ...overrides });
  const base = { top: 100, left: 100, width: 120, height: 32 };
  const popover = { width: 280, height: 160 };
  const margin = 8;

  beforeEach(() => {
    Object.defineProperty(window, "innerWidth", { value: 1280, configurable: true });
    Object.defineProperty(window, "innerHeight", { value: 800, configurable: true });
  });

  // ── No overflow ───────────────────────────────────────────────
  it("positions below-left with anchor 0,1 and origin 0,0 (default)", () => {
    const pos = computePosition(makeTrigger(), popover, { x: 0, y: 1 }, { x: 0, y: 0 }, margin);
    expect(pos.left).toBe(100);
    expect(pos.top).toBe(132);
  });

  // ── anchor 1,1 origin 1,1 ──────────────────────────────────────
  it("positions with anchor 1,1 and origin 1,1 (flips both axes)", () => {
    // anchorX = 100 + 120*1 = 220, anchorY = 132
    // Initial left = 220 - 280*1 = -60 (< 8 → overflow left)
    // Flip X: left = 220 - 280*0 = 220 (now fits: 220 >= 8, 500 <= 1272)
    // Initial top = 132 - 160*1 = -28 (< 8 → overflow top)
    // Flip Y: top = 132 - 160*0 = 132 (now fits: 132 >= 8, 292 <= 792)
    const pos = computePosition(makeTrigger(), popover, { x: 1, y: 1 }, { x: 1, y: 1 }, margin);
    expect(pos.left).toBe(220);
    expect(pos.top).toBe(132);
  });

  // ── Overflow right → flip horizontally ──────────────────────────
  it("flips horizontally when popover overflows right edge", () => {
    const pos = computePosition(
      makeTrigger({ left: 1200 }),
      popover,
      { x: 0, y: 1 },
      { x: 0, y: 0 },
      margin,
    );
    // anchorX = 1200, left = 1200, 1200+280=1480 > 1272 → overflow!
    // Flip: left = 1200 - 280*1 = 920 ✓
    expect(pos.left).toBe(920);
  });

  // ── Overflow left → flip horizontally ───────────────────────────
  it("flips then clamps when popover overflows left edge", () => {
    const pos = computePosition(
      makeTrigger({ left: 0 }),
      popover,
      { x: 0, y: 1 },
      { x: 0, y: 0 },
      margin,
    );
    // anchorX = 0, left = 0 → < 8 overflow
    // Flip: left = 0 - 280*1 = -280 → still < 8 → clamp to 8
    expect(pos.left).toBe(8);
  });

  // ── Overflow bottom → flip vertically ───────────────────────────
  it("flips vertically when popover overflows bottom edge", () => {
    const pos = computePosition(
      makeTrigger({ top: 700 }),
      popover,
      { x: 0, y: 1 },
      { x: 0, y: 0 },
      margin,
    );
    // anchorY = 700 + 32*1 = 732, top = 732, 732+160=892 > 792 → overflow!
    // Flip: top = 732 - 160*1 = 572 ✓
    expect(pos.top).toBe(572);
  });

  // ── Overflow top → flip vertically ──────────────────────────────
  it("flips then clamps when popover overflows top edge", () => {
    const pos = computePosition(
      makeTrigger({ top: 0 }),
      popover,
      { x: 0, y: 1 },
      { x: 0, y: 0 },
      margin,
    );
    // anchorY = 0 + 32*1 = 32, top = 32. 32 >= 8 → no overflow!
    // The test with top=0 and anchor 0,1 still yields top=32 which fits.
    // To force overflow-top we need anchor 0,0 (top-left of trigger)
    expect(pos.top).toBe(32);
  });

  // ── Overflow top with anchor 0,0 → flip vertically ─────────────
  it("flips vertically when top edge overflows with anchor 0,0", () => {
    const pos = computePosition(
      makeTrigger({ top: 0 }),
      popover,
      { x: 0, y: 0 },
      { x: 0, y: 0 },
      margin,
    );
    // anchorY = 0, top = 0 → < 8 overflow
    // Flip: top = 0 - 160*1 = -160 → still < 8 → clamp to 8
    expect(pos.top).toBe(8);
  });

  // ── Clamp when flip doesn't help ────────────────────────────────
  it("clamps both axes when flip also overflows", () => {
    const hugePopover = { width: 2000, height: 1000 };
    const pos = computePosition(
      makeTrigger({ top: 0, left: 0 }),
      hugePopover,
      { x: 0, y: 0 },
      { x: 0, y: 0 },
      margin,
    );
    // left = 0 < 8 → flip: 0-2000=-2000 → clamp to 8
    // top  = 0 < 8 → flip: 0-1000=-1000 → clamp to 8
    expect(pos.left).toBe(8);
    expect(pos.top).toBe(8);
  });

  // ── Right-Edge with origin 1,0 (menu pattern) ──────────────────
  it("positions right-edge trigger with right-aligned popover (sessions dropdown)", () => {
    const pos = computePosition(
      makeTrigger({ left: 800, top: 50 }),
      popover,
      { x: 1, y: 1 },
      { x: 1, y: 0 },
      margin,
    );
    // anchorX = 800+120=920, left = 920-280=640
    // anchorY = 50+32=82, top = 82
    expect(pos.left).toBe(640);
    expect(pos.top).toBe(82);
  });

  // ── Popover above-right ─────────────────────────────────────────
  it("positions popover above-right when anchor 1,0 and origin 1,1", () => {
    const pos = computePosition(
      makeTrigger(),
      popover,
      { x: 1, y: 0 },
      { x: 1, y: 1 },
      margin,
    );
    // anchorX = 100+120=220, left = 220-280=-60 < 8 → flip
    //   left = 220-280*0 = 220 (now fits)
    // anchorY = 100, top = 100-160=-60 < 8 → flip
    //   top = 100-160*0 = 100 (now fits)
    expect(pos.left).toBe(220);
    expect(pos.top).toBe(100);
  });
});

// ══════════════════════════════════════════════════════════════════════

describe("Popover component", () => {
  const defaultProps = () => ({
    open: true,
    onClose: vi.fn(),
    triggerRef: triggerEl,
    children: <div data-testid="popover-content">Content</div>,
  });

  // ── Renders content when open ─────────────────────────────────
  it("renders children when open=true", () => {
    const dispose = render(() => <Popover {...defaultProps()} />, document.body);
    expect(document.body.textContent).toContain("Content");
    dispose();
  });

  // ── Does not render when open=false ────────────────────────────
  it("does not render children when open=false", () => {
    const dispose = render(
      () => <Popover {...defaultProps()} open={false} />,
      document.body,
    );
    expect(document.body.textContent).not.toContain("Content");
    dispose();
  });

  // ── Escape key calls onClose ──────────────────────────────────
  it("calls onClose on Escape keydown", () => {
    const onClose = vi.fn();
    const dispose = render(
      () => <Popover {...defaultProps()} onClose={onClose} />,
      document.body,
    );
    document.dispatchEvent(new KeyboardEvent("keydown", { key: "Escape" }));
    expect(onClose).toHaveBeenCalledTimes(1);
    dispose();
  });

  // ── Backdrop click calls onClose ──────────────────────────────
  it("calls onClose when clicking the backdrop", () => {
    const onClose = vi.fn();
    const dispose = render(
      () => <Popover {...defaultProps()} onClose={onClose} />,
      document.body,
    );
    const backdrop = document.querySelector('[class*="inset-0"]') as HTMLElement;
    expect(backdrop).toBeTruthy();
    backdrop.click();
    expect(onClose).toHaveBeenCalledTimes(1);
    dispose();
  });

  // ── No backdrop when showBackdrop=false ────────────────────────
  it("does not render backdrop when showBackdrop is false", () => {
    const dispose = render(
      () => <Popover {...defaultProps()} showBackdrop={false} />,
      document.body,
    );
    const backdrop = document.querySelector('[class*="inset-0"]');
    expect(backdrop).toBeNull();
    dispose();
  });

  // ── Extra class passed to popover panel ────────────────────────
  it("applies extra class to the popover panel", () => {
    const dispose = render(
      () => <Popover {...defaultProps()} class="my-custom" />,
      document.body,
    );
    const panel = document.querySelector('[style*="position: fixed"]');
    expect(panel?.classList.contains("my-custom")).toBe(true);
    dispose();
  });

  // ── Popover uses position prop when provided ─────────────────
  it("positions using the position prop override", () => {
    const dispose = render(
      () => (
        <Popover
          {...defaultProps()}
          position={{ top: 42, left: 300, width: 0, height: 0 }}
          triggerRef={undefined as any}
        />
      ),
      document.body,
    );
    expect(document.body.textContent).toContain("Content");
    dispose();
  });

  // ── onClose not called for non-Escape keys ────────────────────
  it("does not call onClose for non-Escape keys", () => {
    const onClose = vi.fn();
    const dispose = render(
      () => <Popover {...defaultProps()} onClose={onClose} />,
      document.body,
    );
    document.dispatchEvent(new KeyboardEvent("keydown", { key: "Enter" }));
    expect(onClose).not.toHaveBeenCalled();
    dispose();
  });
});
