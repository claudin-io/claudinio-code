import { Component, createSignal, For, onMount, Show, type Accessor } from "solid-js";
import { Icon } from "../Icon";
import {
  inspectPlugin,
  installPluginFromPath,
  installPluginFromUrl,
  listPlugins,
  openExternalUrl,
  pickFolder,
  scaffoldPlugin,
  setPluginEnabled,
  uninstallPlugin,
  type PluginInfo,
  type PluginInstallScope,
} from "../../lib/ipc";

interface SettingsPluginsProps {
  workspaceRoot: Accessor<string | null>;
}

type Pane = "list" | "url" | "create";

/**
 * The Agent Plugins explorer (https://agent-plugins.org).
 *
 * A plugin is a directory with `plugin.json` plus optional `skills/` and
 * `mcp.json`. Installing one is a copy into `~/.claudinio/plugins/` (or the
 * project's `.claudinio/plugins/`), so the packages stay inspectable on disk.
 *
 * Enable state lives in the app config rather than the package, so toggling a
 * plugin never edits files its author owns.
 */
export const SettingsPlugins: Component<SettingsPluginsProps> = (props) => {
  const [plugins, setPlugins] = createSignal<PluginInfo[]>([]);
  const [loading, setLoading] = createSignal(true);
  const [error, setError] = createSignal<string | null>(null);
  const [notice, setNotice] = createSignal<string | null>(null);
  const [busy, setBusy] = createSignal(false);
  const [pane, setPane] = createSignal<Pane>("list");
  const [expanded, setExpanded] = createSignal<string | null>(null);

  // Install-from-URL form
  const [url, setUrl] = createSignal("");
  const [gitRef, setGitRef] = createSignal("");
  const [subdir, setSubdir] = createSignal("");
  const [scope, setScope] = createSignal<PluginInstallScope>("user");

  // Crafter form
  const [newName, setNewName] = createSignal("");
  const [newDescription, setNewDescription] = createSignal("");
  const [newAuthor, setNewAuthor] = createSignal("");
  const [newLicense, setNewLicense] = createSignal("MIT");
  const [newSkillName, setNewSkillName] = createSignal("");
  const [newSkillDescription, setNewSkillDescription] = createSignal("");

  const refresh = async () => {
    setLoading(true);
    setError(null);
    try {
      setPlugins(await listPlugins(props.workspaceRoot()));
    } catch (e) {
      setError(String(e));
    } finally {
      setLoading(false);
    }
  };

  onMount(() => void refresh());

  /** Run an action, then refresh. An action that returns a string reports that
   * instead of the default label — the crafter uses it to show the new path. */
  const run = async (label: string, action: () => Promise<unknown>) => {
    setBusy(true);
    setError(null);
    setNotice(null);
    try {
      const outcome = await action();
      setNotice(typeof outcome === "string" ? outcome : label);
      await refresh();
    } catch (e) {
      setError(String(e));
    } finally {
      setBusy(false);
    }
  };

  const installFromFolder = async () => {
    const folder = await pickFolder(props.workspaceRoot() ?? undefined);
    if (!folder) return;
    // Report what is wrong before copying anything into the plugins directory.
    const preview = await inspectPlugin(folder).catch(() => null);
    if (preview && !preview.valid) {
      setError(preview.diagnostics[0]?.message ?? "not a valid plugin");
      return;
    }
    await run(`Installed ${preview?.name ?? "plugin"}`, () =>
      installPluginFromPath({
        path: folder,
        scope: scope(),
        workspace: props.workspaceRoot(),
      }),
    );
  };

  const installFromUrl = async () => {
    const value = url().trim();
    if (!value) {
      setError("Paste a git or GitHub URL first.");
      return;
    }
    await run(`Installed from ${value}`, async () => {
      await installPluginFromUrl({
        url: value,
        gitRef: gitRef().trim() || null,
        subdir: subdir().trim() || null,
        scope: scope(),
        workspace: props.workspaceRoot(),
      });
      setUrl("");
      setGitRef("");
      setSubdir("");
      setPane("list");
    });
  };

  const create = async () => {
    const name = newName().trim();
    if (!name) {
      setError("A plugin needs a name.");
      return;
    }
    await run(`Created ${name}`, async () => {
      const skillName = newSkillName().trim();
      const result = await scaffoldPlugin({
        name,
        description: newDescription().trim() || undefined,
        authorName: newAuthor().trim() || undefined,
        license: newLicense().trim() || undefined,
        skills: skillName
          ? [
              {
                name: skillName,
                description:
                  newSkillDescription().trim() || `Use when working with ${skillName}.`,
              },
            ]
          : [],
        scope: scope(),
        workspace: props.workspaceRoot(),
      });
      setNewName("");
      setNewDescription("");
      setNewSkillName("");
      setNewSkillDescription("");
      setPane("list");
      return `Created at ${result.root}`;
    });
  };

  const toggle = (plugin: PluginInfo) =>
    run(`${plugin.enabled ? "Disabled" : "Enabled"} ${plugin.name}`, () =>
      setPluginEnabled(plugin.name, !plugin.enabled, props.workspaceRoot()),
    );

  const remove = (plugin: PluginInfo) =>
    run(`Uninstalled ${plugin.name}`, () =>
      uninstallPlugin(plugin.name, props.workspaceRoot()),
    );

  const componentSummary = (plugin: PluginInfo) => {
    const parts: string[] = [];
    if (plugin.skills.length) {
      parts.push(`${plugin.skills.length} skill${plugin.skills.length === 1 ? "" : "s"}`);
    }
    if (plugin.mcpServers.length) {
      parts.push(
        `${plugin.mcpServers.length} MCP server${plugin.mcpServers.length === 1 ? "" : "s"}`,
      );
    }
    return parts.length ? parts.join(" · ") : "no components";
  };

  return (
    <div class="flex flex-col gap-3">
      <div class="flex items-center justify-between">
        <div>
          <h3 class="text-sm font-semibold text-ink">{"Plugins"}</h3>
          <p class="text-[11px] text-ink-faint">
            {"Portable packages of skills and MCP servers, per the Agent Plugins spec."}
          </p>
        </div>
        <button
          onClick={() => openExternalUrl("https://agent-plugins.org")}
          class="flex items-center gap-1 text-xs text-accent hover:underline"
        >
          {"agent-plugins.org"}
          <Icon name="external-link" class="h-3 w-3" />
        </button>
      </div>

      <div class="flex flex-wrap items-center gap-2">
        <button
          disabled={busy()}
          onClick={() => void installFromFolder()}
          class="flex items-center gap-1.5 rounded-md border border-border-subtle bg-surface-0 px-3 py-1.5 text-sm text-ink hover:bg-surface-2 disabled:opacity-50"
        >
          <Icon name="folder-open" class="h-4 w-4" />
          {"Install from folder"}
        </button>
        <button
          disabled={busy()}
          onClick={() => setPane(pane() === "url" ? "list" : "url")}
          class="flex items-center gap-1.5 rounded-md border border-border-subtle bg-surface-0 px-3 py-1.5 text-sm text-ink hover:bg-surface-2 disabled:opacity-50"
        >
          <Icon name="external-link" class="h-4 w-4" />
          {"Install from URL"}
        </button>
        <button
          disabled={busy()}
          onClick={() => setPane(pane() === "create" ? "list" : "create")}
          class="flex items-center gap-1.5 rounded-md border border-border-subtle bg-surface-0 px-3 py-1.5 text-sm text-ink hover:bg-surface-2 disabled:opacity-50"
        >
          <Icon name="package" class="h-4 w-4" />
          {"Create plugin"}
        </button>
        <div class="ml-auto flex items-center gap-2">
          <label class="text-[11px] text-ink-muted">{"Install to"}</label>
          <select
            value={scope()}
            onChange={(e) => setScope(e.currentTarget.value as PluginInstallScope)}
            class="rounded border border-border-subtle bg-surface-0 px-2 py-1 text-xs text-ink"
          >
            <option value="user">{"User (~/.claudinio/plugins)"}</option>
            <option value="project" disabled={!props.workspaceRoot()}>
              {"Project (.claudinio/plugins)"}
            </option>
          </select>
        </div>
      </div>

      <Show when={pane() === "url"}>
        <div class="flex flex-col gap-2 rounded-md border border-border-subtle bg-surface-0 p-3">
          <label class="text-xs text-ink-muted">{"Repository URL"}</label>
          <input
            type="text"
            value={url()}
            onInput={(e) => setUrl(e.currentTarget.value)}
            placeholder="https://github.com/owner/repo/tree/main/plugins/my-plugin"
            class="w-full rounded border border-border-subtle bg-surface-1 px-2 py-1.5 text-sm text-ink placeholder:text-ink-faint focus:border-accent focus:outline-none"
          />
          <p class="text-[11px] text-ink-faint">
            {
              "A GitHub tree URL carries the branch and subdirectory. For any other git host, fill them in below."
            }
          </p>
          <div class="flex gap-2">
            <input
              type="text"
              value={gitRef()}
              onInput={(e) => setGitRef(e.currentTarget.value)}
              placeholder="branch or tag (optional)"
              class="w-1/2 rounded border border-border-subtle bg-surface-1 px-2 py-1.5 text-sm text-ink placeholder:text-ink-faint focus:border-accent focus:outline-none"
            />
            <input
              type="text"
              value={subdir()}
              onInput={(e) => setSubdir(e.currentTarget.value)}
              placeholder="subdirectory (optional)"
              class="w-1/2 rounded border border-border-subtle bg-surface-1 px-2 py-1.5 text-sm text-ink placeholder:text-ink-faint focus:border-accent focus:outline-none"
            />
          </div>
          <div class="flex justify-end">
            <button
              disabled={busy()}
              onClick={() => void installFromUrl()}
              class="rounded-md bg-accent px-3 py-1.5 text-sm font-medium text-accent-ink hover:bg-accent-hover disabled:opacity-50"
            >
              {busy() ? "Installing…" : "Install"}
            </button>
          </div>
        </div>
      </Show>

      <Show when={pane() === "create"}>
        <div class="flex flex-col gap-2 rounded-md border border-border-subtle bg-surface-0 p-3">
          <p class="text-[11px] text-ink-faint">
            {
              "Scaffolds a spec-compliant package. For richer plugins, ask the agent — the built-in plugin-crafter skill writes and validates them."
            }
          </p>
          <div class="flex gap-2">
            <input
              type="text"
              value={newName()}
              onInput={(e) => setNewName(e.currentTarget.value)}
              placeholder="plugin-name (lowercase, digits, - and .)"
              class="w-1/2 rounded border border-border-subtle bg-surface-1 px-2 py-1.5 text-sm text-ink placeholder:text-ink-faint focus:border-accent focus:outline-none"
            />
            <input
              type="text"
              value={newAuthor()}
              onInput={(e) => setNewAuthor(e.currentTarget.value)}
              placeholder="author (optional)"
              class="w-1/4 rounded border border-border-subtle bg-surface-1 px-2 py-1.5 text-sm text-ink placeholder:text-ink-faint focus:border-accent focus:outline-none"
            />
            <input
              type="text"
              value={newLicense()}
              onInput={(e) => setNewLicense(e.currentTarget.value)}
              placeholder="license"
              class="w-1/4 rounded border border-border-subtle bg-surface-1 px-2 py-1.5 text-sm text-ink placeholder:text-ink-faint focus:border-accent focus:outline-none"
            />
          </div>
          <input
            type="text"
            value={newDescription()}
            onInput={(e) => setNewDescription(e.currentTarget.value)}
            placeholder="What the plugin does, in one line"
            class="w-full rounded border border-border-subtle bg-surface-1 px-2 py-1.5 text-sm text-ink placeholder:text-ink-faint focus:border-accent focus:outline-none"
          />
          <div class="flex gap-2">
            <input
              type="text"
              value={newSkillName()}
              onInput={(e) => setNewSkillName(e.currentTarget.value)}
              placeholder="first skill name (optional)"
              class="w-1/3 rounded border border-border-subtle bg-surface-1 px-2 py-1.5 text-sm text-ink placeholder:text-ink-faint focus:border-accent focus:outline-none"
            />
            <input
              type="text"
              value={newSkillDescription()}
              onInput={(e) => setNewSkillDescription(e.currentTarget.value)}
              placeholder="when the agent should use that skill"
              class="w-2/3 rounded border border-border-subtle bg-surface-1 px-2 py-1.5 text-sm text-ink placeholder:text-ink-faint focus:border-accent focus:outline-none"
            />
          </div>
          <div class="flex justify-end">
            <button
              disabled={busy()}
              onClick={() => void create()}
              class="rounded-md bg-accent px-3 py-1.5 text-sm font-medium text-accent-ink hover:bg-accent-hover disabled:opacity-50"
            >
              {busy() ? "Creating…" : "Create"}
            </button>
          </div>
        </div>
      </Show>

      <Show when={error()}>
        <p class="rounded border border-red-500/30 bg-red-500/10 px-3 py-2 text-xs text-red-400">
          {error()}
        </p>
      </Show>
      <Show when={notice() && !error()}>
        <p class="text-xs text-green-500">{notice()}</p>
      </Show>

      <Show
        when={!loading()}
        fallback={<p class="text-sm text-ink-faint">{"Scanning for plugins…"}</p>}
      >
        <Show
          when={plugins().length > 0}
          fallback={
            <p class="rounded-md border border-dashed border-border-subtle px-3 py-6 text-center text-sm text-ink-faint">
              {"No plugins installed yet."}
            </p>
          }
        >
          <div class="flex flex-col gap-2">
            <For each={plugins()}>
              {(plugin) => (
                <div class="rounded-md border border-border-subtle bg-surface-0 p-3">
                  <div class="flex items-start justify-between gap-3">
                    <button
                      type="button"
                      onClick={() =>
                        setExpanded(expanded() === plugin.name ? null : plugin.name)
                      }
                      class="min-w-0 flex-1 text-left"
                    >
                      <div class="flex flex-wrap items-center gap-2">
                        <span class="text-sm font-medium text-ink">{plugin.name}</span>
                        <Show when={plugin.version}>
                          <span class="text-[11px] text-ink-faint">{`v${plugin.version}`}</span>
                        </Show>
                        <span class="rounded border border-border-subtle px-1.5 py-px text-[10px] text-ink-muted">
                          {plugin.scope === "project" ? "project" : "user"}
                        </span>
                        <Show when={!plugin.valid}>
                          <span class="rounded border border-red-500/40 bg-red-500/10 px-1.5 py-px text-[10px] text-red-400">
                            {"invalid"}
                          </span>
                        </Show>
                        <Show when={plugin.valid && !plugin.enabled}>
                          <span class="rounded border border-border-subtle px-1.5 py-px text-[10px] text-ink-faint">
                            {"disabled"}
                          </span>
                        </Show>
                      </div>
                      <p class="mt-0.5 truncate text-xs text-ink-muted">
                        {plugin.description ?? plugin.root}
                      </p>
                      <p class="mt-0.5 text-[11px] text-ink-faint">
                        {componentSummary(plugin)}
                      </p>
                    </button>
                    <div class="flex shrink-0 items-center gap-2">
                      <Show when={plugin.valid}>
                        <button
                          disabled={busy()}
                          onClick={() => void toggle(plugin)}
                          class="rounded-md border border-border-subtle bg-surface-1 px-2 py-1 text-xs text-ink hover:bg-surface-2 disabled:opacity-50"
                        >
                          {plugin.enabled ? "Disable" : "Enable"}
                        </button>
                      </Show>
                      <button
                        disabled={busy()}
                        onClick={() => void remove(plugin)}
                        class="rounded-md border border-border-subtle bg-surface-1 px-2 py-1 text-xs text-red-400 hover:bg-surface-2 disabled:opacity-50"
                      >
                        {"Uninstall"}
                      </button>
                    </div>
                  </div>

                  <Show when={plugin.diagnostics.length > 0}>
                    <ul class="mt-2 flex flex-col gap-1">
                      <For each={plugin.diagnostics}>
                        {(d) => (
                          <li
                            class="flex items-start gap-1.5 text-[11px]"
                            classList={{
                              "text-red-400": d.severity === "error",
                              "text-amber-500": d.severity === "warning",
                            }}
                          >
                            <Icon
                              name={d.severity === "error" ? "alert-circle" : "alert-triangle"}
                              class="mt-px h-3 w-3 shrink-0"
                            />
                            <span>{d.message}</span>
                          </li>
                        )}
                      </For>
                    </ul>
                  </Show>

                  <Show when={expanded() === plugin.name}>
                    <div class="mt-3 border-t border-border-subtle pt-2 text-xs text-ink-muted">
                      <p class="break-all text-[11px] text-ink-faint">{plugin.root}</p>
                      <Show when={plugin.skills.length > 0}>
                        <p class="mt-2 font-medium text-ink">{"Skills"}</p>
                        <For each={plugin.skills}>
                          {(s) => (
                            <p class="mt-0.5">
                              <span class="text-ink">{s.name}</span>
                              {` — ${s.description}`}
                            </p>
                          )}
                        </For>
                      </Show>
                      <Show when={plugin.mcpServers.length > 0}>
                        <p class="mt-2 font-medium text-ink">{"MCP servers"}</p>
                        <For each={plugin.mcpServers}>
                          {(s) => (
                            <p class="mt-0.5">
                              <span class="text-ink">{s.qualifiedName}</span>
                              {` — ${s.transport}`}
                            </p>
                          )}
                        </For>
                      </Show>
                      <Show when={plugin.homepage || plugin.repository}>
                        <div class="mt-2 flex gap-3">
                          <Show when={plugin.homepage}>
                            <button
                              onClick={() => openExternalUrl(plugin.homepage!)}
                              class="text-accent hover:underline"
                            >
                              {"Homepage"}
                            </button>
                          </Show>
                          <Show when={plugin.repository}>
                            <button
                              onClick={() => openExternalUrl(plugin.repository!)}
                              class="text-accent hover:underline"
                            >
                              {"Repository"}
                            </button>
                          </Show>
                        </div>
                      </Show>
                    </div>
                  </Show>
                </div>
              )}
            </For>
          </div>
        </Show>
      </Show>
    </div>
  );
};
