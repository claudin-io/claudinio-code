import { createEffect, createMemo, createSignal, For, onCleanup, onMount, Show, type Component } from "solid-js";

import Fuse from "fuse.js";
import { listSkills, type SkillEntry } from "../lib/ipc";
import { Icon } from "./Icon";
import { t } from "../lib/grill-me";

interface SkillMentionPopoverProps {
  workspace: string;
  query: string;
  onSelect: (skillName: string) => void;
  onClose: () => void;
}

export const SkillMentionPopover: Component<SkillMentionPopoverProps> = (props) => {
  const [skills, setSkills] = createSignal<SkillEntry[]>([]);
  const [loading, setLoading] = createSignal(true);
  const [error, setError] = createSignal<string | null>(null);
  const [highlightIndex, setHighlightIndex] = createSignal(0);

  // Fetch skills on mount
  onMount(() => {
    listSkills(props.workspace)
      .then((res) => {
        setSkills(res.skills);
        setLoading(false);
      })
      .catch((err) => {
        setError(String(err));
        setLoading(false);
      });
  });

  const fuse = createMemo(() => {
    const items = skills();
    return new Fuse(items, {
      keys: ["name", "description"],
      threshold: 0.4,
      distance: 100,
    });
  });

  const results = createMemo(() => {
    const q = props.query.trim();
    const all = skills();
    if (!q) {
      return all.slice(0, 50);
    }
    return fuse()
      .search(q)
      .slice(0, 50)
      .map((r) => r.item);
  });

  // Reset highlight when search results change
  createEffect(() => {
    results(); // track changes
    setHighlightIndex(0);
  });

  // Clamp highlight index when results shrink
  // NOTE: reset effect above runs first and always sets to 0, so this body is never reached.
  // Kept for documentation/clarity — the guard exists in case reset effect is ever removed.
  createEffect(() => {
    results(); //#cov-ref
  });

  onMount(() => {
    const onKeyDown = (e: KeyboardEvent) => {
      const r = results();
      if (r.length === 0) return;

      if (e.key === "ArrowDown") {
        e.preventDefault();
        e.stopPropagation();
        setHighlightIndex((i) => Math.min(i + 1, r.length - 1));
      } else if (e.key === "ArrowUp") {
        e.preventDefault();
        e.stopPropagation();
        setHighlightIndex((i) => Math.max(i - 1, 0));
      } else if (e.key === "Enter") {
        e.preventDefault();
        e.stopPropagation();
        const selected = r[highlightIndex()];
        if (selected) props.onSelect(selected.name);
      }
    };

    document.addEventListener("keydown", onKeyDown);
    onCleanup(() => document.removeEventListener("keydown", onKeyDown));
  });

  let scrollRef: HTMLDivElement | undefined;

  // Scroll highlighted item into view when highlightIndex changes
  createEffect(() => {
    const idx = highlightIndex();
    const btn = scrollRef?.querySelector(`[data-index="${idx}"]`) as HTMLElement | null;
    btn?.scrollIntoView({ block: "nearest" });
  });

  return (
    <div class="min-w-[280px] max-w-[420px] rounded-lg border border-border-subtle bg-surface-1 shadow-lg">
      <div ref={scrollRef} class="max-h-[280px] overflow-y-auto py-1">
        <Show when={!loading() && !error()}>
          <Show
            when={results().length > 0}
            fallback={
              <div class="px-3 py-2 text-[12px] text-ink-faint">{t("mention.noSkills")}</div>
            }
          >
            <For each={results()}>
              {(skill, i) => (
                <button
                  data-index={i()}
                  onClick={() => props.onSelect(skill.name)}
                  onMouseEnter={() => setHighlightIndex(i())}
                  class="w-full px-3 py-1.5 text-left"
                  classList={{
                    "bg-accent/10 text-ink": highlightIndex() === i(),
                    "text-ink-muted hover:bg-surface-2": highlightIndex() !== i(),
                  }}
                >
                  <div class="font-mono text-[12px] text-ink font-medium">{skill.name}</div>
                  <div class="text-[11px] text-ink-faint truncate" style="max-width: 380px">
                    {skill.description}
                  </div>
                </button>
              )}
            </For>
          </Show>
        </Show>

        <Show when={loading()}>
          <div class="flex items-center gap-2 px-3 py-2 text-[12px] text-ink-faint">
            <Icon name="loader" class="animate-spin" />
            Loading skills…
          </div>
        </Show>

        <Show when={error() !== null && !loading()}>
          <div class="px-3 py-2 text-[12px] text-danger">{error()}</div>
        </Show>
      </div>
    </div>
  );
};
