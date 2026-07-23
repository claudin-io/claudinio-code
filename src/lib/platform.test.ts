import { describe, it, expect, vi } from "vitest";
import { platform, revealLabel } from "./platform";

afterEach(() => {
  vi.unstubAllGlobals();
});

describe("platform()", () => {
  it("returns 'mac' when userAgent includes 'Mac'", () => {
    vi.stubGlobal("navigator", { userAgent: "Mozilla/5.0 (Macintosh; Intel Mac OS X 10_15_7) AppleWebKit/537.36" });
    expect(platform()).toBe("mac");
  });

  it("returns 'win' when userAgent includes 'Win'", () => {
    vi.stubGlobal("navigator", { userAgent: "Mozilla/5.0 (Windows NT 10.0; Win64; x64) AppleWebKit/537.36" });
    expect(platform()).toBe("win");
  });

  it("returns 'linux' for other userAgents", () => {
    vi.stubGlobal("navigator", { userAgent: "Mozilla/5.0 (X11; Linux x86_64) AppleWebKit/537.36" });
    expect(platform()).toBe("linux");
  });
});

describe("revealLabel()", () => {
  it("returns 'Reveal in Finder' on Mac", () => {
    vi.stubGlobal("navigator", { userAgent: "Macintosh" });
    expect(revealLabel()).toBe("Reveal in Finder");
  });

  it("returns 'Show in Explorer' on Windows", () => {
    vi.stubGlobal("navigator", { userAgent: "Windows NT 10.0" });
    expect(revealLabel()).toBe("Show in Explorer");
  });

  it("returns 'Open in File Manager' on Linux", () => {
    vi.stubGlobal("navigator", { userAgent: "Linux x86_64" });
    expect(revealLabel()).toBe("Open in File Manager");
  });
});

