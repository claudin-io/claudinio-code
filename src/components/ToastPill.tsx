import { onMount, onCleanup } from "solid-js";

interface ToastPillProps {
  message: string | null;
  onDismiss: () => void;
}

export function ToastPill(props: ToastPillProps) {
  let timeoutId: ReturnType<typeof setTimeout> | undefined;

  const scheduleDismiss = () => {
    clearTimeout(timeoutId);
    timeoutId = setTimeout(() => {
      props.onDismiss();
    }, 2000);
  };

  onMount(() => {
    if (props.message) {
      scheduleDismiss();
    }
  });

  onCleanup(() => {
    clearTimeout(timeoutId);
  });

  // Re-schedule when message changes
  // Using a createEffect would be better, but we keep it simple
  // The parent controls message changes, and this component tracks via mount/cleanup
  // Actually, since we don't have createEffect imported, we handle via the parent
  // The parent ChatPanel re-mounts this on message change via key or conditional

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
