import { createSignal, For, Show, type Component } from "solid-js";
import {
  sendMessage,
  approveTool,
  rejectTool,
  type AgentEvent,
  type ToolCallData,
  type EditProposalData,
  type DoneData,
  type ToolResultData,
} from "../lib/ipc";
import { marked } from "marked";
import { DiffViewer } from "./DiffViewer";
import { Icon, toolIcon, type IconName } from "./Icon";

type Status = "idle" | "thinking" | "awaiting_approval" | "done" | "error";

interface ChatMessage {
  role: "user" | "assistant";
  text: string;
  steps?: TimelineItem[];
  done?: DoneData;
}

interface TimelineItem {
  type: "thinking" | "tool";
  thinking?: { text: string; startedAt: number; endedAt?: number };
  tool?: {
    call: ToolCallData;
    result?: ToolResultData;
    status: "running" | "ok" | "error";
  };
}

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

export const ChatPanel: Component = () => {
  const [input, setInput] = createSignal("");
  const [messages, setMessages] = createSignal<ChatMessage[]>([]);
  const [status, setStatus] = createSignal<Status>("idle");
  const [currentToolCall, setCurrentToolCall] = createSignal<ToolCallData | null>(null);
  const [currentSteps, setCurrentSteps] = createSignal<TimelineItem[]>([]);
  const [thinkingStart, setThinkingStart] = createSignal(0);
  const [expandedStep, setExpandedStep] = createSignal<number | null>(null);

  let messagesEndRef: HTMLDivElement | undefined;
  let inputRef: HTMLTextAreaElement | undefined;

  const scrollToBottom = () => {
    setTimeout(() => messagesEndRef?.scrollIntoView({ behavior: "smooth" }), 50);
  };

  const addOrUpdateTool = (item: TimelineItem) => {
    setCurrentSteps((prev) => {
      const idx = prev.findIndex(
        (s) => s.type === "tool" && s.tool?.call.toolId === item.tool?.call.toolId,
      );
      if (idx >= 0) {
        const next = [...prev];
        next[idx] = item;
        return next;
      }
      return [...prev, item];
    });
  };

  const applyToolResult = (data: ToolResultData) => {
    setCurrentSteps((prev) => {
      const idx = prev.findIndex(
        (s) => s.type === "tool" && s.tool?.call.toolId === data.toolId,
      );
      if (idx === -1) return prev;
      const next = [...prev];
      const t = next[idx];
      if (t.type !== "tool" || !t.tool) return prev;
      next[idx] = {
        type: "tool",
        tool: { ...t.tool, result: data, status: data.error ? "error" : "ok" },
      };
      return next;
    });
  };

  const handleEvent = (event: AgentEvent) => {
    if (event.event === "Thinking") {
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
      addOrUpdateTool({
        type: "tool",
        tool: { call: data, status: "running" },
      });
      if (data.permission === "requires_approval" && data.editProposal) {
        setStatus("awaiting_approval");
        setCurrentToolCall(data);
      }
      scrollToBottom();
    } else if (event.event === "ToolResult") {
      const data = event.data as ToolResultData;
      applyToolResult(data);
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
        { role: "assistant", text: data.text_output, steps: final, done: data },
      ]);
      setCurrentSteps([]);
      setThinkingStart(0);
      setStatus("done");
      scrollToBottom();
    } else if (event.event === "Error") {
      setMessages((prev) => [...prev, { role: "user", text: `Erro: ${event.data}` }]);
      setCurrentSteps([]);
      setThinkingStart(0);
      setStatus("error");
    }
  };

  const send = async () => {
    const text = input().trim();
    if (!text || status() === "awaiting_approval") return;

    setMessages((prev) => [...prev, { role: "user", text }]);
    setInput("");
    setCurrentSteps([]);
    setThinkingStart(0);
    setStatus("thinking");

    try {
      await sendMessage(text, handleEvent);
    } catch (e) {
      setMessages((prev) => [...prev, { role: "user", text: `Falha ao enviar: ${String(e)}` }]);
      setStatus("error");
    }
  };

  const handleApprove = async () => {
    const tc = currentToolCall();
    if (!tc) return;
    setStatus("thinking");
    setCurrentToolCall(null);
    try {
      await approveTool(tc.sessionId, tc.toolId);
    } catch (e) {
      setMessages((prev) => [...prev, { role: "user", text: `Aprovação falhou: ${String(e)}` }]);
    }
  };

  const handleReject = async () => {
    const tc = currentToolCall();
    if (!tc) return;
    setStatus("thinking");
    setCurrentToolCall(null);
    try {
      await rejectTool(tc.sessionId, tc.toolId);
    } catch (e) {
      setMessages((prev) => [...prev, { role: "user", text: `Rejeição falhou: ${String(e)}` }]);
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

  const statusLabel = () => {
    switch (status()) {
      case "thinking": return "Trabalhando";
      case "awaiting_approval": return "Aguardando aprovação";
      case "done": return "Pronto";
      case "error": return "Erro";
      default: return "Parado";
    }
  };

  const statusDot = (): string => {
    switch (status()) {
      case "thinking": return "bg-accent";
      case "done": return "bg-success";
      case "error": return "bg-danger";
      default: return "bg-ink-faint";
    }
  };

  return (
    <div class="flex h-full flex-col bg-surface-0">
      <div class="flex items-center justify-between border-b border-border-subtle px-6 py-1.5">
        <div class="flex items-center gap-2">
          <span class="text-[11px] font-semibold uppercase tracking-wider text-ink-faint">
            Agente
          </span>
          <span
            class={`inline-block h-[6px] w-[6px] rounded-full ${statusDot()}`}
            classList={{
              "animate-pulse-soft": status() === "thinking" || status() === "awaiting_approval",
            }}
          />
          <span class="text-[11px] text-ink-faint">{statusLabel()}</span>
        </div>
      </div>

      <div class="flex flex-1 flex-col overflow-y-auto">
        <div class="w-full px-6 py-4">
          <For each={messages()}>
            {(msg) => (
              <div class="mb-6">
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
                  <TimelineSteps
                    steps={msg.steps!}
                    expandedStep={expandedStep()}
                    onToggle={(i) => setExpandedStep(expandedStep() === i ? null : i)}
                    isLive={false}
                  />

                  <Show when={msg.text}>
                    <div class="mb-1 mt-4">
                      <span class="text-[11px] font-semibold uppercase tracking-wider text-ink-faint">
                        Resposta
                      </span>
                    </div>
                    <div
                      class="prose-content whitespace-pre-wrap break-words text-[13px] leading-[1.65] text-ink"
                      innerHTML={marked.parse(msg.text, { async: false }) as string}
                    />
                  </Show>

                  <Show when={msg.done}>
                    <div class="mt-2 font-mono text-[11px] text-ink-faint">
                      {formatTokens(msg.done!.input_tokens)} → {formatTokens(msg.done!.output_tokens)} tokens
                    </div>
                  </Show>
                </Show>
              </div>
            )}
          </For>

          <Show when={status() === "thinking" || status() === "done"}>
            <div class="mb-6">
              <TimelineSteps
                steps={currentSteps()}
                expandedStep={expandedStep()}
                onToggle={(i) => setExpandedStep(expandedStep() === i ? null : i)}
                isLive={status() === "thinking"}
              />
            </div>
          </Show>

          <Show when={currentToolCall() && status() === "awaiting_approval"}>
            <div class="mb-6">
              <ApprovalCard
                toolCall={currentToolCall()!}
                onApprove={handleApprove}
                onReject={handleReject}
              />
            </div>
          </Show>

          <div ref={messagesEndRef} />
        </div>
      </div>

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
              disabled={status() === "thinking" || status() === "awaiting_approval"}
              placeholder={
                status() === "awaiting_approval"
                  ? "Aprove ou rejeite a edição primeiro…"
                  : "Pergunte algo sobre o código…"
              }
              class="max-h-[156px] min-h-[36px] flex-1 resize-none border-0 bg-transparent p-1 text-[13px] text-ink placeholder:text-ink-faint focus:outline-none disabled:opacity-50"
              rows={1}
            />
            <button
              onClick={send}
              disabled={!input().trim() || status() === "thinking" || status() === "awaiting_approval"}
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

const TimelineSteps: Component<{
  steps: TimelineItem[];
  expandedStep: number | null;
  onToggle: (index: number) => void;
  isLive: boolean;
}> = (props) => {
  return (
    <div class="flex flex-col gap-0.5">
      <For each={props.steps}>
        {(step, i) => (
          <>
            <Show when={step.type === "thinking" && step.thinking}>
              <ThinkingRow
                thinking={step.thinking!}
                isLive={props.isLive}
                isLast={i() === props.steps.length - 1}
              />
            </Show>
            <Show when={step.type === "tool" && step.tool}>
              <ToolRow
                tool={step.tool!}
                isExpanded={props.expandedStep === i()}
                onToggle={() => props.onToggle(i())}
              />
            </Show>
          </>
        )}
      </For>
    </div>
  );
};

const ThinkingRow: Component<{
  thinking: { text: string; startedAt: number; endedAt?: number };
  isLive: boolean;
  isLast: boolean;
}> = (props) => {
  const duration = () => {
    if (props.thinking.endedAt) {
      return formatDuration(props.thinking.endedAt - props.thinking.startedAt);
    }
    return formatDuration(Date.now() - props.thinking.startedAt);
  };

  return (
    <div class="group flex items-start gap-2 py-0.5">
      <div class="mt-0.5 shrink-0 text-accent">
        <Icon name="brain" class="h-[14px] w-[14px]" />
      </div>
      <div class="min-w-0 flex-1">
        <div class="flex items-center gap-2">
          <span class="text-xs font-medium text-ink-muted">Pensando</span>
          <span class="font-mono text-[11px] text-ink-faint">{duration()}</span>
        </div>
        <Show when={props.isLive && props.isLast}>
          <p class="whitespace-pre-wrap break-words text-xs text-ink-muted">
            {props.thinking.text}
            <span class="stream-cursor" />
          </p>
        </Show>
      </div>
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
        <Icon name={icon()} class="h-[14px] w-[14px] shrink-0 text-ink-muted" />
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

const ApprovalCard: Component<{
  toolCall: ToolCallData;
  onApprove: () => void;
  onReject: () => void;
}> = (props) => {
  const proposal = () => props.toolCall.editProposal as EditProposalData | undefined;

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
          <span class="rounded bg-accent/15 px-1.5 py-0.5 text-[10px] font-semibold text-accent">
            Edição proposta
          </span>
          <span class="truncate font-mono text-[12px] text-ink-muted">
            {proposal()?.path ?? (props.toolCall.args.path as string)}
          </span>
        </div>
      </div>

      <Show when={proposal()}>
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

      <Show when={!proposal()}>
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
        </button>
        <button
          onClick={props.onReject}
          class="flex flex-1 items-center justify-center gap-1.5 rounded-md border border-border-subtle bg-surface-0 px-3 py-1.5 text-sm text-ink hover:border-danger hover:text-danger"
        >
          <Icon name="x" class="h-4 w-4" />
          Rejeitar
        </button>
      </div>
    </div>
  );
};
