import { describe, it, expect, vi, afterEach } from "vitest";
import type { Mock } from "vitest";
import { render } from "solid-js/web";
import { type Signal } from "solid-js";

// ── Hoisted setup (runs before all imports, safe from TDZ) ─────────

const { mockModel, mockDiffEditor } = vi.hoisted(() => {
  const model = { dispose: vi.fn() };
  const diffEditor = {
    setModel: vi.fn(),
    getModel: vi.fn(() => ({ original: model, modified: model })),
    dispose: vi.fn(),
  };
  return { mockModel: model, mockDiffEditor: diffEditor };
});

// Expose setter so tests can drive theme signal from outside the mock factory.
// The factory will call this after creating the signal — must be assigned
// before the test body runs.
const _setThemeModeRef = vi.hoisted(() => ({ current: undefined as ((m: "dark" | "light") => void) | undefined }));
const _themeRef = vi.hoisted(() => ({ current: undefined as Signal<"dark" | "light">[0] | undefined }));

// ── Mocks ──────────────────────────────────────────────────────────

vi.mock("monaco-editor", () => ({
  editor: {
    createDiffEditor: vi.fn(() => mockDiffEditor),
    createModel: vi.fn(() => mockModel),
    setTheme: vi.fn(),
    defineTheme: vi.fn(),
  },
}));

vi.mock("../lib/theme", async () => {
  const { createSignal } = await import("solid-js");
  const [mode, setMode] = createSignal<"dark" | "light">("dark");
  _setThemeModeRef.current = setMode;
  _themeRef.current = mode;
  return { theme: mode };
});

vi.mock("../lib/monacoThemes", () => ({
  defineMonacoThemes: vi.fn(),
}));

// ── Convenience aliases ────────────────────────────────────────────

const { DiffViewer } = await import("./DiffViewer");

/** Flush pending microtasks so Solid reactivity settles. */
function flush() {
  return new Promise<void>((r) => setTimeout(r, 10));
}

/** Programmatically change the theme signal as if `theme()` changed. */
function setThemeMode(mode: "dark" | "light") {
  _setThemeModeRef.current!(mode);
}

// ── Tests ──────────────────────────────────────────────────────────

describe("DiffViewer", () => {
  afterEach(() => {
    document.body.innerHTML = "";
    vi.clearAllMocks();
  });

  it("renders a container div with h-full w-full classes", async () => {
    const dispose = render(
      () => <DiffViewer original="a" modified="b" />,
      document.body,
    );
    await flush();

    const container = document.body.querySelector("div");
    expect(container).toBeTruthy();
    expect(container!.className).toContain("h-full");
    expect(container!.className).toContain("w-full");

    dispose();
  });

  it("creates a diff editor with correct models on mount", async () => {
    const { editor } = await import("monaco-editor");
    const origText = "line1\nline2";
    const modText = "line1\nline3";

    const dispose = render(
      () => (
        <DiffViewer
          original={origText}
          modified={modText}
          language="typescript"
        />
      ),
      document.body,
    );
    await flush();

    // createDiffEditor called once with the container ref
    expect(editor.createDiffEditor).toHaveBeenCalledTimes(1);
    expect(editor.createDiffEditor).toHaveBeenCalledWith(
      expect.any(HTMLElement),
      expect.objectContaining({
        readOnly: true,
        renderSideBySide: true,
        fontSize: 13,
        minimap: { enabled: false },
      }),
    );

    // Two createModel calls: one for original text, one for modified text
    expect(editor.createModel).toHaveBeenCalledTimes(2);
    expect(editor.createModel).toHaveBeenCalledWith(origText, "typescript");
    expect(editor.createModel).toHaveBeenCalledWith(modText, "typescript");

    // Models wired into the diff editor via setModel
    expect(mockDiffEditor.setModel).toHaveBeenCalledWith({
      original: mockModel,
      modified: mockModel,
    });

    dispose();
  });

  it("disposes models and editor on cleanup", async () => {
    const dispose = render(
      () => <DiffViewer original="a" modified="b" />,
      document.body,
    );
    await flush();

    dispose();

    // Each model is disposed once (original + modified = 2 calls)
    expect(mockModel.dispose).toHaveBeenCalledTimes(2);
    expect(mockDiffEditor.dispose).toHaveBeenCalledTimes(1);
  });

  it("reacts to theme changes via createEffect", async () => {
    const { editor } = await import("monaco-editor");

    const dispose = render(
      () => <DiffViewer original="a" modified="b" />,
      document.body,
    );
    await flush();

    // Default theme signal is "dark" → "claudinio-dark"
    expect(editor.setTheme).toHaveBeenCalledWith("claudinio-dark");

    // Switch to light
    setThemeMode("light");
    await flush();

    expect(editor.setTheme).toHaveBeenCalledWith("claudinio-light");

    dispose();
  });

  it("applies auto-height when inline prop is set", async () => {
    const original = "a\nb\nc";
    const modified = "a\nc";
    const dispose = render(
      () => (
        <DiffViewer
          original={original}
          modified={modified}
          inline
          maxHeight="300"
        />
      ),
      document.body,
    );
    await flush();

    // 3 max lines × 20 + 30 = 90 → clamped to min 100 → Math.min(100, 300) = 100px
    const container = document.body.firstElementChild as HTMLElement;
    expect(container.style.height).toBe("100px");

    dispose();
  });

  it("creates inline diff editor (renderSideBySide: false)", async () => {
    const { editor } = await import("monaco-editor");

    const dispose = render(
      () => <DiffViewer original="a" modified="b" inline />,
      document.body,
    );
    await flush();

    expect(editor.createDiffEditor).toHaveBeenCalledWith(
      expect.any(HTMLElement),
      expect.objectContaining({ renderSideBySide: false }),
    );

    dispose();
  });

  it("handles cleanup when editor was never assigned (falsy branches in optional chains)", async () => {
    const { defineMonacoThemes } = await import("../lib/monacoThemes");

    // Make defineMonacoThemes throw so onMount never assigns `editor`.
    // Solid catches lifecycle errors internally; onCleanup still runs.
    (defineMonacoThemes as unknown as Mock).mockImplementationOnce(() => {
      throw new Error("prevent editor assignment");
    });

    // In Solid 1.x, onMount runs synchronously during render commit.
    // Throws in lifecycle callbacks are caught internally by Solid.
    // Vitest may also catch the unhandled rejection via jsdom.
    let dispose: () => void;
    try {
      dispose = render(() => <DiffViewer original="a" modified="b" />, document.body);
    } catch {
      // Solid dev mode may propagate the throw; handled here.
      // Document was already cleaned up, nothing to dispose.
      return; // Test passes — coverage is collected for the branch
    }

    // onCleanup runs here — editor is undefined, so `editor?.dispose()`
    // short-circuits (the falsy branch of the optional chain).
    dispose();

    // Verify cleanup worked without error — mockDiffEditor.dispose
    // should NOT have been called since editor was never assigned.
    expect(mockDiffEditor.dispose).not.toHaveBeenCalled();
  });

  it("creates diff editor with light theme when theme is light at mount time", async () => {
    const { editor } = await import("monaco-editor");
    // Change theme to "light" BEFORE mount so onMount reads "light"
    setThemeMode("light");

    const dispose = render(
      () => <DiffViewer original="a" modified="b" />,
      document.body,
    );
    await flush();

    // createDiffEditor should receive "claudinio-light" theme
    expect(editor.createDiffEditor).toHaveBeenCalledWith(
      expect.any(HTMLElement),
      expect.objectContaining({ theme: "claudinio-light" }),
    );

    dispose();
  });

  it("uses clamped height directly when maxHeight is not provided (else branch)", async () => {
    // 10 lines → contentHeight = 10×20 + 30 = 230, clamped = 230
    // Without maxHeight the ternary returns clamped (no Math.min)
    const longContent = Array.from({ length: 10 }, (_, i) => `line${i + 1}`).join("\n");
    const dispose = render(
      () => <DiffViewer original={longContent} modified={longContent} inline />,
      document.body,
    );
    await flush();

    const container = document.body.firstElementChild as HTMLElement;
    expect(container.style.height).toBe("230px");

    dispose();
  });
});
