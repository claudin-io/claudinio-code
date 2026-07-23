import { describe, it, expect, vi, afterEach, beforeEach } from "vitest";
import { render } from "solid-js/web";

// ── Mocks ──────────────────────────────────────────────────────────
vi.mock("../lib/ipc", () => ({
  gitStatus: vi.fn(),
  gitFileDiff: vi.fn(),
}));


vi.mock("./Icon", () => ({
  Icon: (props: { name: string; class?: string }) => (
    <span data-testid={`icon-${props.name}`} class={props.class} />
  ),
}));

// ── Imports (after mocks) ──────────────────────────────────────────
import { GitChangesModal } from "./GitChangesModal";
import { gitStatus, gitFileDiff } from "../lib/ipc";
import type { Mock } from "vitest";
import type { GitStatus } from "../lib/ipc";

// ── Helpers ─────────────────────────────────────────────────────────
function flush() {
  return new Promise((r) => setTimeout(r, 10));
}

function emptyStatus(): GitStatus {
  return { hasChanges: false, files: [], totalAdditions: 0, totalDeletions: 0 };
}

function statusWithFiles(): GitStatus {
  return {
    hasChanges: true,
    files: [
      { path: "src/main.ts", status: "M", additions: 3, deletions: 1 },
      { path: "src/utils.ts", status: "A", additions: 10, deletions: 0 },
      { path: "src/old.ts", status: "D", additions: 0, deletions: 5 },
    ],
    totalAdditions: 13,
    totalDeletions: 6,
  };
}

const SAMPLE_DIFF = [
  "@@ -1,3 +1,3 @@",
  " hello",
  "-world",
  "+world2",
  "",
  "@@ -10,5 +10,8 @@",
  " context",
  " line",
  "+added line",
  "+another one",
].join("\n");

// ══════════════════════════════════════════════════════════════════════
// GitChangesModal tests
// ══════════════════════════════════════════════════════════════════════

describe("GitChangesModal", () => {
  beforeEach(() => {
    // Default mocks: no changes, empty diff
    (gitStatus as Mock).mockResolvedValue(emptyStatus());
    (gitFileDiff as Mock).mockResolvedValue("");
  });

  afterEach(() => {
    document.body.innerHTML = "";
    vi.clearAllMocks();
  });

  // ── open / close ──────────────────────────────────────────────────

  it("renders nothing when open=false", () => {
    const dispose = render(
      () => (
        <GitChangesModal
          workspace="/test"
          open={false}
          onClose={vi.fn()}
          onCommitPush={vi.fn()}
        />
      ),
      document.body,
    );
    // Modal content should not be in the DOM
    expect(document.body.textContent).toBe("");
    expect(document.body.querySelector('[class*="fixed"]')).toBeNull();
    dispose();
  });

  it("shows loading state initially when open=true", async () => {
    // Keep the promise pending so loading stays true
    (gitStatus as Mock).mockImplementation(
      () => new Promise(() => {}), // never resolves
    );

    const dispose = render(
      () => (
        <GitChangesModal
          workspace="/test"
          open
          onClose={vi.fn()}
          onCommitPush={vi.fn()}
        />
      ),
      document.body,
    );

    // After synchronous mount, loading state should be visible
    expect(document.body.textContent).toContain("Loading...");

    dispose();
  });

  it("shows file list after loading completes", async () => {
    (gitStatus as Mock).mockResolvedValue(statusWithFiles());

    const dispose = render(
      () => (
        <GitChangesModal
          workspace="/test"
          open
          onClose={vi.fn()}
          onCommitPush={vi.fn()}
        />
      ),
      document.body,
    );

    await flush();

    // File paths should be visible
    expect(document.body.textContent).toContain("src/main.ts");
    expect(document.body.textContent).toContain("src/utils.ts");
    expect(document.body.textContent).toContain("src/old.ts");

    // Status labels should be rendered
    expect(document.body.textContent).toContain("Modified");
    expect(document.body.textContent).toContain("Added");
    expect(document.body.textContent).toContain("Deleted");

    // Additions/deletions numbers
    expect(document.body.textContent).toContain("+3");
    expect(document.body.textContent).toContain("−1");
    expect(document.body.textContent).toContain("+10");
    expect(document.body.textContent).toContain("−5");

    // File count in title
    expect(document.body.textContent).toContain("(3)");

    // Icons: diff in header, refresh in footer, git-commit in footer commit button
    expect(
      document.body.querySelector('[data-testid="icon-diff"]'),
    ).not.toBeNull();
    expect(
      document.body.querySelector('[data-testid="icon-refresh"]'),
    ).not.toBeNull();
    expect(
      document.body.querySelector('[data-testid="icon-git-commit"]'),
    ).not.toBeNull();

    dispose();
  });

  // ── edge cases ────────────────────────────────────────────────────

  it("handles empty git status (no changes)", async () => {
    (gitStatus as Mock).mockResolvedValue(emptyStatus());

    const dispose = render(
      () => (
        <GitChangesModal
          workspace="/test"
          open
          onClose={vi.fn()}
          onCommitPush={vi.fn()}
        />
      ),
      document.body,
    );

    await flush();

    // Empty-state message should be shown
    expect(document.body.textContent).toContain("0 changes");
    // No file rows should be present
    expect(document.body.textContent).not.toContain("src/");
    // File count should be 0
    expect(document.body.textContent).toContain("(0)");

    dispose();
  });

  it("handles gitStatus rejection (error case)", async () => {
    (gitStatus as Mock).mockRejectedValue(new Error("git error"));

    const dispose = render(
      () => (
        <GitChangesModal
          workspace="/test"
          open
          onClose={vi.fn()}
          onCommitPush={vi.fn()}
        />
      ),
      document.body,
    );

    await flush();

    // On error, status is null, loading is false, files = [] -> empty state
    expect(document.body.textContent).toContain("0 changes");
    // File count should be 0
    expect(document.body.textContent).toContain("(0)");

    dispose();
  });

  // ── expand / collapse diff ────────────────────────────────────────

  it("expands a file to show its diff then collapses it", async () => {
    (gitStatus as Mock).mockResolvedValue(statusWithFiles());
    (gitFileDiff as Mock).mockImplementation(
      async (_workspace: string, path: string) => {
        if (path === "src/main.ts") return SAMPLE_DIFF;
        return "";
      },
    );

    const dispose = render(
      () => (
        <GitChangesModal
          workspace="/test"
          open
          onClose={vi.fn()}
          onCommitPush={vi.fn()}
        />
      ),
      document.body,
    );

    await flush();

    // Find the toggle button for src/main.ts
    const fileButtons = document.body.querySelectorAll("button");
    const mainBtn = Array.from(fileButtons).find((b) =>
      b.textContent?.includes("src/main.ts"),
    )!;
    expect(mainBtn).toBeTruthy();

    // --- Click to expand ---
    mainBtn.click();
    await flush();

    // gitFileDiff should have been called for this path
    expect(gitFileDiff).toHaveBeenCalledWith("/test", "src/main.ts");

    // Diff should now be visible: check for diff line content
    const diffPre = document.body.querySelector(".diff-pre");
    expect(diffPre).not.toBeNull();
    expect(diffPre!.innerHTML).toContain("world2");
    expect(diffPre!.innerHTML).toContain("diff-add");
    expect(diffPre!.innerHTML).toContain("diff-del");
    expect(diffPre!.innerHTML).toContain("diff-hunk");

    // --- Click to collapse ---
    mainBtn.click();
    await flush();

    // Diff should be hidden again
    expect(document.body.querySelector(".diff-pre")).toBeNull();
    // A second collapse should not trigger another diff fetch
    expect(gitFileDiff).toHaveBeenCalledTimes(1);

    dispose();
  });

  it("does not refetch diff for an already-expanded file", async () => {
    (gitStatus as Mock).mockResolvedValue(statusWithFiles());
    (gitFileDiff as Mock).mockImplementation(
      async (_workspace: string, path: string) => {
        if (path === "src/main.ts") return SAMPLE_DIFF;
        return "";
      },
    );

    const dispose = render(
      () => (
        <GitChangesModal
          workspace="/test"
          open
          onClose={vi.fn()}
          onCommitPush={vi.fn()}
        />
      ),
      document.body,
    );

    await flush();

    const fileButtons = document.body.querySelectorAll("button");
    const mainBtn = Array.from(fileButtons).find((b) =>
      b.textContent?.includes("src/main.ts"),
    )!;

    // Expand
    mainBtn.click();
    await flush();

    // Collapse
    mainBtn.click();
    await flush();

    // Re-expand -- diff is already cached in `diffs()`, so no second fetch
    mainBtn.click();
    await flush();

    expect(gitFileDiff).toHaveBeenCalledTimes(1);

    dispose();
  });

  it("renders status-specific colors for Modified, Added, Deleted, Untracked", async () => {
    const status: GitStatus = {
      hasChanges: true,
      files: [
        { path: "mod.ts", status: "M", additions: 1, deletions: 0 },
        { path: "add.ts", status: "A", additions: 1, deletions: 0 },
        { path: "del.ts", status: "D", additions: 0, deletions: 1 },
        { path: "new.ts", status: "?", additions: 0, deletions: 0 },
      ],
      totalAdditions: 2,
      totalDeletions: 1,
    };
    (gitStatus as Mock).mockResolvedValue(status);

    const dispose = render(
      () => (
        <GitChangesModal
          workspace="/test"
          open
          onClose={vi.fn()}
          onCommitPush={vi.fn()}
        />
      ),
      document.body,
    );

    await flush();

    // All status labels rendered
    expect(document.body.textContent).toContain("Modified");
    expect(document.body.textContent).toContain("Added");
    expect(document.body.textContent).toContain("Deleted");
    expect(document.body.textContent).toContain("Untracked");

    dispose();
  });

  // ── refresh button ────────────────────────────────────────────────

  it("refresh button re-fetches git status", async () => {
    // First call returns files, second call returns empty (simulating commit)
    (gitStatus as Mock)
      .mockResolvedValueOnce(statusWithFiles())
      .mockResolvedValueOnce(emptyStatus());

    const dispose = render(
      () => (
        <GitChangesModal
          workspace="/test"
          open
          onClose={vi.fn()}
          onCommitPush={vi.fn()}
        />
      ),
      document.body,
    );

    await flush();
    // Files are visible
    expect(document.body.textContent).toContain("src/main.ts");

    // Find and click the refresh button
    const refreshBtn = Array.from(document.body.querySelectorAll("button")).find(
      (b) => b.textContent?.includes("Refresh"),
    )!;
    expect(refreshBtn).toBeTruthy();
    refreshBtn.click();

    await flush();

    // Now empty state is shown
    expect(document.body.textContent).toContain("0 changes");
    expect(document.body.textContent).not.toContain("src/main.ts");
    // gitStatus should have been called twice (once on mount, once on refresh)
    expect(gitStatus).toHaveBeenCalledTimes(2);

    dispose();
  });

  it("refresh button is disabled while loading", async () => {
    // Keep pending so loading stays true
    (gitStatus as Mock).mockImplementation(
      () => new Promise(() => {}),
    );

    const dispose = render(
      () => (
        <GitChangesModal
          workspace="/test"
          open
          onClose={vi.fn()}
          onCommitPush={vi.fn()}
        />
      ),
      document.body,
    );

    const refreshBtn = Array.from(document.body.querySelectorAll("button")).find(
      (b) => b.textContent?.includes("Refresh"),
    )!;
    expect((refreshBtn as HTMLButtonElement).disabled).toBe(true);

    dispose();
  });

  // ── commit-push button ────────────────────────────────────────────

  it("commit-push button calls onCommitPush", async () => {
    (gitStatus as Mock).mockResolvedValue(statusWithFiles());
    const onCommitPush = vi.fn();

    const dispose = render(
      () => (
        <GitChangesModal
          workspace="/test"
          open
          onClose={vi.fn()}
          onCommitPush={onCommitPush}
        />
      ),
      document.body,
    );

    await flush();

    const commitBtn = Array.from(document.body.querySelectorAll("button")).find(
      (b) => b.textContent?.includes("Commit & Push"),
    )!;
    expect(commitBtn).toBeTruthy();

    commitBtn.click();

    expect(onCommitPush).toHaveBeenCalledTimes(1);

    dispose();
  });

  it("commit-push button is disabled when there are no files", async () => {
    (gitStatus as Mock).mockResolvedValue(emptyStatus());

    const dispose = render(
      () => (
        <GitChangesModal
          workspace="/test"
          open
          onClose={vi.fn()}
          onCommitPush={vi.fn()}
        />
      ),
      document.body,
    );

    await flush();

    const commitBtn = Array.from(document.body.querySelectorAll("button")).find(
      (b) => b.textContent?.includes("Commit & Push"),
    )!;
    expect((commitBtn as HTMLButtonElement).disabled).toBe(true);

    dispose();
  });

  // ── backdrop click ────────────────────────────────────────────────

  it("closes on backdrop click", async () => {
    (gitStatus as Mock).mockResolvedValue(emptyStatus());
    const onClose = vi.fn();

    const dispose = render(
      () => (
        <GitChangesModal
          workspace="/test"
          open
          onClose={onClose}
          onCommitPush={vi.fn()}
        />
      ),
      document.body,
    );

    await flush();

    // The backdrop is the outermost modal div with the overlay background
    const backdrop = document.body.querySelector(
      '[class*="bg-black/40"]',
    ) as HTMLElement;
    expect(backdrop).not.toBeNull();

    backdrop.click();

    expect(onClose).toHaveBeenCalledTimes(1);

    dispose();
  });

  it("does not close when clicking inside the modal content", async () => {
    (gitStatus as Mock).mockResolvedValue(emptyStatus());
    const onClose = vi.fn();

    const dispose = render(
      () => (
        <GitChangesModal
          workspace="/test"
          open
          onClose={onClose}
          onCommitPush={vi.fn()}
        />
      ),
      document.body,
    );

    await flush();

    // The inner content div (the modal body) -- clicking it should NOT close
    const content = document.body.querySelector(
      '[class*="w-[60vw]"]',
    ) as HTMLElement;
    expect(content).not.toBeNull();

    content.click();

    expect(onClose).not.toHaveBeenCalled();

    dispose();
  });

  it("closes via the X button in the header", async () => {
    (gitStatus as Mock).mockResolvedValue(emptyStatus());
    const onClose = vi.fn();

    const dispose = render(
      () => (
        <GitChangesModal
          workspace="/test"
          open
          onClose={onClose}
          onCommitPush={vi.fn()}
        />
      ),
      document.body,
    );

    await flush();

    // The X button is in the header -- the one wrapping the "x" icon
    const xIcon = document.body.querySelector(
      '[data-testid="icon-x"]',
    ) as HTMLElement;
    expect(xIcon).not.toBeNull();
    const xBtn = xIcon.closest("button") as HTMLElement;
    expect(xBtn).not.toBeNull();

    xBtn.click();

    expect(onClose).toHaveBeenCalledTimes(1);

    dispose();
  });
});
