import { describe, it, expect, vi, afterEach } from "vitest";
import { render } from "solid-js/web";
import { getConfig, setConfig, type AgentConfig } from "../../lib/ipc";
import { SettingsProjectMemory } from "./SettingsProjectMemory";

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

async function mount(projectMemory: AgentConfig["projectMemory"]) {
  vi.mocked(getConfig).mockResolvedValue({ projectMemory } as AgentConfig);
  host = document.createElement("div");
  document.body.appendChild(host);
  dispose = render(() => <SettingsProjectMemory />, host);
  await flush();
  return host;
}

const box = (el: HTMLElement) => el.querySelector<HTMLInputElement>("input[data-project-memory]")!;

// Lean-harness Phase 9: the first run of a new session reads the project's
// AGENTS.md/CLAUDE.md and .claudinio/MEMORY.md; one switch turns both off.
describe("SettingsProjectMemory", () => {
  it("is on for a config written before the setting existed", async () => {
    const el = await mount(undefined);
    expect(box(el).checked).toBe(true);
  });

  it("is off when the user turned it off", async () => {
    const el = await mount(false);
    expect(box(el).checked).toBe(false);
  });

  it("saves the choice", async () => {
    const el = await mount(true);
    box(el).click();
    await flush();
    expect(setConfig).toHaveBeenCalledWith({ projectMemory: false });
    expect(box(el).checked).toBe(false);
  });

  it("says which files it reads and when", async () => {
    const el = await mount(true);
    const text = el.textContent!;
    expect(text).toContain("AGENTS.md");
    expect(text).toContain(".claudinio/MEMORY.md");
    expect(text).toContain("new session");
  });
});
