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

async function mount(cfg: Partial<AgentConfig>, brain = "claudius", builder = "claudinio") {
  vi.mocked(getConfig).mockResolvedValue(cfg as AgentConfig);
  const [brainModel] = createSignal(brain);
  const [builderModel, setBuilderModel] = createSignal(builder);
  host = document.createElement("div");
  document.body.appendChild(host);
  dispose = render(
    () => <SettingsToolSurface brainModel={brainModel} builderModel={builderModel} />,
    host,
  );
  await flush();
  return { el: host, setBuilderModel };
}

const pressed = (el: HTMLElement, model: string) =>
  el
    .querySelector(`[data-surface-row="${model}"] button[aria-pressed="true"]`)
    ?.getAttribute("data-surface");

describe("SettingsToolSurface", () => {
  it("shows every model on the full catalog until one is set", async () => {
    const { el } = await mount({});
    expect(pressed(el, "claudius")).toBe("full");
    expect(pressed(el, "claudinio")).toBe("full");
  });

  it("shows the surface saved for each model", async () => {
    const { el } = await mount({ toolSurface: { claudinio: "lean" } });
    expect(pressed(el, "claudinio")).toBe("lean");
    expect(pressed(el, "claudius")).toBe("full");
  });

  it("saves a choice for that model alone", async () => {
    const { el } = await mount({});
    el.querySelector<HTMLButtonElement>(
      '[data-surface-row="claudinio"] button[data-surface="lean"]',
    )!.click();
    await flush();
    expect(setConfig).toHaveBeenCalledWith({
      toolSurface: { model: "claudinio", surface: "lean" },
    });
    expect(pressed(el, "claudinio")).toBe("lean");
    expect(pressed(el, "claudius")).toBe("full");
  });

  it("lists a model once when it plays both roles", async () => {
    const { el } = await mount({}, "claudinio", "claudinio");
    expect(el.querySelectorAll("[data-surface-row]").length).toBe(1);
    expect(el.textContent).toContain("Brain and Builder");
  });

  it("says when the builder model runs the compact profile instead", async () => {
    const { el, setBuilderModel } = await mount(
      { builderProfile: "compact", builderModel: "local/qwen" },
      "claudius",
      "local/qwen",
    );
    expect(el.querySelector("[data-compact-note]")).not.toBeNull();
    // A model picked but not saved yet is not the one the note was computed for.
    setBuilderModel("claudinio");
    expect(el.querySelector("[data-compact-note]")).toBeNull();
  });

  it("says nothing about compact for a model with room", async () => {
    const { el } = await mount({ builderProfile: "standard", builderModel: "claudinio" });
    expect(el.querySelector("[data-compact-note]")).toBeNull();
  });
});
