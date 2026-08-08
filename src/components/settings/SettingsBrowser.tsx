import { createSignal, createResource, onCleanup, onMount, Show, type Component } from "solid-js";
import { listen } from "@tauri-apps/api/event";
import {
  browserStatus,
  browserInstall,
  browserUninstall,
  browserTest,
  getConfig,
  setConfig,
  type BrowserPrefs,
  type BrowserStatus,
  type BrowserInstallProgress,
} from "../../lib/ipc";
import { Icon } from "../Icon";

const DEFAULT_PREFS: BrowserPrefs = {
  enabled: true,
  headless: false,
  chromePath: null,
  viewportWidth: 1280,
  viewportHeight: 800,
};

function mb(bytes: number): string {
  return `${Math.round(bytes / 1_000_000)} MB`;
}

export const SettingsBrowser: Component = () => {
  const [prefs, setPrefs] = createSignal<BrowserPrefs>(DEFAULT_PREFS);
  const [status, { refetch }] = createResource<BrowserStatus>(browserStatus);
  const [progress, setProgress] = createSignal<BrowserInstallProgress | null>(null);
  const [busy, setBusy] = createSignal(false);
  const [message, setMessage] = createSignal<{ kind: "ok" | "err"; text: string } | null>(null);

  onMount(async () => {
    const cfg = await getConfig();
    if (cfg.browser) setPrefs({ ...DEFAULT_PREFS, ...cfg.browser });

    const unlisten = await listen<BrowserInstallProgress>(
      "browser-install-progress",
      (e) => setProgress(e.payload),
    );
    onCleanup(unlisten);
  });

  const save = async (patch: Partial<BrowserPrefs>) => {
    const next = { ...prefs(), ...patch };
    setPrefs(next);
    await setConfig({ browser: next });
  };

  const install = async () => {
    setBusy(true);
    setMessage(null);
    try {
      await browserInstall();
      await refetch();
      setMessage({ kind: "ok", text: "Chromium installed." });
    } catch (e) {
      setMessage({ kind: "err", text: String(e) });
    } finally {
      setBusy(false);
      setProgress(null);
    }
  };

  const uninstall = async () => {
    setBusy(true);
    setMessage(null);
    try {
      await browserUninstall();
      await refetch();
    } catch (e) {
      setMessage({ kind: "err", text: String(e) });
    } finally {
      setBusy(false);
    }
  };

  const test = async () => {
    setBusy(true);
    setMessage(null);
    try {
      setMessage({ kind: "ok", text: await browserTest() });
    } catch (e) {
      setMessage({ kind: "err", text: String(e) });
    } finally {
      setBusy(false);
    }
  };

  const pct = () => {
    const p = progress();
    if (!p || p.totalBytes === 0) return 0;
    return Math.min(100, Math.round((p.downloadedBytes / p.totalBytes) * 100));
  };

  const ready = () => Boolean(status()?.installed) || Boolean(prefs().chromePath?.trim());

  return (
    <div class="space-y-5">
      <div>
        <h3 class="text-sm font-medium text-ink">Browser</h3>
        <p class="mt-1 text-xs text-ink-faint">
          Lets the agent open pages, read the console and network, and take screenshots. It
          runs a dedicated browser with its own profile — never your Chrome profile, so your
          logged-in sessions are never exposed to a page the agent visits.
        </p>
      </div>

      <label class="flex items-center gap-2 text-sm text-ink">
        <input
          type="checkbox"
          checked={prefs().enabled}
          onChange={(e) => save({ enabled: e.currentTarget.checked })}
        />
        Enable browser tools
        <span class="text-xs text-ink-faint">(when off, they cost no prompt tokens)</span>
      </label>

      <label class="flex items-center gap-2 text-sm text-ink">
        <input
          type="checkbox"
          checked={prefs().headless}
          onChange={(e) => save({ headless: e.currentTarget.checked })}
        />
        Run headless (no visible window)
      </label>

      <div class="flex items-center gap-2 text-sm text-ink">
        <span>Viewport</span>
        <input
          type="number"
          class="w-20 rounded border border-border-subtle bg-surface-1 px-2 py-1"
          value={prefs().viewportWidth}
          onChange={(e) => save({ viewportWidth: Number(e.currentTarget.value) })}
        />
        <span class="text-ink-faint">×</span>
        <input
          type="number"
          class="w-20 rounded border border-border-subtle bg-surface-1 px-2 py-1"
          value={prefs().viewportHeight}
          onChange={(e) => save({ viewportHeight: Number(e.currentTarget.value) })}
        />
      </div>

      <div class="rounded-md border border-border-subtle bg-surface-1 p-3">
        <Show
          when={status()?.supported}
          fallback={
            <p class="text-xs text-ink-faint">
              No Chromium build is published for this platform.
            </p>
          }
        >
          <div class="flex items-center justify-between gap-3">
            <div class="min-w-0">
              <div class="text-sm text-ink">
                Chromium {status()?.version}{" "}
                <Show when={status()?.installed} fallback={<span class="text-ink-faint">— not installed</span>}>
                  <span class="text-ink-faint">— installed</span>
                </Show>
              </div>
              <Show when={status()?.installed && status()?.exePath}>
                <div class="truncate text-[11px] text-ink-faint">{status()!.exePath}</div>
              </Show>
              <Show when={!status()?.installed}>
                <div class="text-[11px] text-ink-faint">
                  One-time download of {mb(status()?.downloadSize ?? 0)}.
                </div>
              </Show>
            </div>
            <div class="flex shrink-0 gap-2">
              <Show
                when={status()?.installed}
                fallback={
                  <button
                    disabled={busy()}
                    onClick={install}
                    class="rounded-md border border-border-subtle bg-surface-2 px-3 py-1.5 text-sm text-ink hover:bg-surface-3 disabled:opacity-50"
                  >
                    Download
                  </button>
                }
              >
                <button
                  disabled={busy()}
                  onClick={uninstall}
                  class="rounded-md border border-border-subtle bg-surface-2 px-3 py-1.5 text-sm text-ink hover:bg-surface-3 disabled:opacity-50"
                >
                  Remove
                </button>
              </Show>
              <button
                disabled={busy() || !ready()}
                onClick={test}
                class="rounded-md border border-border-subtle bg-surface-2 px-3 py-1.5 text-sm text-ink hover:bg-surface-3 disabled:opacity-50"
              >
                Test
              </button>
            </div>
          </div>

          <Show when={progress()?.phase === "download"}>
            <div class="mt-3">
              <div class="h-1.5 overflow-hidden rounded-full bg-surface-3">
                <div class="h-full bg-accent transition-all" style={{ width: `${pct()}%` }} />
              </div>
              <div class="mt-1 text-[11px] text-ink-faint">
                Downloading… {mb(progress()!.downloadedBytes)} / {mb(progress()!.totalBytes)}
              </div>
            </div>
          </Show>
          <Show when={progress()?.phase === "extract"}>
            <div class="mt-3 text-[11px] text-ink-faint">Extracting…</div>
          </Show>
        </Show>
      </div>

      <div>
        <label class="text-sm text-ink">Use an installed browser instead</label>
        <p class="mt-1 text-xs text-ink-faint">
          Skips the download. It is still launched as a separate process against our own
          profile directory.
        </p>
        <div class="mt-2 flex gap-2">
          <input
            type="text"
            placeholder={status()?.systemChrome ?? "Path to a Chrome, Chromium or Edge binary"}
            class="min-w-0 flex-1 rounded border border-border-subtle bg-surface-1 px-2 py-1 text-sm"
            value={prefs().chromePath ?? ""}
            onChange={(e) => save({ chromePath: e.currentTarget.value || null })}
          />
          <Show when={status()?.systemChrome}>
            <button
              onClick={() => save({ chromePath: status()!.systemChrome! })}
              class="shrink-0 rounded-md border border-border-subtle bg-surface-2 px-3 py-1.5 text-sm text-ink hover:bg-surface-3"
            >
              Detect
            </button>
          </Show>
        </div>
      </div>

      <Show when={message()}>
        <div
          class="flex items-start gap-2 rounded-md border border-border-subtle p-2 text-xs"
          classList={{
            "text-danger": message()!.kind === "err",
            "text-ink-faint": message()!.kind === "ok",
          }}
        >
          <Icon name={message()!.kind === "err" ? "alert-triangle" : "check-circle"} class="mt-0.5 h-3 w-3 shrink-0" />
          <span class="break-all">{message()!.text}</span>
        </div>
      </Show>
    </div>
  );
};
