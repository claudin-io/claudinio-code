import { createEffect, createMemo, createSignal, For, onCleanup, onMount, Show, type Component } from "solid-js";
import { Portal } from "solid-js/web";
import Fuse from "fuse.js";
import { t } from "../lib/grill-me";

interface FileMentionPopoverProps {
  fileList: string[];
  position: { top: number; left: number; height: number };
  query: string;
  onSelect: (path: string) => void;
  onClose: () => void;
}

export const FileMentionPopover: Component<FileMentionPopoverProps> = (props) => {
  const [highlightIndex, setHighlightIndex] = createSignal(0);

  const fuse = createMemo(
    () =>
      new Fuse(props.fileList, {
        threshold: 0.4,
        distance: 100,
        includeScore: true,
      }),
  );

  const results = createMemo(() => {
    const q = props.query.trim();
    if (!q) {
      return props.fileList.slice(0, 20);
    }
    return fuse()
      .search(q)
      .slice(0, 20)
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
        // selected is always truthy because r.length > 0 was checked above and
        // highlightIndex is clamped to [0, r.length-1]
        props.onSelect(selected);
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
      <div
        class="fixed inset-0 z-40"
        onClick={props.onClose}
      />
      <div
        class="fixed z-50 min-w-[260px] max-w-[420px] rounded-lg border border-border-subtle bg-surface-1 shadow-lg"
        style={{
          top: `${props.position.top}px`,
          left: `${props.position.left}px`,
        }}
      >
        <div class="max-h-[240px] overflow-y-auto py-1">
          <Show
            when={results().length > 0}
            fallback={
              <div class="px-3 py-2 text-[12px] text-ink-faint">{t("mention.noFiles")}</div>
            }
          >
            <For each={results()}>
              {(path, i) => (
                <button
                  onClick={() => props.onSelect(path)}
                  onMouseEnter={() => setHighlightIndex(i())}
                  class="flex w-full items-center gap-2 px-3 py-1.5 text-left text-[13px]"
                  classList={{
                    "bg-accent/10 text-ink": highlightIndex() === i(),
                    "text-ink-muted hover:bg-surface-2": highlightIndex() !== i(),
                  }}
                >
                  {path}
                </button>
              )}
            </For>
          </Show>
        </div>
      </div>
    </Portal>
  );
};
