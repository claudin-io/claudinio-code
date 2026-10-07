import { describe, it, expect } from "vitest";
import {
  askUserAnswerMessages,
  closeOpenThinking,
  closingText,
  parseAskUserAnswers,
  recordsToMessages,
  segmentMessage,
  startsNewRound,
  type ChatMessage,
  type TimelineItem,
} from "./chatRecords";
import type { AgentEvent, SessionRecord } from "./ipc";

// ---------------------------------------------------------------------------
// Record builders — the shapes persist.rs writes.
// ---------------------------------------------------------------------------

const user = (text: string): SessionRecord[] => [
  { kind: "user", text, ts: 1 },
  { kind: "turn", role: "user", content: [{ type: "text", text }], ts: 1 },
];

/** An assistant turn that calls a tool, and the turn carrying its result. */
const toolRound = (id: string, name: string, result: string, text?: string): SessionRecord[] => [
  {
    kind: "turn",
    role: "assistant",
    content: [
      ...(text ? [{ type: "text", text }] : []),
      { type: "tool_use", id, name, input: {} },
    ],
    ts: 2,
  },
  {
    kind: "turn",
    role: "user",
    content: [{ type: "tool_result", tool_use_id: id, content: result }],
    ts: 2,
  },
];

/** An assistant turn with no tool call: an answer. */
const answer = (text: string): SessionRecord => ({
  kind: "turn",
  role: "assistant",
  content: [{ type: "text", text }],
  ts: 3,
});

/** The harness sending the model back to work after an answer. */
const nudge = (): SessionRecord[] => [
  { kind: "continuation_judge", verdict: "continue", nudged: true, streak: 1, ts: 4 },
  {
    kind: "turn",
    role: "user",
    content: [{ type: "text", text: "[system] Your previous message announced a next step…" }],
    ts: 4,
  },
];

const steering = (text: string): SessionRecord => ({ kind: "steering", text, attachments: [], ts: 5 });

const done = (): SessionRecord => ({ kind: "done", input_tokens: 100, output_tokens: 10, ts: 6 });

/** What a reader sees going down the thread: who spoke, and the visible text. */
const thread = (messages: ChatMessage[]) => messages.map((m) => `${m.role}: ${m.text}`);

const toolNames = (m: ChatMessage) =>
  (m.steps ?? []).filter((s) => s.type === "tool").map((s) => s.tool!.call.toolName);

// ---------------------------------------------------------------------------

describe("recordsToMessages — one message per answer", () => {
  // The session that prompted this. One question typed, one `ask_user`
  // answered, one message steered in mid-run; three answers. On screen: the
  // first question, one collapsed "Worked for…" line, and only the last answer
  // — under the wrong question.
  const session: SessionRecord[] = [
    ...user("Explain the 0.4.0 changes"),
    ...toolRound("t1", "bash", "git log…"),
    answer("# 0.4.0 — what changed"),
    ...nudge(),
    ...toolRound(
      "t2",
      "ask_user",
      "Pergunta: Do you want anything else?\nResposta: Bring 0.4.0 to main",
    ),
    ...toolRound("t3", "bash", "Fast-forward"),
    answer("Done. 0.4.0 is on main."),
    ...nudge(),
    ...toolRound("t4", "tasks_set", "Tasks updated"),
    steering("Is our main clean?"),
    ...toolRound("t5", "bash", "## main...origin/main"),
    answer("Yes, it is clean."),
    done(),
  ];

  it("keeps every answer, each under the question it answers", () => {
    expect(thread(recordsToMessages(session))).toEqual([
      "user: Explain the 0.4.0 changes",
      "assistant: # 0.4.0 — what changed",
      "assistant: ", // the ask_user call, collapsed
      "user: Bring 0.4.0 to main",
      "assistant: Done. 0.4.0 is on main.",
      "assistant: ", // tool-only work after the nudge
      "user: Is our main clean?",
      "assistant: Yes, it is clean.",
    ]);
  });

  it("puts each stretch of tool work in the message it led to", () => {
    const m = recordsToMessages(session);
    expect(toolNames(m[1])).toEqual(["bash"]);
    expect(toolNames(m[2])).toEqual(["ask_user"]);
    expect(toolNames(m[4])).toEqual(["bash"]);
    expect(toolNames(m[5])).toEqual(["tasks_set"]);
    expect(toolNames(m[7])).toEqual(["bash"]);
  });

  it("pairs a tool result with its call across the split", () => {
    const m = recordsToMessages(session);
    expect(m[7].steps![0].tool!.result!.output).toBe("## main...origin/main");
  });

  it("marks a steered message as the user's, and an ask_user reply with its question", () => {
    const m = recordsToMessages(session);
    expect(m[6]).toMatchObject({ role: "user", steering: true });
    expect(m[3]).toEqual({
      role: "user",
      text: "Bring 0.4.0 to main",
      inReplyTo: "Do you want anything else?",
    });
    expect(m[0].steering).toBeUndefined();
  });

  it("puts the run's totals on the last message only", () => {
    const m = recordsToMessages(session);
    expect(m.filter((x) => x.done)).toHaveLength(1);
    expect(m[m.length - 1].done).toMatchObject({ inputTokens: 100, outputTokens: 10 });
  });

  it("leaves steering out of the collapsed steps", () => {
    const steps = recordsToMessages(session).flatMap((m) => m.steps ?? []);
    expect(steps.some((s) => s.type === "steering")).toBe(false);
  });
});

describe("recordsToMessages — what does not split", () => {
  it("a plain question and answer is still one message", () => {
    const m = recordsToMessages([
      ...user("hi"),
      ...toolRound("t1", "bash", "ok", "Let me look."),
      ...toolRound("t2", "read_file", "contents"),
      answer("Here it is."),
      done(),
    ]);
    expect(thread(m)).toEqual(["user: hi", "assistant: Here it is."]);
    expect(toolNames(m[1])).toEqual(["bash", "read_file"]);
  });

  it("text written alongside a tool call is a step, not an answer", () => {
    const m = recordsToMessages([
      ...user("hi"),
      ...toolRound("t1", "bash", "ok", "Checking the status first."),
      ...toolRound("t2", "bash", "ok", "Now the log."),
      answer("All good."),
    ]);
    expect(m).toHaveLength(2);
    expect(m[1].steps!.filter((s) => s.type === "text")).toHaveLength(2);
  });

  it("what follows an answer without a new round stays with that answer", () => {
    // The Stop hook and the run's totals come after the final answer.
    const m = recordsToMessages([
      ...user("hi"),
      answer("Done."),
      { kind: "hook", event: "Stop", command: "notify.sh", status: "ok", ts: 7 },
      done(),
    ]);
    expect(thread(m)).toEqual(["user: hi", "assistant: Done."]);
    expect(m[1].steps!.map((s) => s.type)).toEqual(["hook"]);
    expect(m[1].done).toBeDefined();
  });

  it("a tool hook after an answer opens the next message, with its tool", () => {
    // PreToolUse is written before the assistant turn it guards.
    const m = recordsToMessages([
      ...user("hi"),
      answer("First."),
      ...nudge(),
      { kind: "hook", event: "PreToolUse", command: "guard.sh", status: "ok", ts: 7 },
      ...toolRound("t1", "bash", "ok"),
      answer("Second."),
    ]);
    expect(thread(m)).toEqual(["user: hi", "assistant: First.", "assistant: Second."]);
    expect(m[1].steps).toEqual([]);
    expect(m[2].steps!.map((s) => s.type)).toEqual(["hook", "tool"]);
  });

  it("a tool hook keeps the id of the call it fired around", () => {
    // The hooks of a round are written before the turn that holds its tool
    // calls, so the id is the only thing that says which hook was whose.
    const m = recordsToMessages([
      ...user("hi"),
      { kind: "hook", event: "PreToolUse", command: "guard.sh", status: "ok", tool_id: "t1", ts: 7 },
      { kind: "hook", event: "PostToolUse", command: "fmt.sh", status: "ok", tool_id: "t1", ts: 8 },
      { kind: "hook", event: "Stop", command: "notify.sh", status: "ok", ts: 9 },
      ...toolRound("t1", "bash", "ok"),
      answer("Done."),
    ]);
    const hooks = m[1].steps!.filter((s) => s.type === "hook").map((s) => s.hook!);
    expect(hooks.map((h) => h.toolId)).toEqual(["t1", "t1", undefined]);
    expect("toolId" in hooks[2]).toBe(false);
  });

  it("an ask_user that got no answer stays a tool row", () => {
    const m = recordsToMessages([
      ...user("hi"),
      ...toolRound("t1", "ask_user", "[ask_user] no answer received"),
      answer("Carrying on."),
    ]);
    expect(thread(m)).toEqual(["user: hi", "assistant: Carrying on."]);
  });

  it("an ask_user answered next to another tool keeps both results", () => {
    const m = recordsToMessages([
      ...user("hi"),
      {
        kind: "turn",
        role: "assistant",
        content: [
          { type: "tool_use", id: "a", name: "ask_user", input: {} },
          { type: "tool_use", id: "b", name: "bash", input: {} },
        ],
        ts: 2,
      },
      {
        kind: "turn",
        role: "user",
        content: [
          { type: "tool_result", tool_use_id: "a", content: "Pergunta: Which?\nResposta: The first" },
          { type: "tool_result", tool_use_id: "b", content: "ran" },
        ],
        ts: 2,
      },
      answer("Done."),
    ]);
    expect(thread(m)).toEqual(["user: hi", "assistant: ", "user: The first", "assistant: Done."]);
    expect(m[1].steps!.map((s) => s.tool!.result!.output)).toEqual([
      "Pergunta: Which?\nResposta: The first",
      "ran",
    ]);
  });
});

describe("parseAskUserAnswers", () => {
  it("reads one pair", () => {
    expect(parseAskUserAnswers("Pergunta: Which DB?\nResposta: Postgres")).toEqual([
      { question: "Which DB?", answer: "Postgres" },
    ]);
  });

  it("reads several pairs, and keeps multi-line answers whole", () => {
    const out = "Pergunta: A?\nResposta: one\ntwo\n\nPergunta: B?\nResposta: three";
    expect(parseAskUserAnswers(out)).toEqual([
      { question: "A?", answer: "one\ntwo" },
      { question: "B?", answer: "three" },
    ]);
  });

  it("yields nothing for output in any other shape", () => {
    expect(parseAskUserAnswers("")).toEqual([]);
    expect(parseAskUserAnswers("cancelled by the user")).toEqual([]);
    expect(parseAskUserAnswers("Pergunta: half a pair")).toEqual([]);
  });

  it("makes no bubble out of an empty answer", () => {
    expect(askUserAnswerMessages("Pergunta: A?\nResposta: ")).toEqual([]);
  });
});

describe("segmentMessage", () => {
  it("is nothing when there is nothing to show", () => {
    expect(segmentMessage([], "")).toBeNull();
  });

  it("keeps a stretch that only did work", () => {
    const steps: TimelineItem[] = [{ type: "text", text: "note" }];
    expect(segmentMessage(steps, "")).toEqual({ role: "assistant", text: "", steps });
  });

  it("carries the run's totals when given them", () => {
    const d = { stopReason: "end_turn", textOutput: "x", inputTokens: 1, outputTokens: 2 };
    expect(segmentMessage([], "x", d)).toEqual({ role: "assistant", text: "x", steps: [], done: d });
  });
});

describe("closeOpenThinking", () => {
  const thinking = (startedAt: number, endedAt?: number): TimelineItem => ({
    type: "thinking",
    thinking: { text: "…", startedAt, ...(endedAt === undefined ? {} : { endedAt }) },
  });
  const worked = (steps: TimelineItem[]) =>
    steps.reduce((ms, s) => ms + (s.thinking ? s.thinking.endedAt! - s.thinking.startedAt : 0), 0);

  it("ends the open thought and leaves finished ones alone", () => {
    const out = closeOpenThinking([thinking(0, 10), thinking(20)], 50);
    expect(out[0].thinking!.endedAt).toBe(10);
    expect(out[1].thinking!.endedAt).toBe(50);
  });

  it("returns the same array when nothing is open", () => {
    const steps = [thinking(0, 10), { type: "text", text: "x" } as TimelineItem];
    expect(closeOpenThinking(steps, 99)).toBe(steps);
  });

  it("closing each thought as the next thing happens adds up to the time spent thinking", () => {
    // Three 10s thoughts in a run that then sat for ten minutes. Closed only
    // at the end, each lasts until the end: 630 + 610 + 590 seconds of
    // "work" for 30 seconds of thinking.
    let steps: TimelineItem[] = [];
    for (const start of [0, 20_000, 40_000]) {
      steps = [...closeOpenThinking(steps, start), thinking(start)];
      steps = closeOpenThinking(steps, start + 10_000); // the tool call that followed
    }
    steps = closeOpenThinking(steps, 630_000); // Done
    expect(worked(steps)).toBe(30_000);
  });
});

describe("the live run — when an answer closes its message", () => {
  const ev = (event: string, data: unknown = {}) => ({ event, data }) as AgentEvent;

  it("model activity after an answer means the run went on", () => {
    expect(startsNewRound(ev("TextDelta", { text: "Now" }))).toBe(true);
    expect(startsNewRound(ev("Thinking", "hmm"))).toBe(true);
    expect(startsNewRound(ev("ToolCall"))).toBe(true);
    expect(startsNewRound(ev("SubagentStarted"))).toBe(true);
  });

  it("a PreToolUse hook is the next round starting: it fires before its call is announced", () => {
    // Left with the answer it would sit under it, a message away from the tool
    // call whose line it belongs on.
    expect(startsNewRound(ev("HookStarted", { event: "PreToolUse" }))).toBe(true);
  });

  it("what the harness does after an answer stays with that answer", () => {
    // The gate's "Verifying the goal…" note, its verdict and the Stop hook
    // all come between the final answer and Done.
    expect(startsNewRound(ev("TextStep", { text: "Verifying the goal…" }))).toBe(false);
    expect(startsNewRound(ev("QualityVerdict"))).toBe(false);
    expect(startsNewRound(ev("HookStarted", { event: "Stop" }))).toBe(false);
    expect(startsNewRound(ev("SessionStats"))).toBe(false);
    expect(startsNewRound(ev("Thinking", ""))).toBe(false);
    expect(startsNewRound(ev("Done"))).toBe(false);
  });

  it("Done ends on the answer the last stretch reached", () => {
    expect(closingText("Yes, it is clean.", "Yes, it is clean.", "Done. 0.4.0 is on main.")).toBe(
      "Yes, it is clean.",
    );
    expect(closingText("Done.", null, "")).toBe("Done.");
  });

  it("Done does not print again an answer that already has its message", () => {
    // Answer, nudge, tool calls only, end: the backend's last answer is still
    // the one already on screen.
    expect(closingText("Done. 0.4.0 is on main.", null, "Done. 0.4.0 is on main.")).toBe("");
  });

  it("an answer the model repeated word for word is still its new answer", () => {
    expect(closingText("Same.", "Same.", "Same.")).toBe("Same.");
  });
});
