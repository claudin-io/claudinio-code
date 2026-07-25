/// The shapes a session is persisted and streamed in.
///
/// Types and two normalizers, and deliberately nothing else: this is what both
/// the desktop app and the web peer need in order to read a transcript, so it
/// cannot sit in `lib/ipc.ts` where every type is a sibling of an `invoke` call.
///
/// These describe what the Rust side writes. Keeping them here rather than in
/// each consumer is what stops the web peer from growing its own guess at the
/// record format — a guess that would render an old session subtly wrong rather
/// than failing.

// Replay-only: old sessions may still have "plan" | "execute" | "summary"
// phase records on disk. No new session emits these.
export type Phase = "plan" | "execute" | "summary";

export interface SubagentStartedData {
  subagentId: string;
  parentToolId: string;
  name: string;
  goal: string;
  mode: string;
}

export interface SubagentDoneData {
  subagentId: string;
  status: string;
  rounds: number;
  inputTokens: number;
  outputTokens: number;
  cost: number;
  report?: string;
}

export type SessionMode = "brain" | "builder";

export type ThinkingEffort = "low" | "medium" | "high" | "xhigh" | "max";

/// Slider order, lowest to highest — index in this array is the range value.
export const THINKING_EFFORTS: ThinkingEffort[] = ["low", "medium", "high", "xhigh", "max"];

export function normalizeThinkingEffort(s: unknown): ThinkingEffort {
  return THINKING_EFFORTS.includes(s as ThinkingEffort) ? (s as ThinkingEffort) : "medium";
}

/// Map a persisted mode string to the current ids. Old session JSONLs carry
/// the original names "pensador"/"constructor".
export function normalizeSessionMode(s: unknown): SessionMode {
  return s === "brain" || s === "pensador" ? "brain" : "builder";
}
export type ModeOrigin = "human" | "agent";

export interface ModeChangedData {
  mode: SessionMode;
  origin: ModeOrigin;
  reason?: string | null;
}

export interface GoldenLoopData {
  cycle: number;
  maxCycles: number;
  pending: string[];
  mode: SessionMode;
}

/// Why a session handed off to a linked successor.
export type HandoffReason =
  | "plan_execution"
  | "golden_flip"
  | "context_handoff"
  | "manual_builder";

export interface SessionLinkedData {
  prevSessionId: string;
  sessionId: string;
  reason: HandoffReason;
  mode: SessionMode;
  firstMessage: string;
}

export type AgentEvent =
  | { event: "TextStep"; data: { text: string } }
  | { event: "TextDelta"; data: { text: string } }
  | { event: "ModeChanged"; data: ModeChangedData }
  | { event: "GoldenLoop"; data: GoldenLoopData }
  | { event: "SessionLinked"; data: SessionLinkedData }
  | { event: "Thinking"; data: string }
  | { event: "ToolCall"; data: ToolCallData }
  | { event: "ToolResult"; data: ToolResultData }
  | { event: "AskUser"; data: AskUserData }
  | { event: "Done"; data: DoneData }
  | { event: "SteeringInjected"; data: { text: string; attachments?: Array<{ name: string; mediaType: string; size: number }> } }
  | { event: "Error"; data: string }
  | { event: "Retrying"; data: RetryingData }
  | { event: "SubagentStarted"; data: SubagentStartedData }
  | { event: "SubagentDone"; data: SubagentDoneData }
  | { event: "Subagent"; data: { subagentId: string; event: AgentEvent } }
  | {
      event: "SessionStats";
      data: {
        inputTokens: number;
        outputTokens: number;
        cumulativeCost?: number;
        costInput?: number;
        costOutput?: number;
        costCacheRead?: number;
        contextTokens: number;
        maxContextTokens: number;
        compactThreshold: number;
      };
    };

/** Transient provider failure being retried with backoff (claudin.io
 * failover can take ~2min) — the UI shows a reconnecting banner instead of
 * dropping the run. */
export interface RetryingData {
  attempt: number;
  maxAttempts: number;
  delayMs: number;
  error: string;
}

export interface AskUserOption {
  /** Concise choice shown on the button. */
  label: string;
  /** Optional one-line explanation rendered under the label. */
  description?: string;
}

export interface AskUserQuestion {
  question: string;
  options: AskUserOption[];
  multi_select?: boolean;
}

export interface AskUserData {
  sessionId: string;
  toolId: string;
  questions: AskUserQuestion[];
}

export interface UserAnswer {
  question: string;
  answer: string;
}

export interface ToolCallData {
  sessionId: string;
  toolId: string;
  toolName: string;
  args: Record<string, unknown>;
  permission: string;
  editProposal?: EditProposalData | null;
}

export type ChatStep =
  | { type: "thinking"; text: string }
  | { type: "tool_call"; data: ToolCallData }
  | { type: "tool_result"; data: ToolResultData }
  | { type: "steering"; text: string };

export interface EditProposalData {
  path: string;
  oldString: string;
  newString: string;
  unifiedDiff: string;
}

export interface ToolResultData {
  toolId: string;
  toolName: string;
  output: string;
  error?: string | null;
}

export interface DoneData {
  stopReason: string;
  textOutput: string;
  inputTokens: number;
  outputTokens: number;
}

export interface SessionSummary {
  sessionId: string;
  createdAt: number;
  updatedAt: number;
  title: string;
  turnCount: number;
}

// One line of a session JSONL file. `kind` discriminates the variant; extra
// fields depend on the kind (see the Rust SessionRecord enum).
export type SessionRecord = {
  kind: "meta" | "user" | "phase" | "turn" | "phase_result" | "done" | "error" | "steering" | "compacted" | "status" | "mode" | "tasks" | "golden_cycle" | "continuation_judge" | "base_commit" | "plan_finalized" | "linked_from" | "handoff_to" | "handoff";
  [key: string]: unknown;
};
