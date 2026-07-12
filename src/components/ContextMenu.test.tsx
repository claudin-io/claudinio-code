import { describe, it, expect, vi, afterEach } from "vitest";
import { render } from "solid-js/web";
import { ContextMenu, type ContextMenuItem } from "./ContextMenu";

// ── i18n mock (returns the key as-is for stable assertions) ─────────
vi.mock("../lib/grill-me", () => ({
  t: (key: string) => key,
}));

// ── Icon mock (lightweight SVG stub) ────────────────────────────────
vi.mock("./Icon", () => ({
  Icon: (props: { name: string; class?: string }) => (
    <span data-testid={`icon-${props.name}`} class={props.class} />
  ),
}));

// ══════════════════════════════════════════════════════════════════════

const sampleItems: ContextMenuItem[] = [
  { label: "Cut", icon: "scissors", action: vi.fn() },
  { label: "Copy", icon: "copy", action: vi.fn() },
  {
    label: "Paste",
    icon: "clipboard",
    action: vi.fn(),
    separatorAfter: true,
  },
  { label: "Delete", icon: "trash", action: vi.fn() },
];

const defaultProps = {
  x: 100,
  y: 200,
  items: sampleItems,
  onClose: vi.fn(),
};

function mount(props = defaultProps) {
  const dispose = render(() => <ContextMenu {...props} />, document.body);
  return dispose;
}

// ══════════════════════════════════════════════════════════════════════

describe("ContextMenu", () => {
  afterEach(() => {
    document.body.innerHTML = "";
    vi.clearAllMocks();
  });

  // ── renders all items ──────────────────────────────────────────────
  it("renders all item labels", () => {
    mount();
    expect(document.body.textContent).toContain("Cut");
    expect(document.body.textContent).toContain("Copy");
    expect(document.body.textContent).toContain("Paste");
    expect(document.body.textContent).toContain("Delete");
  });

  // ── renders icons ──────────────────────────────────────────────────
  it("renders an icon for each menu item", () => {
    mount();
    const icons = document.body.querySelectorAll('[data-testid^="icon-"]');
    expect(icons.length).toBe(4);
    expect(icons[0].getAttribute("data-testid")).toBe("icon-scissors");
    expect(icons[1].getAttribute("data-testid")).toBe("icon-copy");
    expect(icons[2].getAttribute("data-testid")).toBe("icon-clipboard");
    expect(icons[3].getAttribute("data-testid")).toBe("icon-trash");
  });

  // ── item click ─────────────────────────────────────────────────────
  it("calls item.action() and onClose() when clicking an item", () => {
    const onClose = vi.fn();
    mount({ ...defaultProps, onClose });

    const buttons = document.body.querySelectorAll("button");
    expect(buttons.length).toBe(4);

    // Click "Copy" (index 1)
    const copyBtn = Array.from(buttons).find(
      (b) => b.textContent === "Copy",
    )!;
    copyBtn.click();

    expect(sampleItems[1].action).toHaveBeenCalledTimes(1);
    expect(onClose).toHaveBeenCalledTimes(1);
  });

  // ── backdrop click closes ──────────────────────────────────────────
  it("closes when clicking the backdrop", () => {
    const onClose = vi.fn();
    mount({ ...defaultProps, onClose });

    // Backdrop is the first div with class "fixed inset-0 z-50"
    const backdrop = document.querySelector('[class*="inset-0"]')!;
    backdrop.dispatchEvent(new MouseEvent("click", { bubbles: true }));

    expect(onClose).toHaveBeenCalledTimes(1);
  });

  // ── backdrop right-click closes ────────────────────────────────────
  it("closes on backdrop right-click (contextmenu)", () => {
    const onClose = vi.fn();
    mount({ ...defaultProps, onClose });

    const backdrop = document.querySelector('[class*="inset-0"]')!;
    backdrop.dispatchEvent(
      new MouseEvent("contextmenu", { bubbles: true, cancelable: true }),
    );

    expect(onClose).toHaveBeenCalledTimes(1);
  });

  // ── click inside menu does NOT close (stopPropagation) ─────────────
  it("does not close when clicking inside the menu panel", () => {
    const onClose = vi.fn();
    mount({ ...defaultProps, onClose });

    // The menu panel has stopPropagation on click
    const menuPanel = document.querySelector('[class*="min-w-\\[200px\\]"]')!;
    menuPanel.dispatchEvent(new MouseEvent("click", { bubbles: true }));

    expect(onClose).not.toHaveBeenCalled();
  });

  // ── Escape key closes ──────────────────────────────────────────────
  it("closes on Escape keydown", () => {
    const onClose = vi.fn();
    mount({ ...defaultProps, onClose });

    document.dispatchEvent(new KeyboardEvent("keydown", { key: "Escape" }));

    expect(onClose).toHaveBeenCalledTimes(1);
  });

  // ── non-Escape key is a no-op ──────────────────────────────────────
  it("does not close on non-Escape keypress", () => {
    const onClose = vi.fn();
    mount({ ...defaultProps, onClose });

    document.dispatchEvent(new KeyboardEvent("keydown", { key: "Enter" }));

    expect(onClose).not.toHaveBeenCalled();
  });

  // ── onCleanup removes keydown listener ─────────────────────────────
  it("removes the keydown event listener on dispose (onCleanup)", () => {
    const onClose = vi.fn();
    const dispose = mount({ ...defaultProps, onClose });

    // Dispose the component → onCleanup fires
    dispose();

    // Dispatch Escape: listener should be gone
    document.dispatchEvent(new KeyboardEvent("keydown", { key: "Escape" }));
    expect(onClose).not.toHaveBeenCalled();
  });

  // ── renders separator between items ──────────────────────────────
  it("renders a separator after items with separatorAfter set", () => {
    mount();

    // Expect 1 separator (after "Paste", which has separatorAfter: true)
    const separators = document.querySelectorAll('[class*="border-t"]');
    expect(separators.length).toBe(1);
  });

  // ── no separator after last item ───────────────────────────────────
  it("does not render a separator after the last item even when separatorAfter is true", () => {
    const itemsWithSepOnLast: ContextMenuItem[] = [
      { label: "One", icon: "file", action: vi.fn(), separatorAfter: true },
      {
        label: "Two",
        icon: "folder",
        action: vi.fn(),
        separatorAfter: true,
      },
    ];
    mount({ ...defaultProps, items: itemsWithSepOnLast });

    // Two items, each with separatorAfter, but only 1 separator should render
    // (not after the last item)
    const separators = document.body.querySelectorAll('[class*="border-t"]');
    expect(separators.length).toBe(1);
  });

  // ── clampX: stays within viewport ───────────────────────────────
  it("clamps x position to viewport width minus 200px", () => {
    // Set viewport width artificially small
    Object.defineProperty(window, "innerWidth", {
      value: 250,
      configurable: true,
    });
    // x=1000 but window.innerWidth(250) - 200 = 50, so left should be 50
    mount({ ...defaultProps, x: 1000 });

    const menuPanel = document.querySelector('[class*="min-w-\\[200px\\]"]') as HTMLElement;
    expect(menuPanel.style.left).toBe("50px");
  });

  // ── clampX: low x stays as-is ─────────────────────────────────────
  it("uses the given x when it fits within viewport", () => {
    Object.defineProperty(window, "innerWidth", {
      value: 1200,
      configurable: true,
    });
    mount({ ...defaultProps, x: 300 });

    const menuPanel = document.querySelector('[class*="min-w-\\[200px\\]"]') as HTMLElement;
    expect(menuPanel.style.left).toBe("300px");
  });

  // ── clampY: stays within viewport ───────────────────────────────
  it("clamps y position so the menu doesn't overflow the viewport bottom", () => {
    Object.defineProperty(window, "innerHeight", {
      value: 400,
      configurable: true,
    });
    // 4 items × 40 ≈ 160 → clampY = min(1000, 400 - 160) = 240
    mount({ ...defaultProps, y: 1000 });

    const menuPanel = document.querySelector('[class*="min-w-\\[200px\\]"]') as HTMLElement;
    expect(menuPanel.style.top).toBe("240px");
  });

  // ── clampY: low y stays as-is ─────────────────────────────────────
  it("uses the given y when it fits within viewport", () => {
    Object.defineProperty(window, "innerHeight", {
      value: 800,
      configurable: true,
    });
    mount({ ...defaultProps, y: 150 });

    const menuPanel = document.querySelector('[class*="min-w-\\[200px\\]"]') as HTMLElement;
    expect(menuPanel.style.top).toBe("150px");
  });

  // ── empty items list ──────────────────────────────────────────────
  it("renders an empty menu panel when items is empty", () => {
    mount({ ...defaultProps, items: [] });

    const buttons = document.body.querySelectorAll("button");
    expect(buttons.length).toBe(0);
    // Backdrop + empty menu panel still render
    const panels = document.querySelectorAll('[class*="min-w-\\[200px\\]"]');
    expect(panels.length).toBe(1);
  });

  // ── single item with no separator ─────────────────────────────────
  it("renders no separators for a single item even if separatorAfter is set", () => {
    mount({
      ...defaultProps,
      items: [{ label: "Only", icon: "file", action: vi.fn(), separatorAfter: true }],
    });

    // i() < props.items.length - 1 → 0 < 0 → false → no <Show>
    const separators = document.body.querySelectorAll('[class*="border-t"]');
    expect(separators.length).toBe(0);
  });

  // ── onMount adds the keydown listener ──────────────────────────────
  it("responds to Escape immediately after mount (onMount worked)", () => {
    const onClose = vi.fn();
    mount({ ...defaultProps, onClose });

    // Freshly mounted, listener should be active
    document.dispatchEvent(new KeyboardEvent("keydown", { key: "Escape" }));
    expect(onClose).toHaveBeenCalledTimes(1);
  });
});
