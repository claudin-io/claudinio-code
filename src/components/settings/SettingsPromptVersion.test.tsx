import { describe, it, expect, vi, afterEach } from "vitest";
import { render } from "solid-js/web";
import { getConfig, setConfig, type AgentConfig } from "../../lib/ipc";
import { SettingsPromptVersion } from "./SettingsPromptVersion";

vi.mock("../../lib/ipc", () => ({
  getConfig: vi.fn(),
  setConfig: vi.fn().mockResolvedValue(undefined),
}));

let dispose: (() => void) | undefined;
let host: HTMLDivElement | undefined;

afterEach(() => {
  dispose?.();
  host?.remove();
  dispose = undefined;
  host = undefined;
  vi.clearAllMocks();
});

const flush = () => new Promise((r) => setTimeout(r, 0));

async function mount(promptVersion: AgentConfig["promptVersion"]) {
  vi.mocked(getConfig).mockResolvedValue({ promptVersion } as AgentConfig);
  host = document.createElement("div");
  document.body.appendChild(host);
  dispose = render(() => <SettingsPromptVersion />, host);
  await flush();
  return host;
}

const pressed = (el: HTMLElement, v: string) =>
  el.querySelector(`button[data-prompt-version="${v}"]`)!.getAttribute("aria-pressed");

describe("SettingsPromptVersion", () => {
  it("shows v2 selected for a config written before the setting existed", async () => {
    const el = await mount(undefined);
    expect(pressed(el, "v2")).toBe("true");
    expect(pressed(el, "v1")).toBe("false");
  });

  it("shows v1 selected when the user chose it", async () => {
    const el = await mount("v1");
    expect(pressed(el, "v1")).toBe("true");
    expect(el.querySelector("[data-prompt-version-hint]")!.textContent).toContain("previous");
  });

  it("saves the choice and selects it", async () => {
    const el = await mount("v2");
    el.querySelector<HTMLButtonElement>('button[data-prompt-version="v1"]')!.click();
    await flush();
    expect(setConfig).toHaveBeenCalledWith({ promptVersion: "v1" });
    expect(pressed(el, "v1")).toBe("true");
  });

  it("explains in one line what the setting changes", async () => {
    const el = await mount("v2");
    const hint = el.querySelector("[data-prompt-version-hint]")!.textContent!;
    expect(hint).toContain("next message");
  });
});
