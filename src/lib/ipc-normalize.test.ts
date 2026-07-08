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
