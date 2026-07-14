import { describe, it, expect, vi, afterEach, beforeEach } from "vitest";
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

// ── ResizeObserver mock (Popover depends on it) ────────────────────
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
              [{ contentRect: { width: 200, height: 160 } } as ResizeObserverEntry],
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

    const backdrop = document.querySelector('[class*="inset-0"]')!;
    backdrop.dispatchEvent(new MouseEvent("click", { bubbles: true }));

    expect(onClose).toHaveBeenCalledTimes(1);
  });

  // ── click inside menu does NOT close (stopPropagation) ─────────────
  it("does not close when clicking inside the menu panel", () => {
    const onClose = vi.fn();
    mount({ ...defaultProps, onClose });

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

  // ── renders separator between items ──────────────────────────────
  it("renders a separator after items with separatorAfter set", () => {
    mount();

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

    const separators = document.body.querySelectorAll('[class*="border-t"]');
    expect(separators.length).toBe(1);
  });

  // ── empty items list ──────────────────────────────────────────────
  it("renders an empty menu panel when items is empty", () => {
    mount({ ...defaultProps, items: [] });

    const buttons = document.body.querySelectorAll("button");
    expect(buttons.length).toBe(0);
    const panels = document.querySelectorAll('[class*="min-w-\\[200px\\]"]');
    expect(panels.length).toBe(1);
  });

  // ── single item with no separator ─────────────────────────────────
  it("renders no separators for a single item even if separatorAfter is set", () => {
    mount({
      ...defaultProps,
      items: [{ label: "Only", icon: "file", action: vi.fn(), separatorAfter: true }],
    });

    const separators = document.body.querySelectorAll('[class*="border-t"]');
    expect(separators.length).toBe(0);
  });
});
