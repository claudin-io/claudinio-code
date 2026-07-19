import { createSignal, For, Match, Show, Switch, onMount, onCleanup, createEffect } from "solid-js";
import { fileIndexMap, loadFileIndex } from "./lib/fileIndex";
import "./App.css";
import { listen } from "@tauri-apps/api/event";
import { pickFolder, openWorkspace, closeWorkspace, setConfig, getConfig, setKeepAwake, listModels, openExternal, openExternalUrl, loginWithClaudinio, logoutClaudinio, validateApiKey, setWorkspaceConfig, listMcpServers, testMcpServer, detectIdes, openInIde, normalizeThinkingEffort, type IndexProgress, type IndexStatus, type McpServerMap, type McpServerStatus, type ThinkingEffort } from "./lib/ipc";
import { workspaceStatus } from "./lib/workspaceStatus";
import "./lib/grill-me";
import { t, locale, setLocale, type LocaleId } from "./lib/grill-me";
import { FileTree } from "./components/FileTree";
import { ChatPanel } from "./components/ChatPanel";
import { EmptyState } from "./components/EmptyState";
import { OnboardingWizard } from "./components/OnboardingWizard";
import { TasksPanel } from "./components/TasksPanel";
import { Icon } from "./components/Icon";
import { resolvedTheme } from "./lib/theme";
import FileEditorModal from "./components/FileEditorModal";
import { openPath } from "@tauri-apps/plugin-opener";
import { openInTerminal, copyPath, gitBranch, checkGitAvailable } from "./lib/ipc";
import { checkForUpdate, type UpdateInfo } from "./lib/ipc";
import { platform } from "./lib/platform";
import { ContextMenu } from "./components/ContextMenu";
import { createVisibilityAwareInterval } from "./lib/visibility";
import { startNetworkActivityListener } from "./lib/networkActivity";
import { startSystemStatsListener } from "./lib/systemStats";
import { AskpassModal } from "./components/AskpassModal";
import { type AskpassRequest } from "./lib/ipc";
import { SettingsPanel } from "./components/SettingsPanel";

const RECENT_KEY = "claudinio_recent_projects";
const OPEN_KEY = "claudinio_open_workspaces";

export function loadRecent(): string[] {
  try {
    const raw = localStorage.getItem(RECENT_KEY);
    return raw ? JSON.parse(raw) : [];
  } catch {
    return [];
  }
}

export function saveRecent(projects: string[]) {
  localStorage.setItem(RECENT_KEY, JSON.stringify(projects));
}

export function loadOpenWorkspaces(): string[] {
  try {
    const raw = localStorage.getItem(OPEN_KEY);
    return raw ? JSON.parse(raw) : [];
  } catch {
    return [];
  }
}

export function saveOpenWorkspaces(workspaces: string[]) {
  localStorage.setItem(OPEN_KEY, JSON.stringify(workspaces));
}

/// Default template inserted into the MCP JSON editor for a fresh server
/// entry — the user renames the key and fills in the real command/url.
function mcpServerTemplate(): Record<string, unknown> {
  return {
    "new-server": {
      type: "stdio",
      command: "npx",
      args: ["-y", "@modelcontextprotocol/server-name"],
      enabled: true,
    },
  };
}

/// Pretty-print an McpServerMap (or empty object) as the editor's initial text.
function mcpMapToJsonText(map: McpServerMap | undefined): string {
  return JSON.stringify(map ?? {}, null, 2);
}

/// Parse the editor's raw text into an McpServerMap. Returns an error message
/// instead of throwing so the UI can show it inline without losing the draft.
function parseMcpJson(text: string): { ok: true; value: McpServerMap } | { ok: false; error: string } {
  const trimmed = text.trim();
  if (trimmed.length === 0) return { ok: true, value: {} };
  try {
    const parsed = JSON.parse(trimmed);
    if (typeof parsed !== "object" || parsed === null || Array.isArray(parsed)) {
      return { ok: false, error: "Expected a JSON object mapping server name -> config" };
    }
    return { ok: true, value: parsed as McpServerMap };
  } catch (e) {
    return { ok: false, error: String(e) };
  }
}

export function addRecent(projects: () => string[], setter: (v: string[]) => void, path: string) {
  const updated = [path, ...projects().filter((p) => p !== path)].slice(0, 10);
  setter(updated);
  saveRecent(updated);
}

function App() {
  const [openWorkspaces, setOpenWorkspaces] = createSignal<string[]>([]);
  const [activeWorkspace, setActiveWorkspace] = createSignal<string | null>(null);
  const [selectedFile, setSelectedFile] = createSignal<string | null>(null);
  const [editorFilePath, setEditorFilePath] = createSignal<string | null>(null);
  const [indexStatusMap, setIndexStatusMap] = createSignal<Record<string, IndexStatus | string | null>>({});
  const [progressMap, setProgressMap] = createSignal<Record<string, IndexProgress | null>>({});
  const [showConfig, setShowConfig] = createSignal(false);
  const [configApiKey, setConfigApiKey] = createSignal("");
  const [configBrainModel, setConfigBrainModel] = createSignal("claudius");
  const [configBuilderModel, setConfigBuilderModel] = createSignal("claudinio");
  const [availableModels, setAvailableModels] = createSignal<string[]>(["claudinio", "claudius"]);
  const [configMaxRounds, setConfigMaxRounds] = createSignal<number | null>(null);
  const [configSubMaxRounds, setConfigSubMaxRounds] = createSignal<number | null>(null);
  const [configMaxGoldenCycles, setConfigMaxGoldenCycles] = createSignal<number | null>(null);
  const [configHandoffTokens, setConfigHandoffTokens] = createSignal<number>(120_000);
  const [configMaxGoldenStalls, setConfigMaxGoldenStalls] = createSignal<number | null>(null);
  const [configMaxParallelAgents, setConfigMaxParallelAgents] = createSignal<number>(4);
  const [configYoloMode, setConfigYoloMode] = createSignal(false);
  const [configYoloBlacklist, setConfigYoloBlacklist] = createSignal("");
  const [configKeepAwake, setConfigKeepAwake] = createSignal(true);
  const [configCodeIntelEnabled, setConfigCodeIntelEnabled] = createSignal(true);
  const [configAutoCommitPlan, setConfigAutoCommitPlan] = createSignal(true);
  const [configThinkingEffort, setConfigThinkingEffort] = createSignal<ThinkingEffort>("medium");
  const [configPreferredIde, setConfigPreferredIde] = createSignal("");
  const [availableIdes, setAvailableIdes] = createSignal<string[]>([]);
  const [configPlanSavePath, setConfigPlanSavePath] = createSignal("");
  const [workspaceConfigFields, setWorkspaceConfigFields] = createSignal<Set<string>>(new Set());
  const [accountLogin, setAccountLogin] = createSignal<string | null>(null);
  const [, setAccountTier] = createSignal<string | null>(null);
  const [loggingIn, setLoggingIn] = createSignal(false);
  const [hasApiKey, setHasApiKey] = createSignal(false);
  const [apiKeyValidating, setApiKeyValidating] = createSignal(false);
  const [onboardingApiKeyError, setOnboardingApiKeyError] = createSignal<string | null>(null);
  const [settingsApiKeyError, setSettingsApiKeyError] = createSignal<string | null>(null);
  const [showTree, setShowTree] = createSignal(false);
  const [contextPos, setContextPos] = createSignal<{ x: number; y: number; path: string } | null>(null);
  const [taskCounts, setTaskCounts] = createSignal<Record<string, number>>({});
  const [recentProjects, setRecentProjects] = createSignal<string[]>(loadRecent());
  const [onboardingSignInError, setOnboardingSignInError] = createSignal<string | null>(null);
  // Easter egg "iddqd" — override de URL e API Key para LLM
  const [easterEggActive, setEasterEggActive] = createSignal(false);
  const [keystrokeBuf, setKeystrokeBuf] = createSignal("");
  const [configOverrideBaseUrl, setConfigOverrideBaseUrl] = createSignal("");
  const [configOverrideApiKey, setConfigOverrideApiKey] = createSignal("");
  const [updateInfo, setUpdateInfo] = createSignal<UpdateInfo | null>(null);
  const [updateBannerDismissed, setUpdateBannerDismissed] = createSignal(false);
  const [_updateCheckState, setUpdateCheckState] = createSignal<"idle" | "checking" | "upToDate" | "error">("idle");
  const [_updateCheckError, setUpdateCheckError] = createSignal<string | null>(null);
  const [updateProgress, setUpdateProgress] = createSignal<number | null>(null);
  const [updateInstallError, setUpdateInstallError] = createSignal<string | null>(null);
  const [activeEditorCursor, setActiveEditorCursor] = createSignal<{ path: string; line: number } | null>(null);

  // Prevent the system from sleeping while any workspace session is actively
  // thinking. Waiting on user input/approval does not hold the wake lock.
  createEffect(() => {
    const anyThinking = Object.values(workspaceStatus).some((s) => s === "thinking");
    setKeepAwake(configKeepAwake() && anyThinking).catch(() => {});
  });

  // --- Git branch in header ---
  const [gitBranchName, setGitBranchName] = createSignal("");
  const [gitAvailable, setGitAvailable] = createSignal<boolean | null>(null);

  checkGitAvailable().then(setGitAvailable);

  createEffect(() => {
    if (gitAvailable() !== true) return;
    const ws = activeWorkspace();
    if (!ws) return;

    let inFlight = false;
    const refresh = async () => {
      if (inFlight) return;
      inFlight = true;
      try {
        const b = await gitBranch(ws);
        setGitBranchName(b);
      } catch {
        setGitBranchName("");
      } finally {
        inFlight = false;
      }
    };
    // Pauses while the window is hidden so no git processes spawn in background.
    createVisibilityAwareInterval(refresh, 30000);
  });

  // MCP server settings: edited as raw JSON text (`{ "name": { type, ... } }`),
  // parsed only on save/test. Statuses come from a live workspace connection
  // when one exists, otherwise reflect "configured, not connected".
  const [configMcpJson, setConfigMcpJson] = createSignal("{}");
  const [mcpJsonError, setMcpJsonError] = createSignal<string | null>(null);
  const [mcpStatuses, setMcpStatuses] = createSignal<Record<string, McpServerStatus>>({});
  const [mcpTesting, setMcpTesting] = createSignal(false);

  // Convenience views scoped to the currently visible workspace.
  const progress = () => {
    const ws = activeWorkspace();
    return ws ? progressMap()[ws] ?? null : null;
  };
  const indexStatus = (): IndexStatus | string | null => {
    const ws = activeWorkspace();
    if (!ws) return null;
    const v = indexStatusMap()[ws];
    return v ?? null;
  };
  const taskCount = () => {
    const ws = activeWorkspace();
    return ws ? taskCounts()[ws] ?? 0 : 0;
  };
  const setWsProgress = (ws: string, p: IndexProgress | null) =>
    setProgressMap((m) => ({ ...m, [ws]: p }));
  const setWsIndexStatus = (ws: string, s: IndexStatus | string | null) =>
    setIndexStatusMap((m) => ({ ...m, [ws]: s }));

  // Listen for global index-progress events (model loading, embedding
  // generation, watcher re-indexing). Events carry the workspace root so each
  // one lands on the right workspace's progress slot.
  onMount(() => {
    startNetworkActivityListener();
    startSystemStatsListener();
  });

  // git/ssh credential prompts from the backend askpass bridge.
  const [askpassRequest, setAskpassRequest] = createSignal<AskpassRequest | null>(null);
  onMount(() => {
    const unlisten = listen<AskpassRequest>("askpass-request", (event) => {
      setAskpassRequest(event.payload);
    });
    // onMount return values are ignored by SolidJS — cleanup must be registered
    onCleanup(() => { unlisten.then((f) => f()); });
  });

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
        setIndexStatusMap((m) => {
          const existing = m[ws];
          if (typeof existing === "object" && existing !== null && "embeddingsCount" in existing) {
            return {
              ...m,
              [ws]: { ...existing, embeddingsCount: event.payload.symbolsIndexed },
            };
          }
          return m;
        });
        clearIf(1500);
      } else if (st === "embeddings_error" || st === "embedding_model_error") {
        clearIf(4000);
      }
    });
    onCleanup(() => { unlisten.then((f) => f()); });
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

  // Load persisted auth state on startup so API-key-only users skip onboarding
  // on app restart.
  onMount(async () => {
    // Initialize theme state — reads localStorage and applies correct data-theme
    resolvedTheme();

    try {
      const cfg = await getConfig();
      setAccountLogin(cfg.accountLogin ?? null);
      setAccountTier(cfg.accountTier ?? null);
      setHasApiKey(cfg.hasApiKey ?? false);
      setConfigThinkingEffort(normalizeThinkingEffort(cfg.thinkingEffort));
    } catch {
      // Config file may not exist yet — silently assume no auth.
    }
  });

  const checkUpdates = async (manual: boolean) => {
    setUpdateCheckState("checking");
    setUpdateCheckError(null);
    try {
      const update = await checkForUpdate();
      if (update) {
        setUpdateInfo(update);
        setUpdateBannerDismissed(false);
        setUpdateCheckState("idle");
      } else {
        setUpdateCheckState(manual ? "upToDate" : "idle");
      }
    } catch (e) {
      // Silencioso na checagem automática (ex.: offline); só o clique manual
      // mostra o erro.
      if (manual) {
        setUpdateCheckError(String(e));
        setUpdateCheckState("error");
      } else {
        setUpdateCheckState("idle");
      }
    }
  };

  const installUpdate = async () => {
    const info = updateInfo();
    if (!info || updateProgress() !== null) return;
    setUpdateInstallError(null);
    setUpdateProgress(0);
    try {
      await info.install((fraction) => setUpdateProgress(fraction));
    } catch (e) {
      setUpdateProgress(null);
      setUpdateInstallError(String(e));
    }
  };

  onMount(() => {
    void checkUpdates(false);
  });

  // Discrete 5-step slider — no debounce needed; set_config only touches the
  // fields present, so the Settings save flow can't clobber this.
  const changeThinkingEffort = (v: ThinkingEffort) => {
    setConfigThinkingEffort(v);
    setConfig({ thinkingEffort: v }).catch(() => {});
  };

  const openConfig = async () => {
    try {
      const [cfg, models] = await Promise.all([getConfig(activeWorkspace() ?? undefined), listModels()]);
      if (cfg) {
        setConfigBrainModel(cfg.brainModel);
        setConfigBuilderModel(cfg.builderModel);
        setConfigMaxRounds(cfg.maxRounds ?? null);
        setConfigSubMaxRounds(cfg.subMaxRounds ?? null);
        setConfigMaxGoldenCycles(cfg.maxGoldenCycles ?? null);
        setConfigHandoffTokens(cfg.handoffContextTokens ?? 120_000);
        setConfigMaxGoldenStalls(cfg.maxGoldenStalls ?? null);
        setConfigMaxParallelAgents(cfg.maxParallelAgents ?? 4);
        setConfigYoloMode(cfg.yoloMode ?? false);
        setConfigYoloBlacklist((cfg.yoloBlacklist ?? []).join(", "));
        setConfigKeepAwake(cfg.keepAwake ?? true);
        setConfigCodeIntelEnabled(cfg.codeIntelEnabled ?? true);
        setConfigAutoCommitPlan(cfg.autoCommitPlan ?? true);
        setConfigThinkingEffort(normalizeThinkingEffort(cfg.thinkingEffort));
        setConfigPreferredIde(cfg.preferredIde ?? "");
        setConfigPlanSavePath(cfg.planSavePath ?? "");
        setConfigOverrideBaseUrl(cfg.overrideBaseUrl ?? "");
        setConfigOverrideApiKey(cfg.overrideApiKey ?? "");
        setConfigMcpJson(mcpMapToJsonText(cfg.mcp));
        setMcpJsonError(null);
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
    // Reset Easter egg — precisa digitar "iddqd" novamente a cada abertura
    setEasterEggActive(false);
    setKeystrokeBuf("");
    setShowConfig(true);
    try {
      const statuses = await listMcpServers(activeWorkspace() ?? undefined);
      setMcpStatuses(Object.fromEntries(statuses.map((s) => [s.name, s])));
    } catch {
      setMcpStatuses({});
    }
    // Detect available IDEs on this machine
    try {
      const ides = await detectIdes();
      setAvailableIdes(ides);
      // If no preferred IDE is set yet, auto-select the first available one
      if (!configPreferredIde() && ides.length > 0) {
        setConfigPreferredIde(ides[0]);
      }
    } catch {
      setAvailableIdes([]);
    }
  };

  const saveConfig = async () => {
    try {
      const changedApiKey = configApiKey();

      // If the user provided an API key through settings and is not already
      // authenticated via OAuth, validate the key before saving.
      if (changedApiKey && !accountLogin()) {
        setSettingsApiKeyError(null);
        try {
          await validateApiKey(changedApiKey);
        } catch (e) {
          setSettingsApiKeyError(String(e));
          return; // Keep the modal open
        }
      }

      const mcpParsed = parseMcpJson(configMcpJson());
      if (!mcpParsed.ok) {
        setMcpJsonError(mcpParsed.error);
        return; // Keep the modal open — don't lose the user's JSON draft
      }
      setMcpJsonError(null);

      await setConfig({
        apiKey: changedApiKey || undefined,
        brainModel: configBrainModel() || undefined,
        builderModel: configBuilderModel() || undefined,
        maxRounds: configMaxRounds(),
        subMaxRounds: configSubMaxRounds(),
        maxGoldenCycles: configMaxGoldenCycles(),
        handoffContextTokens: configHandoffTokens(),
        maxGoldenStalls: configMaxGoldenStalls(),
        maxParallelAgents: configMaxParallelAgents(),
        yoloMode: configYoloMode(),
        keepAwake: configKeepAwake(),
        codeIntelEnabled: configCodeIntelEnabled(),
        autoCommitPlan: configAutoCommitPlan(),
        preferredIde: configPreferredIde() || undefined,
        planSavePath: configPlanSavePath() || undefined,
        yoloBlacklist: configYoloBlacklist()
          .split(",")
          .map((s) => s.trim())
          .filter((s) => s.length > 0),
        overrideBaseUrl: configOverrideBaseUrl() || undefined,
        overrideApiKey: configOverrideApiKey() || undefined,
        mcp: mcpParsed.value,
      });
      // If the user just saved an API key and isn't authenticated via OAuth,
      // mark them as authenticated so onboarding stays hidden.
      if (changedApiKey && !accountLogin()) {
        setHasApiKey(true);
      }
      // Also persist plan_save_path to workspace config (.claudinio.json)
      const ws = activeWorkspace();
      if (ws) {
        const psp = configPlanSavePath();
        await setWorkspaceConfig(ws, psp || null);
      }
      setShowConfig(false);
      setConfigApiKey("");
      setSettingsApiKeyError(null);
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
    setHasApiKey(false);
  };

  const pickPlanPath = async () => {
    const folder = await pickFolder(activeWorkspace() ?? undefined);
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

  /// Merge a default server template into the JSON draft under a fresh key,
  /// so "add server" always leaves valid, ready-to-edit JSON behind — even if
  /// the current draft was invalid (in which case it's replaced outright).
  const addMcpServerTemplate = () => {
    const parsed = parseMcpJson(configMcpJson());
    const base = parsed.ok ? parsed.value : {};
    const template = mcpServerTemplate();
    let key = "new-server";
    let n = 1;
    while (key in base) {
      key = `new-server-${n}`;
      n++;
    }
    const merged = { ...base, [key]: template["new-server"] };
    setConfigMcpJson(JSON.stringify(merged, null, 2));
    setMcpJsonError(null);
  };

  /// Parse the current JSON draft and test-connect every server in it,
  /// populating the status list below the editor.
  const testAllMcpServers = async () => {
    const parsed = parseMcpJson(configMcpJson());
    if (!parsed.ok) {
      setMcpJsonError(parsed.error);
      return;
    }
    setMcpJsonError(null);
    setMcpTesting(true);
    try {
      const entries = Object.entries(parsed.value);
      const results: Record<string, McpServerStatus> = {};
      await Promise.all(
        entries.map(async ([name, entry]) => {
          try {
            results[name] = await testMcpServer(name, entry, activeWorkspace() ?? undefined);
          } catch (e) {
            results[name] = { name, connected: false, toolCount: 0, toolNames: [], error: String(e) };
          }
        }),
      );
      setMcpStatuses(results);
    } finally {
      setMcpTesting(false);
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
      setWsIndexStatus(folder, s);
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

  const onboardingSignIn = async () => {
    setOnboardingSignInError(null);
    try {
      const result = await loginWithClaudinio();
      setAccountLogin(result.login);
      setAccountTier(result.tier ?? null);
    } catch (e) {
      setOnboardingSignInError(t("onboarding.signIn.error") + ": " + String(e));
    }
  };

  const onboardApiKeySubmit = async (key: string) => {
    setOnboardingApiKeyError(null);
    setApiKeyValidating(true);
    try {
      // Save the key first so the backend uses it for validation
      await setConfig({ apiKey: key });
      // Validate by fetching available models
      await validateApiKey(key);
      setHasApiKey(true);
    } catch (e) {
      setOnboardingApiKeyError(String(e));
      setApiKeyValidating(false);
    }
  };

  // Global listener for Easter egg "iddqd" — captura teclas mesmo sem foco
  createEffect(() => {
    if (!showConfig()) {
      setEasterEggActive(false);
      setKeystrokeBuf("");
      return;
    }
    const handler = (e: KeyboardEvent) => {
      if (e.target instanceof HTMLInputElement || e.target instanceof HTMLSelectElement || e.target instanceof HTMLTextAreaElement) return;
      if (easterEggActive()) return;
      const next = keystrokeBuf() + e.key.toLowerCase();
      if ("iddqd".startsWith(next)) {
        setKeystrokeBuf(next);
        if (next === "iddqd") setEasterEggActive(true);
      } else if ("iddqd".startsWith(e.key.toLowerCase())) {
        setKeystrokeBuf(e.key.toLowerCase());
      } else {
        setKeystrokeBuf("");
      }
    };
    document.addEventListener("keydown", handler);
    onCleanup(() => document.removeEventListener("keydown", handler));
  });

  return (
    <div class="flex h-full flex-col">
      <header
        class="flex h-16 shrink-0 items-center border-b border-border-subtle bg-surface-1 pl-2 pr-3"
        data-tauri-drag-region
      >
        <span class="flex items-center gap-2 rounded-full border border-accent/30 px-3 py-1 text-[13px] font-semibold" data-tauri-drag-region>
          <img src="/reddit_icon_256.png" alt="Claudinio" class="h-8 w-8" />
          Claudinio <span class="text-accent">Code</span>
          <span class="ml-0.5 text-[9px] font-extralight text-ink-faint">· v{APP_VERSION}</span>
        </span>
        <Show when={updateInfo()}>
          <button
            onClick={() => void installUpdate()}
            disabled={updateProgress() !== null}
            class="rounded-md bg-warning px-2.5 py-1 text-xs font-medium text-black transition-opacity hover:opacity-90 disabled:cursor-default disabled:opacity-80"
          >
            <Show
              when={updateProgress() !== null}
              fallback={<>{t("update.updateTo", updateInfo()!.version)}</>}
            >
              <span class="flex items-center gap-1.5">
                <Icon name="loader" class="h-3 w-3 animate-spin" />
                {t("update.installing")}
              </span>
            </Show>
          </button>
        </Show>
        <div class="ml-auto flex items-center gap-3">
          <div class="flex flex-col items-end">
            <span class="max-w-[280px] truncate font-mono text-[12px] text-ink-faint" data-tauri-drag-region>
              {activeWorkspace()}
            </span>
            <Show when={gitBranchName()}>
              <span class="flex items-center gap-1 text-[10px] leading-none text-ink-faint">
                <Icon name="git-branch" class="h-3 w-3" />
                {gitBranchName()}
              </span>
            </Show>
          </div>
          <Show when={availableIdes().length > 0}>
            <button
              onClick={() => {
                const ws = activeWorkspace();
                const ide = configPreferredIde() || availableIdes()[0];
                if (ws && ide) openInIde(ws, ide).catch(console.error);
              }}
              class="flex h-7 w-7 items-center justify-center rounded-md text-ink-muted hover:bg-surface-2 hover:text-ink"
              title={t("app.config.openInIde")}
            >
              <Icon name="external-link" />
            </button>
          </Show>
          <button
            onClick={openConfig}
            class="flex h-7 w-7 items-center justify-center rounded-md text-ink-muted hover:bg-surface-2 hover:text-ink"
            title={t("app.config.title")}
          >
            <Icon name="settings" />
          </button>
        </div>
      </header>

      <SettingsPanel
        showConfig={showConfig}
        setShowConfig={setShowConfig}
        language={locale}
        setLanguage={(v: LocaleId | ((prev: LocaleId) => LocaleId)) => {
          const id = typeof v === "function" ? v(locale()) : v;
          setLocale(id);
          return id;
        }}
        configBrainModel={configBrainModel}
        setConfigBrainModel={setConfigBrainModel}
        configBuilderModel={configBuilderModel}
        setConfigBuilderModel={setConfigBuilderModel}
        availableModels={availableModels}
        configMaxParallelAgents={configMaxParallelAgents}
        setConfigMaxParallelAgents={setConfigMaxParallelAgents}
        configMaxRounds={configMaxRounds}
        setConfigMaxRounds={setConfigMaxRounds}
        configSubMaxRounds={configSubMaxRounds}
        setConfigSubMaxRounds={setConfigSubMaxRounds}
        configMaxGoldenCycles={configMaxGoldenCycles}
        setConfigMaxGoldenCycles={setConfigMaxGoldenCycles}
        configMaxGoldenStalls={configMaxGoldenStalls}
        setConfigMaxGoldenStalls={setConfigMaxGoldenStalls}
        configHandoffTokens={configHandoffTokens}
        setConfigHandoffTokens={setConfigHandoffTokens}
        configYoloMode={configYoloMode}
        setConfigYoloMode={setConfigYoloMode}
        configYoloBlacklist={configYoloBlacklist}
        setConfigYoloBlacklist={setConfigYoloBlacklist}
        configKeepAwake={configKeepAwake}
        setConfigKeepAwake={setConfigKeepAwake}
        configCodeIntelEnabled={configCodeIntelEnabled}
        setConfigCodeIntelEnabled={setConfigCodeIntelEnabled}
        configAutoCommitPlan={configAutoCommitPlan}
        setConfigAutoCommitPlan={setConfigAutoCommitPlan}
        configPreferredIde={configPreferredIde}
        setConfigPreferredIde={setConfigPreferredIde}
        availableIdes={availableIdes}
        configPlanSavePath={configPlanSavePath}
        setConfigPlanSavePath={setConfigPlanSavePath}
        workspaceConfigFields={workspaceConfigFields}
        accountLogin={accountLogin}
        hasApiKey={hasApiKey}
        loggingIn={loggingIn}
        configApiKey={configApiKey}
        setConfigApiKey={setConfigApiKey}
        settingsApiKeyError={settingsApiKeyError}
        configMcpJson={configMcpJson}
        setConfigMcpJson={setConfigMcpJson}
        mcpJsonError={mcpJsonError}
        setMcpJsonError={setMcpJsonError}
        mcpStatuses={mcpStatuses}
        mcpTesting={mcpTesting}
        setMcpTesting={setMcpTesting}
        easterEggActive={easterEggActive}
        configOverrideBaseUrl={configOverrideBaseUrl}
        setConfigOverrideBaseUrl={setConfigOverrideBaseUrl}
        configOverrideApiKey={configOverrideApiKey}
        setConfigOverrideApiKey={setConfigOverrideApiKey}
        saveConfig={saveConfig}
        doLogin={doLogin}
        doLogout={doLogout}
        pickPlanPath={pickPlanPath}
        addMcpServerTemplate={addMcpServerTemplate}
        testAllMcpServers={testAllMcpServers}
        openSupportUrl={() => openExternalUrl("https://claudin.io/dashboard#account")}
      />

      {/* Update-available prompt (auto-check on startup) */}
      <Show when={updateInfo() && !updateBannerDismissed()}>
        <div class="fixed bottom-4 right-4 z-50 w-80 rounded-lg border border-border-subtle bg-surface-1 p-4 shadow-modal">
          <div class="mb-2 text-sm font-semibold text-ink">{t("update.available", updateInfo()!.version)}</div>
          <Show when={updateInstallError()}>
            <div class="mb-2 text-xs text-red-400">{t("update.error", updateInstallError() ?? "")}</div>
          </Show>
          <Show
            when={updateProgress() !== null}
            fallback={
              <div class="flex justify-end gap-2">
                <button
                  onClick={() => setUpdateBannerDismissed(true)}
                  class="rounded-md border border-border-subtle bg-surface-2 px-3 py-1.5 text-xs text-ink hover:bg-surface-3"
                >
                  {t("update.later")}
                </button>
                <button
                  onClick={() => void installUpdate()}
                  class="rounded-md bg-accent px-3 py-1.5 text-xs font-medium text-accent-ink hover:bg-accent-hover"
                >
                  {t("update.installNow")}
                </button>
              </div>

            }
          >
            <div class="h-1.5 w-full overflow-hidden rounded-full bg-surface-2">
              <div
                class="h-full rounded-full bg-accent transition-[width]"
                classList={{ "animate-pulse w-full": updateProgress()! < 0 }}
                style={updateProgress()! >= 0 ? { width: `${Math.round(updateProgress()! * 100)}%` } : undefined}
              />
            </div>

            <div class="mt-1.5 text-xs text-ink-muted">{t("update.downloading")}</div>
          </Show>
        </div>
      </Show>

      <Show when={accountLogin() || hasApiKey()} fallback={
        <OnboardingWizard
          onSignIn={onboardingSignIn}
          signingIn={loggingIn()}
          signInError={onboardingSignInError()}
          onApiKeySubmit={onboardApiKeySubmit}
          apiKeyValidating={apiKeyValidating()}
          apiKeyError={onboardingApiKeyError()}
        />
      }>
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
                        onContextMenu={(e) => {
                          e.preventDefault();
                          setContextPos({ x: e.clientX, y: e.clientY, path: proj });
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
                      <div class="group flex w-full items-center gap-2 border-l-2 border-transparent px-3 py-2 text-left text-sm opacity-70 hover:bg-surface-2 hover:opacity-100"
                        onContextMenu={(e) => {
                          e.preventDefault();
                          setContextPos({ x: e.clientX, y: e.clientY, path: proj });
                        }}
                      >
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
                onDblClickFile={setEditorFilePath}
                onOpenExternal={openExternal}
                selectedPath={selectedFile}
                availableIdes={availableIdes()}
                activeEditorCursor={activeEditorCursor}
              />
            </div>

          </Show>

          <Show when={activeWorkspace() && !showTree() && (progress() || indexStatus())}>
            <div class="border-t border-border-subtle px-3 py-2">
              <Show when={progress() !== null}
                fallback={
                  <Show when={typeof indexStatus() === "string"}
                    fallback={
                      <div class="flex flex-col gap-0.5">
                        <div class="flex items-center gap-1 font-mono text-[10px] text-ink-faint">
                          <Icon name="file" class="w-3 h-3" />
                          <span>{t("app.index.filesLabel", (indexStatus() as IndexStatus).filesCount)}</span>
                        </div>

                        <div class="flex items-center gap-1 font-mono text-[10px] text-ink-faint">
                          <Icon name="layers" class="w-3 h-3" />
                          <span>{t("app.index.symbolsLabel", (indexStatus() as IndexStatus).symbolsCount)}</span>
                        </div>

                        <div class="flex items-center gap-1 font-mono text-[10px] text-ink-faint">
                          <Icon name="brain" class="w-3 h-3" />
                          <span>{t("app.index.embeddingsLabel", (indexStatus() as IndexStatus).embeddingsCount)}</span>
                        </div>

                        <Show when={(indexStatus() as IndexStatus).watcherWarning}>
                          <div class="flex items-center gap-1 font-mono text-[10px] text-yellow-400">
                            <Icon name="alert-triangle" class="w-3 h-3" />
                            <span>{(indexStatus() as IndexStatus).watcherWarning}</span>
                          </div>

                        </Show>
                      </div>

                    }
                  >
                    <div class="font-mono text-[10px] text-ink-faint">
                      {indexStatus() as string}
                    </div>

                  </Show>
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
            {/* All panels kept mounted: hidden panels keep receiving their run's
                Channel events so streaming stays live when switching workspaces. */}
            <For each={openWorkspaces()}>
              {(ws) => (
                <div class="h-full" style={{ display: activeWorkspace() === ws ? "block" : "none" }}>
                  <ChatPanel workspace={ws} isActive={() => activeWorkspace() === ws} fileList={fileIndexMap[ws] ?? []} thinkingEffort={configThinkingEffort} onThinkingEffortChange={changeThinkingEffort} />
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
                <Show when={activeWorkspace() === ws}>
                  <div class="h-full">
                    <TasksPanel
                      workspace={ws}
                      onTasksChange={(count) => setTaskCounts((m) => ({ ...m, [ws]: count }))}
                    />
                  </div>
                </Show>
              )}
            </For>
          </aside>
        </Show>
      </div>
      <Show when={editorFilePath() && activeWorkspace()}>
        <FileEditorModal
          filePath={editorFilePath()!}
          rootPath={activeWorkspace()!}
          onClose={() => setEditorFilePath(null)}
          onCursorLineChange={(line) => setActiveEditorCursor({ path: editorFilePath()!, line })}
        />
      </Show>
      <Show when={contextPos()}>
        {(pos) => (
          <ContextMenu
            x={pos().x}
            y={pos().y}
            items={[
              {
                label: platform() === 'mac' ? 'Reveal in Finder' : platform() === 'win' ? 'Show in Explorer' : 'Open in File Manager',
                icon: 'external-link',
                action: () => openPath(pos().path).catch(console.error),
              },
              {
                label: 'Open in Terminal',
                icon: 'terminal',
                action: () => openInTerminal(pos().path).catch(console.error),
              },
              {
                label: 'Copy Path',
                icon: 'file-text',
                separatorAfter: true,
                action: () => copyPath(pos().path),
              },
              ...(availableIdes().length > 0
                ? availableIdes().map((ide) => {
                    const cursor = activeEditorCursor();
                    const gotoLine =
                      cursor && cursor.path === pos().path
                        ? cursor.line
                        : undefined;
                    return {
                      label:
                        ide === "vscode"
                          ? (gotoLine ? `Open in VS Code (line ${gotoLine})` : `Open in VS Code`)
                          : (gotoLine ? `Open in Cursor (line ${gotoLine})` : `Open in Cursor`),
                      icon: 'external-link' as const,
                      action: () => openInIde(pos().path, ide, gotoLine).catch(console.error),
                    };
                  })
                : []),
            ]}
            onClose={() => setContextPos(null)}
          />
        )}
      </Show>
      </Show>
      <AskpassModal request={askpassRequest()} onDone={() => setAskpassRequest(null)} />
    </div>
  );
}

export default App;
