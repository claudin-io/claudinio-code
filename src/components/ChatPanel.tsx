import { createSignal, For, Show, type Component } from "solid-js";
import { sendMessage, approveTool, rejectTool, type AgentEvent, type ToolCallData, type EditProposalData, type DoneData, type ChatStep, type ToolResultData } from "../lib/ipc";
import { DiffViewer } from "./DiffViewer";

type Status = "idle" | "thinking" | "awaiting_approval" | "done" | "error";

interface ChatMessage {
  role: "user" | "assistant" | "system";
  text: string;
  steps?: ChatStep[];
  done?: DoneData;
}

export const ChatPanel: Component = () => {
  const [input, setInput] = createSignal("");
  const [messages, setMessages] = createSignal<ChatMessage[]>([]);
  const [status, setStatus] = createSignal<Status>("idle");
  const [currentToolCall, setCurrentToolCall] = createSignal<ToolCallData | null>(null);
  const [currentSteps, setCurrentSteps] = createSignal<ChatStep[]>([]);

  let messagesEndRef: HTMLDivElement | undefined;

  const scrollToBottom = () => {
    setTimeout(() => messagesEndRef?.scrollIntoView({ behavior: "smooth" }), 50);
  };

  const handleEvent = (event: AgentEvent) => {
    if (event.event === "Thinking") {
      if (!event.data) return;
      setCurrentSteps((prev) => {
        const last = prev[prev.length - 1];
        if (last?.type === "thinking") {
          return [...prev.slice(0, -1), { type: "thinking", text: event.data }];
        }
        return [...prev, { type: "thinking", text: event.data }];
      });
      scrollToBottom();
    } else if (event.event === "ToolCall") {
      const data = event.data as ToolCallData;
      setCurrentSteps((prev) => [...prev, { type: "tool_call", data }]);
      if (data.permission === "requires_approval" && data.editProposal) {
        setStatus("awaiting_approval");
        setCurrentToolCall(data);
      }
      scrollToBottom();
    } else if (event.event === "ToolResult") {
      const data = event.data as ToolResultData;
      setCurrentSteps((prev) => [...prev, { type: "tool_result", data }]);
      scrollToBottom();
    } else if (event.event === "Done") {
      const data = event.data as DoneData;
      const steps = currentSteps();
      setMessages((prev) => [
        ...prev,
        {
          role: "assistant",
          text: data.text_output,
          steps,
          done: data,
        },
      ]);
      setCurrentSteps([]);
      setStatus("done");
      scrollToBottom();
    } else if (event.event === "Error") {
      setMessages((prev) => [
        ...prev,
        { role: "system", text: `Error: ${event.data}` },
      ]);
      setCurrentSteps([]);
      setStatus("error");
    }
  };

  const send = async () => {
    const text = input().trim();
    if (!text || status() === "awaiting_approval") return;

    setMessages((prev) => [...prev, { role: "user", text }]);
    setInput("");
    setCurrentSteps([]);
    setStatus("thinking");

    try {
      await sendMessage(text, handleEvent);
    } catch (e) {
      setMessages((prev) => [
        ...prev,
        { role: "system", text: `Failed to send: ${String(e)}` },
      ]);
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
      setMessages((prev) => [
        ...prev,
        { role: "system", text: `Approval failed: ${String(e)}` },
      ]);
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
      setMessages((prev) => [
        ...prev,
        { role: "system", text: `Reject failed: ${String(e)}` },
      ]);
    }
  };

  const handleKeyDown = (e: KeyboardEvent) => {
    if (e.key === "Enter" && !e.shiftKey) {
      e.preventDefault();
      send();
    }
  };

  return (
    <div class="flex h-full flex-col bg-surface-1">
      <div class="flex items-center justify-between border-b border-border-subtle px-3 py-1.5">
        <span class="text-xs font-semibold uppercase tracking-wide text-ink-muted">
          Agente
        </span>
        <span class="rounded bg-surface-2 px-1.5 py-0.5 text-[10px] text-ink-muted">
          {status()}
        </span>
      </div>

      <div class="flex flex-1 flex-col gap-2 overflow-y-auto p-2">
        <For each={messages()}>
          {(msg) => (
            <ChatBubble message={msg} />
          )}
        </For>

        <Show when={status() === "thinking" || status() === "done"}>
          <StepsList steps={currentSteps()} isLive={status() === "thinking"} />
        </Show>

        <Show when={currentToolCall() && status() === "awaiting_approval"}>
          <ApprovalCard
            toolCall={currentToolCall()!}
            onApprove={handleApprove}
            onReject={handleReject}
          />
        </Show>

        <div ref={messagesEndRef} />
      </div>

      <div class="border-t border-border-subtle p-2">
        <div class="flex gap-2">
          <textarea
            value={input()}
            onInput={(e) => setInput(e.currentTarget.value)}
            onKeyDown={handleKeyDown}
            disabled={status() === "thinking" || status() === "awaiting_approval"}
            placeholder={
              status() === "awaiting_approval"
                ? "Aprove ou rejeite a edição primeiro…"
                : "Pergunte algo sobre o código…"
            }
            class="min-h-[36px] flex-1 resize-none rounded-md border border-border-subtle bg-surface-2 p-2 text-sm text-ink placeholder:text-ink-muted focus:outline-none focus:border-accent disabled:opacity-50"
            rows={2}
          />
          <button
            onClick={send}
            disabled={!input().trim() || status() === "thinking" || status() === "awaiting_approval"}
            class="self-end rounded-md bg-accent px-3 py-1.5 text-sm font-medium text-white hover:opacity-90 disabled:opacity-40"
          >
            Enviar
          </button>
        </div>
      </div>
    </div>
  );
};

const StepsList: Component<{ steps: ChatStep[]; isLive: boolean }> = (props) => {
  return (
    <div class="flex flex-col gap-1.5">
      <For each={props.steps}>
        {(step, i) => <StepItem step={step} index={i()} total={props.steps.length} isLive={props.isLive} />}
      </For>
    </div>
  );
};

const StepItem: Component<{ step: ChatStep; index?: number; total?: number; isLive?: boolean }> = (props) => {
  if (props.step.type === "thinking") {
    const showCursor = props.isLive && props.index === props.total - 1;
    return (
      <div class="rounded-lg bg-surface-2 p-3 text-sm text-ink">
        <div class="mb-1 text-[10px] font-semibold uppercase tracking-wide text-accent">
          Pensando
        </div>
        <p class="whitespace-pre-wrap break-words text-sm leading-relaxed">
          {props.step.text}
          {showCursor ? <span class="animate-pulse">▊</span> : null}
        </p>
      </div>
    );
  }

  if (props.step.type === "tool_call") {
    const tc = props.step.data;
    const argsPreview = JSON.stringify(tc.args).slice(0, 300);
    return (
      <div class="rounded-lg border border-border-subtle bg-surface-0 p-2">
        <div class="flex items-center gap-2">
          <span class="rounded bg-accent/15 px-1.5 py-0.5 text-[10px] font-mono text-accent">
            {tc.toolName}
          </span>
          <span class="truncate text-xs text-ink-muted">
            {tc.toolName === "read_file" && (tc.args.path as string)
              ? tc.args.path as string
              : tc.toolName === "list_dir" && (tc.args.path as string)
                ? tc.args.path as string
                : tc.toolName === "grep" && (tc.args.pattern as string)
                  ? `/${tc.args.pattern}/`
                  : tc.toolName === "edit_file" && (tc.args.path as string)
                    ? tc.args.path as string
                    : ""}
          </span>
        </div>
        <Show when={tc.permission === "auto"}>
          <div class="mt-1 text-xs text-ink-muted break-all font-mono">
            {argsPreview}
            {argsPreview.length >= 300 ? "…" : ""}
          </div>
        </Show>
      </div>
    );
  }

  if (props.step.type === "tool_result") {
    const tr = props.step.data;
    const output = tr.error ?? tr.output;
    const isError = !!tr.error;
    return (
      <div class="rounded-lg border border-border-subtle bg-surface-0 p-2">
        <div class="flex items-center gap-2">
          <span
            class="rounded px-1.5 py-0.5 text-[10px] font-mono"
            classList={{
              "bg-green-900/30 text-green-400": !isError,
              "bg-red-900/30 text-red-400": isError,
            }}
          >
            {isError ? "erro" : "ok"}
          </span>
          <span class="truncate text-xs text-ink-muted">{tr.toolName}</span>
        </div>
        <div class="mt-1 max-h-24 overflow-y-auto text-xs text-ink-muted whitespace-pre-wrap break-all font-mono">
          {output?.slice(0, 1000)}
          {output && output.length > 1000 ? "…" : ""}
        </div>
      </div>
    );
  }

  return null;
};

const ChatBubble: Component<{ message: ChatMessage }> = (props) => {
  const isUser = () => props.message.role === "user";
  return (
    <div
      class="rounded-lg p-3 text-sm"
      classList={{
        "bg-accent/10 self-end max-w-[85%]": isUser(),
        "bg-surface-2 self-start w-full": !isUser(),
        "text-red-400": props.message.role === "system",
      }}
    >
      <Show when={!isUser() && props.message.role !== "system"}>
        <div class="mb-1 text-[10px] font-semibold uppercase tracking-wide text-ink-muted">
          {props.message.done ? "Resposta" : "Mensagem"}
        </div>
      </Show>
      <Show when={props.message.steps && props.message.steps.length > 0}>
        <div class="mb-3 flex flex-col gap-1.5">
          <For each={props.message.steps}>
            {(step) => <StepItem step={step} isLive={false} />}
          </For>
        </div>
      </Show>
      <p class="whitespace-pre-wrap break-words leading-relaxed">
        {props.message.text}
      </p>
      <Show when={props.message.done}>
        <div class="mt-2 flex gap-3 text-[10px] text-ink-muted">
          <span>tokens: {props.message.done!.input_tokens}→{props.message.done!.output_tokens}</span>
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
    <div class="rounded-lg border border-accent/40 bg-surface-2 p-3">
      <div class="mb-2 flex items-center justify-between">
        <div class="flex items-center gap-2">
          <span class="rounded bg-accent/20 px-1.5 py-0.5 text-[10px] font-semibold text-accent">
            edit_file
          </span>
          <span class="truncate text-xs text-ink-muted">
            {proposal()?.path ?? props.toolCall.args.path as string}
          </span>
        </div>
      </div>

      <Show when={proposal()}>
        {(p) => (
          <div class="mb-3 h-48 overflow-hidden rounded border border-border-subtle">
            <DiffViewer
              original={p().oldString}
              modified={p().newString}
              language={detectLanguage(p().path)}
            />
          </div>
        )}
      </Show>

      <Show when={!proposal()}>
        <pre class="mb-3 max-h-32 overflow-auto rounded bg-surface-0 p-2 text-xs text-ink-muted">
          {JSON.stringify(props.toolCall.args, null, 2)}
        </pre>
      </Show>

      <div class="flex gap-2">
        <button
          onClick={props.onApprove}
          class="flex-1 rounded-md bg-accent px-3 py-1.5 text-sm font-medium text-white hover:opacity-90"
        >
          Aprovar
        </button>
        <button
          onClick={props.onReject}
          class="flex-1 rounded-md border border-border-subtle bg-surface-0 px-3 py-1.5 text-sm text-ink hover:bg-surface-2"
        >
          Rejeitar
        </button>
      </div>
    </div>
  );
};
