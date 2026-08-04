import { describe, it, expect, beforeEach } from "vitest";
import { bootstrapTestsPrompt, seedComposer, takeComposerSeed } from "./composerSeed";

beforeEach(() => {
  // Drain anything a previous test left behind.
  takeComposerSeed("/a");
  takeComposerSeed("/b");
});

describe("composer seed", () => {
  it("delivers a seed to the workspace it was addressed to", () => {
    seedComposer("/a", "hello");
    expect(takeComposerSeed("/a")).toBe("hello");
  });

  it("does not deliver another workspace's seed", () => {
    // Two projects can be open at once; a prompt meant for one must not
    // appear in the other's composer.
    seedComposer("/a", "hello");
    expect(takeComposerSeed("/b")).toBeNull();
    expect(takeComposerSeed("/a")).toBe("hello");
  });

  it("is consumed once, so it does not reappear on every re-render", () => {
    seedComposer("/a", "hello");
    expect(takeComposerSeed("/a")).toBe("hello");
    expect(takeComposerSeed("/a")).toBeNull();
  });
});

describe("bootstrapTestsPrompt", () => {
  it("is a goal, so the harness enforces it rather than trusting the agent", () => {
    expect(bootstrapTestsPrompt([])).toContain("<goal>");
  });

  it("demands real assertions and names the check that proves them", () => {
    // The whole risk of AI-written tests is tests that assert nothing. The
    // prompt has to close that itself.
    const prompt = bootstrapTestsPrompt([]);
    expect(prompt).toContain("ASSERT something specific");
    expect(prompt).toContain("mutation");
  });

  it("tells the agent to fit an existing runner when there is one", () => {
    const prompt = bootstrapTestsPrompt(["rust", "js"]);
    expect(prompt).toContain("rust, js");
    expect(prompt).toContain("rather than introducing a second framework");
  });

  it("tells the agent to choose a runner when there is none", () => {
    const prompt = bootstrapTestsPrompt([]);
    expect(prompt).toContain("no test runner at all");
    expect(prompt).toContain("dev dependency");
  });

  it("is ASCII, because the backend rejects non-English input before the run starts", () => {
    for (const detected of [[], ["rust"]]) {
      const prompt = bootstrapTestsPrompt(detected);
      // eslint-disable-next-line no-control-regex
      expect(/^[\x00-\x7F]*$/.test(prompt)).toBe(true);
    }
  });
});
