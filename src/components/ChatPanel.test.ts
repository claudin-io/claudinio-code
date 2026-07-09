import { describe, it, expect } from "vitest";
import {
  applySubagentDone,
  syncSubagentTimelineItems,
  mapSubagentDoneStatus,
  markThinkingEnded,
  type SubagentNode,
  type TimelineNode,
} from "../lib/subagentTimeline";

// ── Pure functions extracted from ChatPanel for testing ────────────

function formatTokens(n: number): string {
  if (n < 1000) return `${n}`;
  return `${(n / 1000).toFixed(n < 10000 ? 1 : 0)}k`;
}

function formatDuration(ms: number): string {
  if (ms < 1000) return `${ms}ms`;
  return `${(ms / 1000).toFixed(1)}s`;
}

function summarizeArgs(args: Record<string, unknown>): string {
  const path = args.path as string | undefined;
  if (path) return path;
  const pattern = args.pattern as string | undefined;
  if (pattern) return `/${pattern}/`;
  const content = args.content as string | undefined;
  if (content) return `${content.slice(0, 60)}\u2026`;
  return JSON.stringify(args).slice(0, 80);
}

function truncate(text: string, maxChars: number): string {
  if (text.length > maxChars) {
    return text.slice(0, maxChars) + "\u2026";
  }
  return text;
}

// ── formatTokens tests ─────────────────────────────────────────────

describe("formatTokens", () => {
  it("formats < 1000 as raw number", () => {
    expect(formatTokens(0)).toBe("0");
    expect(formatTokens(42)).toBe("42");
    expect(formatTokens(999)).toBe("999");
  });

  it("formats 1000-9999 with 1 decimal", () => {
    expect(formatTokens(1000)).toBe("1.0k");
    expect(formatTokens(2500)).toBe("2.5k");
    expect(formatTokens(9999)).toBe("10.0k");
  });

  it("formats >= 10000 with 0 decimals", () => {
    expect(formatTokens(10000)).toBe("10k");
    expect(formatTokens(12345)).toBe("12k");
    expect(formatTokens(100000)).toBe("100k");
    expect(formatTokens(999999)).toBe("1000k");
  });
});

// ── formatDuration tests ───────────────────────────────────────────

describe("formatDuration", () => {
  it("formats < 1000ms as ms", () => {
    expect(formatDuration(0)).toBe("0ms");
    expect(formatDuration(500)).toBe("500ms");
    expect(formatDuration(999)).toBe("999ms");
  });

  it("formats >= 1000ms as seconds with 1 decimal", () => {
    expect(formatDuration(1000)).toBe("1.0s");
    expect(formatDuration(1500)).toBe("1.5s");
    expect(formatDuration(12345)).toBe("12.3s");
    expect(formatDuration(60000)).toBe("60.0s");
  });
});

// ── summarizeArgs tests ────────────────────────────────────────────

describe("summarizeArgs", () => {
  it("returns path when present", () => {
    expect(summarizeArgs({ path: "src/main.ts" })).toBe("src/main.ts");
  });

  it("returns pattern when present", () => {
    expect(summarizeArgs({ pattern: "function.*" })).toBe("/function.*/");
  });

  it("returns truncated content when present", () => {
    const long = "a".repeat(100);
    const result = summarizeArgs({ content: long });
    expect(result).toBe(`${"a".repeat(60)}\u2026`);
  });

  it("returns JSON fallback for other args", () => {
    const result = summarizeArgs({ foo: "bar", baz: 42 });
    expect(result.length).toBeLessThanOrEqual(80);
    expect(result).toContain("bar");
  });
});

// ── truncate tests (simulates the inline logic in SubagentRow) ─────

describe("truncate", () => {
  it("returns full text when within maxChars", () => {
    expect(truncate("hello", 80)).toBe("hello");
    expect(truncate("", 80)).toBe("");
  });

  it("truncates and appends ellipsis when over maxChars", () => {
    const long = "a".repeat(100);
    expect(truncate(long, 80)).toBe("a".repeat(80) + "\u2026");
  });

  it("uses 80 chars for goal truncation", () => {
    const goal = "Find the authentication flow and refactor it to use the new session management system with proper error handling";
    expect(goal.length).toBeGreaterThan(80);
    const truncated = truncate(goal, 80);
    expect(truncated).toHaveLength(81); // 80 + ellipsis
    expect(truncated.endsWith("\u2026")).toBe(true);
  });

  it("uses 120 chars for report truncation", () => {
    const report = "The authentication flow was found in src/auth/flow.ts. It uses session management from src/session/store.ts. Refactored to use new patterns.";
    expect(report.length).toBeGreaterThan(120);
    const truncated = truncate(report, 120);
    expect(truncated).toHaveLength(121); // 120 + ellipsis
    expect(truncated.endsWith("\u2026")).toBe(true);
  });

  it("returns full text when exactly at maxChars", () => {
    const text = "a".repeat(80);
    expect(truncate(text, 80)).toBe(text);
    expect(truncate(text, 80)).toHaveLength(80);
  });
});

// ── SubagentTimelineState structural tests ─────────────────────────

describe("SubagentTimelineState", () => {
  it("accepts report field", () => {
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
      steps: { type: string }[];
    }

    const withReport: SubagentTimelineState = {
      id: "test-1",
      name: "explorer",
      goal: "find the main function",
      mode: "explore",
      status: "completed",
      rounds: 3,
      inputTokens: 1000,
      outputTokens: 500,
      report: "Found main in src/main.ts",
      steps: [{ type: "text" }],
    };
    expect(withReport.report).toBe("Found main in src/main.ts");

    const withoutReport: SubagentTimelineState = {
      id: "test-2",
      name: "coder",
      goal: "fix the bug",
      mode: "code",
      status: "running",
      rounds: 0,
      inputTokens: 0,
      outputTokens: 0,
      steps: [],
    };
    expect(withoutReport.report).toBeUndefined();
  });

  it("accepts empty goal", () => {
    interface SubagentTimelineState {
      id: string;
      name: string;
      goal: string;
      mode: string;
      status: string;
      rounds: number;
      inputTokens: number;
      outputTokens: number;
      report?: string;
      steps: { type: string }[];
    }

    const sa: SubagentTimelineState = {
      id: "test-3",
      name: "agent",
      goal: "",
      mode: "explore",
      status: "completed",
      rounds: 1,
      inputTokens: 100,
      outputTokens: 50,
      steps: [],
    };
    expect(sa.goal).toBe("");
    // When goal is empty, the UI should NOT show it (Show when={sa.goal} in SolidJS
    // evaluates falsy for empty string)
    expect(!!sa.goal).toBe(false);
  });
});

// ── subagent timeline transition tests (the real regression) ───────
//
// Regression: when a subagent finished (SubagentDone), the authoritative
// subagent-state map was updated but the inline timeline snapshot was NOT,
// so the main timeline row stayed stuck on "running" forever even though the
// subagent had reported and the parent turn had already continued.

function runningSubagent(id: string): SubagentNode {
  return {
    id,
    status: "running",
    rounds: 0,
    inputTokens: 0,
    outputTokens: 0,
    steps: [
      { type: "thinking", thinking: { text: "waiting…", startedAt: 100 } },
      { type: "text", text: "did a thing" } as TimelineNode,
    ],
  };
}

describe("mapSubagentDoneStatus", () => {
  it("maps known terminal statuses", () => {
    expect(mapSubagentDoneStatus("completed")).toBe("completed");
    expect(mapSubagentDoneStatus("failed")).toBe("failed");
    expect(mapSubagentDoneStatus("interrupted")).toBe("interrupted");
    expect(mapSubagentDoneStatus("max_rounds")).toBe("max_rounds");
  });

  it("treats unknown/empty status as completed so it never stays running", () => {
    expect(mapSubagentDoneStatus("")).toBe("completed");
    expect(mapSubagentDoneStatus("weird")).toBe("completed");
    // The key invariant: a done subagent is never left as "running".
    expect(mapSubagentDoneStatus("running")).toBe("completed");
  });
});

describe("markThinkingEnded", () => {
  it("closes open thinking spinners without touching finished ones", () => {
    const steps: TimelineNode[] = [
      { type: "thinking", thinking: { text: "a", startedAt: 1 } },
      { type: "thinking", thinking: { text: "b", startedAt: 2, endedAt: 5 } },
      { type: "text" },
    ];
    const out = markThinkingEnded(steps, 999);
    expect(out[0].thinking!.endedAt).toBe(999);
    expect(out[1].thinking!.endedAt).toBe(5); // untouched
    expect(out[2]).toEqual({ type: "text" });
  });
});

describe("applySubagentDone", () => {
  it("moves the subagent to its terminal status with final stats", () => {
    const subagents = { "sid:0": runningSubagent("sid:0") };
    const next = applySubagentDone(
      subagents,
      { subagentId: "sid:0", status: "completed", rounds: 4, inputTokens: 1200, outputTokens: 340, report: "done" },
      777,
    );
    const sa = next["sid:0"];
    expect(sa.status).toBe("completed");
    expect(sa.rounds).toBe(4);
    expect(sa.inputTokens).toBe(1200);
    expect(sa.outputTokens).toBe(340);
    expect(sa.report).toBe("done");
    // Open thinking spinner is closed out.
    expect(sa.steps[0].thinking!.endedAt).toBe(777);
  });

  it("does not mutate the input map", () => {
    const subagents = { "sid:0": runningSubagent("sid:0") };
    applySubagentDone(subagents, { subagentId: "sid:0", status: "completed", rounds: 1, inputTokens: 0, outputTokens: 0 }, 1);
    expect(subagents["sid:0"].status).toBe("running");
  });

  it("is a no-op for an unknown subagent id", () => {
    const subagents = { "sid:0": runningSubagent("sid:0") };
    const next = applySubagentDone(subagents, { subagentId: "sid:missing", status: "completed", rounds: 1, inputTokens: 0, outputTokens: 0 }, 1);
    expect(next).toBe(subagents);
  });
});

describe("syncSubagentTimelineItems (the fix)", () => {
  it("updates the inline timeline snapshot so it stops showing running", () => {
    const running = runningSubagent("sid:0");
    // The main timeline holds a snapshot of the subagent while it runs.
    const timeline: TimelineNode[] = [
      { type: "text", text: "before" } as TimelineNode,
      { type: "subagent", subagent: running },
      { type: "text", text: "after" } as TimelineNode,
    ];

    // Subagent finishes: authoritative map is updated…
    const doneMap = applySubagentDone(
      { "sid:0": running },
      { subagentId: "sid:0", status: "completed", rounds: 3, inputTokens: 500, outputTokens: 200, report: "ok" },
      50,
    );
    // …and the timeline snapshot is synced from it.
    const synced = syncSubagentTimelineItems(timeline, doneMap);

    const item = synced.find((s) => s.type === "subagent")!;
    expect(item.subagent!.status).toBe("completed"); // was "running" — the bug
    expect(item.subagent!.rounds).toBe(3);
    expect(item.subagent!.report).toBe("ok");
    // Surrounding items are preserved and order is stable.
    expect(synced[0].text).toBe("before");
    expect(synced[2].text).toBe("after");
  });

  it("leaves non-subagent items and unknown subagents untouched", () => {
    const timeline: TimelineNode[] = [
      { type: "subagent", subagent: runningSubagent("other") },
      { type: "thinking", thinking: { text: "x", startedAt: 1 } },
    ];
    const synced = syncSubagentTimelineItems(timeline, {});
    expect(synced[0].subagent!.status).toBe("running"); // no state for it → unchanged
    expect(synced[1]).toEqual(timeline[1]);
  });
});

// ── SubagentDoneData structural tests ──────────────────────────────

describe("SubagentDoneData", () => {
  it("accepts report as optional field", () => {
    // Simulates the TypeScript interface from ipc.ts
    interface SubagentDoneData {
      subagentId: string;
      status: string;
      rounds: number;
      inputTokens: number;
      outputTokens: number;
      report?: string;
    }

    const withReport: SubagentDoneData = {
      subagentId: "sid:0",
      status: "completed",
      rounds: 3,
      inputTokens: 1000,
      outputTokens: 500,
      report: "Found the main function at src/main.ts:42",
    };
    expect(withReport.report).toBeTruthy();

    const withoutReport: SubagentDoneData = {
      subagentId: "sid:1",
      status: "completed",
      rounds: 2,
      inputTokens: 500,
      outputTokens: 200,
    };
    expect(withoutReport.report).toBeUndefined();
  });
});
