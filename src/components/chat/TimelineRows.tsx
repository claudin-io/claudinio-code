// Leaf renderers for the chat timeline: one component per row type, plus the
// two modals they open. Split out of ChatPanel, which was carrying all of them
// in one file — the same split already used by components/settings/.
//
// These are presentational: each takes its data through props and holds at most
// local UI state (expanded, hovered). The timeline state itself lives in
// ChatPanel.

import { createEffect, createSignal, onCleanup, onMount, For, Show, type Component } from "solid-js";
import { Icon, type IconName } from "../Icon";
import { ProseContent } from "../ProseContent";
import { DiffViewer } from "../DiffViewer";
import { NetworkIndicator } from "../NetworkIndicator";
import { ToolBody } from "../tool-renderers/ToolBody";
import {
  alwaysShowsBody,
  detectLanguageFromPath,
  toolHeader,
  toolSummary,
} from "../tool-renderers/toolPresentation";
import { renderMarkdown } from "../../lib/markdown";
import {
  ellipsize,
  formatDuration,
  formatTokens,
  modeChangeLabel,
  PHASE_LABEL,
  SUBSTANTIAL_TEXT_CHARS,
  type ChatMessage,
  type SubagentTimelineState,
  type TimelineItem,
} from "../../lib/chatRecords";
import { openExternalUrl } from "../../lib/ipc";
import type {
  EditProposalData,
  Phase,
  ToolCallData,
  ToolResultData,
} from "../../lib/ipc";

export const ArchivedBlock: Component<{
  summary: string;
  messages: ChatMessage[];
}> = (props) => {
  const [expanded, setExpanded] = createSignal(false);

  return (
    <div class="mb-6 overflow-hidden rounded-lg border border-border-subtle bg-surface-1">
      <button
        onClick={() => setExpanded((v) => !v)}
        class="flex w-full items-center gap-2 px-3 py-2 text-left hover:bg-surface-2"
      >
        <Icon
          name="compress"
          class={`h-3.5 w-3.5 shrink-0 text-ink-faint transition-transform duration-120 ${expanded() ? "rotate-90" : ""}`}
        />
        <span class="text-[11px] font-semibold uppercase tracking-wider text-ink-faint">
          {"Compacted history"}
        </span>
        <div class="h-px flex-1 bg-border-subtle" />
        <span class="text-[11px] text-ink-faint">
          {`${String(props.messages.length)} messages`}
        </span>
        <Icon
          name="chevron-right"
          class={`h-3 w-3 text-ink-faint transition-transform duration-120 ${expanded() ? "rotate-90" : ""}`}
        />
      </button>

      <Show when={expanded()}>
        <div class="border-t border-border-subtle px-3 py-2">
          <div class="mb-3 text-[12px] leading-[1.6] text-ink-muted">
            {props.summary}
          </div>
          <div class="space-y-2 opacity-60">
            <For each={props.messages}>
              {(msg) => (
                <div class="rounded bg-surface-0 px-2 py-1.5">
                  <span class="mr-2 text-[10px] font-semibold uppercase tracking-wider text-ink-faint">
                    {msg.role === "user" ? "You" : "Agent"}
                  </span>
                  <span class="text-[12px] text-ink-muted">
                    {ellipsize(msg.text, 120)}
                  </span>
                </div>
              )}
            </For>
          </div>
        </div>
      </Show>
    </div>
  );
};

export const ContextFooter: Component<{
  contextTokens: number;
  maxTokens: number;
  cumulativeTokens: number;
  estimatedCost?: number;
  costInput?: number;
  costOutput?: number;
  costCacheRead?: number;
  isCompacting: boolean;
  onCompact: () => void;
  showCompact: boolean;
}> = (props) => {
  const hasBreakdown = () =>
    props.costInput !== undefined && props.costOutput !== undefined && props.costCacheRead !== undefined;
  const costTitle = () =>
    hasBreakdown()
      ? `in \$${props.costInput!.toFixed(4)} · out \$${props.costOutput!.toFixed(4)} · cache \$${props.costCacheRead!.toFixed(4)}`
      : "Session cumulative cost";
  const pct = () => Math.min((props.contextTokens / props.maxTokens) * 100, 100);
  const barColor = () => {
    if (pct() < 50) return "bg-success";
    if (pct() < 80) return "bg-[#d9a05b]";
    if (pct() < 95) return "bg-accent";
    return "bg-danger";
  };

  const formatTokens = (n: number) => {
    if (n < 1000) return `${n}`;
    return `${(n / 1000).toFixed(n < 10000 ? 1 : 0)}k`;
  };

  return (
    <div class="flex items-center gap-3 border-t border-border-subtle bg-surface-2 px-6 py-1.5">
      <div class="flex flex-1 items-center gap-2" title={"Context for next request"}>
        <div class="h-1.5 flex-1 overflow-hidden rounded-full bg-surface-0">
          <div
            class={`h-full rounded-full transition-[width] duration-300 ease-out ${barColor()}`}
            style={{ width: `${pct()}%` }}
          />
        </div>
        <span class="font-mono text-[11px] text-ink-faint whitespace-nowrap">
          {formatTokens(props.contextTokens)} / {formatTokens(props.maxTokens)}
        </span>
      </div>

      <Show when={props.cumulativeTokens > 0}>
        <span
          class="font-mono text-[11px] text-ink-faint whitespace-nowrap"
          title={"Session cumulative tokens"}
        >
          {`total: ${formatTokens(props.cumulativeTokens)}`}
        </span>
      </Show>

      <Show when={props.estimatedCost !== undefined}>
        <span class="font-mono text-[11px] text-ink-faint" title={costTitle()}>
          ${props.estimatedCost!.toFixed(4)}
        </span>
      </Show>

      <Show when={props.showCompact && !props.isCompacting}>
        <button
          onClick={props.onCompact}
          class="flex items-center gap-1 rounded px-2 py-0.5 text-[11px] text-ink-muted hover:bg-surface-3 hover:text-accent"
        >
          <Icon name="compress" class="h-3 w-3" />
          {"Compact"}
        </button>
      </Show>

      <Show when={props.isCompacting}>
        <span class="flex items-center gap-1 text-[11px] text-accent">
          <span class="inline-block h-2 w-2 animate-pulse-soft rounded-full bg-accent" />
          {"Compacting…"}
        </span>
      </Show>
    </div>
  );
};

export const Trajectory: Component<{
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
    const parts = [`Worked for ${formatDuration(ms)}`, `${String(count)} step${count === 1 ? "" : "s"}`];
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

export const TimelineSteps: Component<{
  steps: TimelineItem[];
  expandedStep: number | null;
  onToggle: (index: number) => void;
  isLive: boolean;
  onViewDetails?: (id: string) => void;
  /// Smoothed live text for the currently-streaming "Thoughts" block, main
  /// agent run only (not passed by the subagent-detail-panel call sites).
  liveThinkingDisplay?: () => string;
}> = (props) => {
  return (
    <For each={props.steps}>
      {(step, i) => (
        <>
          <Show when={step.type === "compaction" && step.compaction}>
            <CompactionRow compaction={step.compaction!} />
          </Show>
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
              liveText={i() === props.steps.length - 1 ? props.liveThinkingDisplay : undefined}
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
            <div class="my-1 ml-6 flex flex-col gap-1">
              <div class="flex items-center gap-1.5">
                <span class="inline-flex items-center gap-1 rounded-full bg-accent/10 px-2 py-0.5 text-[11px] text-accent">
                  <span class="h-1.5 w-1.5 rounded-full bg-accent" />
                  {ellipsize(step.steering!.text, 50)}
                </span>
                <span class="text-[10px] text-ink-faint">{"steering"}</span>
              </div>
              <Show when={step.steering!.attachments && step.steering!.attachments!.length > 0}>
                <div class="ml-6 flex flex-wrap gap-1">
                  <For each={step.steering!.attachments!}>
                    {(att) => (
                      <span class="inline-flex items-center gap-1 rounded-md border border-border-subtle bg-surface-1 px-2 py-0.5 text-[10px] text-ink-muted">
                        <Icon
                          name={(att.mediaType ?? "").startsWith("image/") ? "image" : "file-text"}
                          class="h-3 w-3 shrink-0"
                        />
                        <span class="max-w-[120px] truncate">{att.name}</span>
                        <span class="font-mono text-[10px] text-ink-faint">
                          {att.size > 1024 * 1024
                            ? `${(att.size / (1024 * 1024)).toFixed(1)} MB`
                            : att.size > 1024
                              ? `${(att.size / 1024).toFixed(0)} KB`
                              : `${att.size} B`}
                        </span>
                      </span>
                    )}
                  </For>
                </div>
              </Show>
            </div>
          </Show>
          <Show when={step.type === "subagent" && step.subagent}>
            <SubagentRow subagent={step.subagent!} onViewDetails={props.onViewDetails} />
          </Show>
          <Show when={step.type === "mode" && step.modeChange}>
            <div class="my-1 ml-6 flex items-center gap-1.5">
              <span class="inline-flex items-center gap-1.5 rounded-full bg-accent/10 px-2 py-0.5 text-[11px] text-accent">
                <Icon
                  name={step.modeChange!.mode === "brain" ? "thinking-face" : "construction-worker"}
                  class="h-3 w-3"
                />
                {modeChangeLabel(step.modeChange!)}
              </span>
            </div>
          </Show>
          <Show when={step.type === "golden" && step.golden}>
            <div class="my-1 ml-6 flex items-center gap-1.5">
              <span
                class="gold-outline inline-flex items-center gap-1.5 rounded-full px-2 py-0.5 text-[11px] text-amber-500"
                title={step.golden!.pending.join(", ")}
              >
                <Icon name="goal" class="h-3 w-3" />
                {step.golden!.maxCycles > 0
                  ? `Golden loop: cycle ${String(step.golden!.cycle)} of ${String(step.golden!.maxCycles)} — ${String(step.golden!.pending.length)} goal(s) pending`
                  : `Golden loop: cycle ${String(step.golden!.cycle)} — ${String(step.golden!.pending.length)} goal(s) pending`}
              </span>
            </div>
          </Show>
          <Show when={step.type === "linked" && step.linked}>
            <LinkedRow linked={step.linked!} />
          </Show>
        </>
      )}
    </For>
  );
};

/// Chain divider: the conversation continued in a new linked session (or, with
/// `docOnly`, the predecessor's handoff document). Indicator only — the
/// kickoff / handoff text stays stored in the session JSONL but is never
/// rendered in the thread.
export const LinkedRow: Component<{
  linked: NonNullable<TimelineItem["linked"]>;
}> = (props) => {
  const label = () => {
    if (props.linked.docOnly) return "Handoff document";
    switch (props.linked.reason) {
      case "plan_execution": return "Plan approved — continuing in a new Builder session";
      case "golden_flip": return "Golden loop — continuing in a fresh session";
      case "context_handoff": return "Context limit reached — continued in a fresh session";
      case "manual_builder": return "Continuing with Builder in a new session";
      default: return "Context limit reached — continued in a fresh session";
    }
  };
  return (
    <div class="my-2 flex items-center gap-2">
      <div class="h-px flex-1 bg-border-subtle" />
      <span class="inline-flex items-center gap-1.5 rounded-full bg-accent/10 px-2.5 py-0.5 text-[11px] text-accent">
        <Icon name="git-branch" class="h-3 w-3" />
        <span>{label()}</span>
      </span>
      <div class="h-px flex-1 bg-border-subtle" />
    </div>
  );
};

export const SubagentRow: Component<{
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
      case "running": return "Working";
      case "completed": return `${String(props.subagent.rounds)} rounds`;
      case "failed": return "Failed";
      case "interrupted": return "Interrupted";
      case "max_rounds": return "Max rounds";
    }
  };

  return (
    <div class="my-2 ml-4 border-l-2 border-accent/30 pl-2">
      <button
        onClick={() => props.onViewDetails?.(props.subagent.id)}
        class="flex w-full flex-col gap-0.5 rounded px-1 py-0.5 text-xs hover:bg-surface-2"
      >
        <div class="flex w-full items-center gap-2">
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
                <Show when={props.subagent.cost > 0}>
                  <span class="text-ink-faint">
                    {" · $"}
                    {props.subagent.cost < 0.01
                      ? props.subagent.cost.toFixed(6)
                      : props.subagent.cost < 1
                        ? props.subagent.cost.toFixed(4)
                        : props.subagent.cost.toFixed(2)}
                  </span>
                </Show>
              </span>
            </Show>
            <Icon name="external-link" class="h-3 w-3 text-ink-faint" />
          </div>
        </div>
        <Show when={props.subagent.goal}>
          <div class="ml-6 flex items-start gap-1">
            <span class="shrink-0 text-[10px] font-semibold uppercase tracking-wider text-ink-faint">{"Goal"}</span>
            <span class="truncate text-[11px] text-ink-muted">
              {ellipsize(props.subagent.goal, 80)}
            </span>
          </div>
        </Show>
        <Show when={props.subagent.report}>
          <div class="ml-6 flex items-start gap-1">
            <span class="shrink-0 text-[10px] font-semibold uppercase tracking-wider text-ink-faint">{"Report"}</span>
            <span class="truncate text-[11px] text-ink-muted">
              {ellipsize(props.subagent.report!, 120)}
            </span>
          </div>
        </Show>
      </button>
    </div>
  );
};

export const SubagentModal: Component<{
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
      case "running": return "Working";
      case "completed": return `${String(props.subagent.rounds)} rounds`;
      case "failed": return "Failed";
      case "interrupted": return "Interrupted";
      case "max_rounds": return "Max rounds";
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
        <div class="overflow-y-auto px-5 py-3 space-y-4">
          <Show when={props.subagent.goal}>
            <div class="rounded-md bg-surface-1 p-3">
              <span class="mb-1 block text-[10px] font-semibold uppercase tracking-wider text-ink-faint">{"Goal"}</span>
              <p class="whitespace-pre-wrap break-words font-mono text-[12px] leading-[1.6] text-ink-muted">
                {props.subagent.goal}
              </p>
            </div>
          </Show>

          <TimelineSteps
            steps={props.subagent.steps}
            expandedStep={expandedStep()}
            onToggle={(i) => setExpandedStep(expandedStep() === i ? null : i)}
            isLive={props.subagent.status === "running"}
          />

          <Show when={props.subagent.report}>
            <div class="rounded-md bg-surface-1 p-3">
              <span class="mb-1 block text-[10px] font-semibold uppercase tracking-wider text-ink-faint">{"Report"}</span>
              <ProseContent
                class="prose-content text-[12px] leading-[1.6] text-ink-muted"
                html={renderMarkdown(props.subagent.report!)}
                onClick={(e) => {
                  const a = (e.target as HTMLElement).closest("a[data-link-type]");
                  if (!a) return;
                  e.preventDefault();
                  const href = a.getAttribute("href")!;
                  const linkType = a.getAttribute("data-link-type")!;
                  if (linkType === "external") openExternalUrl(href);
                }}
              />
            </div>
          </Show>
        </div>
      </div>
    </div>
  );
};

export const PhaseRow: Component<{ phase: Phase }> = (props) => {
  return (
    <div class="mt-3 flex h-7 items-center gap-2 px-1 first:mt-0">
      <span class="trajectory-node flex h-5 w-5 shrink-0 items-center justify-center">
        <Icon name="layers" class="h-[13px] w-[13px] text-accent" />
      </span>
      <span class="text-[10px] font-semibold uppercase tracking-wider text-accent">
        {PHASE_LABEL(props.phase)}
      </span>
      <div class="h-px flex-1 bg-border-subtle" />
    </div>
  );
};

export const PhaseResultRow: Component<{ phaseResult: { phase: Phase; text: string } }> = (props) => {
  return (
    <div class="my-1 ml-6">
      <ProseContent
        class="prose-content text-[12px] leading-[1.6] text-ink-muted"
        html={renderMarkdown(props.phaseResult.text)}
        onClick={(e) => {
          const a = (e.target as HTMLElement).closest("a[data-link-type]");
          if (!a) return;
          e.preventDefault();
          const href = a.getAttribute("href")!;
          const linkType = a.getAttribute("data-link-type")!;
          if (linkType === "external") openExternalUrl(href);
        }}
      />
    </div>
  );
};

// Substantial intermediate text (a real explanation the model wrote before a
// tool call) must read like an answer, not like a dim progress note; only
// short one-liner status texts keep the muted style.
export const TextRow: Component<{ text: string }> = (props) => {
  const substantial = () => props.text.length >= SUBSTANTIAL_TEXT_CHARS;
  return (
    <div class="my-1 ml-6">
      <ProseContent
        class={
          substantial()
            ? "prose-content text-[13px] leading-[1.6] text-ink"
            : "prose-content text-[12px] leading-[1.6] text-ink-muted"
        }
        html={renderMarkdown(props.text)}
        onClick={(e) => {
          const a = (e.target as HTMLElement).closest("a[data-link-type]");
          if (!a) return;
          e.preventDefault();
          const href = a.getAttribute("href")!;
          const linkType = a.getAttribute("data-link-type")!;
          if (linkType === "external") openExternalUrl(href);
        }}
      />
    </div>
  );
};

export const CompactionRow: Component<{ compaction: { kind: "start" | "done" | "fail" | "handoff_start" | "handoff_fail"; args: string[] } }> = (props) => {
  const iconName = (): IconName => {
    if (props.compaction.kind === "start" || props.compaction.kind === "handoff_start") return "package-process" as IconName;
    if (props.compaction.kind === "done") return "package" as IconName;
    return "package-out-of-stock" as IconName;
  };

  const label = () => {
    switch (props.compaction.kind) {
      case "start": return `Context at ${props.compaction.args[0]}k / ${props.compaction.args[1]}k — compacting…`;
      case "done": return `Context compacted: ~${props.compaction.args[0]}k → ~${props.compaction.args[1]}k tokens.`;
      case "handoff_start": return `Context at ${props.compaction.args[0]}k / ${props.compaction.args[1]}k — writing handoff for a fresh session…`;
      case "handoff_fail": return `Handoff failed: ${props.compaction.args[0]} — falling back to compaction.`;
      default: return `Compaction failed: ${props.compaction.args[0]} — continuing with full context.`;
    }
  };

  const isActive = () => props.compaction.kind === "start" || props.compaction.kind === "handoff_start";
  const isFail = () => props.compaction.kind === "fail" || props.compaction.kind === "handoff_fail";

  const colorClass = () => {
    if (isActive()) return "text-accent";
    if (props.compaction.kind === "done") return "text-success";
    return "text-danger";
  };

  const isStroke = () => isActive() || isFail();

  return (
    <div class="my-2 ml-4 border-l-2 border-current pl-2" classList={{
      "border-accent/40": isActive(),
      "border-success/40": props.compaction.kind === "done",
      "border-danger/40": isFail(),
    }}>
      <div class="flex items-center gap-2 px-1 py-1 text-[12px]">
        <span class={`trajectory-node flex h-5 w-5 shrink-0 items-center justify-center ${colorClass()}`}>
          <Icon name={iconName()} class={`h-[14px] w-[14px] ${colorClass()}`} stroke={isStroke()} />
        </span>
        <span class="text-ink-muted">{label()}</span>
        <Show when={isActive()}>
          <span class="inline-block h-2 w-2 animate-pulse-soft rounded-full bg-accent" />
        </Show>
      </div>
    </div>
  );
};

const thinkingSvgSpinner = (
  <svg xmlns="http://www.w3.org/2000/svg" width="20" height="20" viewBox="0 0 24 24">
    <defs>
      <filter id="tbGlow">
        <feGaussianBlur in="SourceGraphic" result="y" stdDeviation="1" />
        <feColorMatrix in="y" result="z" values="1 0 0 0 0 0 1 0 0 0 0 0 1 0 0 0 0 0 18 -7" />
        <feBlend in="SourceGraphic" in2="z" />
      </filter>
    </defs>
    <g filter="url(#tbGlow)">
      <circle cx="5" cy="12" r="4" fill="currentColor">
        <animate attributeName="cx" calcMode="spline" dur="2s" keySplines=".36,.62,.43,.99;.79,0,.58,.57" repeatCount="indefinite" values="5;8;5" />
      </circle>
      <circle cx="19" cy="12" r="4" fill="currentColor">
        <animate attributeName="cx" calcMode="spline" dur="2s" keySplines=".36,.62,.43,.99;.79,0,.58,.57" repeatCount="indefinite" values="19;16;19" />
      </circle>
      <animateTransform attributeName="transform" dur="0.75s" repeatCount="indefinite" type="rotate" values="0 12 12;360 12 12" />
    </g>
  </svg>
);

export const ThinkingBar: Component<{
  text: () => string;
}> = (props) => {
  let tooltipRef: HTMLDivElement | undefined;
  const [hovered, setHovered] = createSignal(false);

  createEffect(() => {
    props.text(); // tracked: re-run as the streamed text grows
    const _hovered = hovered();
    if (_hovered && tooltipRef) {
      tooltipRef.scrollTop = tooltipRef.scrollHeight;
    }
  });

  return (
    <div
      class="thinking-bar-wrapper"
      onMouseEnter={() => setHovered(true)}
      onMouseLeave={() => setHovered(false)}
    >
      <div class="thinking-bar">
        <span class="thinking-bar-spinner">
          {thinkingSvgSpinner}
        </span>
        <span class="thinking-bar-label">
          {"Thinking"}
        </span>
        <span class="ml-auto flex items-center gap-3">
          <NetworkIndicator placement="top" />
        </span>
      </div>
      <div ref={tooltipRef} class="thinking-bar-tooltip">
        {props.text() || ""}
      </div>
    </div>
  );
};

export const ThinkingRow: Component<{
  thinking: { text: string; startedAt: number; endedAt?: number };
  isLive: boolean;
  isLast: boolean;
  isExpanded: boolean;
  onToggle: () => void;
  /// Smoothed live text, only used while this row is the live/last step.
  /// Undefined for subagent panels (no smoothing there) or once a newer
  /// step has been pushed after this one — falls back to the always-fully-
  /// accumulated `thinking.text`.
  liveText?: () => string;
}> = (props) => {
  const duration = () => {
    if (props.thinking.endedAt) {
      return formatDuration(props.thinking.endedAt - props.thinking.startedAt);
    }
    return formatDuration(Date.now() - props.thinking.startedAt);
  };

  const showText = () => (props.isLive && props.isLast) || props.isExpanded;
  const bodyText = () =>
    props.liveText && props.isLive && props.isLast ? props.liveText() : props.thinking.text;

  return (
    <div>
      <button
        onClick={props.onToggle}
        class="flex h-7 w-full items-center gap-2 rounded px-1 text-xs hover:bg-surface-2"
      >
        <span class="trajectory-node flex h-5 w-5 shrink-0 items-center justify-center">
          <Icon name="brain" class="h-[14px] w-[14px] text-accent" />
        </span>
        <span class="text-[12px] text-ink-muted">{"Thought"}</span>
        <span class="ml-auto font-mono text-[11px] text-ink-faint">{duration()}</span>
      </button>
      <Show when={showText()}>
        <div class="ml-6 rounded-md bg-surface-1 p-2 text-left">
          <p class="whitespace-pre-wrap break-words text-left text-[12px] leading-[1.6] text-ink-muted">
            {bodyText()}
            <Show when={props.isLive && props.isLast}>
              <span class="stream-cursor" />
            </Show>
          </p>
        </div>
      </Show>
    </div>
  );
};

export const ToolRow: Component<{
  tool: { call: ToolCallData; result?: ToolResultData; status: string };
  isExpanded: boolean;
  onToggle: () => void;
}> = (props) => {
  const header = () => toolHeader(props.tool.call);
  const icon = () => header().icon;
  const title = () => header().title;
  const summary = () => toolSummary(props.tool.call);
  const alwaysShown = () => alwaysShowsBody(props.tool.call.toolName);
  const [showRaw, setShowRaw] = createSignal(false);

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
        onClick={alwaysShown() ? undefined : props.onToggle}
        class="flex h-7 w-full items-center gap-2 rounded px-1 text-xs hover:bg-surface-2"
        classList={{ "cursor-default": alwaysShown() }}
      >
        <span class="trajectory-node flex h-5 w-5 shrink-0 items-center justify-center">
          <Icon name={icon()} class="h-[14px] w-[14px] text-ink-muted" />
        </span>
        <span class="shrink-0 whitespace-nowrap text-[12px] text-ink-muted">{title()}</span>
        <span class="min-w-0 flex-1 truncate text-left font-mono text-[12px] text-ink-faint">{summary()}</span>
        <div class="ml-auto flex items-center gap-1">
          <Icon name={statusIcon() as IconName} class={`h-3 w-3 ${statusClass()}`} />
          <Show when={!alwaysShown()}>
            <Icon
              name="chevron-right"
              class={`h-3 w-3 text-ink-faint transition-transform duration-120 ${props.isExpanded ? "rotate-90" : ""}`}
            />
          </Show>
        </div>
      </button>
      <Show when={alwaysShown() || props.isExpanded}>
        <div class="ml-6 rounded-md bg-surface-1 p-2 text-left text-xs">
          <ToolBody call={props.tool.call} result={props.tool.result} />
          <button
            onClick={() => setShowRaw(!showRaw())}
            class="mt-2 text-[10px] text-ink-faint underline decoration-dotted hover:text-ink-muted"
          >
            {showRaw() ? "Hide raw JSON" : "Show raw JSON"}
          </button>
          <Show when={showRaw()}>
            <div class="mt-1">
              <div class="mb-1 font-mono text-[11px] font-medium text-ink-muted">{"Arguments"}</div>
              <pre class="mb-2 overflow-x-auto whitespace-pre-wrap font-mono text-[11px] text-ink-faint">
                {JSON.stringify(props.tool.call.args, null, 2)}
              </pre>
              <Show when={props.tool.result}>
                <div class="mb-1 font-mono text-[11px] font-medium text-ink-muted">{"Result"}</div>
                <pre class="max-h-48 overflow-y-auto whitespace-pre-wrap break-all font-mono text-[11px] text-ink-faint">
                  {(props.tool.result!.error ?? props.tool.result!.output).slice(0, 5000)}
                </pre>
              </Show>
            </div>
          </Show>
        </div>
      </Show>
    </div>
  );
};


export const ApprovalCard: Component<{
  toolCall: ToolCallData;
  /// Whether the owning ChatPanel is the visible one. Hidden panels keep
  /// their pending ApprovalCards mounted, so without this guard Enter/Esc in
  /// the visible workspace would resolve another workspace's approval.
  isActive: () => boolean;
  onApprove: () => void;
  onReject: () => void;
}> = (props) => {
  const proposal = () => props.toolCall.editProposal as EditProposalData | undefined;
  const isBash = () => props.toolCall.toolName === "bash";

  // The chat input is disabled while an approval is pending, so a global
  // listener is safe: Enter approves, Esc rejects.
  const onKey = (e: KeyboardEvent) => {
    if (!props.isActive()) return;
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

  return (
    <div class="rounded-lg border border-accent/50 bg-surface-1 p-3">
      <div class="mb-2 flex items-center justify-between">
        <div class="flex items-center gap-2">
          <Show
            when={isBash()}
            fallback={
              <>
                <span class="rounded bg-accent/15 px-1.5 py-0.5 text-[10px] font-semibold text-accent">
                  {"Proposed edit"}
                </span>
                <span class="truncate font-mono text-[12px] text-ink-muted">
                  {proposal()?.path ?? (props.toolCall.args.path as string)}
                </span>
              </>
            }
          >
            <span class="rounded bg-amber-500/15 px-1.5 py-0.5 text-[10px] font-semibold text-amber-500">
              {"Bash command"}
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
          <div class="mb-3 overflow-hidden rounded border border-border-subtle">
            <DiffViewer
              original={p().oldString}
              modified={p().newString}
              language={detectLanguageFromPath(p().path)}
              inline
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
          {"Approve"}
          <kbd class="rounded bg-accent-ink/15 px-1 font-mono text-[10px]">⏎</kbd>
        </button>
        <button
          onClick={props.onReject}
          class="flex flex-1 items-center justify-center gap-1.5 rounded-md border border-border-subtle bg-surface-0 px-3 py-1.5 text-sm text-ink hover:border-danger hover:text-danger"
        >
          <Icon name="x" class="h-4 w-4" />
          {"Reject"}
          <kbd class="rounded bg-surface-2 px-1 font-mono text-[10px]">esc</kbd>
        </button>
      </div>
    </div>
  );
};
