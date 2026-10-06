import { createSignal, For, onMount, Show, type Component } from "solid-js";
import { getConfig, setConfig, type JevRoute, type JevStatus } from "../../lib/ipc";

const OFF: JevStatus = { enabled: true, hasApiKey: false, backend: null, route: "off" };

const ROUTES: { id: JevRoute; label: string; hint: string }[] = [
  { id: "off", label: "Off", hint: "You pick Brain or Builder. Nothing is asked." },
  {
    id: "shadow",
    label: "Shadow",
    hint: "You pick. Jev's choice for each new session is recorded next to yours, and changes nothing.",
  },
  {
    id: "on",
    label: "On",
    hint: "New sessions start on Auto: Jev sends a request that needs decisions and a design to Brain, and everything else to Builder. You can still pick either by hand.",
  },
];

/**
 * Jev (TypeSafe "System One") answers the harness's own yes/no questions —
 * is this turn really finished, is the agent going in circles, which part of a
 * long command output matters — for a fraction of a cent, instead of a call to
 * the chat model. Included in Claudinio plans; otherwise it runs on the user's
 * own TypeSafe key or OpenRouter connection.
 */
export const SettingsJev: Component = () => {
  const [status, setStatus] = createSignal<JevStatus>(OFF);
  const [key, setKey] = createSignal("");
  const [saved, setSaved] = createSignal(false);

  const reload = async () => {
    const cfg = await getConfig();
    setStatus({ ...OFF, ...(cfg.jev ?? {}) });
  };
  onMount(reload);

  const toggle = async (enabled: boolean) => {
    setStatus({ ...status(), enabled });
    await setConfig({ jevEnabled: enabled });
  };

  const setRoute = async (route: JevRoute) => {
    setStatus({ ...status(), route });
    await setConfig({ jevRoute: route });
  };

  const saveKey = async (value: string) => {
    await setConfig({ jevApiKey: value.trim() });
    setKey("");
    setSaved(true);
    await reload();
  };

  const backendLabel = () => {
    const s = status();
    if (!s.enabled) return "Off";
    if (s.backend === "typesafe") return "Active — using your TypeSafe key";
    if (s.backend === "claudinio") return "Active — included in your Claudinio plan";
    if (s.backend === "openrouter") return "Active — using your OpenRouter connection";
    return "Inactive — sign in with claudin.io, add a TypeSafe key, or connect OpenRouter in Account";
  };

  return (
    <div class="mt-6 border-t border-border-subtle pt-4">
      <label class="mb-1 flex cursor-pointer items-center gap-2">
        <input
          type="checkbox"
          checked={status().enabled}
          onChange={(e) => toggle(e.currentTarget.checked)}
          class="h-4 w-4 rounded border-border-subtle bg-surface-0 text-accent focus:ring-accent"
        />
        <span class="text-sm font-medium text-ink">{"Jev decisions (TypeSafe)"}</span>
      </label>
      <p class="mb-2 text-[11px] text-ink-faint">
        {
          "A small decision model checks whether a turn is really finished, notices when the agent is going in circles, and keeps the important parts of long command outputs. Included in Claudinio plans; with your own TypeSafe or OpenRouter key it costs about $0.00002 per decision."
        }
      </p>
      <p class="mb-3 text-xs text-ink-muted" data-jev-status>
        {backendLabel()}
      </p>

      <label class="mb-1 block text-xs text-ink-muted">{"TypeSafe API key (optional)"}</label>
      <div class="mb-1 flex gap-2">
        <input
          type="password"
          value={key()}
          onInput={(e) => setKey(e.currentTarget.value)}
          placeholder={status().hasApiKey ? "•••••••• (saved)" : "ts-…"}
          class="flex-1 rounded-md border border-border-subtle bg-surface-0 p-2 text-sm text-ink placeholder:text-ink-muted focus:border-accent focus:outline-none focus:ring-1 focus:ring-accent"
        />
        <button
          data-jev-save
          type="button"
          onClick={() => saveKey(key())}
          class="rounded-md border border-border-subtle bg-surface-2 px-3 text-xs text-ink hover:bg-surface-3"
        >
          {"Save"}
        </button>
        <Show when={status().hasApiKey}>
          <button
            type="button"
            onClick={() => saveKey("")}
            class="rounded-md border border-border-subtle px-3 text-xs text-ink-muted hover:bg-surface-2"
          >
            {"Remove"}
          </button>
        </Show>
      </div>
      <p class="text-[11px] text-ink-faint">
        {"Order of use: your TypeSafe key, then your Claudinio plan, then OpenRouter if connected."}
        <Show when={saved()}>
          <span class="ml-1 text-accent">{"Saved."}</span>
        </Show>
      </p>

      <label class="mb-1 mt-4 block text-xs text-ink-muted">{"Auto mode for new sessions"}</label>
      <div class="mb-1 flex gap-2">
        <For each={ROUTES}>
          {(r) => (
            <button
              type="button"
              data-jev-route={r.id}
              aria-pressed={(status().route ?? "off") === r.id}
              disabled={!status().enabled}
              onClick={() => setRoute(r.id)}
              class="flex-1 rounded-md border p-2 text-sm transition-colors disabled:opacity-50"
              classList={{
                "border-accent bg-accent/10 text-accent": (status().route ?? "off") === r.id,
                "border-border-subtle bg-surface-0 text-ink hover:bg-surface-2":
                  (status().route ?? "off") !== r.id,
              }}
            >
              {r.label}
            </button>
          )}
        </For>
      </div>
      <p class="text-[11px] text-ink-faint" data-jev-route-hint>
        {ROUTES.find((r) => r.id === (status().route ?? "off"))!.hint}
        {" On and Shadow send the first message of each new session to Jev, and nothing after it."}
      </p>
    </div>
  );
};
