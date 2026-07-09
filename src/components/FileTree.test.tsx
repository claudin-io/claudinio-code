import { describe, it, expect, vi, afterEach } from "vitest";
import { render } from "solid-js/web";
import { createSignal } from "solid-js";
import { listDir } from "../lib/ipc";
import type { DirEntry } from "../lib/ipc";
import { FileTree } from "./FileTree";

// ── Mocks ──────────────────────────────────────────────────────────

vi.mock("../lib/ipc", () => ({
  listDir: vi.fn(),
}));

vi.mock("../lib/grill-me", () => ({
  t: vi.fn((key: string) => key),
}));

// ── Fixtures ───────────────────────────────────────────────────────

const rootDir = "/test/workspace";

const fileEntry: DirEntry = {
  name: "main.ts",
  path: "/test/workspace/main.ts",
  isDir: false,
};

const dirEntry: DirEntry = {
  name: "src",
  path: "/test/workspace/src",
  isDir: true,
};

const childEntries: DirEntry[] = [
  { name: "index.ts", path: "/test/workspace/src/index.ts", isDir: false },
  { name: "utils.ts", path: "/test/workspace/src/utils.ts", isDir: false },
];

const rootEntries: DirEntry[] = [dirEntry, fileEntry];

// ── Helpers ────────────────────────────────────────────────────────

/** Flush pending microtasks so that Solid reactivity settles. */
function flush() {
  return new Promise((r) => setTimeout(r, 10));
}

// ── Tests ──────────────────────────────────────────────────────────

describe("FileTree", () => {
  afterEach(() => {
    document.body.innerHTML = "";
    vi.clearAllMocks();
  });

  // ── Rendering ──────────────────────────────────────────────────

  it("renders the root folder name", async () => {
    vi.mocked(listDir).mockResolvedValue([]);

    const dispose = render(
      () => (
        <FileTree
          root={rootDir}
          onOpenFile={vi.fn()}
          onOpenExternal={vi.fn()}
          onDblClickFile={vi.fn()}
          selectedPath={() => null}
        />
      ),
      document.body,
    );
    await flush();

    // The header shows the last segment of root: "workspace"
    expect(document.body.textContent).toContain("workspace");

    dispose();
  });

  it("calls listDir with the root path on mount", async () => {
    vi.mocked(listDir).mockResolvedValue([]);

    const dispose = render(
      () => (
        <FileTree
          root={rootDir}
          onOpenFile={vi.fn()}
          onOpenExternal={vi.fn()}
          onDblClickFile={vi.fn()}
          selectedPath={() => null}
        />
      ),
      document.body,
    );
    await flush();

    expect(listDir).toHaveBeenCalledWith(rootDir, expect.anything());

    dispose();
  });

  // ── Directory expansion ────────────────────────────────────────

  it("expands a directory on click and loads children via listDir", async () => {
    vi.mocked(listDir).mockResolvedValueOnce(rootEntries);
    vi.mocked(listDir).mockResolvedValueOnce(childEntries);

    const dispose = render(
      () => (
        <FileTree
          root={rootDir}
          onOpenFile={vi.fn()}
          onOpenExternal={vi.fn()}
          onDblClickFile={vi.fn()}
          selectedPath={() => null}
        />
      ),
      document.body,
    );
    await flush();

    // Root entries rendered — should see the directory
    expect(document.body.textContent).toContain("src");

    // Click the directory button to expand
    const buttons = document.body.querySelectorAll("button");
    const dirButton = Array.from(buttons).find((b) => b.textContent?.includes("src"));
    expect(dirButton).not.toBeNull();
    dirButton!.click();

    // Flush to let the createResource resolve
    await flush();

    // Children should now appear
    expect(document.body.textContent).toContain("index.ts");
    expect(document.body.textContent).toContain("utils.ts");
    // listDir is called: 1st for root (with info arg), 2nd for src (without info, via arrow fn)
    expect(listDir.mock.calls[1][0]).toBe("/test/workspace/src");

    dispose();
  });

  it("collapses a directory on second click", async () => {
    vi.mocked(listDir).mockResolvedValueOnce(rootEntries);
    vi.mocked(listDir).mockResolvedValueOnce(childEntries);

    const dispose = render(
      () => (
        <FileTree
          root={rootDir}
          onOpenFile={vi.fn()}
          onOpenExternal={vi.fn()}
          onDblClickFile={vi.fn()}
          selectedPath={() => null}
        />
      ),
      document.body,
    );
    await flush();

    // Expand
    const buttons = document.body.querySelectorAll("button");
    const dirButton = Array.from(buttons).find((b) => b.textContent?.includes("src"))!;
    dirButton.click();
    await flush();
    expect(document.body.textContent).toContain("index.ts");

    // Collapse (click again)
    dirButton.click();
    await flush();

    // Children should be hidden
    expect(document.body.textContent).not.toContain("index.ts");

    dispose();
  });

  // ── File click ─────────────────────────────────────────────────

  it("calls onOpenFile when clicking a file", async () => {
    vi.mocked(listDir).mockResolvedValueOnce([fileEntry]);
    const onOpenFile = vi.fn();

    const dispose = render(
      () => (
        <FileTree
          root={rootDir}
          onOpenFile={onOpenFile}
          onOpenExternal={vi.fn()}
          onDblClickFile={vi.fn()}
          selectedPath={() => null}
        />
      ),
      document.body,
    );
    await flush();

    const buttons = document.body.querySelectorAll("button");
    const fileButton = Array.from(buttons).find((b) => b.textContent?.includes("main.ts"))!;
    fileButton.click();

    expect(onOpenFile).toHaveBeenCalledWith("/test/workspace/main.ts");

    dispose();
  });

  // ── Double-click ───────────────────────────────────────────────

  it("calls onDblClickFile on double-clicking a file", async () => {
    vi.mocked(listDir).mockResolvedValueOnce([fileEntry]);
    const onDblClickFile = vi.fn();

    const dispose = render(
      () => (
        <FileTree
          root={rootDir}
          onOpenFile={vi.fn()}
          onOpenExternal={vi.fn()}
          onDblClickFile={onDblClickFile}
          selectedPath={() => null}
        />
      ),
      document.body,
    );
    await flush();

    const buttons = document.body.querySelectorAll("button");
    const fileButton = Array.from(buttons).find((b) => b.textContent?.includes("main.ts"))!;
    fileButton.dispatchEvent(new MouseEvent("dblclick", { bubbles: true }));

    expect(onDblClickFile).toHaveBeenCalledWith("/test/workspace/main.ts");

    dispose();
  });

  it("does not call onDblClickFile on double-clicking a directory", async () => {
    vi.mocked(listDir).mockResolvedValueOnce([dirEntry]);
    const onDblClickFile = vi.fn();

    const dispose = render(
      () => (
        <FileTree
          root={rootDir}
          onOpenFile={vi.fn()}
          onOpenExternal={vi.fn()}
          onDblClickFile={onDblClickFile}
          selectedPath={() => null}
        />
      ),
      document.body,
    );
    await flush();

    const buttons = document.body.querySelectorAll("button");
    const dirButton = Array.from(buttons).find((b) => b.textContent?.includes("src"))!;
    dirButton.dispatchEvent(new MouseEvent("dblclick", { bubbles: true }));

    expect(onDblClickFile).not.toHaveBeenCalled();

    dispose();
  });

  // ── Child file click inside expanded directory ─────────────────

  it("calls onOpenFile when clicking a child file entry in an expanded directory", async () => {
    vi.mocked(listDir).mockResolvedValueOnce(rootEntries);
    vi.mocked(listDir).mockResolvedValueOnce(childEntries);
    const onOpenFile = vi.fn();

    const dispose = render(
      () => (
        <FileTree
          root={rootDir}
          onOpenFile={onOpenFile}
          onOpenExternal={vi.fn()}
          onDblClickFile={vi.fn()}
          selectedPath={() => null}
        />
      ),
      document.body,
    );
    await flush();

    // Expand the src directory
    const buttons = document.body.querySelectorAll("button");
    const dirButton = Array.from(buttons).find((b) => b.textContent?.includes("src"))!;
    dirButton.click();
    await flush();

    // All child entries rendered — find and click index.ts
    const allButtons = document.body.querySelectorAll("button");
    const childButton = Array.from(allButtons).find((b) => b.textContent?.includes("index.ts"))!;
    childButton.click();

    expect(onOpenFile).toHaveBeenCalledWith("/test/workspace/src/index.ts");

    dispose();
  });

  it("calls onDblClickFile when double-clicking a child file in an expanded directory", async () => {
    vi.mocked(listDir).mockResolvedValueOnce(rootEntries);
    vi.mocked(listDir).mockResolvedValueOnce(childEntries);
    const onDblClickFile = vi.fn();

    const dispose = render(
      () => (
        <FileTree
          root={rootDir}
          onOpenFile={vi.fn()}
          onOpenExternal={vi.fn()}
          onDblClickFile={onDblClickFile}
          selectedPath={() => null}
        />
      ),
      document.body,
    );
    await flush();

    // Expand the src directory
    const buttons = document.body.querySelectorAll("button");
    const dirButton = Array.from(buttons).find((b) => b.textContent?.includes("src"))!;
    dirButton.click();
    await flush();

    // Double-click child file entry
    const allButtons = document.body.querySelectorAll("button");
    const childButton = Array.from(allButtons).find((b) => b.textContent?.includes("utils.ts"))!;
    childButton.dispatchEvent(new MouseEvent("dblclick", { bubbles: true }));

    expect(onDblClickFile).toHaveBeenCalledWith("/test/workspace/src/utils.ts");

    dispose();
  });

  // ── Selected path highlighting ─────────────────────────────────

  it("applies selected class to the matching entry", async () => {
    vi.mocked(listDir).mockResolvedValueOnce([fileEntry]);

    const dispose = render(
      () => (
        <FileTree
          root={rootDir}
          onOpenFile={vi.fn()}
          onOpenExternal={vi.fn()}
          onDblClickFile={vi.fn()}
          selectedPath={() => "/test/workspace/main.ts"}
        />
      ),
      document.body,
    );
    await flush();

    const buttons = document.body.querySelectorAll("button");
    const fileButton = Array.from(buttons).find((b) => b.textContent?.includes("main.ts"))!;
    expect(fileButton.className).toContain("text-accent");

    dispose();
  });

  it("does not highlight non-matching entries", async () => {
    vi.mocked(listDir).mockResolvedValueOnce([fileEntry, dirEntry]);

    const dispose = render(
      () => (
        <FileTree
          root={rootDir}
          onOpenFile={vi.fn()}
          onOpenExternal={vi.fn()}
          onDblClickFile={vi.fn()}
          selectedPath={() => "/test/workspace/src"}
        />
      ),
      document.body,
    );
    await flush();

    const buttons = document.body.querySelectorAll("button");
    const fileButton = Array.from(buttons).find((b) => b.textContent?.includes("main.ts"))!;
    expect(fileButton.className).not.toContain("text-accent");

    dispose();
  });

  it("removes highlight when selectedPath changes away", async () => {
    const [selectedPath, setSelectedPath] = createSignal<string | null>("/test/workspace/main.ts");

    vi.mocked(listDir).mockResolvedValueOnce([fileEntry, dirEntry]);

    const dispose = render(
      () => (
        <FileTree
          root={rootDir}
          onOpenFile={vi.fn()}
          onOpenExternal={vi.fn()}
          onDblClickFile={vi.fn()}
          selectedPath={selectedPath}
        />
      ),
      document.body,
    );
    await flush();

    const buttons = () => document.body.querySelectorAll("button");
    const fileButton = () =>
      Array.from(buttons()).find((b) => b.textContent?.includes("main.ts"))!;

    // Initially highlighted
    expect(fileButton().className).toContain("text-accent");

    // Change selectedPath
    setSelectedPath("/test/workspace/src");
    await flush();

    // Highlight should be removed from file
    expect(fileButton().className).not.toContain("text-accent");

    dispose();
  });
});
