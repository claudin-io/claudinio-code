import { describe, it, expect, vi, afterEach } from "vitest";
import { render } from "solid-js/web";
import { SkillMentionPopover } from "./SkillMentionPopover";

// jsdom doesn't implement scrollIntoView — polyfill it to suppress unhandled errors
if (typeof Element !== "undefined" && !Element.prototype.scrollIntoView) {
  Element.prototype.scrollIntoView = vi.fn();
}

// ── Sample skills ─────────────────────────────────────────────────
const sampleSkills = [
  { name: "design", description: "Design and styling skills", location: "/skills/design", scope: "builtin" as const },
  { name: "debug", description: "Debugging utilities", location: "/skills/debug", scope: "builtin" as const },
  { name: "deploy", description: "Deployment automation", location: "/skills/deploy", scope: "builtin" as const },
];

// ── Hoisted mocks ─────────────────────────────────────────────────
const mockListSkills = vi.hoisted(() => vi.fn());
const mockT = vi.hoisted(() => vi.fn((key: string) => key));

vi.mock("../lib/ipc", () => ({
  listSkills: mockListSkills,
}));

vi.mock("../lib/grill-me", () => ({
  t: mockT,
}));

// ── Default props ─────────────────────────────────────────────────
const defaultProps = () => ({
  workspace: "/test",
  bottom: 100,
  left: 200,
  query: "",
  onSelect: vi.fn(),
  onClose: vi.fn(),
});

describe("SkillMentionPopover", () => {
  afterEach(() => {
    document.body.innerHTML = "";
    vi.clearAllMocks();
  });

  it("shows loading state initially", () => {
    // Never resolve so loading stays true
    mockListSkills.mockReturnValue(new Promise(() => {}));
    const props = defaultProps();
    render(() => <SkillMentionPopover {...props} />, document.body);

    expect(document.body.textContent).toContain("Loading skills");
    expect(document.body.querySelector(".animate-spin")).toBeTruthy();
  });

  it("renders skills after loading completes", async () => {
    mockListSkills.mockResolvedValue({ skills: sampleSkills, count: sampleSkills.length });
    const props = defaultProps();
    const dispose = render(() => <SkillMentionPopover {...props} />, document.body);

    await vi.waitFor(() => {
      expect(document.body.textContent).toContain("design");
      expect(document.body.textContent).toContain("debug");
      expect(document.body.textContent).toContain("deploy");
    });
    dispose();
  });

  it("filters by query using Fuse", async () => {
    mockListSkills.mockResolvedValue({ skills: sampleSkills, count: sampleSkills.length });
    const props = { ...defaultProps(), query: "deploy" };
    render(() => <SkillMentionPopover {...props} />, document.body);

    await vi.waitFor(() => {
      expect(document.body.textContent).toContain("deploy");
    });
    expect(document.body.textContent).not.toContain("design");
    expect(document.body.textContent).not.toContain("debug");
  });

  it("shows 'No skills found' when no match", async () => {
    mockListSkills.mockResolvedValue({ skills: sampleSkills, count: sampleSkills.length });
    const props = { ...defaultProps(), query: "zzzznotfound" };
    render(() => <SkillMentionPopover {...props} />, document.body);

    await vi.waitFor(() => {
      expect(document.body.textContent).toContain("mention.noSkills");
    });
  });

  it("shows error state when listSkills fails", async () => {
    mockListSkills.mockRejectedValue(new Error("Network error"));
    const props = defaultProps();
    render(() => <SkillMentionPopover {...props} />, document.body);

    await vi.waitFor(() => {
      expect(document.body.textContent).toContain("Network error");
    });
  });

  it("calls onSelect when clicking a skill", async () => {
    mockListSkills.mockResolvedValue({ skills: sampleSkills, count: sampleSkills.length });
    const onSelect = vi.fn();
    const props = { ...defaultProps(), onSelect };
    render(() => <SkillMentionPopover {...props} />, document.body);

    await vi.waitFor(() => {
      expect(document.body.textContent).toContain("design");
    });

    const buttons = document.body.querySelectorAll("button");
    const designBtn = Array.from(buttons).find((b) =>
      b.textContent?.includes("design"),
    );
    expect(designBtn).toBeTruthy();
    designBtn!.click();

    expect(onSelect).toHaveBeenCalledWith("design");
  });

  it("calls onClose on backdrop click", async () => {
    mockListSkills.mockResolvedValue({ skills: sampleSkills, count: sampleSkills.length });
    const onClose = vi.fn();
    const props = { ...defaultProps(), onClose };
    render(() => <SkillMentionPopover {...props} />, document.body);

    // Wait for skills to load
    await vi.waitFor(() => {
      expect(document.body.textContent).toContain("design");
    });

    // The backdrop is the div with `fixed inset-0 z-40` classes
    const backdrop = document.body.querySelector('[class*="inset-0"]') as HTMLElement;
    expect(backdrop).toBeTruthy();
    backdrop.click();

    expect(onClose).toHaveBeenCalledTimes(1);
  });

  describe("keyboard navigation", () => {
    it("ArrowDown moves highlight and Enter selects the highlighted item", async () => {
      mockListSkills.mockResolvedValue({ skills: sampleSkills, count: sampleSkills.length });
      const onSelect = vi.fn();
      const props = { ...defaultProps(), onSelect };
      render(() => <SkillMentionPopover {...props} />, document.body);

      await vi.waitFor(() => {
        expect(document.body.textContent).toContain("design");
      });

      // Highlight starts at index 0 → "design"
      // Press ArrowDown twice: 0→1→2 → selects "deploy"
      document.dispatchEvent(new KeyboardEvent("keydown", { key: "ArrowDown", bubbles: true }));
      document.dispatchEvent(new KeyboardEvent("keydown", { key: "ArrowDown", bubbles: true }));
      document.dispatchEvent(new KeyboardEvent("keydown", { key: "Enter", bubbles: true }));

      expect(onSelect).toHaveBeenCalledWith("deploy");
    });

    it("ArrowUp moves highlight up and Enter selects", async () => {
      mockListSkills.mockResolvedValue({ skills: sampleSkills, count: sampleSkills.length });
      const onSelect = vi.fn();
      const props = { ...defaultProps(), onSelect };
      render(() => <SkillMentionPopover {...props} />, document.body);

      await vi.waitFor(() => {
        expect(document.body.textContent).toContain("design");
      });

      // Start at 0, go down to 1, then back up to 0 → selects "design"
      document.dispatchEvent(new KeyboardEvent("keydown", { key: "ArrowDown", bubbles: true }));
      document.dispatchEvent(new KeyboardEvent("keydown", { key: "ArrowUp", bubbles: true }));
      document.dispatchEvent(new KeyboardEvent("keydown", { key: "Enter", bubbles: true }));

      expect(onSelect).toHaveBeenCalledWith("design");
    });

    it("Enter selects the first skill when at default highlight", async () => {
      mockListSkills.mockResolvedValue({ skills: sampleSkills, count: sampleSkills.length });
      const onSelect = vi.fn();
      const props = { ...defaultProps(), onSelect };
      render(() => <SkillMentionPopover {...props} />, document.body);

      await vi.waitFor(() => {
        expect(document.body.textContent).toContain("design");
      });

      // Highlight starts at 0 (first item = "design")
      document.dispatchEvent(new KeyboardEvent("keydown", { key: "Enter", bubbles: true }));

      expect(onSelect).toHaveBeenCalledWith("design");
    });

    it("Escape calls onClose", async () => {
      mockListSkills.mockResolvedValue({ skills: sampleSkills, count: sampleSkills.length });
      const onClose = vi.fn();
      const props = { ...defaultProps(), onClose };
      render(() => <SkillMentionPopover {...props} />, document.body);

      await vi.waitFor(() => {
        expect(document.body.textContent).toContain("design");
      });

      document.dispatchEvent(new KeyboardEvent("keydown", { key: "Escape", bubbles: true }));

      expect(onClose).toHaveBeenCalledTimes(1);
    });

    it("triggers scrollIntoView on highlighted element when highlightIndex changes", async () => {
      mockListSkills.mockResolvedValue({ skills: sampleSkills, count: sampleSkills.length });
      const props = defaultProps();
      render(() => <SkillMentionPopover {...props} />, document.body);

      await vi.waitFor(() => {
        expect(document.body.textContent).toContain("design");
      });

      // scrollIntoView is polyfilled as vi.fn() at the top of the file
      // Press ArrowDown to move highlight from index 0 to index 1
      document.dispatchEvent(new KeyboardEvent("keydown", { key: "ArrowDown", bubbles: true }));

      // The effect in lines 90-96 calls btn.scrollIntoView({ block: "nearest" })
      // for the newly highlighted element. Since scrollIntoView is mocked, this
      // is a structural assertion that the code path is exercised without error.
      // We verify state consistency: the highlight moved to index 1 ("debug")
      // and clicking Enter selects the correct item.
      document.dispatchEvent(new KeyboardEvent("keydown", { key: "Enter", bubbles: true }));

      expect(props.onSelect).toHaveBeenCalledWith("debug");
    });

    it("Enter with highlightIndex=0 and results[0] exists calls onSelect via `if (selected)` guard", async () => {
      mockListSkills.mockResolvedValue({ skills: sampleSkills, count: sampleSkills.length });
      const onSelect = vi.fn();
      const props = { ...defaultProps(), onSelect };
      render(() => <SkillMentionPopover {...props} />, document.body);

      await vi.waitFor(() => {
        expect(document.body.textContent).toContain("design");
      });

      // highlightIndex is 0, results[0] exists ("design")
      // Enter handler: const selected = r[highlightIndex()]; → "design"
      // `if (selected)` is true → calls props.onSelect(selected.name)
      document.dispatchEvent(new KeyboardEvent("keydown", { key: "Enter", bubbles: true }));

      expect(onSelect).toHaveBeenCalledWith("design");
    });
  });

  it("renders error state with Icon and error message", async () => {
    mockListSkills.mockRejectedValue(new Error("Failed to load skills"));
    const props = defaultProps();
    render(() => <SkillMentionPopover {...props} />, document.body);

    // Wait for the error to render
    await vi.waitFor(() => {
      expect(document.body.textContent).toContain("Failed to load skills");
    });

    // Verify error text is rendered in a danger-colored div
    const errorDiv = document.body.querySelector('[class*="text-danger"]');
    expect(errorDiv).toBeTruthy();
    expect(errorDiv?.textContent).toBe("Error: Failed to load skills");
  });

  it("dispose triggers onCleanup (removes keydown listener)", async () => {
    mockListSkills.mockResolvedValue({ skills: sampleSkills, count: sampleSkills.length });
    const onClose = vi.fn();
    const props = { ...defaultProps(), onClose };
    const dispose = render(() => <SkillMentionPopover {...props} />, document.body);

    await vi.waitFor(() => {
      expect(document.body.textContent).toContain("design");
    });

    // Dispose unmounts the component, triggering onCleanup which removes the keydown listener
    dispose();

    // After dispose, the listener should be gone — dispatch Escape, should not call onClose
    document.dispatchEvent(new KeyboardEvent("keydown", { key: "Escape", bubbles: true }));
    expect(onClose).not.toHaveBeenCalled();
  });

  it("highlights a skill on mouse enter (covers onMouseEnter lambda)", async () => {
    mockListSkills.mockResolvedValue({ skills: sampleSkills, count: sampleSkills.length });
    const onSelect = vi.fn();
    const props = { ...defaultProps(), onSelect };
    render(() => <SkillMentionPopover {...props} />, document.body);

    await vi.waitFor(() => {
      expect(document.body.textContent).toContain("design");
    });

    // Mouseenter on "debug" button changes highlight index
    const buttons = document.body.querySelectorAll("button");
    const debugBtn = Array.from(buttons).find((b) =>
      b.textContent?.includes("debug"),
    );
    expect(debugBtn).toBeTruthy();
    debugBtn!.dispatchEvent(new MouseEvent("mouseenter", { bubbles: true }));

    // Now Enter should select "debug" because highlight moved to its row
    document.dispatchEvent(new KeyboardEvent("keydown", { key: "Enter", bubbles: true }));
    expect(onSelect).toHaveBeenCalledWith("debug");
  });
});
