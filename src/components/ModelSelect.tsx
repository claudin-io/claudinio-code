import { Component, For, Show, createMemo, createSignal, type Accessor } from "solid-js";
import { Popover } from "./Popover";
import { Icon } from "./Icon";
import type { ModelGroup } from "../lib/ipc";

interface ModelSelectProps {
  value: Accessor<string>;
  onChange: (model: string) => void;
  groups: Accessor<ModelGroup[]>;
  disabled?: Accessor<boolean>;
  /** Extra classList entries forwarded to the trigger button. */
  triggerClassList?: Accessor<Record<string, boolean>>;
}

/** How a model id is shown inside its own group.
 *
 * A group may supply a label, which wins: a local model's id is a content hash
 * ("local/b55d3d9e4269fdb3") and reads as noise. Otherwise the provider prefix
 * is stripped — "openrouter/openai/gpt-4o-mini" shows as "openai/gpt-4o-mini"
 * inside the OpenRouter group. Claudinio ids are unqualified and pass through. */
function displayName(qualifiedId: string, group: ModelGroup): string {
  const label = group.labels?.[qualifiedId];
  if (label) return label;
  const prefix = `${group.providerId}/`;
  return qualifiedId.startsWith(prefix) ? qualifiedId.slice(prefix.length) : qualifiedId;
}

/** The label for whatever is currently selected, searched across every group
 * because the trigger button has no group context of its own. Falls back to the
 * raw id, which is right for a model that is configured but no longer listed. */
function selectedLabel(value: string, groups: ModelGroup[]): string {
  for (const g of groups) {
    const label = g.labels?.[value];
    if (label) return label;
  }
  return value;
}

/** Searchable, provider-grouped model picker. Claudinio's group comes first
 * (the backend guarantees the order); external groups carry an
 * "Experimental" badge. */
export const ModelSelect: Component<ModelSelectProps> = (props) => {
  const [open, setOpen] = createSignal(false);
  const [query, setQuery] = createSignal("");
  let triggerRef: HTMLButtonElement | undefined;
  let searchRef: HTMLInputElement | undefined;

  const filteredGroups = createMemo(() => {
    const q = query().trim().toLowerCase();
    if (!q) return props.groups();
    return props.groups()
      .map((g) => ({
        ...g,
        models: g.models.filter(
          (m) =>
            m.toLowerCase().includes(q) ||
            displayName(m, g).toLowerCase().includes(q) ||
            g.providerName.toLowerCase().includes(q),
        ),
      }))
      .filter((g) => g.models.length > 0);
  });

  const openPicker = () => {
    if (props.disabled?.()) return;
    setQuery("");
    setOpen(true);
    queueMicrotask(() => searchRef?.focus());
  };

  const pick = (model: string) => {
    props.onChange(model);
    setOpen(false);
  };

  return (
    <>
      <button
        type="button"
        ref={triggerRef}
        onClick={openPicker}
        disabled={props.disabled?.()}
        class="flex w-full items-center justify-between gap-2 rounded-md border border-border-subtle p-2 text-left text-sm text-ink focus:border-accent focus:outline-none focus:ring-1 focus:ring-accent"
        classList={{
          "bg-surface-2 text-ink-muted pointer-events-none": props.disabled?.() ?? false,
          "bg-surface-0": !(props.disabled?.() ?? false),
          ...(props.triggerClassList?.() ?? {}),
        }}
      >
        <span class="truncate">{selectedLabel(props.value(), props.groups())}</span>
        <Icon name="chevron-down" class="h-3 w-3 shrink-0 text-ink-faint" />
      </button>

      <Popover
        open={open()}
        onClose={() => setOpen(false)}
        triggerRef={() => triggerRef}
        class="w-72 rounded-md border border-border-subtle bg-surface-0 shadow-lg"
      >
        <div class="border-b border-border-subtle p-2">
          <input
            ref={searchRef}
            type="text"
            value={query()}
            onInput={(e) => setQuery(e.currentTarget.value)}
            placeholder={"Search models…"}
            class="w-full rounded border border-border-subtle bg-surface-0 px-2 py-1 text-sm text-ink placeholder:text-ink-muted focus:border-accent focus:outline-none"
          />
        </div>
        <div class="max-h-64 overflow-y-auto py-1">
          <Show
            when={filteredGroups().length > 0}
            fallback={<p class="px-3 py-2 text-xs text-ink-faint">{"No models match your search."}</p>}
          >
            <For each={filteredGroups()}>
              {(group) => (
                <div>
                  <div class="flex items-center gap-2 px-3 pb-0.5 pt-2">
                    <span class="text-[10px] font-medium uppercase tracking-wide text-ink-faint">
                      {group.providerName}
                    </span>
                    {/* "Experimental" marks a third-party provider reached
                        through the models.dev catalog. A local model is not
                        that — the group name already says what it is. */}
                    <Show when={group.providerId !== "claudinio" && group.providerId !== "local"}>
                      <span class="rounded border border-amber-500/40 bg-amber-500/10 px-1 py-px text-[9px] text-amber-600">
                        {"Experimental"}
                      </span>
                    </Show>
                  </div>
                  <For each={group.models}>
                    {(m) => (
                      <button
                        type="button"
                        onClick={() => pick(m)}
                        class="flex w-full items-center justify-between gap-2 px-3 py-1.5 text-left text-sm hover:bg-surface-2"
                        classList={{
                          "text-accent": props.value() === m,
                          "text-ink": props.value() !== m,
                        }}
                      >
                        <span class="truncate">{displayName(m, group)}</span>
                        <Show when={props.value() === m}>
                          <Icon name="check" class="h-3 w-3 shrink-0" />
                        </Show>
                      </button>
                    )}
                  </For>
                </div>
              )}
            </For>
          </Show>
        </div>
      </Popover>
    </>
  );
};
