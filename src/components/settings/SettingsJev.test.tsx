import { describe, it, expect, vi, afterEach } from "vitest";
import { render } from "solid-js/web";
import { getConfig, setConfig, type AgentConfig } from "../../lib/ipc";
import { SettingsJev } from "./SettingsJev";

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

async function mount(jev: AgentConfig["jev"]) {
  vi.mocked(getConfig).mockResolvedValue({ jev } as AgentConfig);
  host = document.createElement("div");
  document.body.appendChild(host);
  dispose = render(() => <SettingsJev />, host);
  await flush();
  return host;
}

describe("SettingsJev", () => {
  it("says Jev is inactive when there is no credential", async () => {
    const el = await mount({ enabled: true, hasApiKey: false, backend: null });
    expect(el.textContent).toContain("Inactive");
  });

  it("says it runs through OpenRouter when that is the credential", async () => {
    const el = await mount({ enabled: true, hasApiKey: false, backend: "openrouter" });
    expect(el.textContent).toContain("OpenRouter");
  });

  it("says it is included in the Claudinio plan when signed in", async () => {
    const el = await mount({ enabled: true, hasApiKey: false, backend: "claudinio" });
    expect(el.textContent).toContain("included in your Claudinio plan");
  });

  it("says it runs on the TypeSafe key when one is saved", async () => {
    const el = await mount({ enabled: true, hasApiKey: true, backend: "typesafe" });
    expect(el.textContent).toContain("TypeSafe key");
  });

  it("saves a TypeSafe key", async () => {
    const el = await mount({ enabled: true, hasApiKey: false, backend: null });
    const input = el.querySelector<HTMLInputElement>('input[type="password"]')!;
    input.value = "ts-123";
    input.dispatchEvent(new Event("input", { bubbles: true }));
    el.querySelector<HTMLButtonElement>("button[data-jev-save]")!.click();
    await flush();
    expect(setConfig).toHaveBeenCalledWith({ jevApiKey: "ts-123" });
  });

  it("turns Jev off", async () => {
    const el = await mount({ enabled: true, hasApiKey: false, backend: "openrouter" });
    const box = el.querySelector<HTMLInputElement>('input[type="checkbox"]')!;
    box.checked = false;
    box.dispatchEvent(new Event("change", { bubbles: true }));
    await flush();
    expect(setConfig).toHaveBeenCalledWith({ jevEnabled: false });
  });

  it("shows Auto mode off for a config from before the setting existed", async () => {
    const el = await mount({ enabled: true, hasApiKey: false, backend: "claudinio" });
    expect(el.querySelector('button[data-jev-route="off"]')!.getAttribute("aria-pressed")).toBe("true");
    expect(el.querySelector("[data-jev-route-hint]")!.textContent).toContain("Nothing is asked");
  });

  it("shadow says it records without changing anything", async () => {
    const el = await mount({ enabled: true, hasApiKey: false, backend: "claudinio", route: "shadow" });
    expect(el.querySelector('button[data-jev-route="shadow"]')!.getAttribute("aria-pressed")).toBe("true");
    expect(el.querySelector("[data-jev-route-hint]")!.textContent).toContain("changes nothing");
  });

  it("switching Auto mode on saves it and explains what it does", async () => {
    const el = await mount({ enabled: true, hasApiKey: false, backend: "claudinio", route: "shadow" });
    el.querySelector<HTMLButtonElement>('button[data-jev-route="on"]')!.click();
    await flush();
    expect(setConfig).toHaveBeenCalledWith({ jevRoute: "on" });
    expect(el.querySelector('button[data-jev-route="on"]')!.getAttribute("aria-pressed")).toBe("true");
    expect(el.querySelector("[data-jev-route-hint]")!.textContent).toContain("start on Auto");
    // The privacy line is there whichever position is selected.
    expect(el.querySelector("[data-jev-route-hint]")!.textContent).toContain("first message of each new session");
  });

  it("cannot be changed while Jev itself is off", async () => {
    const el = await mount({ enabled: false, hasApiKey: false, backend: null, route: "off" });
    expect(el.querySelector<HTMLButtonElement>('button[data-jev-route="on"]')!.disabled).toBe(true);
  });
});
