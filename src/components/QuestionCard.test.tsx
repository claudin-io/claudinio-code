import { describe, it, expect, vi, afterEach } from "vitest";
import { render } from "solid-js/web";
import type { AskUserData } from "../lib/ipc";

// ── Module mocks ───────────────────────────────────────────────────
vi.mock("../lib/grill-me", () => ({
  t: (key: string) => key,
}));

vi.mock("./Icon", () => ({
  Icon: (props: { name: string; class?: string }) => (
    <span data-testid={`icon-${props.name}`} class={props.class} />
  ),
}));

// ── Imports (after mocks) ──────────────────────────────────────────
import QuestionCard from "./QuestionCard";

// ── Test fixtures ───────────────────────────────────────────────────
const singleAsk: AskUserData = {
  sessionId: "test-session",
  toolId: "test-tool",
  questions: [
    {
      question: "What is your favorite color?",
      options: [{ label: "Red" }, { label: "Blue" }, { label: "Green" }],
      multi_select: false,
    },
  ],
};

const multiAsk: AskUserData = {
  sessionId: "test-session",
  toolId: "test-tool",
  questions: [
    {
      question: "Select all that apply",
      options: [{ label: "Option A" }, { label: "Option B" }, { label: "Option C" }],
      multi_select: true,
    },
  ],
};

const multiQuestionAsk: AskUserData = {
  sessionId: "test-session",
  toolId: "test-tool",
  questions: [
    {
      question: "First question?",
      options: [{ label: "Yes" }, { label: "No" }],
      multi_select: false,
    },
    {
      question: "Second question?",
      options: [{ label: "Option 1" }, { label: "Option 2" }, { label: "Option 3" }],
      multi_select: true,
    },
  ],
};

// Options carrying a `description` — the richer AskUserQuestion shape.
const describedAsk: AskUserData = {
  sessionId: "test-session",
  toolId: "test-tool",
  questions: [
    {
      question: "How should the patch ship?",
      options: [
        { label: "Full release", description: "Commit, tag semver, and push" },
        { label: "Tag only", description: "Move the tag without touching the tree" },
      ],
      multi_select: false,
    },
  ],
};

// ══════════════════════════════════════════════════════════════════════
// QuestionCard tests
// ══════════════════════════════════════════════════════════════════════

describe("QuestionCard", () => {
  afterEach(() => {
    document.body.innerHTML = "";
    vi.clearAllMocks();
  });

  // ── basic rendering ────────────────────────────────────────────────
  it("renders the needs-answer badge", () => {
    const dispose = render(
      () => <QuestionCard ask={singleAsk} onSubmit={vi.fn()} />,
      document.body,
    );
    expect(document.body.textContent).toContain("chat.question.needsAnswer");
    dispose();
  });

  it("renders the question text", () => {
    const dispose = render(
      () => <QuestionCard ask={singleAsk} onSubmit={vi.fn()} />,
      document.body,
    );
    expect(document.body.textContent).toContain("What is your favorite color?");
    dispose();
  });

  it("renders all option buttons", () => {
    const dispose = render(
      () => <QuestionCard ask={singleAsk} onSubmit={vi.fn()} />,
      document.body,
    );
    expect(document.body.textContent).toContain("Red");
    expect(document.body.textContent).toContain("Blue");
    expect(document.body.textContent).toContain("Green");
    dispose();
  });

  it("renders both the option label and its description", () => {
    const dispose = render(
      () => <QuestionCard ask={describedAsk} onSubmit={vi.fn()} />,
      document.body,
    );
    expect(document.body.textContent).toContain("Full release");
    expect(document.body.textContent).toContain("Commit, tag semver, and push");
    expect(document.body.textContent).toContain("Tag only");
    dispose();
  });

  it("submits the option label (not the description) as the answer", () => {
    const onSubmit = vi.fn();
    const dispose = render(
      () => <QuestionCard ask={describedAsk} onSubmit={onSubmit} />,
      document.body,
    );
    const buttons = document.body.querySelectorAll("button");
    const fullRelease = Array.from(buttons).find((b) =>
      b.textContent?.includes("Full release"),
    )!;
    fullRelease.click();

    const allButtons = document.body.querySelectorAll("button");
    const submitBtn = allButtons[allButtons.length - 1] as HTMLButtonElement;
    submitBtn.click();

    expect(onSubmit).toHaveBeenCalledWith([
      { question: "How should the patch ship?", answer: "Full release" },
    ]);
    dispose();
  });

  it('renders the "Other" button', () => {
    const dispose = render(
      () => <QuestionCard ask={singleAsk} onSubmit={vi.fn()} />,
      document.body,
    );
    expect(document.body.textContent).toContain("chat.question.other");
    dispose();
  });

  it("renders the submit button with send icon", () => {
    const dispose = render(
      () => <QuestionCard ask={singleAsk} onSubmit={vi.fn()} />,
      document.body,
    );
    expect(document.body.textContent).toContain("chat.question.submit");

    const buttons = document.body.querySelectorAll("button");
    const submitBtn = buttons[buttons.length - 1];
    const sendIcon = submitBtn.querySelector('[data-testid="icon-send"]');
    expect(sendIcon).not.toBeNull();
    dispose();
  });

  it("renders the submit button disabled initially", () => {
    const dispose = render(
      () => <QuestionCard ask={singleAsk} onSubmit={vi.fn()} />,
      document.body,
    );
    const buttons = document.body.querySelectorAll("button");
    const submitBtn = buttons[buttons.length - 1] as HTMLButtonElement;
    expect(submitBtn.disabled).toBe(true);
    dispose();
  });

  // ── single-select behavior ─────────────────────────────────────────
  it("selects an option on click (single-select)", () => {
    const dispose = render(
      () => <QuestionCard ask={singleAsk} onSubmit={vi.fn()} />,
      document.body,
    );
    const buttons = document.body.querySelectorAll("button");
    const redBtn = Array.from(buttons).find((b) =>
      b.textContent?.includes("Red"),
    )!;
    redBtn.click();
    expect(redBtn.className).toContain("bg-accent/10");
    dispose();
  });

  it("deselects previous option when a different one is clicked (single-select)", () => {
    const dispose = render(
      () => <QuestionCard ask={singleAsk} onSubmit={vi.fn()} />,
      document.body,
    );
    const buttons = document.body.querySelectorAll("button");
    const redBtn = Array.from(buttons).find((b) =>
      b.textContent?.includes("Red"),
    )!;
    const blueBtn = Array.from(buttons).find((b) =>
      b.textContent?.includes("Blue"),
    )!;

    redBtn.click();
    expect(redBtn.className).toContain("bg-accent/10");

    blueBtn.click();
    expect(redBtn.className).toContain("bg-surface-0");
    expect(blueBtn.className).toContain("bg-accent/10");
    dispose();
  });

  it("enables submit button when an option is selected (single question)", () => {
    const dispose = render(
      () => <QuestionCard ask={singleAsk} onSubmit={vi.fn()} />,
      document.body,
    );
    const buttons = document.body.querySelectorAll("button");
    const redBtn = Array.from(buttons).find((b) =>
      b.textContent?.includes("Red"),
    )!;
    redBtn.click();

    const allButtons = document.body.querySelectorAll("button");
    const submitBtn = allButtons[allButtons.length - 1] as HTMLButtonElement;
    expect(submitBtn.disabled).toBe(false);
    dispose();
  });

  // ── multi-select behavior ──────────────────────────────────────────
  it("toggles options independently in multi-select mode", () => {
    const dispose = render(
      () => <QuestionCard ask={multiAsk} onSubmit={vi.fn()} />,
      document.body,
    );
    const buttons = document.body.querySelectorAll("button");
    const optA = Array.from(buttons).find((b) =>
      b.textContent?.includes("Option A"),
    )!;
    const optB = Array.from(buttons).find((b) =>
      b.textContent?.includes("Option B"),
    )!;

    optA.click();
    optB.click();
    expect(optA.className).toContain("bg-accent/10");
    expect(optB.className).toContain("bg-accent/10");

    // Toggle off Option A
    optA.click();
    expect(optA.className).toContain("bg-surface-0");
    expect(optB.className).toContain("bg-accent/10");
    dispose();
  });

  it("shows checkbox-style indicator (rounded-sm) for multi-select", () => {
    const dispose = render(
      () => <QuestionCard ask={multiAsk} onSubmit={vi.fn()} />,
      document.body,
    );
    const buttons = document.body.querySelectorAll("button");
    const optA = Array.from(buttons).find((b) =>
      b.textContent?.includes("Option A"),
    )!;
    const indicator = optA.querySelector(".rounded-sm");
    expect(indicator).not.toBeNull();
    dispose();
  });

  it("shows radio-style indicator (rounded-full) for single-select", () => {
    const dispose = render(
      () => <QuestionCard ask={singleAsk} onSubmit={vi.fn()} />,
      document.body,
    );
    const buttons = document.body.querySelectorAll("button");
    const redBtn = Array.from(buttons).find((b) =>
      b.textContent?.includes("Red"),
    )!;
    const indicator = redBtn.querySelector(".rounded-full");
    expect(indicator).not.toBeNull();
    dispose();
  });

  // ── "Other" behavior ──────────────────────────────────────────────
  it("shows the other-text input when 'Other' is clicked", () => {
    const dispose = render(
      () => <QuestionCard ask={singleAsk} onSubmit={vi.fn()} />,
      document.body,
    );
    const buttons = document.body.querySelectorAll("button");
    const otherBtn = Array.from(buttons).find(
      (b) => b.textContent === "chat.question.other",
    )!;
    otherBtn.click();

    const input = document.body.querySelector(
      'input[placeholder="chat.question.typeAnswer"]',
    ) as HTMLInputElement;
    expect(input).not.toBeNull();
    dispose();
  });

  it("hides the other-text input when 'Other' is deselected (multi-select)", () => {
    const dispose = render(
      () => <QuestionCard ask={multiAsk} onSubmit={vi.fn()} />,
      document.body,
    );
    const buttons = document.body.querySelectorAll("button");
    const otherBtn = Array.from(buttons).find(
      (b) => b.textContent === "chat.question.other",
    )!;

    otherBtn.click();
    expect(
      document.body.querySelector(
        'input[placeholder="chat.question.typeAnswer"]',
      ),
    ).not.toBeNull();

    otherBtn.click();
    expect(
      document.body.querySelector(
        'input[placeholder="chat.question.typeAnswer"]',
      ),
    ).toBeNull();
    dispose();
  });

  it("clears option picks when 'Other' is clicked in single-select mode", () => {
    const dispose = render(
      () => <QuestionCard ask={singleAsk} onSubmit={vi.fn()} />,
      document.body,
    );
    const buttons = document.body.querySelectorAll("button");
    const redBtn = Array.from(buttons).find((b) =>
      b.textContent?.includes("Red"),
    )!;
    const otherBtn = Array.from(buttons).find(
      (b) => b.textContent === "chat.question.other",
    )!;

    redBtn.click();
    expect(redBtn.className).toContain("bg-accent/10");

    otherBtn.click();
    expect(redBtn.className).toContain("bg-surface-0");
    dispose();
  });

  it("clears 'Other' selection when an option is clicked in single-select mode", () => {
    const dispose = render(
      () => <QuestionCard ask={singleAsk} onSubmit={vi.fn()} />,
      document.body,
    );
    const buttons = document.body.querySelectorAll("button");
    const redBtn = Array.from(buttons).find((b) =>
      b.textContent?.includes("Red"),
    )!;
    const otherBtn = Array.from(buttons).find(
      (b) => b.textContent === "chat.question.other",
    )!;

    otherBtn.click();
    expect(
      document.body.querySelector(
        'input[placeholder="chat.question.typeAnswer"]',
      ),
    ).not.toBeNull();

    redBtn.click();
    expect(
      document.body.querySelector(
        'input[placeholder="chat.question.typeAnswer"]',
      ),
    ).toBeNull();
    dispose();
  });

  it("keeps existing picks when toggling 'Other' in multi-select mode", () => {
    const dispose = render(
      () => <QuestionCard ask={multiAsk} onSubmit={vi.fn()} />,
      document.body,
    );
    const buttons = document.body.querySelectorAll("button");
    const optA = Array.from(buttons).find((b) =>
      b.textContent?.includes("Option A"),
    )!;
    const otherBtn = Array.from(buttons).find(
      (b) => b.textContent === "chat.question.other",
    )!;

    optA.click();
    otherBtn.click();

    // Option A should still be selected
    expect(optA.className).toContain("bg-accent/10");

    // Toggle other off — Option A should remain
    otherBtn.click();
    expect(optA.className).toContain("bg-accent/10");
    dispose();
  });

  it("keeps submit disabled when 'Other' is selected but input is empty", () => {
    const dispose = render(
      () => <QuestionCard ask={singleAsk} onSubmit={vi.fn()} />,
      document.body,
    );
    const buttons = document.body.querySelectorAll("button");
    const otherBtn = Array.from(buttons).find(
      (b) => b.textContent === "chat.question.other",
    )!;
    otherBtn.click();

    const allButtons = document.body.querySelectorAll("button");
    const submitBtn = allButtons[allButtons.length - 1] as HTMLButtonElement;
    expect(submitBtn.disabled).toBe(true);
    dispose();
  });

  it("enables submit when 'Other' is selected and text is typed", () => {
    const dispose = render(
      () => <QuestionCard ask={singleAsk} onSubmit={vi.fn()} />,
      document.body,
    );
    const buttons = document.body.querySelectorAll("button");
    const otherBtn = Array.from(buttons).find(
      (b) => b.textContent === "chat.question.other",
    )!;
    otherBtn.click();

    const input = document.body.querySelector(
      'input[placeholder="chat.question.typeAnswer"]',
    ) as HTMLInputElement;
    input.value = "Custom answer";
    input.dispatchEvent(new InputEvent("input", { bubbles: true }));

    const allButtons = document.body.querySelectorAll("button");
    const submitBtn = allButtons[allButtons.length - 1] as HTMLButtonElement;
    expect(submitBtn.disabled).toBe(false);
    dispose();
  });

  // ── submit behavior ───────────────────────────────────────────────
  it("calls onSubmit with the selected answer (single-select)", () => {
    const onSubmit = vi.fn();
    const dispose = render(
      () => <QuestionCard ask={singleAsk} onSubmit={onSubmit} />,
      document.body,
    );

    const buttons = document.body.querySelectorAll("button");
    const redBtn = Array.from(buttons).find((b) =>
      b.textContent?.includes("Red"),
    )!;
    redBtn.click();

    const allButtons = document.body.querySelectorAll("button");
    const submitBtn = allButtons[allButtons.length - 1] as HTMLButtonElement;
    submitBtn.click();

    expect(onSubmit).toHaveBeenCalledTimes(1);
    expect(onSubmit).toHaveBeenCalledWith([
      { question: "What is your favorite color?", answer: "Red" },
    ]);
    dispose();
  });

  it("calls onSubmit with comma-joined answers (multi-select)", () => {
    const onSubmit = vi.fn();
    const dispose = render(
      () => <QuestionCard ask={multiAsk} onSubmit={onSubmit} />,
      document.body,
    );

    const buttons = document.body.querySelectorAll("button");
    const optA = Array.from(buttons).find((b) =>
      b.textContent?.includes("Option A"),
    )!;
    const optC = Array.from(buttons).find((b) =>
      b.textContent?.includes("Option C"),
    )!;
    optA.click();
    optC.click();

    const allButtons = document.body.querySelectorAll("button");
    const submitBtn = allButtons[allButtons.length - 1] as HTMLButtonElement;
    submitBtn.click();

    expect(onSubmit).toHaveBeenCalledWith([
      { question: "Select all that apply", answer: "Option A, Option C" },
    ]);
    dispose();
  });

  it("includes 'Other' text in answer when submitted", () => {
    const onSubmit = vi.fn();
    const dispose = render(
      () => <QuestionCard ask={singleAsk} onSubmit={onSubmit} />,
      document.body,
    );

    const buttons = document.body.querySelectorAll("button");
    const otherBtn = Array.from(buttons).find(
      (b) => b.textContent === "chat.question.other",
    )!;
    otherBtn.click();

    const input = document.body.querySelector(
      'input[placeholder="chat.question.typeAnswer"]',
    ) as HTMLInputElement;
    input.value = "Purple";
    input.dispatchEvent(new InputEvent("input", { bubbles: true }));

    const allButtons = document.body.querySelectorAll("button");
    const submitBtn = allButtons[allButtons.length - 1] as HTMLButtonElement;
    submitBtn.click();

    expect(onSubmit).toHaveBeenCalledWith([
      { question: "What is your favorite color?", answer: "Purple" },
    ]);
    dispose();
  });

  it("includes both picks and 'Other' text in multi-select answer", () => {
    const onSubmit = vi.fn();
    const ask: AskUserData = {
      sessionId: "test",
      toolId: "test",
      questions: [
        {
          question: "Pick colors",
          options: [{ label: "Red" }, { label: "Blue" }],
          multi_select: true,
        },
      ],
    };
    const dispose = render(
      () => <QuestionCard ask={ask} onSubmit={onSubmit} />,
      document.body,
    );

    const buttons = document.body.querySelectorAll("button");
    const redBtn = Array.from(buttons).find((b) =>
      b.textContent?.includes("Red"),
    )!;
    redBtn.click();

    const otherBtn = Array.from(buttons).find(
      (b) => b.textContent === "chat.question.other",
    )!;
    otherBtn.click();

    const input = document.body.querySelector(
      'input[placeholder="chat.question.typeAnswer"]',
    ) as HTMLInputElement;
    input.value = "Green";
    input.dispatchEvent(new InputEvent("input", { bubbles: true }));

    const allButtons = document.body.querySelectorAll("button");
    const submitBtn = allButtons[allButtons.length - 1] as HTMLButtonElement;
    submitBtn.click();

    expect(onSubmit).toHaveBeenCalledWith([
      { question: "Pick colors", answer: "Red, Green" },
    ]);
    dispose();
  });

  // ── multi-question validation ─────────────────────────────────────
  it("does not call onSubmit when not all questions are answered", () => {
    const onSubmit = vi.fn();
    const dispose = render(
      () => <QuestionCard ask={multiQuestionAsk} onSubmit={onSubmit} />,
      document.body,
    );

    const buttons = document.body.querySelectorAll("button");
    const yesBtn = Array.from(buttons).find((b) =>
      b.textContent?.includes("Yes"),
    )!;
    yesBtn.click();

    const allButtons = document.body.querySelectorAll("button");
    const submitBtn = allButtons[allButtons.length - 1] as HTMLButtonElement;
    expect(submitBtn.disabled).toBe(true);
    submitBtn.click();
    expect(onSubmit).not.toHaveBeenCalled();
    dispose();
  });

  it("calls onSubmit only after all questions are answered", () => {
    const onSubmit = vi.fn();
    const dispose = render(
      () => <QuestionCard ask={multiQuestionAsk} onSubmit={onSubmit} />,
      document.body,
    );

    const buttons = document.body.querySelectorAll("button");

    const yesBtn = Array.from(buttons).find((b) =>
      b.textContent?.includes("Yes"),
    )!;
    yesBtn.click();

    const opt1 = Array.from(buttons).find((b) =>
      b.textContent?.includes("Option 1"),
    )!;
    opt1.click();

    const allButtons = document.body.querySelectorAll("button");
    const submitBtn = allButtons[allButtons.length - 1] as HTMLButtonElement;
    expect(submitBtn.disabled).toBe(false);
    submitBtn.click();

    expect(onSubmit).toHaveBeenCalledTimes(1);
    expect(onSubmit).toHaveBeenCalledWith([
      { question: "First question?", answer: "Yes" },
      { question: "Second question?", answer: "Option 1" },
    ]);
    dispose();
  });

  // ── Enter key on other input ──────────────────────────────────────
  it("submits on Enter keypress in the other-text input", () => {
    const onSubmit = vi.fn();
    const dispose = render(
      () => <QuestionCard ask={singleAsk} onSubmit={onSubmit} />,
      document.body,
    );

    const buttons = document.body.querySelectorAll("button");
    const otherBtn = Array.from(buttons).find(
      (b) => b.textContent === "chat.question.other",
    )!;
    otherBtn.click();

    const input = document.body.querySelector(
      'input[placeholder="chat.question.typeAnswer"]',
    ) as HTMLInputElement;
    input.value = "Purple";
    input.dispatchEvent(new InputEvent("input", { bubbles: true }));
    input.dispatchEvent(
      new KeyboardEvent("keydown", { key: "Enter", bubbles: true }),
    );

    expect(onSubmit).toHaveBeenCalledTimes(1);
    dispose();
  });

  // ── visual indicator assertions ───────────────────────────────────
  it("renders a check icon for selected options", () => {
    const dispose = render(
      () => <QuestionCard ask={singleAsk} onSubmit={vi.fn()} />,
      document.body,
    );

    const buttons = document.body.querySelectorAll("button");
    const redBtn = Array.from(buttons).find((b) =>
      b.textContent?.includes("Red"),
    )!;
    redBtn.click();

    const checkIcons = redBtn.querySelectorAll('[data-testid="icon-check"]');
    expect(checkIcons.length).toBe(1);
    dispose();
  });

  it("does not render check icon for unselected options", () => {
    const dispose = render(
      () => <QuestionCard ask={singleAsk} onSubmit={vi.fn()} />,
      document.body,
    );

    const buttons = document.body.querySelectorAll("button");
    const redBtn = Array.from(buttons).find((b) =>
      b.textContent?.includes("Red"),
    )!;
    const blueBtn = Array.from(buttons).find((b) =>
      b.textContent?.includes("Blue"),
    )!;

    redBtn.click();
    const blueCheck = blueBtn.querySelectorAll('[data-testid="icon-check"]');
    expect(blueCheck.length).toBe(0);
    dispose();
  });
});
