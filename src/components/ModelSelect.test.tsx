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
    // one badge for the openrouter group only
    const badges = Array.from(container.querySelectorAll("span")).filter(
      (s) => s.textContent === "Experimental",
    );
    expect(badges.length).toBe(1);
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
