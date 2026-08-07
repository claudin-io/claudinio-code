import { describe, it, expect, vi, afterEach } from "vitest";
import { render } from "solid-js/web";
import { createSignal } from "solid-js";
import {
  inspectPlugin,
  installPluginFromPath,
  installPluginFromUrl,
  listPlugins,
  pickFolder,
  scaffoldPlugin,
  setPluginEnabled,
  uninstallPlugin,
  type PluginInfo,
} from "../../lib/ipc";
import { SettingsPlugins } from "./SettingsPlugins";

vi.mock("../../lib/ipc", () => ({
  listPlugins: vi.fn(),
  inspectPlugin: vi.fn(),
  installPluginFromPath: vi.fn(),
  installPluginFromUrl: vi.fn(),
  uninstallPlugin: vi.fn(),
  setPluginEnabled: vi.fn(),
  scaffoldPlugin: vi.fn(),
  pickFolder: vi.fn(),
  openExternalUrl: vi.fn(),
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

const plugin = (over: Partial<PluginInfo> = {}): PluginInfo => ({
  name: "demo",
  root: "/home/ada/.claudinio/plugins/demo",
  scope: "user",
  enabled: true,
  valid: true,
  version: "1.2.0",
  description: "Does a thing",
  keywords: [],
  skills: [{ name: "summarize", description: "Summarize docs", location: "/a/SKILL.md" }],
  mcpServers: [{ name: "api", qualifiedName: "demo.api", transport: "streamable-http" }],
  diagnostics: [],
  ...over,
});

async function mount(list: PluginInfo[] = [plugin()], workspace: string | null = "/ws") {
  vi.mocked(listPlugins).mockResolvedValue(list);
  const [ws] = createSignal(workspace);
  host = document.createElement("div");
  document.body.appendChild(host);
  dispose = render(() => <SettingsPlugins workspaceRoot={ws} />, host);
  await Promise.resolve();
  await Promise.resolve();
  return host;
}

/** Exact-label lookup: "Install" and "Install from folder" are both buttons,
 * so a substring match would click the wrong one. */
function buttonWithText(root: HTMLElement, text: string): HTMLButtonElement {
  const match = [...root.querySelectorAll("button")].find(
    (b) => (b.textContent ?? "").trim() === text,
  );
  if (!match) throw new Error(`no button labelled "${text}"`);
  return match as HTMLButtonElement;
}

describe("SettingsPlugins", () => {
  it("lists installed plugins with their components", async () => {
    const el = await mount();
    expect(el.textContent).toContain("demo");
    expect(el.textContent).toContain("v1.2.0");
    expect(el.textContent).toContain("1 skill");
    expect(el.textContent).toContain("1 MCP server");
  });

  it("shows an empty state when nothing is installed", async () => {
    const el = await mount([]);
    expect(el.textContent).toContain("No plugins installed yet.");
  });

  it("surfaces diagnostics for a rejected plugin and hides the toggle", async () => {
    const el = await mount([
      plugin({
        valid: false,
        diagnostics: [{ severity: "error", message: "missing plugin.json" }],
      }),
    ]);
    expect(el.textContent).toContain("invalid");
    expect(el.textContent).toContain("missing plugin.json");
    expect(() => buttonWithText(el, "Disable")).toThrow();
  });

  it("toggles a plugin and refreshes the list", async () => {
    const el = await mount();
    vi.mocked(setPluginEnabled).mockResolvedValue([]);
    buttonWithText(el, "Disable").click();
    await Promise.resolve();
    expect(setPluginEnabled).toHaveBeenCalledWith("demo", false, "/ws");
  });

  it("uninstalls a plugin", async () => {
    const el = await mount();
    vi.mocked(uninstallPlugin).mockResolvedValue([]);
    buttonWithText(el, "Uninstall").click();
    await Promise.resolve();
    expect(uninstallPlugin).toHaveBeenCalledWith("demo", "/ws");
  });

  it("installs from a picked folder after validating it", async () => {
    const el = await mount();
    vi.mocked(pickFolder).mockResolvedValue("/src/my-plugin");
    vi.mocked(inspectPlugin).mockResolvedValue(plugin({ name: "my-plugin" }));
    vi.mocked(installPluginFromPath).mockResolvedValue(plugin({ name: "my-plugin" }));

    buttonWithText(el, "Install from folder").click();
    await new Promise((r) => setTimeout(r, 0));

    expect(installPluginFromPath).toHaveBeenCalledWith({
      path: "/src/my-plugin",
      scope: "user",
      workspace: "/ws",
    });
  });

  it("refuses to install a folder that is not a valid plugin", async () => {
    const el = await mount();
    vi.mocked(pickFolder).mockResolvedValue("/src/not-a-plugin");
    vi.mocked(inspectPlugin).mockResolvedValue(
      plugin({ valid: false, diagnostics: [{ severity: "error", message: "missing plugin.json" }] }),
    );

    buttonWithText(el, "Install from folder").click();
    await new Promise((r) => setTimeout(r, 0));

    expect(installPluginFromPath).not.toHaveBeenCalled();
    expect(el.textContent).toContain("missing plugin.json");
  });

  it("installs from a git URL", async () => {
    const el = await mount();
    vi.mocked(installPluginFromUrl).mockResolvedValue(plugin());

    buttonWithText(el, "Install from URL").click();
    await Promise.resolve();
    const input = el.querySelector<HTMLInputElement>('input[placeholder^="https://github.com"]')!;
    input.value = "https://github.com/acme/tools/tree/main/plugins/deploy";
    input.dispatchEvent(new Event("input", { bubbles: true }));

    buttonWithText(el, "Install").click();
    await new Promise((r) => setTimeout(r, 0));

    expect(installPluginFromUrl).toHaveBeenCalledWith({
      url: "https://github.com/acme/tools/tree/main/plugins/deploy",
      gitRef: null,
      subdir: null,
      scope: "user",
      workspace: "/ws",
    });
  });

  it("reports an error instead of installing an empty URL", async () => {
    const el = await mount();
    buttonWithText(el, "Install from URL").click();
    await Promise.resolve();
    buttonWithText(el, "Install").click();
    await new Promise((r) => setTimeout(r, 0));
    expect(installPluginFromUrl).not.toHaveBeenCalled();
    expect(el.textContent).toContain("Paste a git or GitHub URL first.");
  });

  it("scaffolds a new plugin through the crafter form", async () => {
    const el = await mount();
    vi.mocked(scaffoldPlugin).mockResolvedValue({
      root: "/home/ada/.claudinio/plugins/new-thing",
      files: ["plugin.json"],
      plugin: plugin({ name: "new-thing" }),
    });

    buttonWithText(el, "Create plugin").click();
    await Promise.resolve();
    const nameInput = el.querySelector<HTMLInputElement>('input[placeholder^="plugin-name"]')!;
    nameInput.value = "new-thing";
    nameInput.dispatchEvent(new Event("input", { bubbles: true }));

    buttonWithText(el, "Create").click();
    await new Promise((r) => setTimeout(r, 0));

    expect(scaffoldPlugin).toHaveBeenCalledWith(
      expect.objectContaining({ name: "new-thing", scope: "user", workspace: "/ws" }),
    );
    expect(el.textContent).toContain("/home/ada/.claudinio/plugins/new-thing");
  });
});
