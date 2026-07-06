import { createSignal, For, onCleanup, onMount, Show, type Component } from "solid-js";
import {
  sendMessage,
  approveTool,
  rejectTool,
  submitAnswers,
  newSession,
  listSessions,
  loadSession,
  queueSteering,
  interruptSession,
  type AgentEvent,
  type AskUserData,
  type ToolCallData,
  type EditProposalData,
  type DoneData,
  type ToolResultData,
  type SubagentStartedData,
  type SubagentDoneData,
  type Phase,
  type SessionSummary,
  type SessionRecord,
  type UserAnswer,
} from "../lib/ipc";
import { marked } from "marked";
import hljs from "highlight.js";
import { DiffViewer } from "./DiffViewer";
import { Icon, toolIcon, type IconName } from "./Icon";

marked.use({
  renderer: {
    code({ text, lang }) {
      const highlighted = lang && hljs.getLanguage(lang)
        ? hljs.highlight(text, { language: lang }).value
        : hljs.highlightAuto(text).value;
      const label = lang
        ? `<span class="code-lang-label">${lang}</span>`
        : "";
      return `<div class="code-block">${label}<pre class="hljs"><code>${highlighted}</code></pre></div>`;
    },
  },
});

type Status = "idle" | "thinking" | "awaiting_approval" | "awaiting_input" | "done" | "error";

interface ChatMessage {
  role: "user" | "assistant";
  text: string;
  steps?: TimelineItem[];
  done?: DoneData;
}

interface SubagentTimelineState {
  id: string;
  name: string;
  goal: string;
  mode: string;
  status: "running" | "completed" | "failed" | "interrupted" | "max_rounds";
  rounds: number;
  inputTokens: number;
  outputTokens: number;
  steps: TimelineItem[];
}

interface TimelineItem {
  type: "thinking" | "tool" | "phase" | "phase_result" | "text" | "steering" | "subagent";
  thinking?: { text: string; startedAt: number; endedAt?: number };
  tool?: {
    call: ToolCallData;
    result?: ToolResultData;
    status: "running" | "ok" | "error";
  };
  phase?: Phase;
  phaseResult?: { phase: Phase; text: string };
  text?: string;
  steering?: { text: string };
  subagent?: SubagentTimelineState;
}

const PHASE_LABEL: Record<Phase, string> = {
  plan: "Planejamento",
  execute: "Execução",
  summary: "Sumário",
};

function formatTokens(n: number): string {
  if (n < 1000) return `${n}`;
  return `${(n / 1000).toFixed(n < 10000 ? 1 : 0)}k`;
}

function summarizeArgs(args: Record<string, unknown>): string {
  const path = args.path as string | undefined;
  if (path) return path;
  const pattern = args.pattern as string | undefined;
  if (pattern) return `/${pattern}/`;
  const content = args.content as string | undefined;
  if (content) return `${content.slice(0, 60)}…`;
  return JSON.stringify(args).slice(0, 80);
}

function formatDuration(ms: number): string {
  if (ms < 1000) return `${ms}ms`;
  return `${(ms / 1000).toFixed(1)}s`;
}

interface ContentBlockJson {
  type: string;
  text?: string;
  id?: string;
  name?: string;
  input?: Record<string, unknown>;
  tool_use_id?: string;
  content?: string;
}

// Rebuild the chat transcript from a reopened session's JSONL records. User
// turns become user bubbles; everything between them folds into one assistant
// message with a phase/tool timeline. Tool results are paired to their calls by
// tool_use_id across turn records.
function recordsToMessages(records: SessionRecord[]): ChatMessage[] {
  const out: ChatMessage[] = [];
  let steps: TimelineItem[] = [];
  let assistantText = "";
  let done: DoneData | undefined;
  const toolIndex = new Map<string, number>();

  const flush = () => {
    if (steps.length || assistantText || done) {
      out.push({ role: "assistant", text: assistantText, steps: [...steps], done });
      steps = [];
      assistantText = "";
      done = undefined;
      toolIndex.clear();
    }
  };

  for (const rec of records) {
    const kind = rec.kind;
    if (kind === "user") {
      flush();
      out.push({ role: "user", text: String(rec.text ?? "") });
    } else if (kind === "phase") {
      steps.push({ type: "phase", phase: rec.phase as Phase });
    } else if (kind === "phase_result") {
      const phase = rec.phase as Phase;
      const text = String(rec.text ?? "");
      steps.push({ type: "phase_result", phaseResult: { phase, text } });
      if (phase === "summary") assistantText = text;
    } else if (kind === "turn") {
      const role = rec.role as string;
      const content = (rec.content as ContentBlockJson[]) ?? [];
      if (role === "assistant") {
        const hasToolUse = content.some((b) => b.type === "tool_use");
        for (const block of content) {
          if (block.type === "tool_use") {
            steps.push({
              type: "tool",
              tool: {
                call: {
                  sessionId: "",
                  toolId: block.id ?? "",
                  toolName: block.name ?? "",
                  args: block.input ?? {},
                  permission: "auto",
                },
                status: "ok",
              },
            });
            toolIndex.set(block.id ?? "", steps.length - 1);
          } else if (block.type === "text" && block.text) {
            if (hasToolUse) {
              steps.push({ type: "text", text: block.text });
            } else {
              assistantText = block.text;
            }
          }
        }
      } else if (role === "user") {
        for (const block of content) {
          if (block.type === "tool_result" && block.tool_use_id) {
            const idx = toolIndex.get(block.tool_use_id);
            if (idx !== undefined) {
              const item = steps[idx];
              if (item.type === "tool" && item.tool) {
                steps[idx] = {
                  ...item,
                  tool: {
                    ...item.tool,
                    result: {
                      toolId: block.tool_use_id,
                      toolName: item.tool.call.toolName,
                      output: block.content ?? "",
                    },
                  },
                };
              }
            }
          }
        }
      }
    } else if (kind === "steering") {
      steps.push({
        type: "steering",
        steering: { text: String(rec.text ?? "") },
      });
    } else if (kind === "done") {
      done = {
        stopReason: "end_turn",
        textOutput: assistantText,
        inputTokens: Number(rec.input_tokens ?? 0),
        outputTokens: Number(rec.output_tokens ?? 0),
      };
    }
  }
  flush();
  return out;
}

export const ChatPanel: Component = () => {
  const [input, setInput] = createSignal("");
  const [messages, setMessages] = createSignal<ChatMessage[]>([]);
  const [status, setStatus] = createSignal<Status>("idle");
  const [pendingApprovals, setPendingApprovals] = createSignal<(ToolCallData & { subagentName?: string })[]>([]);
  const [currentAskUser, setCurrentAskUser] = createSignal<AskUserData | null>(null);
  const [currentSteps, setCurrentSteps] = createSignal<TimelineItem[]>([]);
  const [subagentState, setSubagentState] = createSignal<Record<string, SubagentTimelineState>>({});
  const [openSubagentId, setOpenSubagentId] = createSignal<string | null>(null);
  const [thinkingStart, setThinkingStart] = createSignal(0);
  const [liveExpandedStep, setLiveExpandedStep] = createSignal<number | null>(null);
  const [sessions, setSessions] = createSignal<SessionSummary[]>([]);
  const [showSessions, setShowSessions] = createSignal(false);
  const [activeSessionId, setActiveSessionId] = createSignal<string | null>(null);
  const [queuedSteering, setQueuedSteering] = createSignal<string[]>([]);

  let messagesEndRef: HTMLDivElement | undefined;
  let inputRef: HTMLTextAreaElement | undefined;

  const scrollToBottom = () => {
    setTimeout(() => messagesEndRef?.scrollIntoView({ behavior: "smooth" }), 50);
  };

  const addOrUpdateToolIn = (steps: TimelineItem[], item: TimelineItem): TimelineItem[] => {
    const idx = steps.findIndex(
      (s) => s.type === "tool" && s.tool?.call.toolId === item.tool?.call.toolId,
    );
    if (idx >= 0) {
      const next = [...steps];
      next[idx] = item;
      return next;
    }
    return [...steps, item];
  };

  const applyToolResultIn = (steps: TimelineItem[], data: ToolResultData): TimelineItem[] => {
    const idx = steps.findIndex(
      (s) => s.type === "tool" && s.tool?.call.toolId === data.toolId,
    );
    if (idx === -1) return steps;
    const next = [...steps];
    const t = next[idx];
    if (t.type !== "tool" || !t.tool) return steps;
    next[idx] = {
      type: "tool",
      tool: { ...t.tool, result: data, status: data.error ? "error" : "ok" },
    };
    return next;
  };

  const updateSubagentTimelineItem = (subagents: Record<string, SubagentTimelineState>): TimelineItem[] | null => {
    const values = Object.values(subagents);
    if (values.length === 0) return null;
    return values.map((sa) => ({
      type: "subagent" as const,
      subagent: sa,
    }));
  };

  const processSubagentEvent = (
    subagents: Record<string, SubagentTimelineState>,
    subagentId: string,
    event: AgentEvent,
  ): { subagents: Record<string, SubagentTimelineState>; approval?: ToolCallData & { subagentName?: string } } => {
    const sa = subagents[subagentId];
    if (!sa) return { subagents };

    let steps = [...sa.steps];
    let approval: (ToolCallData & { subagentName?: string }) | undefined;

    if (event.event === "TextStep") {
      steps = [...steps, { type: "text", text: event.data.text }];
    } else if (event.event === "Thinking") {
      const now = Date.now();
      const last = steps[steps.length - 1];
      if (last?.type === "thinking") {
        steps = steps.map((s, i) =>
          i === steps.length - 1
            ? { type: "thinking" as const, thinking: { ...s.thinking!, text: event.data } }
            : s,
        );
      } else {
        steps = [...steps, { type: "thinking" as const, thinking: { text: event.data, startedAt: now } }];
      }
    } else if (event.event === "ToolCall") {
      const data = event.data as ToolCallData;
      steps = addOrUpdateToolIn(steps, {
        type: "tool",
        tool: { call: data, status: "running" },
      });
      if (data.permission === "requires_approval") {
        approval = { ...data, subagentName: sa.name };
      }
    } else if (event.event === "ToolResult") {
      steps = applyToolResultIn(steps, event.data as ToolResultData);
    }

    return {
      subagents: { ...subagents, [subagentId]: { ...sa, steps } },
      approval,
    };
  };

  const handleEvent = (event: AgentEvent) => {
    if (event.event === "TextStep") {
      setCurrentSteps((prev) => [...prev, { type: "text", text: event.data.text }]);
      scrollToBottom();
    } else if (event.event === "Thinking") {
      if (!event.data) return;
      const now = Date.now();
      if (thinkingStart() === 0) setThinkingStart(now);
      setCurrentSteps((prev) => {
        const last = prev[prev.length - 1];
        if (last?.type === "thinking") {
          return prev.map((s, i) =>
            i === prev.length - 1
              ? { type: "thinking" as const, thinking: { ...s.thinking!, text: event.data } }
              : s,
          );
        }
        return [
          ...prev,
          { type: "thinking" as const, thinking: { text: event.data, startedAt: now } } as TimelineItem,
        ];
      });
      scrollToBottom();
    } else if (event.event === "ToolCall") {
      const data = event.data as ToolCallData;
      setCurrentSteps((prev) => addOrUpdateToolIn(prev, {
        type: "tool",
        tool: { call: data, status: "running" },
      }));
      if (data.permission === "requires_approval") {
        setStatus("awaiting_approval");
        setPendingApprovals((prev) => [...prev, data]);
      }
      scrollToBottom();
    } else if (event.event === "ToolResult") {
      const data = event.data as ToolResultData;
      setCurrentSteps((prev) => applyToolResultIn(prev, data));
      scrollToBottom();
    } else if (event.event === "AskUser") {
      setCurrentAskUser(event.data as AskUserData);
      setStatus("awaiting_input");
      scrollToBottom();
    } else if (event.event === "SteeringInjected") {
      setQueuedSteering((prev) => prev.filter((s) => s !== event.data.text));
      setCurrentSteps((prev) => [
        ...prev,
        { type: "steering" as const, steering: { text: event.data.text } } as TimelineItem,
      ]);
      scrollToBottom();
    } else if (event.event === "SubagentStarted") {
      const d = event.data as SubagentStartedData;
      const now = Date.now();
      setSubagentState((prev) => ({
        ...prev,
        [d.subagentId]: {
          id: d.subagentId,
          name: d.name,
          goal: d.goal,
          mode: d.mode,
          status: "running",
          rounds: 0,
          inputTokens: 0,
          outputTokens: 0,
          steps: [{ type: "thinking" as const, thinking: { text: "Aguardando resposta...", startedAt: now } }],
        },
      }));
      setCurrentSteps((prev) => [
        ...prev,
        { type: "subagent" as const, subagent: { id: d.subagentId, name: d.name, goal: d.goal, mode: d.mode, status: "running", rounds: 0, inputTokens: 0, outputTokens: 0, steps: [] } },
      ]);
      scrollToBottom();
    } else if (event.event === "SubagentDone") {
      const d = event.data as SubagentDoneData;
      setSubagentState((prev) => {
        const sa = prev[d.subagentId];
        if (!sa) return prev;
        const status = d.status === "completed" ? "completed" as const
          : d.status === "failed" ? "failed" as const
          : d.status === "interrupted" ? "interrupted" as const
          : d.status === "max_rounds" ? "max_rounds" as const
          : "completed" as const;
        const finalSteps = sa.steps.map((s) => {
          if (s.type === "thinking") {
            return { ...s, thinking: { ...s.thinking!, endedAt: Date.now() } };
          }
          return s;
        });
        return {
          ...prev,
          [d.subagentId]: { ...sa, status, rounds: d.rounds, inputTokens: d.inputTokens, outputTokens: d.outputTokens, steps: finalSteps },
        };
      });
      scrollToBottom();
    } else if (event.event === "Subagent") {
      const d = event.data;
      const result = processSubagentEvent(subagentState(), d.subagentId, d.event);
      setSubagentState(result.subagents);
      // sync timeline items
      const items = updateSubagentTimelineItem(result.subagents);
      if (items) {
        setCurrentSteps((prev) => {
          // replace subagent items in-place
          const newSteps = prev.map((s) => {
            if (s.type === "subagent") {
              const found = items.find((i) => i.subagent?.id === s.subagent?.id);
              return found ?? s;
            }
            return s;
          });
          return newSteps;
        });
      }
      if (result.approval) {
        setPendingApprovals((prev) => [...prev, result.approval!]);
        setStatus("awaiting_approval");
      }
      scrollToBottom();
    } else if (event.event === "Done") {
      const data = event.data as DoneData;
      const steps = currentSteps();
      const final = steps.map((s) => {
        if (s.type === "thinking") {
          return { ...s, thinking: { ...s.thinking!, endedAt: Date.now() } };
        }
        return s;
      });
      setMessages((prev) => [
        ...prev,
        { role: "assistant", text: data.textOutput, steps: final, done: data },
      ]);
      setCurrentSteps([]);
      setQueuedSteering([]);
      setSubagentState({});
      setPendingApprovals([]);
      setThinkingStart(0);
      setStatus("done");
      scrollToBottom();
    } else if (event.event === "Error") {
      setMessages((prev) => [...prev, { role: "user", text: `Erro: ${event.data}` }]);
      setCurrentSteps([]);
      setThinkingStart(0);
      setSubagentState({});
      setStatus("error");
    }
  };

  const send = async () => {
    const text = input().trim();
    if (!text || status() === "awaiting_approval" || status() === "awaiting_input") return;

    // If the agent is currently thinking, queue the message as steering
    if (status() === "thinking") {
      const sid = activeSessionId();
      if (sid) {
        try {
          await queueSteering(sid, text);
        } catch {
          // best-effort
        }
      }
      setQueuedSteering((prev) => [...prev, text]);
      setInput("");
      return;
    }

    setMessages((prev) => [...prev, { role: "user", text }]);
    setInput("");
    setCurrentSteps([]);
    setThinkingStart(0);
    setStatus("thinking");

    try {
      const result = await sendMessage(text, handleEvent);
      setActiveSessionId(result.sessionId);
    } catch (e) {
      setMessages((prev) => [...prev, { role: "user", text: `Falha ao enviar: ${String(e)}` }]);
      setStatus("error");
    }
  };

  const handleApprove = async (tc: ToolCallData) => {
    if (!tc) return;
    setPendingApprovals((prev) => prev.filter((p) => p.toolId !== tc.toolId));
    if (pendingApprovals().length <= 1) setStatus("thinking");
    try {
      await approveTool(tc.sessionId, tc.toolId);
    } catch (e) {
      setMessages((prev) => [...prev, { role: "user", text: `Aprovação falhou: ${String(e)}` }]);
    }
  };

  const handleAnswers = async (answers: UserAnswer[]) => {
    const ask = currentAskUser();
    if (!ask) return;
    setStatus("thinking");
    setCurrentAskUser(null);
    try {
      await submitAnswers(ask.sessionId, ask.toolId, answers);
    } catch (e) {
      setMessages((prev) => [...prev, { role: "user", text: `Envio de respostas falhou: ${String(e)}` }]);
    }
  };

  const handleReject = async (tc: ToolCallData) => {
    if (!tc) return;
    setPendingApprovals((prev) => prev.filter((p) => p.toolId !== tc.toolId));
    if (pendingApprovals().length <= 1) setStatus("thinking");
    try {
      await rejectTool(tc.sessionId, tc.toolId);
    } catch (e) {
      setMessages((prev) => [...prev, { role: "user", text: `Rejeição falhou: ${String(e)}` }]);
    }
  };

  const startNewSession = async () => {
    if (status() === "thinking" || status() === "awaiting_approval" || status() === "awaiting_input") return;
    try {
      await newSession();
    } catch {
      /* fresh session is best-effort */
    }
    setMessages([]);
    setCurrentSteps([]);
    setThinkingStart(0);
    setStatus("idle");
    setShowSessions(false);
  };

  const toggleSessions = async () => {
    const next = !showSessions();
    setShowSessions(next);
    if (next) {
      try {
        setSessions(await listSessions());
      } catch {
        setSessions([]);
      }
    }
  };

  const reopenSession = async (id: string) => {
    if (status() === "thinking" || status() === "awaiting_approval" || status() === "awaiting_input") return;
    try {
      const records = await loadSession(id);
      setMessages(recordsToMessages(records));
      setCurrentSteps([]);
      setThinkingStart(0);
      setStatus("idle");
      setShowSessions(false);
      scrollToBottom();
    } catch (e) {
      setMessages((prev) => [...prev, { role: "user", text: `Falha ao reabrir: ${String(e)}` }]);
    }
  };

  const handleKeyDown = (e: KeyboardEvent) => {
    if (e.key === "Enter" && !e.shiftKey) {
      e.preventDefault();
      send();
    }
  };

  const autoResize = () => {
    if (inputRef) {
      inputRef.style.height = "auto";
      inputRef.style.height = `${Math.min(inputRef.scrollHeight, 156)}px`;
    }
  };

  // Global ESC handler: only fires when status is "thinking"
  onMount(() => {
    const onKey = (e: KeyboardEvent) => {
      if (e.key === "Escape" && status() === "thinking") {
        e.preventDefault();
        const sid = activeSessionId();
        if (sid) {
          interruptSession(sid).catch(() => {});
        }
      }
    };
    document.addEventListener("keydown", onKey);
    onCleanup(() => document.removeEventListener("keydown", onKey));
  });

  const statusLabel = () => {
    switch (status()) {
      case "thinking": return "Trabalhando";
      case "awaiting_approval": return "Aguardando aprovação";
      case "awaiting_input": return "Aguardando sua resposta";
      case "done": return "Pronto";
      case "error": return "Erro";
      default: return "Parado";
    }
  };

  const statusDot = (): string => {
    switch (status()) {
      case "thinking": return "bg-accent";
      case "awaiting_input": return "bg-accent";
      case "done": return "bg-success";
      case "error": return "bg-danger";
      default: return "bg-ink-faint";
    }
  };

  return (
    <div class="flex h-full flex-col bg-surface-0">
      <div class="relative flex items-center justify-between border-b border-border-subtle px-6 py-1.5">
        <div class="flex items-center gap-2">
          <span class="text-[11px] font-semibold uppercase tracking-wider text-ink-faint">
            Agente
          </span>
          <span
            class={`inline-block h-[6px] w-[6px] rounded-full ${statusDot()}`}
            classList={{
              "animate-pulse-soft":
                status() === "thinking" || status() === "awaiting_approval" || status() === "awaiting_input",
            }}
          />
          <span class="text-[11px] text-ink-faint">{statusLabel()}</span>
        </div>

        <div class="flex items-center gap-1">
          <button
            onClick={startNewSession}
            disabled={status() === "thinking" || status() === "awaiting_approval"}
            class="flex items-center gap-1 rounded px-2 py-1 text-[11px] text-ink-muted hover:bg-surface-2 disabled:opacity-30"
            title="Nova sessão"
          >
            <Icon name="plus" class="h-3.5 w-3.5" />
            Nova
          </button>
          <button
            onClick={toggleSessions}
            class="flex items-center gap-1 rounded px-2 py-1 text-[11px] text-ink-muted hover:bg-surface-2"
            title="Sessões salvas"
          >
            <Icon name="clock" class="h-3.5 w-3.5" />
            Histórico
          </button>
        </div>

        <Show when={showSessions()}>
          <div class="absolute right-4 top-9 z-20 max-h-80 w-80 overflow-y-auto rounded-lg border border-border-subtle bg-surface-1 py-1 shadow-lg">
            <Show
              when={sessions().length > 0}
              fallback={<div class="px-3 py-2 text-[12px] text-ink-faint">Nenhuma sessão salva.</div>}
            >
              <For each={sessions()}>
                {(s) => (
                  <button
                    onClick={() => reopenSession(s.sessionId)}
                    class="flex w-full flex-col gap-0.5 px-3 py-1.5 text-left hover:bg-surface-2"
                  >
                    <span class="truncate text-[12px] text-ink">{s.title}</span>
                    <span class="font-mono text-[10px] text-ink-faint">
                      {new Date(s.updatedAt).toLocaleString()} · {s.turnCount} turno{s.turnCount === 1 ? "" : "s"}
                    </span>
                  </button>
                )}
              </For>
            </Show>
          </div>
        </Show>
      </div>

      <div class="flex flex-1 flex-col overflow-y-auto">
        <div class="w-full px-6 py-4">
          <For each={messages()}>
            {(msg) => (
              <div class="mb-6 max-w-[70ch]">
                <Show when={msg.role === "user"}>
                  <div class="mb-1">
                    <span class="text-[11px] font-semibold uppercase tracking-wider text-accent">
                      Você
                    </span>
                  </div>
                  <div class="border-l-2 border-accent/60 pl-3">
                    <p class="whitespace-pre-wrap break-words text-[13px] leading-[1.65] text-ink">
                      {msg.text}
                    </p>
                  </div>
                </Show>

                <Show when={msg.role === "assistant" && msg.steps && msg.steps!.length > 0}>
                  <Trajectory
                    steps={msg.steps!}
                    tokens={msg.done ? { input: msg.done.inputTokens, output: msg.done.outputTokens } : undefined}
                    onViewDetails={(id) => setOpenSubagentId(id)}
                  />

                  <Show when={msg.text}>
                    <div
                      class="prose-content text-[13px] text-ink"
                      innerHTML={marked.parse(msg.text, { async: false }) as string}
                    />
                  </Show>
                </Show>
              </div>
            )}
          </For>

          <Show when={status() === "thinking" || status() === "done" || status() === "awaiting_input"}>
            <div class="mb-6 max-w-[70ch]">
              <div class="trajectory-rail flex flex-col gap-0.5">
                <TimelineSteps
                  steps={currentSteps()}
                  expandedStep={liveExpandedStep()}
                  onToggle={(i) => setLiveExpandedStep(liveExpandedStep() === i ? null : i)}
                  isLive={status() === "thinking"}
                  onViewDetails={(id) => setOpenSubagentId(id)}
                />
              </div>
            </div>
          </Show>

          <Show when={pendingApprovals().length > 0 && status() === "awaiting_approval"}>
            <div class="mb-6 max-w-[70ch] flex flex-col gap-3">
              <For each={pendingApprovals()}>
                {(tc) => (
                  <div>
                    <Show when={tc.subagentName}>
                      <div class="mb-1 flex items-center gap-1.5">
                        <span class="text-[10px] font-semibold uppercase tracking-wider text-ink-faint">
                          Subagent: {tc.subagentName}
                        </span>
                      </div>
                    </Show>
                    <ApprovalCard
                      toolCall={tc}
                      onApprove={() => handleApprove(tc)}
                      onReject={() => handleReject(tc)}
                    />
                  </div>
                )}
              </For>
            </div>
          </Show>

          <Show when={currentAskUser() && status() === "awaiting_input"}>
            <div class="mb-6 max-w-[70ch]">
              <QuestionCard ask={currentAskUser()!} onSubmit={handleAnswers} />
            </div>
          </Show>

          <div ref={messagesEndRef} />
        </div>
      </div>

      <Show when={openSubagentId()}>
        <SubagentModal
          subagent={subagentState()[openSubagentId()!]}
          onClose={() => setOpenSubagentId(null)}
        />
      </Show>

      <Show when={queuedSteering().length > 0}>
        <div class="flex flex-wrap gap-1.5 border-t border-border-subtle px-6 py-1.5">
          <For each={queuedSteering()}>
            {(s) => (
              <span class="inline-flex items-center gap-1 rounded-full bg-accent/10 px-2 py-0.5 text-[11px] text-accent">
                <span class="h-1.5 w-1.5 rounded-full bg-accent" />
                {s.length > 40 ? `${s.slice(0, 40)}…` : s}
              </span>
            )}
          </For>
        </div>
      </Show>

      <div class="border-t border-border-subtle px-6 py-3">
        <div class="w-full">
          <div class="flex items-end gap-2 rounded-lg border border-border-subtle bg-surface-2 p-2 focus-within:border-accent/60">
            <textarea
              ref={inputRef!}
              value={input()}
              onInput={(e) => {
                setInput(e.currentTarget.value);
                autoResize();
              }}
              onKeyDown={handleKeyDown}
              disabled={status() === "awaiting_approval" || status() === "awaiting_input"}
              placeholder={
                status() === "awaiting_approval"
                  ? "Aprove ou rejeite a edição primeiro…"
                  : status() === "awaiting_input"
                    ? "Responda as perguntas acima primeiro…"
                    : status() === "thinking"
                      ? "Digite para orientar o agente… (Esc para pausar)"
                      : "Pergunte algo sobre o código…"
              }
              class="max-h-[156px] min-h-[36px] flex-1 resize-none border-0 bg-transparent p-1 text-[13px] text-ink placeholder:text-ink-faint focus:outline-none disabled:opacity-50"
              rows={1}
            />
            <button
              onClick={send}
              disabled={
                !input().trim() ||
                status() === "awaiting_approval" ||
                status() === "awaiting_input"
              }
              class="flex h-8 w-8 shrink-0 items-center justify-center rounded-md text-ink-muted hover:bg-accent hover:text-accent-ink disabled:opacity-30"
            >
              <Icon name="send" class="h-4 w-4" />
            </button>
          </div>
        </div>
      </div>
    </div>
  );
};

const Trajectory: Component<{
  steps: TimelineItem[];
  tokens?: { input: number; output: number };
  onViewDetails?: (id: string) => void;
}> = (props) => {
  const [expanded, setExpanded] = createSignal(false);
  const [expandedStep, setExpandedStep] = createSignal<number | null>(null);

  const stats = () => {
    let ms = 0;
    let count = 0;
    for (const s of props.steps) {
      if (s.type === "thinking" && s.thinking) {
        count++;
        ms += (s.thinking.endedAt ?? Date.now()) - s.thinking.startedAt;
      } else if (s.type === "tool") {
        count++;
      }
    }
    return { ms, count };
  };

  const hasTrajectory = () => stats().count > 0;

  const tokensLabel = () =>
    props.tokens ? `${formatTokens(props.tokens.input)} → ${formatTokens(props.tokens.output)} tokens` : "";

  const summary = () => {
    const { ms, count } = stats();
    const parts = [`Trabalhou por ${formatDuration(ms)}`, `${count} passo${count === 1 ? "" : "s"}`];
    if (props.tokens) parts.push(tokensLabel());
    return parts.join(" · ");
  };

  return (
    <Show
      when={hasTrajectory()}
      fallback={
        <div class="mb-4">
          <div class="trajectory-rail flex flex-col gap-0.5">
            <TimelineSteps
              steps={props.steps}
              expandedStep={expandedStep()}
              onToggle={(i) => setExpandedStep(expandedStep() === i ? null : i)}
              isLive={false}
              onViewDetails={props.onViewDetails}
            />
          </div>
          <Show when={props.tokens}>
            <div class="mt-1 font-mono text-[11px] text-ink-faint">{tokensLabel()}</div>
          </Show>
        </div>
      }
    >
      <div class="mb-4">
        <button
          onClick={() => setExpanded((v) => !v)}
          class="flex items-center gap-1.5 rounded px-1 py-0.5 text-[11px] text-ink-faint hover:text-ink-muted"
        >
          <Icon
            name="chevron-right"
            class={`h-3 w-3 shrink-0 transition-transform duration-120 ${expanded() ? "rotate-90" : ""}`}
          />
          <span>{summary()}</span>
        </button>
        <div class={`trajectory-collapse ${expanded() ? "is-open" : ""}`}>
          <div>
            <div class="trajectory-rail mt-2 flex flex-col gap-0.5">
              <TimelineSteps
                steps={props.steps}
                expandedStep={expandedStep()}
                onToggle={(i) => setExpandedStep(expandedStep() === i ? null : i)}
                isLive={false}
                onViewDetails={props.onViewDetails}
              />
            </div>
          </div>
        </div>
      </div>
    </Show>
  );
};

const TimelineSteps: Component<{
  steps: TimelineItem[];
  expandedStep: number | null;
  onToggle: (index: number) => void;
  isLive: boolean;
  onViewDetails?: (id: string) => void;
}> = (props) => {
  return (
    <For each={props.steps}>
      {(step, i) => (
        <>
          <Show when={step.type === "phase" && step.phase}>
            <PhaseRow phase={step.phase!} />
          </Show>
          <Show when={step.type === "phase_result" && step.phaseResult}>
            <PhaseResultRow phaseResult={step.phaseResult!} />
          </Show>
          <Show when={step.type === "text" && step.text}>
            <TextRow text={step.text!} />
          </Show>
          <Show when={step.type === "thinking" && step.thinking}>
            <ThinkingRow
              thinking={step.thinking!}
              isLive={props.isLive}
              isLast={i() === props.steps.length - 1}
              isExpanded={props.expandedStep === i()}
              onToggle={() => props.onToggle(i())}
            />
          </Show>
          <Show when={step.type === "tool" && step.tool}>
            <ToolRow
              tool={step.tool!}
              isExpanded={props.expandedStep === i()}
              onToggle={() => props.onToggle(i())}
            />
          </Show>
          <Show when={step.type === "steering" && step.steering}>
            <div class="my-1 ml-6 flex items-center gap-1.5">
              <span class="inline-flex items-center gap-1 rounded-full bg-accent/10 px-2 py-0.5 text-[11px] text-accent">
                <span class="h-1.5 w-1.5 rounded-full bg-accent" />
                {step.steering!.text.length > 50
                  ? `${step.steering!.text.slice(0, 50)}…`
                  : step.steering!.text}
              </span>
              <span class="text-[10px] text-ink-faint">orientação</span>
            </div>
          </Show>
          <Show when={step.type === "subagent" && step.subagent}>
            <SubagentRow subagent={step.subagent!} onViewDetails={props.onViewDetails} />
          </Show>
        </>
      )}
    </For>
  );
};

const SubagentRow: Component<{
  subagent: SubagentTimelineState;
  onViewDetails?: (id: string) => void;
}> = (props) => {
  const badgeClass = () => {
    switch (props.subagent.status) {
      case "running": return "bg-accent/15 text-accent";
      case "completed": return "bg-success/15 text-success";
      case "failed": return "bg-danger/15 text-danger";
      case "interrupted": return "bg-amber-500/15 text-amber-500";
      case "max_rounds": return "bg-amber-500/15 text-amber-500";
    }
  };

  const statusLabel = () => {
    switch (props.subagent.status) {
      case "running": return "Trabalhando";
      case "completed": return `${props.subagent.rounds} rounds`;
      case "failed": return "Falhou";
      case "interrupted": return "Interrompido";
      case "max_rounds": return "Limite de rounds";
    }
  };

  return (
    <div class="my-2 ml-4 border-l-2 border-accent/30 pl-2">
      <button
        onClick={() => props.onViewDetails?.(props.subagent.id)}
        class="flex w-full items-center gap-2 rounded px-1 py-0.5 text-xs hover:bg-surface-2"
      >
        <span class="trajectory-node flex h-4 w-4 shrink-0 items-center justify-center">
          <Icon name="layers" class="h-[11px] w-[11px] text-accent" />
        </span>
        <span class="font-mono text-[12px] text-ink-muted">{props.subagent.name}</span>
        <span class={`rounded px-1 py-0.5 text-[10px] font-medium ${badgeClass()}`}>
          {props.subagent.mode}
        </span>
        <span class="text-[11px] text-ink-faint">{statusLabel()}</span>
        <Show when={props.subagent.status === "running"}>
          <span class="inline-block h-2 w-2 animate-pulse-soft rounded-full bg-accent" />
        </Show>
        <div class="ml-auto flex items-center gap-2">
          <Show when={props.subagent.inputTokens > 0}>
            <span class="font-mono text-[10px] text-ink-faint">
              {formatTokens(props.subagent.inputTokens)}→{formatTokens(props.subagent.outputTokens)}
            </span>
          </Show>
          <Icon name="external-link" class="h-3 w-3 text-ink-faint" />
        </div>
      </button>
    </div>
  );
};

const SubagentModal: Component<{
  subagent: SubagentTimelineState;
  onClose: () => void;
}> = (props) => {
  const [expandedStep, setExpandedStep] = createSignal<number | null>(null);

  onMount(() => {
    const onKey = (e: KeyboardEvent) => {
      if (e.key === "Escape") props.onClose();
    };
    document.addEventListener("keydown", onKey);
    onCleanup(() => document.removeEventListener("keydown", onKey));
  });

  const badgeClass = () => {
    switch (props.subagent.status) {
      case "running": return "bg-accent/15 text-accent";
      case "completed": return "bg-success/15 text-success";
      case "failed": return "bg-danger/15 text-danger";
      case "interrupted": return "bg-amber-500/15 text-amber-500";
      case "max_rounds": return "bg-amber-500/15 text-amber-500";
    }
  };

  const statusLabel = () => {
    switch (props.subagent.status) {
      case "running": return "Trabalhando";
      case "completed": return `${props.subagent.rounds} rounds`;
      case "failed": return "Falhou";
      case "interrupted": return "Interrompido";
      case "max_rounds": return "Limite de rounds";
    }
  };

  return (
    <div
      class="fixed inset-0 z-50 flex items-center justify-center bg-black/40"
      onClick={(e) => { if (e.target === e.currentTarget) props.onClose(); }}
    >
      <div class="flex max-h-[85vh] w-full max-w-3xl flex-col rounded-xl bg-surface-0 shadow-2xl">
        <div class="flex items-center justify-between border-b border-border-subtle px-5 py-3">
          <div class="flex items-center gap-2">
            <span class="font-mono text-[14px] font-semibold text-ink">{props.subagent.name}</span>
            <span class={`rounded px-1.5 py-0.5 text-[10px] font-medium ${badgeClass()}`}>
              {props.subagent.mode}
            </span>
            <span class="text-[12px] text-ink-faint">{statusLabel()}</span>
          </div>
          <button
            onClick={props.onClose}
            class="flex h-7 w-7 items-center justify-center rounded-md text-ink-muted hover:bg-surface-2"
          >
            <Icon name="x" class="h-4 w-4" />
          </button>
        </div>
        <div class="overflow-y-auto px-5 py-3">
          <TimelineSteps
            steps={props.subagent.steps}
            expandedStep={expandedStep()}
            onToggle={(i) => setExpandedStep(expandedStep() === i ? null : i)}
            isLive={props.subagent.status === "running"}
          />
        </div>
      </div>
    </div>
  );
};

const PhaseRow: Component<{ phase: Phase }> = (props) => {
  return (
    <div class="mt-3 flex h-7 items-center gap-2 px-1 first:mt-0">
      <span class="trajectory-node flex h-5 w-5 shrink-0 items-center justify-center">
        <Icon name="layers" class="h-[13px] w-[13px] text-accent" />
      </span>
      <span class="text-[10px] font-semibold uppercase tracking-wider text-accent">
        {PHASE_LABEL[props.phase]}
      </span>
      <div class="h-px flex-1 bg-border-subtle" />
    </div>
  );
};

const PhaseResultRow: Component<{ phaseResult: { phase: Phase; text: string } }> = (props) => {
  return (
    <div class="my-1 ml-6">
      <div
        class="prose-content text-[12px] leading-[1.6] text-ink-muted"
        innerHTML={marked.parse(props.phaseResult.text, { async: false }) as string}
      />
    </div>
  );
};

const TextRow: Component<{ text: string }> = (props) => {
  return (
    <div class="my-1 ml-6">
      <div
        class="prose-content text-[12px] leading-[1.6] text-ink-muted"
        innerHTML={marked.parse(props.text, { async: false }) as string}
      />
    </div>
  );
};

const ThinkingRow: Component<{
  thinking: { text: string; startedAt: number; endedAt?: number };
  isLive: boolean;
  isLast: boolean;
  isExpanded: boolean;
  onToggle: () => void;
}> = (props) => {
  const duration = () => {
    if (props.thinking.endedAt) {
      return formatDuration(props.thinking.endedAt - props.thinking.startedAt);
    }
    return formatDuration(Date.now() - props.thinking.startedAt);
  };

  const showText = () => (props.isLive && props.isLast) || props.isExpanded;

  return (
    <div>
      <button
        onClick={props.onToggle}
        class="flex h-7 w-full items-center gap-2 rounded px-1 text-xs hover:bg-surface-2"
      >
        <span class="trajectory-node flex h-5 w-5 shrink-0 items-center justify-center">
          <Icon name="brain" class="h-[14px] w-[14px] text-accent" />
        </span>
        <span class="text-[12px] text-ink-muted">Pensou</span>
        <span class="ml-auto font-mono text-[11px] text-ink-faint">{duration()}</span>
      </button>
      <Show when={showText()}>
        <div class="ml-6 rounded-md bg-surface-1 p-2">
          <p class="whitespace-pre-wrap break-words text-[12px] leading-[1.6] text-ink-muted">
            {props.thinking.text}
            <Show when={props.isLive && props.isLast}>
              <span class="stream-cursor" />
            </Show>
          </p>
        </div>
      </Show>
    </div>
  );
};

const ToolRow: Component<{
  tool: { call: ToolCallData; result?: ToolResultData; status: string };
  isExpanded: boolean;
  onToggle: () => void;
}> = (props) => {
  const icon = () => toolIcon(props.tool.call.toolName) as IconName;
  const label = () => props.tool.call.toolName;
  const summary = () => summarizeArgs(props.tool.call.args);

  const statusIcon = () => {
    if (props.tool.status === "running") return "loader";
    if (props.tool.status === "error") return "x";
    return "check";
  };

  const statusClass = () => {
    if (props.tool.status === "running") return "text-accent animate-spin-slow";
    if (props.tool.status === "error") return "text-danger";
    return "text-success";
  };

  return (
    <div>
      <button
        onClick={props.onToggle}
        class="flex h-7 w-full items-center gap-2 rounded px-1 text-xs hover:bg-surface-2"
      >
        <span class="trajectory-node flex h-5 w-5 shrink-0 items-center justify-center">
          <Icon name={icon()} class="h-[14px] w-[14px] text-ink-muted" />
        </span>
        <span class="font-mono text-[12px] text-ink-muted">{label()}</span>
        <span class="truncate text-[12px] text-ink-faint">{summary()}</span>
        <div class="ml-auto flex items-center gap-1">
          <Icon name={statusIcon() as IconName} class={`h-3 w-3 ${statusClass()}`} />
          <Icon
            name="chevron-right"
            class={`h-3 w-3 text-ink-faint transition-transform duration-120 ${props.isExpanded ? "rotate-90" : ""}`}
          />
        </div>
      </button>
      <Show when={props.isExpanded}>
        <div class="ml-6 rounded-md bg-surface-1 p-2 text-xs">
          <div class="mb-1 font-mono text-[11px] font-medium text-ink-muted">Argumentos</div>
          <pre class="mb-2 overflow-x-auto whitespace-pre-wrap font-mono text-[11px] text-ink-faint">
            {JSON.stringify(props.tool.call.args, null, 2)}
          </pre>
          <Show when={props.tool.result}>
            <div class="mb-1 font-mono text-[11px] font-medium text-ink-muted">Resultado</div>
            <pre class="max-h-48 overflow-y-auto whitespace-pre-wrap break-all font-mono text-[11px] text-ink-faint">
              {(props.tool.result!.error ?? props.tool.result!.output).slice(0, 5000)}
            </pre>
          </Show>
        </div>
      </Show>
    </div>
  );
};

interface QuestionDraft {
  picks: number[];
  otherSelected: boolean;
  otherText: string;
}

const QuestionCard: Component<{
  ask: AskUserData;
  onSubmit: (answers: UserAnswer[]) => void;
}> = (props) => {
  const [drafts, setDrafts] = createSignal<QuestionDraft[]>(
    props.ask.questions.map(() => ({ picks: [], otherSelected: false, otherText: "" })),
  );

  const updateDraft = (qi: number, patch: Partial<QuestionDraft>) => {
    setDrafts((prev) => prev.map((d, i) => (i === qi ? { ...d, ...patch } : d)));
  };

  const pickOption = (qi: number, oi: number, multi: boolean) => {
    const d = drafts()[qi];
    if (multi) {
      const picks = d.picks.includes(oi) ? d.picks.filter((p) => p !== oi) : [...d.picks, oi];
      updateDraft(qi, { picks });
    } else {
      updateDraft(qi, { picks: [oi], otherSelected: false });
    }
  };

  const pickOther = (qi: number, multi: boolean) => {
    const d = drafts()[qi];
    if (multi) {
      updateDraft(qi, { otherSelected: !d.otherSelected });
    } else {
      updateDraft(qi, { picks: [], otherSelected: true });
    }
  };

  const answered = (d: QuestionDraft) =>
    d.picks.length > 0 || (d.otherSelected && d.otherText.trim().length > 0);

  const allAnswered = () => drafts().every(answered);

  const submit = () => {
    if (!allAnswered()) return;
    const answers: UserAnswer[] = props.ask.questions.map((q, qi) => {
      const d = drafts()[qi];
      const parts = d.picks.map((oi) => q.options[oi]);
      if (d.otherSelected && d.otherText.trim()) parts.push(d.otherText.trim());
      return { question: q.question, answer: parts.join(", ") };
    });
    props.onSubmit(answers);
  };

  return (
    <div class="rounded-lg border border-accent/50 bg-surface-1 p-3">
      <div class="mb-3 flex items-center gap-2">
        <span class="rounded bg-accent/15 px-1.5 py-0.5 text-[10px] font-semibold text-accent">
          O agente precisa da sua resposta
        </span>
      </div>

      <For each={props.ask.questions}>
        {(q, qi) => {
          const multi = () => q.multi_select === true;
          const draft = () => drafts()[qi()];
          return (
            <div class="mb-4 last:mb-3">
              <p class="mb-2 text-[13px] font-medium leading-[1.5] text-ink">{q.question}</p>
              <div class="flex flex-col gap-1">
                <For each={q.options}>
                  {(opt, oi) => (
                    <button
                      onClick={() => pickOption(qi(), oi(), multi())}
                      class={`flex items-center gap-2 rounded-md border px-3 py-1.5 text-left text-[13px] transition-colors ${
                        draft().picks.includes(oi())
                          ? "border-accent bg-accent/10 text-ink"
                          : "border-border-subtle bg-surface-0 text-ink-muted hover:border-accent/40"
                      }`}
                    >
                      <span
                        class={`flex h-3.5 w-3.5 shrink-0 items-center justify-center border ${
                          multi() ? "rounded-sm" : "rounded-full"
                        } ${draft().picks.includes(oi()) ? "border-accent bg-accent" : "border-ink-faint"}`}
                      >
                        <Show when={draft().picks.includes(oi())}>
                          <Icon name="check" class="h-2.5 w-2.5 text-accent-ink" />
                        </Show>
                      </span>
                      {opt}
                    </button>
                  )}
                </For>

                <button
                  onClick={() => pickOther(qi(), multi())}
                  class={`flex items-center gap-2 rounded-md border px-3 py-1.5 text-left text-[13px] transition-colors ${
                    draft().otherSelected
                      ? "border-accent bg-accent/10 text-ink"
                      : "border-border-subtle bg-surface-0 text-ink-muted hover:border-accent/40"
                  }`}
                >
                  <span
                    class={`flex h-3.5 w-3.5 shrink-0 items-center justify-center border ${
                      multi() ? "rounded-sm" : "rounded-full"
                    } ${draft().otherSelected ? "border-accent bg-accent" : "border-ink-faint"}`}
                  >
                    <Show when={draft().otherSelected}>
                      <Icon name="check" class="h-2.5 w-2.5 text-accent-ink" />
                    </Show>
                  </span>
                  Outra resposta…
                </button>

                <Show when={draft().otherSelected}>
                  <input
                    type="text"
                    value={draft().otherText}
                    onInput={(e) => updateDraft(qi(), { otherText: e.currentTarget.value })}
                    onKeyDown={(e) => {
                      if (e.key === "Enter") submit();
                    }}
                    placeholder="Digite sua resposta…"
                    class="mt-1 rounded-md border border-border-subtle bg-surface-0 px-3 py-1.5 text-[13px] text-ink placeholder:text-ink-faint focus:border-accent/60 focus:outline-none"
                  />
                </Show>
              </div>
            </div>
          );
        }}
      </For>

      <button
        onClick={submit}
        disabled={!allAnswered()}
        class="flex w-full items-center justify-center gap-1.5 rounded-md bg-accent px-3 py-1.5 text-sm font-medium text-accent-ink hover:bg-accent-hover disabled:opacity-30"
      >
        <Icon name="send" class="h-4 w-4" />
        Responder
      </button>
    </div>
  );
};

const ApprovalCard: Component<{
  toolCall: ToolCallData;
  onApprove: () => void;
  onReject: () => void;
}> = (props) => {
  const proposal = () => props.toolCall.editProposal as EditProposalData | undefined;
  const isBash = () => props.toolCall.toolName === "bash";

  // The chat input is disabled while an approval is pending, so a global
  // listener is safe: Enter approves, Esc rejects.
  const onKey = (e: KeyboardEvent) => {
    if (e.key === "Enter") {
      e.preventDefault();
      props.onApprove();
    } else if (e.key === "Escape") {
      e.preventDefault();
      props.onReject();
    }
  };
  onMount(() => document.addEventListener("keydown", onKey));
  onCleanup(() => document.removeEventListener("keydown", onKey));

  const detectLanguage = (path: string): string => {
    if (path.endsWith(".ts") || path.endsWith(".tsx")) return "typescript";
    if (path.endsWith(".rs")) return "rust";
    if (path.endsWith(".py")) return "python";
    if (path.endsWith(".swift")) return "swift";
    if (path.endsWith(".js") || path.endsWith(".jsx")) return "javascript";
    if (path.endsWith(".css")) return "css";
    if (path.endsWith(".json")) return "json";
    if (path.endsWith(".html")) return "html";
    return "plaintext";
  };

  return (
    <div class="rounded-lg border border-accent/50 bg-surface-1 p-3">
      <div class="mb-2 flex items-center justify-between">
        <div class="flex items-center gap-2">
          <Show
            when={isBash()}
            fallback={
              <>
                <span class="rounded bg-accent/15 px-1.5 py-0.5 text-[10px] font-semibold text-accent">
                  Edição proposta
                </span>
                <span class="truncate font-mono text-[12px] text-ink-muted">
                  {proposal()?.path ?? (props.toolCall.args.path as string)}
                </span>
              </>
            }
          >
            <span class="rounded bg-amber-500/15 px-1.5 py-0.5 text-[10px] font-semibold text-amber-500">
              Comando bash
            </span>
          </Show>
        </div>
      </div>

      <Show when={isBash()}>
        <div class="mb-3 rounded-md border border-border-subtle bg-surface-0">
          <div class="flex items-center gap-1.5 border-b border-border-subtle px-3 py-1.5">
            <Icon name="terminal" class="h-3.5 w-3.5 text-ink-muted" />
            <span class="text-[11px] font-medium text-ink-muted">$</span>
          </div>
          <pre class="overflow-x-auto p-3 font-mono text-[13px] leading-relaxed text-ink">
            {String(props.toolCall.args.command ?? "")}
          </pre>
        </div>
      </Show>

      <Show when={!isBash() && proposal()}>
        {(p) => (
          <div class="mb-3 h-56 overflow-hidden rounded border border-border-subtle">
            <DiffViewer
              original={p().oldString}
              modified={p().newString}
              language={detectLanguage(p().path)}
            />
          </div>
        )}
      </Show>

      <Show when={!isBash() && !proposal()}>
        <pre class="mb-3 max-h-32 overflow-auto rounded bg-surface-0 p-2 font-mono text-[12px] text-ink-faint">
          {JSON.stringify(props.toolCall.args, null, 2)}
        </pre>
      </Show>

      <div class="flex gap-2">
        <button
          onClick={props.onApprove}
          class="flex flex-1 items-center justify-center gap-1.5 rounded-md bg-accent px-3 py-1.5 text-sm font-medium text-accent-ink hover:bg-accent-hover"
        >
          <Icon name="check" class="h-4 w-4" />
          Aprovar
          <kbd class="rounded bg-accent-ink/15 px-1 font-mono text-[10px]">⏎</kbd>
        </button>
        <button
          onClick={props.onReject}
          class="flex flex-1 items-center justify-center gap-1.5 rounded-md border border-border-subtle bg-surface-0 px-3 py-1.5 text-sm text-ink hover:border-danger hover:text-danger"
        >
          <Icon name="x" class="h-4 w-4" />
          Rejeitar
          <kbd class="rounded bg-surface-2 px-1 font-mono text-[10px]">esc</kbd>
        </button>
      </div>
    </div>
  );
};
