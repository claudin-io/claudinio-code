import { describe, it, expect, vi, afterEach } from "vitest";

describe("theme", () => {
  afterEach(() => {
    vi.unstubAllGlobals();
    vi.resetModules();
  });

  it("is a function", async () => {
    vi.stubGlobal("matchMedia", vi.fn((query: string) => ({
      matches: false,
      media: query,
      onchange: null,
      addListener: vi.fn(),
      removeListener: vi.fn(),
      addEventListener: vi.fn(),
      removeEventListener: vi.fn(),
      dispatchEvent: vi.fn(),
    })));
    const { theme } = await import("./theme");
    expect(typeof theme).toBe("function");
  });

  it("returns 'dark' when prefers-color-scheme is not light (matches: false)", async () => {
    vi.stubGlobal("matchMedia", vi.fn((query: string) => ({
      matches: false,
      media: query,
      onchange: null,
      addListener: vi.fn(),
      removeListener: vi.fn(),
      addEventListener: vi.fn(),
      removeEventListener: vi.fn(),
      dispatchEvent: vi.fn(),
    })));
    const { theme } = await import("./theme");
    expect(theme()).toBe("dark");
  });

  it("returns 'light' when prefers-color-scheme is light (matches: true)", async () => {
    vi.stubGlobal("matchMedia", vi.fn((query: string) => ({
      matches: true,
      media: query,
      onchange: null,
      addListener: vi.fn(),
      removeListener: vi.fn(),
      addEventListener: vi.fn(),
      removeEventListener: vi.fn(),
      dispatchEvent: vi.fn(),
    })));
    const { theme } = await import("./theme");
    expect(theme()).toBe("light");
    expect(document.documentElement.dataset.theme).toBe("light");
  });

  it("registers a change event listener on matchMedia", async () => {
    const addEventListenerSpy = vi.fn();
    vi.stubGlobal("matchMedia", vi.fn((query: string) => ({
      matches: false,
      media: query,
      onchange: null,
      addListener: vi.fn(),
      removeListener: vi.fn(),
      addEventListener: addEventListenerSpy,
      removeEventListener: vi.fn(),
      dispatchEvent: vi.fn(),
    })));
    const { theme } = await import("./theme");
    expect(addEventListenerSpy).toHaveBeenCalledWith("change", expect.any(Function));
    expect(theme()).toBe("dark");
  });

  it("update function reacts to change event — toggles between dark/light", async () => {
    let storedHandler: (() => void) | null = null;
    let matchesValue = false;

    vi.stubGlobal("matchMedia", vi.fn((query: string) => ({
      get matches() { return matchesValue; },
      media: query,
      onchange: null,
      addListener: vi.fn(),
      removeListener: vi.fn(),
      addEventListener: vi.fn((_event: string, handler: () => void) => {
        storedHandler = handler;
      }),
      removeEventListener: vi.fn(),
      dispatchEvent: vi.fn(),
    })));

    const { theme } = await import("./theme");
    // Initial: matches=false → dark
    expect(theme()).toBe("dark");
    expect(document.documentElement.dataset.theme).toBe("dark");

    // Change event fires → matches becomes true → light
    matchesValue = true;
    storedHandler!();
    expect(theme()).toBe("light");
    expect(document.documentElement.dataset.theme).toBe("light");

    // Change event fires back → matches becomes false → dark
    matchesValue = false;
    storedHandler!();
    expect(theme()).toBe("dark");
    expect(document.documentElement.dataset.theme).toBe("dark");
  });
});
