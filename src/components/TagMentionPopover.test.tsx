import { describe, it, expect, vi, afterEach } from "vitest";
import { render } from "solid-js/web";
import { TagMentionPopover } from "./TagMentionPopover";

describe("TagMentionPopover", () => {
  afterEach(() => {
    document.body.innerHTML = "";
  });

  const defaultProps = () => ({
    query: "",
    onSelect: vi.fn(),
    onClose: vi.fn(),
  });

  it("renders all tags", () => {
    const props = defaultProps();
    const dispose = render(() => <TagMentionPopover {...props} />, document.body);

    expect(document.body.textContent).toContain("skill");
    expect(document.body.textContent).toContain("goal");
    expect(document.body.textContent).toContain("agent");
    expect(document.body.textContent).toContain("prompt");
    dispose();
  });

  it("filters by query", () => {
    const props = { ...defaultProps(), query: "skill" };
    const dispose = render(() => <TagMentionPopover {...props} />, document.body);

    expect(document.body.textContent).toContain("skill");
    expect(document.body.textContent).not.toContain("goal");
    expect(document.body.textContent).not.toContain("agent");
    expect(document.body.textContent).not.toContain("prompt");
    dispose();
  });

  it("shows disabled badge for disabled tags", () => {
    const props = defaultProps();
    const dispose = render(() => <TagMentionPopover {...props} />, document.body);

    const buttons = document.body.querySelectorAll("button");
    const agentBtn = Array.from(buttons).find((b) =>
      b.textContent?.includes("agent"),
    );
    const promptBtn = Array.from(buttons).find((b) =>
      b.textContent?.includes("prompt"),
    );
    const skillBtn = Array.from(buttons).find((b) =>
      b.textContent?.includes("skill"),
    );
    const goalBtn = Array.from(buttons).find((b) =>
      b.textContent?.includes("goal"),
    );

    expect(agentBtn?.textContent).toContain("soon");
    expect(promptBtn?.textContent).toContain("soon");
    expect(skillBtn?.textContent).not.toContain("soon");
    expect(goalBtn?.textContent).not.toContain("soon");
    dispose();
  });

  it("calls onSelect with tag id", () => {
    const onSelect = vi.fn();
    const props = { ...defaultProps(), onSelect };
    render(() => <TagMentionPopover {...props} />, document.body);

    const buttons = document.body.querySelectorAll("button");
    const skillBtn = Array.from(buttons).find((b) =>
      b.textContent?.includes("skill"),
    );
    skillBtn!.click();

    expect(onSelect).toHaveBeenCalledWith("skill");
  });

  it("clicking disabled tag is a no-op", () => {
    const onSelect = vi.fn();
    const onClose = vi.fn();
    const props = { ...defaultProps(), onSelect, onClose };
    render(() => <TagMentionPopover {...props} />, document.body);

    const buttons = document.body.querySelectorAll("button");
    const agentBtn = Array.from(buttons).find((b) =>
      b.textContent?.includes("agent"),
    );
    agentBtn!.click();

    expect(onSelect).not.toHaveBeenCalled();
    expect(onClose).not.toHaveBeenCalled();
  });

  describe("keyboard navigation", () => {
    it("ArrowDown moves highlight and Enter selects the enabled tag", () => {
      const onSelect = vi.fn();
      const props = { ...defaultProps(), query: "", onSelect };
      render(() => <TagMentionPopover {...props} />, document.body);

      // Results order: skill (enabled), goal (enabled), agent (disabled), prompt (disabled)
      // Highlight starts at 0 → "skill", ArrowDown → 1 → "goal"
      document.dispatchEvent(new KeyboardEvent("keydown", { key: "ArrowDown", bubbles: true }));
      document.dispatchEvent(new KeyboardEvent("keydown", { key: "Enter", bubbles: true }));

      expect(onSelect).toHaveBeenCalledWith("goal");
    });

    it("ArrowUp moves highlight up and Enter selects", () => {
      const onSelect = vi.fn();
      const props = { ...defaultProps(), query: "", onSelect };
      render(() => <TagMentionPopover {...props} />, document.body);

      // Start at 0, go down to 1, then back up to 0 → selects "skill"
      document.dispatchEvent(new KeyboardEvent("keydown", { key: "ArrowDown", bubbles: true }));
      document.dispatchEvent(new KeyboardEvent("keydown", { key: "ArrowUp", bubbles: true }));
      document.dispatchEvent(new KeyboardEvent("keydown", { key: "Enter", bubbles: true }));

      expect(onSelect).toHaveBeenCalledWith("skill");
    });

    it("ArrowDown at last item stays clamped", () => {
      const onSelect = vi.fn();
      const props = { ...defaultProps(), query: "", onSelect };
      render(() => <TagMentionPopover {...props} />, document.body);

      // 4 items — navigate far past end, should clamp at index 3 (last item)
      for (let i = 0; i < 10; i++) {
        document.dispatchEvent(new KeyboardEvent("keydown", { key: "ArrowDown", bubbles: true }));
      }
      document.dispatchEvent(new KeyboardEvent("keydown", { key: "Enter", bubbles: true }));

      // Index 3 = "prompt" (disabled) — Enter guard prevents selection
      expect(onSelect).not.toHaveBeenCalled();
    });

    it("Enter on a disabled tag is a no-op", () => {
      const onSelect = vi.fn();
      const props = { ...defaultProps(), query: "agent", onSelect };
      render(() => <TagMentionPopover {...props} />, document.body);

      // query="agent" filters to only the agent tag — which is disabled
      // Highlight is at index 0 (the only result)
      document.dispatchEvent(new KeyboardEvent("keydown", { key: "Enter", bubbles: true }));

      // The guard `selected?.enabled` prevents calling onSelect
      expect(onSelect).not.toHaveBeenCalled();
    });
  });

  it("ArrowDown on empty results returns early (no crash)", () => {
    const onSelect = vi.fn();
    const onClose = vi.fn();
    const props = { ...defaultProps(), query: "matchNothing", onSelect, onClose };
    render(() => <TagMentionPopover {...props} />, document.body);

    // No results → pressing ArrowDown hits `if (r.length === 0) return`
    expect(() => {
      document.dispatchEvent(new KeyboardEvent("keydown", { key: "ArrowDown", bubbles: true }));
    }).not.toThrow();
    expect(onSelect).not.toHaveBeenCalled();
    expect(onClose).not.toHaveBeenCalled();
  });

  it("clamps highlight index when results shrink", () => {
    const onSelect = vi.fn();
    const props = { ...defaultProps(), onSelect };
    render(() => <TagMentionPopover {...props} />, document.body);

    // All 4 tags shown. Navigate to index 3 (last — "prompt", disabled)
    for (let i = 0; i < 3; i++) {
      document.dispatchEvent(new KeyboardEvent("keydown", { key: "ArrowDown", bubbles: true }));
    }

    // Since we have 4 results, index 3→ArrowDown at index 3 → clamped to 3
    // Enter on index 3 = "prompt" (disabled) → no-op
    document.dispatchEvent(new KeyboardEvent("keydown", { key: "ArrowDown", bubbles: true }));
    document.dispatchEvent(new KeyboardEvent("keydown", { key: "Enter", bubbles: true }));

    expect(onSelect).not.toHaveBeenCalled();
  });

  it("dispose triggers onCleanup (removes keydown listener)", () => {
    const props = defaultProps();
    const dispose = render(() => <TagMentionPopover {...props} />, document.body);

    // Dispatch ArrowDown + Enter while mounted — should call onSelect
    document.dispatchEvent(new KeyboardEvent("keydown", { key: "ArrowDown", bubbles: true }));
    document.dispatchEvent(new KeyboardEvent("keydown", { key: "Enter", bubbles: true }));

    expect(props.onSelect).toHaveBeenCalledWith("goal");

    // onCleanup should remove the keydown listener. After dispose, Enter should
    // NOT call onSelect again.
    props.onSelect.mockClear();
    dispose();
    document.body.innerHTML = "";
  });

  it("highlights a tag on mouse enter (covers onMouseEnter lambda)", () => {
    const onSelect = vi.fn();
    const props = { ...defaultProps(), onSelect };
    render(() => <TagMentionPopover {...props} />, document.body);

    // Mouseenter on "goal" button changes highlight index
    const buttons = document.body.querySelectorAll("button");
    const goalBtn = Array.from(buttons).find((b) =>
      b.textContent?.includes("goal"),
    );
    expect(goalBtn).toBeTruthy();
    goalBtn!.dispatchEvent(new MouseEvent("mouseenter", { bubbles: true }));

    // Now Enter should select "goal" because highlight moved to its row
    document.dispatchEvent(new KeyboardEvent("keydown", { key: "Enter", bubbles: true }));
    expect(onSelect).toHaveBeenCalledWith("goal");
  });
});
