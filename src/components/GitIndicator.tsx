import { createSignal, createMemo, Show, type Component } from "solid-js";
import { gitStatus, checkGitAvailable, type GitStatus } from "../lib/ipc";
import { Icon } from "./Icon";
import { createVisibilityAwareInterval } from "../lib/visibility";

export const GitIndicator: Component<{
  workspace: string;
  /** Only the active (visible) workspace panel polls git — inactive panels
      stay mounted but must not keep spawning git processes. */
  active: boolean;
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

  // Poll only while git is confirmed available, this panel is the active
  // workspace, and the window is visible. Pausing in background stops the
  // constant git.exe spawning that showed up as CPU churn on Windows.
  createVisibilityAwareInterval(
    refreshStatus,
    // 30s: each poll walks the whole working tree; on network-drive
    // workspaces (SMB) that was a steady stream of remote I/O.
    30000,
    () => gitAvailable() === true && props.active,
  );

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
      return `${String(fileCount())} changes (+${String(additions())} −${String(deletions())})`;
    }
    return "0 changes";
  });

  const tooltip = createMemo(() => {
    const fc = fileCount();
    return fc > 0 ? `${String(fc)} files changed` : "0 changes";
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
