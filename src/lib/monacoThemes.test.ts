import { describe, it, expect, vi, beforeEach } from "vitest";

const { mockDefineTheme } = vi.hoisted(() => ({
  mockDefineTheme: vi.fn(),
}));

vi.mock("monaco-editor", () => ({
  editor: {
    defineTheme: mockDefineTheme,
  },
}));

describe("defineMonacoThemes", () => {
  beforeEach(() => {
    vi.resetModules();
    vi.clearAllMocks();
  });

  it("defines all three theme variants (dark, light, sepia)", async () => {
    const { defineMonacoThemes } = await import("./monacoThemes");

    defineMonacoThemes();

    expect(mockDefineTheme).toHaveBeenCalledTimes(3);
    expect(mockDefineTheme).toHaveBeenCalledWith(
      "claudinio-dark",
      expect.objectContaining({ base: "vs-dark" }),
    );
    expect(mockDefineTheme).toHaveBeenCalledWith(
      "claudinio-light",
      expect.objectContaining({ base: "vs" }),
    );
    expect(mockDefineTheme).toHaveBeenCalledWith(
      "claudinio-sepia",
      expect.objectContaining({ base: "vs" }),
    );
  });

  it("is idempotent — calling twice only defines themes once", async () => {
    const { defineMonacoThemes } = await import("./monacoThemes");

    defineMonacoThemes();
    defineMonacoThemes();

    expect(mockDefineTheme).toHaveBeenCalledTimes(3);
  });
});
