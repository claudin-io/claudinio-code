import { createSignal, createMemo, For, Show, type Component } from "solid-js";
import { Icon } from "./Icon";
import { t } from "../lib/grill-me";
import {
  activeOps,
  lastFinishedOp,
  formatBytes,
  formatDuration,
  type NetOp,
} from "../lib/networkActivity";

/** Localized name + explanation for one network operation's source. */
const sourceName = (op: NetOp) => t(`net.source.${op.source}`);
const sourceWhy = (op: NetOp) => t(`net.why.${op.source}`);

/**
 * Status-bar globe that lights up whenever the backend has an open network
 * connection. Hovering shows which subsystem is using the network and why —
 * so a forgotten agent run streaming in the background is visible at a glance.
 */
export const NetworkIndicator: Component = () => {
  const [hovered, setHovered] = createSignal(false);
  const ops = activeOps;
  const active = createMemo(() => ops().length > 0);

  return (
    <div
      class="relative"
      onMouseEnter={() => setHovered(true)}
      onMouseLeave={() => setHovered(false)}
    >
      <button
        class="flex items-center gap-1 rounded px-2 py-1 text-[11px] hover:bg-surface-2"
        classList={{
          "text-accent": active(),
          "text-ink-faint": !active(),
        }}
        aria-label={t("net.title")}
      >
        <Icon name="globe" class={"h-3.5 w-3.5" + (active() ? " animate-pulse" : "")} />
        <Show when={active()}>
          <span>{ops().length}</span>
        </Show>
      </button>

      {/* Hover tooltip: one row per active connection (origin + why). */}
      <Show when={hovered()}>
        <div class="absolute right-0 top-full z-50 mt-1 w-80 rounded-lg border border-border-subtle bg-surface-1 p-3 shadow-modal">
          <p class="mb-2 text-[10px] font-semibold uppercase tracking-wider text-ink-faint">
            {t("net.title")}
          </p>
          <Show
            when={active()}
            fallback={
              <div>
                <p class="text-[12px] text-ink-muted">{t("net.idle")}</p>
                <Show when={lastFinishedOp()}>
                  {(op) => (
                    <p class="mt-1 text-[11px] text-ink-faint">
                      {t("net.lastOp", sourceName(op()))}
                      <Show when={op().detail}> — {op().detail}</Show>
                    </p>
                  )}
                </Show>
              </div>
            }
          >
            <div class="space-y-2">
              <For each={ops()}>
                {(op) => (
                  <div class="border-l-2 border-accent/40 pl-2">
                    <p class="text-[12px] font-medium text-ink">
                      {sourceName(op)}
                      <span class="ml-1 font-normal text-ink-faint">
                        {formatDuration(op.elapsedMs)}
                        <Show when={op.bytes > 0}> · {formatBytes(op.bytes)}</Show>
                      </span>
                    </p>
                    <Show when={op.detail}>
                      <p class="break-words text-[11px] text-ink-muted">{op.detail}</p>
                    </Show>
                    <p class="text-[11px] leading-snug text-ink-faint">{sourceWhy(op)}</p>
                  </div>
                )}
              </For>
            </div>
          </Show>
        </div>
      </Show>
    </div>
  );
};
