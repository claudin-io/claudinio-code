// Pure translation from the persisted session JSONL into the shape the chat
// timeline renders. Extracted from ChatPanel so it can be tested directly:
// these functions used to be duplicated into ChatPanel.test.ts, which meant the
// tests exercised copies rather than the code that ships.

import {
  normalizeSessionMode,
  type DoneData,
  type GoldenLoopData,
  type HandoffReason,
  type ModeChangedData,
  type Phase,
  type SessionMode,
  type SessionRecord,
  type ToolCallData,
  type ToolResultData,
} from "./ipc";

export type Status = "idle" | "thinking" | "awaiting_approval" | "awaiting_input" | "done" | "error";

export interface ArchivedBlock {
  summary: string;
  messages: ChatMessage[];
}

export interface ChatMessage {
  role: "user" | "assistant" | "archived";
  text: string;
  steps?: TimelineItem[];
  done?: DoneData;
  archived?: ArchivedBlock;
  /** Files attached to a user message, shown as pills in the chat bubble */
  attachments?: { name: string; mediaType: string; size: number }[];
}

export interface SubagentTimelineState {
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

export interface TimelineItem {
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

export interface QueuedSteeringEntry {
  text: string;
  attachments: Array<{ name: string; mediaType: string; size: number }>;
}

export const MODE_CHANGE_LABEL: Record<string, string> = {
  "brain.human": "Brain mode enabled",
  "builder.human": "Builder mode enabled",
  "brain.agent": "The agent entered Brain mode",
  "builder.agent": "The agent returned to Builder mode",
};

export function modeChangeLabel(mc: ModeChangedData): string {
  const label = MODE_CHANGE_LABEL[`${mc.mode}.${mc.origin}`] ?? `${mc.mode} mode enabled`;
  return mc.reason ? `${label} — Reason: ${mc.reason}` : label;
}

export const PHASE_LABEL = (phase: Phase): string => {
  switch (phase) {
    case "plan": return "Plan";
    case "execute": return "Execute";
    case "summary": return "Summary";
  }
};

export function formatTokens(n: number): string {
  if (n < 1000) return `${n}`;
  return `${(n / 1000).toFixed(n < 10000 ? 1 : 0)}k`;
}

/** Clip to `maxChars` and mark it, for the one-line previews in the timeline. */
export function ellipsize(text: string, maxChars: number): string {
  return text.length > maxChars ? `${text.slice(0, maxChars)}\u2026` : text;
}

export function formatDuration(ms: number): string {
  if (ms < 1000) return `${ms}ms`;
  return `${(ms / 1000).toFixed(1)}s`;
}

export interface ContentBlockJson {
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
export function isRealUserTurn(rec: SessionRecord): boolean {
  if (rec.kind !== "turn" || rec.role !== "user") return false;
  const content = (rec.content as ContentBlockJson[]) ?? [];
  return content.length > 0 && content[0].type === "text";
}

// Mirror of persist.rs tail_start_index: where the kept-verbatim tail of a
// Compacted marker begins. Expands backwards so the tail starts on a real
// user turn; drops the tail (returns compactIdx) when none exists.
export function tailStartIndex(recs: SessionRecord[], compactIdx: number, tailTurns: number): number {
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
export function normalizeCompactTails(records: SessionRecord[]): SessionRecord[] {
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
export const SUBSTANTIAL_TEXT_CHARS = 280;

// A short closing message after tool calls often just points at a longer
// explanation the model wrote mid-turn (before a trailing tasks_set). That
// explanation would otherwise stay hidden inside the collapsed trajectory, so
// hoist the last substantial text step into the visible answer.
export function promoteSubstantialText(
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

export function recordsToMessages(rawRecords: SessionRecord[]): ChatMessage[] {
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
