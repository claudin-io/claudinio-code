import { createSignal, For, onMount, type Component } from "solid-js";
import { getConfig, setConfig, type PromptVersion } from "../../lib/ipc";

const VERSIONS: { id: PromptVersion; label: string; hint: string }[] = [
  {
    id: "v2",
    label: "v2 (default)",
    hint: "The lean prompt: it explains how the tools fit together and leaves the rules to the tools' own errors; Brain asks only what changes the plan. Takes effect from the next message.",
  },
  {
    id: "v1",
    label: "v1",
    hint: "The previous, longer prompt with its full rule lists and step-by-step Brain interview, kept for one release. Takes effect from the next message.",
  },
];

/**
 * Which Standard system prompt sessions are sent (lean-harness Phase 5,
 * decision D7): v2 by default, v1 selectable for one release so a user who
 * prefers the old behaviour can flip back. The Compact profile and the git
 * operator have one prompt each and ignore it.
 */
export const SettingsPromptVersion: Component = () => {
  const [version, setVersion] = createSignal<PromptVersion>("v2");

  onMount(async () => {
    const cfg = await getConfig();
    setVersion(cfg.promptVersion ?? "v2");
  });

  const choose = async (v: PromptVersion) => {
    setVersion(v);
    await setConfig({ promptVersion: v });
  };

  return (
    <div class="mt-6 border-t border-border-subtle pt-4">
      <label class="mb-1 block text-xs text-ink-muted">{"System prompt"}</label>
      <div class="mb-1 flex gap-2">
        <For each={VERSIONS}>
          {(v) => (
            <button
              type="button"
              data-prompt-version={v.id}
              aria-pressed={version() === v.id}
              onClick={() => choose(v.id)}
              class="flex-1 rounded-md border p-2 text-sm transition-colors"
              classList={{
                "border-accent bg-accent/10 text-accent": version() === v.id,
                "border-border-subtle bg-surface-0 text-ink hover:bg-surface-2": version() !== v.id,
              }}
            >
              {v.label}
            </button>
          )}
        </For>
      </div>
      <p class="text-[11px] text-ink-faint" data-prompt-version-hint>
        {VERSIONS.find((v) => v.id === version())!.hint}
      </p>
    </div>
  );
};
