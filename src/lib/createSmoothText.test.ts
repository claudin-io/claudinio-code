import { describe, it, expect, vi, afterEach } from "vitest";
import { createRoot, createSignal } from "solid-js";
import { createSmoothText, balanceMarkdown } from "./createSmoothText";

afterEach(() => {
  vi.useRealTimers();
});

// Solid only flushes a createEffect's first run after the createRoot
// callback that created it returns (it's queued, not synchronous), so setup
// happens inside createRoot and every assertion/timer-advance happens after
// it returns — otherwise the effect (and the timer it starts) never fires.
function setup(target: () => string, finished: () => boolean, opts?: Parameters<typeof createSmoothText>[2]) {
  let smooth!: ReturnType<typeof createSmoothText>;
  let dispose!: () => void;
  createRoot((d) => {
    dispose = d;
    smooth = createSmoothText(target, finished, opts);
  });
  return { smooth, dispose };
}

describe("balanceMarkdown", () => {
  it("leaves text with an even number of fences untouched", () => {
    expect(balanceMarkdown("plain text")).toBe("plain text");
    expect(balanceMarkdown("```js\ncode\n```")).toBe("```js\ncode\n```");
  });

  it("closes an odd trailing fence", () => {
    expect(balanceMarkdown("before ```js\ncode")).toBe("before ```js\ncode\n```");
  });

  it("does not duplicate a newline before the closing fence", () => {
    expect(balanceMarkdown("```js\ncode\n")).toBe("```js\ncode\n```");
  });
});

describe("createSmoothText", () => {
  it("reveals words progressively at the base rate, not all at once", () => {
    vi.useFakeTimers();
    const [target] = createSignal("one two three four five six seven eight nine ten");
    const [finished] = createSignal(false);
    const { smooth, dispose } = setup(target, finished, { baseWps: 10, backlogScale: 1000 });

    expect(smooth.displayed()).toBe("");
    vi.advanceTimersByTime(500); // ~5 words at 10 wps
    const midway = smooth.displayed();
    expect(midway.length).toBeGreaterThan(0);
    expect(midway.length).toBeLessThan(target().length);
    expect(target().startsWith(midway)).toBe(true);

    dispose();
  });

  it("reaches the full target eventually", () => {
    vi.useFakeTimers();
    const [target] = createSignal("one two three four five");
    const [finished] = createSignal(false);
    const { smooth, dispose } = setup(target, finished, { baseWps: 20 });

    vi.advanceTimersByTime(5000);
    expect(smooth.displayed()).toBe(target());
    expect(smooth.isDrained()).toBe(true);

    dispose();
  });

  it("speeds up with a larger backlog", () => {
    vi.useFakeTimers();
    const longText = Array.from({ length: 200 }, (_, i) => `word${i}`).join(" ");
    const [target] = createSignal(longText);
    const [finished] = createSignal(false);
    const { smooth, dispose } = setup(target, finished, { baseWps: 10, backlogScale: 20, maxWps: 1000 });

    vi.advanceTimersByTime(1000);
    // With a 200-word backlog and backlogScale=20, the rate scales well past
    // the 10 wps base — expect much more than 10 words revealed in 1s.
    const wordsRevealed = smooth.displayed().trim().split(/\s+/).length;
    expect(wordsRevealed).toBeGreaterThan(15);
    expect(smooth.displayed().length).toBeLessThanOrEqual(longText.length);

    dispose();
  });

  it("drains fast once finished() is true", () => {
    vi.useFakeTimers();
    const [target] = createSignal("one two three four five six seven eight");
    const [finished, setFinished] = createSignal(false);
    const { smooth, dispose } = setup(target, finished, { baseWps: 5, finishMultiplier: 10 });

    vi.advanceTimersByTime(200);
    expect(smooth.displayed().length).toBeLessThan(target().length);

    setFinished(true);
    vi.advanceTimersByTime(500);
    expect(smooth.displayed()).toBe(target());

    dispose();
  });

  it("resets when the target shrinks (retry) instead of showing a stale prefix", () => {
    vi.useFakeTimers();
    const [target, setTarget] = createSignal("the original streamed answer continues");
    const [finished] = createSignal(false);
    const { smooth, dispose } = setup(target, finished, { baseWps: 50 });

    vi.advanceTimersByTime(200);
    expect(smooth.displayed().length).toBeGreaterThan(0);

    setTarget("retry");
    expect(smooth.displayed()).toBe("");
    expect(target().startsWith(smooth.displayed())).toBe(true);

    dispose();
  });

  it("resets when the new target diverges even if it's longer", () => {
    vi.useFakeTimers();
    const [target, setTarget] = createSignal("hello world this is a test");
    const [finished] = createSignal(false);
    const { smooth, dispose } = setup(target, finished, { baseWps: 50 });

    vi.advanceTimersByTime(200);
    expect(smooth.displayed().length).toBeGreaterThan(0);

    // Diverges from the prior snapshot despite being a longer string.
    setTarget("goodbye world this is different text entirely");
    expect(smooth.displayed()).toBe("");

    dispose();
  });

  it("flush() jumps straight to the full target", () => {
    const [target] = createSignal("one two three four five");
    const [finished] = createSignal(false);
    const { smooth, dispose } = setup(target, finished);

    smooth.flush();
    expect(smooth.displayed()).toBe(target());
    expect(smooth.isDrained()).toBe(true);

    dispose();
  });

  it("reset() clears back to empty", () => {
    vi.useFakeTimers();
    const [target] = createSignal("one two three four five");
    const [finished] = createSignal(false);
    const { smooth, dispose } = setup(target, finished, { baseWps: 50 });

    vi.advanceTimersByTime(200);
    expect(smooth.displayed().length).toBeGreaterThan(0);

    smooth.reset();
    expect(smooth.displayed()).toBe("");

    dispose();
  });
});
