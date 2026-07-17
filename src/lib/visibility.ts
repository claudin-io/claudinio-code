import { createEffect, createSignal, onCleanup } from "solid-js";

// Global window-visibility signal. Polling loops (git status, tasks) subscribe
// to this so they stop spawning work while the app is minimized/hidden —
// background git.exe churn on Windows was traced to intervals that never
// paused (see docs/plans 2026-07-17 CPU study).
const [windowVisible, setWindowVisible] = createSignal(
  typeof document === "undefined" ? true : !document.hidden,
);

if (typeof document !== "undefined") {
  document.addEventListener("visibilitychange", () => {
    setWindowVisible(!document.hidden);
  });
}

export const isWindowVisible = windowVisible;

/**
 * setInterval that only fires while the window is visible (and, optionally,
 * while `enabled()` is true). On becoming visible again it fires immediately
 * so stale UI refreshes right away instead of waiting a full period.
 *
 * Must be called inside a reactive owner (component/createEffect) — cleanup
 * is registered with onCleanup.
 */
export function createVisibilityAwareInterval(
  fn: () => void,
  ms: number,
  enabled: () => boolean = () => true,
) {
  let intervalId: ReturnType<typeof setInterval> | null = null;

  const stop = () => {
    if (intervalId !== null) {
      clearInterval(intervalId);
      intervalId = null;
    }
  };

  const sync = () => {
    const shouldRun = windowVisible() && enabled();
    if (shouldRun && intervalId === null) {
      // Immediate tick on (re)start so the UI catches up after being hidden.
      fn();
      intervalId = setInterval(fn, ms);
    } else if (!shouldRun) {
      stop();
    }
  };

  // Track both signals reactively.
  createEffect(sync);
  onCleanup(stop);
}
