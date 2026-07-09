import { createEffect, createSignal, For, onCleanup, onMount, Show, type Component } from "solid-js";
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
  compactSession,
  getSessionStats,
  getConfig,
  loginWithClaudinio,
  openExternalUrl,
  readAttachment,
  setSessionMode,
  normalizeSessionMode,
  pickFiles,
  type ModeOrigin,
  type SessionMode,
  type ModeChangedData,
  type GoldenLoopData,
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
import { applySubagentDone, syncSubagentTimelineItems } from "../lib/subagentTimeline";
import { marked } from "marked";
import hljs from "highlight.js";
import { DiffViewer } from "./DiffViewer";
import { Icon, toolIcon, type IconName } from "./Icon";
import TextEditorModal from "./TextEditorModal";
import { FileMentionPopover } from "./FileMentionPopover";
import { TagMentionPopover } from "./TagMentionPopover";
import { SkillMentionPopover } from "./SkillMentionPopover";
import ContextWarning from "./ContextWarning";
import { getCurrentWindow } from "@tauri-apps/api/window";
import { t } from "../lib/grill-me";
import { setWorkspaceStatus } from "../lib/workspaceStatus";

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
  report?: string;
  steps: TimelineItem[];
}

interface TimelineItem {
  type: "thinking" | "tool" | "phase" | "phase_result" | "text" | "steering" | "subagent" | "compaction" | "mode" | "golden";
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
  compaction?: {
    kind: "start" | "done" | "fail";
    args: string[];
  };
  modeChange?: ModeChangedData;
  golden?: GoldenLoopData;
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

function detectLanguageFromPath(path: string): string {
  if (path.endsWith(".ts") || path.endsWith(".tsx")) return "typescript";
  if (path.endsWith(".rs")) return "rust";
  if (path.endsWith(".py")) return "python";
  if (path.endsWith(".swift")) return "swift";
  if (path.endsWith(".js") || path.endsWith(".jsx")) return "javascript";
  if (path.endsWith(".css")) return "css";
  if (path.endsWith(".json")) return "json";
  if (path.endsWith(".html")) return "html";
  return "plaintext";
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

  for (const rec of records) {
    const kind = rec.kind;
    if (kind === "compacted") {
      // Flush current assistant message into preCompact pile
      flush();
      // Wrap all pre-compact messages into an ArchivedBlock
      if (preCompact.length > 0) {
        out.push({
          role: "archived",
          text: "",
          archived: {
            summary: String(rec.summary ?? ""),
            messages: [...preCompact],
          },
        });
        preCompact = [];
      }
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
        steering: { text: String(rec.text ?? "") },
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

export const ChatPanel: Component<{
  /// Root path of the workspace this panel belongs to. One panel is mounted
  /// per open workspace; hidden ones keep streaming their run's events.
  workspace: string;
  /// Whether this panel is the visible one. Global listeners (ESC interrupt,
  /// drag-drop) must only act on the active panel.
  isActive: () => boolean;
  /// Flat list of all workspace files for @-mention autocomplete.
  fileList: string[];
}> = (props) => {
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
  const [retryableError, setRetryableError] = createSignal<string | null>(null);
  // Budget do plano estourado: mostra banner de upgrade em vez do retry bar.
  const isBudgetError = () => retryableError()?.startsWith("BUDGET_EXCEEDED::") ?? false;
  // Attachments to send with the next message
  const [attachments, setAttachments] = createSignal<{ name: string; path: string; mediaType: string; size: number }[]>([]);
  const [showEditor, setShowEditor] = createSignal(false);
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

  // Human toggle: persists a Mode record in the session JSONL immediately so
  // the mode survives reloads; a running workflow picks it up next round.
  const switchMode = async (m: SessionMode) => {
    if (m === mode()) return;
    setMode(m);
    setCurrentSteps((prev) => [
      ...prev,
      { type: "mode" as const, modeChange: { mode: m, origin: "human" as const } },
    ]);
    try {
      const result = await setSessionMode(props.workspace, m);
      setActiveSessionId(result.sessionId);
    } catch {
      // backend unavailable — sendMessage will sync the mode on next send
    }
  };

  // Auto-switch to Builder mode and send "Execute the plan" when the user
  // clicks the "Continue with Builder" button after a Brain planning session.
  const continueWithBuilder = async () => {
    try {
      await switchMode("builder");
      const msg = t("mode.continueMessage");
      setMessages((prev) => [
        ...prev,
        { role: "user" as const, text: msg },
      ]);
      setCurrentSteps([]);
      setThinkingStart(0);
      setStatus("thinking");
      scrollToBottom(true);
      const result = await sendMessage(
        props.workspace,
        msg,
        [],
        handleEvent,
        "builder",
      );
      setActiveSessionId(result.sessionId);
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
        // Compute caret pixel position for the skill popover
        const pos = getCaretCoordinates(el, insertionEnd);
        const POPOVER_WIDTH = 320;
        const MARGIN = 8;
        // Position above: compute bottom distance from viewport bottom
        const bottom = window.innerHeight - pos.top + 4;
        let left = pos.left;
        const maxLeft = window.innerWidth - POPOVER_WIDTH - MARGIN;
        if (left > maxLeft) left = maxLeft;
        if (left < MARGIN) left = MARGIN;
        setSkillQuery("");
        setSkillPosition({ top: bottom, left, height: pos.height });
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
      } else {
        setCurrentSteps((prev) => [...prev, { type: "text", text }]);
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
          steps: [{ type: "thinking" as const, thinking: { text: t("chat.timeline.waiting"), startedAt: now } }],
        },
      }));
      setCurrentSteps((prev) => [
        ...prev,
        { type: "subagent" as const, subagent: { id: d.subagentId, name: d.name, goal: d.goal, mode: d.mode, status: "running", rounds: 0, inputTokens: 0, outputTokens: 0, steps: [] } },
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
      // Stats are NOT recomputed here: the last SessionStats event from the
      // backend already carries the authoritative numbers.
      const promoted = promoteSubstantialText(final, data.textOutput);
      setMessages((prev) => [
        ...prev,
        { role: "assistant" as const, text: promoted.text, steps: promoted.steps, done: data },
      ]);
      setCurrentSteps([]);
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
    } else if (event.event === "Error") {
      // Don't show error in message list — we render an error bar below input
      setRetryableError(event.data);
      setCurrentSteps([]);
      setThinkingStart(0);
      setSubagentState({});
      setStatus("error");
    }
  };

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

  const send = async () => {
    const text = input().trim();
    if (!text || isCompacting() || status() === "awaiting_approval" || status() === "awaiting_input") return;

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
    if (status() === "thinking" || status() === "awaiting_approval" || status() === "awaiting_input") return;
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

  const reopenSession = async (id: string) => {
    if (status() === "thinking" || status() === "awaiting_approval" || status() === "awaiting_input") return;
    try {
      const records = await loadSession(props.workspace, id);
      setMessages(recordsToMessages(records));
      statsFromRecords(records);
      // The JSONL is the source of truth for the mode too: restore the last one.
      const lastMode = [...records].reverse().find((r) => r.kind === "mode");
      setMode(lastMode ? normalizeSessionMode(lastMode.mode) : "builder");
      setActiveSessionId(id);
      setCurrentSteps([]);
      setThinkingStart(0);
      setStatus("idle");
      setShowSessions(false);
      scrollToBottom(true);
    } catch (e) {
      setMessages((prev) => [...prev, { role: "user", text: t("chat.message.failedToReopen", String(e)) }]);
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
      case "thinking": return t("chat.status.thinking");
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
            onClick={startNewSession}
            disabled={status() === "thinking" || status() === "awaiting_approval"}
            class="flex items-center gap-1 rounded px-2 py-1 text-[11px] text-ink-muted hover:bg-surface-2 disabled:opacity-30"
            title={t("chat.header.newSession")}
          >
            <Icon name="plus" class="h-3.5 w-3.5" />
            {t("chat.header.new")}
          </button>
          <ContextWarning workspace={props.workspace} />
          <button
            onClick={toggleSessions}
            class="flex items-center gap-1 rounded px-2 py-1 text-[11px] text-ink-muted hover:bg-surface-2"
            title={t("chat.header.savedSessions")}
          >
            <Icon name="clock" class="h-3.5 w-3.5" />
            {t("chat.header.history")}
          </button>
        </div>

        <Show when={showSessions()}>
          <div class="absolute right-4 top-9 z-20 max-h-80 w-80 overflow-y-auto rounded-lg border border-border-subtle bg-surface-1 py-1 shadow-lg">
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
        </Show>
      </div>

      <div class="relative flex flex-1 flex-col overflow-hidden">
        <div
          ref={scrollContainerRef}
          onScroll={handleScroll}
          class="flex flex-1 flex-col overflow-y-auto"
        >
          <div class="w-full px-6 py-4">
          <For each={messages()}>
            {(msg) => (
              <div class="mb-6">
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
                    <p class="whitespace-pre-wrap break-words text-[13px] leading-[1.65] text-ink">
                      {msg.text}
                    </p>
                    <Show when={msg.attachments && msg.attachments!.length > 0}>
                      <div class="mt-2 flex flex-wrap gap-1.5">
                        <For each={msg.attachments!}>
                          {(att) => (
                            <span class="inline-flex items-center gap-1 rounded-md border border-accent/20 bg-accent/[0.06] px-1.5 py-0.5 text-[11px] text-ink-muted">
                              <Icon
                                name={att.mediaType.startsWith("image/") ? "image" : "file-text"}
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
                    <div
                      class="prose-content text-[13px] text-ink"
                      innerHTML={marked.parse(msg.text, { async: false }) as string}
                    />
                  </Show>
                </Show>

                <Show when={msg.role === "archived" && msg.archived}>
                  <ArchivedBlock
                    summary={msg.archived!.summary}
                    messages={msg.archived!.messages}
                  />
                </Show>
              </div>
            )}
          </For>

          <Show when={mode() === "brain" && modeOrigin() === "human" && status() === "done"}>
            <div class="mb-6 flex justify-center">
              <button
                onClick={continueWithBuilder}
                class="inline-flex items-center gap-2 rounded-full bg-accent px-5 py-2.5 text-sm font-semibold text-accent-ink shadow-lg shadow-accent/20 transition-all hover:bg-accent/90 hover:shadow-xl hover:shadow-accent/30 active:scale-[0.98]"
              >
                <span class="inline-flex h-4 w-4 items-center justify-center">
                  <span class="i-lucide:construction-worker h-4 w-4" />
                </span>
                {t("mode.continueWithBuilder")}
              </button>
            </div>
          </Show>

          <Show when={status() === "thinking" || status() === "done" || status() === "awaiting_input"}>
            <div class="mb-6">
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

      <Show when={attachments().length > 0}>
        <div class="flex flex-wrap gap-2 border-t border-border-subtle px-6 py-2">
          <For each={attachments()}>
            {(att, i) => (
              <div class="group flex items-center gap-1.5 rounded-md border border-border-subtle bg-surface-1 px-2 py-1 text-xs text-ink-muted hover:border-accent/40">
                <Icon
                  name={att.mediaType.startsWith("image/") ? "image" : "file-text"}
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
          <div class="flex items-center gap-2 rounded-lg border border-border-subtle bg-surface-2 p-2 focus-within:border-accent/60">
            <button
              onClick={() => setShowEditor(true)}
              disabled={isCompacting() || status() === "awaiting_approval" || status() === "awaiting_input"}
              class="flex h-8 w-8 shrink-0 items-center justify-center rounded-md text-ink-muted hover:bg-surface-3 hover:text-accent disabled:opacity-30"
              title={t("editor.open")}
            >
              <Icon name="notebook-pen" class="h-4 w-4" stroke />
            </button>
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
                  // Compute caret pixel position using mirror div
                  const pos = getCaretCoordinates(textarea, caret);
                  // Smart positioning: default above, flip below if not enough room
                  const POPOVER_ESTIMATED_HEIGHT = 260; // max list + padding + shadow
                  const POPOVER_WIDTH = 280; // min-width
                  const MARGIN = 8;

                  let top: number;
                  if (pos.top - POPOVER_ESTIMATED_HEIGHT >= MARGIN) {
                    // Room above — show above
                    top = pos.top - POPOVER_ESTIMATED_HEIGHT;
                  } else {
                    // Not enough above — show below
                    top = pos.top + pos.height + 4;
                  }

                  // Clamp left so popover doesn't overflow viewport edges
                  let left = pos.left;
                  const maxLeft = window.innerWidth - POPOVER_WIDTH - MARGIN;
                  if (left > maxLeft) left = maxLeft;
                  if (left < MARGIN) left = MARGIN;

                  setMentionQuery(query);
                  setMentionPosition({ top, left, height: pos.height });
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
                  // Position popover right above the < character (ltIdx),
                  // not at the caret — so the popover hugs the < symbol.
                  const pos = getCaretCoordinates(textarea, ltIdx + 1);
                  const POPOVER_WIDTH = 220;
                  const MARGIN = 8;
                  // Use bottom positioning: popover grows upward from 4px above the < line
                  const bottom = window.innerHeight - pos.top + 4;
                  let left = pos.left;
                  const maxLeft = window.innerWidth - POPOVER_WIDTH - MARGIN;
                  if (left > maxLeft) left = maxLeft;
                  if (left < MARGIN) left = MARGIN;
                  setTagQuery(query);
                  setTagPosition({ top: bottom, left, height: pos.height }); // reuse as { bottom, left }
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
                    const POPOVER_WIDTH = 340;
                    const MARGIN = 8;
                    // Position above: compute bottom distance from viewport bottom
                    const bottom = window.innerHeight - pos.top + 4;
                    let left = pos.left;
                    const maxLeft = window.innerWidth - POPOVER_WIDTH - MARGIN;
                    if (left > maxLeft) left = maxLeft;
                    if (left < MARGIN) left = MARGIN;
                    setSkillQuery(skillQ);
                    setSkillPosition({ top: bottom, left, height: pos.height });
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
              class="max-h-[156px] min-h-[32px] flex-1 resize-none border-0 bg-transparent px-1 py-1.5 text-[13px] leading-[18px] text-ink placeholder:text-ink-faint focus:outline-none disabled:opacity-50"
              rows={1}
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

      <Show when={mentionPosition() !== null && props.fileList.length > 0}>
        <FileMentionPopover
          fileList={props.fileList}
          position={mentionPosition()!}
          query={mentionQuery()}
          onSelect={handleMentionSelect}
          onClose={() => setMentionPosition(null)}
        />
      </Show>

      <Show when={tagPosition() !== null && tagFlowStep() === "tag"}>
        <TagMentionPopover
          bottom={tagPosition()!.top}
          left={tagPosition()!.left}
          query={tagQuery()}
          onSelect={handleTagSelect}
          onClose={handlePopoverClose}
        />
      </Show>

      <Show when={skillPosition() !== null && tagFlowStep() === "skill"}>
        <SkillMentionPopover
          workspace={props.workspace}
          bottom={skillPosition()!.top}
          left={skillPosition()!.left}
          query={skillQuery()}
          onSelect={handleSkillSelect}
          onClose={handlePopoverClose}
        />
      </Show>

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
          props.costInput!.toFixed(2),
          props.costOutput!.toFixed(2),
          props.costCacheRead!.toFixed(2),
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
          ~${props.estimatedCost!.toFixed(2)}
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
              <span class="text-[10px] text-ink-faint">{t("chat.timeline.steering")}</span>
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
              <div
                class="prose-content text-[12px] leading-[1.6] text-ink-muted"
                innerHTML={marked.parse(props.subagent.report!, { async: false }) as string}
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
      <div
        class="prose-content text-[12px] leading-[1.6] text-ink-muted"
        innerHTML={marked.parse(props.phaseResult.text, { async: false }) as string}
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
      <div
        class={
          substantial()
            ? "prose-content text-[13px] leading-[1.6] text-ink"
            : "prose-content text-[12px] leading-[1.6] text-ink-muted"
        }
        innerHTML={marked.parse(props.text, { async: false }) as string}
      />
    </div>
  );
};

const CompactionRow: Component<{ compaction: { kind: "start" | "done" | "fail"; args: string[] } }> = (props) => {
  const iconName = (): IconName => {
    if (props.compaction.kind === "start") return "package-process" as IconName;
    if (props.compaction.kind === "done") return "package" as IconName;
    return "package-out-of-stock" as IconName;
  };

  const label = () => {
    if (props.compaction.kind === "start") return t("chat.compact.start", props.compaction.args[0], props.compaction.args[1]);
    if (props.compaction.kind === "done") return t("chat.compact.done", props.compaction.args[0], props.compaction.args[1]);
    return t("chat.compact.fail", props.compaction.args[0]);
  };

  const colorClass = () => {
    if (props.compaction.kind === "start") return "text-accent";
    if (props.compaction.kind === "done") return "text-success";
    return "text-danger";
  };

  const isStroke = () => props.compaction.kind === "start" || props.compaction.kind === "fail";

  return (
    <div class="my-2 ml-4 border-l-2 border-current pl-2" classList={{
      "border-accent/40": props.compaction.kind === "start",
      "border-success/40": props.compaction.kind === "done",
      "border-danger/40": props.compaction.kind === "fail",
    }}>
      <div class="flex items-center gap-2 px-1 py-1 text-[12px]">
        <span class={`trajectory-node flex h-5 w-5 shrink-0 items-center justify-center ${colorClass()}`}>
          <Icon name={iconName()} class={`h-[14px] w-[14px] ${colorClass()}`} stroke={isStroke()} />
        </span>
        <span class="text-ink-muted">{label()}</span>
        <Show when={props.compaction.kind === "start"}>
          <span class="inline-block h-2 w-2 animate-pulse-soft rounded-full bg-accent" />
        </Show>
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
        <span class="text-[12px] text-ink-muted">{t("chat.timeline.thought")}</span>
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
  const isEditFile = () => props.tool.call.toolName === "edit_file";

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
        onClick={isEditFile() ? undefined : props.onToggle}
        class="flex h-7 w-full items-center gap-2 rounded px-1 text-xs hover:bg-surface-2"
        classList={{ "cursor-default": isEditFile() }}
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
            class={`h-3 w-3 text-ink-faint transition-transform duration-120 ${isEditFile() || props.isExpanded ? "rotate-90" : ""}`}
          />
        </div>
      </button>
      <Show when={isEditFile() || props.isExpanded}>
        <div class="ml-6 rounded-md bg-surface-1 p-2 text-xs">
          <Show when={isEditFile()}>
            <div class="mb-3 overflow-hidden rounded border border-border-subtle">
              <DiffViewer
                original={props.tool.call.args.old_string as string ?? ""}
                modified={props.tool.call.args.new_string as string ?? ""}
                language={detectLanguageFromPath(props.tool.call.args.path as string ?? "")}
                inline
              />
            </div>
          </Show>
          <Show when={!isEditFile()}>
            <div class="mb-1 font-mono text-[11px] font-medium text-ink-muted">{t("chat.timeline.args")}</div>
            <pre class="mb-2 overflow-x-auto whitespace-pre-wrap font-mono text-[11px] text-ink-faint">
              {JSON.stringify(props.tool.call.args, null, 2)}
            </pre>
          </Show>
          <Show when={props.tool.result}>
            <div class="mb-1 font-mono text-[11px] font-medium text-ink-muted">{t("chat.timeline.result")}</div>
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
          {t("chat.question.needsAnswer")}
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
                  {t("chat.question.other")}
                </button>

                <Show when={draft().otherSelected}>
                  <input
                    type="text"
                    value={draft().otherText}
                    onInput={(e) => updateDraft(qi(), { otherText: e.currentTarget.value })}
                    onKeyDown={(e) => {
                      if (e.key === "Enter") submit();
                    }}
                    placeholder={t("chat.question.typeAnswer")}
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
        {t("chat.question.submit")}
      </button>
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
