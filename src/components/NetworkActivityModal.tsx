import { createSignal, For, Show, onMount, onCleanup, type Component } from "solid-js";
import { NET_SOURCE_NAME } from "./NetworkIndicator";
import { Icon } from "./Icon";
import { getNetworkLog, type LogEntry } from "../lib/ipc";

const statusDotClass = (code?: number): string => {
  const base = "w-2 h-2 rounded-full mt-1.5 shrink-0";
  if (code == null) return `${base} bg-gray-400`;
  if (code >= 200 && code < 300) return `${base} bg-emerald-500`;
  if (code >= 300 && code < 500) return `${base} bg-amber-500`;
  return `${base} bg-red-500`;
};

const statusBadgeClass = (code?: number): string => {
  const base = "text-[10px] font-semibold px-1 py-px rounded";
  if (code == null) return "";
  if (code >= 200 && code < 300) return `${base} bg-emerald-500/15 text-emerald-400`;
  if (code >= 300 && code < 500) return `${base} bg-amber-500/15 text-amber-400`;
  return `${base} bg-red-500/15 text-red-400`;
};

function formatBytes(bytes: number): string {
  if (bytes < 1024) return `${bytes} B`;
  if (bytes < 1024 * 1024) return `${(bytes / 1024).toFixed(1)} KB`;
  return `${(bytes / (1024 * 1024)).toFixed(1)} MB`;
}

function formatDuration(ms: number): string {
  const s = Math.floor(ms / 1000);
  if (s < 60) return `${s}s`;
  return `${Math.floor(s / 60)}m${s % 60}s`;
}

const NetworkActivityModal: Component<{
  workspace: string;
  onClose: () => void;
}> = (props) => {
  const [entries, setEntries] = createSignal<LogEntry[]>([]);
  const [loading, setLoading] = createSignal(true);

  onMount(() => {
    getNetworkLog(props.workspace).then((data) => {
      setEntries(data);
      setLoading(false);
    }).catch(() => {
      setLoading(false);
    });

    const onKey = (e: KeyboardEvent) => {
      if (e.key === "Escape") props.onClose();
    };
    window.addEventListener("keydown", onKey);
    onCleanup(() => window.removeEventListener("keydown", onKey));
  });

  return (
    <div
      class="fixed inset-0 z-50 flex items-center justify-center bg-black/40"
      onClick={(e) => {
        if (e.target === e.currentTarget) props.onClose();
      }}
    >
      <div class="flex w-[640px] max-h-[500px] flex-col rounded-xl bg-surface-0 shadow-2xl">
        {/* Header */}
        <div class="flex items-center justify-between border-b border-border-subtle px-5 py-3 shrink-0">
          <span class="font-semibold text-ink">{"Network Log"}</span>
          <button onClick={props.onClose} class="rounded-md p-1 hover:bg-surface-2">
            <Icon name="x" class="h-4 w-4 text-ink-faint" />
          </button>
        </div>

        {/* Body — scrollable */}
        <div class="flex-1 min-h-0 overflow-y-auto p-4">
          <Show
            when={!loading()}
            fallback={<p class="text-sm text-ink-muted">{"Loading..."}</p>}
          >
            <Show
              when={entries().length > 0}
              fallback={<p class="text-sm text-ink-muted">{"No requests logged for this workspace."}</p>}
            >
              <div class="space-y-0">
                <For each={entries()}>
                  {(entry) => (
                    <div class="flex items-start gap-3 border-l-2 border-border-subtle py-2 pl-3">
                      <div class={statusDotClass(entry.statusCode)} />
                      <div class="flex-1 min-w-0">
                        <div class="flex items-center gap-2">
                          <span class="text-xs font-medium text-ink">
                            {NET_SOURCE_NAME[entry.source] ?? entry.source}
                          </span>
                          <Show when={entry.statusCode}>
                            <span class={statusBadgeClass(entry.statusCode)}>
                              {entry.statusCode}
                            </span>
                          </Show>
                        </div>
                        <Show when={entry.detail}>
                          <p class="text-[11px] text-ink-muted truncate">{entry.detail}</p>
                        </Show>
                        <div class="mt-0.5 flex gap-3 text-[10px] text-ink-faint">
                          <span>{formatDuration(entry.durationMs)}</span>
                          <Show when={entry.bytes > 0}>
                            <span>{formatBytes(entry.bytes)}</span>
                          </Show>
                        </div>
                      </div>
                    </div>
                  )}
                </For>
              </div>
            </Show>
          </Show>
        </div>
      </div>
    </div>
  );
};

export default NetworkActivityModal;
