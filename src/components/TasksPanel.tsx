import { createSignal, For, Show, type Component } from "solid-js";
import { getTasks, setTasks, dismissGoldenTasks, type TaskItem } from "../lib/ipc";
import { Icon } from "./Icon";
import { createVisibilityAwareInterval } from "../lib/visibility";
import { docStatusLabel } from "./DocumentPanel";

/** The chip's colours: the document panel's, so both plans read the same. */
function chipClass(status: string): string {
  if (status === "done") return "bg-success/15 text-success";
  if (status === "doing") return "bg-amber-500/15 text-amber-500";
  return "bg-ink-faint/10 text-ink-muted";
}

const isGolden = (task: TaskItem) => task.id.startsWith("golden-");

// A goal is one task and reads as the goal. A "-0" id is the plan half a
// session from before one timeline still carries.
const displayTitle = (task: TaskItem) =>
  isGolden(task) && task.id.endsWith("-0") ? `Plan: ${task.title}` : task.title;

/** The row's tooltip: what the task is and what was found doing it. */
const tooltip = (task: TaskItem) =>
  [isGolden(task) ? "Golden — mandatory goal" : "", task.description, ...task.journal]
    .filter(Boolean)
    .join("\n\n");

/**
 * The session's task list — its plan when it has no living document — as
 * rows: a status chip (click cycles it) and the title, golden goals first.
 * Under a document (`underDocument`) only the person's golden goals remain
 * (the document replaced the list), and nothing is shown without them.
 * Without a list, a narrow note says the work is a single step.
 */
export const TasksPanel: Component<{
  workspace: string;
  underDocument?: boolean;
  /// Called with the task count each time the list is loaded or changed:
  /// App sizes the column from it.
  onTasksChange?: (count: number) => void;
}> = (props) => {
  const [tasks, setTasksState] = createSignal<TaskItem[]>([]);

  const load = async () => {
    try {
      const t = await getTasks(props.workspace);
      setTasksState(t);
      props.onTasksChange?.(t.length);
    } catch {
      // no workspace open or other transient error
    }
  };

  // Pauses while the window is hidden — no reason to poll tasks in background.
  createVisibilityAwareInterval(load, 10000);

  const ordered = () => [...tasks().filter(isGolden), ...tasks().filter((t) => !isGolden(t))];

  const cycleStatus = async (id: string) => {
    const next: Record<string, TaskItem["status"]> = { todo: "doing", doing: "done", done: "todo" };
    const updated = tasks().map((t) => (t.id === id ? { ...t, status: next[t.status] || "todo" } : t));
    setTasksState(updated);
    try {
      await setTasks(props.workspace, updated);
    } catch {
      load();
    }
  };

  const dismissGolden = async (id: string) => {
    try {
      const remaining = await dismissGoldenTasks(props.workspace, id);
      setTasksState(remaining);
      props.onTasksChange?.(remaining.length);
    } catch {
      load();
    }
  };

  return (
    <Show
      when={tasks().length > 0}
      fallback={
        <Show when={!props.underDocument}>
          {/* Single-step work gets no task list: say so rather than stand empty. */}
          <div
            data-tasks-empty
            title={"No task list — single-step work"}
            class="flex h-full w-10 flex-col items-center gap-2 border-l border-border-subtle bg-surface-1 pt-3 text-ink-faint"
          >
            <Icon name="layers" class="h-3.5 w-3.5" />
            <span class="text-[10px] tracking-wide [writing-mode:vertical-rl]">{"Single step"}</span>
          </div>
        </Show>
      }
    >
      <div data-tasks-panel class="flex w-full flex-col bg-surface-1" classList={{ "h-full": !props.underDocument }}>
        <div class="flex items-center gap-2 border-b border-border-subtle px-3 py-2">
          <Icon name={props.underDocument ? "goal" : "layers"} class="h-3.5 w-3.5 shrink-0 text-ink-faint" />
          <span class="text-[12px] font-medium text-ink">{props.underDocument ? "Goals" : "Plan"}</span>
        </div>
        <div class="overflow-y-auto py-1" classList={{ "flex-1": !props.underDocument }}>
          <For each={ordered()}>
            {(task) => (
              <div
                data-task-row
                class="mx-1 flex items-center gap-2 rounded px-2 py-1 hover:bg-surface-2"
                classList={{ "gold-outline": isGolden(task) }}
                title={tooltip(task)}
              >
                <button
                  data-task-status
                  class={`shrink-0 whitespace-nowrap rounded-full px-1.5 py-0.5 text-[10px] font-medium ${chipClass(task.status)}`}
                  title={"Cycle status"}
                  onClick={() => cycleStatus(task.id)}
                >
                  {docStatusLabel(task.status)}
                </button>
                <span
                  data-task-title
                  class="min-w-0 flex-1 truncate text-[12px]"
                  classList={{ "text-ink": task.status !== "done", "text-ink-muted": task.status === "done" }}
                >
                  {displayTitle(task)}
                </span>
                <Show when={isGolden(task)}>
                  {/* Golden goals can be completed but not deleted by the
                      model; this lets the person drop a stale goal so it stops
                      re-triggering the golden loop on later turns. */}
                  <button
                    data-task-dismiss
                    class="shrink-0 text-ink-faint hover:text-ink-muted"
                    title={"Dismiss this goal"}
                    onClick={() => dismissGolden(task.id)}
                  >
                    <Icon name="x" class="h-3 w-3" />
                  </button>
                </Show>
              </div>
            )}
          </For>
        </div>
      </div>
    </Show>
  );
};
