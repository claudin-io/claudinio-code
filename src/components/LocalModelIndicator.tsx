import { createSignal, onCleanup, onMount, Show, For, type Component } from "solid-js";
import { localRuntimeStats, type LocalModelStats, type LocalPhase } from "../lib/ipc";
import { ENGINE_LABEL } from "../lib/localEngines";
import { Icon } from "./Icon";

/** Slow on purpose: this is a status line, not telemetry. Each poll is three
 *  loopback requests per resident model, and nothing here changes in a way a
 *  human reads faster than this. */
const POLL_MS = 4000;

/// What each phase is called on screen. `loading` and `readingPrompt` are the
/// ones worth naming: both produce no tokens, and without a label a minute of
/// silence is indistinguishable from a hang.
const PHASE_LABEL: Record<LocalPhase, string> = {
  loading: "loading model",
  readingPrompt: "reading prompt",
  generating: "generating",
  idle: "idle",
  sleeping: "unloaded",
};

function mem(bytes: number): string {
  if (bytes >= 1_000_000_000) return `${(bytes / 1_000_000_000).toFixed(1)}GB`;
  return `${Math.round(bytes / 1_000_000)}MB`;
}

/**
 * Shows what a local model is costing while it is loaded, and nothing at all
 * when none is — a status bar item that is always present but usually says
 * "0" is just noise.
 */
export const LocalModelIndicator: Component<{ visible?: () => boolean }> = (props) => {
  const [stats, setStats] = createSignal<LocalModelStats[]>([]);
  const [open, setOpen] = createSignal(false);
  let timer: ReturnType<typeof setInterval> | undefined;

  const poll = async () => {
    // Polling a hidden window burns battery for pixels nobody is looking at —
    // the lesson already recorded for the index and network indicators.
    if (props.visible && !props.visible()) return;
    try {
      setStats(await localRuntimeStats());
    } catch {
      setStats([]);
    }
  };

  onMount(() => {
    void poll();
    timer = setInterval(() => void poll(), POLL_MS);
    onCleanup(() => clearInterval(timer));
  });

  const primary = () => stats()[0];

  return (
    <Show when={primary()}>
      <button
        type="button"
        onClick={() => setOpen((v) => !v)}
        class="flex items-center gap-1 rounded px-1.5 py-1 font-mono text-[11px] text-ink-faint hover:bg-surface-2"
        title="Local model"
      >
        <Icon name="monitor" class={`h-3.5 w-3.5 ${primary()!.busy ? "text-accent" : ""}`} />
        <span classList={{ "text-accent": primary()!.busy }}>
          {/* While nothing is coming out, say which kind of nothing it is. */}
          {primary()!.phase === "loading" || primary()!.phase === "readingPrompt"
            ? PHASE_LABEL[primary()!.phase]
            : primary()!.phase === "sleeping"
              ? "unloaded"
              : `${primary()!.tokensPerSecond.toFixed(1)} tok/s`}
        </span>
      </button>

      <Show when={open()}>
        <div class="absolute right-2 top-10 z-50 w-72 rounded-md border border-border-subtle bg-surface-0 p-3 shadow-lg">
          <For each={stats()}>
            {(s) => (
              <div class="space-y-1">
                <div class="truncate text-sm text-ink">{s.displayName}</div>
                <div class="text-[11px] text-ink-faint">
                  {ENGINE_LABEL[s.engine]} · {PHASE_LABEL[s.phase]}
                </div>
                <dl class="mt-1 space-y-0.5 text-[11px]">
                  <div class="flex justify-between">
                    <dt class="text-ink-faint">Memory</dt>
                    <dd class="font-mono text-ink">{mem(s.memoryBytes)}</dd>
                  </div>
                  <div class="flex justify-between">
                    <dt class="text-ink-faint">Context</dt>
                    <dd class="font-mono text-ink">
                      {s.ctxUsed.toLocaleString()} / {s.ctxSize.toLocaleString()}
                    </dd>
                  </div>
                  <div class="flex justify-between">
                    <dt class="text-ink-faint">Generation</dt>
                    <dd class="font-mono text-ink">{s.tokensPerSecond.toFixed(1)} tok/s</dd>
                  </div>
                  <div class="flex justify-between">
                    <dt class="text-ink-faint">Prompt</dt>
                    <dd class="font-mono text-ink">
                      {s.promptTokensPerSecond.toFixed(1)} tok/s
                    </dd>
                  </div>
                  <div class="flex justify-between">
                    <dt class="text-ink-faint">Generated</dt>
                    <dd class="font-mono text-ink">{s.tokensGenerated.toLocaleString()}</dd>
                  </div>
                </dl>
              </div>
            )}
          </For>
        </div>
      </Show>
    </Show>
  );
};
