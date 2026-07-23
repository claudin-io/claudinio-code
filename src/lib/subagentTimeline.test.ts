import { describe, it, expect } from "vitest";
import {
  mapSubagentDoneStatus,
  markThinkingEnded,
  applySubagentDone,
  syncSubagentTimelineItems,
} from "./subagentTimeline";
import type {
  SubagentStatus,
  SubagentNode,
  TimelineNode,
  SubagentDoneInput,
} from "./subagentTimeline";

// ---------------------------------------------------------------------------
// Helpers
// ---------------------------------------------------------------------------

function createSubagent(overrides: Partial<SubagentNode> = {}): SubagentNode {
  return {
    id: "sa-1",
    status: "running",
    rounds: 0,
    inputTokens: 0,
    outputTokens: 0,
    cost: 0,
    steps: [],
    ...overrides,
  };
}

function createThinkingStep(
  overrides: Partial<TimelineNode> = {},
): TimelineNode {
  return {
    type: "thinking",
    thinking: { text: "...", startedAt: 100, endedAt: undefined },
    ...overrides,
  };
}

// ---------------------------------------------------------------------------
// mapSubagentDoneStatus
// ---------------------------------------------------------------------------

describe("mapSubagentDoneStatus", () => {
  it("maps 'failed' to 'failed'", () => {
    expect(mapSubagentDoneStatus("failed")).toBe<SubagentStatus>("failed");
  });

  it("maps 'interrupted' to 'interrupted'", () => {
    expect(mapSubagentDoneStatus("interrupted")).toBe<SubagentStatus>(
      "interrupted",
    );
  });

  it("maps 'max_rounds' to 'max_rounds'", () => {
    expect(mapSubagentDoneStatus("max_rounds")).toBe<SubagentStatus>(
      "max_rounds",
    );
  });

  it("maps 'completed' to 'completed'", () => {
    expect(mapSubagentDoneStatus("completed")).toBe<SubagentStatus>(
      "completed",
    );
  });

  it("defaults empty string to 'completed'", () => {
    expect(mapSubagentDoneStatus("")).toBe<SubagentStatus>("completed");
  });

  it("defaults unknown/garbage string to 'completed'", () => {
    expect(mapSubagentDoneStatus("garbage")).toBe<SubagentStatus>("completed");
  });
});

// ---------------------------------------------------------------------------
// markThinkingEnded
// ---------------------------------------------------------------------------

describe("markThinkingEnded", () => {
  it("closes an open thinking step with endedAt = now", () => {
    const step = createThinkingStep();
    const now = 999;

    const result = markThinkingEnded([step], now);

    expect(result).toHaveLength(1);
    expect(result[0].thinking?.endedAt).toBe(now);
  });

  it("leaves an already-closed thinking step unchanged", () => {
    const step = createThinkingStep({
      thinking: { text: "...", startedAt: 100, endedAt: 200 },
    });
    const now = 999;

    const result = markThinkingEnded([step], now);

    expect(result).toHaveLength(1);
    expect(result[0].thinking?.endedAt).toBe(200);
  });

  it("ignores non-thinking steps (type !== 'thinking')", () => {
    const step: TimelineNode = { type: "message" };
    const now = 999;

    const result = markThinkingEnded([step], now);

    expect(result).toHaveLength(1);
    expect(result[0]).toBe(step); // same reference — unchanged
  });

  it("ignores thinking steps with no thinking object", () => {
    const step: TimelineNode = { type: "thinking" };
    const now = 999;

    const result = markThinkingEnded([step], now);

    expect(result).toHaveLength(1);
    expect(result[0]).toBe(step);
  });

  it("returns empty array unchanged", () => {
    const result = markThinkingEnded([], 999);

    expect(result).toEqual([]);
  });

  it("preserves non-thinking items when mixed with open thinking steps", () => {
    const message: TimelineNode = { type: "message" };
    const thinking = createThinkingStep();
    const now = 500;

    const result = markThinkingEnded([message, thinking], now);

    expect(result).toHaveLength(2);
    // message unchanged
    expect(result[0]).toBe(message);
    // thinking closed
    expect(result[1].thinking?.endedAt).toBe(now);
  });
});

// ---------------------------------------------------------------------------
// applySubagentDone
// ---------------------------------------------------------------------------

describe("applySubagentDone", () => {
  const now = 5000;

  it("updates the matching subagent with all fields", () => {
    const sa = createSubagent({ steps: [createThinkingStep()] });
    const map: Record<string, SubagentNode> = { "sa-1": sa };
    const input: SubagentDoneInput = {
      subagentId: "sa-1",
      status: "completed",
      rounds: 5,
      inputTokens: 1000,
      outputTokens: 2000,
      cost: 0,
      report: "All done.",
    };

    const result = applySubagentDone(map, input, now);
    const updated = result["sa-1"];

    expect(updated.status).toBe("completed");
    expect(updated.rounds).toBe(5);
    expect(updated.inputTokens).toBe(1000);
    expect(updated.outputTokens).toBe(2000);
    expect(updated.report).toBe("All done.");
  });

  it("closes thinking steps on the target subagent", () => {
    const sa = createSubagent({ steps: [createThinkingStep()] });
    const map: Record<string, SubagentNode> = { "sa-1": sa };
    const input: SubagentDoneInput = {
      subagentId: "sa-1",
      status: "completed",
      rounds: 0,
      inputTokens: 0,
      outputTokens: 0,
      cost: 0,
    };

    const result = applySubagentDone(map, input, now);

    expect(result["sa-1"].steps[0].thinking?.endedAt).toBe(now);
  });

  it("returns map unchanged when subagentId is not found", () => {
    const map: Record<string, SubagentNode> = { "sa-1": createSubagent() };
    const input: SubagentDoneInput = {
      subagentId: "sa-unknown",
      status: "completed",
      rounds: 0,
      inputTokens: 0,
      outputTokens: 0,
      cost: 0,
    };

    const result = applySubagentDone(map, input, now);

    // Same reference — nothing mutated
    expect(result).toBe(map);
  });

  it("preserves other subagents in the map unchanged", () => {
    const sa1 = createSubagent({ id: "sa-1" });
    const sa2 = createSubagent({ id: "sa-2" });
    const map: Record<string, SubagentNode> = { "sa-1": sa1, "sa-2": sa2 };
    const input: SubagentDoneInput = {
      subagentId: "sa-1",
      status: "interrupted",
      rounds: 3,
      inputTokens: 500,
      outputTokens: 750,
      cost: 0,
    };

    const result = applySubagentDone(map, input, now);

    expect(result["sa-2"]).toBe(sa2);
    expect(result["sa-2"].status).toBe("running");
  });

  it("maps status through mapSubagentDoneStatus (handles default)", () => {
    const sa = createSubagent();
    const map: Record<string, SubagentNode> = { "sa-1": sa };
    const input: SubagentDoneInput = {
      subagentId: "sa-1",
      status: "garbage",
      rounds: 0,
      inputTokens: 0,
      outputTokens: 0,
      cost: 0,
    };

    const result = applySubagentDone(map, input, now);

    expect(result["sa-1"].status).toBe("completed");
  });

  it("writes undefined report when not provided", () => {
    const sa = createSubagent();
    const map: Record<string, SubagentNode> = { "sa-1": sa };
    const input: SubagentDoneInput = {
      subagentId: "sa-1",
      status: "failed",
      rounds: 1,
      inputTokens: 100,
      outputTokens: 200,
      cost: 0,
    };

    const result = applySubagentDone(map, input, now);

    expect(result["sa-1"].report).toBeUndefined();
  });
});

// ---------------------------------------------------------------------------
// syncSubagentTimelineItems
// ---------------------------------------------------------------------------

describe("syncSubagentTimelineItems", () => {
  it("updates subagent items matching from the map", () => {
    const sa = createSubagent({ status: "completed", rounds: 3 });
    const steps: TimelineNode[] = [
      { type: "subagent", subagent: createSubagent({ id: "sa-1" }) },
    ];
    const map: Record<string, SubagentNode> = { "sa-1": sa };

    const result = syncSubagentTimelineItems(steps, map);

    expect(result).toHaveLength(1);
    expect(result[0].subagent?.status).toBe("completed");
    expect(result[0].subagent?.rounds).toBe(3);
  });

  it("ignores non-subagent items (type !== 'subagent')", () => {
    const msg: TimelineNode = { type: "message" };
    const steps: TimelineNode[] = [msg];
    const map: Record<string, SubagentNode> = {
      "sa-1": createSubagent({ status: "completed" }),
    };

    const result = syncSubagentTimelineItems(steps, map);

    expect(result).toHaveLength(1);
    expect(result[0]).toBe(msg); // same reference
  });

  it("ignores subagent items with no subagent property", () => {
    const step: TimelineNode = { type: "subagent" };
    const steps: TimelineNode[] = [step];
    const map: Record<string, SubagentNode> = {
      "sa-1": createSubagent({ status: "completed" }),
    };

    const result = syncSubagentTimelineItems(steps, map);

    expect(result).toHaveLength(1);
    expect(result[0]).toBe(step);
  });

  it("returns empty array unchanged", () => {
    const result = syncSubagentTimelineItems([], {});

    expect(result).toEqual([]);
  });

  it("leaves items unchanged when subagentId is not in the map", () => {
    const step: TimelineNode = {
      type: "subagent",
      subagent: createSubagent({ id: "sa-unknown" }),
    };
    const steps: TimelineNode[] = [step];
    const map: Record<string, SubagentNode> = {
      "sa-1": createSubagent({ status: "completed" }),
    };

    const result = syncSubagentTimelineItems(steps, map);

    expect(result).toHaveLength(1);
    expect(result[0]).toBe(step);
  });

  it("preserves non-subagent items when updating subagent items", () => {
    const msg: TimelineNode = { type: "message" };
    const updated = createSubagent({ id: "sa-1", status: "failed" });
    const steps: TimelineNode[] = [
      msg,
      { type: "subagent", subagent: createSubagent({ id: "sa-1" }) },
    ];
    const map: Record<string, SubagentNode> = { "sa-1": updated };

    const result = syncSubagentTimelineItems(steps, map);

    expect(result).toHaveLength(2);
    expect(result[0]).toBe(msg);
    expect(result[1].subagent?.status).toBe("failed");
  });

  it("preserves item identity when the mapped subagent is unchanged (reference equal)", () => {
    const sa = createSubagent({ id: "sa-1", status: "running" });
    const step: TimelineNode = { type: "subagent", subagent: sa };
    const steps: TimelineNode[] = [step];
    const map: Record<string, SubagentNode> = { "sa-1": sa };

    const result = syncSubagentTimelineItems(steps, map);

    expect(result[0]).toBe(step);
  });

  it("only recreates the item whose subagent actually changed, leaving others' identity intact", () => {
    const saA = createSubagent({ id: "sa-a", status: "running" });
    const saBOld = createSubagent({ id: "sa-b", status: "running" });
    const saBNew = createSubagent({ id: "sa-b", status: "completed" });
    const stepA: TimelineNode = { type: "subagent", subagent: saA };
    const stepB: TimelineNode = { type: "subagent", subagent: saBOld };
    const steps: TimelineNode[] = [stepA, stepB];
    const map: Record<string, SubagentNode> = { "sa-a": saA, "sa-b": saBNew };

    const result = syncSubagentTimelineItems(steps, map);

    expect(result[0]).toBe(stepA);
    expect(result[1]).not.toBe(stepB);
    expect(result[1].subagent?.status).toBe("completed");
  });
});
