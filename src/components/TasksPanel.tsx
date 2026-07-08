import { createSignal, For, Show, onMount, onCleanup, type Component } from "solid-js";
import { Portal } from "solid-js/web";
import { getTasks, setTasks, type TaskItem } from "../lib/ipc";
import { t } from "../lib/grill-me";
import { Icon } from "./Icon";

export const TasksPanel: Component<{
  workspace: string;
  onTasksChange?: (count: number) => void;
}> = (props) => {
  const [tasks, setTasksState] = createSignal<TaskItem[]>([]);
  const [hoveredId, setHoveredId] = createSignal<string | null>(null);
  const [hoveredTop, setHoveredTop] = createSignal(0);
  const [pollTimer, setPollTimer] = createSignal<ReturnType<typeof setInterval> | null>(null);
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

  onMount(() => {
    load();
    const id = setInterval(load, 3000);
    setPollTimer(id);
  });

  onCleanup(() => {
    if (pollTimer()) clearInterval(pollTimer()!);
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

  const dotColor = (s: string) => {
    if (s === "done") return "bg-success";
    if (s === "doing") return "bg-white";
    return "bg-ink-faint";
  };

  const isGolden = (task: TaskItem) => task.id.startsWith("golden-");

  // Golden task ids end in -0 (plan) or -1 (execute); the backend keeps the
  // raw goal text in `title` so the visible label is composed here, localized.
  const goldenPhase = (task: TaskItem): "plan" | "execute" =>
    task.id.endsWith("-0") ? "plan" : "execute";

  const displayTitle = (task: TaskItem) =>
    isGolden(task) ? t(`golden.task.${goldenPhase(task)}`, task.title) : task.title;

  const displayDescription = (task: TaskItem) =>
    isGolden(task) ? t(`golden.task.${goldenPhase(task)}.desc`, task.title) : task.description;

  const statusLabel = (s: string) => {
    if (s === "done") return t("tasks.status.done");
    if (s === "doing") return t("tasks.status.doing");
    return t("tasks.status.todo");
  };

  const scheduleClose = () => {
    if (closeTimer) clearTimeout(closeTimer);
    closeTimer = setTimeout(() => {
      setHoveredId(null);
    }, 150);
  };

  const cancelClose = () => {
    if (closeTimer) {
      clearTimeout(closeTimer);
      closeTimer = null;
    }
  };

  const hoveredTask = () => {
    const id = hoveredId();
    return id ? tasks().find((t) => t.id === id) ?? null : null;
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
                const rect = e.currentTarget.getBoundingClientRect();
                setHoveredTop(rect.top);
                setHoveredId(task.id);
              }}
              onMouseLeave={scheduleClose}
              class="shrink-0 rounded-full hover:ring-2 hover:ring-accent/40"
              classList={{ "gold-outline p-px": isGolden(task) }}
              title={
                isGolden(task)
                  ? `${displayTitle(task)} (${t("golden.task.badge")}) — ${task.status}`
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
          <span class="inline-block h-2 w-2 rounded-full bg-ink-faint" title={t("tasks.status.todo")} />
          <span class="inline-block h-2 w-2 rounded-full bg-white" title={t("tasks.status.doing")} />
          <span class="inline-block h-2 w-2 rounded-full bg-success" title={t("tasks.status.done")} />
        </div>
      </Show>

      {/* Popover via Portal — floats above everything */}
      <Portal>
        <Show when={hoveredTask()} keyed>
          {(task) => (
            <div
              style={{
                position: "fixed",
                right: "48px",
                top: `${hoveredTop()}px`,
                "z-index": 9999,
              }}
              onMouseEnter={cancelClose}
              onMouseLeave={scheduleClose}
              class="w-64 rounded-lg bg-surface-1 p-3"
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
                  {t("golden.task.badge")}
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
                    "bg-white/15 text-white": task.status === "doing",
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
                    {t("tasks.panel.journal")}
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
              <p class="mt-2 text-[10px] text-ink-faint">{t("tasks.panel.cycleStatus")}</p>
            </div>
          )}
        </Show>
      </Portal>
    </div>
  );
};
