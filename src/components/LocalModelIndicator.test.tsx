import { describe, it, expect, vi, afterEach } from "vitest";
import { render } from "solid-js/web";
import { localRuntimeStats, type LocalModelStats } from "../lib/ipc";
import { LocalModelIndicator } from "./LocalModelIndicator";

vi.mock("../lib/ipc", () => ({ localRuntimeStats: vi.fn() }));
vi.mock("./Icon", () => ({
  Icon: (props: { name: string; class?: string }) => (
    <span data-testid={`icon-${props.name}`} class={props.class ?? ""} />
  ),
}));

let dispose: (() => void) | undefined;
let host: HTMLDivElement | undefined;

afterEach(() => {
  dispose?.();
  host?.remove();
  dispose = undefined;
  host = undefined;
  vi.resetAllMocks();
});

const stats = (over: Partial<LocalModelStats> = {}): LocalModelStats => ({
  modelKey: "abc",
  displayName: "Qwen3-8B (Q4_K_M)",
  engine: "mlx",
  memoryBytes: 19_400_000_000,
  ctxSize: 131_072,
  ctxUsed: 4_210,
  tokensPerSecond: 13.9,
  promptTokensPerSecond: 240.9,
  tokensGenerated: 1234,
  busy: false,
  sleeping: false,
  ...over,
});

async function mount(list: LocalModelStats[], visible = true) {
  vi.mocked(localRuntimeStats).mockResolvedValue(list);
  host = document.createElement("div");
  document.body.appendChild(host);
  dispose = render(() => <LocalModelIndicator visible={() => visible} />, host);
  for (let i = 0; i < 6; i++) await Promise.resolve();
  return host;
}

describe("LocalModelIndicator", () => {
  /// A status item that is always present but usually says "0" is noise.
  it("shows nothing when no model is resident", async () => {
    const el = await mount([]);
    expect(el.textContent).toBe("");
  });

  it("shows the generation rate while a model is loaded", async () => {
    const el = await mount([stats()]);
    expect(el.textContent).toContain("13.9 tok/s");
  });

  /// A sleeping model costs no memory and produces nothing; showing its last
  /// rate would read as if it were still working.
  it("says idle when the weights have been unloaded", async () => {
    const el = await mount([stats({ sleeping: true })]);
    expect(el.textContent).toContain("idle");
    expect(el.textContent).not.toContain("tok/s");
  });

  it("expands to the numbers that explain the memory cost", async () => {
    const el = await mount([stats()]);
    el.querySelector("button")!.click();
    await Promise.resolve();
    const text = el.textContent ?? "";
    expect(text).toContain("Qwen3-8B (Q4_K_M)");
    expect(text).toContain("MLX");
    expect(text).toContain("19.4GB");
    expect(text).toContain("131,072");
  });

  it("names llama.cpp when that is the engine", async () => {
    const el = await mount([stats({ engine: "llamacpp" })]);
    el.querySelector("button")!.click();
    await Promise.resolve();
    expect(el.textContent).toContain("llama.cpp");
  });

  /// Polling a hidden window burns battery for pixels nobody is looking at.
  it("does not poll while the workspace is not visible", async () => {
    await mount([stats()], false);
    expect(localRuntimeStats).not.toHaveBeenCalled();
  });
});
