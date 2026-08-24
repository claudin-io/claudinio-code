import { describe, it, expect, vi, afterEach } from "vitest";
import { render } from "solid-js/web";
import { createSignal } from "solid-js";
import type { ModelGroup } from "../lib/ipc";


vi.mock("./Icon", () => ({
  Icon: (props: { name: string; class?: string }) => (
    <span data-testid={`icon-${props.name}`} class={props.class ?? ""} />
  ),
}));

// The real Popover positions via Portal + ResizeObserver, which jsdom
// doesn't drive — render children inline when open instead.
vi.mock("./Popover", () => ({
  Popover: (props: { open: boolean; children: unknown }) => (
    <div data-testid="popover">{props.open ? (props.children as never) : null}</div>
  ),
}));

import { ModelSelect } from "./ModelSelect";

function flush() {
  return new Promise((r) => setTimeout(r, 10));
}

const GROUPS: ModelGroup[] = [
  { providerId: "claudinio", providerName: "Claudinio", models: ["claudinio", "claudius"] },
  {
    providerId: "openrouter",
    providerName: "OpenRouter",
    models: ["openrouter/openai/gpt-4o-mini", "openrouter/deepseek/deepseek-chat"],
  },
  {
    providerId: "local",
    providerName: "Local (llama.cpp)",
    models: ["local/b55d3d9e4269fdb3"],
    labels: { "local/b55d3d9e4269fdb3": "Qwen3.8-27B-GGUF (IQ3_S)" },
  },
];

describe("ModelSelect", () => {
  let dispose: (() => void) | undefined;
  let container: HTMLDivElement;

  function mount(value = "claudinio", onChange: (m: string) => void = () => {}) {
    container = document.createElement("div");
    document.body.appendChild(container);
    const [val] = createSignal(value);
    const [groups] = createSignal(GROUPS);
    dispose = render(
      () => <ModelSelect value={val} onChange={onChange} groups={groups} />,
      container,
    );
  }

  afterEach(() => {
    dispose?.();
    container?.remove();
  });

  it("shows the current value on the trigger", () => {
    mount("claudius");
    expect(container.querySelector("button")!.textContent).toContain("claudius");
  });

  it("opens with grouped models and an Experimental badge on external groups", async () => {
    mount();
    container.querySelector("button")!.click();
    await flush();
    const text = container.textContent ?? "";
    expect(text).toContain("Claudinio");
    expect(text).toContain("OpenRouter");
    // one badge for the openrouter group only: "Experimental" means a
    // third-party catalog provider, which neither Claudinio nor a local model is
    const badges = Array.from(container.querySelectorAll("span")).filter(
      (s) => s.textContent === "Experimental",
    );
    expect(badges.length).toBe(1);
    expect(text).toContain("Local (llama.cpp)");
    // external models display without their provider prefix
    expect(text).toContain("openai/gpt-4o-mini");
    expect(text).not.toContain("openrouter/openai/gpt-4o-mini");
  });

  it("filters models by search query", async () => {
    mount();
    container.querySelector("button")!.click();
    await flush();
    const search = container.querySelector<HTMLInputElement>("input[type=text]")!;
    search.value = "deepseek";
    search.dispatchEvent(new Event("input", { bubbles: true }));
    await flush();
    const text = container.textContent ?? "";
    expect(text).toContain("deepseek/deepseek-chat");
    expect(text).not.toContain("claudius");
  });

  it("shows the empty state when nothing matches", async () => {
    mount();
    container.querySelector("button")!.click();
    await flush();
    const search = container.querySelector<HTMLInputElement>("input[type=text]")!;
    search.value = "zzz-no-such-model";
    search.dispatchEvent(new Event("input", { bubbles: true }));
    await flush();
    expect(container.textContent).toContain("No models match your search.");
  });

  /// A local model's id is the content hash of its weights — right for a
  /// directory name, unreadable in a picker. The group carries the label.
  it("shows a labelled model by name, not by its id", async () => {
    mount();
    container.querySelector("button")!.click();
    await flush();
    const text = container.textContent ?? "";
    expect(text).toContain("Qwen3.8-27B-GGUF (IQ3_S)");
    expect(text).not.toContain("b55d3d9e4269fdb3");
  });

  it("shows the label on the trigger when a labelled model is selected", () => {
    mount("local/b55d3d9e4269fdb3");
    expect(container.querySelector("button")!.textContent).toContain("Qwen3.8-27B-GGUF (IQ3_S)");
  });

  /// A model configured before it was deleted still has to render as something.
  it("falls back to the raw id when no group claims it", () => {
    mount("local/gone-from-disk");
    expect(container.querySelector("button")!.textContent).toContain("local/gone-from-disk");
  });

  it("finds a labelled model by its name", async () => {
    mount();
    container.querySelector("button")!.click();
    await flush();
    const search = container.querySelector<HTMLInputElement>("input[type=text]")!;
    search.value = "qwen";
    search.dispatchEvent(new Event("input", { bubbles: true }));
    await flush();
    const text = container.textContent ?? "";
    expect(text).toContain("Qwen3.8-27B-GGUF (IQ3_S)");
    expect(text).not.toContain("gpt-4o-mini");
  });

  it("selecting a labelled model calls onChange with the id, not the label", async () => {
    const onChange = vi.fn();
    mount("claudinio", onChange);
    container.querySelector("button")!.click();
    await flush();
    const option = Array.from(container.querySelectorAll("button")).find((b) =>
      b.textContent?.includes("Qwen3.8-27B-GGUF"),
    )!;
    option.click();
    expect(onChange).toHaveBeenCalledWith("local/b55d3d9e4269fdb3");
  });

  it("selecting a model calls onChange with the qualified id", async () => {
    const onChange = vi.fn();
    mount("claudinio", onChange);
    container.querySelector("button")!.click();
    await flush();
    const option = Array.from(container.querySelectorAll("button")).find((b) =>
      b.textContent?.includes("openai/gpt-4o-mini"),
    )!;
    option.click();
    expect(onChange).toHaveBeenCalledWith("openrouter/openai/gpt-4o-mini");
  });
});
