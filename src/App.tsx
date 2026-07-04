import { createSignal, For, Show } from "solid-js";
import "./App.css";
import { pickFolder, openWorkspace, setConfig, getConfig } from "./lib/ipc";
import { FileTree } from "./components/FileTree";
import { ChatPanel } from "./components/ChatPanel";

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
  const [showConfig, setShowConfig] = createSignal(false);
  const [configApiKey, setConfigApiKey] = createSignal("");
  const [configBaseUrl, setConfigBaseUrl] = createSignal("https://api.claudin.io");
  const [configModel, setConfigModel] = createSignal("claudinio");
  const [showTree, setShowTree] = createSignal(false);
  const [recentProjects, setRecentProjects] = createSignal<string[]>(loadRecent());

  const openConfig = async () => {
    try {
      const cfg = await getConfig();
      if (cfg) {
        setConfigBaseUrl(cfg.baseUrl);
        setConfigModel(cfg.model);
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
      });
      setShowConfig(false);
      setConfigApiKey("");
    } catch (e) {
      alert(`Erro ao salvar config: ${e}`);
    }
  };

  const indexProject = async (folder: string) => {
    setSelectedFile(null);
    setRoot(folder);
    setShowTree(false);
    setIndexStatus("indexando…");
    try {
      const s = await openWorkspace(folder);
      setIndexStatus(`${s.filesCount} arquivos, ${s.symbolsCount} símbolos`);
    } catch (e) {
      setIndexStatus(`erro: ${e}`);
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

  return (
    <div class="flex h-full flex-col">
      <header class="flex shrink-0 items-center gap-2 border-b border-border-subtle bg-surface-1 px-3 py-1.5">
        <span class="text-sm font-semibold whitespace-nowrap">
          Claudinio <span class="text-accent">Code</span>
        </span>
        <button
          class="rounded-md border border-border-subtle bg-surface-2 px-2.5 py-1 text-xs hover:border-accent whitespace-nowrap"
          onClick={openFolder}
        >
          Abrir pasta…
        </button>
        <Show when={root()}>
          <span class="truncate text-xs text-ink-muted">{root()}</span>
        </Show>
        <div class="ml-auto flex items-center gap-2">
          <button
            onClick={openConfig}
            class="rounded-md border border-border-subtle bg-surface-2 px-2 py-1 text-xs hover:border-accent"
            title="Configurar API"
          >
            Config
          </button>
        </div>
      </header>

      <Show when={showConfig()}>
        <div class="fixed inset-0 z-50 flex items-center justify-center bg-black/60">
          <div class="w-96 rounded-lg border border-border-subtle bg-surface-1 p-5 shadow-xl">
            <h2 class="mb-4 text-sm font-semibold text-ink">Configuração da API</h2>

            <label class="mb-1 block text-xs text-ink-muted">API Key</label>
            <input
              type="password"
              value={configApiKey()}
              onInput={(e) => setConfigApiKey(e.currentTarget.value)}
              placeholder="sk-..."
              class="mb-3 w-full rounded border border-border-subtle bg-surface-2 p-2 text-sm text-ink placeholder:text-ink-muted focus:outline-none focus:border-accent"
            />

            <label class="mb-1 block text-xs text-ink-muted">Base URL</label>
            <input
              value={configBaseUrl()}
              onInput={(e) => setConfigBaseUrl(e.currentTarget.value)}
              class="mb-3 w-full rounded border border-border-subtle bg-surface-2 p-2 text-sm text-ink focus:outline-none focus:border-accent"
            />

            <label class="mb-1 block text-xs text-ink-muted">Modelo</label>
            <input
              value={configModel()}
              onInput={(e) => setConfigModel(e.currentTarget.value)}
              placeholder="claudinio"
              class="mb-4 w-full rounded border border-border-subtle bg-surface-2 p-2 text-sm text-ink focus:outline-none focus:border-accent"
            />

            <div class="flex justify-end gap-2">
              <button
                onClick={() => setShowConfig(false)}
                class="rounded border border-border-subtle bg-surface-2 px-3 py-1.5 text-sm text-ink hover:bg-surface-0"
              >
                Cancelar
              </button>
              <button
                onClick={saveConfig}
                class="rounded-md bg-accent px-3 py-1.5 text-sm font-medium text-white hover:opacity-90"
              >
                Salvar
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
                  <span class="text-xs font-semibold uppercase tracking-wide text-ink-muted">
                    Projetos
                  </span>
                </div>

                <div class="flex-1 overflow-y-auto">
                  <For each={recentProjects()}>
                    {(proj) => (
                      <button
                        class="flex w-full items-center gap-2 px-3 py-2 text-left text-sm hover:bg-surface-2"
                        classList={{
                          "bg-surface-2 text-accent": root() === proj,
                        }}
                        onClick={() => openRecent(proj)}
                      >
                        <span class="text-ink-muted">📁</span>
                        <div class="min-w-0">
                          <div class="truncate text-ink">
                            {proj.split("/").pop()}
                          </div>
                          <div class="truncate text-[10px] text-ink-muted">
                            {proj}
                          </div>
                        </div>
                      </button>
                    )}
                  </For>

                  <Show when={recentProjects().length === 0}>
                    <div class="px-3 py-8 text-center text-xs text-ink-muted">
                      Nenhum projeto recente
                    </div>
                  </Show>
                </div>

                <div class="border-t border-border-subtle p-2">
                  <button
                    onClick={openFolder}
                    class="w-full rounded-md border border-dashed border-border-subtle px-3 py-2 text-xs text-ink-muted hover:border-accent hover:text-accent"
                  >
                    + Abrir pasta
                  </button>
                  <Show when={root()}>
                    <button
                      onClick={() => setShowTree(true)}
                      class="mt-1 w-full rounded-md px-3 py-1.5 text-xs text-ink-muted hover:bg-surface-2 hover:text-ink"
                    >
                      ▸ Explorar arquivos
                    </button>
                  </Show>
                </div>
              </>
            }
          >
            <div class="flex items-center gap-2 border-b border-border-subtle px-2 py-1.5">
              <button
                onClick={() => setShowTree(false)}
                class="rounded px-1.5 py-0.5 text-xs text-ink-muted hover:bg-surface-2 hover:text-ink"
              >
                ← Voltar
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

          <Show when={indexStatus()}>
            <div class="border-t border-border-subtle px-2 py-1 text-[10px] text-ink-muted">
              {indexStatus()}
            </div>
          </Show>
        </aside>

        <main class="min-w-0 flex-1">
          <ChatPanel />
        </main>
      </div>
    </div>
  );
}

export default App;
