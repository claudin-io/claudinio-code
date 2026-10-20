import { describe, it, expect, vi, afterEach } from "vitest";
import { render } from "solid-js/web";
import { getTasks, setTasks, dismissGoldenTasks, type TaskItem } from "../lib/ipc";
import { TasksPanel } from "./TasksPanel";

// The task list as rows, the document panel's own shape (single-timeline
// plan, Phase 2): a status chip and a readable title per task, instead of a
// column of dots that only said what a task was on hover. The person who
// asked for it could not tell what the agent was doing without hovering.

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

function flush() {
  return Promise.resolve();
}

async function mount(tasks = sampleTasks, props: { underDocument?: boolean } = {}) {
  vi.mocked(getTasks).mockResolvedValue(tasks);
  const dispose = render(
    () => <TasksPanel workspace="/test" underDocument={props.underDocument} />,
    document.body,
  );
  await flush();
  return dispose;
}

const rows = () => Array.from(document.body.querySelectorAll<HTMLElement>("[data-task-row]"));
const chip = (i: number) => rows()[i].querySelector<HTMLButtonElement>("[data-task-status]")!;

// ── Tests ──────────────────────────────────────────────────────────

describe("TasksPanel", () => {
  afterEach(() => {
    document.body.innerHTML = "";
    vi.clearAllMocks();
  });

  it("shows each task as a row with its title readable, golden goals first", async () => {
    const dispose = await mount();
    const titles = rows().map((r) => r.querySelector("[data-task-title]")!.textContent);
    expect(titles).toEqual([
      // A goal reads as the goal; a "-0" plan half only exists in a
      // session from before one timeline.
      "Plan: Refactor auth flow",
      "Implement caching",
      "First Task",
      "In Progress",
      "Completed Task",
    ]);
    dispose();
  });

  it("calls getTasks with the workspace on mount", async () => {
    const dispose = await mount();
    expect(getTasks).toHaveBeenCalledWith("/test");
    dispose();
  });

  it("labels each row's status in words", async () => {
    const dispose = await mount();
    expect(rows().map((r) => r.querySelector("[data-task-status]")!.textContent)).toEqual([
      "Doing",
      "Done",
      "Todo",
      "Doing",
      "Done",
    ]);
    dispose();
  });

  it("cycles a task's status from its chip and persists it", async () => {
    const dispose = await mount();
    chip(2).click(); // First Task: todo → doing
    await flush();
    expect(chip(2).textContent).toBe("Doing");
    expect(setTasks).toHaveBeenCalledWith(
      "/test",
      expect.arrayContaining([expect.objectContaining({ id: "task-1", status: "doing" })]),
    );
    chip(2).click();
    await flush();
    expect(chip(2).textContent).toBe("Done");
    chip(2).click();
    await flush();
    expect(chip(2).textContent).toBe("Todo");
    dispose();
  });

  it("reverts to the stored list when saving a cycle fails", async () => {
    vi.mocked(setTasks).mockRejectedValueOnce(new Error("network"));
    const dispose = await mount();
    chip(2).click();
    await flush();
    await flush();
    expect(getTasks).toHaveBeenCalledTimes(2);
    dispose();
  });

  it("puts a task's description and journal in the row's tooltip", async () => {
    const dispose = await mount();
    const tip = rows()[2].getAttribute("title")!;
    expect(tip).toContain("Has description and journal");
    expect(tip).toContain("Found the main function");
    dispose();
  });

  it("marks golden goals and lets the person dismiss only those", async () => {
    const remaining = sampleTasks.filter((t) => t.id !== "golden-plan-0");
    vi.mocked(dismissGoldenTasks).mockResolvedValueOnce(remaining);
    const dispose = await mount();
    expect(rows()[0].classList.contains("gold-outline")).toBe(true);
    expect(rows()[2].classList.contains("gold-outline")).toBe(false);
    expect(rows()[2].querySelector("[data-task-dismiss]")).toBeNull();
    rows()[0].querySelector<HTMLButtonElement>("[data-task-dismiss]")!.click();
    await flush();
    expect(dismissGoldenTasks).toHaveBeenCalledWith("/test", "golden-plan-0");
    expect(rows().length).toBe(remaining.length);
    dispose();
  });

  it("explains single-step work when there is no task list", async () => {
    const dispose = await mount([]);
    expect(document.body.querySelector("[data-tasks-empty]")).not.toBeNull();
    expect(rows().length).toBe(0);
    dispose();
  });

  // Under a document the list holds only what survives it — the person's
  // golden goals — and says nothing when there are none.
  it("under a document, shows the goals and nothing when there are none", async () => {
    let dispose = await mount(sampleTasks.filter((t) => t.id.startsWith("golden-")), {
      underDocument: true,
    });
    expect(rows().length).toBe(2);
    expect(document.body.querySelector("[data-tasks-empty]")).toBeNull();
    dispose();
    document.body.innerHTML = "";
    dispose = await mount([], { underDocument: true });
    expect(document.body.textContent).toBe("");
    dispose();
  });

  it("reports the task count on load", async () => {
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

  it("stops polling once disposed", async () => {
    vi.useFakeTimers();
    vi.mocked(getTasks).mockResolvedValue(sampleTasks);
    const dispose = render(() => <TasksPanel workspace="/test" />, document.body);
    await flush();
    dispose();
    const callsBefore = vi.mocked(getTasks).mock.calls.length;
    vi.advanceTimersByTime(30000);
    await flush();
    expect(vi.mocked(getTasks).mock.calls.length).toBe(callsBefore);
    vi.useRealTimers();
  });
});
