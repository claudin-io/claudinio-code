import { describe, it, expect, vi, beforeEach, afterEach } from "vitest";
import { render } from "solid-js/web";
import { ToastPill } from "./ToastPill";

describe("ToastPill", () => {
  let container: HTMLDivElement;
  let dispose: () => void;

  beforeEach(() => {
    container = document.createElement("div");
  });

  afterEach(() => {
    dispose?.();
    vi.useRealTimers();
  });

  it("renders with a message", () => {
    dispose = render(
      () => <ToastPill message="Hello" onDismiss={vi.fn()} />,
      container,
    );
    expect(container.textContent).toBe("Hello");
  });

  it("renders null message", () => {
    dispose = render(
      () => <ToastPill message={null} onDismiss={vi.fn()} />,
      container,
    );
    expect(container.textContent).toBe("");
  });

  it("has opacity-100 class when message is non-null", () => {
    dispose = render(
      () => <ToastPill message="Hello" onDismiss={vi.fn()} />,
      container,
    );
    const outerDiv = container.firstElementChild!;
    expect(outerDiv.className).toContain("opacity-100");
  });

  it("has opacity-0 class when message is null", () => {
    dispose = render(
      () => <ToastPill message={null} onDismiss={vi.fn()} />,
      container,
    );
    const outerDiv = container.firstElementChild!;
    expect(outerDiv.className).toContain("opacity-0");
  });

  it("calls onDismiss after 2000ms", () => {
    vi.useFakeTimers();
    const onDismiss = vi.fn();
    dispose = render(
      () => <ToastPill message="Hello" onDismiss={onDismiss} />,
      container,
    );
    vi.advanceTimersByTime(2000);
    expect(onDismiss).toHaveBeenCalledTimes(1);
  });
});
