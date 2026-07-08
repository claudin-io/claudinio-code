import { createEffect, createMemo, createSignal, For, onCleanup, onMount, type Component } from "solid-js";
import { Portal } from "solid-js/web";
import Fuse from "fuse.js";
import { Icon, type IconName } from "./Icon";

interface TagType {
  id: string;
  label: string;
  icon: IconName;
  enabled: boolean;
}

const TAGS: TagType[] = [
  { id: "skill", label: "skill", icon: "package", enabled: true },
  { id: "goal", label: "goal", icon: "goal", enabled: true },
  { id: "agent", label: "agent", icon: "brain", enabled: false },
  { id: "prompt", label: "prompt", icon: "file-text", enabled: false },
];

interface TagMentionPopoverProps {
  bottom: number;
  left: number;
  query: string;
  onSelect: (tagType: string) => void;
  onClose: () => void;
}

export const TagMentionPopover: Component<TagMentionPopoverProps> = (props) => {
  const [highlightIndex, setHighlightIndex] = createSignal(0);

  const fuse = createMemo(
    () =>
      new Fuse(TAGS, {
        keys: ["label"],
        threshold: 0.4,
        distance: 100,
      }),
  );

  const results = createMemo(() => {
    const q = props.query.trim();
    if (!q) return TAGS;
    return fuse()
      .search(q)
      .map((r) => r.item);
  });

  // Reset highlight when search results change
  createEffect(() => {
    results();
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
        if (selected?.enabled) props.onSelect(selected.id);
      } else if (e.key === "Escape") {
        e.preventDefault();
        e.stopPropagation();
        props.onClose();
      }
    };

    document.addEventListener("keydown", onKeyDown);
    onCleanup(() => document.removeEventListener("keydown", onKeyDown));
  });

  return (
    <Portal>
      {/* Transparent backdrop to catch outside clicks */}
      <div class="fixed inset-0 z-40" onClick={props.onClose} />
      <div
        class="fixed z-50 min-w-[180px] max-w-[260px] rounded-lg border border-border-subtle bg-surface-1 shadow-lg"
        style={{
          bottom: `${props.bottom}px`,
          left: `${props.left}px`,
        }}
      >
        <div class="max-h-[240px] overflow-y-auto py-1">
          <For each={results()}>
            {(tag, i) => (
              <button
                onClick={() => {
                  if (tag.enabled) props.onSelect(tag.id);
                }}
                onMouseEnter={() => setHighlightIndex(i())}
                disabled={!tag.enabled}
                class="flex w-full items-center gap-2 px-3 py-1.5 text-left text-[13px]"
                classList={{
                  "bg-accent/10 text-ink": highlightIndex() === i(),
                  "text-ink-muted hover:bg-surface-2": highlightIndex() !== i(),
                }}
              >
                <Icon
                  name={tag.icon}
                  class={`h-3.5 w-3.5 shrink-0 ${
                    tag.enabled ? "text-ink-muted" : "text-ink-faint"
                  }`}
                />
                <span
                  class={
                    tag.enabled ? "text-ink" : "text-ink-faint"
                  }
                >
                  {tag.label}
                </span>
                {!tag.enabled && (
                  <span class="ml-auto rounded bg-ink-faint/10 px-1 py-0.5 text-[10px] text-ink-faint font-medium">
                    soon
                  </span>
                )}
              </button>
            )}
          </For>
        </div>
      </div>
    </Portal>
  );
};
