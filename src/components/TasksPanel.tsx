import { createSignal, For, Show, onCleanup, type Component } from "solid-js";
import { Popover } from "./Popover";
import { getTasks, setTasks, dismissGoldenTasks, type TaskItem } from "../lib/ipc";
import { Icon } from "./Icon";
import { createVisibilityAwareInterval } from "../lib/visibility";

export const TasksPanel: Component<{
  workspace: string;
  onTasksChange?: (count: number) => void;
}> = (props) => {
  const [tasks, setTasksState] = createSignal<TaskItem[]>([]);
  // Only the setter is used: the tooltip renders from hoveredTaskSnapshot.
  const [, setHoveredId] = createSignal<string | null>(null);
  const [hoveredTaskSnapshot, setHoveredTaskSnapshot] = createSignal<TaskItem | null>(null);
  const [hoveredElement, setHoveredElement] = createSignal<HTMLElement | null>(null);
  let closeTimer: ReturnType<typeof setTimeout> | null = null;

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

  onCleanup(() => {
    if (closeTimer) clearTimeout(closeTimer);
  });

  const cycleStatus = async (id: string) => {
    const next: Record<string, TaskItem['status']> = {
      todo: "doing",
      doing: "done",
      done: "todo",
    };
    const updated = tasks().map((t) =>
      t.id === id ? { ...t, status: next[t.status] || "todo" } : t,
    );
    setTasksState(updated);
    try {
      await setTasks(props.workspace, updated);
    } catch {
      load();
    }
  };

  const dismissGolden = async (id: string) => {
    setHoveredId(null);
    setHoveredTaskSnapshot(null);
    try {
      const remaining = await dismissGoldenTasks(props.workspace, id);
      setTasksState(remaining);
      props.onTasksChange?.(remaining.length);
    } catch {
      load();
    }
  };

  const dotColor = (s: string) => {
    if (s === "done") return "bg-success";
    if (s === "doing") return "bg-amber-500";
    return "bg-ink-faint";
  };

  const isGolden = (task: TaskItem) => task.id.startsWith("golden-");

  // Golden task ids end in -0 (plan) or -1 (execute); the backend keeps the
  // raw goal text in `title` so the visible label is composed here, localized.
  const goldenPhase = (task: TaskItem): "plan" | "execute" =>
    task.id.endsWith("-0") ? "plan" : "execute";

  const displayTitle = (task: TaskItem) =>
    isGolden(task)
      ? goldenPhase(task) === "plan"
        ? `Plan: ${task.title}`
        : `Execute: ${task.title}`
      : task.title;

  const displayDescription = (task: TaskItem) =>
    isGolden(task)
      ? goldenPhase(task) === "plan"
        ? `Create a detailed plan to achieve the goal: ${task.title}`
        : `Execute the plan and verify the goal is met: ${task.title}`
      : task.description;

  const statusLabel = (s: string) => {
    if (s === "done") return "Done";
    if (s === "doing") return "Doing";
    return "Todo";
  };

  const scheduleClose = () => {
    if (closeTimer) clearTimeout(closeTimer);
    closeTimer = setTimeout(() => {
      setHoveredId(null);
      setHoveredTaskSnapshot(null);
    }, 150);
  };

  const cancelClose = () => {
    if (closeTimer) {
      clearTimeout(closeTimer);
      closeTimer = null;
    }
  };

  return (
    <div class="flex h-full w-10 flex-col items-center gap-1.5 border-l border-border-subtle bg-surface-1 py-2">
      {/* Task dots column */}
      <div class="flex flex-1 flex-col items-center gap-2 overflow-y-auto px-1">
        <For each={tasks()}>
          {(task) => (
            <button
              onClick={() => cycleStatus(task.id)}
              onMouseEnter={(e) => {
                cancelClose();
                setHoveredElement(e.currentTarget);
                setHoveredId(task.id);
                setHoveredTaskSnapshot(task);
              }}
              onMouseLeave={scheduleClose}
              class="shrink-0 rounded-full hover:ring-2 hover:ring-accent/40"
              classList={{ "gold-outline p-px": isGolden(task) }}
              title={
                isGolden(task)
                  ? `${displayTitle(task)} (${"Golden — mandatory goal"}) — ${task.status}`
                  : `${task.title} — ${task.status}`
              }
            >
              <span
                class={`block h-2.5 w-2.5 rounded-full ${dotColor(task.status)}`}
              />
            </button>
          )}
        </For>
      </div>

      {/* Summary — three mini dots, color legend */}
      <Show when={tasks().length > 0}>
        <div class="flex flex-col items-center gap-1.5 border-t border-border-subtle px-1 pt-2">
          <span class="inline-block h-2 w-2 rounded-full bg-ink-faint" title={"Todo"} />
          <span class="inline-block h-2 w-2 rounded-full bg-amber-500" title={"Doing"} />
          <span class="inline-block h-2 w-2 rounded-full bg-success" title={"Done"} />
        </div>
      </Show>

      {/* Popover — floats above everything */}
      <Popover
        open={hoveredTaskSnapshot() !== null}
        onClose={() => {}}
        triggerRef={() => hoveredElement()}
        anchorPoint={{x:1,y:0}}
        originPoint={{x:0,y:0}}
        showBackdrop={false}
        gap={{x:8}}
      >
        <Show when={hoveredTaskSnapshot()} keyed>
          {(task) => (
            <div
              onMouseEnter={cancelClose}
              onMouseLeave={scheduleClose}
              class="w-64 rounded-lg bg-surface-1 p-3 max-h-[50vh] overflow-y-auto"
              classList={{
                "gold-outline": isGolden(task),
                "border border-border-subtle shadow-modal": !isGolden(task),
              }}
            >
              {/* Golden badge */}
              <Show when={isGolden(task)}>
                <span class="mb-1 inline-flex items-center gap-1 rounded-full bg-amber-500/10 px-1.5 py-0.5 text-[10px] font-medium text-amber-500">
                  <Icon
                    name={task.status === "done" ? "goal-achieved" : "goal"}
                    stroke={task.status === "done"}
                    class="h-3 w-3"
                  />
                  {"Golden — mandatory goal"}
                </span>
              </Show>
              {/* Title + status */}
              <div class="flex items-start justify-between gap-2">
                <span
                  class="text-sm font-medium leading-tight"
                  classList={{
                    "text-ink": task.status !== "done",
                    "text-ink-faint line-through": task.status === "done",
                  }}
                >
                  {displayTitle(task)}
                </span>
                <span
                  class="shrink-0 whitespace-nowrap rounded-full px-1.5 py-0.5 text-[10px] font-medium"
                  classList={{
                    "bg-success/15 text-success": task.status === "done",
                    "bg-amber-500/15 text-amber-500": task.status === "doing",
                    "bg-ink-faint/10 text-ink-muted": task.status === "todo",
                  }}
                >
                  {statusLabel(task.status)}
                </span>
              </div>

              {/* Description */}
              <Show when={displayDescription(task)}>
                <p class="mt-2 whitespace-pre-wrap break-words text-[12px] leading-relaxed text-ink-muted">
                  {displayDescription(task)}
                </p>
              </Show>

              {/* Journal */}
              <Show when={task.journal.length > 0}>
                <div class="mt-2 space-y-0.5">
                  <span class="text-[10px] font-semibold uppercase tracking-wider text-ink-faint">
                    {"Journal"}
                  </span>
                  <For each={task.journal}>
                    {(entry) => (
                      <p class="ml-1 border-l-2 border-accent/20 pl-2 text-[11px] leading-relaxed text-ink-muted">
                        {entry}
                      </p>
                    )}
                  </For>
                </div>
              </Show>

              {/* Click hint */}
              <p class="mt-2 text-[10px] text-ink-faint">{"Cycle status"}</p>

              {/* Dismiss — golden tasks can be completed but not deleted by
                  the model; this lets the user drop a stale goal so it stops
                  re-triggering the golden loop on later turns. */}
              <Show when={isGolden(task)}>
                <button
                  onClick={() => dismissGolden(task.id)}
                  class="mt-2 text-[10px] font-medium text-ink-faint hover:text-ink-muted hover:underline"
                >
                  {"Dismiss this goal"}
                </button>
              </Show>
            </div>
          )}
        </Show>
      </Popover>
    </div>
  );
};
