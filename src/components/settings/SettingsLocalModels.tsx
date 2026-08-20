import {
  createMemo,
  createResource,
  createSignal,
  For,
  onCleanup,
  onMount,
  Show,
  type Component,
} from "solid-js";
import { listen } from "@tauri-apps/api/event";
import {
  getConfig,
  setConfig,
  localCancelInstall,
  localCuratedModels,
  localDiskUsage,
  localHardware,
  localInstallModel,
  localInstallMlx,
  localInstallServer,
  localListModels,
  localRemoveModel,
  localRepoQuants,
  localSearchModels,
  localStatus,
  localTestModel,
  localUninstallMlx,
  localUninstallServer,
  localUnloadModel,
  type Fit,
  type LlamaBackend,
  type LocalEngine,
  type LocalPrefs,
  type ModelDownloadProgress,
  type RepoQuants,
} from "../../lib/ipc";
import { Icon } from "../Icon";

const DEFAULT_PREFS: LocalPrefs = {
  enabled: false,
  serverPath: null,
  backend: "auto",
  // Overwritten by whatever the backend reports on mount; the backend picks
  // MLX on Apple Silicon and llama.cpp elsewhere.
  engine: "llamacpp",
  ctxSize: 0,
  gpuLayers: "auto",
  parallel: 1,
  sleepIdleSeconds: 300,
  maxLoadedModels: 1,
};

function mb(bytes: number): string {
  if (bytes >= 1_000_000_000) return `${(bytes / 1_000_000_000).toFixed(1)} GB`;
  return `${Math.round(bytes / 1_000_000)} MB`;
}

const FIT_LABEL: Record<Fit, string> = {
  comfortable: "fits",
  tight: "tight",
  wontFit: "too big",
};

export const SettingsLocalModels: Component<{ onChanged?: () => void }> = (props) => {
  const [prefs, setPrefs] = createSignal<LocalPrefs>(DEFAULT_PREFS);
  const [status, { refetch: refetchStatus }] = createResource(localStatus);
  const [hardware] = createResource(localHardware);
  const [curated] = createResource(localCuratedModels);
  const [installed, { refetch: refetchInstalled }] = createResource(localListModels);
  const [usage, { refetch: refetchUsage }] = createResource(localDiskUsage);

  const [progress, setProgress] = createSignal<ModelDownloadProgress | null>(null);
  const [runtimeProgress, setRuntimeProgress] = createSignal<ModelDownloadProgress | null>(null);
  const [busy, setBusy] = createSignal(false);
  const [message, setMessage] = createSignal<{ kind: "ok" | "err"; text: string } | null>(null);

  const [query, setQuery] = createSignal("");
  const [results, setResults] = createSignal<{ repo: string; downloads: number; gated: boolean }[]>(
    [],
  );
  const [openRepo, setOpenRepo] = createSignal<RepoQuants | null>(null);
  const [downloadingKey, setDownloadingKey] = createSignal<string | null>(null);

  onMount(async () => {
    const cfg = await getConfig();
    if (cfg.local) setPrefs({ ...DEFAULT_PREFS, ...cfg.local });

    const un1 = await listen<ModelDownloadProgress>("local-model-download-progress", (e) => {
      setProgress(e.payload);
      setDownloadingKey(e.payload.phase === "done" ? null : e.payload.key);
    });
    const un2 = await listen<ModelDownloadProgress>("llama-server-install-progress", (e) =>
      setRuntimeProgress(e.payload),
    );
    onCleanup(() => {
      un1();
      un2();
    });
  });

  const save = async (patch: Partial<LocalPrefs>) => {
    const next = { ...prefs(), ...patch };
    setPrefs(next);
    await setConfig({ local: next });
    // The switch gates whether local models appear in the Brain/Builder
    // pickers at all, so the lists behind them are now stale.
    if (patch.enabled !== undefined) props.onChanged?.();
  };

  const run = async (fn: () => Promise<unknown>, ok?: string) => {
    setBusy(true);
    setMessage(null);
    try {
      const result = await fn();
      setMessage({ kind: "ok", text: ok ?? String(result ?? "Done.") });
    } catch (e) {
      setMessage({ kind: "err", text: String(e) });
    } finally {
      setBusy(false);
    }
  };

  const installMlx = () =>
    run(async () => {
      await localInstallMlx();
      await refetchStatus();
      setRuntimeProgress(null);
      return "MLX runtime installed.";
    });

  const removeMlx = () =>
    run(async () => {
      await localUninstallMlx();
      await refetchStatus();
      return "MLX runtime removed.";
    });

  const installRuntime = () =>
    run(async () => {
      await localInstallServer();
      await refetchStatus();
      setRuntimeProgress(null);
      return "Runtime installed.";
    });

  const removeRuntime = () =>
    run(async () => {
      await localUninstallServer();
      await refetchStatus();
      return "Runtime removed.";
    });

  const search = async () => {
    if (!query().trim()) return;
    setBusy(true);
    setMessage(null);
    setOpenRepo(null);
    try {
      setResults(await localSearchModels(query(), 20));
    } catch (e) {
      setMessage({ kind: "err", text: String(e) });
    } finally {
      setBusy(false);
    }
  };

  const openQuants = async (repo: string) => {
    setBusy(true);
    setMessage(null);
    try {
      setOpenRepo(await localRepoQuants(repo));
    } catch (e) {
      setOpenRepo(null);
      setMessage({ kind: "err", text: String(e) });
    } finally {
      setBusy(false);
    }
  };

  const download = async (repo: string, quant: string, fit: Fit) => {
    if (fit === "wontFit") {
      const ok = window.confirm(
        `This quantization is larger than the memory available for weights. ` +
          `It will most likely fail to load, or swap the machine to a standstill. Download anyway?`,
      );
      if (!ok) return;
    }
    setMessage(null);
    try {
      await localInstallModel(repo, quant);
      await Promise.all([refetchInstalled(), refetchUsage()]);
      props.onChanged?.();
      setMessage({
        kind: "ok",
        text: `${repo} (${quant}) installed — pick it under “Local” in Settings → Models.`,
      });
    } catch (e) {
      setMessage({ kind: "err", text: String(e) });
    } finally {
      setProgress(null);
      setDownloadingKey(null);
    }
  };

  const overallPct = createMemo(() => {
    const p = progress();
    if (!p || p.overallTotal === 0) return 0;
    return Math.min(100, Math.round((p.overallDone / p.overallTotal) * 100));
  });

  const runtimePct = createMemo(() => {
    const p = runtimeProgress();
    if (!p || p.totalBytes === 0) return 0;
    return Math.min(100, Math.round((p.downloadedBytes / p.totalBytes) * 100));
  });

  const hardwareLine = () => {
    const hw = hardware();
    if (!hw) return "";
    const parts = [`${(hw.totalRamBytes / 1_000_000_000).toFixed(0)} GB RAM`, `${hw.logicalCores} cores`];
    if (hw.unifiedMemory) parts.push("unified memory");
    if (hw.gpuName) parts.push(`${hw.gpuName}${hw.vramBytes ? ` (${mb(hw.vramBytes)})` : ""}`);
    return parts.join(" · ");
  };

  return (
    <div class="space-y-5">
      <div>
        <h3 class="text-sm font-medium text-ink">Local models</h3>
        <p class="mt-1 text-xs text-ink-faint">
          Runs models on this machine with llama.cpp. Nothing leaves the computer and nothing
          is billed — in exchange, a local model is slower and less capable than a hosted one.
          Downloaded models appear in the Brain and Builder pickers under “Local”.
        </p>
      </div>

      <label class="flex items-center gap-2 text-sm text-ink">
        <input
          type="checkbox"
          checked={prefs().enabled}
          onChange={(e) => save({ enabled: e.currentTarget.checked })}
        />
        Enable local models
        <span class="text-xs text-ink-faint">(they only appear in the model pickers when on)</span>
      </label>

      <Show when={!prefs().enabled && (installed() ?? []).some((m) => m.complete)}>
        <div class="flex items-start gap-2 rounded-md border border-border-subtle bg-surface-1 p-2 text-xs text-ink-faint">
          <Icon name="alert-triangle" class="mt-0.5 h-3 w-3 shrink-0" />
          <span>
            You have models downloaded, but local models are switched off — that is why they are
            not in the Brain and Builder pickers. Tick the box above.
          </span>
        </div>
      </Show>

      <Show when={status()?.mlxSupported}>
        <div class="rounded-md border border-border-subtle bg-surface-1 p-3">
          <div class="flex items-center justify-between gap-3">
            <div class="min-w-0">
              <div class="text-sm text-ink">Engine</div>
              <div class="text-[11px] text-ink-faint">
                MLX is Apple's framework for this hardware and generates faster on the same
                weights; llama.cpp reads GGUF and runs everywhere. They take different model
                formats, so each engine has its own downloads.
              </div>
            </div>
            <select
              class="shrink-0 rounded border border-border-subtle bg-surface-1 px-2 py-1 text-sm"
              value={prefs().engine}
              onChange={(e) => save({ engine: e.currentTarget.value as LocalEngine })}
            >
              <option value="mlx">MLX</option>
              <option value="llamacpp">llama.cpp</option>
            </select>
          </div>

          <Show when={prefs().engine === "mlx"}>
            <div class="mt-3 flex items-center justify-between gap-3">
              <div class="min-w-0">
                <div class="text-sm text-ink">
                  MLX runtime {status()?.mlxVersion}{" "}
                  <span class="text-ink-faint">
                    — {status()?.mlxInstalled ? "installed" : "not installed"}
                  </span>
                </div>
                <Show when={!status()?.mlxInstalled}>
                  <div class="text-[11px] text-ink-faint">
                    One-time download of {mb(status()?.mlxDownloadSize ?? 0)}.
                  </div>
                </Show>
              </div>
              <Show
                when={status()?.mlxInstalled}
                fallback={
                  <button
                    disabled={busy()}
                    onClick={installMlx}
                    class="shrink-0 rounded-md border border-border-subtle bg-surface-2 px-3 py-1.5 text-sm text-ink hover:bg-surface-3 disabled:opacity-50"
                  >
                    Download
                  </button>
                }
              >
                <button
                  disabled={busy()}
                  onClick={removeMlx}
                  class="shrink-0 rounded-md border border-border-subtle bg-surface-2 px-3 py-1.5 text-sm text-ink hover:bg-surface-3 disabled:opacity-50"
                >
                  Remove MLX
                </button>
              </Show>
            </div>
          </Show>
        </div>
      </Show>

      {/* Runtime */}
      <Show when={prefs().engine === "llamacpp" || !status()?.mlxSupported}>
      <div class="rounded-md border border-border-subtle bg-surface-1 p-3">
        <Show
          when={status()?.supported}
          fallback={
            <p class="text-xs text-ink-faint">
              llama.cpp publishes no build for this platform.
            </p>
          }
        >
          <div class="flex items-center justify-between gap-3">
            <div class="min-w-0">
              <div class="text-sm text-ink">
                llama.cpp {status()?.build}{" "}
                <span class="text-ink-faint">
                  — {status()?.serverInstalled ? "installed" : "not installed"}
                  <Show when={status()?.target}> ({status()!.target})</Show>
                </span>
              </div>
              <Show when={status()?.serverInstalled && status()?.exePath}>
                <div class="truncate text-[11px] text-ink-faint">{status()!.exePath}</div>
              </Show>
              <Show when={!status()?.serverInstalled}>
                <div class="text-[11px] text-ink-faint">
                  One-time download of {mb(status()?.downloadSize ?? 0)}.
                </div>
              </Show>
              <div class="text-[11px] text-ink-faint">{hardwareLine()}</div>
            </div>
            <div class="flex shrink-0 gap-2">
              <Show
                when={status()?.serverInstalled}
                fallback={
                  <button
                    disabled={busy()}
                    onClick={installRuntime}
                    class="rounded-md border border-border-subtle bg-surface-2 px-3 py-1.5 text-sm text-ink hover:bg-surface-3 disabled:opacity-50"
                  >
                    Download
                  </button>
                }
              >
                <button
                  disabled={busy()}
                  onClick={removeRuntime}
                  class="rounded-md border border-border-subtle bg-surface-2 px-3 py-1.5 text-sm text-ink hover:bg-surface-3 disabled:opacity-50"
                >
                  Remove runtime
                </button>
              </Show>
            </div>
          </div>

          <Show when={runtimeProgress()}>
            <div class="mt-3">
              <div class="h-1.5 overflow-hidden rounded-full bg-surface-3">
                <div class="h-full bg-accent transition-all" style={{ width: `${runtimePct()}%` }} />
              </div>
              <div class="mt-1 text-[11px] text-ink-faint">
                Downloading the runtime… {mb(runtimeProgress()!.downloadedBytes)} /{" "}
                {mb(runtimeProgress()!.totalBytes)}
              </div>
            </div>
          </Show>

          <div class="mt-3 flex flex-wrap items-center gap-3 text-sm text-ink">
            <span>Backend</span>
            <select
              class="rounded border border-border-subtle bg-surface-1 px-2 py-1 text-sm"
              value={prefs().backend}
              onChange={(e) => save({ backend: e.currentTarget.value as LlamaBackend })}
            >
              <option value="auto">Auto</option>
              <option value="vulkan">Vulkan (GPU)</option>
              <option value="cpu">CPU only</option>
            </select>
            <span class="text-xs text-ink-faint">
              Changing this downloads a different build.
            </span>
          </div>

          <div class="mt-3 flex flex-wrap items-center gap-3 text-sm text-ink">
            <span>Context</span>
            <input
              type="number"
              min="0"
              class="w-24 rounded border border-border-subtle bg-surface-1 px-2 py-1"
              value={prefs().ctxSize}
              onChange={(e) => save({ ctxSize: Number(e.currentTarget.value) })}
            />
            <span class="text-xs text-ink-faint">
              0 = follow the session handoff limit
            </span>
            <span>Unload after</span>
            <input
              type="number"
              min="0"
              class="w-20 rounded border border-border-subtle bg-surface-1 px-2 py-1"
              value={prefs().sleepIdleSeconds}
              onChange={(e) => save({ sleepIdleSeconds: Number(e.currentTarget.value) })}
            />
            <span class="text-xs text-ink-faint">seconds idle (0 = never)</span>
          </div>

          <p class="mt-1 text-xs text-ink-faint">
            A model often advertises far more context than a session ever reaches — and every
            unused token still costs memory the moment it loads. Left at 0, the window is sized
            to the handoff limit in Settings → Models, plus room for the reply.
          </p>

          <div class="mt-3">
            <label class="text-sm text-ink">Use an installed llama-server instead</label>
            <div class="mt-2 flex gap-2">
              <input
                type="text"
                placeholder={status()?.systemServer ?? "Path to a llama-server binary"}
                class="min-w-0 flex-1 rounded border border-border-subtle bg-surface-1 px-2 py-1 text-sm"
                value={prefs().serverPath ?? ""}
                onChange={(e) => save({ serverPath: e.currentTarget.value || null })}
              />
              <Show when={status()?.systemServer}>
                <button
                  onClick={() => save({ serverPath: status()!.systemServer! })}
                  class="shrink-0 rounded-md border border-border-subtle bg-surface-2 px-3 py-1.5 text-sm text-ink hover:bg-surface-3"
                >
                  Detect
                </button>
              </Show>
            </div>
          </div>
        </Show>
      </div>
      </Show>

      {/* Installed models */}
      <div>
        <div class="flex items-baseline justify-between">
          <h4 class="text-sm font-medium text-ink">Installed models</h4>
          <span class="text-[11px] text-ink-faint">{mb(usage() ?? 0)} on disk</span>
        </div>
        <Show
          when={(installed() ?? []).length > 0}
          fallback={<p class="mt-1 text-xs text-ink-faint">No models downloaded yet.</p>}
        >
          <div class="mt-2 space-y-2">
            <For each={installed()}>
              {(m) => (
                <div class="flex items-center justify-between gap-3 rounded-md border border-border-subtle bg-surface-1 p-2">
                  <div class="min-w-0">
                    <div class="truncate text-sm text-ink">
                      {m.displayName}
                      <Show when={m.running}>
                        <span class="ml-2 text-[11px] text-accent">running</span>
                      </Show>
                      <Show when={!m.complete}>
                        <span class="ml-2 text-[11px] text-danger">incomplete</span>
                      </Show>
                    </div>
                    <div class="truncate text-[11px] text-ink-faint">
                      {m.repo} · {mb(m.totalBytes)}
                      <Show when={m.contextLength}> · up to {m.contextLength} ctx</Show>
                      <Show when={!m.hasChatTemplate}>
                        {" "}
                        · <span class="text-danger">no chat template — tool calls will not work</span>
                      </Show>
                    </div>
                  </div>
                  <div class="flex shrink-0 gap-2">
                    <button
                      disabled={busy()}
                      onClick={() => run(() => localTestModel(m.key))}
                      class="rounded-md border border-border-subtle bg-surface-2 px-2 py-1 text-xs text-ink hover:bg-surface-3 disabled:opacity-50"
                    >
                      Test
                    </button>
                    <Show when={m.running}>
                      <button
                        disabled={busy()}
                        onClick={() =>
                          run(async () => {
                            await localUnloadModel(m.key);
                            await refetchInstalled();
                            return "Unloaded.";
                          })
                        }
                        class="rounded-md border border-border-subtle bg-surface-2 px-2 py-1 text-xs text-ink hover:bg-surface-3 disabled:opacity-50"
                      >
                        Unload
                      </button>
                    </Show>
                    <button
                      disabled={busy()}
                      onClick={() =>
                        run(async () => {
                          await localRemoveModel(m.key);
                          await Promise.all([refetchInstalled(), refetchUsage()]);
                          props.onChanged?.();
                          return "Removed.";
                        })
                      }
                      class="rounded-md border border-border-subtle bg-surface-2 px-2 py-1 text-xs text-ink hover:bg-surface-3 disabled:opacity-50"
                    >
                      Remove
                    </button>
                  </div>
                </div>
              )}
            </For>
          </div>
        </Show>
      </div>

      {/* Add a model */}
      <div>
        <h4 class="text-sm font-medium text-ink">Add a model</h4>
        <p class="mt-1 text-xs text-ink-faint">
          Suggestions are what the Hugging Face Hub is trending for the selected engine. Search
          the Hub, or paste a model's URL to go straight to it. Only models the selected engine
          can load are listed — the two engines read different formats.
        </p>

        <div class="mt-2 space-y-2">
          <Show when={curated()?.some((c) => c.offline)}>
            <p class="text-[11px] text-ink-faint">
              Could not reach the Hub — showing the built-in list instead.
            </p>
          </Show>
          <For each={curated()}>
            {(c) => (
              <div class="flex items-center justify-between gap-3 rounded-md border border-border-subtle bg-surface-1 p-2">
                <div class="min-w-0">
                  <div class="truncate text-sm text-ink">{c.displayName}</div>
                  <div class="truncate text-[11px] text-ink-faint">
                    {c.blurb ??
                      `${c.repo} · ${c.downloads.toLocaleString()} downloads`}
                  </div>
                </div>
                <button
                  disabled={busy()}
                  onClick={() => openQuants(c.repo)}
                  class="shrink-0 rounded-md border border-border-subtle bg-surface-2 px-2 py-1 text-xs text-ink hover:bg-surface-3 disabled:opacity-50"
                >
                  Choose
                </button>
              </div>
            )}
          </For>
        </div>

        <div class="mt-3 flex gap-2">
          <input
            type="text"
            placeholder="Search the Hub, or paste a model URL…"
            class="min-w-0 flex-1 rounded border border-border-subtle bg-surface-1 px-2 py-1 text-sm"
            value={query()}
            onInput={(e) => setQuery(e.currentTarget.value)}
            onKeyDown={(e) => e.key === "Enter" && search()}
          />
          <button
            disabled={busy()}
            onClick={search}
            class="shrink-0 rounded-md border border-border-subtle bg-surface-2 px-3 py-1.5 text-sm text-ink hover:bg-surface-3 disabled:opacity-50"
          >
            Search
          </button>
        </div>

        <Show when={results().length > 0}>
          <div class="mt-2 space-y-1">
            <For each={results()}>
              {(r) => (
                <button
                  disabled={busy()}
                  onClick={() => openQuants(r.repo)}
                  class="flex w-full items-center justify-between gap-3 rounded-md border border-border-subtle bg-surface-1 p-2 text-left hover:bg-surface-2 disabled:opacity-50"
                >
                  <span class="truncate text-sm text-ink">{r.repo}</span>
                  <span class="shrink-0 text-[11px] text-ink-faint">
                    {r.gated ? "gated" : `${r.downloads.toLocaleString()} downloads`}
                  </span>
                </button>
              )}
            </For>
          </div>
        </Show>

        <Show when={openRepo()}>
          <div class="mt-3 rounded-md border border-border-subtle bg-surface-1 p-3">
            <div class="text-sm text-ink">{openRepo()!.repo}</div>
            <div class="text-[11px] text-ink-faint">
              <Show when={openRepo()!.contextLength}>{openRepo()!.contextLength} ctx · </Show>
              <Show when={openRepo()!.architecture}>{openRepo()!.architecture} · </Show>
              {openRepo()!.hasChatTemplate ? (
                "has a chat template"
              ) : (
                <span class="text-danger">no chat template — tool calls will not work</span>
              )}
            </div>

            <div class="mt-2 space-y-1">
              <For each={openRepo()!.quants}>
                {(q) => (
                  <div class="flex items-center justify-between gap-3 py-1">
                    <div class="min-w-0 text-sm text-ink">
                      {q.quant}
                      <Show when={openRepo()!.recommended === q.quant}>
                        <span class="ml-2 text-[11px] text-accent">recommended</span>
                      </Show>
                      <span class="ml-2 text-[11px] text-ink-faint">
                        {mb(q.totalBytes)}
                        <Show when={q.shards > 1}> · {q.shards} files</Show> · {FIT_LABEL[q.fit]}
                      </span>
                    </div>
                    <button
                      disabled={busy() || downloadingKey() !== null}
                      onClick={() => download(openRepo()!.repo, q.quant, q.fit)}
                      class="shrink-0 rounded-md border border-border-subtle bg-surface-2 px-2 py-1 text-xs text-ink hover:bg-surface-3 disabled:opacity-50"
                      classList={{ "text-danger": q.fit === "wontFit" }}
                    >
                      Download
                    </button>
                  </div>
                )}
              </For>
            </div>
          </div>
        </Show>

        <Show when={progress() && progress()!.phase !== "done"}>
          <div class="mt-3">
            <div class="h-1.5 overflow-hidden rounded-full bg-surface-3">
              <div class="h-full bg-accent transition-all" style={{ width: `${overallPct()}%` }} />
            </div>
            <div class="mt-1 flex items-center justify-between text-[11px] text-ink-faint">
              <span>
                Downloading {mb(progress()!.overallDone)} / {mb(progress()!.overallTotal)}
                <Show when={progress()!.fileCount > 1}>
                  {" "}
                  (file {progress()!.fileIndex} of {progress()!.fileCount})
                </Show>
              </span>
              <button
                onClick={() => localCancelInstall(progress()!.key)}
                class="rounded border border-border-subtle px-2 py-0.5 hover:bg-surface-2"
              >
                Cancel
              </button>
            </div>
            <div class="mt-1 text-[11px] text-ink-faint">
              Cancelling discards this file — finished files are kept.
            </div>
          </div>
        </Show>
      </div>

      <Show when={message()}>
        <div
          class="flex items-start gap-2 rounded-md border border-border-subtle p-2 text-xs"
          classList={{
            "text-danger": message()!.kind === "err",
            "text-ink-faint": message()!.kind === "ok",
          }}
        >
          <Icon
            name={message()!.kind === "err" ? "alert-triangle" : "check-circle"}
            class="mt-0.5 h-3 w-3 shrink-0"
          />
          <span class="break-all">{message()!.text}</span>
        </div>
      </Show>
    </div>
  );
};
