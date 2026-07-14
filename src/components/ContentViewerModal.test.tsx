import { describe, it, expect, vi, afterEach } from "vitest";
import { render } from "solid-js/web";

// ── Hoisted mocks ────────────────────────────────────────────

const mockEditorInstance = vi.hoisted(() => ({
  dispose: vi.fn(),
}));

// ── Module mocks ─────────────────────────────────────────────

vi.mock("monaco-editor", () => ({
  editor: {
    create: vi.fn(() => mockEditorInstance),
    defineTheme: vi.fn(),
  },
}));

vi.mock("@tauri-apps/api/core", () => ({
  convertFileSrc: vi.fn((path: string) => `tauri://localhost${path}`),
}));

vi.mock("../lib/theme", () => ({ theme: vi.fn(() => "dark") }));

vi.mock("./Icon", () => ({ Icon: () => null }));
vi.mock("../lib/grill-me", () => ({ t: (k: string) => k }));
vi.mock("../lib/monacoThemes", () => ({
  defineMonacoThemes: vi.fn(),
  getMonacoTheme: vi.fn((t: string) => {
    if (t.startsWith("claudinio-")) return t;
    if (t === "claudinio") return "claudinio-dark";
    return `claudinio-${t}`;
  }),
}));
vi.mock("./FileEditorModal", () => ({
  detectLanguage: vi.fn(() => "typescript"),
}));

// Mock ipc BEFORE importing the component
vi.mock("../lib/ipc", () => ({
  readFile: vi.fn(),
  openExternal: vi.fn(),
}));

// ── Imports (vi.mock is hoisted above these) ─────────────────

import ContentViewerModal from "./ContentViewerModal";
import { readFile, openExternal } from "../lib/ipc";
import { convertFileSrc } from "@tauri-apps/api/core";
import { theme } from "../lib/theme";
import * as monaco from "monaco-editor";

// ── Helpers ───────────────────────────────────────────────────

/** Flush pending microtasks so Solid reactivity settles. */
function flush() {
  return new Promise<void>((r) => setTimeout(r, 10));
}

// ── Tests ─────────────────────────────────────────────────────

describe("ContentViewerModal", () => {
  afterEach(() => {
    document.body.innerHTML = "";
    vi.clearAllMocks();
  });

  // ────── Image content type ──────────────────────────────────

  it("renders an img with convertFileSrc URL for image contentType", async () => {
    const onClose = vi.fn();
    render(
      () => (
        <ContentViewerModal
          contentType="image"
          filePath="/images/photo.png"
          title="Photo"
          workspace="/ws"
          onClose={onClose}
        />
      ),
      document.body,
    );
    await flush();

    const img = document.body.querySelector("img");
    expect(img).not.toBeNull();
    expect(img!.getAttribute("src")).toBe("tauri://localhost/images/photo.png");
    expect(img!.getAttribute("alt")).toBe("Photo");
    expect(img!.getAttribute("class")).toContain("object-contain");
  });

  it("shows open externally button for image contentType", async () => {
    const onClose = vi.fn();
    render(
      () => (
        <ContentViewerModal
          contentType="image"
          filePath="./relative.png"
          title="Pic"
          workspace="/project"
          onClose={onClose}
        />
      ),
      document.body,
    );
    await flush();

    // Icon is mocked to null so button text is just the t() string
    const buttons = Array.from(document.body.querySelectorAll("button"));
    const externalBtn = buttons.find((b) =>
      b.textContent?.includes("contentViewer.openExternally"),
    );
    expect(externalBtn).not.toBeUndefined();

    // Click it
    externalBtn!.click();
    // The resolved path should be "/project/relative.png"
    expect(openExternal).toHaveBeenCalledWith("/project/relative.png");
  });

  // ────── Text content type: loading state ────────────────────

  it("shows loading indicator initially for text contentType", async () => {
    // readFile will not resolve until we control it
    vi.mocked(readFile).mockReturnValue(new Promise(() => {})); // never resolves
    const onClose = vi.fn();

    render(
      () => (
        <ContentViewerModal
          contentType="text"
          filePath="/docs/readme.md"
          title="Readme"
          workspace="/ws"
          onClose={onClose}
        />
      ),
      document.body,
    );
    await flush();

    // Icon is mocked to null so the "loader" SVG is not rendered.
    // The loading text is rendered by the Show block.
    expect(document.body.textContent).toContain("contentViewer.loading");
  });

  // ────── Text content type: editor after load ────────────────

  it("creates Monaco editor after readFile resolves", async () => {
    vi.mocked(readFile).mockResolvedValue("console.log('hello');\n");
    const onClose = vi.fn();

    render(
      () => (
        <ContentViewerModal
          contentType="text"
          filePath="/project/main.ts"
          title="Main"
          workspace="/ws"
          onClose={onClose}
        />
      ),
      document.body,
    );
    await flush();

    // Loading should be gone
    const spinner = document.body.querySelector(".animate-spin");
    expect(spinner).toBeNull();

    expect(monaco.editor.create).toHaveBeenCalledTimes(1);
    expect(monaco.editor.create).toHaveBeenCalledWith(
      expect.any(HTMLElement),
      expect.objectContaining({
        value: "console.log('hello');\n",
        language: "typescript",
        readOnly: true,
        wordWrap: "on",
        minimap: { enabled: false },
        lineNumbers: "on",
      }),
    );
  });

  // ────── Text content type: error state ──────────────────────

  it("shows error message when readFile fails", async () => {
    vi.mocked(readFile).mockRejectedValue(new Error("permission denied"));
    const onClose = vi.fn();

    render(
      () => (
        <ContentViewerModal
          contentType="text"
          filePath="/secret/notes.txt"
          title="Notes"
          workspace="/ws"
          onClose={onClose}
        />
      ),
      document.body,
    );
    await flush();

    // Loading should be gone
    const spinner = document.body.querySelector(".animate-spin");
    expect(spinner).toBeNull();

    // Error indicator should be shown
    expect(document.body.textContent).toContain("contentViewer.error");

    // The error text should use the error styling
    const errorText = document.body.querySelector(".text-red-400");
    expect(errorText).not.toBeNull();

    // No editor should have been created
    expect(monaco.editor.create).not.toHaveBeenCalled();
  });

  // ────── Video content type ──────────────────────────────────

  it("renders a video element for video contentType", async () => {
    const onClose = vi.fn();
    render(
      () => (
        <ContentViewerModal
          contentType="video"
          filePath="/media/clip.mp4"
          title="Clip"
          workspace="/ws"
          onClose={onClose}
        />
      ),
      document.body,
    );
    await flush();

    const video = document.body.querySelector("video");
    expect(video).not.toBeNull();
    expect(video!.getAttribute("controls")).not.toBeNull();

    const source = video!.querySelector("source");
    expect(source).not.toBeNull();
    expect(source!.getAttribute("src")).toBe("tauri://localhost/media/clip.mp4");

    // convertFileSrc was called with the resolved path
    expect(convertFileSrc).toHaveBeenCalledWith("/media/clip.mp4");
  });

  it("resolves relative file path for video src", async () => {
    const onClose = vi.fn();
    render(
      () => (
        <ContentViewerModal
          contentType="video"
          filePath="./demo.mp4"
          title="Demo"
          workspace="/home/user"
          onClose={onClose}
        />
      ),
      document.body,
    );
    await flush();

    const source = document.body.querySelector("video source");
    expect(source!.getAttribute("src")).toBe(
      "tauri://localhost/home/user/demo.mp4",
    );
  });

  // ────── Audio content type ──────────────────────────────────

  it("renders an audio element for audio contentType", async () => {
    const onClose = vi.fn();
    render(
      () => (
        <ContentViewerModal
          contentType="audio"
          filePath="/music/song.mp3"
          title="Song"
          workspace="/ws"
          onClose={onClose}
        />
      ),
      document.body,
    );
    await flush();

    const audio = document.body.querySelector("audio");
    expect(audio).not.toBeNull();
    expect(audio!.getAttribute("controls")).not.toBeNull();

    const source = audio!.querySelector("source");
    expect(source).not.toBeNull();
    expect(source!.getAttribute("src")).toBe("tauri://localhost/music/song.mp3");
  });

  // ────── Close button (X) ────────────────────────────────────

  it("calls onClose when the X (close) button is clicked", async () => {
    const onClose = vi.fn();
    render(
      () => (
        <ContentViewerModal
          contentType="text"
          filePath="/f.ts"
          title="File"
          workspace="/ws"
          onClose={onClose}
        />
      ),
      document.body,
    );
    await flush();

    const closeBtn = document.body.querySelector(
      'button[title="contentViewer.close"]',
    ) as HTMLElement | null;
    expect(closeBtn).not.toBeNull();

    closeBtn!.click();
    expect(onClose).toHaveBeenCalledTimes(1);
  });

  // ────── Backdrop click ──────────────────────────────────────

  it("calls onClose when the backdrop overlay is clicked", async () => {
    const onClose = vi.fn();
    render(
      () => (
        <ContentViewerModal
          contentType="text"
          filePath="/f.ts"
          title="File"
          workspace="/ws"
          onClose={onClose}
        />
      ),
      document.body,
    );
    await flush();

    // The outermost div (fixed inset-0) is the backdrop
    const backdrop = document.body.firstElementChild as HTMLElement;
    expect(backdrop).not.toBeNull();

    // Click the backdrop itself (not a child)
    backdrop.click();

    expect(onClose).toHaveBeenCalledTimes(1);
  });

  it("does NOT call onClose when clicking inside the modal content", async () => {
    const onClose = vi.fn();
    render(
      () => (
        <ContentViewerModal
          contentType="image"
          filePath="/f.png"
          title="Image"
          workspace="/ws"
          onClose={onClose}
        />
      ),
      document.body,
    );
    await flush();

    // The inner modal has class "rounded-xl"
    const innerModal = document.body.querySelector(
      ".rounded-xl",
    ) as HTMLElement;
    expect(innerModal).not.toBeNull();

    innerModal.click();

    expect(onClose).not.toHaveBeenCalled();
  });

  // ────── Escape key ──────────────────────────────────────────

  it("calls onClose when Escape key is pressed", async () => {
    const onClose = vi.fn();
    render(
      () => (
        <ContentViewerModal
          contentType="text"
          filePath="/f.ts"
          title="File"
          workspace="/ws"
          onClose={onClose}
        />
      ),
      document.body,
    );
    await flush();

    // SolidJS uses event delegation — keydown is captured at document level
    document.dispatchEvent(new KeyboardEvent("keydown", { key: "Escape" }));

    expect(onClose).toHaveBeenCalledTimes(1);
  });

  it("does NOT call onClose for non-Escape keys", async () => {
    const onClose = vi.fn();
    render(
      () => (
        <ContentViewerModal
          contentType="text"
          filePath="/f.ts"
          title="File"
          workspace="/ws"
          onClose={onClose}
        />
      ),
      document.body,
    );
    await flush();

    document.dispatchEvent(new KeyboardEvent("keydown", { key: "Enter" }));
    document.dispatchEvent(new KeyboardEvent("keydown", { key: "Tab" }));

    expect(onClose).not.toHaveBeenCalled();
  });

  // ────── Title rendering ─────────────────────────────────────

  it("renders the title in the header", async () => {
    const onClose = vi.fn();
    render(
      () => (
        <ContentViewerModal
          contentType="text"
          filePath="/f.ts"
          title="My Special File.ts"
          workspace="/ws"
          onClose={onClose}
        />
      ),
      document.body,
    );
    await flush();

    expect(document.body.textContent).toContain("My Special File.ts");
  });

  // ────── file:// prefix stripping ────────────────────────────

  it("strips file:// prefix when resolving path", async () => {
    const onClose = vi.fn();
    render(
      () => (
        <ContentViewerModal
          contentType="image"
          filePath="file:///absolute/path/banner.jpg"
          title="Banner"
          workspace="/ws"
          onClose={onClose}
        />
      ),
      document.body,
    );
    await flush();

    const img = document.body.querySelector("img")!;
    expect(img.getAttribute("src")).toBe(
      "tauri://localhost/absolute/path/banner.jpg",
    );
  });

  // ────── Editor cleanup (line 69) ────────────────────────────

  it("disposes editor when component unmounts after text load", async () => {
    vi.mocked(readFile).mockResolvedValue("some code");
    const onClose = vi.fn();

    const dispose = render(
      () => (
        <ContentViewerModal
          contentType="text"
          filePath="/f.ts"
          title="File"
          workspace="/ws"
          onClose={onClose}
        />
      ),
      document.body,
    );
    await flush();

    // Editor was created
    expect(monaco.editor.create).toHaveBeenCalledTimes(1);

    // Unmount the component — onCleanup runs editor?.dispose()
    dispose();

    expect(mockEditorInstance.dispose).toHaveBeenCalled();
  });

  // ────── Theme branches in the createEffect ──────────────────

  it("creates editor with sepia monaco theme when theme is sepia", async () => {
    vi.mocked(readFile).mockResolvedValue("code");
    vi.mocked(theme).mockReturnValue("sepia");

    const onClose = vi.fn();
    render(
      () => (
        <ContentViewerModal
          contentType="text"
          filePath="/f.ts"
          title="File"
          workspace="/ws"
          onClose={onClose}
        />
      ),
      document.body,
    );
    await flush();

    expect(monaco.editor.create).toHaveBeenCalledWith(
      expect.any(HTMLElement),
      expect.objectContaining({ theme: "claudinio-sepia" }),
    );
  });

  it("creates editor with light monaco theme when theme is light", async () => {
    vi.mocked(readFile).mockResolvedValue("code");
    vi.mocked(theme).mockReturnValue("light");

    const onClose = vi.fn();
    render(
      () => (
        <ContentViewerModal
          contentType="text"
          filePath="/f.ts"
          title="File"
          workspace="/ws"
          onClose={onClose}
        />
      ),
      document.body,
    );
    await flush();

    expect(monaco.editor.create).toHaveBeenCalledWith(
      expect.any(HTMLElement),
      expect.objectContaining({ theme: "claudinio-light" }),
    );
  });

  // ────── Unmount before readFile resolves (mounted = false guard) ──

  it("does not set content or create editor when unmounted before readFile resolves", async () => {
    // readFile doesn't resolve immediately — hold it open
    let resolveReadFile!: (v: string) => void;
    vi.mocked(readFile).mockReturnValue(
      new Promise((resolve) => {
        resolveReadFile = resolve;
      }),
    );

    const onClose = vi.fn();
    const dispose = render(
      () => (
        <ContentViewerModal
          contentType="text"
          filePath="/f.ts"
          title="File"
          workspace="/ws"
          onClose={onClose}
        />
      ),
      document.body,
    );
    await flush();

    // Unmount before readFile resolves — mounted becomes false
    dispose();

    // Now resolve — the `if (!mounted) return` guards should fire
    resolveReadFile("should be ignored");
    await flush();

    // Editor should NOT have been created
    expect(monaco.editor.create).not.toHaveBeenCalled();

    // mockEditorInstance.dispose may or may not be called, but editor was
    // never assigned (create was never called), so the optional chain on
    // editor?.dispose() should short-circuit without error.
  });

  it("shows error that respects mounted guard (unmount before reject)", async () => {
    let rejectReadFile!: (e: Error) => void;
    vi.mocked(readFile).mockReturnValue(
      new Promise((_resolve, reject) => {
        rejectReadFile = reject;
      }),
    );

    const onClose = vi.fn();
    const dispose = render(
      () => (
        <ContentViewerModal
          contentType="text"
          filePath="/f.ts"
          title="File"
          workspace="/ws"
          onClose={onClose}
        />
      ),
      document.body,
    );
    await flush();

    dispose();

    rejectReadFile(new Error("too late"));
    await flush();

    // No error content should appear since mounted is false
    expect(monaco.editor.create).not.toHaveBeenCalled();
    expect(document.body.textContent).not.toContain("contentViewer.error");
  });

  // ────── Empty string content (falsy value in Show condition) ──

  it("does not render editor container when readFile returns empty string", async () => {
    vi.mocked(readFile).mockResolvedValue(""); // empty string is falsy
    const onClose = vi.fn();

    render(
      () => (
        <ContentViewerModal
          contentType="text"
          filePath="/empty.txt"
          title="Empty"
          workspace="/ws"
          onClose={onClose}
        />
      ),
      document.body,
    );
    await flush();

    // Loading is finished
    expect(document.body.textContent).not.toContain("contentViewer.loading");
    // No error
    expect(document.body.textContent).not.toContain("contentViewer.error");

    // content() is "" which is falsy, so the Show condition
    // !loading() && !error() && content() evaluates to false
    // → editor container div should NOT be rendered
    // → monaco.editor.create should NOT be called
    const editorContainers = document.body.querySelectorAll('[class*="h-full w-full"]');
    expect(editorContainers.length).toBe(0);
    expect(monaco.editor.create).not.toHaveBeenCalled();
  });

  // ────── Aspect-ratio image (cover vs contain variant) ────────

  it("renders conversion asset:// URL for image via convertFileSrc", async () => {
    const onClose = vi.fn();
    render(
      () => (
        <ContentViewerModal
          contentType="image"
          filePath="/assets/hero.svg"
          title="Hero"
          workspace="/ws"
          onClose={onClose}
        />
      ),
      document.body,
    );
    await flush();

    expect(convertFileSrc).toHaveBeenCalledWith("/assets/hero.svg");
    const img = document.body.querySelector("img")!;
    expect(img.getAttribute("src")).toBe("tauri://localhost/assets/hero.svg");
  });

  it("renders open externally button for image with correctly resolved path from file prefix", async () => {
    const onClose = vi.fn();
    render(
      () => (
        <ContentViewerModal
          contentType="image"
          filePath="file:///photos/wallpaper.jpg"
          title="Wallpaper"
          workspace="/ws"
          onClose={onClose}
        />
      ),
      document.body,
    );
    await flush();

    const buttons = Array.from(document.body.querySelectorAll("button"));
    const externalBtn = buttons.find((b) =>
      b.textContent?.includes("contentViewer.openExternally"),
    );
    expect(externalBtn).not.toBeUndefined();
    externalBtn!.click();
    expect(openExternal).toHaveBeenCalledWith("/photos/wallpaper.jpg");
  });

  // ────── Non-text content type — ensure createEffect short-circuits at contentType check ──

  it("does not attempt to create Monaco editor for non-text contentType", async () => {
    const onClose = vi.fn();
    // Use image (already tested to render img) and verify editor.create was NEVER called
    render(
      () => (
        <ContentViewerModal
          contentType="image"
          filePath="/img.png"
          title="No Editor"
          workspace="/ws"
          onClose={onClose}
        />
      ),
      document.body,
    );
    await flush();

    expect(monaco.editor.create).not.toHaveBeenCalled();
  });

  // ────── Modal closes only on backdrop, not on inner content clicks, for audio type ──

  it("does not close when clicking inside the modal content for audio contentType", async () => {
    const onClose = vi.fn();
    render(
      () => (
        <ContentViewerModal
          contentType="audio"
          filePath="/s.mp3"
          title="Song"
          workspace="/ws"
          onClose={onClose}
        />
      ),
      document.body,
    );
    await flush();

    const innerModal = document.body.querySelector(
      ".rounded-xl",
    ) as HTMLElement;
    expect(innerModal).not.toBeNull();
    innerModal.click();
    expect(onClose).not.toHaveBeenCalled();
  });
});
