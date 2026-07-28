import { describe, expect, it } from "vitest";
import { baseName } from "./path";

describe("baseName", () => {
  it("returns the last segment of a POSIX path", () => {
    expect(baseName("/home/me/my-app")).toBe("my-app");
  });

  // The regression this helper exists for: splitting on "/" alone left
  // Windows paths untouched, so the sidebar showed the whole path.
  it("returns the last segment of a Windows path", () => {
    expect(baseName("C:\\Users\\me\\my-app")).toBe("my-app");
  });

  it("handles mixed separators", () => {
    expect(baseName("C:/Users\\me/my-app")).toBe("my-app");
  });

  it("ignores trailing separators", () => {
    expect(baseName("/home/me/my-app/")).toBe("my-app");
    expect(baseName("C:\\Users\\me\\my-app\\")).toBe("my-app");
  });

  it("handles a drive root", () => {
    expect(baseName("C:\\")).toBe("C:");
  });

  it("returns bare names unchanged", () => {
    expect(baseName("my-app")).toBe("my-app");
  });

  it("returns an empty path unchanged rather than empty-stringing it", () => {
    expect(baseName("")).toBe("");
  });
});
