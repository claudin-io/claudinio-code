import { Component, createSignal, createMemo, Show, For, onMount, onCleanup } from "solid-js";
import { marked } from "marked";
import { commitAndPush, interruptSession, AgentEvent, ToolCallData, ToolResultData, AskUserData, UserAnswer, submitAnswers } from "../lib/ipc";
import { t } from "../lib/grill-me";
import { Icon } from "./Icon";
import QuestionCard from "./QuestionCard";

interface TimelineStep {
  type: "thinking" | "tool" | "text";
  thinking?: { text: string; startedAt: number; endedAt?: number };
  tool?: { call: ToolCallData; result?: ToolResultData; status: "running" | "ok" | "error" };
  text?: string;
}

const CommitPushModal: Component<{
  workspace: string;
  open: boolean;
  onClose: () => void;
}> = (props) => {
  let sessionId: string | null = null;
  const [status, setStatus] = createSignal<"running" | "completed" | "failed" | "interrupted">("running");
  const [steps, setSteps] = createSignal<TimelineStep[]>([]);
  const [expandedStep, setExpandedStep] = createSignal<number | null>(null);
  const [currentAskUser, setCurrentAskUser] = createSignal<AskUserData | null>(null);

  const handleEvent = (event: AgentEvent) => {
    switch (event.event) {
      case "ToolCall":
        setSteps((prev) => [
          ...prev,
          {
            type: "tool",
            tool: { call: event.data, status: "running" },
          },
        ]);
        break;
      case "ToolResult":
        setSteps((prev) => {
          const next = [...prev];
          for (let i = next.length - 1; i >= 0; i--) {
            if (next[i].type === "tool" && next[i].tool?.call.toolId === event.data.toolId) {
              next[i] = {
                ...next[i],
                tool: { ...next[i].tool!, result: event.data, status: event.data.error ? "error" : "ok" },
              };
              break;
            }
          }
          return next;
        });
        break;
      case "Thinking":
        setSteps((prev) => {
          const next = [...prev];
          const last = next[next.length - 1];
          if (last?.type === "thinking") {
            next[next.length - 1] = {
              ...last,
              thinking: { ...last.thinking!, text: last.thinking!.text + event.data },
            };
          } else {
            next.push({
              type: "thinking",
              thinking: { text: event.data, startedAt: Date.now() },
            });
          }
          return next;
        });
        break;
      case "TextStep":
        setSteps((prev) => [
          ...prev,
          { type: "text", text: event.data.text },
        ]);
        break;
      case "AskUser":
        setCurrentAskUser(event.data as AskUserData);
        break;
      case "Done":
        setStatus("completed");
        break;
      case "Error":
        setStatus("failed");
        break;
    }
  };

  const handleCancel = async () => {
    if (sessionId) {
      setStatus("interrupted");
      try {
        await interruptSession(sessionId);
      } catch {}
      // Short delay to show interrupted state
      setTimeout(() => {
        props.onClose();
      }, 800);
    } else {
      props.onClose();
    }
  };

  const handleAnswers = async (answers: UserAnswer[]) => {
    const ask = currentAskUser();
    if (!ask) return;
    try {
      await submitAnswers(ask.sessionId, ask.toolId, answers);
      setCurrentAskUser(null);
    } catch (e) {
      console.error("Failed to submit answers:", e);
    }
  };

  onMount(async () => {
    if (!props.open) return;
    try {
      const result = await commitAndPush(props.workspace, handleEvent);
      sessionId = result.sessionId;
    } catch (e) {
      setStatus("failed");
    }
  });

  // Keyboard handler: ESC cancels
  onMount(() => {
    const onKey = (e: KeyboardEvent) => {
      if (e.key === "Escape") {
        e.preventDefault();
        handleCancel();
      }
    };
    document.addEventListener("keydown", onKey);
    onCleanup(() => document.removeEventListener("keydown", onKey));
  });

  // Auto-close when completed
  createMemo(() => {
    if (status() === "completed" || status() === "failed") {
      setTimeout(() => props.onClose(), 1500);
    }
  });

  const badgeClass = () => {
    switch (status()) {
      case "running": return "bg-accent/15 text-accent";
      case "completed": return "bg-success/15 text-success";
      case "failed": return "bg-danger/15 text-danger";
      case "interrupted": return "bg-amber-500/15 text-amber-500";
    }
  };

  const statusLabel = () => {
    switch (status()) {
      case "running": return t("commitPush.committing");
      case "completed": return t("commitPush.completed");
      case "failed": return t("commitPush.failed");
      case "interrupted": return t("commitPush.interrupted");
    }
  };

  return (
    <Show when={props.open}>
      <div class="fixed inset-0 z-50 flex items-center justify-center bg-black/40">
        <div class="flex max-h-[85vh] w-full max-w-3xl flex-col rounded-xl bg-surface-0 shadow-2xl">
          {/* Header */}
          <div class="flex items-center justify-between border-b border-border-subtle px-5 py-3">
            <div class="flex items-center gap-2">
              <Icon name="git-commit" class="h-4 w-4 text-ink-muted" stroke />
              <span class="font-semibold text-[14px] text-ink">{t("commitPush.modalTitle")}</span>
              <span class={`rounded px-1.5 py-0.5 text-[10px] font-medium ${badgeClass()}`}>
                {statusLabel()}
              </span>
            </div>
            <Show when={status() === "running"}>
              <button
                onClick={handleCancel}
                class="flex h-7 items-center gap-1.5 rounded-md bg-danger/20 px-3 text-[12px] font-medium text-danger hover:bg-danger/40"
              >
                <Icon name="stop" class="h-3.5 w-3.5" />
                {t("commitPush.cancel")}
              </button>
            </Show>
            <Show when={status() !== "running"}>
              <button
                onClick={props.onClose}
                class="flex h-7 w-7 items-center justify-center rounded-md text-ink-muted hover:bg-surface-2"
              >
                <Icon name="x" class="h-4 w-4" />
              </button>
            </Show>
          </div>
          {/* Timeline */}
          <div class="overflow-y-auto px-5 py-3 space-y-4">
            <Show when={currentAskUser()}>
              <div class="pb-2">
                <QuestionCard ask={currentAskUser()!} onSubmit={handleAnswers} />
              </div>
            </Show>
            <For each={steps()}>
              {(step, i) => (
                <>
                  <Show when={step.type === "thinking" && step.thinking}>
                    <div>
                      <button
                        onClick={() => setExpandedStep(expandedStep() === i() ? null : i())}
                        class="flex w-full items-center gap-2 rounded-md bg-surface-1 px-3 py-2 text-left"
                      >
                        <span class="h-1.5 w-1.5 shrink-0 rounded-full bg-accent" />
                        <span class="text-[12px] font-medium text-ink-muted">
                          {expandedStep() === i() ? "Hide" : "Show"} reasoning
                        </span>
                        <span class="text-[10px] text-ink-faint">
                          {(step.thinking!.text.length / 4).toFixed(0)} tokens
                        </span>
                      </button>
                      <Show when={expandedStep() === i()}>
                        <pre class="mt-1 whitespace-pre-wrap break-words rounded-md bg-surface-1/50 px-3 py-2 font-mono text-[11px] leading-[1.6] text-ink-muted">
                          {step.thinking!.text}
                        </pre>
                      </Show>
                    </div>
                  </Show>
                  <Show when={step.type === "tool" && step.tool}>
                    <div>
                      <button
                        onClick={() => setExpandedStep(expandedStep() === i() ? null : i())}
                        class="flex w-full items-center gap-2 rounded-md bg-surface-1 px-3 py-2 text-left"
                      >
                        <span class={`h-1.5 w-1.5 shrink-0 rounded-full ${step.tool!.status === "running" ? "bg-accent animate-pulse" : step.tool!.status === "ok" ? "bg-success" : "bg-danger"}`} />
                        <span class="font-mono text-[12px] text-ink">{step.tool!.call.toolName}</span>
                        <span class="truncate text-[11px] text-ink-faint">
                          {JSON.stringify(step.tool!.call.args).slice(0, 80)}
                        </span>
                      </button>
                      <Show when={expandedStep() === i()}>
                        <div class="mt-1 space-y-1">
                          <pre class="whitespace-pre-wrap break-words rounded-md bg-surface-1/50 px-3 py-2 font-mono text-[11px] leading-[1.6] text-ink-muted">
                            {JSON.stringify(step.tool!.call.args, null, 2)}
                          </pre>
                          <Show when={step.tool!.result}>
                            <div class="rounded-md bg-surface-1/30 px-3 py-2">
                              <span class="text-[10px] font-semibold uppercase tracking-wider text-ink-faint">Result</span>
                              <pre class="mt-0.5 whitespace-pre-wrap break-words font-mono text-[11px] leading-[1.6] text-ink-muted">
                                {step.tool!.result!.output?.slice(0, 500) || step.tool!.result!.error}
                              </pre>
                            </div>
                          </Show>
                        </div>
                      </Show>
                    </div>
                  </Show>
                  <Show when={step.type === "text" && step.text}>
                    <div class="rounded-md bg-surface-1 p-3">
                      <div
                        class="prose-content text-[12px] leading-[1.6] text-ink-muted"
                        innerHTML={marked.parse(step.text!, { async: false }) as string}
                      />
                    </div>
                  </Show>
                </>
              )}
            </For>
            <Show when={steps().length === 0 && status() === "running"}>
              <div class="flex items-center justify-center py-8">
                <div class="flex items-center gap-2 text-ink-faint">
                  <div class="h-4 w-4 animate-spin rounded-full border-2 border-accent border-t-transparent" />
                  <span class="text-[13px]">Starting...</span>
                </div>
              </div>
            </Show>
          </div>
        </div>
      </div>
    </Show>
  );
};

export default CommitPushModal;
