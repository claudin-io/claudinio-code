import { onCleanup, onMount, type Component } from "solid-js";
import { Portal } from "solid-js/web";
import { t } from "../lib/grill-me";
import { Icon } from "./Icon";

interface NewSessionPopoverProps {
  position: { top: number; left: number; height: number };
  onConfirm: () => void;
  onClose: () => void;
}

export const NewSessionPopover: Component<NewSessionPopoverProps> = (props) => {
  onMount(() => {
    const onKeyDown = (e: KeyboardEvent) => {
      if (e.key === "Escape") {
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
      {/* Backdrop — catches outside clicks */}
      <div
        class="fixed inset-0 z-40"
        onClick={props.onClose}
      />
      <div
        class="fixed z-50 w-[320px] rounded-lg border border-amber-500/30 bg-surface-1 shadow-lg"
        style={{
          top: `${props.position.top + props.position.height + 4}px`,
          left: `${props.position.left}px`,
        }}
      >
        <div class="flex gap-3 p-4">
          <Icon name="alert-triangle-filled" class="h-8 w-8 shrink-0 text-amber-500" />
          <p class="text-[13px] leading-relaxed text-ink-muted">
            {t("chat.header.newPopover.body")}
          </p>
        </div>
        <div class="flex justify-end gap-2 px-4 pb-4">
          <button
            onClick={props.onClose}
            class="rounded-md px-3 py-1.5 text-[12px] font-medium text-ink-muted hover:bg-surface-2"
          >
            {t("chat.header.newPopover.goBack")}
          </button>
          <button
            onClick={props.onConfirm}
            class="rounded-md bg-accent px-3 py-1.5 text-[12px] font-medium text-white hover:bg-accent-hover"
          >
            {t("chat.header.newPopover.create")}
          </button>
        </div>
      </div>
    </Portal>
  );
};
