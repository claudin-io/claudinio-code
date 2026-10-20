import { describe, it, expect, vi, afterEach } from "vitest";
import { createSignal } from "solid-js";
import { render } from "solid-js/web";
import { getConfig, setConfig, type AgentConfig } from "../../lib/ipc";
import { SettingsToolSurface } from "./SettingsToolSurface";

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

async function mount(cfg: Partial<AgentConfig>, initial = "claudinio") {
  vi.mocked(getConfig).mockResolvedValue(cfg as AgentConfig);
  const [model, setModel] = createSignal(initial);
  host = document.createElement("div");
  document.body.appendChild(host);
  dispose = render(() => <SettingsToolSurface model={model} />, host);
  await flush();
  return { el: host, setModel };
}

const pressed = (el: HTMLElement, model: string) =>
  el
    .querySelector(`[data-surface-row="${model}"] button[aria-pressed="true"]`)
    ?.getAttribute("data-surface");

// One timeline: one model per session, so one row — the session's model,
// for the main session and its subagents alike.
describe("SettingsToolSurface", () => {
  it("shows the session's model, on the full catalog until set", async () => {
    const { el } = await mount({});
    expect(el.querySelectorAll("[data-surface-row]").length).toBe(1);
    expect(pressed(el, "claudinio")).toBe("full");
    expect(el.textContent).not.toMatch(/Brain|Builder/);
  });

  it("shows the surface saved for the model", async () => {
    const { el } = await mount({ toolSurface: { claudinio: "lean" } });
    expect(pressed(el, "claudinio")).toBe("lean");
  });

  it("saves a choice for that model", async () => {
    const { el } = await mount({});
    el.querySelector<HTMLButtonElement>(
      '[data-surface-row="claudinio"] button[data-surface="lean"]',
    )!.click();
    await flush();
    expect(setConfig).toHaveBeenCalledWith({
      toolSurface: { model: "claudinio", surface: "lean" },
    });
    expect(pressed(el, "claudinio")).toBe("lean");
  });

  it("follows the model picked above", async () => {
    const { el, setModel } = await mount({});
    setModel("claudius");
    expect(el.querySelector('[data-surface-row="claudius"]')).not.toBeNull();
  });

  it("says when the model runs the compact profile instead", async () => {
    const { el, setModel } = await mount({ profile: "compact", model: "local/qwen" }, "local/qwen");
    expect(el.querySelector("[data-compact-note]")).not.toBeNull();
    // A model picked but not saved yet is not the one the note was computed for.
    setModel("claudinio");
    expect(el.querySelector("[data-compact-note]")).toBeNull();
  });

  it("says nothing about compact for a model with room", async () => {
    const { el } = await mount({ profile: "standard", model: "claudinio" });
    expect(el.querySelector("[data-compact-note]")).toBeNull();
  });
});
