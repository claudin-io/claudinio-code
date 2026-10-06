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
// Quiet hooks — an icon each, a line per run
// ─────────────────────────────────────────────────────────────

function mountSteps(steps: () => TimelineItem[]): HTMLDivElement {
  host = document.createElement("div");
  document.body.appendChild(host);
  dispose = render(
    () => <TimelineSteps steps={steps()} expandedStep={null} onToggle={() => {}} isLive={false} />,
    host,
  );
  return host;
}

const hookStep = (over: Partial<NonNullable<TimelineItem["hook"]>> = {}): TimelineItem => ({
  type: "hook",
  hook: hookItem(over),
});

const toolStep = (toolId: string): TimelineItem => ({
  type: "tool",
  tool: {
    call: { sessionId: "s", toolId, toolName: "bash", args: { command: "git status" }, permission: "allow" },
    status: "ok",
  },
});

const chips = (el: HTMLElement): HTMLButtonElement[] =>
  Array.from(el.querySelectorAll<HTMLButtonElement>('button[aria-label*=" hook · "]'));

describe("quiet hooks in the timeline", () => {
  it("a hook with nothing to report is an icon, not a row of text", () => {
    // The point of the change: a PreToolUse hook on every call used to put
    // "PreToolUse hook · exit 0 · 42ms" under every tool in the thread.
    const el = mountSteps(() => [hookStep()]);
    expect(chips(el)).toHaveLength(1);
    expect(el.textContent).not.toContain("PreToolUse hook");
    expect(el.textContent).not.toContain("exit 0");
  });

  it("the icon still names what ran, for whoever cannot hover", () => {
    const el = mountSteps(() => [hookStep()]);
    expect(chips(el)[0].getAttribute("aria-label")).toBe("PreToolUse hook · ok · exit 0 · 42ms");
  });

  it("hooks that fired together share one line", () => {
    const el = mountSteps(() => [
      hookStep(),
      hookStep({ hookId: "s:PostToolUse:1", event: "PostToolUse" }),
    ]);
    const found = chips(el);
    expect(found).toHaveLength(2);
    expect(found[0].parentElement).toBe(found[1].parentElement);
  });

  it("hooks around different tool calls stay with their own call", () => {
    const el = mountSteps(() => [hookStep(), toolStep("t1"), hookStep(), toolStep("t2")]);
    const found = chips(el);
    expect(found).toHaveLength(2);
    expect(found[0].parentElement).not.toBe(found[1].parentElement);
  });

  it("a failure among quiet hooks keeps its full row", () => {
    // Shrinking this one would make a broken hook look like a working one.
    const el = mountSteps(() => [
      hookStep(),
      hookStep({ status: "error", exitCode: 127, error: "brain: not found" }),
      hookStep(),
    ]);
    expect(el.textContent).toContain("PreToolUse hook failed");
    expect(el.textContent).toContain("brain: not found");
    expect(chips(el)).toHaveLength(2);
  });

  it.each([
    ["blocked", "blocked this"],
    ["timeout", "timed out"],
    ["skipped_untrusted", "waiting for your approval"],
  ])("a %s hook keeps its full row", (status, text) => {
    const el = mountSteps(() => [hookStep({ status, exitCode: null })]);
    expect(el.textContent).toContain(text);
    expect(chips(el)).toHaveLength(0);
  });

  it("a hook that left a message for the user keeps its full row", () => {
    const el = mountSteps(() => [hookStep({ systemMessage: "formatted 3 files" })]);
    expect(el.textContent).toContain("formatted 3 files");
    expect(chips(el)).toHaveLength(0);
  });

  it("context a hook injected keeps its full row", () => {
    const el = mountSteps(() => [
      hookStep({ event: "UserPromptSubmit", command: "", context: "the port is 8080" }),
    ]);
    expect(el.textContent).toContain("UserPromptSubmit hook added context");
    expect(el.textContent).toContain("the port is 8080");
  });

  it("a running hook shows the label its author wrote", () => {
    const el = mountSteps(() => [
      hookStep({ status: "running", statusMessage: "Reading this project's brain" }),
    ]);
    expect(el.textContent).toContain("Reading this project's brain");
  });

  it("hovering explains the hook: what fired, how it ended, how long it took", () => {
    const el = mountSteps(() => [hookStep({ decision: "allow" })]);
    const chip = chips(el)[0];
    chip.dispatchEvent(new MouseEvent("mouseenter"));
    const page = () => document.body.textContent ?? "";
    expect(page()).toContain("PreToolUse hook");
    expect(page()).toContain("Fires before a tool call is dispatched.");
    expect(page()).toContain("Exit code0");
    expect(page()).toContain("Took42ms");
    expect(page()).toContain("Decisionallow");
    expect(page()).toContain("plugin: claudinio-brain");
    expect(page()).toContain("/ws/guard.sh");

    chip.dispatchEvent(new MouseEvent("mouseleave"));
    expect(page()).not.toContain("Fires before a tool call is dispatched.");
  });

  it("keyboard focus explains it too", () => {
    const el = mountSteps(() => [hookStep()]);
    const chip = chips(el)[0];
    chip.dispatchEvent(new FocusEvent("focus"));
    expect(document.body.textContent).toContain("Fires before a tool call is dispatched.");
    chip.dispatchEvent(new FocusEvent("blur"));
    expect(document.body.textContent).not.toContain("Fires before a tool call is dispatched.");
  });

  it("clicking shows the command that ran and what it printed", () => {
    const el = mountSteps(() => [hookStep({ output: "lint: 0 problems" })]);
    const chip = chips(el)[0];
    expect(el.textContent).not.toContain("/ws/guard.sh");

    chip.click();
    expect(chip.getAttribute("aria-expanded")).toBe("true");
    expect(el.textContent).toContain("/ws/guard.sh");
    expect(el.textContent).toContain("lint: 0 problems");

    chip.click();
    expect(chip.getAttribute("aria-expanded")).toBe("false");
    expect(el.textContent).not.toContain("/ws/guard.sh");
  });

  it("says so when a hook printed nothing, rather than showing an empty box", () => {
    const el = mountSteps(() => [hookStep({ output: "" })]);
    chips(el)[0].click();
    expect(el.textContent).toContain("No output to show.");
  });

  it("shows stderr read back from a reopened session", () => {
    // chatRecords maps the record's stderr onto `error`; an ok hook that only
    // warned must not turn into a failure row, and must not lose the warning.
    const el = mountSteps(() => [hookStep({ error: "warning: cache is stale" })]);
    expect(el.textContent).not.toContain("warning: cache is stale");
    chips(el)[0].click();
    expect(el.textContent).toContain("warning: cache is stale");
  });

  it("opening one hook closes the one that was open", () => {
    const el = mountSteps(() => [
      hookStep({ command: "/ws/first.sh" }),
      hookStep({ command: "/ws/second.sh" }),
    ]);
    const [first, second] = chips(el);
    first.click();
    second.click();
    expect(el.textContent).toContain("/ws/second.sh");
    expect(el.textContent).not.toContain("/ws/first.sh");
  });

  it("the tooltip stands down while the hook's panel is open", () => {
    const el = mountSteps(() => [hookStep()]);
    const chip = chips(el)[0];
    chip.dispatchEvent(new MouseEvent("mouseenter"));
    chip.click();
    expect(document.body.textContent).not.toContain("Click to see the command and its output.");
  });

  it("follows a live run: the next hook joins the line, the running one settles", () => {
    const pre = hookStep({ status: "running", exitCode: null, durationMs: undefined });
    const [steps, setSteps] = createSignal<TimelineItem[]>([toolStep("t1"), pre]);
    const el = mountSteps(steps);
    expect(chips(el)[0].getAttribute("aria-label")).toBe("PreToolUse hook · running");

    // ChatPanel replaces the step object when HookFinished lands, then appends.
    const done = hookStep();
    const post = hookStep({ hookId: "s:PostToolUse:1", event: "PostToolUse", durationMs: 7 });
    setSteps([toolStep("t1"), done, post]);

    const found = chips(el);
    expect(found.map((c) => c.getAttribute("aria-label"))).toEqual([
      "PreToolUse hook · ok · exit 0 · 42ms",
      "PostToolUse hook · ok · exit 0 · 7ms",
    ]);
    expect(found[0].parentElement).toBe(found[1].parentElement);
  });
});
