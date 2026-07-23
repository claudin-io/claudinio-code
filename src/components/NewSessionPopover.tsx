import type { Component } from "solid-js";
import { Icon } from "./Icon";

interface NewSessionPopoverProps {
  onConfirm: () => void;
  onClose: () => void;
}

export const NewSessionPopover: Component<NewSessionPopoverProps> = (props) => {
  return (
    <div class="w-[320px] rounded-lg border border-amber-500/30 bg-surface-1 shadow-lg">
      <div class="flex gap-3 p-4">
        <Icon name="alert-triangle-filled" class="h-8 w-8 shrink-0 text-amber-500" />
        <p class="text-[13px] leading-relaxed text-ink-muted">
          {"Starting a new session will stop the current one. You can resume it later from History."}
        </p>
      </div>
      <div class="flex justify-end gap-2 px-4 pb-4">
        <button
          onClick={props.onClose}
          class="rounded-md px-3 py-1.5 text-[12px] font-medium text-ink-muted hover:bg-surface-2"
        >
          {"Go back"}
        </button>
        <button
          onClick={props.onConfirm}
          class="rounded-md bg-accent px-3 py-1.5 text-[12px] font-medium text-white hover:bg-accent-hover"
        >
          {"Create new"}
        </button>
      </div>
    </div>
  );
};
