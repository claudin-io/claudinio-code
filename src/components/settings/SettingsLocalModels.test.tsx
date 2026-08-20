import { describe, it, expect, vi, afterEach } from "vitest";
import { render } from "solid-js/web";
import {
  getConfig,
  openExternalUrl,
  setConfig,
  localCancelInstall,
  localInstallModel,
  localRemoveModel,
  localRepoQuants,
  localCuratedModels,
  localDiskUsage,
  localHardware,
  localListModels,
  localStatus,
  type SuggestedModel,
  type LocalModelView,
  type LocalStatus,
  type ModelDownloadProgress,
} from "../../lib/ipc";
import { SettingsLocalModels } from "./SettingsLocalModels";

vi.mock("../../lib/ipc", () => ({
  getConfig: vi.fn(),
  setConfig: vi.fn(),
  localStatus: vi.fn(),
  localHardware: vi.fn(),
  localCuratedModels: vi.fn(),
  localListModels: vi.fn(),
  localDiskUsage: vi.fn(),
  localInstallServer: vi.fn(),
  localUninstallServer: vi.fn(),
  localSearchModels: vi.fn(),
  localRepoQuants: vi.fn(),
  localInstallModel: vi.fn(),
  localCancelInstall: vi.fn(),
  openExternalUrl: vi.fn(),
  localRemoveModel: vi.fn(),
  localUnloadModel: vi.fn(),
  localServerLogs: vi.fn(),
  localTestModel: vi.fn(),
}));

/** Captured so a test can push a progress event the way the backend would. */
const listeners = new Map<string, (e: { payload: unknown }) => void>();
vi.mock("@tauri-apps/api/event", () => ({
  listen: vi.fn(async (name: string, cb: (e: { payload: unknown }) => void) => {
    listeners.set(name, cb);
    return () => listeners.delete(name);
  }),
}));

let dispose: (() => void) | undefined;
let host: HTMLDivElement | undefined;
const onChanged = vi.fn();

afterEach(() => {
  dispose?.();
  host?.remove();
  dispose = undefined;
  host = undefined;
  listeners.clear();
  onChanged.mockReset();
  vi.resetAllMocks();
});

const status = (over: Partial<LocalStatus> = {}): LocalStatus => ({
  supported: true,
  build: "b10502",
  target: "macos-arm64",
  serverInstalled: true,
  exePath: "/data/llama/llama-server",
  downloadSize: 11_087_640,
  systemServer: null,
  mlxSupported: false,
  mlxInstalled: false,
  mlxVersion: "v0.1.0",
  mlxDownloadSize: 50_000_000,
  engine: "llamacpp",
  ...over,
});

const model = (over: Partial<LocalModelView> = {}): LocalModelView => ({
  key: "518f7f28dd2a35e7",
  displayName: "Qwen3-8B-GGUF (Q4_K_M)",
  repo: "unsloth/Qwen3-8B-GGUF",
  quant: "Q4_K_M",
  totalBytes: 4_700_000_000,
  contextLength: 40960,
  hasChatTemplate: true,
  architecture: "qwen3",
  format: "gguf",
  installedAt: "2026-08-19T00:00:00Z",
  running: false,
  complete: true,
  fit: "comfortable",
  ...over,
});

function buttonWithText(root: HTMLElement, text: string): HTMLButtonElement {
  const match = [...root.querySelectorAll("button")].find(
    (b) => (b.textContent ?? "").trim() === text,
  );
  if (!match) throw new Error(`no button labelled "${text}"`);
  return match as HTMLButtonElement;
}

async function mount(
  over: {
    status?: LocalStatus;
    models?: LocalModelView[];
    enabled?: boolean;
    curated?: SuggestedModel[];
  } = {},
) {
  vi.mocked(getConfig).mockResolvedValue({
    local: { enabled: over.enabled ?? true, engine: "llamacpp" },
  } as never);
  vi.mocked(localStatus).mockResolvedValue(over.status ?? status());
  vi.mocked(localHardware).mockResolvedValue({
    totalRamBytes: 69_000_000_000,
    availableRamBytes: 30_000_000_000,
    vramBytes: null,
    unifiedMemory: true,
    logicalCores: 12,
    gpuName: null,
  });
  vi.mocked(localCuratedModels).mockResolvedValue(over.curated ?? []);
  vi.mocked(localListModels).mockResolvedValue(over.models ?? []);
  vi.mocked(localDiskUsage).mockResolvedValue(0);

  host = document.createElement("div");
  document.body.appendChild(host);
  dispose = render(() => <SettingsLocalModels onChanged={onChanged} />, host);
  for (let i = 0; i < 6; i++) await Promise.resolve();
  return host;
}

function emitProgress(p: ModelDownloadProgress) {
  const cb = listeners.get("local-model-download-progress");
  if (!cb) throw new Error("component never subscribed to download progress");
  cb({ payload: p });
}

const progress = (over: Partial<ModelDownloadProgress> = {}): ModelDownloadProgress => ({
  key: "abc",
  fileIndex: 2,
  fileCount: 3,
  downloadedBytes: 1_000_000_000,
  totalBytes: 5_000_000_000,
  overallDone: 6_000_000_000,
  overallTotal: 15_000_000_000,
  phase: "download",
  ...over,
});

describe("SettingsLocalModels", () => {
  it("reports the pinned runtime and the detected hardware", async () => {
    const el = await mount();
    expect(el.textContent).toContain("llama.cpp b10502");
    expect(el.textContent).toContain("installed");
    expect(el.textContent).toContain("69 GB RAM");
    expect(el.textContent).toContain("unified memory");
  });

  it("offers the download when the runtime is missing", async () => {
    const el = await mount({ status: status({ serverInstalled: false, exePath: null }) });
    expect(el.textContent).toContain("not installed");
    expect(el.textContent).toContain("11 MB");
  });

  it("says so when the platform has no build", async () => {
    const el = await mount({ status: status({ supported: false, target: null }) });
    expect(el.textContent).toContain("no build for this platform");
  });

  it("lists installed models with size and context", async () => {
    const el = await mount({ models: [model()] });
    expect(el.textContent).toContain("Qwen3-8B-GGUF (Q4_K_M)");
    expect(el.textContent).toContain("4.7 GB");
    expect(el.textContent).toContain("up to 40960 ctx");
  });

  /// Trending is a reason to go look at a model before committing 20 GB of
  /// download to it.
  it("opens an installed model's page on the Hub", async () => {
    const el = await mount({ models: [model()] });
    const link = el.querySelector<HTMLButtonElement>('button[title="Open on Hugging Face"]');
    expect(link).toBeTruthy();
    link!.click();
    expect(openExternalUrl).toHaveBeenCalledWith(
      "https://huggingface.co/unsloth/Qwen3-8B-GGUF",
    );
  });

  /// The point of measuring here: two models on the same machine, comparable
  /// in a way their published numbers are not.
  it("shows what a model has cost on this machine", async () => {
    const el = await mount({
      models: [
        model({
          benchmark: {
            modelKey: "518f7f28dd2a35e7",
            loadSeconds: 42.5,
            loadSamples: 3,
            firstTokenSeconds: 6.2,
            tokensPerSecond: 13.9,
            promptTokensPerSecond: 240.9,
            generationSamples: 5,
            lastPromptTokens: 12000,
            lastRunAt: "2026-08-20T00:00:00Z",
          },
        }),
      ],
    });
    expect(el.textContent).toContain("13.9 tok/s");
    expect(el.textContent).toContain("6.2s to first token");
    expect(el.textContent).toContain("43s to load");
  });

  /// A model that has never run has nothing to say, and an empty row of zeroes
  /// would read as "this model is broken".
  it("says nothing about a model that has never run", async () => {
    const el = await mount({ models: [model()] });
    expect(el.textContent).not.toContain("to first token");
  });

  it("warns when a model has no chat template", async () => {
    const el = await mount({ models: [model({ hasChatTemplate: false })] });
    expect(el.textContent).toContain("tool calls will not work");
  });

  /// A three-shard download whose bar tracked the current file would read 20%
  /// while 40% of the bytes were already down, and reset twice on the way.
  it("drives the progress bar from the overall byte count, not the current file", async () => {
    const el = await mount();
    emitProgress(progress());
    await Promise.resolve();

    const bar = el.querySelector<HTMLElement>('[style*="width"]');
    expect(bar?.style.width).toBe("40%");
    expect(el.textContent).toContain("6.0 GB / 15.0 GB");
    expect(el.textContent).toContain("file 2 of 3");
  });

  it("cancels the download that is actually running", async () => {
    const el = await mount();
    emitProgress(progress({ key: "the-running-one" }));
    await Promise.resolve();

    const cancel = [...el.querySelectorAll("button")].find(
      (b) => (b.textContent ?? "").trim() === "Cancel",
    );
    expect(cancel).toBeTruthy();
    cancel!.click();
    await Promise.resolve();
    expect(localCancelInstall).toHaveBeenCalledWith("the-running-one");
  });

  /// The pickers are fed by a list App.tsx fetches when Settings opens. A model
  /// downloaded after that is invisible until something asks for the list
  /// again — which is exactly how a working download looked broken.
  it("asks for the model list again once a download finishes", async () => {
    const el = await mount({
      curated: [
        {
          repo: "unsloth/Qwen3-8B-GGUF",
          displayName: "Qwen3-8B-GGUF",
          downloads: 1000,
          likes: 10,
          blurb: null,
          offline: false,
        },
      ],
    });
    vi.mocked(localRepoQuants).mockResolvedValue({
      repo: "unsloth/Qwen3-8B-GGUF",
      gated: false,
      quants: [{ quant: "Q4_K_M", totalBytes: 4_700_000_000, shards: 1, fit: "comfortable" }],
      recommended: "Q4_K_M",
      contextLength: 40960,
      hasChatTemplate: true,
      architecture: "qwen3",
      format: "gguf",
    });
    vi.mocked(localInstallModel).mockResolvedValue(model());

    buttonWithText(el, "Choose").click();
    for (let i = 0; i < 6; i++) await Promise.resolve();

    buttonWithText(el, "Download").click();
    for (let i = 0; i < 8; i++) await Promise.resolve();

    expect(localInstallModel).toHaveBeenCalledWith("unsloth/Qwen3-8B-GGUF", "Q4_K_M");
    expect(onChanged).toHaveBeenCalled();
    expect(el.textContent).toContain("Settings → Models");
  });

  /// "Remove" (a model) and "Remove runtime" are different buttons; an exact
  /// label match is what keeps this test honest about which one it clicked.
  it("re-reads the model list when a model is removed", async () => {
    const el = await mount({ models: [model()] });
    vi.mocked(localRemoveModel).mockResolvedValue(undefined);
    buttonWithText(el, "Remove").click();
    for (let i = 0; i < 6; i++) await Promise.resolve();
    expect(localRemoveModel).toHaveBeenCalledWith(model().key);
    expect(onChanged).toHaveBeenCalled();
  });

  it("re-reads the model list when the feature is switched on", async () => {
    const el = await mount({ enabled: false });
    vi.mocked(setConfig).mockResolvedValue(undefined);
    const box = el.querySelector<HTMLInputElement>('input[type="checkbox"]');
    box!.checked = true;
    box!.dispatchEvent(new Event("change", { bubbles: true }));
    for (let i = 0; i < 6; i++) await Promise.resolve();
    expect(onChanged).toHaveBeenCalled();
  });

  /// The dead end the feature shipped with: weights on disk, switch off, and
  /// nothing in the picker to explain why.
  it("explains why downloaded models are missing from the pickers", async () => {
    const el = await mount({ enabled: false, models: [model()] });
    expect(el.textContent).toContain("local models are switched off");
  });

  it("does not nag when nothing is downloaded yet", async () => {
    const el = await mount({ enabled: false, models: [] });
    expect(el.textContent).not.toContain("local models are switched off");
  });

  it("hides the progress bar once the download is done", async () => {
    const el = await mount();
    emitProgress(progress({ phase: "done" }));
    await Promise.resolve();
    expect(el.textContent).not.toContain("Cancel");
  });
});
