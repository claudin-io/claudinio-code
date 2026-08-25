import { Component, createEffect, createSignal, For, Show, type Accessor } from "solid-js";
import {
  approveHooks,
  listHooks,
  reloadHooks,
  revokeHooks,
  setHooksEnabled,
  testHook,
  type HookInfo,
  type HookRunView,
  type HooksInfo,
} from "../../lib/ipc";

interface SettingsHooksProps {
  workspaceRoot: Accessor<string | null>;
}

/** The nine events, in the order a session goes through them. */
const EVENT_ORDER = [
  "SessionStart",
  "UserPromptSubmit",
  "PreToolUse",
  "PostToolUse",
  "Notification",
  "Stop",
  "SubagentStop",
  "PreCompact",
  "SessionEnd",
];

const SOURCE_LABEL: Record<string, string> = {
  userSettings: "user settings",
  appConfig: "app settings",
  plugin: "plugin",
  workspaceConfig: ".claudinio.json",
  projectSettings: "project settings",
  localSettings: "local settings",
};

/**
 * Lifecycle hooks.
 *
 * Two jobs, and the second one is the reason this panel is worth its space.
 *
 * The first is consent: a hook is arbitrary code a repository can ship, so
 * nothing runs until the user has seen the resolved commands and approved them.
 *
 * The second is diagnosis. The characteristic failure of hooks everywhere is a
 * config that installs cleanly, runs on every prompt and does nothing — a
 * matcher that matches no tool, a binary that is not installed, a script that
 * prints something the harness does not read. So every row says which tools its
 * matcher will actually hit, and every row has a button that runs it and prints
 * the exit code.
 */
export const SettingsHooks: Component<SettingsHooksProps> = (props) => {
  const [info, setInfo] = createSignal<HooksInfo | null>(null);
  const [error, setError] = createSignal<string | null>(null);
  const [busy, setBusy] = createSignal(false);
  const [runs, setRuns] = createSignal<Record<number, HookRunView>>({});

  const load = async (ws: string | null) => {
    try {
      setInfo(await listHooks(ws));
      setError(null);
    } catch (e) {
      setError(String(e));
    }
  };

  createEffect(() => {
    const ws = props.workspaceRoot();
    setRuns({});
    void load(ws);
  });

  const act = (fn: () => Promise<HooksInfo>) => {
    setBusy(true);
    void (async () => {
      try {
        setInfo(await fn());
        setError(null);
      } catch (e) {
        setError(String(e));
      } finally {
        setBusy(false);
      }
    })();
  };

  const run = (index: number) => {
    const ws = props.workspaceRoot();
    if (!ws) return;
    void (async () => {
      try {
        const result = await testHook(ws, index);
        setRuns({ ...runs(), [index]: result });
      } catch (e) {
        setError(String(e));
      }
    })();
  };

  const byEvent = (event: string): Array<{ hook: HookInfo; index: number }> =>
    (info()?.hooks ?? [])
      .map((hook, index) => ({ hook, index }))
      .filter((h) => h.hook.event === event);

  const trustLabel = () => {
    switch (info()?.trust) {
      case "trusted":
        return "Approved — these hooks run";
      case "changed":
        return "A command changed since you approved — nothing runs until you review it";
      case "pending":
        return "Waiting for your approval — nothing runs yet";
      default:
        return "No hooks found in this project";
    }
  };

  const trustClass = () =>
    info()?.trust === "trusted"
      ? "border-emerald-500/40 bg-emerald-500/10 text-emerald-500"
      : info()?.trust === "noHooks"
        ? "border-border-subtle bg-surface-0 text-ink-muted"
        : "border-amber-500/40 bg-amber-500/10 text-amber-500";

  return (
    <>
      <label class="mb-1 flex cursor-pointer items-center gap-2">
        <input
          type="checkbox"
          checked={info()?.enabled ?? true}
          onChange={(e) =>
            act(() => setHooksEnabled(e.currentTarget.checked, props.workspaceRoot()))
          }
          class="h-4 w-4 rounded border-border-subtle bg-surface-0 text-accent focus:ring-accent"
        />
        <span class="text-sm font-medium text-ink">{"Lifecycle hooks"}</span>
      </label>
      <p class="mb-4 text-[11px] text-ink-faint">
        {
          "Programs Claudinio runs at fixed points in a session — before a tool call, on every prompt, when the context is about to be compacted. Configured in Claude Code's format, so a hooks block written for it works here unedited: ~/.claude/settings.json, this project's .claude/settings.json and settings.local.json, the hooks key in .claudinio.json, and any installed plugin. Every source contributes; none overrides another."
        }
      </p>

      <Show
        when={props.workspaceRoot()}
        fallback={
          <p class="text-sm text-ink-muted">
            {"Open a project to see its hooks — a hook is scoped to a workspace, and one that could run anywhere would have no blast radius anybody can reason about."}
          </p>
        }
      >
        <Show when={info()}>
          {(data) => (
            <>
              <div class={`mb-3 rounded-md border p-3 text-xs ${trustClass()}`}>
                <div class="font-medium">{trustLabel()}</div>
                <Show when={data().trust !== "noHooks"}>
                  <div class="mt-2 flex flex-wrap items-center gap-2">
                    <Show when={data().trust !== "trusted"}>
                      <button
                        type="button"
                        disabled={busy()}
                        onClick={() =>
                          act(() =>
                            approveHooks(props.workspaceRoot()!, data().fingerprint),
                          )
                        }
                        class="rounded-md bg-accent px-2 py-1 text-xs font-medium text-white disabled:opacity-50"
                      >
                        {`Allow these ${data().hooks.length} hook${data().hooks.length === 1 ? "" : "s"}`}
                      </button>
                    </Show>
                    <Show when={data().trust === "trusted" || data().trust === "changed"}>
                      <button
                        type="button"
                        disabled={busy()}
                        onClick={() => act(() => revokeHooks(props.workspaceRoot()!))}
                        class="rounded-md border border-border-subtle px-2 py-1 text-xs text-ink disabled:opacity-50"
                      >
                        {"Revoke"}
                      </button>
                    </Show>
                    <button
                      type="button"
                      disabled={busy()}
                      onClick={() => act(() => reloadHooks(props.workspaceRoot()!))}
                      class="rounded-md border border-border-subtle px-2 py-1 text-xs text-ink disabled:opacity-50"
                      title="A run keeps the hooks it started with. This makes the next message re-read them from disk."
                    >
                      {"Reload from disk"}
                    </button>
                  </div>
                </Show>
              </div>

              <Show when={data().diagnostics.length > 0}>
                <div class="mb-3 rounded-md border border-amber-500/40 bg-amber-500/10 p-3">
                  <div class="mb-1 text-xs font-medium text-amber-500">
                    {"Problems in the configuration"}
                  </div>
                  <For each={data().diagnostics}>
                    {(d) => (
                      <div class="text-[11px] text-ink-muted">
                        <span class="font-mono">{d.source}</span>
                        {": "}
                        {d.message}
                      </div>
                    )}
                  </For>
                </div>
              </Show>

              <For each={EVENT_ORDER.filter((e) => byEvent(e).length > 0)}>
                {(event) => (
                  <>
                    <div class="mb-1 mt-3 text-xs font-medium text-ink-muted">{event}</div>
                    <For each={byEvent(event)}>
                      {({ hook, index }) => (
                        <div class="mb-2 rounded-md border border-border-subtle bg-surface-0 p-2">
                          <div class="flex items-start justify-between gap-2">
                            <code class="break-all text-[11px] text-ink">{hook.display}</code>
                            <button
                              type="button"
                              onClick={() => run(index)}
                              class="shrink-0 rounded border border-border-subtle px-1.5 py-0.5 text-[10px] text-ink"
                              title="Run this hook now against a synthetic payload"
                            >
                              {"Run now"}
                            </button>
                          </div>
                          <div class="mt-1 flex flex-wrap items-center gap-x-3 gap-y-1 text-[10px] text-ink-faint">
                            <span>
                              {SOURCE_LABEL[hook.sourceKind] ?? hook.sourceKind}
                              {" · "}
                              <span class="font-mono">{hook.source}</span>
                            </span>
                            <span>{`${hook.timeoutSecs}s timeout`}</span>
                            <Show when={hook.matcher}>
                              <span>{`matcher: ${hook.matcher}`}</span>
                            </Show>
                            <Show when={hook.alsoFrom.length > 0}>
                              <span>{`also declared in ${hook.alsoFrom.length} other place${hook.alsoFrom.length === 1 ? "" : "s"} · runs once`}</span>
                            </Show>
                          </div>
                          <Show when={!hook.matcherValid}>
                            <div class="mt-1 text-[10px] text-amber-500">
                              {"This matcher is not a valid regex — it will only match that exact name."}
                            </div>
                          </Show>
                          <Show
                            when={
                              hook.hits.length === 0 &&
                              (hook.event === "PreToolUse" ||
                                hook.event === "PostToolUse" ||
                                hook.event === "PreCompact" ||
                                hook.event === "SessionStart")
                            }
                          >
                            <div class="mt-1 text-[10px] text-amber-500">
                              {"This matcher selects nothing — the hook will never fire."}
                            </div>
                          </Show>
                          <Show when={hook.hits.length > 0}>
                            <div class="mt-1 text-[10px] text-ink-faint">
                              {`fires on: ${hook.hits.join(", ")}`}
                            </div>
                          </Show>
                          <Show when={runs()[index]}>
                            {(r) => (
                              <div class="mt-2 border-t border-border-subtle pt-2 text-[10px]">
                                <div
                                  class={
                                    r().status === "ok" ? "text-emerald-500" : "text-amber-500"
                                  }
                                >
                                  {`${r().status}${r().exitCode !== null ? ` · exit ${r().exitCode}` : ""} · ${r().durationMs}ms`}
                                </div>
                                <Show when={r().additionalContext}>
                                  <pre class="mt-1 whitespace-pre-wrap break-all text-ink-muted">
                                    {`context added: ${r().additionalContext}`}
                                  </pre>
                                </Show>
                                <Show when={r().error}>
                                  <pre class="mt-1 whitespace-pre-wrap break-all text-amber-500">
                                    {r().error}
                                  </pre>
                                </Show>
                                <Show when={r().stdout}>
                                  <pre class="mt-1 max-h-24 overflow-auto whitespace-pre-wrap break-all text-ink-faint">
                                    {r().stdout}
                                  </pre>
                                </Show>
                                <Show when={r().stderr}>
                                  <pre class="mt-1 max-h-24 overflow-auto whitespace-pre-wrap break-all text-ink-faint">
                                    {r().stderr}
                                  </pre>
                                </Show>
                              </div>
                            )}
                          </Show>
                        </div>
                      )}
                    </For>
                  </>
                )}
              </For>

              <Show when={data().hooks.length === 0 && data().enabled}>
                <p class="text-sm text-ink-muted">
                  {"No hooks declared for this project."}
                </p>
              </Show>

              <p class="mt-4 text-[11px] text-ink-faint">
                {
                  "One difference from Claude Code worth knowing: transcript_path points at this session's .claudinio/sessions/*.jsonl, which is a complete transcript in Claudinio's own format rather than Claude Code's. Hooks that only locate the file are fine; one that parses it can read transcript_format and bail."
                }
              </p>
            </>
          )}
        </Show>
      </Show>

      <Show when={error()}>
        <p class="mt-2 text-xs text-red-500">{error()}</p>
      </Show>
    </>
  );
};
