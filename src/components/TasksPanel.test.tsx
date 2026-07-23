import { describe, it, expect, vi, afterEach } from "vitest";
import { render } from "solid-js/web";
import { getTasks, setTasks, dismissGoldenTasks, type TaskItem } from "../lib/ipc";
import { TasksPanel } from "./TasksPanel";

// ── Mocks ──────────────────────────────────────────────────────────

vi.mock("../lib/ipc", () => ({
  getTasks: vi.fn(),
  setTasks: vi.fn().mockResolvedValue(undefined),
  dismissGoldenTasks: vi.fn().mockResolvedValue([]),
}));


// ── Fixtures ───────────────────────────────────────────────────────

const sampleTasks: TaskItem[] = [
  {
    id: "task-1",
    title: "First Task",
    description: "Has description and journal",
    journal: ["Found the main function", "Refactored the module"],
    status: "todo",
  },
  {
    id: "task-2",
    title: "In Progress",
    description: "Working on it",
    journal: ["Started implementation"],
    status: "doing",
  },
  {
    id: "golden-plan-0",
    title: "Refactor auth flow",
    description: "Plan the auth refactor",
    journal: ["Identified dependencies"],
    status: "doing",
  },
  {
    id: "golden-exec-1",
    title: "Implement caching",
    description: "Execute the caching layer",
    journal: [],
    status: "done",
  },
  {
    id: "task-5",
    title: "Completed Task",
    description: "",
    journal: [],
    status: "done",
  },
];

// ── Helpers ────────────────────────────────────────────────────────

/** Flush pending microtasks so that Solid reactivity settles. */
function flush() {
  return Promise.resolve();
}

/**
 * Render TasksPanel into document.body and wait for getTasks to resolve.
 * Returns the dispose function for cleanup.
 */
async function mount(workspace = "/test") {
  vi.mocked(getTasks).mockResolvedValue(sampleTasks);
  const dispose = render(() => <TasksPanel workspace={workspace} />, document.body);
  await flush();
  return dispose;
}

/** Find the nth task dot's <span> by re-querying (DOM reference may go stale on re-render). */
function dotSpan(buttonIndex: number) {
  const buttons = document.body.querySelectorAll("button");
  return buttons[buttonIndex].querySelector("span")!;
}

// ── Tests ──────────────────────────────────────────────────────────

describe("TasksPanel", () => {
  afterEach(() => {
    document.body.innerHTML = "";
    vi.clearAllMocks();
  });

  // ── Rendering ──────────────────────────────────────────────────

  it("renders a button dot for each task", async () => {
    const dispose = await mount();
    expect(document.body.querySelectorAll("button").length).toBe(sampleTasks.length);
    dispose();
  });

  it("calls getTasks with the workspace on mount", async () => {
    const dispose = await mount("/my-workspace");
    expect(getTasks).toHaveBeenCalledWith("/my-workspace");
    dispose();
  });

  it("renders dot colors matching task status", async () => {
    const dispose = await mount();

    // task-1: todo → bg-ink-faint
    expect(dotSpan(0).className).toContain("bg-ink-faint");

    // task-2: doing → bg-amber-500
    expect(dotSpan(1).className).toContain("bg-amber-500");

    // task-5: done → bg-success
    expect(dotSpan(4).className).toContain("bg-success");

    dispose();
  });

  // ── Status cycling ─────────────────────────────────────────────

  it("cycles status on click: todo → doing → done → todo", async () => {
    const dispose = await mount();

    // Re-query each time since Solid may replace the DOM node on re-render
    const clickAndCheck = async (expectedClass: string) => {
      const buttons = document.body.querySelectorAll("button");
      buttons[0].click();
      await flush();
      const span = document.body.querySelectorAll("button")[0].querySelector("span")!;
      expect(span.className).toContain(expectedClass);
    };

    // todo → doing (amber-500)
    expect(dotSpan(0).className).toContain("bg-ink-faint");
    await clickAndCheck("bg-amber-500"); // doing

    // doing → done
    await clickAndCheck("bg-success"); // done

    // done → todo
    await clickAndCheck("bg-ink-faint"); // todo

    dispose();
  });

  it("persists each cycle via setTasks with the updated array", async () => {
    const dispose = await mount();
    const buttons = document.body.querySelectorAll("button");

    buttons[0].click();
    await flush();

    expect(setTasks).toHaveBeenCalledWith(
      "/test",
      expect.arrayContaining([
        expect.objectContaining({ id: "task-1", status: "doing" }),
      ]),
    );
    dispose();
  });

  it("reverts to server state on setTasks failure", async () => {
    vi.mocked(setTasks).mockRejectedValueOnce(new Error("network"));
    const dispose = await mount();
    const buttons = document.body.querySelectorAll("button");

    buttons[0].click();
    await flush();

    // After the rejection, load() re-fetches from getTasks to revert
    expect(getTasks).toHaveBeenCalledTimes(2);
    dispose();
  });

  // ── Hover popover ──────────────────────────────────────────────

  it("shows popover on mouse enter with title and status", async () => {
    const dispose = await mount();
    const buttons = document.body.querySelectorAll("button");

    buttons[0].dispatchEvent(new MouseEvent("mouseenter", { bubbles: true }));

    expect(document.body.textContent).toContain("First Task");
    expect(document.body.textContent).toContain("Todo");

    dispose();
  });

  it("shows description and journal entries in the popover", async () => {
    const dispose = await mount();
    const buttons = document.body.querySelectorAll("button");

    buttons[0].dispatchEvent(new MouseEvent("mouseenter", { bubbles: true }));

    expect(document.body.textContent).toContain("Has description and journal");
    expect(document.body.textContent).toContain("Found the main function");
    expect(document.body.textContent).toContain("Refactored the module");
    expect(document.body.textContent).toContain("Journal");

    dispose();
  });

  it("omits journal section from popover when journal is empty", async () => {
    const dispose = await mount();
    const buttons = document.body.querySelectorAll("button");

    // golden-exec-1 (index 3) has empty journal
    buttons[3].dispatchEvent(new MouseEvent("mouseenter", { bubbles: true }));

    expect(document.body.textContent).not.toContain("Journal");

    dispose();
  });

  // ── Golden tasks ───────────────────────────────────────────────

  it("applies gold-outline class to golden task buttons", async () => {
    const dispose = await mount();
    const buttons = document.body.querySelectorAll("button");

    // Tasks whose id starts with "golden-"
    expect(buttons[2].className).toContain("gold-outline");
    expect(buttons[3].className).toContain("gold-outline");

    // Regular task must NOT have gold-outline
    expect(buttons[0].className).not.toContain("gold-outline");

    dispose();
  });

  it("shows golden badge in popover for golden tasks", async () => {
    const dispose = await mount();
    const buttons = document.body.querySelectorAll("button");

    buttons[2].dispatchEvent(new MouseEvent("mouseenter", { bubbles: true }));

    expect(document.body.textContent).toContain("Golden \u2014 mandatory goal");

    dispose();
  });

  it("prefixes a golden task title with its phase", async () => {
    const dispose = await mount();
    const buttons = document.body.querySelectorAll("button");

    // golden-plan-0 ends in -0 → "plan" phase → `Plan: ${title}`
    buttons[2].dispatchEvent(new MouseEvent("mouseenter", { bubbles: true }));
    expect(document.body.textContent).toContain("Plan: Refactor auth flow");

    dispose();
  });

  it("shows a dismiss button for golden tasks and calls dismissGoldenTasks on click", async () => {
    const remaining = sampleTasks.filter((t) => t.id !== "golden-plan-0");
    vi.mocked(dismissGoldenTasks).mockResolvedValueOnce(remaining);
    const dispose = await mount();
    const buttons = document.body.querySelectorAll("button");

    buttons[2].dispatchEvent(new MouseEvent("mouseenter", { bubbles: true }));
    const dismissBtn = Array.from(document.body.querySelectorAll("button")).find(
      (b) => b.textContent === "Dismiss this goal",
    );
    expect(dismissBtn).toBeTruthy();

    dismissBtn!.click();
    await flush();

    expect(dismissGoldenTasks).toHaveBeenCalledWith("/test", "golden-plan-0");
    expect(document.body.querySelectorAll("button").length).toBe(remaining.length);

    dispose();
  });

  it("does not show a dismiss button for non-golden tasks", async () => {
    const dispose = await mount();
    const buttons = document.body.querySelectorAll("button");

    buttons[0].dispatchEvent(new MouseEvent("mouseenter", { bubbles: true }));
    const dismissBtn = Array.from(document.body.querySelectorAll("button")).find(
      (b) => b.textContent === "Dismiss this goal",
    );
    expect(dismissBtn).toBeFalsy();

    dispose();
  });

  // ── Hover dismiss ──────────────────────────────────────────────

  it("hides popover on mouse leave after 150ms delay", async () => {
    vi.useFakeTimers();

    vi.mocked(getTasks).mockResolvedValue(sampleTasks);
    const dispose = render(() => <TasksPanel workspace="/test" />, document.body);
    await flush();

    const buttons = document.body.querySelectorAll("button");

    // Show popover
    buttons[0].dispatchEvent(new MouseEvent("mouseenter", { bubbles: true }));
    expect(document.body.textContent).toContain("First Task");

    // Leave
    buttons[0].dispatchEvent(new MouseEvent("mouseleave", { bubbles: true }));
    expect(document.body.textContent).toContain("First Task"); // still visible

    // Advance past the 150ms close timeout
    vi.advanceTimersByTime(200);

    expect(document.body.textContent).not.toContain("First Task");

    vi.useRealTimers();
    dispose();
  });

  it("cancels close timer when re-entering before timeout", async () => {
    vi.useFakeTimers();

    vi.mocked(getTasks).mockResolvedValue(sampleTasks);
    const dispose = render(() => <TasksPanel workspace="/test" />, document.body);
    await flush();

    const buttons = document.body.querySelectorAll("button");

    // Show popover
    buttons[0].dispatchEvent(new MouseEvent("mouseenter", { bubbles: true }));
    expect(document.body.textContent).toContain("First Task");

    // Leave — starts 150ms close timer
    buttons[0].dispatchEvent(new MouseEvent("mouseleave", { bubbles: true }));

    // Re-enter partway through the timer (before 150ms elapses)
    vi.advanceTimersByTime(100);
    buttons[0].dispatchEvent(new MouseEvent("mouseenter", { bubbles: true }));

    // Advance past the original timeout — should NOT hide because timer was cancelled
    vi.advanceTimersByTime(200);
    expect(document.body.textContent).toContain("First Task");

    // Now leave and let the NEW timer fire
    buttons[0].dispatchEvent(new MouseEvent("mouseleave", { bubbles: true }));
    vi.advanceTimersByTime(200);
    expect(document.body.textContent).not.toContain("First Task");

    vi.useRealTimers();
    dispose();
  });

  // ── Summary legend ─────────────────────────────────────────────

  it("renders summary legend dots (todo / doing / done)", async () => {
    const dispose = await mount();

    // The summary section renders three <span> elements identified by title
    const todoDot = document.body.querySelector('[title="Todo"]');
    const doingDot = document.body.querySelector('[title="Doing"]');
    const doneDot = document.body.querySelector('[title="Done"]');

    expect(todoDot).toBeTruthy();
    expect(todoDot!.className).toContain("bg-ink-faint");
    expect(doingDot).toBeTruthy();
    expect(doingDot!.className).toContain("bg-amber-500");
    expect(doneDot).toBeTruthy();
    expect(doneDot!.className).toContain("bg-success");

    dispose();
  });

  it("hides summary section when there are no tasks", async () => {
    vi.mocked(getTasks).mockResolvedValue([]);
    const dispose = render(() => <TasksPanel workspace="/test" />, document.body);
    await flush();

    const todoDot = document.body.querySelector('[title="Todo"]');
    expect(todoDot).toBeNull();

    dispose();
  });

  // ── Callbacks ──────────────────────────────────────────────────

  it("calls onTasksChange with task count on mount", async () => {
    const onTasksChange = vi.fn();
    vi.mocked(getTasks).mockResolvedValue(sampleTasks);
    const dispose = render(
      () => <TasksPanel workspace="/test" onTasksChange={onTasksChange} />,
      document.body,
    );
    await flush();

    expect(onTasksChange).toHaveBeenCalledWith(sampleTasks.length);

    dispose();
  });

  // ── hoveredTask edge cases ────────────────────────────────────

  it("hoveredTask returns null when no task is hovered (initial state)", async () => {
    vi.mocked(getTasks).mockResolvedValue(sampleTasks);
    const dispose = render(() => <TasksPanel workspace="/test" />, document.body);
    await flush();

    // No mouseenter event has fired → hoveredId is null
    // hoveredTask() → id is null → returns null (no popover rendered)
    // Verify no popover content is visible despite tasks existing
    expect(document.body.textContent).not.toContain("Journal");
    expect(document.body.textContent).not.toContain("Cycle status");

    dispose();
  });

  it("keeps popover visible even when task list changes (snapshot frozen on hover)", async () => {
    vi.useFakeTimers();

    // First call returns sampleTasks (includes "task-1"), second call returns empty
    vi.mocked(getTasks)
      .mockResolvedValueOnce(sampleTasks)
      .mockResolvedValue([]);

    const dispose = render(() => <TasksPanel workspace="/test" />, document.body);
    await flush();

    // Hover over task-1 → snapshot is frozen
    const buttons = document.body.querySelectorAll("button");
    buttons[0].dispatchEvent(new MouseEvent("mouseenter", { bubbles: true }));
    expect(document.body.textContent).toContain("Cycle status");

    // Advance timers to trigger the 3s poll interval → load() re-fetches tasks → empty array
    vi.advanceTimersByTime(3000);
    await flush();

    // hoveredTaskSnapshot is still the frozen task from mouseenter.
    // The popover stays visible despite the task list being empty now.
    // This is the fix: no more flicker on poll updates.
    expect(document.body.textContent).toContain("Cycle status");
    expect(document.body.textContent).toContain("First Task");

    // Manually clear to prove it CAN be dismissed (e.g. via the backdrop or
    // a mouseleave on the popover card itself, but in this edge case where
    // trigger elements are gone, user would move mouse or click elsewhere)
    // In practice tasks don't just disappear — the test proves the freeze works.
    const popoverCard = document.body.querySelector('[class*="rounded-lg"]') as HTMLElement;
    popoverCard.dispatchEvent(new MouseEvent("mouseleave", { bubbles: true }));
    vi.advanceTimersByTime(200);
    expect(document.body.textContent).not.toContain("Cycle status");

    vi.useRealTimers();
    dispose();
  });

  // ── scheduleClose: double mouseleave (covers if(closeTimer) true branch) ──
  it("scheduleClose clears previous timer when called twice (double mouseleave)", async () => {
    vi.useFakeTimers();

    vi.mocked(getTasks).mockResolvedValue(sampleTasks);
    const dispose = render(() => <TasksPanel workspace="/test" />, document.body);
    await flush();

    const buttons = document.body.querySelectorAll("button");

    // Show popover
    buttons[0].dispatchEvent(new MouseEvent("mouseenter", { bubbles: true }));
    expect(document.body.textContent).toContain("First Task");

    // Leave — first call: closeTimer was null, timer is set
    buttons[0].dispatchEvent(new MouseEvent("mouseleave", { bubbles: true }));
    expect(document.body.textContent).toContain("First Task"); // still visible

    // Leave again (before timer fires) — second call: closeTimer is truthy,
    // so clearTimeout is called (covering the if/true branch), then new timer set
    buttons[0].dispatchEvent(new MouseEvent("mouseleave", { bubbles: true }));

    // Only the second timer counts down
    vi.advanceTimersByTime(100);
    expect(document.body.textContent).toContain("First Task"); // still visible

    vi.advanceTimersByTime(100); // total 200ms past the second leave
    expect(document.body.textContent).not.toContain("First Task");

    vi.useRealTimers();
    dispose();
  });

  // ── cycleStatus with unknown status fallback (line 45: || "todo") ──
  it("cycleStatus falls back to 'todo' for unknown status", async () => {
    const unknownStatusTasks: TaskItem[] = [
      {
        id: "weird",
        title: "Weird status",
        description: "",
        journal: [],
        status: "unknown" as TaskItem['status'],
      },
    ];
    vi.mocked(getTasks).mockResolvedValue(unknownStatusTasks);
    const dispose = render(() => <TasksPanel workspace="/test" />, document.body);
    await flush();

    // Click the weird task button — cycles via next["unknown"] = undefined || "todo"
    const button = document.body.querySelector("button") as HTMLElement;
    button.click();
    await flush();

    // After cycle, status should be "todo" (the fallback)
    expect(setTasks).toHaveBeenCalledWith(
      "/test",
      expect.arrayContaining([
        expect.objectContaining({ id: "weird", status: "todo" }),
      ]),
    );

    dispose();
  });

  // ── onCleanup with running pollTimer (line 34: if(pollTimer()) true branch) ──
  it("calls clearInterval on cleanup when pollTimer is active", async () => {
    vi.useFakeTimers();
    vi.mocked(getTasks).mockResolvedValue(sampleTasks);

    const dispose = render(() => <TasksPanel workspace="/test" />, document.body);
    await flush();

    // pollTimer should be set (setInterval was called in onMount)
    // On dispose, onCleanup fires → pollTimer() is truthy → clearInterval is called
    dispose();

    // After dispose, no further polling calls to getTasks should happen
    const callsBefore = vi.mocked(getTasks).mock.calls.length;
    vi.advanceTimersByTime(3000);
    await flush();
    expect(vi.mocked(getTasks).mock.calls.length).toBe(callsBefore);

    vi.useRealTimers();
  });
});
