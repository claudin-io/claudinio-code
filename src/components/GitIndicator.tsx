import { createSignal, createMemo, createEffect, onCleanup, Show, type Component } from "solid-js";
import { gitStatus, checkGitAvailable, type GitStatus } from "../lib/ipc";
import { Icon } from "./Icon";
import { t } from "../lib/grill-me";

export const GitIndicator: Component<{
  workspace: string;
  onShowChanges: () => void;
}> = (props) => {
  const [status, setStatus] = createSignal<GitStatus | null>(null);
  const [loading, setLoading] = createSignal(true);
  const [gitAvailable, setGitAvailable] = createSignal<boolean | null>(null);

  // Guards against overlapping invokes: if a git call is slow (large repo,
  // antivirus scanning on Windows, etc.), the next poll tick skips instead of
  // queuing another invoke on top of the one still in flight.
  let statusInFlight = false;

  const refreshStatus = async () => {
    if (statusInFlight) return;
    statusInFlight = true;
    try {
      const s = await gitStatus(props.workspace);
      setStatus(s);
    } catch (e) {
      console.warn("[GitIndicator] gitStatus failed:", e);
      setStatus(null);
    } finally {
      setLoading(false);
      statusInFlight = false;
    }
  };

  // Check git availability once on mount
  checkGitAvailable().then(setGitAvailable);

  // Start polling only when git is confirmed available; cleanup on unmount
  createEffect(() => {
    if (gitAvailable() !== true) return;

    refreshStatus();

    const intervalId = setInterval(refreshStatus, 10000);

    onCleanup(() => {
      clearInterval(intervalId);
    });
  });

  const s = status;
  const loading_ = loading;

  const hasChanges = createMemo(() => {
    const v = s();
    return v !== null && v.hasChanges && v.files.length > 0;
  });

  const fileCount = createMemo(() => s()?.files.length ?? 0);
  const additions = createMemo(() => s()?.totalAdditions ?? 0);
  const deletions = createMemo(() => s()?.totalDeletions ?? 0);

  const label = createMemo(() => {
    if (loading_()) return "…";
    if (hasChanges()) {
      return t("git.changes", String(fileCount()), String(additions()), String(deletions()));
    }
    return t("git.noChanges");
  });

  const tooltip = createMemo(() => {
    const fc = fileCount();
    return fc > 0 ? t("git.filesChanged", String(fc)) : t("git.noChanges");
  });

  const btnClass = createMemo(() => {
    const base = "flex items-center gap-1 rounded px-2 py-1 text-[11px] hover:bg-surface-2";
    if (loading_()) return `${base} text-ink-faint opacity-30`;
    if (hasChanges()) return `${base} text-ink-muted`;
    return `${base} text-ink-faint opacity-50`;
  });

  return (
    <Show when={gitAvailable() === true}>
      <button
        onClick={props.onShowChanges}
        title={tooltip()}
        class={btnClass()}
      >
        <Icon name="diff" class="h-3.5 w-3.5" />
        <span>{label()}</span>
      </button>
    </Show>
  );
};
