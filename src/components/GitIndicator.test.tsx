import { describe, it, expect, vi, afterEach, beforeEach } from "vitest";
import { render } from "solid-js/web";

// ── Module mocks (hoisted before imports) ──────────────────────────

vi.mock("../lib/ipc", () => ({
  gitStatus: vi.fn(),
  checkGitAvailable: vi.fn(),
}));

vi.mock("../lib/grill-me", () => ({
  t: (key: string, ..._args: (string | number)[]) => key,
}));

vi.mock("./Icon", () => ({
  Icon: (props: { name: string; class?: string }) => (
    <span data-testid={`icon-${props.name}`} class={props.class} />
  ),
}));

// ── Imports (after mocks) ──────────────────────────────────────────

import { GitIndicator } from "./GitIndicator";
import { gitStatus, checkGitAvailable } from "../lib/ipc";
import type { Mock } from "vitest";
import type { GitStatus } from "../lib/ipc";

// ── Helpers ─────────────────────────────────────────────────────────

/**
 * Drain the microtask queue so Solid's reactive effects and promise.then()
 * callbacks settle. With vi.useFakeTimers, setTimeout is faked, so the
 * standard flush() pattern (setTimeout 10) doesn't work. Instead we await
 * Promise.resolve() multiple times — Solid 1.x schedules its createEffect
 * first run as a microtask, and our checkGitAvailable.then() / gitStatus()
 * .then() callbacks also fire on microtasks.
 *
 * Three rounds covers: round 1 → checkGitAvailable.then() fires → triggers
 * createEffect → inside effect, refreshStatus/refreshBranch are called →
 * round 2 → gitStatus/gitBranch .then() fires → round 3 → downstream
 * signals settle.
 */
async function drainMicrotasks() {
  await Promise.resolve();
  await Promise.resolve();
  await Promise.resolve();
}

function makeStatus(overrides: Partial<GitStatus> = {}): GitStatus {
  return {
    hasChanges: false,
    files: [],
    totalAdditions: 0,
    totalDeletions: 0,
    ...overrides,
  };
}

const defaultProps = {
  workspace: "/test/workspace",
  active: true,
  onShowChanges: vi.fn(),
};

// ══════════════════════════════════════════════════════════════════════
// GitIndicator tests
// ══════════════════════════════════════════════════════════════════════

describe("GitIndicator", () => {
  beforeEach(() => {
    vi.useFakeTimers();
    (checkGitAvailable as Mock).mockResolvedValue(true);
    (gitStatus as Mock).mockResolvedValue(makeStatus());
  });

  afterEach(() => {
    document.body.innerHTML = "";
    vi.clearAllMocks();
    vi.useRealTimers();
  });

  function mount(props = defaultProps) {
    return render(() => <GitIndicator {...props} />, document.body);
  }

  // ── git not available ───────────────────────────────────────────────

  it("renders nothing when checkGitAvailable resolves to false", async () => {
    (checkGitAvailable as Mock).mockResolvedValue(false);
    mount();
    await drainMicrotasks();

    // gitAvailable = false → <Show when={gitAvailable() === true}> hides everything
    expect(document.body.innerHTML).toBe("");
    expect(document.body.querySelector("button")).toBeNull();
  });

  it("renders nothing while checkGitAvailable is pending (gitAvailable = null)", () => {
    (checkGitAvailable as Mock).mockImplementation(() => new Promise(() => {}));
    mount();

    // gitAvailable starts null → Show condition fails → nothing in DOM
    expect(document.body.innerHTML).toBe("");
  });

  // ── loading state ────────────────────────────────────────────────────

  it("shows loading state with ellipsis before the first git status resolves", async () => {
    // Keep gitStatus pending so loading stays true
    (gitStatus as Mock).mockImplementation(() => new Promise(() => {}));

    mount();
    await drainMicrotasks();

    const btn = document.body.querySelector("button");
    expect(btn).not.toBeNull();
    expect(btn!.textContent).toContain("…");
    // Loading class: opacity-30
    expect(btn!.className).toContain("opacity-30");
    expect(btn!.className).not.toContain("opacity-50");
  });

  // ── shows changes count ──────────────────────────────────────────────

  it("shows changes label when git has changes", async () => {
    (gitStatus as Mock).mockResolvedValue(
      makeStatus({
        hasChanges: true,
        files: [
          { path: "a.ts", status: "M", additions: 3, deletions: 1 },
          { path: "b.ts", status: "A", additions: 10, deletions: 0 },
        ],
        totalAdditions: 13,
        totalDeletions: 1,
      }),
    );

    mount();
    await drainMicrotasks();

    const btn = document.body.querySelector("button")!;
    // t("git.changes", "2", "13", "1") returns the key "git.changes" per our mock
    expect(btn.textContent).toContain("git.changes");
    // Has-changes class: text-ink-muted, no opacity modifier
    expect(btn.className).toContain("text-ink-muted");
    expect(btn.className).not.toContain("opacity-50");
    expect(btn.className).not.toContain("opacity-30");
  });

  it("shows no-changes label when git has no changes", async () => {
    (gitStatus as Mock).mockResolvedValue(makeStatus());

    mount();
    await drainMicrotasks();

    const btn = document.body.querySelector("button")!;
    expect(btn.textContent).toContain("git.noChanges");
    // No-changes class: text-ink-faint opacity-50
    expect(btn.className).toContain("text-ink-faint");
    expect(btn.className).toContain("opacity-50");
  });

  it("shows the diff icon", async () => {
    mount();
    await drainMicrotasks();

    const icon = document.body.querySelector('[data-testid="icon-diff"]');
    expect(icon).not.toBeNull();
    expect(icon!.getAttribute("class")).toContain("h-3.5");
    expect(icon!.getAttribute("class")).toContain("w-3.5");
  });

  // ── click handler ────────────────────────────────────────────────────

  it("calls onShowChanges when the button is clicked", async () => {
    const onShowChanges = vi.fn();
    mount({ ...defaultProps, onShowChanges });
    await drainMicrotasks();

    const btn = document.body.querySelector("button")!;
    btn.click();
    expect(onShowChanges).toHaveBeenCalledTimes(1);
  });

  // ── polling behavior ─────────────────────────────────────────────────

  it("polls gitStatus every 10 seconds", async () => {
    mount();
    await drainMicrotasks();

    // Clear initial call history so we only count poll-driven calls
    vi.clearAllMocks();

    // Advance 10s → gitStatus interval fires once
    vi.advanceTimersByTime(10000);
    // Drain microtasks so the resolved promise's .then() resets statusInFlight
    await drainMicrotasks();
    expect(gitStatus).toHaveBeenCalledTimes(1);

    // Advance another 10s (total 20s) → gitStatus fires again
    vi.advanceTimersByTime(10000);
    await drainMicrotasks();
    expect(gitStatus).toHaveBeenCalledTimes(2);
  });

  it("skips a gitStatus poll tick when the previous request is still in-flight", async () => {
    (gitStatus as Mock).mockImplementation(() => new Promise(() => {}));
    mount();
    await drainMicrotasks();

    // Initial refreshStatus called gitStatus and set statusInFlight = true
    vi.clearAllMocks();

    // Advance 20s — 2 ticks would fire, but the in-flight guard prevents both
    vi.advanceTimersByTime(20000);
    expect(gitStatus).toHaveBeenCalledTimes(0);
  });

  it("does not start polling when git is not available", async () => {
    (checkGitAvailable as Mock).mockResolvedValue(false);
    mount();
    await drainMicrotasks();

    vi.clearAllMocks();

    // Advance 30s — createEffect returned early (gitAvailable !== true),
    // so no intervals were created → no calls should happen
    vi.advanceTimersByTime(30000);
    expect(gitStatus).toHaveBeenCalledTimes(0);
  });

  // ── error handling ───────────────────────────────────────────────────

  it("handles gitStatus rejection gracefully — shows no-changes label", async () => {
    (gitStatus as Mock).mockRejectedValue(new Error("git error"));
    mount();
    await drainMicrotasks();

    const btn = document.body.querySelector("button")!;
    expect(btn.textContent).toContain("git.noChanges");
    expect(btn.className).toContain("text-ink-faint");
    expect(btn.className).toContain("opacity-50");
  });

  it("applies has-changes button classes when git has changes", async () => {
    (gitStatus as Mock).mockResolvedValue(
      makeStatus({
        hasChanges: true,
        files: [{ path: "x.ts", status: "M", additions: 1, deletions: 0 }],
        totalAdditions: 1,
        totalDeletions: 0,
      }),
    );

    mount();
    await drainMicrotasks();

    const btn = document.body.querySelector("button")!;
    expect(btn.className).toContain("text-ink-muted");
    // No opacity modifier for has-changes state
    expect(btn.className).not.toMatch(/opacity-/);
  });

  it("applies no-changes button classes when git has no changes", async () => {
    (gitStatus as Mock).mockResolvedValue(makeStatus());

    mount();
    await drainMicrotasks();

    const btn = document.body.querySelector("button")!;
    expect(btn.className).toContain("text-ink-faint");
    expect(btn.className).toContain("opacity-50");
  });

  it("applies loading-specific classes while loading", async () => {
    (gitStatus as Mock).mockImplementation(() => new Promise(() => {}));

    mount();
    await drainMicrotasks();

    const btn = document.body.querySelector("button")!;
    expect(btn.className).toContain("text-ink-faint");
    expect(btn.className).toContain("opacity-30");
  });

  // ── onCleanup stops timers ───────────────────────────────────────────

  it("stops polling when the component is disposed (onCleanup clears intervals)", async () => {
    const dispose = render(
      () => <GitIndicator {...defaultProps} />,
      document.body,
    );
    await drainMicrotasks();

    vi.clearAllMocks();

    // Dispose the component → onCleanup runs → intervals cleared
    dispose();

    // Advance 30s — no intervals should fire because they were cleared
    vi.advanceTimersByTime(30000);
    expect(gitStatus).toHaveBeenCalledTimes(0);
  });

  // ── guard reset after completion ─────────────────────────────────────

  it("allows a new status poll after the previous one completes (inFlight resets)", async () => {
    mount();
    await drainMicrotasks();

    vi.clearAllMocks();

    // Advance 10s → refreshStatus fires, gitStatus is called
    vi.advanceTimersByTime(10000);
    await drainMicrotasks();
    expect(gitStatus).toHaveBeenCalledTimes(1);

    // Advance another 10s → the previous call has resolved, so inFlight was
    // reset and this tick fires again
    vi.advanceTimersByTime(10000);
    await drainMicrotasks();
    expect(gitStatus).toHaveBeenCalledTimes(2);
  });

  // ── workspace propagation ────────────────────────────────────────────

  it("calls gitStatus with the provided workspace", async () => {
    mount();
    await drainMicrotasks();

    expect(gitStatus).toHaveBeenCalledWith("/test/workspace");
  });
});
