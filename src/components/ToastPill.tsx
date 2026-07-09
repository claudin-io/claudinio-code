import { createEffect, onCleanup } from "solid-js";

interface ToastPillProps {
  message: string | null;
  onDismiss: () => void;
}

export function ToastPill(props: ToastPillProps) {
  let timeoutId: ReturnType<typeof setTimeout> | undefined;

  createEffect(() => {
    clearTimeout(timeoutId);
    if (props.message) {
      timeoutId = setTimeout(() => {
        props.onDismiss();
      }, 2000);
    }
  });

  onCleanup(() => {
    clearTimeout(timeoutId);
  });

  return (
    <div
      class={"fixed bottom-24 left-1/2 -translate-x-1/2 z-50 transition-all duration-300 pointer-events-none " +
        (props.message
          ? "opacity-100 translate-y-0"
          : "opacity-0 translate-y-2")}
    >
      <div class="bg-surface-2 border border-accent/30 rounded-lg px-3 py-1.5 text-xs text-ink shadow-lg">
        {props.message || ""}
      </div>
    </div>
  );
}
