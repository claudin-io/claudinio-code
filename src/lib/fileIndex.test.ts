import { describe, it, expect, vi, beforeEach } from "vitest";
import { fileIndexMap, loadFileIndex } from "./fileIndex";
import { walkDirectory } from "./ipc";

vi.mock("./ipc", () => ({
  walkDirectory: vi.fn(),
}));

const mockedWalkDirectory = vi.mocked(walkDirectory);

describe("loadFileIndex", () => {
  beforeEach(() => {
    vi.clearAllMocks();
  });

  it("populates fileIndexMap with filtered paths", async () => {
    mockedWalkDirectory.mockResolvedValue([
      { path: "src/main.ts", isDir: false },
      { path: "src/lib/utils.ts", isDir: false },
      { path: "", isDir: false },
      { path: "src/components", isDir: true },
    ]);

    await loadFileIndex("/test/workspace");

    expect(fileIndexMap["/test/workspace"]).toEqual([
      "src/main.ts",
      "src/lib/utils.ts",
      "src/components",
    ]);
  });

  it("handles empty results", async () => {
    mockedWalkDirectory.mockResolvedValue([]);

    await loadFileIndex("/empty/workspace");

    expect(fileIndexMap["/empty/workspace"]).toEqual([]);
  });

  it("handles error and sets empty array", async () => {
    mockedWalkDirectory.mockRejectedValue(new Error("permission denied"));

    await loadFileIndex("/error/workspace");

    expect(fileIndexMap["/error/workspace"]).toEqual([]);
  });
});
