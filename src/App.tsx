import { createSignal, For, Match, Show, Switch, onMount } from "solid-js";
import "./App.css";
import { listen } from "@tauri-apps/api/event";
import { pickFolder, openWorkspace, setConfig, getConfig, type IndexProgress } from "./lib/ipc";
import "./lib/theme";
import "./lib/grill-me";
import { t, locale, setLocale, type LocaleId } from "./lib/grill-me";
import { FileTree } from "./components/FileTree";
import { ChatPanel } from "./components/ChatPanel";
import { EmptyState } from "./components/EmptyState";
import { Icon } from "./components/Icon";

const RECENT_KEY = "claudinio_recent_projects";

function loadRecent(): string[] {
  try {
    const raw = localStorage.getItem(RECENT_KEY);
    return raw ? JSON.parse(raw) : [];
  } catch {
    return [];
  }
}

function saveRecent(projects: string[]) {
  localStorage.setItem(RECENT_KEY, JSON.stringify(projects));
}

function addRecent(projects: () => string[], setter: (v: string[]) => void, path: string) {
  const updated = [path, ...projects().filter((p) => p !== path)].slice(0, 10);
  setter(updated);
  saveRecent(updated);
}

function App() {
  const [root, setRoot] = createSignal<string | null>(null);
  const [selectedFile, setSelectedFile] = createSignal<string | null>(null);
  const [indexStatus, setIndexStatus] = createSignal("");
  const [progress, setProgress] = createSignal<IndexProgress | null>(null);
  const [showConfig, setShowConfig] = createSignal(false);
  const [configApiKey, setConfigApiKey] = createSignal("");
  const [configBaseUrl, setConfigBaseUrl] = createSignal("https://api.claudin.io");
  const [configModel, setConfigModel] = createSignal("claudinio");
  const [configMaxRounds, setConfigMaxRounds] = createSignal<number | null>(null);
  const [configSubMaxRounds, setConfigSubMaxRounds] = createSignal<number | null>(null);
  const [configYoloMode, setConfigYoloMode] = createSignal(false);
  const [configYoloBlacklist, setConfigYoloBlacklist] = createSignal("");
  const [showTree, setShowTree] = createSignal(false);
  const [recentProjects, setRecentProjects] = createSignal<string[]>(loadRecent());

  // Listen for global index-progress events (model loading, embedding
  // generation, watcher re-indexing)
  onMount(() => {
    const unlisten = listen<IndexProgress>("index-progress", (event) => {
      setProgress(event.payload);
      const st = event.payload.status;
      if (st === "embeddings_done") {
        setIndexStatus((prev) =>
          prev
            ? `${prev} · ${event.payload.symbolsIndexed} embeddings`
            : `${event.payload.symbolsIndexed} embeddings`,
        );
        setTimeout(() => setProgress((p) => (p?.status === st ? null : p)), 1500);
      } else if (st === "embeddings_error" || st === "embedding_model_error") {
        setTimeout(() => setProgress((p) => (p?.status === st ? null : p)), 4000);
      }
    });
    return () => { unlisten.then((f) => f()); };
  });

  const openConfig = async () => {
    try {
      const cfg = await getConfig();
      if (cfg) {
        setConfigBaseUrl(cfg.baseUrl);
        setConfigModel(cfg.model);
        setConfigMaxRounds(cfg.maxRounds ?? null);
        setConfigSubMaxRounds(cfg.subMaxRounds ?? null);
        setConfigYoloMode(cfg.yoloMode ?? false);
        setConfigYoloBlacklist((cfg.yoloBlacklist ?? []).join(", "));
      }
    } catch {}
    setShowConfig(true);
  };

  const saveConfig = async () => {
    try {
      await setConfig({
        baseUrl: configBaseUrl() || undefined,
        apiKey: configApiKey() || undefined,
        model: configModel() || undefined,
        maxRounds: configMaxRounds(),
        subMaxRounds: configSubMaxRounds(),
        yoloMode: configYoloMode(),
        yoloBlacklist: configYoloBlacklist()
          .split(",")
          .map((s) => s.trim())
          .filter((s) => s.length > 0),
      });
      setShowConfig(false);
      setConfigApiKey("");
    } catch (e) {
      alert(t("app.config.saveError", String(e)));
    }
  };

  const indexProject = async (folder: string) => {
    setSelectedFile(null);
    setRoot(folder);
    setShowTree(false);
    setIndexStatus(t("app.index.indexingStatus"));
    setProgress(null);
    try {
      const s = await openWorkspace(folder, (p) => setProgress(p));
      setIndexStatus(t("app.index.filesCount", s.filesCount, s.symbolsCount));
      // Only clear scan progress — the embedding phase keeps reporting after
      // openWorkspace returns and clears itself on its terminal statuses.
      setTimeout(
        () => setProgress((p) => (p && p.status !== "done" && p.status !== "indexing" ? p : null)),
        800,
      );
    } catch (e) {
      setIndexStatus(`${t("chat.status.error")}: ${e}`);
      setProgress(null);
    }
  };

  const openFolder = async () => {
    const folder = await pickFolder();
    if (folder) {
      addRecent(recentProjects, setRecentProjects, folder);
      await indexProject(folder);
    }
  };

  const openRecent = async (folder: string) => {
    addRecent(recentProjects, setRecentProjects, folder);
    await indexProject(folder);
  };

  const isMac = () => document.documentElement.classList.contains("is-macos");

  return (
    <div class="flex h-full flex-col">
      <header
        class="flex h-11 shrink-0 items-center gap-2 border-b border-border-subtle bg-surface-1 px-3"
        classList={{ "pl-[78px]": isMac() }}
        data-tauri-drag-region
      >
        <span class="whitespace-nowrap text-[13px] font-semibold" data-tauri-drag-region>
          Claudinio <span class="text-accent">Code</span>
        </span>
        <span class="max-w-[280px] truncate font-mono text-[12px] text-ink-faint" data-tauri-drag-region>
          {root()}
        </span>
        <div class="ml-auto flex items-center gap-1.5">
          <button
            onClick={openConfig}
            class="flex h-7 w-7 items-center justify-center rounded-md text-ink-muted hover:bg-surface-2 hover:text-ink"
            title={t("app.config.title")}
          >
            <Icon name="settings" />
          </button>
        </div>
      </header>

      <Show when={showConfig()}>
        <div class="fixed inset-0 z-50 flex items-center justify-center bg-black/40 backdrop-blur-[2px]">
          <div class="w-[400px] rounded-lg bg-surface-1 p-5 shadow-modal">
            <h2 class="mb-4 text-sm font-semibold text-ink">{t("app.config.title")}</h2>

            {/* Lang selector */}
            <label class="mb-1 block text-xs text-ink-muted">Idioma / Language</label>
            <select
              value={locale()}
              onChange={(e) => setLocale(e.currentTarget.value as LocaleId)}
              class="mb-4 w-full rounded-md border border-border-subtle bg-surface-0 p-2 text-sm text-ink focus:border-accent focus:outline-none focus:ring-1 focus:ring-accent"
            >
              <option value="pt-BR">🇧🇷 Português</option>
              <option value="en-US">🇺🇸 English</option>
            </select>

            <label class="mb-1 block text-xs text-ink-muted">{t("app.config.apiKey")}</label>
            <input
              type="password"
              value={configApiKey()}
              onInput={(e) => setConfigApiKey(e.currentTarget.value)}
              placeholder="sk-..."
              class="mb-3 w-full rounded-md border border-border-subtle bg-surface-0 p-2 text-sm text-ink placeholder:text-ink-muted focus:border-accent focus:outline-none focus:ring-1 focus:ring-accent"
            />

            <label class="mb-1 block text-xs text-ink-muted">{t("app.config.baseUrl")}</label>
            <input
              value={configBaseUrl()}
              onInput={(e) => setConfigBaseUrl(e.currentTarget.value)}
              class="mb-3 w-full rounded-md border border-border-subtle bg-surface-0 p-2 text-sm text-ink focus:border-accent focus:outline-none focus:ring-1 focus:ring-accent"
            />

            <label class="mb-1 block text-xs text-ink-muted">{t("app.config.model")}</label>
            <input
              value={configModel()}
              onInput={(e) => setConfigModel(e.currentTarget.value)}
              placeholder="claudinio"
              class="mb-4 w-full rounded-md border border-border-subtle bg-surface-0 p-2 text-sm text-ink focus:border-accent focus:outline-none focus:ring-1 focus:ring-accent"
            />

            <hr class="mb-4 border-border-subtle" />

            <label class="mb-1 block text-xs text-ink-muted">{t("app.config.maxRounds")}</label>
            <input
              type="number"
              min="0"
              value={configMaxRounds() ?? ""}
              onInput={(e) => {
                const v = e.currentTarget.value;
                setConfigMaxRounds(v === "" ? null : Math.max(1, parseInt(v, 10) || 1));
              }}
              placeholder={t("app.config.unlimited")}
              class="mb-1 w-full rounded-md border border-border-subtle bg-surface-0 p-2 text-sm text-ink placeholder:text-ink-muted focus:border-accent focus:outline-none focus:ring-1 focus:ring-accent"
            />
            <p class="mb-3 text-[11px] text-ink-faint">{t("app.config.maxRoundsHint")}</p>

            <label class="mb-1 block text-xs text-ink-muted">{t("app.config.subMaxRounds")}</label>
            <input
              type="number"
              min="0"
              value={configSubMaxRounds() ?? ""}
              onInput={(e) => {
                const v = e.currentTarget.value;
                setConfigSubMaxRounds(v === "" ? null : Math.max(1, parseInt(v, 10) || 1));
              }}
              placeholder={t("app.config.unlimited")}
              class="mb-1 w-full rounded-md border border-border-subtle bg-surface-0 p-2 text-sm text-ink placeholder:text-ink-muted focus:border-accent focus:outline-none focus:ring-1 focus:ring-accent"
            />
            <p class="mb-4 text-[11px] text-ink-faint">{t("app.config.subMaxRoundsHint")}</p>

            <hr class="mb-4 border-border-subtle" />

            <label class="mb-2 flex cursor-pointer items-center gap-2">
              <input
                type="checkbox"
                checked={configYoloMode()}
                onChange={(e) => setConfigYoloMode(e.currentTarget.checked)}
                class="h-4 w-4 rounded border-border-subtle bg-surface-0 text-accent focus:ring-accent"
              />
              <span class="text-sm font-medium text-ink">{t("app.config.yoloMode")}</span>
              <span class="text-[11px] text-ink-faint">{t("app.config.yoloModeHint")}</span>
            </label>

            <Show when={configYoloMode()}>
              <label class="mb-1 block text-xs text-ink-muted">{t("app.config.yoloBlacklist")}</label>
              <textarea
                value={configYoloBlacklist()}
                onInput={(e) => setConfigYoloBlacklist(e.currentTarget.value)}
                placeholder="edit_file, bash"
                rows={2}
                class="mb-4 w-full rounded-md border border-border-subtle bg-surface-0 p-2 text-sm text-ink placeholder:text-ink-muted focus:border-accent focus:outline-none focus:ring-1 focus:ring-accent"
              />
              <p class="-mt-3 mb-4 text-[11px] text-ink-faint">{t("app.config.yoloBlacklistHint")}</p>
            </Show>

            <div class="flex justify-end gap-2">
              <button
                onClick={() => setShowConfig(false)}
                class="rounded-md border border-border-subtle bg-surface-2 px-3 py-1.5 text-sm text-ink hover:bg-surface-3"
              >
                {t("app.config.cancel")}
              </button>
              <button
                onClick={saveConfig}
                class="rounded-md bg-accent px-3 py-1.5 text-sm font-medium text-accent-ink hover:bg-accent-hover"
              >
                {t("app.config.save")}
              </button>
            </div>
          </div>
        </div>
      </Show>

      <div class="flex min-h-0 flex-1">
        <aside class="flex w-60 shrink-0 flex-col border-r border-border-subtle bg-surface-1">
          <Show
            when={showTree() && root()}
            fallback={
              <>
                <div class="flex items-center justify-between border-b border-border-subtle px-3 py-2">
                  <span class="text-[11px] font-semibold uppercase tracking-wider text-ink-faint">
                    {t("app.sidebar.projects")}
                  </span>
                </div>

                <div class="flex-1 overflow-y-auto">
                  <For each={recentProjects()}>
                    {(proj) => (
                      <button
                        class="flex w-full items-center gap-2 border-l-2 border-transparent px-3 py-2 text-left text-sm hover:bg-surface-2"
                        classList={{
                          "border-accent bg-surface-2": root() === proj,
                        }}
                        onClick={() => openRecent(proj)}
                      >
                        <Icon name="folder" class="shrink-0 text-ink-muted" />
                        <div class="min-w-0">
                          <div class="truncate text-[13px] text-ink">
                            {proj.split("/").pop()}
                          </div>
                          <div class="truncate text-[11px] text-ink-faint">
                            {proj}
                          </div>
                        </div>
                      </button>
                    )}
                  </For>

                  <Show when={recentProjects().length === 0}>
                    <div class="px-3 py-8 text-center text-xs text-ink-faint">
                      {t("app.sidebar.noRecent")}
                    </div>
                  </Show>
                </div>

                <div class="border-t border-border-subtle p-2">
                  <button
                    onClick={openFolder}
                    class="flex w-full items-center gap-2 rounded-md border border-dashed border-border-subtle px-3 py-2 text-xs text-ink-muted hover:border-accent hover:text-accent"
                  >
                    <Icon name="plus" class="h-3.5 w-3.5" />
                    {t("app.sidebar.openFolder")}
                  </button>
                  <Show when={root()}>
                    <button
                      onClick={() => setShowTree(true)}
                      class="mt-1 flex w-full items-center gap-2 rounded-md px-3 py-1.5 text-xs text-ink-muted hover:bg-surface-2 hover:text-ink"
                    >
                      <Icon name="chevron-right" class="h-3 w-3" />
                      {t("app.sidebar.browseFiles")}
                    </button>
                  </Show>
                </div>
              </>
            }
          >
            <div class="flex items-center gap-2 border-b border-border-subtle px-2 py-1.5">
              <button
                onClick={() => setShowTree(false)}
                class="flex items-center gap-1 rounded px-1.5 py-0.5 text-xs text-ink-muted hover:bg-surface-2 hover:text-ink"
              >
                <Icon name="arrow-left" class="h-3 w-3" />
                {t("app.sidebar.back")}
              </button>
              <span class="truncate text-xs font-semibold text-ink">
                {root()?.split("/").pop()}
              </span>
            </div>
            <div class="flex-1 overflow-hidden">
              <FileTree
                root={root()!}
                onOpenFile={setSelectedFile}
                selectedPath={selectedFile}
              />
            </div>
          </Show>

          <Show when={root() && !showTree() && (progress() || indexStatus())}>
            <div class="border-t border-border-subtle px-3 py-2">
              <Show when={progress() !== null}
                fallback={
                  <div class="font-mono text-[10px] text-ink-faint">
                    {indexStatus()}
                  </div>
                }
              >
                <Switch
                  fallback={
                    <div class="font-mono text-[10px] text-ink-faint">
                      {t("app.index.loadingModel")}
                    </div>
                  }
                >
                  <Match when={progress()!.status === "embeddings_done"}>
                    <div class="font-mono text-[10px] text-ink-faint">
                      {t("app.index.embeddingsReady")} · {progress()!.symbolsIndexed} {t("app.index.symbols")}
                    </div>
                  </Match>
                  <Match
                    when={
                      progress()!.status === "embeddings_error" ||
                      progress()!.status === "embedding_model_error"
                    }
                  >
                    <div class="font-mono text-[10px] text-red-400">
                      {t("app.index.embeddingFailed")}
                    </div>
                  </Match>
                  <Match when={progress()!.totalFiles > 0}>
                    <div class="flex flex-col gap-1">
                      <div class="flex items-center justify-between text-[10px]">
                        <span class="text-ink-muted">
                          {progress()!.status === "embedding"
                            ? t("app.index.embeddingStatus")
                            : t("app.index.indexing")}
                        </span>
                        <span class="font-mono text-ink-faint">
                          {progress()!.filesIndexed}/{progress()!.totalFiles}
                        </span>
                      </div>
                      <div class="h-1 w-full overflow-hidden rounded-full bg-surface-0">
                        <div
                          class="h-full rounded-full bg-accent transition-[width] duration-300 ease-out"
                          style={{
                            width:
                              progress()!.totalFiles > 0
                                ? `${(progress()!.filesIndexed / progress()!.totalFiles) * 100}%`
                                : "0%",
                          }}
                        />
                      </div>
                      <div class="font-mono text-[9px] text-ink-faint">
                        {progress()!.symbolsIndexed} {t("app.index.symbols")}
                      </div>
                    </div>
                  </Match>
                </Switch>
              </Show>
            </div>
          </Show>
        </aside>

        <main class="min-w-0 flex-1">
          <Show when={root()} fallback={
            <EmptyState
              recentProjects={recentProjects()}
              openRecent={openRecent}
              openFolder={openFolder}
            />
          }>
            <ChatPanel />
          </Show>
        </main>
      </div>
    </div>
  );
}

export default App;
