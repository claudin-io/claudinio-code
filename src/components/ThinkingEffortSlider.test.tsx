import { describe, it, expect, vi, beforeEach, afterEach } from "vitest";
import { render } from "solid-js/web";
import { createSignal } from "solid-js";
import { ThinkingEffortSlider } from "./ThinkingEffortSlider";
import { THINKING_EFFORTS, type ThinkingEffort } from "../lib/ipc";

describe("ThinkingEffortSlider", () => {
  let container: HTMLDivElement;
  let dispose: () => void;

  beforeEach(() => {
    // Attached to the document so Solid's delegated onInput handler
    // (listening at document level) receives dispatched events.
    container = document.createElement("div");
    document.body.appendChild(container);
  });

  afterEach(() => {
    dispose?.();
    container.remove();
    vi.restoreAllMocks();
  });

  const slider = () => container.querySelector("input[type=range]") as HTMLInputElement;

  it("renders a 5-step range reflecting the current value's index", () => {
    dispose = render(
      () => <ThinkingEffortSlider value={() => "high"} onChange={vi.fn()} />,
      container,
    );
    expect(slider().min).toBe("0");
    expect(slider().max).toBe("4");
    expect(slider().value).toBe(String(THINKING_EFFORTS.indexOf("high")));
  });

  it("calls onChange with the level matching the slider index", () => {
    const onChange = vi.fn();
    dispose = render(
      () => <ThinkingEffortSlider value={() => "medium"} onChange={onChange} />,
      container,
    );
    for (let i = 0; i < THINKING_EFFORTS.length; i++) {
      slider().value = String(i);
      slider().dispatchEvent(new Event("input", { bubbles: true }));
      expect(onChange).toHaveBeenLastCalledWith(THINKING_EFFORTS[i]);
    }
    expect(onChange).toHaveBeenCalledTimes(THINKING_EFFORTS.length);
  });

  it("shows the level label and updates it when the value changes", () => {
    const [value, setValue] = createSignal<ThinkingEffort>("low");
    dispose = render(
      () => <ThinkingEffortSlider value={value} onChange={vi.fn()} />,
      container,
    );
    expect(container.textContent).toContain("Low");
    setValue("max");
    expect(container.textContent).toContain("Max");
    expect(slider().value).toBe("4");
  });

  it("disables the input when disabled() is true", () => {
    dispose = render(
      () => (
        <ThinkingEffortSlider value={() => "medium"} onChange={vi.fn()} disabled={() => true} />
      ),
      container,
    );
    expect(slider().disabled).toBe(true);
  });
});
