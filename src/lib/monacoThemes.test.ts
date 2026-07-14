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

  it("defines all 15 theme variants (3 legacy + 12 new)", async () => {
    const { defineMonacoThemes } = await import("./monacoThemes");

    defineMonacoThemes();

    expect(mockDefineTheme).toHaveBeenCalledTimes(15);

    // Legacy themes
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

    // New dark themes
    expect(mockDefineTheme).toHaveBeenCalledWith(
      "claudinio-dracula",
      expect.objectContaining({ base: "vs-dark" }),
    );
    expect(mockDefineTheme).toHaveBeenCalledWith(
      "claudinio-nord",
      expect.objectContaining({ base: "vs-dark" }),
    );
    expect(mockDefineTheme).toHaveBeenCalledWith(
      "claudinio-solarized-dark",
      expect.objectContaining({ base: "vs-dark" }),
    );
    expect(mockDefineTheme).toHaveBeenCalledWith(
      "claudinio-monokai",
      expect.objectContaining({ base: "vs-dark" }),
    );
    expect(mockDefineTheme).toHaveBeenCalledWith(
      "claudinio-one-dark",
      expect.objectContaining({ base: "vs-dark" }),
    );
    expect(mockDefineTheme).toHaveBeenCalledWith(
      "claudinio-catppuccin",
      expect.objectContaining({ base: "vs-dark" }),
    );
    expect(mockDefineTheme).toHaveBeenCalledWith(
      "claudinio-tokyo-night",
      expect.objectContaining({ base: "vs-dark" }),
    );
    expect(mockDefineTheme).toHaveBeenCalledWith(
      "claudinio-gruvbox-dark",
      expect.objectContaining({ base: "vs-dark" }),
    );
    expect(mockDefineTheme).toHaveBeenCalledWith(
      "claudinio-rose-pine",
      expect.objectContaining({ base: "vs-dark" }),
    );
    expect(mockDefineTheme).toHaveBeenCalledWith(
      "claudinio-everforest",
      expect.objectContaining({ base: "vs-dark" }),
    );

    // New light themes
    expect(mockDefineTheme).toHaveBeenCalledWith(
      "claudinio-solarized-light",
      expect.objectContaining({ base: "vs" }),
    );
    expect(mockDefineTheme).toHaveBeenCalledWith(
      "claudinio-gruvbox-light",
      expect.objectContaining({ base: "vs" }),
    );
  });

  it("is idempotent — calling twice only defines themes once", async () => {
    const { defineMonacoThemes } = await import("./monacoThemes");

    defineMonacoThemes();
    defineMonacoThemes();

    expect(mockDefineTheme).toHaveBeenCalledTimes(15);
  });
});

describe("getMonacoTheme", () => {
  it("maps 'claudinio' to 'claudinio-dark'", async () => {
    const { getMonacoTheme } = await import("./monacoThemes");
    expect(getMonacoTheme("claudinio" as any)).toBe("claudinio-dark");
  });

  it("maps new theme IDs to claudinio-{id}", async () => {
    const { getMonacoTheme } = await import("./monacoThemes");
    expect(getMonacoTheme("dracula" as any)).toBe("claudinio-dracula");
    expect(getMonacoTheme("nord" as any)).toBe("claudinio-nord");
    expect(getMonacoTheme("tokyo-night" as any)).toBe("claudinio-tokyo-night");
    expect(getMonacoTheme("everforest" as any)).toBe("claudinio-everforest");
  });

  it("passes through already-prefixed themes unchanged", async () => {
    const { getMonacoTheme } = await import("./monacoThemes");
    expect(getMonacoTheme("claudinio-light" as any)).toBe("claudinio-light");
    expect(getMonacoTheme("claudinio-sepia" as any)).toBe("claudinio-sepia");
    expect(getMonacoTheme("claudinio-dracula" as any)).toBe("claudinio-dracula");
  });
});
