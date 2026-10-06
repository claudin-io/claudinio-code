import { createMemo, createSignal, For, onMount, Show, type Accessor, type Component } from "solid-js";
import { getConfig, setConfig, type ToolSurface } from "../../lib/ipc";

const SURFACES: { id: ToolSurface; label: string }[] = [
  { id: "full", label: "Full" },
  { id: "lean", label: "Lean" },
];

interface Props {
  brainModel: Accessor<string>;
  builderModel: Accessor<string>;
}

/**
 * Which tools each of the two models in use is offered. Every tool's schema is
 * sent with every request, so a tool a model never calls is paid for anyway.
 *
 * The choice is per model, not per session: the tool list is part of what the
 * provider caches, and a list that moved between turns would throw the cache
 * away. It applies from the next message.
 */
export const SettingsToolSurface: Component<Props> = (props) => {
  const [surfaces, setSurfaces] = createSignal<Record<string, ToolSurface>>({});
  // The builder model the profile below was computed for — the saved one,
  // which a model picked a moment ago and not yet saved is not.
  const [compactFor, setCompactFor] = createSignal<string | null>(null);

  onMount(async () => {
    const cfg = await getConfig();
    setSurfaces(cfg.toolSurface ?? {});
    setCompactFor(cfg.builderProfile === "compact" ? (cfg.builderModel ?? null) : null);
  });

  // One row per distinct model: the same model in both roles has one surface.
  const rows = createMemo(() => {
    const brain = props.brainModel();
    const builder = props.builderModel();
    if (brain === builder) return [{ model: brain, role: "Brain and Builder" }];
    return [
      { model: brain, role: "Brain" },
      { model: builder, role: "Builder and subagents" },
    ];
  });

  const surfaceOf = (model: string): ToolSurface => surfaces()[model] ?? "full";

  const choose = async (model: string, surface: ToolSurface) => {
    setSurfaces({ ...surfaces(), [model]: surface });
    await setConfig({ toolSurface: { model, surface } });
  };

  return (
    <div class="mt-6 border-t border-border-subtle pt-4" data-tool-surface>
      <span class="text-sm font-medium text-ink">{"Tools offered to each model"}</span>
      <p class="mb-3 mt-1 text-[11px] text-ink-faint">
        {
          "Lean leaves out go_to_definition, find_references and symbol_lookup, and the prompt lines that point to them. Search, outline and every other tool stay. Applies from the next message."
        }
      </p>
      <For each={rows()}>
        {(row) => (
          <div class="mb-2 flex items-center gap-3" data-surface-row={row.model}>
            <div class="min-w-0 flex-1">
              <div class="truncate text-sm text-ink">{row.model}</div>
              <div class="text-[11px] text-ink-faint">{row.role}</div>
            </div>
            <div class="flex gap-2">
              <For each={SURFACES}>
                {(s) => (
                  <button
                    type="button"
                    data-surface={s.id}
                    aria-pressed={surfaceOf(row.model) === s.id}
                    onClick={() => choose(row.model, s.id)}
                    class="rounded-md border px-3 py-1 text-sm transition-colors"
                    classList={{
                      "border-accent bg-accent/10 text-accent": surfaceOf(row.model) === s.id,
                      "border-border-subtle bg-surface-0 text-ink hover:bg-surface-2":
                        surfaceOf(row.model) !== s.id,
                    }}
                  >
                    {s.label}
                  </button>
                )}
              </For>
            </div>
          </div>
        )}
      </For>
      <Show when={compactFor() !== null && compactFor() === props.builderModel()}>
        <p class="mt-2 text-[11px] text-ink-muted" data-compact-note>
          {
            "This Builder model's context window is under 64k tokens, so its Builder sessions run the compact profile whatever is chosen here: a short prompt, ten tools, no subagents, skills or MCP tools, and the session edits files itself."
          }
        </p>
      </Show>
    </div>
  );
};
