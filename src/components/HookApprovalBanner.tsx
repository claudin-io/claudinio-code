import { Component, createSignal, For, Show } from "solid-js";
import { approveHooks, type HooksAwaitingApprovalData } from "../lib/ipc";
import { Icon } from "./Icon";

/**
 * This workspace declares hooks nobody has approved, so none of them ran.
 *
 * Shown in the thread rather than only in Settings, because the moment this
 * matters is the moment the user notices their hook did nothing — and a
 * silently inert hook is indistinguishable from a working one with nothing to
 * say. The commands are listed **after** expansion: approving
 * `${CLAUDE_PLUGIN_ROOT}/hooks/run.sh` without seeing where that resolves to is
 * not consent.
 */
const HookApprovalBanner: Component<{
  data: HooksAwaitingApprovalData;
  onApproved: () => void;
  onDismiss: () => void;
}> = (props) => {
  const [busy, setBusy] = createSignal(false);
  const [error, setError] = createSignal<string | null>(null);

  const approve = () => {
    setBusy(true);
    void (async () => {
      try {
        await approveHooks(props.data.workspace, props.data.hash);
        props.onApproved();
      } catch (e) {
        setError(String(e));
      } finally {
        setBusy(false);
      }
    })();
  };

  return (
    <div class="mx-4 my-2 rounded-md border border-amber-500/40 bg-amber-500/10 p-3">
      <div class="flex items-start gap-2">
        <Icon name="alert-triangle" class="mt-0.5 h-4 w-4 shrink-0 text-amber-500" />
        <div class="min-w-0 flex-1">
          <div class="text-xs font-medium text-amber-500">
            {`This project declares ${String(props.data.count)} lifecycle hook${props.data.count === 1 ? "" : "s"}. ${props.data.count === 1 ? "It has" : "They have"} not run.`}
          </div>
          <p class="mt-1 text-[11px] text-ink-muted">
            {"A hook is a program this repository asks Claudinio to run at fixed points in a session. Nothing runs until you have read what it is."}
          </p>
          <div class="mt-2 flex flex-col gap-0.5">
            <For each={props.data.commands}>
              {(cmd) => (
                <code class="break-all font-mono text-[10px] text-ink">{cmd}</code>
              )}
            </For>
          </div>
          <Show when={error()}>
            <p class="mt-2 text-[11px] text-red-500">{error()}</p>
          </Show>
          <div class="mt-2 flex gap-2">
            <button
              type="button"
              disabled={busy()}
              onClick={approve}
              class="rounded-md bg-accent px-2 py-1 text-[11px] font-medium text-white disabled:opacity-50"
            >
              {"Allow these hooks"}
            </button>
            <button
              type="button"
              onClick={props.onDismiss}
              class="rounded-md border border-border-subtle px-2 py-1 text-[11px] text-ink"
            >
              {"Not now"}
            </button>
          </div>
        </div>
      </div>
    </div>
  );
};

export default HookApprovalBanner;
