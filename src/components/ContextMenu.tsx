import { For, Show, onCleanup, onMount, type Component } from "solid-js";
import { Portal } from "solid-js/web";
import { Icon, type IconName } from "./Icon";

export interface ContextMenuItem {
  label: string;
  icon: IconName;
  action: () => void;
  separatorAfter?: boolean;
}

export const ContextMenu: Component<{
  x: number;
  y: number;
  items: ContextMenuItem[];
  onClose: () => void;
}> = (props) => {
  // Close on Escape key
  const onKeyDown = (e: KeyboardEvent) => {
    if (e.key === "Escape") props.onClose();
  };
  onMount(() => document.addEventListener("keydown", onKeyDown));
  onCleanup(() => document.removeEventListener("keydown", onKeyDown));

  // Clamp position to stay within viewport
  const clampX = () => Math.min(props.x, window.innerWidth - 200);
  const clampY = () => Math.min(props.y, window.innerHeight - 40 * props.items.length);

  return (
    <Portal>
      {/* Backdrop — catches outside clicks */}
      <div
        class="fixed inset-0 z-50"
        onClick={props.onClose}
        onContextMenu={(e) => { e.preventDefault(); props.onClose(); }}
      />
      {/* Menu panel */}
      <div
        class="fixed z-50 min-w-[200px] overflow-hidden rounded-lg border border-border-subtle bg-surface-1 py-1 shadow-lg"
        style={{
          left: `${clampX()}px`,
          top: `${clampY()}px`,
        }}
        onClick={(e) => e.stopPropagation()}
      >
        <For each={props.items}>
          {(item, i) => (
            <>
              <button
                class="flex w-full items-center gap-2.5 px-3 py-1.5 text-left text-sm text-ink hover:bg-surface-2"
                onClick={() => {
                  item.action();
                  props.onClose();
                }}
              >
                <Icon name={item.icon} class="h-4 w-4 shrink-0 text-ink-muted" />
                <span class="truncate">{item.label}</span>
              </button>
              <Show when={item.separatorAfter && i() < props.items.length - 1}>
                <div class="mx-2 border-t border-border-subtle" />
              </Show>
            </>
          )}
        </For>
      </div>
    </Portal>
  );
};
