import { describe, it, expect, vi, afterEach } from "vitest";

let __resetState: () => void;

/** Helper: stub matchMedia + localStorage so theme.ts doesn't throw */
function stubGlobals(matchesLight = false, storedTheme: string | null = null) {
  const store: Record<string, string> = {};
  if (storedTheme !== undefined && storedTheme !== null) {
    store["claudinio_theme"] = storedTheme;
  }

  vi.stubGlobal("matchMedia", vi.fn(() => ({
    matches: matchesLight,
    media: "(prefers-color-scheme: light)",
    addEventListener: vi.fn(),
    removeEventListener: vi.fn(),
  })));

  vi.stubGlobal("localStorage", {
    getItem: vi.fn((k: string) => store[k] ?? null),
    setItem: vi.fn((k: string, v: string) => { store[k] = v; }),
  });

  return { store };
}

describe("theme", () => {
  afterEach(() => {
    if (__resetState) __resetState();
    vi.unstubAllGlobals();
    vi.resetModules();
  });

  it("is a function", async () => {
    stubGlobals(false);
    const mod = await import("./theme");
    __resetState = mod.__resetState;
    expect(typeof mod.theme).toBe("function");
  });

  it("returns 'dark' when prefers-color-scheme is not light (matches: false)", async () => {
    stubGlobals(false);
    const mod = await import("./theme");
    __resetState = mod.__resetState;
    expect(mod.theme()).toBe("dark");
  });

  it("returns 'light' when prefers-color-scheme is light (matches: true)", async () => {
    stubGlobals(true);
    const mod = await import("./theme");
    __resetState = mod.__resetState;
    expect(mod.theme()).toBe("light");
    expect(document.documentElement.dataset.theme).toBe("light");
  });

  it("returns 'dark' when window is undefined (SSR — skips browser code)", async () => {
    // Patch window/globals via stubGlobals-like approach within the test scope
    // then restore. NOTE: We cannot use stubGlobals here because localStorage
    // stubs need to be set up before importing.
    const origWindow = (globalThis as any).window;
    const origMatchMedia = (globalThis as any).matchMedia;
    const origLocalStorage = (globalThis as any).localStorage;
    (globalThis as any).window = undefined;
    (globalThis as any).matchMedia = undefined;
    (globalThis as any).localStorage = undefined;
    const mod = await import("./theme") as any;
    __resetState = mod.__resetState;
    expect(mod.theme()).toBe("dark");
    (globalThis as any).window = origWindow;
    (globalThis as any).matchMedia = origMatchMedia;
    (globalThis as any).localStorage = origLocalStorage;
  });

  it("reacts to matchMedia change event — toggles between dark/light", async () => {
    let listener: ((e: { matches: boolean }) => void) | null = null;
    let matchesLight = false;

    vi.stubGlobal("matchMedia", vi.fn(() => ({
      get matches() { return matchesLight; },
      media: "(prefers-color-scheme: light)",
      addEventListener: vi.fn((_e: string, h: () => void) => { listener = h; }),
      removeEventListener: vi.fn(),
    })));
    vi.stubGlobal("localStorage", { getItem: vi.fn(() => null), setItem: vi.fn() });

    const _mod = await import("./theme") as any;
    __resetState = (_mod as any).__resetState;
    expect((_mod as any).theme()).toBe("dark");
    expect(document.documentElement.dataset.theme).toBe("dark");

    // Simulate OS switching to light
    matchesLight = true;
    (listener as any)({ matches: true });
    expect((_mod as any).theme()).toBe("light");
    expect(document.documentElement.dataset.theme).toBe("light");

    // Simulate OS switching back to dark
    matchesLight = false;
    (listener as any)({ matches: false });
    expect((_mod as any).theme()).toBe("dark");
    expect(document.documentElement.dataset.theme).toBe("dark");
  });

  // ── New tests: preference(), cycleTheme(), setThemePreference() ──

  it("preference() defaults to 'system' with no stored value", async () => {
    stubGlobals(false);
    const mod = await import("./theme") as any;
    __resetState = mod.__resetState;
    expect(mod.preference()).toBe("system");
  });

  it("preference() reads 'dark' from localStorage", async () => {
    stubGlobals(true, "dark"); // OS says light, but stored says dark
    const mod = await import("./theme") as any;
    __resetState = mod.__resetState;
    expect(mod.preference()).toBe("dark");
    expect(mod.theme()).toBe("dark");
    expect(document.documentElement.dataset.theme).toBe("dark");
  });

  it("preference() reads 'light' from localStorage", async () => {
    stubGlobals(false, "light");
    const mod = await import("./theme") as any;
    __resetState = mod.__resetState;
    expect(mod.preference()).toBe("light");
    expect(mod.theme()).toBe("light");
    expect(document.documentElement.dataset.theme).toBe("light");
  });

  it("preference() reads 'sepia' from localStorage", async () => {
    stubGlobals(false, "sepia");
    const mod = await import("./theme") as any;
    __resetState = mod.__resetState;
    expect(mod.preference()).toBe("sepia");
    expect(mod.theme()).toBe("sepia");
    expect(document.documentElement.dataset.theme).toBe("sepia");
  });

  it("cycleTheme() cycles system → dark → light → sepia → system", async () => {
    stubGlobals(false);
    const mod = await import("./theme") as any;
    __resetState = mod.__resetState;

    mod.setThemePreference("system");
    expect(mod.preference()).toBe("system");

    mod.cycleTheme();
    expect(mod.preference()).toBe("dark");

    mod.cycleTheme();
    expect(mod.preference()).toBe("light");

    mod.cycleTheme();
    expect(mod.preference()).toBe("sepia");

    mod.cycleTheme();
    expect(mod.preference()).toBe("system");
  });

  it("setThemePreference persists to localStorage", async () => {
    const { store } = stubGlobals(false);
    const mod = await import("./theme") as any;
    __resetState = mod.__resetState;

    mod.setThemePreference("sepia");
    expect(mod.preference()).toBe("sepia");
    expect(store["claudinio_theme"]).toBe("sepia");

    mod.setThemePreference("dark");
    expect(mod.preference()).toBe("dark");
    expect(store["claudinio_theme"]).toBe("dark");
  });

  it("setThemePreference('system') removes stored override", async () => {
    const { store } = stubGlobals(false, "dark");
    const mod = await import("./theme") as any;
    __resetState = mod.__resetState;

    expect(mod.preference()).toBe("dark");
    expect(mod.theme()).toBe("dark");

    mod.setThemePreference("system");
    expect(mod.preference()).toBe("system");
    expect(store["claudinio_theme"]).toBe("system");
    expect(mod.theme()).toBe("dark"); // matches: false → dark
  });

  it("resolvedTheme() returns the same value as theme()", async () => {
    stubGlobals(false, "light");
    const mod = await import("./theme") as any;
    __resetState = mod.__resetState;
    expect(mod.resolvedTheme()).toBe(mod.theme());
    expect(mod.resolvedTheme()).toBe("light");
  });
});
