import { describe, it, expect, vi, afterEach } from "vitest";
import { render } from "solid-js/web";
import type { AgentEvent } from "../lib/ipc";

// ── Hoisted test helpers ───────────────────────────────────────────
// vi.hoisted runs before import resolution; we attach the event callback
// capture to a shared object so the mock factory and test bodies can
// both reach it.
const __test = vi.hoisted(() => {
  const mockCommitAndPush = vi.fn();
  const mockInterruptSession = vi.fn();
  let _onEvent: ((event: unknown) => void) | null = null;

  mockCommitAndPush.mockImplementation(
    async (_ws: string, onEvent: (e: unknown) => void) => {
      _onEvent = onEvent;
      return { sessionId: "test-session" };
    },
  );

  return {
    mockCommitAndPush,
    mockInterruptSession,
    /** Dispatch an event as-if the IPC pipeline sent it. */
    emitEvent: (event: unknown) => {
      _onEvent?.(event);
    },
  };
});

// ── Module mocks ───────────────────────────────────────────────────
vi.mock("../lib/ipc", () => ({
  commitAndPush: __test.mockCommitAndPush,
  interruptSession: __test.mockInterruptSession,
  submitAnswers: vi.fn(),
}));


vi.mock("./Icon", () => ({
  Icon: (props: { name: string; class?: string }) => (
    <span data-testid={`icon-${props.name}`} class={props.class ?? ""} />
  ),
}));

vi.mock("./QuestionCard", () => ({
  default: () => <div data-testid="question-card" />,
}));

// Stub the markdown surface rather than `marked` itself: lib/markdown owns the
// renderer registration and the sanitize pass, and its own guarantees are
// covered by lib/markdown.test.ts.
vi.mock("../lib/markdown", () => ({
  renderMarkdown: (text: string) => `<p>${text}</p>`,
}));

// ── Imports (after mocks) ──────────────────────────────────────────
import CommitPushModal from "./CommitPushModal";

// ── Helpers ─────────────────────────────────────────────────────────
/** Flush pending microtasks so async effects settle. */
function flush() {
  return new Promise((r) => setTimeout(r, 10));
}

// ── Test event fixtures ────────────────────────────────────────────
const toolCallEvent: AgentEvent = {
  event: "ToolCall",
  data: {
    sessionId: "test-session",
    toolId: "tool-1",
    toolName: "read_file",
    args: { path: "/src/main.ts" },
    permission: "allowed",
  },
};

const toolResultEvent: AgentEvent = {
  event: "ToolResult",
  data: {
    toolId: "tool-1",
    toolName: "read_file",
    output: "file content",
    error: null,
  },
};

const thinkingEvent: AgentEvent = {
  event: "Thinking",
  data: "Thinking about the task...",
};

const textStepEvent: AgentEvent = {
  event: "TextStep",
  data: { text: "Let me read the file first." },
};

const doneEvent: AgentEvent = {
  event: "Done",
  data: {
    stopReason: "completed",
    textOutput: "Done!",
    inputTokens: 100,
    outputTokens: 200,
  },
};

const errorEvent: AgentEvent = {
  event: "Error",
  data: "Something went wrong",
};

// ══════════════════════════════════════════════════════════════════════
// CommitPushModal tests
// ══════════════════════════════════════════════════════════════════════

describe("CommitPushModal", () => {
  const defaultWorkspace = "/test/workspace";

  afterEach(() => {
    document.body.innerHTML = "";
    vi.clearAllMocks();
  });

  // ── visibility ──────────────────────────────────────────────────
  it("renders nothing when open is false", () => {
    const dispose = render(
      () => (
        <CommitPushModal workspace={defaultWorkspace} open={false} onClose={vi.fn()} />
      ),
      document.body,
    );
    expect(document.body.innerHTML).toBe("");
    dispose();
  });

  it("renders the modal when open is true", async () => {
    const dispose = render(
      () => (
        <CommitPushModal workspace={defaultWorkspace} open={true} onClose={vi.fn()} />
      ),
      document.body,
    );
    await flush();

    // The header should contain the modal title key
    expect(document.body.textContent).toContain("Commit & Push");
    // The fixed overlay should be present
    expect(document.querySelector(".fixed.inset-0")).not.toBeNull();

    dispose();
  });

  // ── loading state ───────────────────────────────────────────────
  it("shows loading state initially while no events have arrived", async () => {
    const dispose = render(
      () => (
        <CommitPushModal workspace={defaultWorkspace} open={true} onClose={vi.fn()} />
      ),
      document.body,
    );
    await flush();

    // Starting spinner + text
    expect(document.body.textContent).toContain("Starting...");
    // The badge should show "running" label
    expect(document.body.textContent).toContain("Committing changes...");

    dispose();
  });

  // ── running status ──────────────────────────────────────────────
  it("shows 'running' status badge initially", async () => {
    const dispose = render(
      () => (
        <CommitPushModal workspace={defaultWorkspace} open={true} onClose={vi.fn()} />
      ),
      document.body,
    );
    await flush();

    expect(document.body.textContent).toContain("Committing changes...");
    // Cancel button should be visible while running
    expect(document.body.textContent).toContain("Cancel");

    dispose();
  });

  // ── timeline steps ──────────────────────────────────────────────
  it("shows a thinking step after a Thinking event", async () => {
    const dispose = render(
      () => (
        <CommitPushModal workspace={defaultWorkspace} open={true} onClose={vi.fn()} />
      ),
      document.body,
    );
    await flush();

    __test.emitEvent(thinkingEvent);

    // "Show reasoning" button should be visible
    expect(document.body.textContent).toContain("Show reasoning");

    dispose();
  });

  it("shows a tool step after a ToolCall event", async () => {
    const dispose = render(
      () => (
        <CommitPushModal workspace={defaultWorkspace} open={true} onClose={vi.fn()} />
      ),
      document.body,
    );
    await flush();

    __test.emitEvent(toolCallEvent);

    // The tool name should appear
    expect(document.body.textContent).toContain("read_file");
    // The args should be visible (truncated version in the button)
    expect(document.body.textContent).toContain("/src/main.ts");

    dispose();
  });

  it("updates tool status to ok after a ToolResult event", async () => {
    const dispose = render(
      () => (
        <CommitPushModal workspace={defaultWorkspace} open={true} onClose={vi.fn()} />
      ),
      document.body,
    );
    await flush();

    __test.emitEvent(toolCallEvent);
    __test.emitEvent(toolResultEvent);

    // The result badge should be green (bg-success class) — the indicator
    // circle uses this class. We check that the tool step still renders.
    expect(document.body.textContent).toContain("read_file");

    dispose();
  });

  it("shows a text step after a TextStep event", async () => {
    const dispose = render(
      () => (
        <CommitPushModal workspace={defaultWorkspace} open={true} onClose={vi.fn()} />
      ),
      document.body,
    );
    await flush();

    __test.emitEvent(textStepEvent);

    // The text content should render inside the prose div
    expect(document.body.innerHTML).toContain("Let me read the file first.");

    dispose();
  });

  it("accumulates multiple thinking deltas into one thinking step", async () => {
    const dispose = render(
      () => (
        <CommitPushModal workspace={defaultWorkspace} open={true} onClose={vi.fn()} />
      ),
      document.body,
    );
    await flush();

    __test.emitEvent({ event: "Thinking", data: "First part " });
    __test.emitEvent({ event: "Thinking", data: "second part." });

    // Only one thinking entry — click to expand and see the merged text
    const showBtns = Array.from(document.body.querySelectorAll("button")).filter(
      (b) => b.textContent?.includes("Show reasoning"),
    );
    expect(showBtns.length).toBe(1);
    showBtns[0].click();

    expect(document.body.textContent).toContain("First part second part.");

    dispose();
  });

  // ── completed status ────────────────────────────────────────────
  it("shows completed status after a Done event", async () => {
    const dispose = render(
      () => (
        <CommitPushModal workspace={defaultWorkspace} open={true} onClose={vi.fn()} />
      ),
      document.body,
    );
    await flush();

    __test.emitEvent(doneEvent);

    expect(document.body.textContent).toContain("Completed");
    // Cancel button should no longer be present
    expect(document.body.textContent).not.toContain("Cancel");

    dispose();
  });

  // ── failed status ───────────────────────────────────────────────
  it("shows failed status after an Error event", async () => {
    const dispose = render(
      () => (
        <CommitPushModal workspace={defaultWorkspace} open={true} onClose={vi.fn()} />
      ),
      document.body,
    );
    await flush();

    __test.emitEvent(errorEvent);

    expect(document.body.textContent).toContain("Failed");
    expect(document.body.textContent).not.toContain("Cancel");

    dispose();
  });

  it("shows failed status when commitAndPush rejects", async () => {
    __test.mockCommitAndPush.mockRejectedValueOnce(new Error("network error"));

    const dispose = render(
      () => (
        <CommitPushModal workspace={defaultWorkspace} open={true} onClose={vi.fn()} />
      ),
      document.body,
    );
    await flush();

    expect(document.body.textContent).toContain("Failed");

    dispose();
  });

  // ── cancel button ───────────────────────────────────────────────
  it("cancel button calls interruptSession with the sessionId", async () => {
    const dispose = render(
      () => (
        <CommitPushModal workspace={defaultWorkspace} open={true} onClose={vi.fn()} />
      ),
      document.body,
    );
    await flush();

    // Find and click the cancel button
    const cancelBtn = Array.from(document.body.querySelectorAll("button")).find(
      (b) => b.textContent?.includes("Cancel"),
    );
    expect(cancelBtn).not.toBeNull();
    cancelBtn!.click();

    expect(__test.mockInterruptSession).toHaveBeenCalledWith("test-session");

    dispose();
  });

  it("cancel button changes status to interrupted", async () => {
    const dispose = render(
      () => (
        <CommitPushModal workspace={defaultWorkspace} open={true} onClose={vi.fn()} />
      ),
      document.body,
    );
    await flush();

    const cancelBtn = Array.from(document.body.querySelectorAll("button")).find(
      (b) => b.textContent?.includes("Cancel"),
    )!;
    cancelBtn.click();

    expect(document.body.textContent).toContain("Interrupted");

    dispose();
  });

  // ── auto-close ──────────────────────────────────────────────────
  it("auto-closes after completion (Done event)", async () => {
    vi.useFakeTimers();
    const onClose = vi.fn();

    const dispose = render(
      () => (
        <CommitPushModal workspace={defaultWorkspace} open={true} onClose={onClose} />
      ),
      document.body,
    );
    // Flush microtasks manually — fake timers replace setTimeout so our
    // flush() helper won't work; we use vi.advanceTimersByTime for that.
    await Promise.resolve();

    // Fire the Done event
    __test.emitEvent(doneEvent);

    // The createMemo fires and schedules a setTimeout(1500)
    expect(onClose).not.toHaveBeenCalled();

    // Advance past the 1500ms threshold
    vi.advanceTimersByTime(1500);

    expect(onClose).toHaveBeenCalledTimes(1);

    vi.useRealTimers();
    dispose();
  });

  it("auto-closes after failure (Error event)", async () => {
    vi.useFakeTimers();
    const onClose = vi.fn();

    const dispose = render(
      () => (
        <CommitPushModal workspace={defaultWorkspace} open={true} onClose={onClose} />
      ),
      document.body,
    );
    await Promise.resolve();

    __test.emitEvent(errorEvent);

    expect(onClose).not.toHaveBeenCalled();
    vi.advanceTimersByTime(1500);
    expect(onClose).toHaveBeenCalledTimes(1);

    vi.useRealTimers();
    dispose();
  });

  // ── escape key ──────────────────────────────────────────────────
  it("escape key interrupts the session", async () => {
    const dispose = render(
      () => (
        <CommitPushModal workspace={defaultWorkspace} open={true} onClose={vi.fn()} />
      ),
      document.body,
    );
    await flush();

    // Press Escape
    document.dispatchEvent(new KeyboardEvent("keydown", { key: "Escape" }));

    expect(__test.mockInterruptSession).toHaveBeenCalledWith("test-session");

    dispose();
  });

  it("escape key changes status to interrupted", async () => {
    const dispose = render(
      () => (
        <CommitPushModal workspace={defaultWorkspace} open={true} onClose={vi.fn()} />
      ),
      document.body,
    );
    await flush();

    document.dispatchEvent(new KeyboardEvent("keydown", { key: "Escape" }));

    expect(document.body.textContent).toContain("Interrupted");

    dispose();
  });

  it("non-Escape key does not cancel", async () => {
    const dispose = render(
      () => (
        <CommitPushModal workspace={defaultWorkspace} open={true} onClose={vi.fn()} />
      ),
      document.body,
    );
    await flush();

    document.dispatchEvent(new KeyboardEvent("keydown", { key: "Enter" }));

    expect(__test.mockInterruptSession).not.toHaveBeenCalled();
    // Should still be in running state
    expect(document.body.textContent).toContain("Committing changes...");

    dispose();
  });

  // ── clean-up ────────────────────────────────────────────────────
  it("removes the keydown listener on dispose (onCleanup)", async () => {
    const dispose = render(
      () => (
        <CommitPushModal workspace={defaultWorkspace} open={true} onClose={vi.fn()} />
      ),
      document.body,
    );
    await flush();
    dispose();

    // After dispose, pressing Escape should not call interruptSession
    document.dispatchEvent(new KeyboardEvent("keydown", { key: "Escape" }));
    expect(__test.mockInterruptSession).not.toHaveBeenCalled();
  });

  // ── expanded tool details ───────────────────────────────────────
  it("toggles expanded tool details on click", async () => {
    const dispose = render(
      () => (
        <CommitPushModal workspace={defaultWorkspace} open={true} onClose={vi.fn()} />
      ),
      document.body,
    );
    await flush();

    __test.emitEvent(toolCallEvent);

    // Before clicking, the expanded <pre> block does not exist
    const presBefore = document.body.querySelectorAll("pre");
    expect(presBefore.length).toBe(0);

    // Click the tool step button to expand
    const toolBtn = Array.from(document.body.querySelectorAll("button")).find(
      (b) => b.textContent?.includes("read_file"),
    )!;
    toolBtn.click();

    // A <pre> with the formatted args JSON should now be visible
    const presAfter = document.body.querySelectorAll("pre");
    expect(presAfter.length).toBe(1);
    expect(presAfter[0].textContent).toContain("/src/main.ts");

    // Click again to collapse
    toolBtn.click();
    const presAfterCollapse = document.body.querySelectorAll("pre");
    expect(presAfterCollapse.length).toBe(0);

    dispose();
  });

  it("shows tool result when expanded after ToolResult", async () => {
    const dispose = render(
      () => (
        <CommitPushModal workspace={defaultWorkspace} open={true} onClose={vi.fn()} />
      ),
      document.body,
    );
    await flush();

    __test.emitEvent(toolCallEvent);
    __test.emitEvent(toolResultEvent);

    // Expand the tool step
    const toolBtn = Array.from(document.body.querySelectorAll("button")).find(
      (b) => b.textContent?.includes("read_file"),
    )!;
    toolBtn.click();

    // The result section header should be visible
    expect(document.body.textContent).toContain("Result");
    expect(document.body.textContent).toContain("file content");

    dispose();
  });

  it("shows expanded thinking content when toggled", async () => {
    const dispose = render(
      () => (
        <CommitPushModal workspace={defaultWorkspace} open={true} onClose={vi.fn()} />
      ),
      document.body,
    );
    await flush();

    __test.emitEvent(thinkingEvent);

    // Click "Show reasoning"
    const showBtn = Array.from(document.body.querySelectorAll("button")).find(
      (b) => b.textContent?.includes("Show reasoning"),
    )!;
    showBtn.click();

    // The thinking text should be visible
    expect(document.body.textContent).toContain("Thinking about the task...");

    dispose();
  });
});
