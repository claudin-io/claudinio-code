import { describe, it, expect, vi, afterEach } from "vitest";
import { createSignal } from "solid-js";
import { render } from "solid-js/web";

// TimelineRows pulls in DiffViewer, which pulls in monaco — it needs browser
// APIs jsdom does not implement. Same stub the other viewer tests use.
vi.mock("monaco-editor", () => ({
  editor: { create: vi.fn(), createDiffEditor: vi.fn(), defineTheme: vi.fn() },
}));

import { HookRow, QualityRow, TimelineSteps } from "./TimelineRows";
import type { TimelineItem } from "../../lib/chatRecords";
import type { QualityVerdictData } from "../../lib/ipc";

let dispose: (() => void) | undefined;
let host: HTMLDivElement | undefined;

afterEach(() => {
  dispose?.();
  host?.remove();
  dispose = undefined;
  host = undefined;
});

function mount(quality: QualityVerdictData): HTMLDivElement {
  host = document.createElement("div");
  document.body.appendChild(host);
  dispose = render(() => <QualityRow quality={quality} />, host);
  return host;
}

const verdict = (over: Partial<QualityVerdictData> = {}): QualityVerdictData => ({
  pass: true,
  summary: "all good",
  trigger: "tool",
  layers: [
    { layer: "tests", stack: "rust", status: "pass", summary: "12 passed, 0 failed" },
  ],
  ...over,
});

describe("QualityRow", () => {
  it("shows a passing run as passed", () => {
    const el = mount(verdict());
    expect(el.textContent).toContain("Checks passed");
    expect(el.textContent).toContain("12 passed, 0 failed");
  });

  it("shows a failing run as failed", () => {
    const el = mount(
      verdict({
        pass: false,
        layers: [
          { layer: "tests", stack: "js", status: "fail", summary: "9 passed, 3 failed" },
        ],
      }),
    );
    expect(el.textContent).toContain("Checks failed");
    expect(el.textContent).toContain("9 passed, 3 failed");
  });

  it("marks a harness-enforced run so the user knows who verified it", () => {
    const el = mount(verdict({ trigger: "harness" }));
    expect(el.textContent).toContain("verified by the harness");
  });

  it("does not credit the harness when the agent asked for the run", () => {
    const el = mount(verdict({ trigger: "tool" }));
    expect(el.textContent).not.toContain("verified by the harness");
  });

  it("distinguishes a layer that could not run from one that passed", () => {
    // "we did not measure this" must never read like a green check — that is
    // the whole difference between evidence and the appearance of evidence.
    const el = mount(
      verdict({
        layers: [
          { layer: "tests", stack: "rust", status: "pass", summary: "3 passed" },
          {
            layer: "coverage",
            stack: "rust",
            status: "unavailable",
            summary: "coverage tooling is not installed",
          },
        ],
      }),
    );
    expect(el.textContent).toContain("coverage tooling is not installed");
    const marks = Array.from(el.querySelectorAll("span")).map((s) => s.textContent?.trim());
    expect(marks).toContain("✓");
    expect(marks).toContain("–");
  });

  it("lists every layer and stack that ran", () => {
    const el = mount(
      verdict({
        layers: [
          { layer: "tests", stack: "rust", status: "pass", summary: "a" },
          { layer: "tests", stack: "js", status: "pass", summary: "b" },
          { layer: "coverage", stack: "js", status: "fail", summary: "c" },
        ],
      }),
    );
    expect(el.textContent).toContain("tests (rust)");
    expect(el.textContent).toContain("tests (js)");
    expect(el.textContent).toContain("coverage (js)");
  });
});

// ─────────────────────────────────────────────────────────────
// HookRow
// ─────────────────────────────────────────────────────────────

function mountHook(hook: NonNullable<TimelineItem["hook"]>): HTMLDivElement {
  host = document.createElement("div");
  document.body.appendChild(host);
  dispose = render(() => <HookRow hook={hook} />, host);
  return host;
}

const hookItem = (
  over: Partial<NonNullable<TimelineItem["hook"]>> = {},
): NonNullable<TimelineItem["hook"]> => ({
  hookId: "s:PreToolUse:1",
  event: "PreToolUse",
  command: "/ws/guard.sh",
  source: "plugin: claudinio-brain",
  status: "ok",
  exitCode: 0,
  durationMs: 42,
  ...over,
});

describe("HookRow", () => {
  it("shows a running hook by its statusMessage", () => {
    // "Reading this project's brain" is the label the config author wrote; a
    // finished-only row would throw it away at the moment it is useful.
    const el = mountHook(
      hookItem({ status: "running", statusMessage: "Reading this project's brain" }),
    );
    expect(el.textContent).toContain("Reading this project's brain");
  });

  it("shows a completed hook with its exit code and duration", () => {
    const el = mountHook(hookItem());
    expect(el.textContent).toContain("PreToolUse hook");
    expect(el.textContent).toContain("exit 0");
    expect(el.textContent).toContain("42ms");
  });

  it("a failing hook reads as a failure, not as silence", () => {
    const el = mountHook(hookItem({ status: "error", exitCode: 127, error: "brain: not found" }));
    expect(el.textContent).toContain("failed");
    expect(el.textContent).toContain("brain: not found");
  });

  it("a timeout says so", () => {
    const el = mountHook(hookItem({ status: "timeout", exitCode: null }));
    expect(el.textContent).toContain("timed out");
  });

  it("a hook that was not run for lack of approval says that", () => {
    // Distinct from "it ran and had nothing to say", which is the whole point.
    const el = mountHook(hookItem({ status: "skipped_untrusted", exitCode: null }));
    expect(el.textContent).toContain("waiting for your approval");
  });

  it("a blocking hook says it blocked something", () => {
    const el = mountHook(hookItem({ status: "blocked", decision: "deny" }));
    expect(el.textContent).toContain("blocked this");
  });

  it("attributes injected context instead of leaving it in the user's message", () => {
    const el = mountHook(
      hookItem({
        event: "UserPromptSubmit",
        command: "",
        context: "the port is 8080",
        exitCode: null,
        durationMs: undefined,
      }),
    );
    expect(el.textContent).toContain("UserPromptSubmit hook added context");
    expect(el.textContent).toContain("the port is 8080");
  });
});

// ─────────────────────────────────────────────────────────────
// Quiet hooks — on their tool call's line, or one line per run
// ─────────────────────────────────────────────────────────────

function mountSteps(
  steps: () => TimelineItem[],
  onToggle: (index: number) => void = () => {},
): HTMLDivElement {
  host = document.createElement("div");
  document.body.appendChild(host);
  dispose = render(
    () => <TimelineSteps steps={steps()} expandedStep={null} onToggle={onToggle} isLive={false} />,
    host,
  );
  return host;
}

const hookStep = (over: Partial<NonNullable<TimelineItem["hook"]>> = {}): TimelineItem => ({
  type: "hook",
  hook: hookItem(over),
});

const pre = (toolId: string, over: Partial<NonNullable<TimelineItem["hook"]>> = {}): TimelineItem =>
  hookStep({ toolId, ...over });

const post = (toolId: string, over: Partial<NonNullable<TimelineItem["hook"]>> = {}): TimelineItem =>
  hookStep({ hookId: "s:PostToolUse:1", event: "PostToolUse", durationMs: 7, toolId, ...over });

const toolStep = (toolId: string, command = "git status"): TimelineItem => ({
  type: "tool",
  tool: {
    call: { sessionId: "s", toolId, toolName: "bash", args: { command }, permission: "allow" },
    status: "ok",
  },
});

/// The hook icons: one per tool call that has hooks, one per line of hooks.
const hookButtons = (el: HTMLElement): HTMLButtonElement[] =>
  Array.from(el.querySelectorAll<HTMLButtonElement>('button[aria-label*=" hook"]'));

/// The line a button sits on, as the text a user reads across it.
const lineOf = (button: HTMLElement): string => button.closest(".h-7")?.textContent ?? "";

describe("quiet hooks in the timeline", () => {
  it("a tool call's hooks sit on its own line, behind one icon", () => {
    // The point of the change: the hooks used to take a line of their own
    // under every tool call, which doubled the height of the thread.
    const el = mountSteps(() => [pre("t1"), toolStep("t1"), post("t1")]);
    const found = hookButtons(el);
    expect(found).toHaveLength(1);
    expect(lineOf(found[0])).toContain("git status");
    expect(el.querySelectorAll(".h-7")).toHaveLength(1);
    expect(el.textContent).not.toContain("PreToolUse hook");
    expect(el.textContent).not.toContain("exit 0");
  });

  it("the icon comes before the tool call's title", () => {
    const el = mountSteps(() => [pre("t1"), toolStep("t1")]);
    const line = hookButtons(el)[0].closest(".h-7")!;
    const title = Array.from(line.querySelectorAll("button")).find((b) => b.textContent?.includes("git status"))!;
    expect(
      hookButtons(el)[0].compareDocumentPosition(title) & Node.DOCUMENT_POSITION_FOLLOWING,
    ).toBeTruthy();
  });

  it("the icon still names what ran, for whoever cannot hover", () => {
    const one = mountSteps(() => [pre("t1"), toolStep("t1")]);
    expect(hookButtons(one)[0].getAttribute("aria-label")).toBe("PreToolUse hook · ok · exit 0 · 42ms");
    dispose?.();
    host?.remove();

    const two = mountSteps(() => [pre("t1"), toolStep("t1"), post("t1")]);
    expect(hookButtons(two)[0].getAttribute("aria-label")).toBe("2 hooks · ok · 49ms");
  });

  it("hooks around different tool calls stay with their own call", () => {
    // Live order: one call's PostToolUse is followed at once by the next
    // call's PreToolUse. They are neighbours in the list and not in meaning.
    const el = mountSteps(() => [
      pre("t1"),
      toolStep("t1", "git status"),
      post("t1"),
      pre("t2"),
      toolStep("t2", "git log"),
      post("t2"),
    ]);
    const found = hookButtons(el);
    expect(found).toHaveLength(2);
    expect(lineOf(found[0])).toContain("git status");
    expect(lineOf(found[1])).toContain("git log");
  });

  it("finds its call by id, wherever a reopened session put the hook", () => {
    // On disk the hooks of a round are written before the assistant turn that
    // holds its tool calls, so position says nothing about which is whose.
    const el = mountSteps(() => [
      pre("t1"),
      post("t1"),
      post("t2"),
      toolStep("t1", "git status"),
      toolStep("t2", "git log"),
    ]);
    const found = hookButtons(el);
    expect(found.map((b) => b.getAttribute("aria-label"))).toEqual([
      "2 hooks · ok · 49ms",
      "PostToolUse hook · ok · exit 0 · 7ms",
    ]);
    expect(lineOf(found[0])).toContain("git status");
    expect(lineOf(found[1])).toContain("git log");
  });

  it("a tool call without hooks has no icon", () => {
    const el = mountSteps(() => [pre("t1"), toolStep("t1"), toolStep("t2", "git log")]);
    expect(hookButtons(el)).toHaveLength(1);
    expect(lineOf(hookButtons(el)[0])).toContain("git status");
  });

  it("hooks that belong to no tool call share one line, icon and text", () => {
    const el = mountSteps(() => [
      hookStep({ event: "SessionStart" }),
      hookStep({ event: "UserPromptSubmit", durationMs: 8 }),
    ]);
    const found = hookButtons(el);
    expect(found).toHaveLength(1);
    expect(lineOf(found[0])).toContain("2 hooks");
    expect(lineOf(found[0])).toContain("ok · 50ms");
  });

  it("a lone hook's line says which hook it was", () => {
    const el = mountSteps(() => [hookStep({ event: "Stop" })]);
    expect(lineOf(hookButtons(el)[0])).toContain("Stop hook");
    expect(lineOf(hookButtons(el)[0])).toContain("ok · exit 0 · 42ms");
  });

  it("a session recorded before hooks named their call keeps them on one line", () => {
    const el = mountSteps(() => [hookStep(), hookStep({ event: "PostToolUse" }), toolStep("t1")]);
    const found = hookButtons(el);
    expect(found).toHaveLength(1);
    expect(lineOf(found[0])).toContain("2 hooks");
    expect(lineOf(found[0])).not.toContain("git status");
  });

  it("a failure among a call's hooks keeps its full row", () => {
    // Shrinking this one would make a broken hook look like a working one.
    const el = mountSteps(() => [
      pre("t1"),
      pre("t1", { status: "error", exitCode: 127, error: "brain: not found" }),
      toolStep("t1"),
    ]);
    expect(el.textContent).toContain("PreToolUse hook failed");
    expect(el.textContent).toContain("brain: not found");
    expect(hookButtons(el)).toHaveLength(1);
    expect(hookButtons(el)[0].getAttribute("aria-label")).toBe("PreToolUse hook · ok · exit 0 · 42ms");
  });

  it.each([
    ["blocked", "blocked this"],
    ["timeout", "timed out"],
    ["skipped_untrusted", "waiting for your approval"],
  ])("a %s hook keeps its full row", (status, text) => {
    const el = mountSteps(() => [pre("t1", { status, exitCode: null }), toolStep("t1")]);
    expect(el.textContent).toContain(text);
    expect(hookButtons(el)).toHaveLength(0);
  });

  it("a hook that left a message for the user keeps its full row", () => {
    const el = mountSteps(() => [post("t1", { systemMessage: "formatted 3 files" }), toolStep("t1")]);
    expect(el.textContent).toContain("formatted 3 files");
    expect(hookButtons(el)).toHaveLength(0);
  });

  it("context a hook injected keeps its full row", () => {
    const el = mountSteps(() => [
      hookStep({ event: "UserPromptSubmit", command: "", context: "the port is 8080" }),
    ]);
    expect(el.textContent).toContain("UserPromptSubmit hook added context");
    expect(el.textContent).toContain("the port is 8080");
  });

  it("a running hook shows the label its author wrote", () => {
    const alone = mountSteps(() => [
      hookStep({ status: "running", statusMessage: "Reading this project's brain" }),
    ]);
    expect(lineOf(hookButtons(alone)[0])).toContain("Reading this project's brain");
    dispose?.();
    host?.remove();

    // On a tool call's line too: PostToolUse runs while its call is on screen.
    const onCall = mountSteps(() => [
      toolStep("t1"),
      post("t1", { status: "running", statusMessage: "Formatting what changed" }),
    ]);
    expect(lineOf(hookButtons(onCall)[0])).toContain("Formatting what changed");
    expect(hookButtons(onCall)[0].getAttribute("aria-label")).toBe("PostToolUse hook · running");
  });

  it("hovering explains every hook of the call: what fired, how it ended, how long it took", () => {
    const el = mountSteps(() => [
      pre("t1", { decision: "allow" }),
      toolStep("t1"),
      post("t1", { command: "/ws/format.sh", source: "project" }),
    ]);
    const button = hookButtons(el)[0];
    button.dispatchEvent(new MouseEvent("mouseenter"));
    const page = () => document.body.textContent ?? "";
    expect(page()).toContain("2 hooksok · 49ms");
    expect(page()).toContain("PreToolUse hookok · exit 0 · 42ms");
    expect(page()).toContain("Fires before a tool call is dispatched.");
    expect(page()).toContain("Decisionallow");
    expect(page()).toContain("plugin: claudinio-brain");
    expect(page()).toContain("/ws/guard.sh");
    expect(page()).toContain("PostToolUse hookok · exit 0 · 7ms");
    expect(page()).toContain("Fires after a tool returns.");
    expect(page()).toContain("/ws/format.sh");

    button.dispatchEvent(new MouseEvent("mouseleave"));
    expect(page()).not.toContain("Fires before a tool call is dispatched.");
  });

  it("the tooltip grows with its text up to a limit, and a long path wraps inside it", () => {
    // A settings path and a script path ran out of the right edge of a
    // fixed-width box. jsdom lays nothing out, so this pins what makes the
    // text wrap rather than the wrapping itself: a width cap instead of a
    // width, a value column allowed to be narrower than its longest word, and
    // somewhere to break inside a path.
    const source = "/Users/me/.claude/settings.json";
    const command = "/Users/me/.claude/hooks/rtk-rewrite.sh";
    const el = mountSteps(() => [pre("t1", { source, command }), toolStep("t1")]);
    hookButtons(el)[0].dispatchEvent(new MouseEvent("mouseenter"));

    const tip = document.body.querySelector<HTMLElement>(".shadow-modal")!;
    expect(tip.className).toContain("w-max");
    expect(tip.className).toMatch(/max-w-\[min\(\d+px,calc\(100vw-/);
    expect(tip.className).not.toMatch(/(^|\s)w-\d+(\s|$)/);
    expect(tip.querySelector("dl")!.className).toContain("grid-cols-[auto_minmax(0,1fr)]");

    const value = (text: string) =>
      Array.from(tip.querySelectorAll("dd")).find((dd) => dd.textContent === text)!;
    for (const text of [source, command]) {
      // The text itself is untouched: the break points are elements, not characters.
      expect(value(text)).toBeTruthy();
      expect(value(text).className).toContain("break-words");
      expect(value(text).querySelectorAll("wbr")).toHaveLength(text.split("/").length - 1);
    }
  });

  it("hovering a line of hooks explains them the same way", () => {
    const el = mountSteps(() => [hookStep({ event: "Stop" })]);
    hookButtons(el)[0].dispatchEvent(new MouseEvent("mouseenter"));
    expect(document.body.textContent).toContain("Fires when the run is about to end.");
  });

  it("keyboard focus explains it too", () => {
    const el = mountSteps(() => [pre("t1"), toolStep("t1")]);
    const button = hookButtons(el)[0];
    button.dispatchEvent(new FocusEvent("focus"));
    expect(document.body.textContent).toContain("Fires before a tool call is dispatched.");
    button.dispatchEvent(new FocusEvent("blur"));
    expect(document.body.textContent).not.toContain("Fires before a tool call is dispatched.");
  });

  it("clicking opens every hook of the call: the command that ran and what it printed", () => {
    const onToggle = vi.fn();
    const el = mountSteps(
      () => [
        pre("t1", { output: "lint: 0 problems" }),
        toolStep("t1"),
        post("t1", { command: "/ws/format.sh", output: "formatted 2 files" }),
      ],
      onToggle,
    );
    const button = hookButtons(el)[0];
    expect(el.textContent).not.toContain("/ws/guard.sh");

    button.click();
    expect(button.getAttribute("aria-expanded")).toBe("true");
    expect(el.textContent).toContain("/ws/guard.sh");
    expect(el.textContent).toContain("lint: 0 problems");
    expect(el.textContent).toContain("/ws/format.sh");
    expect(el.textContent).toContain("formatted 2 files");

    button.click();
    expect(button.getAttribute("aria-expanded")).toBe("false");
    expect(el.textContent).not.toContain("/ws/guard.sh");

    // The icon opens the hooks. It is not another way to open the tool call.
    expect(onToggle).not.toHaveBeenCalled();
  });

  it("the rest of the line still opens the tool call, and only that", () => {
    const onToggle = vi.fn();
    const el = mountSteps(() => [pre("t1"), toolStep("t1")], onToggle);
    const line = hookButtons(el)[0].closest(".h-7")!;
    Array.from(line.querySelectorAll("button"))
      .find((b) => b.textContent?.includes("git status"))!
      .click();
    expect(onToggle).toHaveBeenCalledTimes(1);
    expect(onToggle).toHaveBeenCalledWith(1);
    expect(hookButtons(el)[0].getAttribute("aria-expanded")).toBe("false");
  });

  it("clicking a line of hooks opens them all", () => {
    const el = mountSteps(() => [
      hookStep({ event: "SessionStart", command: "/ws/first.sh" }),
      hookStep({ event: "UserPromptSubmit", command: "/ws/second.sh" }),
    ]);
    hookButtons(el)[0].click();
    expect(el.textContent).toContain("/ws/first.sh");
    expect(el.textContent).toContain("/ws/second.sh");
  });

  it("says so when a hook printed nothing, rather than showing an empty box", () => {
    const el = mountSteps(() => [pre("t1", { output: "" }), toolStep("t1")]);
    hookButtons(el)[0].click();
    expect(el.textContent).toContain("No output to show.");
  });

  it("shows stderr read back from a reopened session", () => {
    // chatRecords maps the record's stderr onto `error`; an ok hook that only
    // warned must not turn into a failure row, and must not lose the warning.
    const el = mountSteps(() => [pre("t1", { error: "warning: cache is stale" }), toolStep("t1")]);
    expect(el.textContent).not.toContain("warning: cache is stale");
    hookButtons(el)[0].click();
    expect(el.textContent).toContain("warning: cache is stale");
  });

  it("the tooltip stands down while the hooks' panel is open", () => {
    const el = mountSteps(() => [pre("t1"), toolStep("t1")]);
    const button = hookButtons(el)[0];
    button.dispatchEvent(new MouseEvent("mouseenter"));
    button.click();
    expect(document.body.textContent).not.toContain("Click to see the command and its output.");
  });

  it("follows a live run: a hook waits on its own line, then joins its call", () => {
    // PreToolUse fires before the call is announced, so for a moment there is
    // no line for it to join. Its label is the only sign of what the wait is.
    const running = pre("t1", {
      status: "running",
      exitCode: null,
      durationMs: undefined,
      statusMessage: "Checking the command",
    });
    const [steps, setSteps] = createSignal<TimelineItem[]>([running]);
    const el = mountSteps(steps);
    expect(lineOf(hookButtons(el)[0])).toContain("Checking the command");

    // ChatPanel replaces the step object when HookFinished lands, then appends.
    const done = pre("t1");
    const tool = toolStep("t1");
    setSteps([done, tool]);
    expect(hookButtons(el)).toHaveLength(1);
    expect(lineOf(hookButtons(el)[0])).toContain("git status");
    expect(el.textContent).not.toContain("Checking the command");

    setSteps([done, tool, post("t1")]);
    expect(hookButtons(el)).toHaveLength(1);
    expect(hookButtons(el)[0].getAttribute("aria-label")).toBe("2 hooks · ok · 49ms");
  });

  it("the open panel survives the next hook landing on the same call", () => {
    const done = pre("t1");
    const tool = toolStep("t1");
    const [steps, setSteps] = createSignal<TimelineItem[]>([done, tool]);
    const el = mountSteps(steps);
    hookButtons(el)[0].click();
    setSteps([done, tool, post("t1", { command: "/ws/format.sh" })]);
    expect(hookButtons(el)[0].getAttribute("aria-expanded")).toBe("true");
    expect(el.textContent).toContain("/ws/guard.sh");
    expect(el.textContent).toContain("/ws/format.sh");
  });
});
