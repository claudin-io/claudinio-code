import { createEffect, createMemo, createSignal, ErrorBoundary, For, onCleanup, onMount, Show, type Component } from "solid-js";
import {
  sendMessage,
  approveTool,
  rejectTool,
  submitAnswers,
  newSession,
  listSessions,
  loadSession,
  listPlans,
  queueSteering,
  interruptSession,
  compactSession,
  getSessionStats,
  getConfig,
  loginWithClaudinio,
  openExternalUrl,
  readAttachment,
  writeClipboardBlob,
  setSessionMode,
  continueWithBuilderSession,
  checkPlanExists,
  normalizeSessionMode,
  pickFiles,
  enhancePrompt,
  getTasks,
  type EnhancePromptContext,
  type ModeOrigin,
  type SessionMode,
  type ThinkingEffort,
  type ModeChangedData,
  type GoldenLoopData,
  type SessionLinkedData,
  type HandoffReason,
  type AgentEvent,
  type RetryingData,
  type AskUserData,
  type ToolCallData,
  type EditProposalData,
  type DoneData,
  type ToolResultData,
  type SubagentStartedData,
  type SubagentDoneData,
  type Phase,
  type SessionSummary,
  type PlanEntry,
  type SessionRecord,
  type UserAnswer,
} from "../lib/ipc";
import { applySubagentDone, syncSubagentTimelineItems } from "../lib/subagentTimeline";
import { createSmoothText, balanceMarkdown } from "../lib/createSmoothText";
import { marked } from "marked";
import hljs from "highlight.js";
import { DiffViewer } from "./DiffViewer";
import { ProseContent } from "./ProseContent";
import { Icon, type IconName } from "./Icon";
import { ToolBody } from "./tool-renderers/ToolBody";
import { alwaysShowsBody, detectLanguageFromPath, toolHeader, toolSummary } from "./tool-renderers/toolPresentation";
import TextEditorModal from "./TextEditorModal";
import { FileMentionPopover } from "./FileMentionPopover";
import { ThinkingEffortSlider } from "./ThinkingEffortSlider";
import { TagMentionPopover } from "./TagMentionPopover";
import { SkillMentionPopover } from "./SkillMentionPopover";
import ContextWarning from "./ContextWarning";
import { getCurrentWindow } from "@tauri-apps/api/window";
import { t } from "../lib/grill-me";
import { setWorkspaceStatus, isBusy } from "../lib/workspaceStatus";
import { ToastPill } from "./ToastPill";
import { GitIndicator } from "./GitIndicator";
import { NetworkIndicator } from "./NetworkIndicator";
import NetworkActivityModal from "./NetworkActivityModal";
import { cpuPercent, memoryRssBytes, formatMemory } from "../lib/systemStats";
import { GitChangesModal } from "./GitChangesModal";
import CommitPushModal from "./CommitPushModal";
import ContentViewerModal from "./ContentViewerModal";
import { Popover } from "./Popover";
import { NewSessionPopover } from "./NewSessionPopover";
import QuestionCard from "./QuestionCard";

// The live streaming block re-parses the whole message every smooth-text tick
// (~30/s), which used to re-highlight every code block each time. Completed
// blocks hit this cache instead; only the still-growing block re-highlights.
const highlightCache = new Map<string, string>();
const HIGHLIGHT_CACHE_MAX = 300;

// True while parsing the live streaming message (see parseLiveMarkdown):
// unlabeled blocks skip hljs.highlightAuto there — it tries every registered
// grammar and is by far the worst per-tick cost.
let parsingLiveMessage = false;

function escapeHtml(text: string): string {
  return text
    .replace(/&/g, "&amp;")
    .replace(/</g, "&lt;")
    .replace(/>/g, "&gt;");
}

/** marked.parse for the live streaming block, with auto-detection disabled. */
function parseLiveMarkdown(text: string): string {
  parsingLiveMessage = true;
  try {
    return marked.parse(text, { async: false }) as string;
  } finally {
    parsingLiveMessage = false;
  }
}

marked.use({
  renderer: {
    code({ text, lang }) {
      // lang + text joined by a separator no fence language token can
      // contain, so "js"+"xA" and "jsx"+"A" never collide as one key. Must
      // not be a NUL byte: that makes git treat this whole file as binary.
      const key = `${lang ?? ""}\x1f${text}`;
      if (lang === "mermaid") {
        // Emit a placeholder carrying the (encoded) source. ProseContent
        // hydrates it into an SVG after injection; the visible <pre> is the
        // fallback shown while streaming and on any render error. This runs
        // inside marked.parse() during render, so it must not throw:
        // encodeURIComponent rejects lone surrogates (possible in streamed or
        // model-authored source), which would otherwise blank the message.
        let encoded: string;
        try {
          encoded = encodeURIComponent(text);
        } catch {
          // Fall back to a plain code block — no diagram, but nothing breaks.
          return `<div class="code-block"><pre class="hljs"><code>${escapeHtml(text)}</code></pre></div>`;
        }
        return `<div class="mermaid-block" data-mermaid="${encoded}">`
          + `<pre class="mermaid-src"><code>${escapeHtml(text)}</code></pre></div>`;
      }
      let highlighted = highlightCache.get(key);
      if (highlighted === undefined) {
        if (lang && hljs.getLanguage(lang)) {
          highlighted = hljs.highlight(text, { language: lang }).value;
        } else if (parsingLiveMessage) {
          // Don't cache: once the message completes, the final render
          // auto-detects (one-time cost) and caches that result.
          return `<div class="code-block"><pre class="hljs"><code>${escapeHtml(text)}</code></pre></div>`;
        } else {
          highlighted = hljs.highlightAuto(text).value;
        }
        if (highlightCache.size >= HIGHLIGHT_CACHE_MAX) highlightCache.clear();
        highlightCache.set(key, highlighted);
      }
      const label = lang
        ? `<span class="code-lang-label">${lang}</span>`
        : "";
      return `<div class="code-block">${label}<pre class="hljs"><code>${highlighted}</code></pre></div>`;
    },
    link({ href, title, text }) {
      const clean = href.split('?')[0].split('#')[0];
      const ext = clean.split('.').pop()?.toLowerCase();
      let dataType = 'file';
      if (href.match(/^https?:\/\//)) {
        dataType = 'external';
      } else if (ext && ['png', 'jpg', 'jpeg', 'gif', 'webp', 'svg'].includes(ext)) {
        dataType = 'image';
      } else if (ext && ['mp4', 'webm', 'mov'].includes(ext)) {
        dataType = 'video';
      } else if (ext && ['mp3', 'wav', 'ogg', 'flac'].includes(ext)) {
        dataType = 'audio';
      }
      const titleAttr = title ? ` title="${title}"` : '';
      return `<a href="${href}"${titleAttr} data-link-type="${dataType}">${text}</a>`;
    },
  },
});

type Status = "idle" | "thinking" | "awaiting_approval" | "awaiting_input" | "done" | "error";

interface ArchivedBlock {
  summary: string;
  messages: ChatMessage[];
}

interface ChatMessage {
  role: "user" | "assistant" | "archived";
  text: string;
  steps?: TimelineItem[];
  done?: DoneData;
  archived?: ArchivedBlock;
  /** Files attached to a user message, shown as pills in the chat bubble */
  attachments?: { name: string; mediaType: string; size: number }[];
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
  cost: number;
  report?: string;
  steps: TimelineItem[];
}

interface TimelineItem {
  type: "thinking" | "tool" | "phase" | "phase_result" | "text" | "steering" | "subagent" | "compaction" | "mode" | "golden" | "linked";
  thinking?: { text: string; startedAt: number; endedAt?: number };
  tool?: {
    call: ToolCallData;
    result?: ToolResultData;
    status: "running" | "ok" | "error";
  };
  phase?: Phase;
  phaseResult?: { phase: Phase; text: string };
  text?: string;
  steering?: { text: string; attachments?: Array<{ name: string; mediaType: string; size: number }> };
  subagent?: SubagentTimelineState;
  compaction?: {
    kind: "start" | "done" | "fail" | "handoff_start" | "handoff_fail";
    args: string[];
  };
  modeChange?: ModeChangedData;
  golden?: GoldenLoopData;
  /// Chain divider: this conversation continued in a new linked session.
  /// `firstMessage` (when present) is the successor's kickoff prompt / handoff
  /// document, rendered collapsed. `docOnly` marks the predecessor-side
  /// handoff-document record.
  linked?: {
    reason: HandoffReason;
    mode?: SessionMode;
    firstMessage?: string;
    docOnly?: boolean;
  };
}

interface QueuedSteeringEntry {
  text: string;
  attachments: Array<{ name: string; mediaType: string; size: number }>;
}

function modeChangeLabel(mc: ModeChangedData): string {
  const label = t(`mode.changed.${mc.mode}.${mc.origin}`);
  return mc.reason ? `${label} — ${t("mode.changed.reason", mc.reason)}` : label;
}

const PHASE_LABEL = (phase: Phase): string => {
  switch (phase) {
    case "plan": return t("chat.phase.plan");
    case "execute": return t("chat.phase.execute");
    case "summary": return t("chat.phase.summary");
  }
};

function formatTokens(n: number): string {
  if (n < 1000) return `${n}`;
  return `${(n / 1000).toFixed(n < 10000 ? 1 : 0)}k`;
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

// Mirror of persist.rs is_real_user_turn: a turn record that starts a real
// user exchange (role "user", first content block is plain text).
function isRealUserTurn(rec: SessionRecord): boolean {
  if (rec.kind !== "turn" || rec.role !== "user") return false;
  const content = (rec.content as ContentBlockJson[]) ?? [];
  return content.length > 0 && content[0].type === "text";
}

// Mirror of persist.rs tail_start_index: where the kept-verbatim tail of a
// Compacted marker begins. Expands backwards so the tail starts on a real
// user turn; drops the tail (returns compactIdx) when none exists.
function tailStartIndex(recs: SessionRecord[], compactIdx: number, tailTurns: number): number {
  if (tailTurns <= 0) return compactIdx;
  let start = compactIdx;
  let count = 0;
  for (let i = compactIdx - 1; i >= 0; i--) {
    if (recs[i].kind === "turn") {
      start = i;
      count++;
      if (count >= tailTurns) break;
    }
  }
  if (count === 0) return compactIdx;
  for (;;) {
    if (isRealUserTurn(recs[start])) break;
    let prev = -1;
    for (let i = start - 1; i >= 0; i--) {
      if (recs[i].kind === "turn") {
        prev = i;
        break;
      }
    }
    if (prev < 0) return compactIdx;
    start = prev;
  }
  // Pull in the raw user/steering records that precede the tail's user turn
  // so the "Você" bubble renders in the active view, not the archive.
  while (start > 0 && (recs[start - 1].kind === "user" || recs[start - 1].kind === "steering")) {
    start--;
  }
  return start;
}

// Relocate each Compacted marker to before its kept-verbatim tail so the fold
// logic below archives only what the backend actually summarized.
function normalizeCompactTails(records: SessionRecord[]): SessionRecord[] {
  const recs = [...records];
  for (let i = 0; i < recs.length; i++) {
    const rec = recs[i];
    if (rec.kind !== "compacted") continue;
    const tailTurns = Number(rec.tail_turns ?? 0);
    if (tailTurns <= 0) continue;
    const start = tailStartIndex(recs, i, tailTurns);
    if (start < i) {
      recs.splice(i, 1);
      recs.splice(start, 0, rec);
    }
  }
  return recs;
}

// Rebuild the chat transcript from a reopened session's JSONL records. User
// turns become user bubbles; everything between them folds into one assistant
// message with a phase/tool timeline. Tool results are paired to their calls by
// tool_use_id across turn records.
//
// Compacted records are rendered as an ArchivedBlock: the messages the
// compaction summarized fold into a collapsible section; the kept-verbatim
// tail (tail_turns) stays in the active view.
// Text long enough to be a real explanation rather than a one-line status note.
const SUBSTANTIAL_TEXT_CHARS = 280;

// A short closing message after tool calls often just points at a longer
// explanation the model wrote mid-turn (before a trailing tasks_set). That
// explanation would otherwise stay hidden inside the collapsed trajectory, so
// hoist the last substantial text step into the visible answer.
function promoteSubstantialText(
  steps: TimelineItem[],
  text: string,
): { steps: TimelineItem[]; text: string } {
  if (text.length >= SUBSTANTIAL_TEXT_CHARS) return { steps, text };
  let idx = -1;
  for (let i = steps.length - 1; i >= 0; i--) {
    const s = steps[i];
    if (s.type === "text" && s.text && s.text.length >= SUBSTANTIAL_TEXT_CHARS) {
      idx = i;
      break;
    }
  }
  if (idx === -1) return { steps, text };
  const hoisted = steps[idx].text!;
  return {
    steps: steps.filter((_, i) => i !== idx),
    text: text ? `${hoisted}\n\n${text}` : hoisted,
  };
}

function recordsToMessages(rawRecords: SessionRecord[]): ChatMessage[] {
  const records = normalizeCompactTails(rawRecords);
  const out: ChatMessage[] = [];
  let steps: TimelineItem[] = [];
  let assistantText = "";
  let done: DoneData | undefined;
  const toolIndex = new Map<string, number>();
  // Pile of messages accumulated before a Compacted record
  let preCompact: ChatMessage[] = [];

  const flush = () => {
    if (steps.length || assistantText || done) {
      const promoted = promoteSubstantialText([...steps], assistantText);
      const msg: ChatMessage = { role: "assistant", text: promoted.text, steps: promoted.steps };
      if (done) msg.done = done;
      preCompact.push(msg);
      steps = [];
      assistantText = "";
      done = undefined;
      toolIndex.clear();
    }
  };

  const flushToOut = () => {
    flush();
    if (preCompact.length > 0) {
      out.push(...preCompact);
      preCompact = [];
    }
  };

  // Set right after a linked_from record: the next raw `user` record is the
  // harness-composed kickoff / handoff wrapper, which folds into the divider
  // (collapsed) instead of rendering as a user bubble.
  let pendingLinkIdx = -1;

  for (const rec of records) {
    const kind = rec.kind;
    // Metadata records (base_commit, tasks, mode, status…) sit between
    // linked_from and the kickoff user record — only real content resets the
    // pending fold.
    if (kind === "turn" || kind === "steering") pendingLinkIdx = -1;
    if (kind === "linked_from") {
      flush();
      steps.push({
        type: "linked",
        linked: { reason: (rec.reason as HandoffReason) ?? "context_handoff" },
      });
      pendingLinkIdx = steps.length - 1;
      continue;
    }
    if (kind === "handoff") {
      steps.push({
        type: "linked",
        linked: {
          reason: "context_handoff",
          firstMessage: String(rec.text ?? ""),
          docOnly: true,
        },
      });
      continue;
    }
    if (kind === "user" && pendingLinkIdx >= 0) {
      const idx = pendingLinkIdx;
      pendingLinkIdx = -1;
      const item = steps[idx];
      if (item?.type === "linked" && item.linked) {
        steps[idx] = {
          ...item,
          linked: { ...item.linked, firstMessage: String(rec.text ?? "") },
        };
        continue;
      }
    }
    if (kind === "compacted") {
      // Flush current assistant message into preCompact pile
      flush();
      // Wrap all pre-compact messages into an ArchivedBlock.
      // If the last item before the marker is a bare "user" record, peel it back out:
      // compaction can be inserted after receiving a user message but before its
      // response turns are written (e.g. high context triggered auto-compact on "oi").
      // The peeled user starts the visible post-compact transcript so the chat
      // renders the prompt + the activity that followed the compaction marker.
      let liveLead: ChatMessage | null = null;
      if (preCompact.length > 0 && preCompact[preCompact.length - 1].role === "user") {
        liveLead = preCompact.pop()!;
      }
      if (preCompact.length > 0) {
        out.push({
          role: "archived",
          text: "",
          archived: {
            summary: String(rec.summary ?? ""),
            messages: [...preCompact],
          },
        });
      }
      preCompact = [];
      if (liveLead) preCompact.push(liveLead);
    } else if (kind === "user") {
      flush();
      preCompact.push({ role: "user", text: String(rec.text ?? "") });
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
        steering: { text: String(rec.text ?? ""), attachments: rec.attachments as Array<{ name: string; mediaType: string; size: number }> | undefined },
      });
    } else if (kind === "mode") {
      steps.push({
        type: "mode",
        modeChange: {
          mode: normalizeSessionMode(rec.mode),
          origin: rec.origin as ModeChangedData["origin"],
        },
      });
    } else if (kind === "golden_cycle") {
      steps.push({
        type: "golden",
        golden: {
          cycle: Number(rec.cycle ?? 0),
          maxCycles: 0,
          pending: (rec.goals as string[]) ?? [],
          mode: normalizeSessionMode(rec.mode),
        },
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
  flushToOut();
  return out;
}

// Fallback shown when a single message (or the live block) throws while
// rendering. It MUST stay trivially safe — no markdown, no Icon, no ProseContent
// — because it renders precisely when something else in the thread just failed.
// Its whole purpose is to contain that failure to one bubble instead of letting
// it blank the entire conversation (there is no ErrorBoundary above the list).
const RenderErrorFallback: Component = () => (
  <div class="my-2 ml-6 text-[11px] text-ink-faint">⚠ {t("chat.message.renderError")}</div>
);

export const ChatPanel: Component<{
  /// Root path of the workspace this panel belongs to. One panel is mounted
  /// per open workspace; hidden ones keep streaming their run's events.
  workspace: string;
  /// Whether this panel is the visible one. Global listeners (ESC interrupt,
  /// drag-drop) must only act on the active panel.
  isActive: () => boolean;
  /// Flat list of all workspace files for @-mention autocomplete.
  fileList: string[];
  /// Global thinking-effort setting shown/edited by the toolbar slider.
  thinkingEffort: () => ThinkingEffort;
  onThinkingEffortChange: (v: ThinkingEffort) => void;
}> = (props) => {
  const [input, setInput] = createSignal("");
  const [messages, setMessages] = createSignal<ChatMessage[]>([]);
  const [status, setStatus] = createSignal<Status>("idle");
  const [pendingApprovals, setPendingApprovals] = createSignal<(ToolCallData & { subagentName?: string })[]>([]);
  const [currentAskUser, setCurrentAskUser] = createSignal<AskUserData | null>(null);
  const [currentSteps, setCurrentSteps] = createSignal<TimelineItem[]>([]);
  // Live typewriter preview: `liveText` is the latest TextDelta/Done snapshot,
  // smoothed word-by-word by `smoothLiveText`. `pendingDone` holds the Done
  // payload while the preview finishes draining, so promotion into
  // `messages` doesn't happen until the typewriter has caught up.
  const [liveText, setLiveText] = createSignal("");
  const [liveFinished, setLiveFinished] = createSignal(false);
  const [pendingDone, setPendingDone] = createSignal<{ data: DoneData; final: TimelineItem[] } | null>(null);
  const smoothLiveText = createSmoothText(liveText, liveFinished);
  // Same typewriter treatment for the live "Thoughts" block. No defer-until-
  // drained logic is needed here (unlike Done/pendingDone above): once a
  // thinking step stops being the timeline's last item, ThinkingRow just
  // stops showing its body — there's nothing "promoted" that could be cut
  // off mid-word, and the raw `props.thinking.text` is always fully
  // accumulated regardless of how far the typewriter got.
  const [liveThinkingText, setLiveThinkingText] = createSignal("");
  const liveThinkingActive = () => {
    const steps = currentSteps();
    const last = steps[steps.length - 1];
    return status() === "thinking" && last?.type === "thinking";
  };
  const smoothThinking = createSmoothText(liveThinkingText, () => !liveThinkingActive());
  const [subagentState, setSubagentState] = createSignal<Record<string, SubagentTimelineState>>({});
  const [openSubagentId, setOpenSubagentId] = createSignal<string | null>(null);
  // Resolves the subagent shown in the modal. `subagentState` is cleared once
  // the parent turn finishes (Done/Error), so a click on a subagent row that
  // belongs to a completed message needs to fall back to the snapshot
  // embedded in that message's steps (or the live `currentSteps`).
  const openSubagent = createMemo<SubagentTimelineState | undefined>(() => {
    const id = openSubagentId();
    if (!id) return undefined;
    const live = subagentState()[id];
    if (live) return live;
    const findIn = (steps: TimelineItem[] | undefined) =>
      steps?.find((s) => s.type === "subagent" && s.subagent?.id === id)?.subagent;
    return findIn(currentSteps()) ?? messages().map((m) => findIn(m.steps)).find((s) => s);
  });
  const [thinkingStart, setThinkingStart] = createSignal(0);
  const [liveExpandedStep, setLiveExpandedStep] = createSignal<number | null>(null);
  const [sessions, setSessions] = createSignal<SessionSummary[]>([]);
  const [showSessions, setShowSessions] = createSignal(false);
  const [plans, setPlans] = createSignal<PlanEntry[]>([]);
  const [showPlans, setShowPlans] = createSignal(false);
  const [showNewPopover, setShowNewPopover] = createSignal(false);
  let newButtonRef: HTMLButtonElement | undefined;
  const [activeSessionId, setActiveSessionId] = createSignal<string | null>(null);
  const [queuedSteering, setQueuedSteering] = createSignal<QueuedSteeringEntry[]>([]);
  const [retryableError, setRetryableError] = createSignal<string | null>(null);
  // Retry automático com backoff em andamento (ex.: failover do claudin.io
  // após um 502) — mostra banner "reconectando" em vez de parecer travado.
  const [retryingInfo, setRetryingInfo] = createSignal<RetryingData | null>(null);
  // Budget do plano estourado: mostra banner de upgrade em vez do retry bar.
  const isBudgetError = () => retryableError()?.startsWith("BUDGET_EXCEEDED::") ?? false;
  // Attachments to send with the next message
  const [attachments, setAttachments] = createSignal<{ name: string; path: string; mediaType: string; size: number }[]>([]);
  const [toastMessage, setToastMessage] = createSignal<string | null>(null);
  const showToast = (msg: string) => setToastMessage(msg);
  const dismissToast = () => setToastMessage(null);
  const [viewerFile, setViewerFile] = createSignal<{ type: "text" | "image" | "video" | "audio"; path: string; title: string } | null>(null);

  const handleLinkClick = (href: string, linkType: string) => {
    if (linkType === "external") {
      openExternalUrl(href);
      return;
    }

    // Resolve relative paths against workspace root
    let p = href.replace(/^file:\/\//, "");
    if (!p.startsWith("/")) {
      const ws = props.workspace.replace(/\\/g, "/").replace(/\/$/, "");
      p = ws + "/" + p.replace(/^\.\//, "");
    }
    const title = p.replace(/\\/g, "/").split("/").pop() ?? p;
    const typeMap: Record<string, "text" | "image" | "video" | "audio"> = {
      external: "text",
      file: "text",
      image: "image",
      video: "video",
      audio: "audio",
    };
    setViewerFile({ type: typeMap[linkType] ?? "text", path: p, title });
  };
  const [showEditor, setShowEditor] = createSignal(false);
  const [, setIsEnhancing] = createSignal(false);
  const [showGitModal, setShowGitModal] = createSignal(false);
  const [showNetModal, setShowNetModal] = createSignal(false);
  const [showCommitPushModal, setShowCommitPushModal] = createSignal(false);
  const [isDragging, setIsDragging] = createSignal(false);
  // @-mention autocomplete state
  const [mentionQuery, setMentionQuery] = createSignal("");
  const [mentionPosition, setMentionPosition] = createSignal<{ top: number; left: number; height: number } | null>(null);
  // `<` tag / `<skill>` autocomplete state
  const [tagQuery, setTagQuery] = createSignal("");
  const [tagPosition, setTagPosition] = createSignal<{ top: number; left: number; height: number } | null>(null);
  // .top is repurposed as "bottom" distance from viewport bottom for the tag popover
  const [skillQuery, setSkillQuery] = createSignal("");
  const [skillPosition, setSkillPosition] = createSignal<{ top: number; left: number; height: number } | null>(null);
  const [tagFlowStep, setTagFlowStep] = createSignal<"tag" | "skill" | null>(null);
  // Two distinct numbers, both computed by the backend (single source of
  // truth): contextTokens = size of the NEXT request's context (drops after
  // compaction); cumulative totals/cost never reset.
  const [contextStats, setContextStats] = createSignal<{
    contextTokens: number;
    cumulativeTokens: number;
    estimatedCost?: number;
    costInput?: number;
    costOutput?: number;
    costCacheRead?: number;
  }>({ contextTokens: 0, cumulativeTokens: 0 });
  const [maxContextTokens, setMaxContextTokens] = createSignal(256_000);
  const [compactThreshold, setCompactThreshold] = createSignal(192_000);
  const [isCompacting, setIsCompacting] = createSignal(false);
  const [mode, setMode] = createSignal<SessionMode>("builder");
  const [modeOrigin, setModeOrigin] = createSignal<ModeOrigin>("human");
  const [hasPlanBeenWritten, setHasPlanBeenWritten] = createSignal(false);

  // Human toggle: persists a Mode record in the session JSONL immediately so
  // the mode survives reloads; a running workflow picks it up next round.
  const switchMode = async (m: SessionMode) => {
    if (m === mode()) return;
    setHasPlanBeenWritten(false);
    setMode(m);
    setCurrentSteps((prev) => [
      ...prev,
      { type: "mode" as const, modeChange: { mode: m, origin: "human" as const } },
    ]);
    try {
      const result = await setSessionMode(props.workspace, m);
      setActiveSessionId(result.sessionId);

      // If switching to brain mode and a plan already exists on disk,
      // show the "Continue with Builder" button immediately
      if (m === "brain") {
        const planExists = await checkPlanExists(props.workspace);
        if (planExists) setHasPlanBeenWritten(true);
      }
    } catch {
      // backend unavailable — sendMessage will sync the mode on next send
    }
  };

  // Approve the plan: the backend creates a NEW linked Builder session whose
  // first prompt carries the plan, and starts executing it. The SessionLinked
  // event on the channel inserts the chain divider and flips the mode — for
  // the user it's the same continuous conversation.
  const continueWithBuilder = async () => {
    try {
      setHasPlanBeenWritten(false);
      flushPendingDone();
      setThinkingStart(0);
      setStatus("thinking");
      scrollToBottom(true);
      const result = await continueWithBuilderSession(props.workspace, handleEvent);
      setActiveSessionId(result.sessionId);
      setMode("builder");
      setModeOrigin("human");
    } catch (e) {
      setRetryableError(String(e));
      setStatus("error");
    }
  };

  // Feed the sidebar's per-workspace running indicator.
  createEffect(() => setWorkspaceStatus(props.workspace, status()));

  // When this panel becomes visible again, restore the scroll position —
  // scrollIntoView is a no-op while the panel is display:none.
  createEffect(() => {
    if (props.isActive()) scrollToBottom(true);
  });

  const addAttachment = async (filePath: string) => {
    try {
      const data = await readAttachment(filePath);
      setAttachments((prev) => [...prev, {
        name: data.name,
        path: filePath,
        mediaType: data.mediaType,
        size: data.size,
      }]);
    } catch (e) {
      // Silently ignore unreadable files
    }
  };

  const removeAttachment = (index: number) => {
    setAttachments((prev) => prev.filter((_, i) => i !== index));
  };

  const handlePaste = async (e: ClipboardEvent) => {
    // Only handle pastes when the textarea is enabled
    if (isCompacting() || status() === "awaiting_approval" || status() === "awaiting_input") return;

    const items = e.clipboardData?.items;
    if (!items) return;

    let handled = false;

    // Phase 1: Check for image blobs in clipboard items
    // (e.g. screenshots, copied images from browser)
    for (let i = 0; i < items.length; i++) {
      const item = items[i];
      if (item.kind === "file" && item.type.startsWith("image/")) {
        const blob = item.getAsFile();
        if (!blob) continue;

        const base64Data = await new Promise<string>((resolve, reject) => {
          const reader = new FileReader();
          reader.onload = () => {
            const result = reader.result as string;
            const comma = result.indexOf(",");
            resolve(comma >= 0 ? result.slice(comma + 1) : result);
          };
          reader.onerror = () => reject(reader.error);
          reader.readAsDataURL(blob);
        });

        const ext = item.type.split("/")[1] || "png";
        const name = `clipboard-${Date.now()}.${ext}`;
        try {
          const result = await writeClipboardBlob(base64Data, name, item.type);
          await addAttachment(result.path);
          handled = true;
        } catch {
          // Silently ignore if write fails
        }
        break; // One image at a time
      }
    }

    // Phase 2: Check for file objects with path (copied from OS file manager)
    if (!handled && e.clipboardData.files.length > 0) {
      for (let i = 0; i < e.clipboardData.files.length; i++) {
        const file = e.clipboardData.files[i];
        const filePath = (file as any).path as string | undefined;
        if (filePath) {
          try {
            await addAttachment(filePath);
            handled = true;
          } catch {
            // Silently ignore
          }
        }
      }
    }

    // Phase 3: If we attached something via paste, prevent default text paste and show toast
    if (handled) {
      e.preventDefault();
      showToast(t("chat.toast.fileAttached"));
    }
  };

  const handleMentionSelect = (path: string) => {
    const text = input();
    const caret = inputRef?.selectionStart ?? text.length;
    // Scan backwards to find the @ that triggered the popover
    let atIdx = -1;
    for (let i = caret - 1; i >= 0; i--) {
      const ch = text[i];
      if (ch === " " || ch === "\n") break;
      if (ch === "@") { atIdx = i; break; }
    }
    if (atIdx < 0) return;

    const before = text.slice(0, atIdx + 1); // include the @
    const after = text.slice(caret); // after the query
    setInput(`${before}${path}${after}`);
    setMentionQuery("");
    setMentionPosition(null);
    // Re-focus textarea and place cursor at end of inserted path
    setTimeout(() => {
      const el = inputRef;
      if (el) {
        el.focus();
        const pos = atIdx + 1 + path.length;
        el.setSelectionRange(pos, pos);
      }
    }, 0);
  };

  // When user selects "skill" from the tag popover: replace <query with
  // <skill> and open the skill picker.
  const handleTagSelect = (tagType: string) => {
    const text = input();
    const caret = inputRef?.selectionStart ?? text.length;
    // Scan backwards to find the < that triggered the popover
    let atIdx = -1;
    for (let i = caret - 1; i >= 0; i--) {
      const ch = text[i];
      if (ch === " " || ch === "\n" || ch === "@") break;
      if (ch === "<") { atIdx = i; break; }
    }
    if (atIdx < 0) return;

    const before = text.slice(0, atIdx + 1); // include the <
    const after = text.slice(caret); // after the query

    // "goal" has no picker step: insert <goal></goal> and put the cursor
    // between the tags so the user types the goal text directly.
    if (tagType === "goal") {
      setInput(`${before}goal></goal>${after}`);
      setTagQuery("");
      setTagPosition(null);
      setTagFlowStep(null);
      const cursorPos = atIdx + "<goal>".length;
      setTimeout(() => {
        const el = inputRef;
        if (el) {
          el.focus();
          el.setSelectionRange(cursorPos, cursorPos);
        }
      }, 0);
      return;
    }

    // Insert <tagname> (for skill it's <skill>)
    setInput(`${before}${tagType}>${after}`);
    setTagQuery("");
    setTagPosition(null);
    setTagFlowStep("skill");
    // Compute new caret position: right after <skill>
    const insertionEnd = atIdx + 1 + tagType.length + 1; // < + tagType + >
    setTimeout(() => {
      const el = inputRef;
      if (el) {
        el.focus();
        el.setSelectionRange(insertionEnd, insertionEnd);
        // Raw caret coordinates for skill popover — computePosition handles flip+clamp
        const pos = getCaretCoordinates(el, insertionEnd);
        setSkillQuery("");
        setSkillPosition({ top: pos.top, left: pos.left, height: pos.height });
      }
    }, 0);
  };

  // When user selects a skill: wrap with </skill> and place cursor between tags.
  const handleSkillSelect = (skillName: string) => {
    const text = input();
    const caret = inputRef?.selectionStart ?? text.length;
    // Find the opening <skill> tag before the caret
    let skillStart = -1;
    for (let i = caret - 1; i >= 0; i--) {
      if (text[i] === " " || text[i] === "\n" || text[i] === "@") break;
      if (text[i] === "<") { skillStart = i; break; }
    }
    if (skillStart < 0) return;

    // <skill> starts at skillStart, length is 7. The query is between
    // <skill> and the caret.
    const beforeTag = text.slice(0, skillStart + 7); // up to <skill>
    const queryStart = skillStart + 7; // right after <skill>
    const after = text.slice(caret);
    // Replace the query and close the tag: <skill>skillName</skill>
    setInput(`${beforeTag}${skillName}</skill>${after}`);
    // Clear all popover state
    setSkillQuery("");
    setSkillPosition(null);
    setTagFlowStep(null);
    // Place cursor right after </skill>
    const cursorPos = queryStart + skillName.length + 8; // 8 = "</skill>".length
    setTimeout(() => {
      const el = inputRef;
      if (el) {
        el.focus();
        el.setSelectionRange(cursorPos, cursorPos);
      }
    }, 0);
  };

  // Close all tag/skill popovers
  const handlePopoverClose = () => {
    setTagQuery("");
    setTagPosition(null);
    setSkillQuery("");
    setSkillPosition(null);
    setTagFlowStep(null);
  };

  /**
   * Compute pixel coordinates of a character position in a textarea by
   * rendering a mirror div with the same font metrics. Adapted from the
   * classic textarea-caret-position library.
   */
  function getCaretCoordinates(textarea: HTMLTextAreaElement, pos: number): { top: number; left: number; height: number } {
    const textareaStyles = window.getComputedStyle(textarea);
    const font = [
      textareaStyles.fontSize,
      textareaStyles.fontFamily,
      textareaStyles.lineHeight,
      textareaStyles.fontWeight,
      textareaStyles.fontStyle,
    ].join(" ");

    const mirror = document.createElement("div");
    mirror.style.cssText = [
      "position: fixed",
      "top: 0",
      "left: 0",
      "visibility: hidden",
      "white-space: pre-wrap",
      "word-wrap: break-word",
      `width: ${textarea.offsetWidth}px`,
      `font: ${font}`,
      `padding: ${textareaStyles.padding}`,
      `border: ${textareaStyles.border}`,
      `box-sizing: ${textareaStyles.boxSizing}`,
      `letter-spacing: ${textareaStyles.letterSpacing}`,
    ].join(";");

    const text = textarea.value.slice(0, pos);
    mirror.textContent = text;
    document.body.appendChild(mirror);

    // Add a span at the caret position
    const span = document.createElement("span");
    span.textContent = ".";
    mirror.appendChild(span);

    const textareaRect = textarea.getBoundingClientRect();
    const spanRect = span.getBoundingClientRect();

    // Mirror sits at viewport origin (fixed; top:0; left:0). Shift to the
    // textarea's actual viewport position and subtract scroll offsets.
    const top = textareaRect.top + spanRect.top - textarea.scrollTop;
    const height = spanRect.height;
    const left = textareaRect.left + spanRect.left - textarea.scrollLeft;

    document.body.removeChild(mirror);

    return { top, left, height };
  }

  onMount(() => {
    getConfig()
      .then((cfg) => {
        if (cfg.maxContextTokens) setMaxContextTokens(cfg.maxContextTokens);
        if (cfg.compactThreshold) setCompactThreshold(cfg.compactThreshold);
      })
      .catch(() => {});

    // Listen for native file drop events via Tauri window API. Every mounted
    // panel receives these, so only the visible one may react.
    const unlistenDrop = getCurrentWindow().onDragDropEvent(async (event) => {
      if (!props.isActive()) return;
      const payload = event.payload;
      if (payload.type === "over") {
        setIsDragging(true);
      } else if (payload.type === "drop") {
        setIsDragging(false);
        for (const filePath of payload.paths) {
          await addAttachment(filePath);
        }
      } else if (payload.type === "leave") {
        setIsDragging(false);
      }
    });

    onCleanup(() => {
      unlistenDrop.then((f) => f());
    });
  });

  const statsFromRecords = (records: SessionRecord[]) => {
    const s = getSessionStats(records);
    setContextStats({
      contextTokens: s.contextTokens ?? 0,
      cumulativeTokens: s.totalInputTokens + s.totalOutputTokens,
      estimatedCost: s.totalCost,
      costInput: s.costInput,
      costOutput: s.costOutput,
      costCacheRead: s.costCacheRead,
    });
  };

  const doCompact = async () => {
    if (isCompacting() || !activeSessionId()) return;
    // Never compact mid-stream: the running workflow already auto-compacts.
    if (status() === "thinking" || status() === "awaiting_approval" || status() === "awaiting_input") return;
    setIsCompacting(true);
    try {
      await compactSession(props.workspace, activeSessionId()!, (event) => {
        if (event.event === "TextStep") {
          setMessages((prev) => {
            const last = prev[prev.length - 1];
            if (last?.role === "archived") {
              const updated = [...prev];
              updated[updated.length - 1] = {
                ...last,
                text: event.data.text,
              };
              return updated;
            }
            return prev;
          });
        }
      });
      // Reload session to get the updated view and the new (smaller) context
      const records = await loadSession(props.workspace, activeSessionId()!);
      setMessages(recordsToMessages(records));
      statsFromRecords(records);
      setCurrentSteps([]);
      scrollToBottom(true);
    } catch (e) {
      if (String(e).includes("API key not configured")) {
        setMessages((prev) => [...prev, { role: "user" as const, text: "__auth_card__" }]);
      } else {
        setMessages((prev) => [...prev, { role: "user" as const, text: t("chat.message.failedToCompact", String(e)) }]);
      }
    } finally {
      setIsCompacting(false);
    }
  };

  let messagesEndRef: HTMLDivElement | undefined;
  let scrollContainerRef: HTMLDivElement | undefined;
  let inputRef: HTMLTextAreaElement | undefined;
  let historyButtonRef: HTMLButtonElement | undefined;
  let plansButtonRef: HTMLButtonElement | undefined;
  const [pendingMessage, setPendingMessage] = createSignal<string | null>(null);
  const [authSigningIn, setAuthSigningIn] = createSignal(false);

  // Smart scroll: only auto-follow new content while the user is at the
  // bottom. Growing scrollHeight doesn't fire scroll events, so isAtBottom
  // reflects the last position the user (or an auto-scroll) settled on.
  const [isAtBottom, setIsAtBottom] = createSignal(true);
  let autoScrolling = false;
  const NEAR_BOTTOM_PX = 80;

  const handleScroll = () => {
    const el = scrollContainerRef;
    if (!el) return;
    const atBottom = el.scrollHeight - el.scrollTop - el.clientHeight < NEAR_BOTTOM_PX;
    if (autoScrolling) {
      // Events fired by the smooth animation aren't user intent; release
      // the guard only once the animation reaches the bottom.
      if (atBottom) autoScrolling = false;
      return;
    }
    setIsAtBottom(atBottom);
  };

  const scrollToBottom = (force = false) => {
    if (!force && !isAtBottom()) return;
    autoScrolling = true;
    setIsAtBottom(true);
    messagesEndRef?.scrollIntoView({ behavior: "instant" });
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
    // Any event other than Retrying means the connection is alive again —
    // drop the reconnecting banner.
    if (event.event !== "Retrying" && retryingInfo() !== null) {
      setRetryingInfo(null);
    }
    if (event.event === "TextDelta") {
      const text = event.data.text;
      // Compaction markers only ever arrive as a complete TextStep; this is
      // defensive in case a marker is ever mid-flight in a delta snapshot.
      if (text.startsWith("__compact") || text.startsWith("__handoff")) return;
      setLiveText(text);
      setRetryableError(null);
    } else if (event.event === "TextStep") {
      // Check for compaction markers
      const text = event.data.text;
      if (text.startsWith("__compact_start__:")) {
        const args = text.slice("__compact_start__:".length).split("/");
        setCurrentSteps((prev) => [...prev, { type: "compaction", compaction: { kind: "start", args } }]);
      } else if (text.startsWith("__compact_done__:")) {
        const args = text.slice("__compact_done__:".length).split("/");
        setCurrentSteps((prev) => [...prev, { type: "compaction", compaction: { kind: "done", args } }]);
      } else if (text.startsWith("__compact_fail__:")) {
        const args = [text.slice("__compact_fail__:".length)];
        setCurrentSteps((prev) => [...prev, { type: "compaction", compaction: { kind: "fail", args } }]);
      } else if (text.startsWith("__handoff_start__:")) {
        const args = text.slice("__handoff_start__:".length).split("/");
        setCurrentSteps((prev) => [...prev, { type: "compaction", compaction: { kind: "handoff_start", args } }]);
      } else if (text.startsWith("__handoff_done__:")) {
        // The SessionLinked event that follows carries the full divider; the
        // done marker itself needs no timeline row.
      } else if (text.startsWith("__handoff_fail__:")) {
        const args = [text.slice("__handoff_fail__:".length)];
        setCurrentSteps((prev) => [...prev, { type: "compaction", compaction: { kind: "handoff_fail", args } }]);
      } else {
        setCurrentSteps((prev) => [...prev, { type: "text", text }]);
        setLiveText("");
      }
      setRetryableError(null);
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
      setLiveThinkingText(event.data);
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
      if (data.toolName === "write_plan") setHasPlanBeenWritten(true);
      if (data.toolName === "ask_user") {
        setCurrentAskUser(null);
        setStatus("thinking");
      }
      scrollToBottom();
    } else if (event.event === "AskUser") {
      setCurrentAskUser(event.data as AskUserData);
      setStatus("awaiting_input");
      scrollToBottom();
    } else if (event.event === "ModeChanged") {
      const data = event.data as ModeChangedData;
      setMode(data.mode);
      setModeOrigin(data.origin);
      setCurrentSteps((prev) => [
        ...prev,
        { type: "mode" as const, modeChange: data } as TimelineItem,
      ]);
      scrollToBottom();
    } else if (event.event === "GoldenLoop") {
      const data = event.data as GoldenLoopData;
      setMode(data.mode);
      setCurrentSteps((prev) => [
        ...prev,
        { type: "golden" as const, golden: data } as TimelineItem,
      ]);
      scrollToBottom();
    } else if (event.event === "SessionLinked") {
      // The run continues in a fresh linked session on the SAME channel: for
      // the user this is one conversation. Promote the finished segment's
      // steps into `messages` (no Done arrived — the old session ended in a
      // handoff), then start the new segment with a chain divider. Input
      // stays locked: status remains "thinking".
      const data = event.data as SessionLinkedData;
      flushPendingDone();
      const steps = syncSubagentTimelineItems(currentSteps(), subagentState());
      const final = steps.map((s) =>
        s.type === "thinking" ? { ...s, thinking: { ...s.thinking!, endedAt: Date.now() } } : s,
      );
      if (final.length > 0 || liveText()) {
        const promoted = promoteSubstantialText(final, liveText());
        setMessages((prev) => [
          ...prev,
          { role: "assistant" as const, text: promoted.text, steps: promoted.steps },
        ]);
      }
      setSubagentState({});
      setLiveText("");
      setLiveFinished(false);
      setPendingDone(null);
      smoothLiveText.reset();
      setLiveThinkingText("");
      smoothThinking.reset();
      setThinkingStart(0);
      setCurrentSteps([
        {
          type: "linked" as const,
          linked: { reason: data.reason, mode: data.mode, firstMessage: data.firstMessage },
        } as TimelineItem,
      ]);
      setActiveSessionId(data.sessionId);
      setMode(data.mode);
      setModeOrigin(data.reason === "manual_builder" ? "human" : "agent");
      setStatus("thinking");
      scrollToBottom();
    } else if (event.event === "SteeringInjected") {
      setQueuedSteering((prev) => prev.filter((s) => s.text !== event.data.text));
      setCurrentSteps((prev) => [
        ...prev,
        { type: "steering" as const, steering: { text: event.data.text, attachments: event.data.attachments } } as TimelineItem,
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
          cost: 0,
          steps: [{ type: "thinking" as const, thinking: { text: t("chat.timeline.waiting"), startedAt: now } }],
        },
      }));
      setCurrentSteps((prev) => [
        ...prev,
        { type: "subagent" as const, subagent: { id: d.subagentId, name: d.name, goal: d.goal, mode: d.mode, status: "running", rounds: 0, inputTokens: 0, outputTokens: 0, cost: 0, steps: [] } },
      ]);
      scrollToBottom();
    } else if (event.event === "SubagentDone") {
      const d = event.data as SubagentDoneData;
      const next = applySubagentDone(subagentState(), d, Date.now());
      setSubagentState(next);
      // Sync the inline timeline snapshot so the main timeline reflects the
      // subagent's terminal status instead of staying stuck on "running".
      setCurrentSteps((prev) => syncSubagentTimelineItems(prev, next));
      scrollToBottom();
    } else if (event.event === "Subagent") {
      const d = event.data;
      const result = processSubagentEvent(subagentState(), d.subagentId, d.event);
      setSubagentState(result.subagents);
      // Note: the inline timeline item is intentionally NOT resynced here.
      // SubagentRow only renders name/mode/status/rounds/tokens/goal/report,
      // none of which change mid-run (only the inner `steps`, read by the
      // modal from subagentState() directly). Resyncing on every streaming
      // event replaced the item's object identity continuously, which made
      // the reference-keyed <For> recreate the bubble's DOM node and eat
      // clicks. See syncSubagentTimelineItems for the one real sync point
      // (SubagentDone).
      if (result.approval) {
        setPendingApprovals((prev) => [...prev, result.approval!]);
        setStatus("awaiting_approval");
      }
      scrollToBottom();
    } else if (event.event === "Done") {
      const data = event.data as DoneData;
      // Sync subagent snapshots into the timeline before promoting into
      // `messages`: the inline items are no longer resynced on every
      // streaming event (see the Subagent handler above), so a run that
      // ends without a SubagentDone (e.g. an interrupt mid-subagent) would
      // otherwise promote a stale snapshot.
      const steps = syncSubagentTimelineItems(currentSteps(), subagentState());
      const final = steps.map((s) => {
        if (s.type === "thinking") {
          return { ...s, thinking: { ...s.thinking!, endedAt: Date.now() } };
        }
        return s;
      });
      // Unlock input right away, but defer promoting into `messages` until
      // the typewriter preview has caught up to the authoritative text —
      // see the drain effect below. `liveText`/`liveFinished` keep the
      // live row in TimelineSteps visible (status()==="done" still renders
      // it) while the last words finish typing.
      setLiveText(data.textOutput);
      setLiveFinished(true);
      setPendingDone({ data, final });
      setCurrentAskUser(null);
      setQueuedSteering([]);
      setSubagentState({});
      setPendingApprovals([]);
      setThinkingStart(0);
      setStatus("done");
      setRetryableError(null);
      scrollToBottom();
    } else if (event.event === "SessionStats") {
      const data = event.data;
      setContextStats({
        contextTokens: data.contextTokens,
        cumulativeTokens: data.inputTokens + data.outputTokens,
        estimatedCost: data.cumulativeCost,
        costInput: data.costInput,
        costOutput: data.costOutput,
        costCacheRead: data.costCacheRead,
      });
      if (data.maxContextTokens) setMaxContextTokens(data.maxContextTokens);
      if (data.compactThreshold) setCompactThreshold(data.compactThreshold);
    } else if (event.event === "Retrying") {
      // Transient provider failure (e.g. 502 during claudin.io failover):
      // the backend is waiting out the backoff and will retry by itself.
      // Keep the timeline and the "thinking" status — only surface a banner.
      setRetryingInfo(event.data);
    } else if (event.event === "Error") {
      // Don't show error in message list — we render an error bar below input.
      // Keep currentSteps: wiping them made the whole turn's work vanish from
      // the timeline on a provider outage, as if the run had never happened
      // (it's all still in the session JSONL).
      setRetryableError(event.data);
      setThinkingStart(0);
      setSubagentState({});
      setStatus("error");
      setLiveText("");
      setLiveFinished(false);
      setPendingDone(null);
      smoothLiveText.reset();
      setLiveThinkingText("");
      smoothThinking.reset();
    }
  };

  // Promotes a deferred Done payload into `messages` immediately, flushing
  // the typewriter to its full text first. Used both by the drain effect
  // (normal case: preview caught up) and by any action that starts a new
  // run before the previous one finished typing (interrupt, retry, new
  // message) so the reply isn't lost.
  const flushPendingDone = () => {
    const pending = pendingDone();
    if (!pending) return;
    const { data, final } = pending;
    const promoted = promoteSubstantialText(final, data.textOutput);
    setMessages((prev) => [
      ...prev,
      { role: "assistant" as const, text: promoted.text, steps: promoted.steps, done: data },
    ]);
    setCurrentSteps([]);
    setLiveText("");
    setLiveFinished(false);
    setPendingDone(null);
    smoothLiveText.reset();
    setLiveThinkingText("");
    smoothThinking.reset();
    scrollToBottom(true);
  };

  createEffect(() => {
    // Read both signals unconditionally so this effect stays subscribed to
    // isDrained() even while pendingDone() is null (Solid only tracks
    // signals actually read on a given run — `&&` short-circuiting would
    // silently drop the isDrained() subscription until the next Done).
    const pending = pendingDone();
    const drained = smoothLiveText.isDrained();
    if (pending && drained) flushPendingDone();
  });

  let lastLiveScroll = 0;
  createEffect(() => {
    smoothLiveText.displayed();
    smoothThinking.displayed();
    const now = Date.now();
    if (now - lastLiveScroll >= 150) {
      lastLiveScroll = now;
      scrollToBottom();
    }
  });

  createEffect(() => {
    // Assim que a resposta (final ou intermediária) começa a aparecer, o
    // "Thought" correspondente para de fazer sentido animar em paralelo —
    // pula pro texto completo pra não ter as duas animações rodando juntas.
    if (liveText()) smoothThinking.flush();
  });

  const handleAuthSignIn = async () => {
    setAuthSigningIn(true);
    try {
      await loginWithClaudinio();
      setMessages((prev) => prev.filter((m) => !m.text.startsWith('__auth_card__')));
      const pending = pendingMessage();
      if (pending) {
        setPendingMessage(null);
        // Don't call send() — the original message is already in the messages array.
        // Call sendMessage() directly via IPC so we don't duplicate the message bubble.
        flushPendingDone();
        setCurrentSteps([]);
        setThinkingStart(0);
        setStatus("thinking");
        scrollToBottom(true);
        try {
          const result = await sendMessage(
            props.workspace,
            pending,
            [],
            handleEvent,
            mode(),
          );
          setActiveSessionId(result.sessionId);
        } catch (e) {
          if (String(e).includes("API key not configured")) {
            setMessages((prev) => [...prev, { role: "user" as const, text: "__auth_card__" }]);
          } else {
            setMessages((prev) => [...prev, { role: "user" as const, text: t("chat.message.failedToSend", String(e)) }]);
          }
          setStatus("idle");
        }
      }
    } catch {
      // Login failed — card stays visible, user can retry
    } finally {
      setAuthSigningIn(false);
    }
  };

  const handleRetryContinue = async () => {
    if (status() !== "error") return;
    flushPendingDone();
    setRetryableError(null);
    setCurrentSteps([]);
    setThinkingStart(0);
    setStatus("thinking");
    scrollToBottom(true);
    try {
      const result = await sendMessage(
        props.workspace,
        "[system] continue from where you stopped",
        [],
        handleEvent,
        mode(),
      );
      setActiveSessionId(result.sessionId);
    } catch (e) {
      setRetryableError(String(e));
      setStatus("error");
    }
  };

  const enhanceHandler = async (text: string): Promise<string> => {
    setIsEnhancing(true);
    try {
      const ctx = await buildEnhanceContext(text);
      return await enhancePrompt(props.workspace, text, ctx);
    } catch (e) {
      showToast(t("enhance.error", String(e)));
      throw e;
    } finally {
      setIsEnhancing(false);
    }
  };

  const buildEnhanceContext = async (text: string): Promise<EnhancePromptContext> => {
    // Last 10 user/assistant messages, exclude archived
    const msgs = messages();
    const recent = msgs
      .filter((m) => m.role === "user" || m.role === "assistant")
      .slice(-10)
      .map((m) => ({ role: m.role, text: m.text.length > 500 ? m.text.slice(0, 500) + "..." : m.text }));

    // Extract @-mentioned files from the current input
    const mentionPattern = /@([\w./-]+)/g;
    const mentioned: string[] = [];
    let match;
    while ((match = mentionPattern.exec(text)) !== null) {
      mentioned.push(match[1]!);
    }

    // Active task titles (best-effort)
    let activeTaskTitles: string[] = [];
    try {
      const tasks = await getTasks(props.workspace);
      activeTaskTitles = tasks.filter((t) => t.status === "doing" || t.status === "todo").map((t) => t.title);
    } catch {
      // best-effort
    }

    // Project name from workspace path
    const projectSummary = props.workspace.split("/").pop() ?? props.workspace;

    return {
      messages: recent,
      mode: mode(),
      mentionedFiles: mentioned,
      activeTaskTitles,
      projectSummary,
    };
  };

  const send = async () => {
    const text = input().trim();
    if (!text || isCompacting() || status() === "awaiting_approval" || status() === "awaiting_input") return;

    // If the agent is currently thinking, queue the message as steering
    if (status() === "thinking") {
      const sid = activeSessionId();
      const atts = attachments();
      if (sid) {
        try {
          await queueSteering(sid, text, atts.map((a) => ({ path: a.path })));
        } catch {
          // best-effort
        }
      }
      setQueuedSteering((prev) => [
        ...prev,
        {
          text,
          attachments: atts.map((a) => ({
            name: a.name,
            mediaType: a.mediaType,
            size: a.size,
          })),
        },
      ]);
      setAttachments([]);
      setInput("");
      return;
    }

    // A reply may still be mid-typewriter-drain (status is already "done"
    // but pendingDone hasn't been promoted yet) — flush it now so the text
    // isn't lost or double-rendered once the new run's events start.
    flushPendingDone();

    setMessages((prev) => [
      ...prev,
      {
        role: "user",
        text,
        attachments: attachments().map((a) => ({
          name: a.name,
          mediaType: a.mediaType,
          size: a.size,
        })),
      },
    ]);
    setInput("");
    setCurrentSteps([]);
    setThinkingStart(0);
    setStatus("thinking");
    scrollToBottom(true);

    try {
      const atts = attachments();
      const result = await sendMessage(
        props.workspace,
        text,
        atts.map((a) => ({ path: a.path })),
        handleEvent,
        mode(),
      );
      setActiveSessionId(result.sessionId);
      setAttachments([]);
    } catch (e) {
      if (String(e).includes("API key not configured")) {
        setPendingMessage(text);
        setMessages((prev) => [...prev, { role: "user" as const, text: "__auth_card__" }]);
        setStatus("idle");
      } else {
        setMessages((prev) => [...prev, { role: "user" as const, text: t("chat.message.failedToSend", String(e)) }]);
        setStatus("error");
      }
    }
  };

  const handleApprove = async (tc: ToolCallData) => {
    if (!tc) return;
    setPendingApprovals((prev) => prev.filter((p) => p.toolId !== tc.toolId));
    if (pendingApprovals().length <= 1) setStatus("thinking");
    try {
      await approveTool(tc.sessionId, tc.toolId);
    } catch (e) {
      setMessages((prev) => [...prev, { role: "user", text: t("chat.approval.failed", String(e)) }]);
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
      setMessages((prev) => [...prev, { role: "user", text: t("chat.question.answerFailed", String(e)) }]);
    }
  };

  const handleReject = async (tc: ToolCallData) => {
    if (!tc) return;
    setPendingApprovals((prev) => prev.filter((p) => p.toolId !== tc.toolId));
    if (pendingApprovals().length <= 1) setStatus("thinking");
    try {
      await rejectTool(tc.sessionId, tc.toolId);
    } catch (e) {
      setMessages((prev) => [...prev, { role: "user", text: t("chat.approval.rejectFailed", String(e)) }]);
    }
  };

  const startNewSession = async () => {
    if (isBusy(status())) {
      setShowNewPopover(true);
      return;
    }
    flushPendingDone();
    try {
      await newSession(props.workspace);
    } catch {
      /* fresh session is best-effort */
    }
    setMessages([]);
    setCurrentSteps([]);
    setThinkingStart(0);
    setContextStats({ contextTokens: 0, cumulativeTokens: 0 });
    setMode("builder");
    setStatus("idle");
    setShowSessions(false);
  };

  const handleConfirmNew = async () => {
    setShowNewPopover(false);
    flushPendingDone();
    try {
      await newSession(props.workspace);
    } catch {
      /* fresh session is best-effort */
    }
    setMessages([]);
    setCurrentSteps([]);
    setThinkingStart(0);
    setContextStats({ contextTokens: 0, cumulativeTokens: 0 });
    setMode("builder");
    setStatus("idle");
    setShowSessions(false);
  };

  const toggleSessions = async () => {
    const next = !showSessions();
    setShowSessions(next);
    if (next) {
      try {
        setSessions(await listSessions(props.workspace));
      } catch {
        setSessions([]);
      }
    }
  };

  const togglePlans = async () => {
    const next = !showPlans();
    setShowPlans(next);
    if (next) {
      try {
        setPlans(await listPlans(props.workspace));
      } catch {
        setPlans([]);
      }
    }
  };

  const openPlan = (path: string, title: string) => {
    setShowPlans(false);
    setViewerFile({ type: "text", path, title });
  };

  const readLatestPlan = async () => {
    try {
      const planList = await listPlans(props.workspace);
      if (planList.length > 0) {
        setViewerFile({ type: "text", path: planList[0].path, title: planList[0].name });
      }
    } catch {
      // silently fail
    }
  };

  const reopenSession = async (id: string) => {
    if (status() === "thinking" || status() === "awaiting_approval" || status() === "awaiting_input") return;
    flushPendingDone();
    try {
      const records = await loadSession(props.workspace, id);
      setMessages(recordsToMessages(records));
      try {
        statsFromRecords(records);
      } catch (e) {
        console.error("statsFromRecords failed", e);
      }
      // The JSONL is the source of truth for the mode too: restore the last one.
      const lastMode = [...records].reverse().find((r) => r.kind === "mode");
      try {
        setMode(lastMode ? normalizeSessionMode(lastMode.mode) : "builder");
      } catch (e) {
        console.error("setMode failed", e);
        setMode("builder");
      }
      setActiveSessionId(id);
      setCurrentSteps([]);
      setThinkingStart(0);
      setStatus("idle");
      setShowSessions(false);
      scrollToBottom(true);
    } catch (e) {
      console.error("reopenSession failed", e);
      setMessages((prev) => [...prev, { role: "user", text: t("chat.message.failedToReopen", String(e)) }]);
      setShowSessions(false);
    }
  };

  const handleKeyDown = (e: KeyboardEvent) => {
    if (e.key === "Enter" && !e.shiftKey) {
      // If @-mention popover is open, let it handle Enter instead of sending
      if (mentionPosition() && mentionQuery().length >= 0) return;
      // If tag or skill popover is open, let the popover handle Enter
      if (tagPosition()) return;
      if (skillPosition()) return;
      e.preventDefault();
      send();
    }
  };

  // Global ESC handler: only fires on the visible panel while it's thinking,
  // otherwise ESC would interrupt every workspace running in parallel.
  onMount(() => {
    const onKey = (e: KeyboardEvent) => {
      if (!props.isActive()) return;
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
      case "thinking": return t("chat.status.working");
      case "awaiting_approval": return t("chat.status.awaitingApproval");
      case "awaiting_input": return t("chat.status.awaitingInput");
      case "done": return t("chat.status.done");
      case "error": return t("chat.status.error");
      default: return t("chat.status.idle");
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
            {t("chat.header.agent")}
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
            ref={newButtonRef}
            onClick={() => {
              if (isBusy(status())) {
                setShowNewPopover(true);
              } else {
                startNewSession();
              }
            }}
            class="flex items-center gap-1 rounded px-2 py-1 text-[11px] text-ink-muted hover:bg-surface-2"
            title={t("chat.header.newSession")}
          >
            <Icon name="plus" class="h-3.5 w-3.5" />
            {t("chat.header.new")}
          </button>
          <ContextWarning workspace={props.workspace} />
          <button
            ref={historyButtonRef}
            onClick={toggleSessions}
            class="flex items-center gap-1 rounded px-2 py-1 text-[11px] text-ink-muted hover:bg-surface-2"
            title={t("chat.header.savedSessions")}
          >
            <Icon name="clock" class="h-3.5 w-3.5" />
            {t("chat.header.history")}
          </button>
          <button
            ref={plansButtonRef}
            onClick={togglePlans}
            class="flex items-center gap-1 rounded px-2 py-1 text-[11px] text-ink-muted hover:bg-surface-2"
            title={t("chat.header.plans")}
          >
            <Icon name="archive-drawer" class="h-3.5 w-3.5" />
            {t("chat.header.plans")}
          </button>
          <GitIndicator workspace={props.workspace} active={props.isActive()} onShowChanges={() => setShowGitModal(true)} />
          <NetworkIndicator workspace={props.workspace} onClick={() => setShowNetModal(true)} />
          <span class="font-mono text-[11px] text-ink-faint whitespace-nowrap">
            CPU {cpuPercent().toFixed(0)}% · MEM {formatMemory(memoryRssBytes())}
          </span>
        </div>

        <Popover
          open={showNewPopover()}
          onClose={() => { setShowNewPopover(false); }}
          triggerRef={newButtonRef}
          anchorPoint={{x: 0, y: 1}}
          originPoint={{x: 0, y: 0}}
        >
          <NewSessionPopover
            onConfirm={handleConfirmNew}
            onClose={() => { setShowNewPopover(false); }}
          />
        </Popover>

        <Popover
          open={showSessions()}
          onClose={() => setShowSessions(false)}
          triggerRef={historyButtonRef}
          anchorPoint={{x:1,y:1}}
          originPoint={{x:1,y:0}}
          class="z-20"
        >
          <div class="max-h-80 w-80 overflow-y-auto rounded-lg border border-border-subtle bg-surface-1 py-1 shadow-lg">
            <Show
              when={sessions().length > 0}
              fallback={<div class="px-3 py-2 text-[12px] text-ink-faint">{t("chat.header.noSessions")}</div>}
            >
              <For each={sessions()}>
                {(s) => (
                  <button
                    onClick={() => reopenSession(s.sessionId)}
                    class="flex w-full flex-col gap-0.5 px-3 py-1.5 text-left hover:bg-surface-2"
                  >
                    <span class="truncate text-[12px] text-ink">{s.title}</span>
                    <span class="font-mono text-[10px] text-ink-faint">
                      {new Date(s.updatedAt).toLocaleString()} · {s.turnCount} {s.turnCount === 1 ? t("chat.header.turn") : t("chat.header.turns")}
                    </span>
                  </button>
                )}
              </For>
            </Show>
          </div>
        </Popover>

        <Popover
          open={showPlans()}
          onClose={() => setShowPlans(false)}
          triggerRef={plansButtonRef}
          anchorPoint={{x:1,y:1}}
          originPoint={{x:1,y:0}}
          class="z-20"
        >
          <div class="max-h-80 w-80 overflow-y-auto rounded-lg border border-border-subtle bg-surface-1 py-1 shadow-lg">
            <Show
              when={plans().length > 0}
              fallback={<div class="px-3 py-2 text-[12px] text-ink-faint">{t("chat.header.noPlans")}</div>}
            >
              <For each={plans()}>
                {(p) => (
                  <button
                    onClick={() => openPlan(p.path, p.name)}
                    class="flex w-full flex-col gap-0.5 px-3 py-1.5 text-left hover:bg-surface-2"
                  >
                    <span class="truncate text-[12px] text-ink">{p.name}</span>
                    <span class="font-mono text-[10px] text-ink-faint">
                      {new Date(p.modifiedAt * 1000).toLocaleString()}
                    </span>
                  </button>
                )}
              </For>
            </Show>
          </div>
        </Popover>
      </div>

      <div class="relative flex flex-1 flex-col overflow-hidden">
        <div
          ref={scrollContainerRef}
          onScroll={handleScroll}
          class="flex flex-1 flex-col overflow-y-auto"
        >
          <div class="w-full px-6 py-4">
          <For each={messages()}>
            {(msg) => {
              // Per-message containment: a malformed message, tool step, or
              // Mermaid diagram must degrade to a one-line notice, never abort
              // the <For> and blank every other message in the thread.
              return (<ErrorBoundary fallback={<RenderErrorFallback />}><div class="mb-6">
                <Show when={msg.role === "user"}>
                  <div class="mb-1">
                    <span class="text-[11px] font-semibold uppercase tracking-wider text-accent">
                      {t("chat.message.you")}
                    </span>
                  </div>
                  <Show when={msg.text === "__auth_card__"}>
                    <div class="rounded-lg border border-border-subtle bg-surface-1 p-4">
                      <h3 class="mb-1 text-sm font-semibold text-ink">{t("chat.authCard.title")}</h3>
                      <p class="mb-3 text-xs text-ink-muted">{t("chat.authCard.description")}</p>
                      <button
                        onClick={handleAuthSignIn}
                        disabled={authSigningIn()}
                        class="rounded-md bg-accent px-3 py-1.5 text-sm font-medium text-white hover:opacity-90 disabled:opacity-50"
                      >
                        {authSigningIn() ? t("chat.authCard.signingIn") : t("chat.authCard.signIn")}
                      </button>
                    </div>
                  </Show>
                  <Show when={msg.text !== "__auth_card__"}>
                  <div class="border-l-2 border-accent/60 pl-3">
                    <p class="whitespace-pre-wrap break-words text-left text-[13px] leading-[1.65] text-ink">
                      {msg.text}
                    </p>
                    <Show when={msg.attachments && msg.attachments!.length > 0}>
                      <div class="mt-2 flex flex-wrap gap-1.5">
                        <For each={msg.attachments!}>
                          {(att) => (
                            <span class="inline-flex items-center gap-1 rounded-md border border-accent/20 bg-accent/[0.06] px-1.5 py-0.5 text-[11px] text-ink-muted">
                              <Icon
                                name={(att.mediaType ?? "").startsWith("image/") ? "image" : "file-text"}
                                class="h-3 w-3 shrink-0"
                              />
                              <span class="max-w-[140px] truncate">{att.name}</span>
                              <span class="font-mono text-[9px] text-ink-faint">
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
                </Show>

                <Show when={msg.role === "assistant" && (msg.text || (msg.steps && msg.steps!.length > 0))}>
                  <Show when={msg.steps && msg.steps!.length > 0}>
                    <Trajectory
                      steps={msg.steps!}
                      tokens={msg.done ? { input: msg.done.inputTokens, output: msg.done.outputTokens } : undefined}
                      onViewDetails={(id) => setOpenSubagentId(id)}
                    />
                  </Show>

                  <Show when={msg.text}>
                    <ProseContent
                      class="prose-content text-[13px] text-ink"
                      html={marked.parse(msg.text, { async: false }) as string}
                      onClick={(e) => {
                        const a = (e.target as HTMLElement).closest("a[data-link-type]");
                        if (!a) return;
                        e.preventDefault();
                        handleLinkClick(a.getAttribute("href")!, a.getAttribute("data-link-type")!);
                      }}
                    />
                  </Show>
                </Show>

                <Show when={msg.role === "archived" && msg.archived}>
                  <ArchivedBlock
                    summary={msg.archived!.summary}
                    messages={msg.archived!.messages}
                  />
                </Show>
              </div></ErrorBoundary>);
            }}
          </For>

          <Show when={mode() === "brain" && modeOrigin() === "human" && status() === "done" && hasPlanBeenWritten()}>
            <div class="mb-6 flex justify-center gap-3">
              <button
                onClick={readLatestPlan}
                class="inline-flex items-center gap-2 rounded-full border border-border-subtle bg-surface-1 px-5 py-2.5 text-sm font-semibold text-ink-muted transition-all hover:bg-surface-2 hover:text-ink active:scale-[0.98]"
              >
                <Icon name="archive-drawer" class="h-4 w-4" />
                {t("mode.readPlan")}
              </button>
              <button
                onClick={continueWithBuilder}
                class="inline-flex items-center gap-2 rounded-full bg-accent px-5 py-2.5 text-sm font-semibold text-accent-ink shadow-lg shadow-accent/20 transition-all hover:bg-accent/90 hover:shadow-xl hover:shadow-accent/30 active:scale-[0.98]"
              >
                <Icon name="construction-worker" class="h-4 w-4" />
                {t("mode.continueWithBuilder")}
              </button>
            </div>
          </Show>

          <Show when={status() === "thinking" || status() === "done" || status() === "awaiting_input"}>
            <ErrorBoundary fallback={<RenderErrorFallback />}>
            <div class="mb-6">
              <div class="trajectory-rail flex flex-col gap-0.5">
                <TimelineSteps
                  steps={status() === "thinking" ? currentSteps().filter((s) => s.type !== "thinking") : currentSteps()}
                  expandedStep={liveExpandedStep()}
                  onToggle={(i) => setLiveExpandedStep(liveExpandedStep() === i ? null : i)}
                  isLive={status() === "thinking"}
                  onViewDetails={(id) => setOpenSubagentId(id)}
                  liveThinkingDisplay={smoothThinking.displayed}
                />
                <Show when={smoothLiveText.displayed()}>
                  <div class="my-1 ml-6">
                    <ProseContent
                      live
                      class="prose-content text-[13px] leading-[1.6] text-ink"
                      html={parseLiveMarkdown(balanceMarkdown(smoothLiveText.displayed()))}
                      onClick={(e) => {
                        const a = (e.target as HTMLElement).closest("a[data-link-type]");
                        if (!a) return;
                        e.preventDefault();
                        handleLinkClick(a.getAttribute("href")!, a.getAttribute("data-link-type")!);
                      }}
                    />
                  </div>
                </Show>
              </div>
            </div>
            </ErrorBoundary>
          </Show>

          <Show when={pendingApprovals().length > 0 && status() === "awaiting_approval"}>
            <div class="mb-6 flex flex-col gap-3">
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
                      isActive={props.isActive}
                      onApprove={() => handleApprove(tc)}
                      onReject={() => handleReject(tc)}
                    />
                  </div>
                )}
              </For>
            </div>
          </Show>

          <Show when={currentAskUser() && status() === "awaiting_input"}>
            <div class="mb-6">
              <QuestionCard ask={currentAskUser()!} onSubmit={handleAnswers} />
            </div>
          </Show>

          <div ref={messagesEndRef} />
          </div>
        </div>

        <Show when={!isAtBottom()}>
          <button
            class="absolute bottom-4 right-6 z-10 flex h-8 w-8 items-center justify-center rounded-full border border-border-subtle bg-surface text-ink-muted shadow-md transition-colors hover:text-ink"
            onClick={() => scrollToBottom(true)}
            title={t("chat.scrollToBottom")}
          >
            <Icon name="chevron-down" class="h-4 w-4" />
          </button>
        </Show>
      </div>

      <Show when={liveThinkingActive()}>
        <ThinkingBar text={smoothThinking.displayed} />
      </Show>

      <Show when={openSubagent()}>
        <SubagentModal
          subagent={openSubagent()!}
          onClose={() => setOpenSubagentId(null)}
        />
      </Show>

      <Show when={queuedSteering().length > 0}>
        <div class="flex flex-wrap gap-1.5 border-t border-border-subtle px-6 py-1.5">
          <For each={queuedSteering()}>
            {(s) => (
              <>
                <span class="inline-flex items-center gap-1 rounded-full bg-accent/10 px-2 py-0.5 text-[11px] text-accent">
                  <span class="h-1.5 w-1.5 rounded-full bg-accent" />
                  {s.text.length > 40 ? `${s.text.slice(0, 40)}…` : s.text}
                </span>
                <For each={s.attachments}>
                  {(att) => (
                    <span class="inline-flex items-center gap-1 rounded-md border border-border-subtle bg-surface-1 px-2 py-0.5 text-[11px] text-ink-muted">
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
              </>
            )}
          </For>
        </div>
      </Show>

      <Show when={attachments().length > 0}>
        <div class="flex flex-wrap gap-2 border-t border-border-subtle px-6 py-2">
          <For each={attachments()}>
            {(att, i) => (
              <div class="group flex items-center gap-1.5 rounded-md border border-border-subtle bg-surface-1 px-2 py-1 text-xs text-ink-muted hover:border-accent/40">
                <Icon
                  name={(att.mediaType ?? "").startsWith("image/") ? "image" : "file-text"}
                  class="h-3.5 w-3.5 shrink-0"
                />
                <span class="max-w-[180px] truncate">{att.name}</span>
                <span class="font-mono text-[10px] text-ink-faint">
                  {att.size > 1024 * 1024
                    ? `${(att.size / (1024 * 1024)).toFixed(1)} MB`
                    : att.size > 1024
                      ? `${(att.size / 1024).toFixed(0)} KB`
                      : `${att.size} B`}
                </span>
                <button
                  onClick={() => removeAttachment(i())}
                  class="ml-0.5 flex h-4 w-4 items-center justify-center rounded text-ink-faint hover:bg-danger/20 hover:text-danger"
                >
                  <Icon name="x" class="h-3 w-3" />
                </button>
              </div>
            )}
          </For>
        </div>
      </Show>

      <Show when={retryableError() !== null && isBudgetError()}>
        <div class="border-t border-accent/40 bg-accent/10 px-4 py-3.5">
          <div class="flex items-start justify-between gap-4">
            <div class="flex items-start gap-3 min-w-0">
              <div class="mt-0.5 flex h-7 w-7 shrink-0 items-center justify-center rounded-full bg-accent/20 text-accent">
                <Icon name="external-link" class="h-4 w-4" />
              </div>
              <div class="min-w-0">
                <p class="text-[13px] font-semibold text-ink">{t("chat.budgetBanner.title")}</p>
                <p class="text-[12px] text-ink-muted mt-0.5">{t("chat.budgetBanner.description")}</p>
              </div>
            </div>
            <div class="flex gap-2 shrink-0">
              <button
                onClick={() => setRetryableError(null)}
                class="rounded-md px-3 py-1.5 text-[12px] font-medium text-ink-muted hover:bg-surface-2"
              >
                {t("chat.budgetBanner.dismiss")}
              </button>
              <button
                onClick={() => openExternalUrl("https://claudin.io/dashboard#billing")}
                class="rounded-md bg-accent px-3 py-1.5 text-[12px] font-semibold text-accent-ink hover:bg-accent/80"
              >
                {t("chat.budgetBanner.upgrade")}
              </button>
            </div>
          </div>
        </div>
      </Show>

      <Show when={retryingInfo()}>
        {(info) => (
          <div class="border-t border-amber-500/30 bg-amber-500/5 px-4 py-2.5">
            <div class="flex items-center gap-3">
              <div class="h-3.5 w-3.5 shrink-0 animate-spin rounded-full border-2 border-amber-500/30 border-t-amber-500" />
              <p class="min-w-0 truncate text-[13px] text-amber-600">
                {t(
                  "chat.reconnecting.banner",
                  String(info().attempt),
                  String(info().maxAttempts),
                  String(Math.round(info().delayMs / 1000)),
                )}
                <span class="ml-2 text-[11px] text-ink-faint">{info().error}</span>
              </p>
            </div>
          </div>
        )}
      </Show>

      <Show when={retryableError() !== null && !isBudgetError()}>
        <div class="border-t border-danger/30 bg-danger/5 px-4 py-3">
          <div class="flex items-center justify-between gap-4">
            <p class="text-[13px] text-danger shrink-0">{t("chat.status.error")}: {retryableError()}</p>
            <div class="flex gap-2 shrink-0">
              <button
                onClick={() => setRetryableError(null)}
                class="rounded-md px-3 py-1.5 text-[12px] font-medium text-ink-muted hover:bg-surface-2"
              >
                {t("chat.errorBar.dismiss")}
              </button>
              <button
                onClick={handleRetryContinue}
                class="rounded-md bg-accent px-3 py-1.5 text-[12px] font-medium text-accent-ink hover:bg-accent/80"
              >
                {t("chat.errorBar.continue")}
              </button>
            </div>
          </div>
        </div>
      </Show>

      <div class="border-t border-border-subtle px-6 py-3">
        <div class="w-full">
          <div class="flex flex-col gap-1.5 rounded-lg border border-border-subtle bg-surface-2 p-2 focus-within:border-accent/60">
            <textarea
              ref={inputRef!}
              value={input()}
              onInput={(e) => {
                const textarea = e.currentTarget;
                const text = textarea.value;
                setInput(text);
                // Detect @-mention trigger
                const caret = textarea.selectionStart;
                // Scan backwards from caret to find an active @
                let atIdx = -1;
                for (let i = caret - 1; i >= 0; i--) {
                  const ch = text[i];
                  if (ch === " " || ch === "\n") break;
                  if (ch === "@") { atIdx = i; break; }
                }
                if (atIdx >= 0) {
                  const query = text.slice(atIdx + 1, caret);
                  // Raw caret coordinates — computePosition with anchorPoint/originPoint
                  // handles the flip+clamp automatically.
                  const pos = getCaretCoordinates(textarea, caret);
                  setMentionQuery(query);
                  setMentionPosition({ top: pos.top, left: pos.left, height: pos.height });
                  // Clear tag/skill popovers while @-mention is active
                  if (tagFlowStep()) handlePopoverClose();
                } else {
                  setMentionQuery("");
                  setMentionPosition(null);
                }

                // Detect < tag trigger (only if @-mention popover is not active
                // and we're not already in the skill selection step)
                const ltIdx = tagFlowStep() === "skill" ? -1 : (() => {
                  for (let i = caret - 1; i >= 0; i--) {
                    const ch = text[i];
                    if (ch === " " || ch === "\n" || ch === "@") return -1;
                    if (ch === "<") return i;
                  }
                  return -1;
                })();
                if (ltIdx >= 0 && mentionPosition() === null) {
                  const query = text.slice(ltIdx + 1, caret);
                  // Raw caret coordinates right after the < — computePosition
                  // with anchorPoint {x:0,y:1} handles flip+clamp automatically.
                  const pos = getCaretCoordinates(textarea, ltIdx + 1);
                  setTagQuery(query);
                  setTagPosition({ top: pos.top, left: pos.left, height: pos.height });
                  setTagFlowStep("tag");
                  // Clear skill popover if we're back to tag selection
                  setSkillQuery("");
                  setSkillPosition(null);
                } else if (tagFlowStep() === "skill" && mentionPosition() === null) {
                  // We're inside a <skill> context — update the skill query
                  // Find the text after <skill> and before caret
                  let skillClose = -1;
                  for (let i = caret - 1; i >= 0; i--) {
                    if (text.slice(i, i + 7) === "<skill>") { skillClose = i + 7; break; }
                    if (text[i] === "\n") break;
                  }
                  if (skillClose >= 0) {
                    const skillQ = text.slice(skillClose, caret);
                    const pos = getCaretCoordinates(textarea, caret);
                    setSkillQuery(skillQ);
                    setSkillPosition({ top: pos.top, left: pos.left, height: pos.height });
                  } else {
                    // <skill> text no longer present — close everything
                    handlePopoverClose();
                  }
                  // Clear tag popover
                  setTagQuery("");
                  setTagPosition(null);
                } else if (ltIdx < 0 && tagFlowStep() !== "skill") {
                  // Not inside any < trigger — close tag popover
                  setTagQuery("");
                  setTagPosition(null);
                  if (tagFlowStep() !== "skill") setTagFlowStep(null);
                }
              }}
              onPaste={handlePaste}
              onKeyDown={handleKeyDown}
              disabled={isCompacting() || status() === "awaiting_approval" || status() === "awaiting_input"}
              placeholder={
                isCompacting()
                  ? t("chat.input.compacting")
                  : status() === "awaiting_approval"
                    ? t("chat.input.approveFirst")
                    : status() === "awaiting_input"
                      ? t("chat.input.answerFirst")
                      : status() === "thinking"
                        ? t("chat.input.steerAgent")
                        : t("chat.input.askCode")
              }
              class="max-h-[156px] min-h-[32px] w-full resize-none border-0 bg-transparent px-1 py-1.5 text-[13px] leading-[18px] text-ink placeholder:text-ink-faint focus:outline-none disabled:opacity-50"
              rows={1}
            />
            <div class="flex items-center gap-2">
              <div class="flex items-center gap-1">
                <button
                  onClick={async () => {
                    const files = await pickFiles();
                    for (const f of files) {
                      await addAttachment(f);
                    }
                  }}
                  disabled={isCompacting() || status() === "awaiting_approval" || status() === "awaiting_input"}
                  class="flex h-8 w-8 shrink-0 items-center justify-center rounded-md text-ink-muted hover:bg-surface-3 hover:text-accent disabled:opacity-30"
                  title={t("chat.input.attachFile")}
                >
                  <Icon name="paperclip" class="h-4 w-4" />
                </button>
                <button
                  onClick={() => setShowEditor(true)}
                  disabled={isCompacting() || status() === "awaiting_approval" || status() === "awaiting_input"}
                  class="flex h-8 w-8 shrink-0 items-center justify-center rounded-md text-ink-muted hover:bg-surface-3 hover:text-accent disabled:opacity-30"
                  title={t("editor.open")}
                >
                  <Icon name="notebook-pen" class="h-4 w-4" stroke />
                </button>
                <div class="flex shrink-0 items-center rounded-md border border-border-subtle bg-surface-0 p-0.5">
                  <button
                    onClick={() => switchMode("brain")}
                    class={`flex h-7 w-7 items-center justify-center rounded ${
                      mode() === "brain"
                        ? "bg-accent/15 text-accent"
                        : "text-ink-faint hover:bg-surface-3 hover:text-ink-muted"
                    }`}
                    title={t("mode.brain.tooltip")}
                  >
                    <Icon name="thinking-face" class="h-4 w-4" />
                  </button>
                  <button
                    onClick={() => switchMode("builder")}
                    class={`flex h-7 w-7 items-center justify-center rounded ${
                      mode() === "builder"
                        ? "bg-accent/15 text-accent"
                        : "text-ink-faint hover:bg-surface-3 hover:text-ink-muted"
                    }`}
                    title={t("mode.builder.tooltip")}
                  >
                    <Icon name="construction-worker" class="h-4 w-4" />
                  </button>
                </div>
              </div>
              <div class="flex-1" />
              <div class="flex items-center gap-2">
                <ThinkingEffortSlider
                  value={props.thinkingEffort}
                  onChange={props.onThinkingEffortChange}
                />
                <Show when={status() === "thinking" || status() === "awaiting_approval"}>
                  <button
                    onClick={() => {
                      const sid = activeSessionId();
                      if (sid) interruptSession(sid).catch(() => {});
                    }}
                    class="flex h-8 w-8 shrink-0 items-center justify-center rounded-md bg-danger/20 text-danger hover:bg-danger/40"
                    title={t("chat.input.stop")}
                  >
                    <Icon name="stop" class="h-4 w-4" />
                  </button>
                </Show>
                <button
                  onClick={send}
                  disabled={
                    !input().trim() ||
                    isCompacting() ||
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
      </div>

      <Popover
        open={mentionPosition() !== null && props.fileList.length > 0}
        onClose={() => setMentionPosition(null)}
        position={mentionPosition()!}
        anchorPoint={{x: 0, y: 1}}
      >
        <FileMentionPopover
          fileList={props.fileList}
          query={mentionQuery()}
          onSelect={handleMentionSelect}
          onClose={() => setMentionPosition(null)}
        />
      </Popover>

      <Popover
        open={tagPosition() !== null && tagFlowStep() === "tag"}
        onClose={handlePopoverClose}
        position={tagPosition()!}
        anchorPoint={{x: 0, y: 1}}
      >
        <TagMentionPopover
          query={tagQuery()}
          onSelect={handleTagSelect}
          onClose={handlePopoverClose}
        />
      </Popover>

      <Popover
        open={skillPosition() !== null && tagFlowStep() === "skill"}
        onClose={handlePopoverClose}
        position={skillPosition()!}
        anchorPoint={{x: 0, y: 1}}
      >
        <SkillMentionPopover
          workspace={props.workspace}
          query={skillQuery()}
          onSelect={handleSkillSelect}
          onClose={handlePopoverClose}
        />
      </Popover>

      <ContextFooter
        contextTokens={contextStats().contextTokens}
        maxTokens={maxContextTokens()}
        cumulativeTokens={contextStats().cumulativeTokens}
        estimatedCost={contextStats().estimatedCost}
        costInput={contextStats().costInput}
        costOutput={contextStats().costOutput}
        costCacheRead={contextStats().costCacheRead}
        isCompacting={isCompacting()}
        onCompact={doCompact}
        showCompact={
          contextStats().contextTokens > compactThreshold() * 0.85 &&
          status() !== "thinking" &&
          status() !== "awaiting_approval" &&
          status() !== "awaiting_input"
        }
      />

      <Show when={isDragging()}>
        <div class="drop-overlay">
          <div class="drop-overlay-inner">
            <Icon name="paperclip" class="h-8 w-8" />
            <span>{t("chat.drop.title")}</span>
            <small>{t("chat.drop.hint")}</small>
          </div>
        </div>
      </Show>
      <Show when={showEditor()}>
        <TextEditorModal
          initialText={input()}
          onEnhance={async (text) => {
            try {
              return await enhanceHandler(text);
            } catch {
              return text; // fallback to original on error
            }
          }}
          onClose={(text) => {
            setInput(text);
            setShowEditor(false);
            setTimeout(() => {
              const el = inputRef;
              if (el) {
                el.focus();
                const pos = text.length;
                el.setSelectionRange(pos, pos);
              }
            }, 0);
          }}
        />
      </Show>
      <Show when={showGitModal()}>
        <GitChangesModal
          workspace={props.workspace}
          open={showGitModal()}
          onClose={() => setShowGitModal(false)}
          onCommitPush={() => {
            setShowGitModal(false);
            setShowCommitPushModal(true);
          }}
        />
      </Show>
      <Show when={showCommitPushModal()}>
        <CommitPushModal
          workspace={props.workspace}
          open={showCommitPushModal()}
          onClose={() => setShowCommitPushModal(false)}
        />
      </Show>
      <Show when={viewerFile()}>
        <ContentViewerModal
          contentType={viewerFile()!.type}
          filePath={viewerFile()!.path}
          title={viewerFile()!.title}
          workspace={props.workspace}
          onClose={() => setViewerFile(null)}
        />
      </Show>
      <Show when={showNetModal()}>
        <NetworkActivityModal workspace={props.workspace} onClose={() => setShowNetModal(false)} />
      </Show>
      <ToastPill message={toastMessage()} onDismiss={dismissToast} />
    </div>
  );
};

const ArchivedBlock: Component<{
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
          {t("chat.archived.title")}
        </span>
        <div class="h-px flex-1 bg-border-subtle" />
        <span class="text-[11px] text-ink-faint">
          {t("chat.archived.messages", String(props.messages.length))}
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
                    {msg.role === "user" ? t("chat.archived.you") : t("chat.archived.agent")}
                  </span>
                  <span class="text-[12px] text-ink-muted">
                    {msg.text.length > 120 ? `${msg.text.slice(0, 120)}…` : msg.text}
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

const ContextFooter: Component<{
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
      ? t(
          "chat.context.costBreakdown",
          props.costInput!.toFixed(4),
          props.costOutput!.toFixed(4),
          props.costCacheRead!.toFixed(4),
        )
      : t("chat.context.sessionCost");
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
      <div class="flex flex-1 items-center gap-2" title={t("chat.context.nextRequest")}>
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
          title={t("chat.context.sessionTokens")}
        >
          {t("chat.context.total", formatTokens(props.cumulativeTokens))}
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
          {t("chat.context.compact")}
        </button>
      </Show>

      <Show when={props.isCompacting}>
        <span class="flex items-center gap-1 text-[11px] text-accent">
          <span class="inline-block h-2 w-2 animate-pulse-soft rounded-full bg-accent" />
          {t("chat.context.compacting")}
        </span>
      </Show>
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
    const parts = [t("chat.timeline.workedFor", formatDuration(ms)), t("chat.timeline.steps", String(count), count === 1 ? "" : "s")];
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
                  {step.steering!.text.length > 50
                    ? `${step.steering!.text.slice(0, 50)}…`
                    : step.steering!.text}
                </span>
                <span class="text-[10px] text-ink-faint">{t("chat.timeline.steering")}</span>
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
                  ? t("golden.loop", String(step.golden!.cycle), String(step.golden!.maxCycles), String(step.golden!.pending.length))
                  : t("golden.loop.replay", String(step.golden!.cycle), String(step.golden!.pending.length))}
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
const LinkedRow: Component<{
  linked: NonNullable<TimelineItem["linked"]>;
}> = (props) => {
  const label = () => {
    if (props.linked.docOnly) return t("chat.linked.handoffDoc");
    switch (props.linked.reason) {
      case "plan_execution": return t("chat.linked.planExecution");
      case "golden_flip": return t("chat.linked.goldenFlip");
      case "context_handoff": return t("chat.linked.contextHandoff");
      case "manual_builder": return t("chat.linked.manualBuilder");
      default: return t("chat.linked.contextHandoff");
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
      case "running": return t("chat.subagent.running");
      case "completed": return t("chat.subagent.completed", String(props.subagent.rounds));
      case "failed": return t("chat.subagent.failed");
      case "interrupted": return t("chat.subagent.interrupted");
      case "max_rounds": return t("chat.subagent.maxRounds");
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
            <span class="shrink-0 text-[10px] font-semibold uppercase tracking-wider text-ink-faint">{t("chat.subagent.goal")}</span>
            <span class="truncate text-[11px] text-ink-muted">
              {props.subagent.goal.length > 80 ? props.subagent.goal.slice(0, 80) + "…" : props.subagent.goal}
            </span>
          </div>
        </Show>
        <Show when={props.subagent.report}>
          <div class="ml-6 flex items-start gap-1">
            <span class="shrink-0 text-[10px] font-semibold uppercase tracking-wider text-ink-faint">{t("chat.subagent.report")}</span>
            <span class="truncate text-[11px] text-ink-muted">
              {props.subagent.report!.length > 120 ? props.subagent.report!.slice(0, 120) + "…" : props.subagent.report}
            </span>
          </div>
        </Show>
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
      case "running": return t("chat.subagent.running");
      case "completed": return t("chat.subagent.completed", String(props.subagent.rounds));
      case "failed": return t("chat.subagent.failed");
      case "interrupted": return t("chat.subagent.interrupted");
      case "max_rounds": return t("chat.subagent.maxRounds");
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
              <span class="mb-1 block text-[10px] font-semibold uppercase tracking-wider text-ink-faint">{t("chat.subagent.goal")}</span>
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
              <span class="mb-1 block text-[10px] font-semibold uppercase tracking-wider text-ink-faint">{t("chat.subagent.report")}</span>
              <ProseContent
                class="prose-content text-[12px] leading-[1.6] text-ink-muted"
                html={marked.parse(props.subagent.report!, { async: false }) as string}
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

const PhaseRow: Component<{ phase: Phase }> = (props) => {
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

const PhaseResultRow: Component<{ phaseResult: { phase: Phase; text: string } }> = (props) => {
  return (
    <div class="my-1 ml-6">
      <ProseContent
        class="prose-content text-[12px] leading-[1.6] text-ink-muted"
        html={marked.parse(props.phaseResult.text, { async: false }) as string}
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
const TextRow: Component<{ text: string }> = (props) => {
  const substantial = () => props.text.length >= SUBSTANTIAL_TEXT_CHARS;
  return (
    <div class="my-1 ml-6">
      <ProseContent
        class={
          substantial()
            ? "prose-content text-[13px] leading-[1.6] text-ink"
            : "prose-content text-[12px] leading-[1.6] text-ink-muted"
        }
        html={marked.parse(props.text, { async: false }) as string}
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

const CompactionRow: Component<{ compaction: { kind: "start" | "done" | "fail" | "handoff_start" | "handoff_fail"; args: string[] } }> = (props) => {
  const iconName = (): IconName => {
    if (props.compaction.kind === "start" || props.compaction.kind === "handoff_start") return "package-process" as IconName;
    if (props.compaction.kind === "done") return "package" as IconName;
    return "package-out-of-stock" as IconName;
  };

  const label = () => {
    switch (props.compaction.kind) {
      case "start": return t("chat.compact.start", props.compaction.args[0], props.compaction.args[1]);
      case "done": return t("chat.compact.done", props.compaction.args[0], props.compaction.args[1]);
      case "handoff_start": return t("chat.handoff.start", props.compaction.args[0], props.compaction.args[1]);
      case "handoff_fail": return t("chat.handoff.fail", props.compaction.args[0]);
      default: return t("chat.compact.fail", props.compaction.args[0]);
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

const ThinkingBar: Component<{
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
          {t("chat.status.thinking")}
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

const ThinkingRow: Component<{
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
        <span class="text-[12px] text-ink-muted">{t("chat.timeline.thought")}</span>
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

const ToolRow: Component<{
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
            {showRaw() ? t("chat.timeline.hideRaw") : t("chat.timeline.showRaw")}
          </button>
          <Show when={showRaw()}>
            <div class="mt-1">
              <div class="mb-1 font-mono text-[11px] font-medium text-ink-muted">{t("chat.timeline.args")}</div>
              <pre class="mb-2 overflow-x-auto whitespace-pre-wrap font-mono text-[11px] text-ink-faint">
                {JSON.stringify(props.tool.call.args, null, 2)}
              </pre>
              <Show when={props.tool.result}>
                <div class="mb-1 font-mono text-[11px] font-medium text-ink-muted">{t("chat.timeline.result")}</div>
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


const ApprovalCard: Component<{
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
                  {t("chat.approval.proposedEdit")}
                </span>
                <span class="truncate font-mono text-[12px] text-ink-muted">
                  {proposal()?.path ?? (props.toolCall.args.path as string)}
                </span>
              </>
            }
          >
            <span class="rounded bg-amber-500/15 px-1.5 py-0.5 text-[10px] font-semibold text-amber-500">
              {t("chat.approval.bashCommand")}
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
          {t("chat.approval.approve")}
          <kbd class="rounded bg-accent-ink/15 px-1 font-mono text-[10px]">⏎</kbd>
        </button>
        <button
          onClick={props.onReject}
          class="flex flex-1 items-center justify-center gap-1.5 rounded-md border border-border-subtle bg-surface-0 px-3 py-1.5 text-sm text-ink hover:border-danger hover:text-danger"
        >
          <Icon name="x" class="h-4 w-4" />
          {t("chat.approval.reject")}
          <kbd class="rounded bg-surface-2 px-1 font-mono text-[10px]">esc</kbd>
        </button>
      </div>
    </div>
  );
};
