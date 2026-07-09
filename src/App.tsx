import { createSignal, For, Match, Show, Switch, onMount } from "solid-js";
import { fileIndexMap, loadFileIndex } from "./lib/fileIndex";
import "./App.css";
import { listen } from "@tauri-apps/api/event";
import { pickFolder, openWorkspace, closeWorkspace, setConfig, getConfig, listModels, openExternal, loginWithClaudinio, logoutClaudinio, setWorkspaceConfig, type IndexProgress } from "./lib/ipc";
import { workspaceStatus } from "./lib/workspaceStatus";
import "./lib/theme";
import "./lib/grill-me";
import { t, locale, setLocale, type LocaleId } from "./lib/grill-me";
import { FileTree } from "./components/FileTree";
import { ChatPanel } from "./components/ChatPanel";
import { EmptyState } from "./components/EmptyState";
import { TasksPanel } from "./components/TasksPanel";
import { Icon } from "./components/Icon";

const RECENT_KEY = "claudinio_recent_projects";
const OPEN_KEY = "claudinio_open_workspaces";

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

function loadOpenWorkspaces(): string[] {
  try {
    const raw = localStorage.getItem(OPEN_KEY);
    return raw ? JSON.parse(raw) : [];
  } catch {
    return [];
  }
}

function saveOpenWorkspaces(workspaces: string[]) {
  localStorage.setItem(OPEN_KEY, JSON.stringify(workspaces));
}

function addRecent(projects: () => string[], setter: (v: string[]) => void, path: string) {
  const updated = [path, ...projects().filter((p) => p !== path)].slice(0, 10);
  setter(updated);
  saveRecent(updated);
}

function App() {
  const [openWorkspaces, setOpenWorkspaces] = createSignal<string[]>([]);
  const [activeWorkspace, setActiveWorkspace] = createSignal<string | null>(null);
  const [selectedFile, setSelectedFile] = createSignal<string | null>(null);
  const [indexStatusMap, setIndexStatusMap] = createSignal<Record<string, string>>({});
  const [progressMap, setProgressMap] = createSignal<Record<string, IndexProgress | null>>({});
  const [showConfig, setShowConfig] = createSignal(false);
  const [configApiKey, setConfigApiKey] = createSignal("");
  const [configBrainModel, setConfigBrainModel] = createSignal("claudinio");
  const [configBuilderModel, setConfigBuilderModel] = createSignal("claudinio");
  const [availableModels, setAvailableModels] = createSignal<string[]>(["claudinio", "claudius"]);
  const [configMaxRounds, setConfigMaxRounds] = createSignal<number | null>(null);
  const [configSubMaxRounds, setConfigSubMaxRounds] = createSignal<number | null>(null);
  const [configMaxGoldenCycles, setConfigMaxGoldenCycles] = createSignal<number | null>(null);
  const [configMaxGoldenStalls, setConfigMaxGoldenStalls] = createSignal<number | null>(null);
  const [configYoloMode, setConfigYoloMode] = createSignal(false);
  const [configYoloBlacklist, setConfigYoloBlacklist] = createSignal("");
  const [configPlanSavePath, setConfigPlanSavePath] = createSignal("");
  const [workspaceConfigFields, setWorkspaceConfigFields] = createSignal<Set<string>>(new Set());
  const [accountLogin, setAccountLogin] = createSignal<string | null>(null);
  const [accountTier, setAccountTier] = createSignal<string | null>(null);
  const [loggingIn, setLoggingIn] = createSignal(false);
  const [showAdvancedAuth, setShowAdvancedAuth] = createSignal(false);
  const [showTree, setShowTree] = createSignal(false);
  const [taskCounts, setTaskCounts] = createSignal<Record<string, number>>({});
  const [recentProjects, setRecentProjects] = createSignal<string[]>(loadRecent());

  // Convenience views scoped to the currently visible workspace.
  const progress = () => {
    const ws = activeWorkspace();
    return ws ? progressMap()[ws] ?? null : null;
  };
  const indexStatus = () => {
    const ws = activeWorkspace();
    return ws ? indexStatusMap()[ws] ?? "" : "";
  };
  const taskCount = () => {
    const ws = activeWorkspace();
    return ws ? taskCounts()[ws] ?? 0 : 0;
  };
  const setWsProgress = (ws: string, p: IndexProgress | null) =>
    setProgressMap((m) => ({ ...m, [ws]: p }));
  const setWsIndexStatus = (ws: string, s: string) =>
    setIndexStatusMap((m) => ({ ...m, [ws]: s }));

  // Listen for global index-progress events (model loading, embedding
  // generation, watcher re-indexing). Events carry the workspace root so each
  // one lands on the right workspace's progress slot.
  onMount(() => {
    const unlisten = listen<IndexProgress>("index-progress", (event) => {
      const ws = event.payload.workspace;
      if (!ws) return;
      const st = event.payload.status;
      // Watcher re-index events carry no file totals and have no terminal
      // event to clear them — keep them out of the progress panel entirely.
      if (st === "reindexing" || st === "reindexed" || st === "reindex_error") return;
      setWsProgress(ws, event.payload);
      const clearIf = (delay: number) =>
        setTimeout(
          () => setProgressMap((m) => (m[ws]?.status === st ? { ...m, [ws]: null } : m)),
          delay,
        );
      if (st === "embeddings_done") {
        setIndexStatusMap((m) => ({
          ...m,
          [ws]: m[ws]
            ? `${m[ws]} · ${event.payload.symbolsIndexed} embeddings`
            : `${event.payload.symbolsIndexed} embeddings`,
        }));
        clearIf(1500);
      } else if (st === "embeddings_error" || st === "embedding_model_error") {
        clearIf(4000);
      }
    });
    return () => { unlisten.then((f) => f()); };
  });

  // Restore workspaces that were open in the previous run.
  onMount(() => {
    const stored = loadOpenWorkspaces();
    if (stored.length === 0) return;
    setOpenWorkspaces(stored);
    setActiveWorkspace(stored[0]);
    for (const ws of stored) {
      void indexProject(ws, { activate: false });
    }
  });

  const openConfig = async () => {
    try {
      const [cfg, models] = await Promise.all([getConfig(activeWorkspace() ?? undefined), listModels()]);
      if (cfg) {
        setConfigBrainModel(cfg.brainModel);
        setConfigBuilderModel(cfg.builderModel);
        setConfigMaxRounds(cfg.maxRounds ?? null);
        setConfigSubMaxRounds(cfg.subMaxRounds ?? null);
        setConfigMaxGoldenCycles(cfg.maxGoldenCycles ?? null);
        setConfigMaxGoldenStalls(cfg.maxGoldenStalls ?? null);
        setConfigYoloMode(cfg.yoloMode ?? false);
        setConfigYoloBlacklist((cfg.yoloBlacklist ?? []).join(", "));
        setConfigPlanSavePath(cfg.planSavePath ?? "");
        setAccountLogin(cfg.accountLogin ?? null);
        setAccountTier(cfg.accountTier ?? null);
        // Build set of field names that come from workspace config
        const wsKeys = new Set<string>();
        if (cfg.workspaceConfig && typeof cfg.workspaceConfig === "object") {
          for (const key of Object.keys(cfg.workspaceConfig)) {
            wsKeys.add(key);
          }
        }
        setWorkspaceConfigFields(wsKeys);
      }
      if (models && models.length > 0) {
        setAvailableModels(models);
      }
    } catch {}
    setShowConfig(true);
  };

  const saveConfig = async () => {
    try {
      await setConfig({
        apiKey: configApiKey() || undefined,
        brainModel: configBrainModel() || undefined,
        builderModel: configBuilderModel() || undefined,
        maxRounds: configMaxRounds(),
        subMaxRounds: configSubMaxRounds(),
        maxGoldenCycles: configMaxGoldenCycles(),
        maxGoldenStalls: configMaxGoldenStalls(),
        yoloMode: configYoloMode(),
        planSavePath: configPlanSavePath() || undefined,
        yoloBlacklist: configYoloBlacklist()
          .split(",")
          .map((s) => s.trim())
          .filter((s) => s.length > 0),
      });
      // Also persist plan_save_path to workspace config (.claudinio.json)
      const ws = activeWorkspace();
      if (ws) {
        const psp = configPlanSavePath();
        await setWorkspaceConfig(ws, psp || null);
      }
      setShowConfig(false);
      setConfigApiKey("");
    } catch (e) {
      alert(t("app.config.saveError", String(e)));
    }
  };

  const doLogin = async () => {
    setLoggingIn(true);
    try {
      const result = await loginWithClaudinio();
      setAccountLogin(result.login);
      setAccountTier(result.tier ?? null);
    } catch (e) {
      alert(t("app.config.loginError", String(e)));
    } finally {
      setLoggingIn(false);
    }
  };

  const doLogout = async () => {
    try {
      await logoutClaudinio();
    } catch {}
    setAccountLogin(null);
    setAccountTier(null);
  };

  const pickPlanPath = async () => {
    const folder = await pickFolder();
    if (!folder) return;
    const ws = activeWorkspace();
    if (!ws) {
      setConfigPlanSavePath(folder);
      return;
    }
    // Convert absolute path to relative (relative to workspace root)
    if (folder.startsWith(ws)) {
      let rel = folder.slice(ws.length);
      if (rel.startsWith("/") || rel.startsWith("\\")) rel = rel.slice(1);
      setConfigPlanSavePath(rel || ".");
    } else {
      setConfigPlanSavePath(folder);
    }
  };

  const addOpenWorkspace = (folder: string) => {
    setOpenWorkspaces((prev) => {
      if (prev.includes(folder)) return prev;
      const updated = [...prev, folder];
      saveOpenWorkspaces(updated);
      return updated;
    });
  };

  const indexProject = async (folder: string, opts?: { activate?: boolean }) => {
    const activate = opts?.activate ?? true;
    addOpenWorkspace(folder);
    if (activate) {
      setSelectedFile(null);
      setActiveWorkspace(folder);
      setShowTree(false);
    }
    setWsIndexStatus(folder, t("app.index.indexingStatus"));
    setWsProgress(folder, null);
    try {
      const s = await openWorkspace(folder, (p) => setWsProgress(folder, p));
      setWsIndexStatus(folder, t("app.index.filesCount", s.filesCount, s.symbolsCount));
      // Load the flat file list for @-mention autocomplete
      await loadFileIndex(folder);
      // Only clear scan progress — the embedding phase keeps reporting after
      // openWorkspace returns and clears itself on its terminal statuses.
      setTimeout(
        () =>
          setProgressMap((m) => {
            const p = m[folder];
            return p && p.status !== "done" && p.status !== "indexing" ? m : { ...m, [folder]: null };
          }),
        800,
      );
    } catch (e) {
      setWsIndexStatus(folder, `${t("chat.status.error")}: ${e}`);
      setWsProgress(folder, null);
    }
  };

  /// Bring an already-open workspace to the front (no re-index — the backend
  /// short-circuits anyway, but we avoid even the IPC round-trip).
  const activateWorkspace = (folder: string) => {
    setSelectedFile(null);
    setShowTree(false);
    setActiveWorkspace(folder);
  };

  const closeOpenWorkspace = async (folder: string) => {
    const updated = openWorkspaces().filter((w) => w !== folder);
    setOpenWorkspaces(updated);
    saveOpenWorkspaces(updated);
    if (activeWorkspace() === folder) {
      setActiveWorkspace(updated[0] ?? null);
    }
    try {
      await closeWorkspace(folder);
    } catch {}
  };

  /// Remove um projeto da lista de recentes.
  const removeFromRecent = (folder: string) => {
    const updated = recentProjects().filter((p) => p !== folder);
    setRecentProjects(updated);
    saveRecent(updated);
  };

  const openFolder = async () => {
    const folder = await pickFolder();
    if (folder) {
      addRecent(recentProjects, setRecentProjects, folder);
      if (openWorkspaces().includes(folder)) {
        activateWorkspace(folder);
      } else {
        await indexProject(folder);
      }
    }
  };

  const openRecent = async (folder: string) => {
    addRecent(recentProjects, setRecentProjects, folder);
    if (openWorkspaces().includes(folder)) {
      activateWorkspace(folder);
    } else {
      await indexProject(folder);
    }
  };

  return (
    <div class="flex h-full flex-col">
      <header
        class="flex h-16 shrink-0 items-center border-b border-border-subtle bg-surface-1 pl-2 pr-3"
        data-tauri-drag-region
      >
        <span class="flex items-center gap-2 rounded-full border border-accent/30 px-3 py-1 text-[13px] font-semibold" data-tauri-drag-region>
          <img src="/reddit_icon_256.png" alt="Claudinio" class="h-8 w-8" />
          Claudinio <span class="text-accent">Code</span>
        </span>
        <div class="ml-auto flex items-center gap-3">
          <span class="max-w-[280px] truncate font-mono text-[12px] text-ink-faint" data-tauri-drag-region>
            {activeWorkspace()}
          </span>
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
          <div class="w-[400px] max-h-[90vh] overflow-y-auto rounded-lg bg-surface-1 p-5 shadow-modal">
            <h2 class="mb-4 text-sm font-semibold text-ink">{t("app.config.title")}</h2>

            {/* Lang selector */}
            <label class="mb-1 block text-xs text-ink-muted">Idioma / Language</label>
            <select
              value={locale()}
              onChange={(e) => setLocale(e.currentTarget.value as LocaleId)}
              class="mb-4 w-full appearance-none rounded-md border border-border-subtle bg-surface-0 p-2 text-sm text-ink focus:border-accent focus:outline-none focus:ring-1 focus:ring-accent"
            >
              <option value="pt-BR">🇧🇷 Português</option>
              <option value="en-US">🇺🇸 English</option>
            </select>

            <label class="mb-1 block text-xs text-ink-muted">{t("app.config.account")}</label>
            <Show
              when={accountLogin()}
              fallback={
                <button
                  onClick={doLogin}
                  disabled={loggingIn()}
                  class="mb-2 w-full rounded-md bg-accent p-2 text-sm font-semibold text-white hover:opacity-90 disabled:opacity-50"
                >
                  {loggingIn() ? t("app.config.signingIn") : t("app.config.signIn")}
                </button>
              }
            >
              <div class="mb-2 flex items-center justify-between rounded-md border border-border-subtle bg-surface-0 p-2 text-sm">
                <span class="truncate text-ink">
                  {t("app.config.signedInAs", accountLogin() ?? "")}
                  <Show when={accountTier()}> — {accountTier()}</Show>
                </span>
                <button onClick={doLogout} class="ml-2 shrink-0 text-xs text-ink-muted hover:text-ink hover:underline">
                  {t("app.config.signOut")}
                </button>
              </div>
            </Show>

            <button
              onClick={() => setShowAdvancedAuth((v) => !v)}
              class="mb-2 text-xs text-ink-muted hover:text-ink hover:underline"
            >
              {showAdvancedAuth() ? t("app.config.hideAdvanced") : t("app.config.showAdvanced")}
            </button>
            <Show when={showAdvancedAuth()}>
              <label class="mb-1 block text-xs text-ink-muted">{t("app.config.apiKey")}</label>
              <input
                type="password"
                value={configApiKey()}
                onInput={(e) => setConfigApiKey(e.currentTarget.value)}
                placeholder="sk-..."
                class="mb-4 w-full rounded-md border border-border-subtle bg-surface-0 p-2 text-sm text-ink placeholder:text-ink-muted focus:border-accent focus:outline-none focus:ring-1 focus:ring-accent"
              />
            </Show>

            <hr class="mb-4 border-border-subtle" />

            {/* Plan save path — always editable */}
            <label class="mb-1 block text-xs text-ink-muted">{t("app.config.planSavePath")}</label>
            <div class="mb-1 flex gap-1">
              <div class="relative flex-1">
                <input
                  type="text"
                  value={configPlanSavePath()}
                  onInput={(e) => setConfigPlanSavePath(e.currentTarget.value)}
                  placeholder=".claudinio/plans"
                  class="w-full rounded-md border border-border-subtle bg-surface-0 p-2 pr-8 text-sm text-ink placeholder:text-ink-muted focus:border-accent focus:outline-none focus:ring-1 focus:ring-accent"
                />
                <Show when={configPlanSavePath()}>
                  <button
                    onClick={() => setConfigPlanSavePath("")}
                    class="absolute right-2 top-1/2 -translate-y-1/2 text-ink-faint hover:text-ink"
                    title={t("app.config.resetToDefault")}
                  >
                    <Icon name="x" class="h-3.5 w-3.5" />
                  </button>
                </Show>
              </div>
              <button
                onClick={pickPlanPath}
                class="flex h-9 w-9 shrink-0 items-center justify-center rounded-md border border-border-subtle text-ink-muted hover:bg-surface-2 hover:text-ink"
                title={t("app.config.browseFolder")}
              >
                <Icon name="folder-open" class="h-4 w-4" />
              </button>
            </div>
            <div class="mb-4 flex items-center gap-2">
              <Show when={!configPlanSavePath()}>
                <span class="rounded border border-border-subtle bg-surface-2 px-1.5 py-px text-[10px] text-ink-faint">{t("app.config.default")}</span>
              </Show>
              <p class="text-[11px] text-ink-faint">{t("app.config.planSavePathHint")}</p>
            </div>

            {/* Brain model selector */}
            <div class="flex items-center gap-2 mb-1">
              <label class="block text-xs text-ink-muted">{t("app.config.brainModel")}</label>
              <Show when={workspaceConfigFields().has("brain_model")}>
                <span class="rounded border border-accent/40 bg-accent/10 px-1.5 py-px text-[10px] font-medium text-accent">{t("app.config.sourceWorkspace")}</span>
              </Show>
              <Show when={!workspaceConfigFields().has("brain_model")}>
                <span class="rounded border border-border-subtle bg-surface-2 px-1.5 py-px text-[10px] text-ink-faint">{t("app.config.sourceLocal")}</span>
              </Show>
            </div>
            <select
              value={configBrainModel()}
              onChange={(e) => setConfigBrainModel(e.currentTarget.value)}
              disabled={workspaceConfigFields().has("brain_model")}
              class="mb-4 w-full appearance-none rounded-md border border-border-subtle p-2 text-sm text-ink focus:border-accent focus:outline-none focus:ring-1 focus:ring-accent"
              classList={{
                "bg-surface-2 text-ink-muted pointer-events-none": workspaceConfigFields().has("brain_model"),
                "bg-surface-0": !workspaceConfigFields().has("brain_model"),
              }}
            >
              <For each={availableModels()}>
                {(m) => <option value={m} selected={configBrainModel() === m}>{m}</option>}
              </For>
            </select>

            {/* Builder model selector */}
            <div class="flex items-center gap-2 mb-1">
              <label class="block text-xs text-ink-muted">{t("app.config.builderModel")}</label>
              <Show when={workspaceConfigFields().has("builder_model")}>
                <span class="rounded border border-accent/40 bg-accent/10 px-1.5 py-px text-[10px] font-medium text-accent">{t("app.config.sourceWorkspace")}</span>
              </Show>
              <Show when={!workspaceConfigFields().has("builder_model")}>
                <span class="rounded border border-border-subtle bg-surface-2 px-1.5 py-px text-[10px] text-ink-faint">{t("app.config.sourceLocal")}</span>
              </Show>
            </div>
            <select
              value={configBuilderModel()}
              onChange={(e) => setConfigBuilderModel(e.currentTarget.value)}
              disabled={workspaceConfigFields().has("builder_model")}
              class="mb-4 w-full appearance-none rounded-md border border-border-subtle p-2 text-sm text-ink focus:border-accent focus:outline-none focus:ring-1 focus:ring-accent"
              classList={{
                "bg-surface-2 text-ink-muted pointer-events-none": workspaceConfigFields().has("builder_model"),
                "bg-surface-0": !workspaceConfigFields().has("builder_model"),
              }}
            >
              <For each={availableModels()}>
                {(m) => <option value={m} selected={configBuilderModel() === m}>{m}</option>}
              </For>
            </select>

            <hr class="mb-4 border-border-subtle" />

            <div class="flex items-center gap-2 mb-1">
              <label class="block text-xs text-ink-muted">{t("app.config.maxRounds")}</label>
              <Show when={workspaceConfigFields().has("max_rounds")}>
                <span class="rounded border border-accent/40 bg-accent/10 px-1.5 py-px text-[10px] font-medium text-accent">{t("app.config.sourceWorkspace")}</span>
              </Show>
              <Show when={!workspaceConfigFields().has("max_rounds")}>
                <span class="rounded border border-border-subtle bg-surface-2 px-1.5 py-px text-[10px] text-ink-faint">{t("app.config.sourceLocal")}</span>
              </Show>
            </div>
            <input
              type="number"
              min="0"
              value={configMaxRounds() ?? ""}
              onInput={(e) => {
                const v = e.currentTarget.value;
                setConfigMaxRounds(v === "" ? null : Math.max(1, parseInt(v, 10) || 1));
              }}
              placeholder={t("app.config.unlimited")}
              class="mb-1 w-full rounded-md border border-border-subtle p-2 text-sm text-ink placeholder:text-ink-muted focus:border-accent focus:outline-none focus:ring-1 focus:ring-accent"
              classList={{
                "bg-surface-2 text-ink-muted pointer-events-none": workspaceConfigFields().has("max_rounds"),
                "bg-surface-0": !workspaceConfigFields().has("max_rounds"),
              }}
              disabled={workspaceConfigFields().has("max_rounds")}
            />
            <p class="mb-3 text-[11px] text-ink-faint">{t("app.config.maxRoundsHint")}</p>

            <div class="flex items-center gap-2 mb-1">
              <label class="block text-xs text-ink-muted">{t("app.config.subMaxRounds")}</label>
              <Show when={workspaceConfigFields().has("sub_max_rounds")}>
                <span class="rounded border border-accent/40 bg-accent/10 px-1.5 py-px text-[10px] font-medium text-accent">{t("app.config.sourceWorkspace")}</span>
              </Show>
              <Show when={!workspaceConfigFields().has("sub_max_rounds")}>
                <span class="rounded border border-border-subtle bg-surface-2 px-1.5 py-px text-[10px] text-ink-faint">{t("app.config.sourceLocal")}</span>
              </Show>
            </div>
            <input
              type="number"
              min="0"
              value={configSubMaxRounds() ?? ""}
              onInput={(e) => {
                const v = e.currentTarget.value;
                setConfigSubMaxRounds(v === "" ? null : Math.max(1, parseInt(v, 10) || 1));
              }}
              placeholder={t("app.config.unlimited")}
              class="mb-1 w-full rounded-md border border-border-subtle p-2 text-sm text-ink placeholder:text-ink-muted focus:border-accent focus:outline-none focus:ring-1 focus:ring-accent"
              classList={{
                "bg-surface-2 text-ink-muted pointer-events-none": workspaceConfigFields().has("sub_max_rounds"),
                "bg-surface-0": !workspaceConfigFields().has("sub_max_rounds"),
              }}
              disabled={workspaceConfigFields().has("sub_max_rounds")}
            />
            <p class="mb-4 text-[11px] text-ink-faint">{t("app.config.subMaxRoundsHint")}</p>

            <label class="mb-1 block text-xs text-ink-muted">{t("settings.maxGoldenCycles")}</label>
            <input
              type="number"
              min="0"
              value={configMaxGoldenCycles() ?? ""}
              onInput={(e) => {
                const v = e.currentTarget.value;
                setConfigMaxGoldenCycles(v === "" ? null : Math.max(0, parseInt(v, 10) || 0));
              }}
              placeholder="5"
              class="mb-1 w-full rounded-md border border-border-subtle bg-surface-0 p-2 text-sm text-ink placeholder:text-ink-muted focus:border-accent focus:outline-none focus:ring-1 focus:ring-accent"
            />
            <p class="mb-3 text-[11px] text-ink-faint">{t("settings.maxGoldenCyclesHint")}</p>

            <label class="mb-1 block text-xs text-ink-muted">{t("settings.maxGoldenStalls")}</label>
            <input
              type="number"
              min="0"
              value={configMaxGoldenStalls() ?? ""}
              onInput={(e) => {
                const v = e.currentTarget.value;
                setConfigMaxGoldenStalls(v === "" ? null : Math.max(0, parseInt(v, 10) || 0));
              }}
              placeholder="2"
              class="mb-1 w-full rounded-md border border-border-subtle bg-surface-0 p-2 text-sm text-ink placeholder:text-ink-muted focus:border-accent focus:outline-none focus:ring-1 focus:ring-accent"
            />
            <p class="mb-4 text-[11px] text-ink-faint">{t("settings.maxGoldenStallsHint")}</p>

            <hr class="mb-4 border-border-subtle" />

            <label class="mb-2 flex cursor-pointer items-center gap-2">
              <input
                type="checkbox"
                checked={configYoloMode()}
                onChange={(e) => setConfigYoloMode(e.currentTarget.checked)}
                class="h-4 w-4 rounded border-border-subtle bg-surface-0 text-accent focus:ring-accent"
                disabled={workspaceConfigFields().has("yolo_mode")}
              />
              <span class="text-sm font-medium text-ink">{t("app.config.yoloMode")}</span>
              <span class="text-[11px] text-ink-faint">{t("app.config.yoloModeHint")}</span>
              <Show when={workspaceConfigFields().has("yolo_mode")}>
                <span class="rounded border border-accent/40 bg-accent/10 px-1.5 py-px text-[10px] font-medium text-accent">{t("app.config.sourceWorkspace")}</span>
              </Show>
              <Show when={!workspaceConfigFields().has("yolo_mode")}>
                <span class="rounded border border-border-subtle bg-surface-2 px-1.5 py-px text-[10px] text-ink-faint">{t("app.config.sourceLocal")}</span>
              </Show>
            </label>

            <Show when={configYoloMode()}>
              <label class="mb-1 block text-xs text-ink-muted">{t("app.config.yoloBlacklist")}</label>
              <textarea
                value={configYoloBlacklist()}
                onInput={(e) => setConfigYoloBlacklist(e.currentTarget.value)}
                placeholder="edit_file, bash"
                rows={2}
                class="mb-4 w-full rounded-md border border-border-subtle bg-surface-0 p-2 text-sm text-ink placeholder:text-ink-muted focus:border-accent focus:outline-none focus:ring-1 focus:ring-accent"
                classList={{
                  "bg-surface-2 text-ink-muted pointer-events-none": workspaceConfigFields().has("yolo_blacklist"),
                }}
                disabled={workspaceConfigFields().has("yolo_blacklist")}
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
            when={showTree() && activeWorkspace()}
            fallback={
              <>
                <div class="flex items-center justify-between border-b border-border-subtle px-3 py-2">
                  <span class="text-[11px] font-semibold uppercase tracking-wider text-ink-faint">
                    {t("app.sidebar.projects")}
                  </span>
                </div>

                <div class="flex-1 overflow-y-auto">
                  {/* Open workspaces — clicking switches the visible chat; each
                      entry shows its agent status so a background workspace's
                      run stays visible. */}
                  <For each={openWorkspaces()}>
                    {(proj) => (
                      <div
                        class="group flex w-full items-center gap-2 border-l-2 border-transparent px-3 py-2 text-left text-sm hover:bg-surface-2"
                        classList={{
                          "border-accent bg-surface-2": activeWorkspace() === proj,
                        }}
                      >
                        <button
                          class="flex min-w-0 flex-1 items-center gap-2 text-left"
                          onClick={() => activateWorkspace(proj)}
                        >
                          <span class="relative shrink-0">
                            <Icon name="folder" class="text-ink-muted" />
                            <Show when={workspaceStatus[proj]}>
                              <span
                                class="absolute -right-1 -top-1 block h-2 w-2 rounded-full"
                                classList={{
                                  "bg-accent animate-pulse": workspaceStatus[proj] === "thinking",
                                  "bg-amber-400 animate-pulse":
                                    workspaceStatus[proj] === "awaiting_approval" ||
                                    workspaceStatus[proj] === "awaiting_input",
                                  "bg-success": workspaceStatus[proj] === "done",
                                  "bg-red-400": workspaceStatus[proj] === "error",
                                  hidden: workspaceStatus[proj] === "idle",
                                }}
                                title={workspaceStatus[proj]}
                              />
                            </Show>
                          </span>
                          <div class="min-w-0">
                            <div class="truncate text-[13px] text-ink">
                              {proj.split("/").pop()}
                            </div>
                            <div class="truncate text-[11px] text-ink-faint">
                              {workspaceStatus[proj] === "thinking"
                                ? t("chat.status.thinking")
                                : workspaceStatus[proj] === "awaiting_approval"
                                  ? t("chat.status.awaitingApproval")
                                  : workspaceStatus[proj] === "awaiting_input"
                                    ? t("chat.status.awaitingInput")
                                    : proj}
                            </div>
                          </div>
                        </button>
                        <button
                          class="hidden shrink-0 rounded p-0.5 text-ink-faint hover:bg-surface-3 hover:text-ink group-hover:block"
                          title={t("app.sidebar.closeWorkspace")}
                          onClick={(e) => {
                            e.stopPropagation();
                            closeOpenWorkspace(proj);
                          }}
                        >
                          <Icon name="x" class="h-3 w-3" />
                        </button>
                      </div>
                    )}
                  </For>

                  {/* Recent projects not currently open */}
                  <For each={recentProjects().filter((p) => !openWorkspaces().includes(p))}>
                    {(proj) => (
                      <div class="group flex w-full items-center gap-2 border-l-2 border-transparent px-3 py-2 text-left text-sm opacity-70 hover:bg-surface-2 hover:opacity-100">
                        <button
                          class="flex min-w-0 flex-1 items-center gap-2 text-left"
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
                        <button
                          class="hidden shrink-0 rounded p-0.5 text-ink-faint hover:bg-surface-3 hover:text-ink group-hover:block"
                          title={t("app.sidebar.closeWorkspace")}
                          onClick={(e) => {
                            e.stopPropagation();
                            removeFromRecent(proj);
                          }}
                        >
                          <Icon name="x" class="h-3 w-3" />
                        </button>
                      </div>
                    )}
                  </For>

                  <Show when={recentProjects().length === 0 && openWorkspaces().length === 0}>
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
                  <Show when={activeWorkspace()}>
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
                {activeWorkspace()?.split("/").pop()}
              </span>
            </div>
            <div class="flex-1 overflow-hidden">
              <FileTree
                root={activeWorkspace()!}
                onOpenFile={setSelectedFile}
                onOpenExternal={openExternal}
                selectedPath={selectedFile}
              />
            </div>
          </Show>

          <Show when={activeWorkspace() && !showTree() && (progress() || indexStatus())}>
            <div class="border-t border-border-subtle px-3 py-2">
              <Show when={progress() !== null}
                fallback={
                  <div class="font-mono text-[10px] text-ink-faint">
                    {indexStatus()}
                  </div>
                }
              >
                <Switch>
                  <Match when={progress()!.status === "loading_model"}>
                    <div class="font-mono text-[10px] text-ink-faint">
                      {t("app.index.loadingModel")}
                    </div>
                  </Match>
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
          <Show when={activeWorkspace()} fallback={
            <EmptyState
              recentProjects={recentProjects()}
              openRecent={openRecent}
              openFolder={openFolder}
            />
          }>
            {/* One ChatPanel per open workspace, all kept mounted: hidden
                panels keep receiving their run's Channel events, so agents in
                background workspaces stream in parallel without losing state. */}
            <For each={openWorkspaces()}>
              {(ws) => (
                <div
                  class="h-full"
                  style={{ display: activeWorkspace() === ws ? "block" : "none" }}
                >
                  <ChatPanel workspace={ws} isActive={() => activeWorkspace() === ws} fileList={fileIndexMap[ws] ?? []} />
                </div>
              )}
            </For>
          </Show>
        </main>

        <Show when={activeWorkspace()}>
          <aside
            class="shrink-0 border-l border-border-subtle bg-surface-1 overflow-hidden transition-[width] duration-100"
            classList={{ "w-10": taskCount() > 0, "w-0": taskCount() === 0 }}
          >
            <For each={openWorkspaces()}>
              {(ws) => (
                <div
                  class="h-full"
                  style={{ display: activeWorkspace() === ws ? "block" : "none" }}
                >
                  <TasksPanel
                    workspace={ws}
                    onTasksChange={(count) => setTaskCounts((m) => ({ ...m, [ws]: count }))}
                  />
                </div>
              )}
            </For>
          </aside>
        </Show>
      </div>
    </div>
  );
}

export default App;
