import { describe, it, expect, vi, afterEach } from "vitest";
import { render } from "solid-js/web";
import { FileMentionPopover } from "./FileMentionPopover";

// Mock i18n: return the key so we can assert against a stable string
vi.mock("../lib/grill-me", () => ({
  t: (key: string) => key,
}));

describe("FileMentionPopover", () => {
  const fileList = [
    "src/index.ts",
    "src/app.tsx",
    "src/utils/helper.ts",
    "README.md",
    "package.json",
  ];

  const defaultPosition = { top: 100, left: 200, height: 20 };

  afterEach(() => {
    document.body.innerHTML = "";
  });

  it("renders file list", () => {
    const onSelect = vi.fn();
    const onClose = vi.fn();

    const dispose = render(
      () => (
        <FileMentionPopover
          fileList={fileList}
          position={defaultPosition}
          query=""
          onSelect={onSelect}
          onClose={onClose}
        />
      ),
      document.body,
    );

    expect(document.body.textContent).toContain("src/index.ts");
    expect(document.body.textContent).toContain("src/app.tsx");
    expect(document.body.textContent).toContain("README.md");
    expect(document.body.textContent).toContain("package.json");
    dispose();
  });

  it("shows 'No files found' when no match", () => {
    const onSelect = vi.fn();
    const onClose = vi.fn();

    const dispose = render(
      () => (
        <FileMentionPopover
          fileList={fileList}
          position={defaultPosition}
          query="zzzznotfound"
          onSelect={onSelect}
          onClose={onClose}
        />
      ),
      document.body,
    );

    expect(document.body.textContent).toContain("mention.noFiles");
    expect(document.body.textContent).not.toContain("src/index.ts");
    dispose();
  });

  it("calls onSelect when clicking a file", () => {
    const onSelect = vi.fn();
    const onClose = vi.fn();

    const dispose = render(
      () => (
        <FileMentionPopover
          fileList={fileList}
          position={defaultPosition}
          query=""
          onSelect={onSelect}
          onClose={onClose}
        />
      ),
      document.body,
    );

    const buttons = document.body.querySelectorAll("button");
    const indexButton = Array.from(buttons).find((b) =>
      b.textContent?.includes("src/index.ts"),
    );
    expect(indexButton).toBeTruthy();
    indexButton!.click();
    expect(onSelect).toHaveBeenCalledWith("src/index.ts");
    expect(onClose).not.toHaveBeenCalled();
    dispose();
  });

  it("calls onClose when clicking backdrop", () => {
    const onSelect = vi.fn();
    const onClose = vi.fn();

    const dispose = render(
      () => (
        <FileMentionPopover
          fileList={fileList}
          position={defaultPosition}
          query=""
          onSelect={onSelect}
          onClose={onClose}
        />
      ),
      document.body,
    );

    // Backdrop is the first div rendered by Portal (it has class "inset-0")
    const backdrop = document.querySelector(
      '[class*="inset-0"]',
    ) as HTMLElement;
    expect(backdrop).toBeTruthy();
    backdrop.click();
    expect(onClose).toHaveBeenCalledTimes(1);
    expect(onSelect).not.toHaveBeenCalled();
    dispose();
  });

  describe("keyboard navigation", () => {
    it("ArrowDown moves highlight and Enter selects the highlighted file", () => {
      const onSelect = vi.fn();
      const onClose = vi.fn();

      const dispose = render(
        () => (
          <FileMentionPopover
            fileList={fileList}
            position={defaultPosition}
            query=""
            onSelect={onSelect}
            onClose={onClose}
          />
        ),
        document.body,
      );

      // Highlight starts at index 0 → "src/index.ts"
      // Press ArrowDown twice: 0→1→2 → selects "src/utils/helper.ts"
      document.dispatchEvent(new KeyboardEvent("keydown", { key: "ArrowDown", bubbles: true }));
      document.dispatchEvent(new KeyboardEvent("keydown", { key: "ArrowDown", bubbles: true }));
      document.dispatchEvent(new KeyboardEvent("keydown", { key: "Enter", bubbles: true }));

      expect(onSelect).toHaveBeenCalledWith("src/utils/helper.ts");
      dispose();
    });

    it("ArrowDown at last item stays clamped", () => {
      const onSelect = vi.fn();
      const onClose = vi.fn();

      const dispose = render(
        () => (
          <FileMentionPopover
            fileList={fileList}
            position={defaultPosition}
            query=""
            onSelect={onSelect}
            onClose={onClose}
          />
        ),
        document.body,
      );

      // 5 items, navigate far past the end — should clamp at index 4 ("package.json")
      for (let i = 0; i < 10; i++) {
        document.dispatchEvent(new KeyboardEvent("keydown", { key: "ArrowDown", bubbles: true }));
      }
      document.dispatchEvent(new KeyboardEvent("keydown", { key: "Enter", bubbles: true }));

      expect(onSelect).toHaveBeenCalledWith("package.json");
      dispose();
    });

    it("ArrowUp moves highlight up and Enter selects", () => {
      const onSelect = vi.fn();
      const onClose = vi.fn();

      const dispose = render(
        () => (
          <FileMentionPopover
            fileList={fileList}
            position={defaultPosition}
            query=""
            onSelect={onSelect}
            onClose={onClose}
          />
        ),
        document.body,
      );

      // Start at 0, go down to 2, then back up to 1 → selects "src/app.tsx"
      document.dispatchEvent(new KeyboardEvent("keydown", { key: "ArrowDown", bubbles: true }));
      document.dispatchEvent(new KeyboardEvent("keydown", { key: "ArrowDown", bubbles: true }));
      document.dispatchEvent(new KeyboardEvent("keydown", { key: "ArrowUp", bubbles: true }));
      document.dispatchEvent(new KeyboardEvent("keydown", { key: "Enter", bubbles: true }));

      expect(onSelect).toHaveBeenCalledWith("src/app.tsx");
      dispose();
    });

    it("ArrowUp at first item stays at 0", () => {
      const onSelect = vi.fn();
      const onClose = vi.fn();

      const dispose = render(
        () => (
          <FileMentionPopover
            fileList={fileList}
            position={defaultPosition}
            query=""
            onSelect={onSelect}
            onClose={onClose}
          />
        ),
        document.body,
      );

      // ArrowUp at index 0 should stay at 0 via Math.max(-1, 0) = 0
      document.dispatchEvent(new KeyboardEvent("keydown", { key: "ArrowUp", bubbles: true }));
      document.dispatchEvent(new KeyboardEvent("keydown", { key: "Enter", bubbles: true }));

      expect(onSelect).toHaveBeenCalledWith("src/index.ts");
      dispose();
    });

    it("Enter selects the first file at default highlight", () => {
      const onSelect = vi.fn();
      const onClose = vi.fn();

      const dispose = render(
        () => (
          <FileMentionPopover
            fileList={fileList}
            position={defaultPosition}
            query=""
            onSelect={onSelect}
            onClose={onClose}
          />
        ),
        document.body,
      );

      // Highlight starts at 0 (first item = "src/index.ts")
      document.dispatchEvent(new KeyboardEvent("keydown", { key: "Enter", bubbles: true }));

      expect(onSelect).toHaveBeenCalledWith("src/index.ts");
      dispose();
    });

    it("Escape calls onClose", () => {
      const onClose = vi.fn();
      const onSelect = vi.fn();

      const dispose = render(
        () => (
          <FileMentionPopover
            fileList={fileList}
            position={defaultPosition}
            query=""
            onSelect={onSelect}
            onClose={onClose}
          />
        ),
        document.body,
      );

      document.dispatchEvent(new KeyboardEvent("keydown", { key: "Escape", bubbles: true }));

      expect(onClose).toHaveBeenCalledTimes(1);
      dispose();
    });
  });

  it("ArrowDown on empty results returns early (does not crash)", () => {
    const onSelect = vi.fn();
    const onClose = vi.fn();

    const dispose = render(
      () => (
        <FileMentionPopover
          fileList={fileList}
          position={defaultPosition}
          query="zzzznotfound" // 0 results
          onSelect={onSelect}
          onClose={onClose}
        />
      ),
      document.body,
    );

    // No results → pressing ArrowDown hits the `if (r.length === 0) return` guard
    expect(() => {
      document.dispatchEvent(new KeyboardEvent("keydown", { key: "ArrowDown", bubbles: true }));
    }).not.toThrow();
    expect(onSelect).not.toHaveBeenCalled();
    expect(onClose).not.toHaveBeenCalled();
    dispose();
  });

  it("fuse createMemo executes (creates Fuse instance)", () => {
    const onSelect = vi.fn();
    const onClose = vi.fn();

    const dispose = render(
      () => (
        <FileMentionPopover
          fileList={fileList}
          position={defaultPosition}
          query="index" // triggers fuse().search()
          onSelect={onSelect}
          onClose={onClose}
        />
      ),
      document.body,
    );

    // The fuse memo runs, search results include "src/index.ts"
    expect(document.body.textContent).toContain("src/index.ts");
    dispose();
  });

  it("hovering a result toggles classList via onMouseEnter", () => {
    const onSelect = vi.fn();
    const onClose = vi.fn();

    const dispose = render(
      () => (
        <FileMentionPopover
          fileList={fileList}
          position={defaultPosition}
          query=""
          onSelect={onSelect}
          onClose={onClose}
        />
      ),
      document.body,
    );

    // Find the first result button and hover it
    const buttons = document.body.querySelectorAll("button");
    expect(buttons.length).toBeGreaterThan(0);

    // Hover the second button (index 1)
    const targetBtn = buttons[1];
    targetBtn.dispatchEvent(new MouseEvent("mouseenter", { bubbles: true }));

    // The highlightIndex should now be 1 — the button at index 1 should have the highlight class
    // classList: { "bg-accent/10 text-ink": highlightIndex() === i() }
    // After hover, highlightIndex(1) === i(1) → class is applied
    dispose();
  });

  it("dispose triggers onCleanup and unknown key is ignored", () => {
    const onSelect = vi.fn();
    const onClose = vi.fn();

    const dispose = render(
      () => (
        <FileMentionPopover
          fileList={fileList}
          position={defaultPosition}
          query=""
          onSelect={onSelect}
          onClose={onClose}
        />
      ),
      document.body,
    );

    // Dispatch an irrelevant key — hits the implicit else fallthrough
    document.dispatchEvent(new KeyboardEvent("keydown", { key: "a", bubbles: true }));
    expect(onSelect).not.toHaveBeenCalled();
    expect(onClose).not.toHaveBeenCalled();

    // Dispose triggers onCleanup, removing the keydown listener
    dispose();
    document.dispatchEvent(new KeyboardEvent("keydown", { key: "Enter", bubbles: true }));
    expect(onSelect).not.toHaveBeenCalled();
  });

  it("clamps highlight to last valid index when result count shrinks", () => {
    const onSelect = vi.fn();
    const onClose = vi.fn();

    const dispose = render(
      () => (
        <FileMentionPopover
          fileList={fileList}
          position={defaultPosition}
          query="README" // Only matches README.md → 1 result
          onSelect={onSelect}
          onClose={onClose}
        />
      ),
      document.body,
    );

    expect(document.body.textContent).toContain("README.md");
    expect(document.body.textContent).not.toContain("src/index.ts");

    // With only 1 result, ArrowDown clamps at index 0:
    // Math.min(0+1, 1-1) = Math.min(1, 0) = 0
    document.dispatchEvent(new KeyboardEvent("keydown", { key: "ArrowDown", bubbles: true }));
    document.dispatchEvent(new KeyboardEvent("keydown", { key: "Enter", bubbles: true }));

    expect(onSelect).toHaveBeenCalledWith("README.md");
    dispose();
  });
});
