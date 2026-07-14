import { For, Show, type Component } from "solid-js";
import { Popover } from "./Popover";
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
  return (
    <Popover
      open={true}
      onClose={props.onClose}
      position={{ top: props.y, left: props.x, width: 0, height: 0 }}
      anchorPoint={{ x: 0, y: 0 }}
      originPoint={{ x: 0, y: 0 }}
      class="min-w-[200px] overflow-hidden rounded-lg border border-border-subtle bg-surface-1 py-1 shadow-lg"
    >
      <div onClick={(e) => e.stopPropagation()}>
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
    </Popover>
  );
};
