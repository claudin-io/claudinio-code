import { describe, it, expect } from "vitest";
import { autoAvailable, modeToSend } from "./modeChoice";
import type { JevStatus } from "./ipc";

const jev = (over: Partial<JevStatus>): JevStatus => ({
  enabled: true,
  hasApiKey: false,
  backend: "claudinio",
  route: "on",
  ...over,
});

describe("autoAvailable", () => {
  it("is offered on a fresh session when the router is on", () => {
    expect(autoAvailable(jev({}), true)).toBe(true);
  });

  it("is gone once the session has had its first prompt", () => {
    expect(autoAvailable(jev({}), false)).toBe(false);
  });

  it("is not offered while the router only watches, or is off", () => {
    expect(autoAvailable(jev({ route: "shadow" }), true)).toBe(false);
    expect(autoAvailable(jev({ route: "off" }), true)).toBe(false);
  });

  it("is not offered when Jev is disabled or its status is unknown", () => {
    expect(autoAvailable(jev({ enabled: false }), true)).toBe(false);
    expect(autoAvailable(undefined, true)).toBe(false);
    // A config from before the setting existed has no route at all.
    expect(autoAvailable({ enabled: true, hasApiKey: false, backend: null }, true)).toBe(false);
  });
});

describe("modeToSend", () => {
  it("sends auto instead of the mode when the choice was left to the harness", () => {
    expect(modeToSend("builder", true)).toBe("auto");
    expect(modeToSend("brain", true)).toBe("auto");
  });

  it("sends the mode the user picked otherwise", () => {
    expect(modeToSend("brain", false)).toBe("brain");
    expect(modeToSend("builder", false)).toBe("builder");
  });
});
