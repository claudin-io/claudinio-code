import { describe, it, expect } from "vitest";
import { autoAvailable, modeToSend, routerOn } from "./modeChoice";
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
    // On is the default, so "on" alone says nothing about whether there is a
    // Jev to ask: with no credential there is nobody to leave the choice to.
    expect(autoAvailable(jev({ backend: null }), true)).toBe(false);
    // A config from before the setting existed has no route at all.
    expect(autoAvailable({ enabled: true, hasApiKey: false, backend: null }, true)).toBe(false);
  });
});

describe("modeToSend", () => {
  it("sends auto instead of the mode when the choice was left to the harness", () => {
    expect(modeToSend("builder", true, jev({}))).toBe("auto");
    expect(modeToSend("brain", true, jev({}))).toBe("auto");
  });

  it("sends the mode the user picked otherwise", () => {
    expect(modeToSend("brain", false, jev({}))).toBe("brain");
    expect(modeToSend("builder", false, jev({}))).toBe("builder");
  });

  it("sends the mode when there is no router to leave the choice to", () => {
    expect(modeToSend("builder", true, jev({ route: "shadow" }))).toBe("builder");
    expect(modeToSend("brain", true, jev({ enabled: false }))).toBe("brain");
    expect(modeToSend("builder", true, undefined)).toBe("builder");
    expect(routerOn(jev({}))).toBe(true);
  });
});
