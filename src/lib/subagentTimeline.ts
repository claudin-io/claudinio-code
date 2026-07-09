// Pure state-transition helpers for subagent timeline entries.
//
// The main chat timeline (`currentSteps`) holds a *snapshot* of each subagent
// as a "subagent" item. A separate `subagentState` map holds the authoritative
// state that the detail modal reads. These two MUST be kept in sync: when a
// subagent finishes (SubagentDone) the timeline snapshot has to be updated too,
// otherwise the inline row stays stuck on "running" even though the subagent
// already reported and the parent turn moved on.
//
// These helpers are extracted so the transition is unit-testable without
// mounting the SolidJS component.

export type SubagentStatus =
  | "running"
  | "completed"
  | "failed"
  | "interrupted"
  | "max_rounds";

/// Minimal shape of a timeline step needed by the sync helpers. The real
/// `TimelineItem` in ChatPanel is structurally compatible.
export interface TimelineNode {
  type: string;
  thinking?: { text: string; startedAt: number; endedAt?: number };
  subagent?: SubagentNode;
}

/// Minimal shape of a subagent entry needed by these helpers.
export interface SubagentNode {
  id: string;
  status: SubagentStatus;
  rounds: number;
  inputTokens: number;
  outputTokens: number;
  report?: string;
  steps: TimelineNode[];
}

export interface SubagentDoneInput {
  subagentId: string;
  status: string;
  rounds: number;
  inputTokens: number;
  outputTokens: number;
  report?: string;
}

/// Map the backend's free-form status string onto a known terminal status.
/// Anything unrecognized is treated as a normal completion so a finished
/// subagent never lingers as "running".
export function mapSubagentDoneStatus(raw: string): SubagentStatus {
  switch (raw) {
    case "failed":
      return "failed";
    case "interrupted":
      return "interrupted";
    case "max_rounds":
      return "max_rounds";
    case "completed":
    default:
      return "completed";
  }
}

/// Close out any open "thinking" spinner steps so they stop animating once the
/// subagent is done.
export function markThinkingEnded<T extends TimelineNode>(steps: T[], now: number): T[] {
  return steps.map((s) =>
    s.type === "thinking" && s.thinking && s.thinking.endedAt === undefined
      ? { ...s, thinking: { ...s.thinking, endedAt: now } }
      : s,
  );
}

/// Apply a SubagentDone event to the subagent-state map, producing a new map in
/// which the target subagent carries its terminal status, final token/round
/// counts, report, and closed-out thinking steps.
export function applySubagentDone<S extends SubagentNode>(
  subagents: Record<string, S>,
  data: SubagentDoneInput,
  now: number,
): Record<string, S> {
  const sa = subagents[data.subagentId];
  if (!sa) return subagents;
  const updated: S = {
    ...sa,
    status: mapSubagentDoneStatus(data.status),
    rounds: data.rounds,
    inputTokens: data.inputTokens,
    outputTokens: data.outputTokens,
    report: data.report,
    steps: markThinkingEnded(sa.steps, now),
  };
  return { ...subagents, [data.subagentId]: updated };
}

/// Rewrite the "subagent" snapshot items inside a timeline (`currentSteps`) so
/// they match the authoritative subagent-state map. This is what makes the
/// inline row reflect the real status (e.g. "completed") instead of staying on
/// "running" after the subagent finishes.
export function syncSubagentTimelineItems<T extends TimelineNode>(
  steps: T[],
  subagents: Record<string, SubagentNode>,
): T[] {
  return steps.map((s) => {
    if (s.type === "subagent" && s.subagent) {
      const latest = subagents[s.subagent.id];
      if (latest) return { ...s, subagent: latest } as T;
    }
    return s;
  });
}
