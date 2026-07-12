import { describe, it, expect, vi, afterEach, beforeEach } from "vitest";
import { render } from "solid-js/web";

// ── Hoisted setup ────────────────────────────────────────────
// Runs before all vi.mock factories so the mockEditor object
// is available when the monaco-editor mock is created.

const mockEditor = vi.hoisted(() => ({
  getValue: vi.fn(),
  setValue: vi.fn(),
  focus: vi.fn(),
  dispose: vi.fn(),
  onDidChangeModelContent: vi.fn(),
  /** Stores the content-change callback passed to onDidChangeModelContent. */
  _contentChangeCb: null as (() => void) | null,
}));

/**
 * Simulate a Monaco model content change.
 * Changes the getValue mock return, then fires the stored callback.
 */
function simulateContentChange(newValue: string) {
  mockEditor.getValue.mockReturnValue(newValue);
  mockEditor._contentChangeCb?.();
}

// ── Mocks (hoisted by vitest) ────────────────────────────────

vi.mock("monaco-editor", () => ({
  editor: {
    create: vi.fn(() => mockEditor),
    defineTheme: vi.fn(),
  },
}));

vi.mock("./Icon", () => ({
  Icon: () => null,
}));

vi.mock("../lib/grill-me", () => ({
  t: (k: string) => k,
}));

vi.mock("../lib/ipc", () => ({
  readFile: vi.fn().mockResolvedValue("original content"),
  writeFile: vi.fn().mockResolvedValue(undefined),
}));

vi.mock("../lib/monacoThemes", () => ({
  defineMonacoThemes: vi.fn(),
}));

vi.mock("../lib/theme", () => ({
  theme: vi.fn(() => "dark"),
}));

// ── Imports ──────────────────────────────────────────────────

import FileEditorModal from "./FileEditorModal";
import { writeFile } from "../lib/ipc";

// ── Helpers ──────────────────────────────────────────────────

/** Flush pending micro/macro tasks so Solid reactivity settles. */
function flush(): Promise<void> {
  return new Promise((r) => setTimeout(r, 10));
}

/**
 * Render the modal and wait for the async initEditor to settle.
 * The component uses createEffect + async initEditor, so it takes
 * two event-loop turns to fully settle.
 */
async function renderModal(props: {
  filePath?: string;
  rootPath?: string;
} = {}) {
  const onClose = vi.fn();
  const dispose = render(
    () => (
      <FileEditorModal
        filePath={props.filePath ?? "/test/file.ts"}
        rootPath={props.rootPath ?? "/test"}
        onClose={onClose}
      />
    ),
    document.body,
  );
  // First flush: createEffect fires → initEditor starts (async)
  await flush();
  // Second flush: initEditor completes (readFile, monaco.editor.create, etc.)
  await flush();
  return { onClose, dispose };
}

// ── Tests ────────────────────────────────────────────────────

describe("FileEditorModal component", () => {
  afterEach(() => {
    document.body.innerHTML = "";
    vi.clearAllMocks();
  });

  beforeEach(() => {
    // Re-bind the onDidChangeModelContent mock to store the callback,
    // and reset getValue to the default (clearAllMocks clears these).
    mockEditor._contentChangeCb = null;
    mockEditor.getValue.mockReturnValue("original content");
    mockEditor.onDidChangeModelContent.mockImplementation(
      (cb: () => void) => {
        mockEditor._contentChangeCb = cb;
        return { dispose: vi.fn() };
      },
    );
  });

  // ────── Basic rendering ────────────────────────────────────

  it("renders file name and relative path in the header", async () => {
    const { dispose } = await renderModal();

    const nameEl = document.body.querySelector(".font-semibold");
    expect(nameEl).toBeTruthy();
    expect(nameEl!.textContent).toBe("file.ts");

    const pathEl = document.body.querySelector(".text-ink-faint");
    expect(pathEl).toBeTruthy();
    expect(pathEl!.textContent).toBe("file.ts");

    dispose();
  });

  it("renders save and close buttons", async () => {
    const { dispose } = await renderModal();

    const buttons = document.body.querySelectorAll("button");
    expect(buttons.length).toBe(2);

    const saveBtn = Array.from(buttons).find(
      (b) => b.textContent === "fileEditor.save",
    );
    expect(saveBtn).toBeTruthy();

    const closeBtn = Array.from(buttons).find(
      (b) => b.textContent !== "fileEditor.save",
    );
    expect(closeBtn).toBeTruthy();

    dispose();
  });

  it("creates Monaco editor with the file content", async () => {
    const { dispose } = await renderModal();

    const { editor } = await import("monaco-editor");
    expect(editor.create).toHaveBeenCalledTimes(1);
    expect(editor.create).toHaveBeenCalledWith(
      expect.any(HTMLElement),
      expect.objectContaining({
        value: "original content",
        language: "typescript",
        theme: "claudinio-dark",
        automaticLayout: true,
        minimap: { enabled: true },
        scrollBeyondLastLine: false,
        wordWrap: "off",
        fontSize: 13,
        tabSize: 2,
      }),
    );

    dispose();
  });

  it("calls readFile with the correct file path", async () => {
    const { dispose } = await renderModal({
      filePath: "/root/sub/code.py",
    });

    const { readFile } = await import("../lib/ipc");
    expect(readFile).toHaveBeenCalledTimes(1);
    expect(readFile).toHaveBeenCalledWith("/root/sub/code.py");

    dispose();
  });

  // ────── Saving (Ctrl+S / Cmd+S) ────────────────────────────

  it("saves file when Ctrl+S is pressed", async () => {
    const { onClose, dispose } = await renderModal();

    mockEditor.getValue.mockReturnValue("modified content");
    document.dispatchEvent(
      new KeyboardEvent("keydown", { key: "s", ctrlKey: true }),
    );
    await flush();

    expect(writeFile).toHaveBeenCalledTimes(1);
    expect(writeFile).toHaveBeenCalledWith("/test/file.ts", "modified content");
    // Save should not close the modal
    expect(onClose).not.toHaveBeenCalled();

    dispose();
  });

  it("saves file when Cmd+S is pressed (macOS)", async () => {
    const { onClose, dispose } = await renderModal();

    mockEditor.getValue.mockReturnValue("cmd-s content");
    document.dispatchEvent(
      new KeyboardEvent("keydown", { key: "s", metaKey: true }),
    );
    await flush();

    expect(writeFile).toHaveBeenCalledWith("/test/file.ts", "cmd-s content");
    expect(onClose).not.toHaveBeenCalled();

    dispose();
  });

  it("does not trigger save for S without Ctrl/Meta", async () => {
    const { dispose } = await renderModal();

    document.dispatchEvent(new KeyboardEvent("keydown", { key: "s" }));

    expect(writeFile).not.toHaveBeenCalled();

    dispose();
  });

  it("prefers preventDefault on Ctrl+S so the browser does not save-page", async () => {
    const { dispose } = await renderModal();

    const evt = new KeyboardEvent("keydown", {
      key: "s",
      ctrlKey: true,
      cancelable: true,
    });
    const preventDefault = vi.spyOn(evt, "preventDefault");
    document.dispatchEvent(evt);

    expect(preventDefault).toHaveBeenCalled();

    dispose();
  });

  // ────── Dirty state `*` indicator ──────────────────────────

  it("shows no dirty indicator initially", async () => {
    const { dispose } = await renderModal();

    // The dirty indicator is <span class="text-accent font-bold">*</span>
    expect(
      document.body.querySelector(".text-accent.font-bold"),
    ).toBeFalsy();

    dispose();
  });

  it("shows dirty indicator * after content changes", async () => {
    const { dispose } = await renderModal();

    expect(
      document.body.querySelector(".text-accent.font-bold"),
    ).toBeFalsy();

    simulateContentChange("modified content");
    await flush();

    const indicator = document.body.querySelector(".text-accent.font-bold");
    expect(indicator).toBeTruthy();
    expect(indicator!.textContent).toBe("*");

    dispose();
  });

  it("hides dirty indicator after save", async () => {
    mockEditor.getValue.mockReturnValue("modified content");
    const { dispose } = await renderModal();

    // Make dirty
    simulateContentChange("modified content");
    await flush();
    expect(
      document.body.querySelector(".text-accent.font-bold"),
    ).toBeTruthy();

    // Save (Ctrl+S) — writeFile resets originalContent, clearing dirty
    document.dispatchEvent(
      new KeyboardEvent("keydown", { key: "s", ctrlKey: true }),
    );
    await flush();

    expect(
      document.body.querySelector(".text-accent.font-bold"),
    ).toBeFalsy();

    dispose();
  });

  it("does not show dirty indicator when content matches original", async () => {
    const { dispose } = await renderModal();

    // getValue still returns "original content", which matches originalContent.
    // Firing the callback should NOT set dirty.
    simulateContentChange("original content");
    await flush();

    expect(
      document.body.querySelector(".text-accent.font-bold"),
    ).toBeFalsy();

    dispose();
  });

  // ────── Close button ───────────────────────────────────────

  it("calls onClose when close button is clicked (not dirty)", async () => {
    const { onClose, dispose } = await renderModal();

    const closeBtn = [...document.body.querySelectorAll("button")].find(
      (b) => b.textContent !== "fileEditor.save",
    )!;
    closeBtn.click();

    expect(onClose).toHaveBeenCalledTimes(1);

    dispose();
  });

  it("shows unsaved confirm when close button clicked while dirty, and closes on accept", async () => {
    const { onClose, dispose } = await renderModal();

    simulateContentChange("modified content");
    await flush();

    const confirmSpy = vi.spyOn(window, "confirm");
    confirmSpy.mockReturnValue(true);

    const closeBtn = [...document.body.querySelectorAll("button")].find(
      (b) => b.textContent !== "fileEditor.save",
    )!;
    closeBtn.click();

    expect(confirmSpy).toHaveBeenCalledWith("fileEditor.unsavedMessage");
    expect(onClose).toHaveBeenCalledTimes(1);

    confirmSpy.mockRestore();
    dispose();
  });

  it("does NOT close when close button clicked while dirty and confirm cancelled", async () => {
    const { onClose, dispose } = await renderModal();

    simulateContentChange("modified content");
    await flush();

    const confirmSpy = vi.spyOn(window, "confirm");
    confirmSpy.mockReturnValue(false);

    const closeBtn = [...document.body.querySelectorAll("button")].find(
      (b) => b.textContent !== "fileEditor.save",
    )!;
    closeBtn.click();

    expect(confirmSpy).toHaveBeenCalled();
    expect(onClose).not.toHaveBeenCalled();

    confirmSpy.mockRestore();
    dispose();
  });

  it("save button saves content but does NOT close the modal", async () => {
    const { onClose, dispose } = await renderModal();

    mockEditor.getValue.mockReturnValue("save btn content");
    const saveBtn = [...document.body.querySelectorAll("button")].find(
      (b) => b.textContent === "fileEditor.save",
    )!;
    saveBtn.click();
    await flush();

    expect(writeFile).toHaveBeenCalledWith(
      "/test/file.ts",
      "save btn content",
    );
    expect(onClose).not.toHaveBeenCalled();

    dispose();
  });

  // ────── Escape key ─────────────────────────────────────────

  it("calls onClose when Escape is pressed (not dirty)", async () => {
    const { onClose, dispose } = await renderModal();

    document.dispatchEvent(new KeyboardEvent("keydown", { key: "Escape" }));

    expect(onClose).toHaveBeenCalledTimes(1);

    dispose();
  });

  it("shows unsaved confirm when Escape pressed while dirty, and closes on accept", async () => {
    const { onClose, dispose } = await renderModal();

    simulateContentChange("modified content");
    await flush();

    const confirmSpy = vi.spyOn(window, "confirm");
    confirmSpy.mockReturnValue(true);

    document.dispatchEvent(new KeyboardEvent("keydown", { key: "Escape" }));

    expect(confirmSpy).toHaveBeenCalledWith("fileEditor.unsavedMessage");
    expect(onClose).toHaveBeenCalledTimes(1);

    confirmSpy.mockRestore();
    dispose();
  });

  it("does NOT close on Escape when confirm is cancelled", async () => {
    const { onClose, dispose } = await renderModal();

    simulateContentChange("modified content");
    await flush();

    const confirmSpy = vi.spyOn(window, "confirm");
    confirmSpy.mockReturnValue(false);

    document.dispatchEvent(new KeyboardEvent("keydown", { key: "Escape" }));

    expect(onClose).not.toHaveBeenCalled();

    confirmSpy.mockRestore();
    dispose();
  });

  it("does not call onClose for non-Escape keys", async () => {
    const { onClose, dispose } = await renderModal();

    document.dispatchEvent(new KeyboardEvent("keydown", { key: "Enter" }));
    document.dispatchEvent(new KeyboardEvent("keydown", { key: "Tab" }));
    document.dispatchEvent(new KeyboardEvent("keydown", { key: " " }));

    expect(onClose).not.toHaveBeenCalled();

    dispose();
  });

  // ────── Overlay / backdrop click ───────────────────────────

  it("calls onClose when clicking overlay background (not dirty)", async () => {
    const { onClose, dispose } = await renderModal();

    const overlay = document.body.firstElementChild as HTMLElement;
    overlay.click();

    expect(onClose).toHaveBeenCalledTimes(1);

    dispose();
  });

  it("shows confirm when clicking overlay while dirty, and closes on accept", async () => {
    const { onClose, dispose } = await renderModal();

    simulateContentChange("modified content");
    await flush();

    const confirmSpy = vi.spyOn(window, "confirm");
    confirmSpy.mockReturnValue(true);

    const overlay = document.body.firstElementChild as HTMLElement;
    overlay.click();

    expect(confirmSpy).toHaveBeenCalledWith("fileEditor.unsavedMessage");
    expect(onClose).toHaveBeenCalledTimes(1);

    confirmSpy.mockRestore();
    dispose();
  });

  it("does NOT close when clicking overlay while dirty and confirm cancelled", async () => {
    const { onClose, dispose } = await renderModal();

    simulateContentChange("modified content");
    await flush();

    const confirmSpy = vi.spyOn(window, "confirm");
    confirmSpy.mockReturnValue(false);

    const overlay = document.body.firstElementChild as HTMLElement;
    overlay.click();

    expect(onClose).not.toHaveBeenCalled();

    confirmSpy.mockRestore();
    dispose();
  });

  it("does NOT call onClose when clicking inside the modal content", async () => {
    const { onClose, dispose } = await renderModal();

    const innerModal = document.body.querySelector(".rounded-xl") as HTMLElement;
    innerModal.click();

    expect(onClose).not.toHaveBeenCalled();

    dispose();
  });

  // ────── Language detection in editor ───────────────────────

  it("passes correct language for .rs file", async () => {
    const { dispose } = await renderModal({ filePath: "/test/main.rs" });

    const { editor } = await import("monaco-editor");
    expect(editor.create).toHaveBeenCalledWith(
      expect.any(HTMLElement),
      expect.objectContaining({ language: "rust" }),
    );

    dispose();
  });

  it("passes correct language for .py file", async () => {
    const { dispose } = await renderModal({ filePath: "/test/script.py" });

    const { editor } = await import("monaco-editor");
    expect(editor.create).toHaveBeenCalledWith(
      expect.any(HTMLElement),
      expect.objectContaining({ language: "python" }),
    );

    dispose();
  });

  // ────── Theme variants (lines 100-101) ─────────────────────

  it("uses claudinio-sepia theme when theme() returns sepia", async () => {
    // Since the module is already mocked, updating the mockImplementation
    // overrides it for this test.
    const themeModule = await import("../lib/theme");
    vi.mocked(themeModule.theme).mockReturnValue("sepia" as any);

    const { dispose } = await renderModal();

    const { editor } = await import("monaco-editor");
    expect(editor.create).toHaveBeenCalledWith(
      expect.any(HTMLElement),
      expect.objectContaining({ theme: "claudinio-sepia" }),
    );

    dispose();
  });

  it("uses claudinio-light theme when theme() returns light", async () => {
    const themeModule = await import("../lib/theme");
    vi.mocked(themeModule.theme).mockReturnValue("light" as any);

    const { dispose } = await renderModal();

    const { editor } = await import("monaco-editor");
    expect(editor.create).toHaveBeenCalledWith(
      expect.any(HTMLElement),
      expect.objectContaining({ theme: "claudinio-light" }),
    );

    dispose();
  });

  // ────── Cleanup ────────────────────────────────────────────

  it("disposes editor and removes keyboard listener on unmount", async () => {
    const { onClose, dispose } = await renderModal();

    // Listener is active during lifetime
    document.dispatchEvent(new KeyboardEvent("keydown", { key: "Escape" }));
    expect(onClose).toHaveBeenCalledTimes(1);

    // Unmount
    dispose();

    // Editor should be disposed
    expect(mockEditor.dispose).toHaveBeenCalledTimes(1);

    // Event listener should be removed — Escape should not trigger onClose again
    document.dispatchEvent(new KeyboardEvent("keydown", { key: "Escape" }));
    expect(onClose).toHaveBeenCalledTimes(1);
  });
});
