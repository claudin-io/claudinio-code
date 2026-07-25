import { describe, it, expect, vi, beforeEach, afterEach } from "vitest";
import { createSignal } from "solid-js";
import { render } from "solid-js/web";
import { invoke } from "@tauri-apps/api/core";
import { SettingsPanel } from "./SettingsPanel";

const mocked = vi.mocked(invoke);

/// The panel animates in over a `requestAnimationFrame`, and the remote probe is
/// three awaited IPC calls deep. Both need real macrotasks in jsdom.
const flush = () => new Promise((r) => setTimeout(r, 20));

let dispose: (() => void) | undefined;
let host: HTMLDivElement | undefined;

afterEach(() => {
  dispose?.();
  host?.remove();
  dispose = undefined;
  host = undefined;
});

beforeEach(() => {
  mocked.mockReset();
});

/// A build with remote access compiled in, with nothing paired yet.
function remoteIsAvailable() {
  mocked.mockImplementation(async (cmd: string) => {
    switch (cmd) {
      case "remote_status":
        return { enabled: false, deviceKey: "aa".repeat(32), error: null };
      case "remote_policy":
        return {
          path: "/tmp/remote-policy.json",
          active: false,
          inertBecause: "remote access is off",
          effective: {
            send_message: false,
            steer: false,
            interrupt: false,
            set_mode: false,
            approve_edit: false,
            approve_bash: "never",
            read_attachment: false,
            export_file: false,
          },
          workspaces: [],
          bashDenylist: [],
        };
      case "remote_pairings":
        return [];
      case "remote_revoked":
        return [];
      default:
        throw new Error(`Command ${cmd} not found`);
    }
  });
}

/// A default build: no remote commands exist at all.
function remoteIsNotCompiledIn() {
  mocked.mockImplementation(async (cmd: string) => {
    throw new Error(`Command ${cmd} not found`);
  });
}

/// `SettingsPanel` takes every config field as a signal pair. None of them matter
/// to what is tested here, so they are filled mechanically and left alone.
function baseProps(showConfig: () => boolean) {
  const sig = <T,>(v: T) => {
    const [get, set] = createSignal<T>(v);
    return [get, set] as const;
  };
  const [brainModel, setBrainModel] = sig("");
  const [builderModel, setBuilderModel] = sig("");
  const [maxParallelAgents, setMaxParallelAgents] = sig(1);
  const [maxRounds, setMaxRounds] = sig<number | null>(null);
  const [subMaxRounds, setSubMaxRounds] = sig<number | null>(null);
  const [maxGoldenCycles, setMaxGoldenCycles] = sig<number | null>(null);
  const [maxGoldenStalls, setMaxGoldenStalls] = sig<number | null>(null);
  const [handoffTokens, setHandoffTokens] = sig(0);
  const [yoloMode, setYoloMode] = sig(false);
  const [yoloBlacklist, setYoloBlacklist] = sig("");
  const [keepAwake, setKeepAwake] = sig(false);
  const [codeIntel, setCodeIntel] = sig(false);
  const [autoCommitPlan, setAutoCommitPlan] = sig(false);
  const [preferredIde, setPreferredIde] = sig("");
  const [planSavePath, setPlanSavePath] = sig("");
  const [apiKey, setApiKey] = sig("");
  const [mcpJson, setMcpJson] = sig("{}");
  const [mcpJsonError, setMcpJsonError] = sig<string | null>(null);
  const [mcpTesting, setMcpTesting] = sig(false);
  const [overrideBaseUrl, setOverrideBaseUrl] = sig("");
  const [overrideApiKey, setOverrideApiKey] = sig("");

  return {
    showConfig,
    setShowConfig: vi.fn(),
    configBrainModel: brainModel,
    setConfigBrainModel: setBrainModel,
    configBuilderModel: builderModel,
    setConfigBuilderModel: setBuilderModel,
    modelGroups: () => [],
    configMaxParallelAgents: maxParallelAgents,
    setConfigMaxParallelAgents: setMaxParallelAgents,
    configMaxRounds: maxRounds,
    setConfigMaxRounds: setMaxRounds,
    configSubMaxRounds: subMaxRounds,
    setConfigSubMaxRounds: setSubMaxRounds,
    configMaxGoldenCycles: maxGoldenCycles,
    setConfigMaxGoldenCycles: setMaxGoldenCycles,
    configMaxGoldenStalls: maxGoldenStalls,
    setConfigMaxGoldenStalls: setMaxGoldenStalls,
    configHandoffTokens: handoffTokens,
    setConfigHandoffTokens: setHandoffTokens,
    configYoloMode: yoloMode,
    setConfigYoloMode: setYoloMode,
    configYoloBlacklist: yoloBlacklist,
    setConfigYoloBlacklist: setYoloBlacklist,
    configKeepAwake: keepAwake,
    setConfigKeepAwake: setKeepAwake,
    configCodeIntelEnabled: codeIntel,
    setConfigCodeIntelEnabled: setCodeIntel,
    configAutoCommitPlan: autoCommitPlan,
    setConfigAutoCommitPlan: setAutoCommitPlan,
    configPreferredIde: preferredIde,
    setConfigPreferredIde: setPreferredIde,
    availableIdes: () => [],
    configPlanSavePath: planSavePath,
    setConfigPlanSavePath: setPlanSavePath,
    workspaceConfigFields: () => new Set<string>(),
    accountLogin: () => null,
    hasApiKey: () => false,
    loggingIn: () => false,
    configApiKey: apiKey,
    setConfigApiKey: setApiKey,
    settingsApiKeyError: () => null,
    configMcpJson: mcpJson,
    setConfigMcpJson: setMcpJson,
    mcpJsonError,
    setMcpJsonError,
    mcpStatuses: () => ({}),
    mcpTesting,
    setMcpTesting,
    easterEggActive: () => false,
    configOverrideBaseUrl: overrideBaseUrl,
    setConfigOverrideBaseUrl: setOverrideBaseUrl,
    configOverrideApiKey: overrideApiKey,
    setConfigOverrideApiKey: setOverrideApiKey,
    providers: () => ({}),
    openrouterConnecting: () => false,
    providerError: () => null,
    onOpenrouterConnect: vi.fn(),
    onOpenrouterCancel: vi.fn(),
    onDisconnectProvider: vi.fn(),
    onOpenProviderCatalog: vi.fn(),
    saveConfig: vi.fn().mockResolvedValue(undefined),
    doLogin: vi.fn().mockResolvedValue(undefined),
    doLogout: vi.fn().mockResolvedValue(undefined),
    pickPlanPath: vi.fn().mockResolvedValue(undefined),
    addMcpServerTemplate: vi.fn(),
    testAllMcpServers: vi.fn().mockResolvedValue(undefined),
    openSupportUrl: vi.fn(),
  };
}

async function mount(open = true) {
  host = document.createElement("div");
  document.body.appendChild(host);
  const [showConfig, setShowConfig] = createSignal(open);
  const props = baseProps(showConfig);
  // eslint-disable-next-line @typescript-eslint/no-explicit-any
  dispose = render(() => <SettingsPanel {...(props as any)} />, host);
  await flush();
  return { root: document.body, props, setShowConfig };
}

const tab = (label: string) =>
  Array.from(document.querySelectorAll<HTMLButtonElement>(".settings-category-item")).find(
    (b) => b.textContent?.trim() === label,
  );

const footerButtons = () =>
  Array.from(document.querySelectorAll<HTMLButtonElement>(".settings-panel-footer button")).map(
    (b) => b.textContent?.trim(),
  );

describe("SettingsPanel", () => {
  /// Remote access is a compile-time feature. On a build without it the tab is
  /// absent rather than greyed out: a disabled tab sends the user looking for a
  /// switch that turns it on, and there is none.
  it("does not offer a Remote tab when the build has no remote commands", async () => {
    remoteIsNotCompiledIn();
    await mount();

    expect(tab("Remote")).toBeUndefined();
    expect(tab("General")).toBeTruthy();
  });

  it("offers a Remote tab when the device answers", async () => {
    remoteIsAvailable();
    await mount();

    expect(tab("Remote")).toBeTruthy();
  });

  it("shows the remote panel when the tab is chosen", async () => {
    remoteIsAvailable();
    await mount();

    tab("Remote")!.click();
    await flush();

    expect(document.body.textContent).toContain("Paired browsers");
    expect(document.body.textContent).toContain("Public by design");
  });

  /// Every other tab stages edits that Save commits. Remote does not: a revoke has
  /// already happened, so a Cancel button would promise an undo that cannot exist.
  it("replaces Cancel and Save with Close on the Remote tab", async () => {
    remoteIsAvailable();
    await mount();

    expect(footerButtons()).toEqual(["Cancel", "Save"]);

    tab("Remote")!.click();
    await flush();

    expect(footerButtons()).toEqual(["Close"]);
  });

  it("keeps Save on every other tab", async () => {
    remoteIsAvailable();
    await mount();

    tab("Remote")!.click();
    await flush();
    tab("Models")!.click();
    await flush();

    expect(footerButtons()).toEqual(["Cancel", "Save"]);
  });

  /// Reading the identity off disk is work the user has no reason to pay for
  /// before they open settings.
  it("does not read the device identity until the panel is opened", async () => {
    remoteIsAvailable();
    const { setShowConfig } = await mount(false);

    expect(mocked).not.toHaveBeenCalled();

    setShowConfig(true);
    await flush();

    expect(mocked).toHaveBeenCalledWith("remote_status");
  });

  it("finds the remote panel by searching for what is in it", async () => {
    remoteIsAvailable();
    await mount();

    const search = document.querySelector<HTMLInputElement>(".settings-search input")!;
    search.value = "paired browsers";
    search.dispatchEvent(new Event("input", { bubbles: true }));
    await flush();

    expect(tab("Remote")).toBeTruthy();
    expect(tab("Models")).toBeUndefined();
  });
});
