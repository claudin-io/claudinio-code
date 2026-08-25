import { describe, it, expect, vi, afterEach } from "vitest";
import { render } from "solid-js/web";

// TimelineRows pulls in DiffViewer, which pulls in monaco — it needs browser
// APIs jsdom does not implement. Same stub the other viewer tests use.
vi.mock("monaco-editor", () => ({
  editor: { create: vi.fn(), createDiffEditor: vi.fn(), defineTheme: vi.fn() },
}));

import { HookRow, QualityRow } from "./TimelineRows";
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
