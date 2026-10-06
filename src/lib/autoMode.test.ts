import { describe, it, expect } from "vitest";
import { createRoot, createSignal } from "solid-js";
import { createAutoMode } from "./autoMode";
import type { JevStatus, SessionMode } from "./ipc";

const ROUTER_ON: JevStatus = { enabled: true, hasApiKey: false, backend: "claudinio", route: "on" };

/** A conversation and its mode control, wired the way the chat panel wires them. */
function chat(settings: { jev: JevStatus | undefined } = { jev: ROUTER_ON }) {
  const { jev } = settings;
  return createRoot(() => {
    const [messages, setMessages] = createSignal<string[]>([]);
    const [mode, setMode] = createSignal<SessionMode>("builder");
    let release: (() => void) | undefined;
    const auto = createAutoMode({
      mode,
      sessionIsFresh: () => messages().length === 0,
      // Held back until `settle()` when a test wants the read to be slow.
      loadJev: () =>
        new Promise((resolve) => {
          release = () => resolve(jev);
        }),
    });
    const settle = async () => {
      release?.();
      await Promise.resolve();
      await Promise.resolve();
    };
    return { auto, setMessages, setMode, settle };
  });
}

async function armed(settings?: { jev: JevStatus | undefined }) {
  const c = chat(settings);
  const arming = c.auto.arm();
  await c.settle();
  await arming;
  return c;
}

describe("createAutoMode", () => {
  it("starts a new conversation on Auto when the router is on", async () => {
    const { auto } = await armed();
    expect(auto.offered()).toBe(true);
    expect(auto.active()).toBe(true);
  });

  // The bug this module was extracted to prevent: the prompt is added to the
  // conversation before the request is built, so anything that asks "is this
  // conversation still fresh?" at that point gets "no" and sends "builder".
  it("sends auto for the first prompt even after that prompt is in the conversation", async () => {
    const { auto, setMessages } = await armed();
    setMessages(["add sharing to sessions"]);
    expect(auto.take()).toBe("auto");
  });

  it("is spent by one prompt", async () => {
    const { auto, setMessages } = await armed();
    expect(auto.take()).toBe("auto");
    setMessages(["first"]);
    expect(auto.take()).toBe("builder");
    expect(auto.active()).toBe(false);
    expect(auto.offered()).toBe(false);
  });

  it("sends the mode the user picked by hand", async () => {
    const { auto, setMode } = await armed();
    auto.disarm();
    setMode("brain");
    expect(auto.active()).toBe(false);
    expect(auto.take()).toBe("brain");
    // Still on offer for a fresh session: they can go back to it.
    expect(auto.offered()).toBe(true);
  });

  // The control is three choices until the first prompt: Brain, Builder, and
  // back to Auto, in any order. Picking Brain used to remove the Auto button.
  it("can be picked again after a mode was picked by hand", async () => {
    const { auto, setMode } = await armed();
    auto.disarm();
    setMode("brain");
    expect(auto.offered()).toBe(true);
    expect(auto.active()).toBe(false);
    auto.select();
    expect(auto.active()).toBe(true);
    expect(auto.take()).toBe("auto");
  });

  it("keeps a mode picked while the settings were still being read", async () => {
    const c = chat();
    const arming = c.auto.arm();
    c.auto.disarm();
    await c.settle();
    await arming;
    expect(c.auto.active()).toBe(false);
    // Still on offer: the click chose a mode, it did not switch the router off.
    expect(c.auto.offered()).toBe(true);
    expect(c.auto.take()).toBe("builder");
  });

  it("never sends auto when the router is off, watching, or Jev is disabled", async () => {
    for (const jev of [
      { ...ROUTER_ON, route: "off" as const },
      { ...ROUTER_ON, route: "shadow" as const },
      { ...ROUTER_ON, enabled: false },
      undefined,
    ]) {
      const { auto } = await armed({ jev });
      expect(auto.offered()).toBe(false);
      expect(auto.take()).toBe("builder");
    }
  });

  it("does not show Auto as selected on a conversation that already has prompts", async () => {
    const { auto, setMessages } = await armed();
    setMessages(["an older prompt"]);
    expect(auto.active()).toBe(false);
    expect(auto.offered()).toBe(false);
  });

  it("falls back to no Auto when the settings cannot be read", async () => {
    const c = createRoot(() => {
      const [mode] = createSignal<SessionMode>("builder");
      return createAutoMode({
        mode,
        sessionIsFresh: () => true,
        loadJev: () => Promise.reject(new Error("backend unavailable")),
      });
    });
    await c.arm();
    expect(c.active()).toBe(false);
    expect(c.take()).toBe("builder");
  });
});
