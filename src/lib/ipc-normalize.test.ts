import { describe, it, expect } from "vitest";

describe("normalizeSessionMode", () => {
  it("returns 'brain' for 'brain'", async () => {
    const { normalizeSessionMode } = await import("./ipc");
    expect(normalizeSessionMode("brain")).toBe("brain");
  });

  it("returns 'brain' for legacy 'pensador'", async () => {
    const { normalizeSessionMode } = await import("./ipc");
    expect(normalizeSessionMode("pensador")).toBe("brain");
  });

  it("returns 'builder' for 'builder'", async () => {
    const { normalizeSessionMode } = await import("./ipc");
    expect(normalizeSessionMode("builder")).toBe("builder");
  });

  it("returns 'builder' for null", async () => {
    const { normalizeSessionMode } = await import("./ipc");
    expect(normalizeSessionMode(null)).toBe("builder");
  });

  it("returns 'builder' for undefined", async () => {
    const { normalizeSessionMode } = await import("./ipc");
    expect(normalizeSessionMode(undefined)).toBe("builder");
  });

  it("returns 'builder' for empty string", async () => {
    const { normalizeSessionMode } = await import("./ipc");
    expect(normalizeSessionMode("")).toBe("builder");
  });

  it("returns 'builder' for unknown string", async () => {
    const { normalizeSessionMode } = await import("./ipc");
    expect(normalizeSessionMode("unknown")).toBe("builder");
  });
});

describe("normalizeThinkingEffort", () => {
  it("passes through every valid level", async () => {
    const { normalizeThinkingEffort, THINKING_EFFORTS } = await import("./ipc");
    for (const level of THINKING_EFFORTS) {
      expect(normalizeThinkingEffort(level)).toBe(level);
    }
  });

  it("falls back to 'medium' for unknown, null and undefined", async () => {
    const { normalizeThinkingEffort } = await import("./ipc");
    expect(normalizeThinkingEffort("turbo")).toBe("medium");
    expect(normalizeThinkingEffort(null)).toBe("medium");
    expect(normalizeThinkingEffort(undefined)).toBe("medium");
    expect(normalizeThinkingEffort("")).toBe("medium");
  });

  it("keeps slider order lowest to highest", async () => {
    const { THINKING_EFFORTS } = await import("./ipc");
    expect(THINKING_EFFORTS).toEqual(["low", "medium", "high", "xhigh", "max"]);
  });
});
