import { describe, it, expect, vi, afterEach } from "vitest";
import { render } from "solid-js/web";

// ── Hoisted setup ────────────────────────────────────────────

const mockEditor = vi.hoisted(() => ({
  getValue: vi.fn(() => "default editor value"),
  setValue: vi.fn(),
  focus: vi.fn(),
  dispose: vi.fn(),
  onDidChangeModelContent: vi.fn(() => ({ dispose: vi.fn() })),
}));

// ── Mocks ────────────────────────────────────────────────────

vi.mock("monaco-editor", () => ({
  editor: {
    create: vi.fn(() => mockEditor),
    setTheme: vi.fn(),
    defineTheme: vi.fn(),
  },
}));

vi.mock("./Icon", () => ({ Icon: () => null }));
vi.mock("../lib/grill-me", () => ({ t: (k: string) => k }));

// ── Import the component (vi.mock is hoisted above imports) ──

import TextEditorModal from "./TextEditorModal";

// ── Helpers ───────────────────────────────────────────────────

/** Flush pending microtasks so Solid reactivity settles. */
function flush() {
  return new Promise<void>((r) => setTimeout(r, 10));
}

// ── Tests ─────────────────────────────────────────────────────

describe("TextEditorModal", () => {
  afterEach(() => {
    document.body.innerHTML = "";
    vi.clearAllMocks();
  });

  // ────── Render & initialText ────────────────────────────────

  it("renders modal with initialText and creates Monaco editor", async () => {
    const dispose = render(
      () => <TextEditorModal initialText="hello world" onClose={vi.fn()} />,
      document.body,
    );
    await flush();

    const { editor } = await import("monaco-editor");
    expect(editor.create).toHaveBeenCalledTimes(1);
    expect(editor.create).toHaveBeenCalledWith(
      expect.any(HTMLElement),
      expect.objectContaining({
        value: "hello world",
        language: "text",
        theme: "vs-dark",
        automaticLayout: true,
        minimap: { enabled: false },
        scrollBeyondLastLine: false,
        wordWrap: "on",
      }),
    );

    // Title is rendered via t("editor.title")
    const title = document.body.querySelector(".font-semibold");
    expect(title).toBeTruthy();
    expect(title!.textContent).toBe("editor.title");

    dispose();
  });

  // ────── Close button → returns editor value ────────────────

  it("calls onClose with editor value when close button is clicked", async () => {
    const onClose = vi.fn();
    mockEditor.getValue.mockReturnValue("edited content");

    render(
      () => <TextEditorModal initialText="original" onClose={onClose} />,
      document.body,
    );
    await flush();

    // Last button is the close (x) button
    const buttons = document.body.querySelectorAll("button");
    const closeButton = buttons[buttons.length - 1];
    closeButton.click();

    expect(onClose).toHaveBeenCalledTimes(1);
    expect(onClose).toHaveBeenCalledWith("edited content");
  });

  it("falls back to initialText when editor.getValue returns nullish", async () => {
    const onClose = vi.fn();
    mockEditor.getValue.mockReturnValue(undefined as unknown as string);

    render(
      () => (
        <TextEditorModal initialText="fallback text" onClose={onClose} />
      ),
      document.body,
    );
    await flush();

    const buttons = document.body.querySelectorAll("button");
    buttons[buttons.length - 1].click();

    expect(onClose).toHaveBeenCalledWith("fallback text");
  });

  // ────── Escape key ──────────────────────────────────────────

  it("calls onClose when Escape key is pressed", async () => {
    const onClose = vi.fn();
    mockEditor.getValue.mockReturnValue("escaped value");

    render(
      () => <TextEditorModal initialText="text" onClose={onClose} />,
      document.body,
    );
    await flush();

    document.dispatchEvent(new KeyboardEvent("keydown", { key: "Escape" }));

    expect(onClose).toHaveBeenCalledTimes(1);
    expect(onClose).toHaveBeenCalledWith("escaped value");
  });

  it("does NOT call onClose for non-Escape key presses", async () => {
    const onClose = vi.fn();

    render(
      () => <TextEditorModal initialText="text" onClose={onClose} />,
      document.body,
    );
    await flush();

    document.dispatchEvent(new KeyboardEvent("keydown", { key: "Enter" }));
    document.dispatchEvent(new KeyboardEvent("keydown", { key: "Tab" }));

    expect(onClose).not.toHaveBeenCalled();
  });

  // ────── Overlay click (backdrop) ────────────────────────────

  it("calls onClose when clicking the overlay background", async () => {
    const onClose = vi.fn();
    mockEditor.getValue.mockReturnValue("overlay value");

    render(
      () => <TextEditorModal initialText="text" onClose={onClose} />,
      document.body,
    );
    await flush();

    // The outermost div is the backdrop (.fixed.inset-0)
    const overlay = document.body.firstElementChild as HTMLElement;
    overlay.click();

    expect(onClose).toHaveBeenCalledWith("overlay value");
  });

  it("does NOT call onClose when clicking inside the modal content", async () => {
    const onClose = vi.fn();

    render(
      () => <TextEditorModal initialText="text" onClose={onClose} />,
      document.body,
    );
    await flush();

    // The inner modal div has class "rounded-xl"
    const innerModal = document.body.querySelector(".rounded-xl") as HTMLElement;
    innerModal.click();

    expect(onClose).not.toHaveBeenCalled();
  });

  // ────── Enhance button visibility ───────────────────────────

  it("hides enhance button when onEnhance is NOT provided", async () => {
    render(
      () => <TextEditorModal initialText="text" onClose={vi.fn()} />,
      document.body,
    );
    await flush();

    // Only the close button should be present
    const buttons = document.body.querySelectorAll("button");
    expect(buttons.length).toBe(1);
    expect(
      Array.from(buttons).some((b) => b.title === "enhance.button"),
    ).toBe(false);
  });

  it("shows enhance button when onEnhance is provided", async () => {
    render(
      () => (
        <TextEditorModal
          initialText="text"
          onClose={vi.fn()}
          onEnhance={async (t) => t}
        />
      ),
      document.body,
    );
    await flush();

    // Enhance button + close button
    const buttons = document.body.querySelectorAll("button");
    expect(buttons.length).toBe(2);
  });

  // ────── onEnhance: happy path ───────────────────────────────

  it("calls onEnhance with editor value and updates editor with result", async () => {
    const onEnhance = vi.fn(async (text: string) => text + " [enhanced]");
    mockEditor.getValue.mockReturnValue("original text");

    render(
      () => (
        <TextEditorModal
          initialText="original"
          onClose={vi.fn()}
          onEnhance={onEnhance}
        />
      ),
      document.body,
    );
    await flush();

    const buttons = document.body.querySelectorAll("button");
    const enhanceButton = buttons[0];
    enhanceButton.click();

    await flush(); // Wait for the async handler to resolve

    expect(onEnhance).toHaveBeenCalledTimes(1);
    expect(onEnhance).toHaveBeenCalledWith("original text");

    // After enhance resolves, the result is set into the editor and focus restored
    expect(mockEditor.setValue).toHaveBeenCalledWith("original text [enhanced]");
    expect(mockEditor.focus).toHaveBeenCalled();
  });

  // ────── onEnhance loading state (spinner + disabled) ────────

  it("shows spinner and disables button during enhance loading", async () => {
    let resolvePromise!: (v: string) => void;
    const enhancePromise = new Promise<string>((r) => {
      resolvePromise = r;
    });
    const onEnhance = vi.fn(() => enhancePromise);
    mockEditor.getValue.mockReturnValue("enhance me");

    render(
      () => (
        <TextEditorModal
          initialText="text"
          onClose={vi.fn()}
          onEnhance={onEnhance}
        />
      ),
      document.body,
    );
    await flush();

    const buttons = document.body.querySelectorAll("button");
    const enhanceButton = buttons[0];

    // Click to start enhancing
    enhanceButton.click();
    // Flush once so the click handler fires and sets isEnhancing(true)
    await flush();

    // Button should be disabled while loading
    expect((enhanceButton as HTMLButtonElement).disabled).toBe(true);

    // The title should reflect the loading state
    expect((enhanceButton as HTMLButtonElement).title).toBe(
      "enhance.enhancing",
    );

    // Resolve the promise
    resolvePromise("final text");
    await flush();

    // Button should be re-enabled after enhancement completes
    expect((enhanceButton as HTMLButtonElement).disabled).toBe(false);

    // Title should be back to the non-loading state
    expect((enhanceButton as HTMLButtonElement).title).toBe("enhance.button");
  });

  it("calls onEnhance and stays enabled when enhancement finishes", async () => {
    const onEnhance = vi.fn(async (t: string) => t);
    mockEditor.getValue.mockReturnValue("text");

    render(
      () => (
        <TextEditorModal
          initialText="text"
          onClose={vi.fn()}
          onEnhance={onEnhance}
        />
      ),
      document.body,
    );
    await flush();

    const buttons = document.body.querySelectorAll("button");
    const enhanceButton = buttons[0];
    enhanceButton.click();
    await flush();

    // After completion, button is re-enabled
    expect((enhanceButton as HTMLButtonElement).disabled).toBe(false);
  });

  // ────── onEnhance error handling ────────────────────────────

  it("resets isEnhancing when onEnhance throws", async () => {
    const onEnhance = vi.fn(async () => {
      throw new Error("enhancement failed");
    });
    mockEditor.getValue.mockReturnValue("text");

    render(
      () => (
        <TextEditorModal
          initialText="text"
          onClose={vi.fn()}
          onEnhance={onEnhance}
        />
      ),
      document.body,
    );
    await flush();

    const buttons = document.body.querySelectorAll("button");
    const enhanceButton = buttons[0];
    enhanceButton.click();
    await flush(); // Wait for the rejected promise

    // Button should be re-enabled even after error
    expect((enhanceButton as HTMLButtonElement).disabled).toBe(false);
    // Editor value should NOT have been updated
    expect(mockEditor.setValue).not.toHaveBeenCalled();
  });

  // ────── Cleanup ─────────────────────────────────────────────

  it("disposes editor and removes event listener on unmount", async () => {
    const onClose = vi.fn();
    const dispose = render(
      () => <TextEditorModal initialText="text" onClose={onClose} />,
      document.body,
    );
    await flush();

    // Confirm listener is active during lifetime
    document.dispatchEvent(new KeyboardEvent("keydown", { key: "Escape" }));
    expect(onClose).toHaveBeenCalledTimes(1);

    // Unmount
    dispose();

    // Editor should be disposed
    expect(mockEditor.dispose).toHaveBeenCalledTimes(1);

    // Event listener should be removed — Escape should no longer trigger onClose
    document.dispatchEvent(new KeyboardEvent("keydown", { key: "Escape" }));
    expect(onClose).toHaveBeenCalledTimes(1); // Still 1 — no new call
  });
});
