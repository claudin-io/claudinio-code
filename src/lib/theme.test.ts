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

  it("returns 'claudinio' (default dark) when prefers-color-scheme is not light", async () => {
    stubGlobals(false);
    const mod = await import("./theme");
    __resetState = mod.__resetState;
    expect(mod.theme()).toBe("claudinio");
  });

  it("returns 'claudinio-light' when prefers-color-scheme is light (matches: true)", async () => {
    stubGlobals(true);
    const mod = await import("./theme");
    __resetState = mod.__resetState;
    expect(mod.theme()).toBe("claudinio-light");
    expect(document.documentElement.dataset.theme).toBe("claudinio-light");
  });

  it("returns 'claudinio' when window is undefined (SSR — skips browser code)", async () => {
    const origWindow = (globalThis as any).window;
    const origMatchMedia = (globalThis as any).matchMedia;
    const origLocalStorage = (globalThis as any).localStorage;
    (globalThis as any).window = undefined;
    (globalThis as any).matchMedia = undefined;
    (globalThis as any).localStorage = undefined;
    const mod = await import("./theme") as any;
    __resetState = mod.__resetState;
    expect(mod.theme()).toBe("claudinio");
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
    expect((_mod as any).theme()).toBe("claudinio");
    expect(document.documentElement.dataset.theme).toBe("claudinio");

    // Simulate OS switching to light
    matchesLight = true;
    (listener as any)({ matches: true });
    expect((_mod as any).theme()).toBe("claudinio-light");
    expect(document.documentElement.dataset.theme).toBe("claudinio-light");

    // Simulate OS switching back to dark
    matchesLight = false;
    (listener as any)({ matches: false });
    expect((_mod as any).theme()).toBe("claudinio");
    expect(document.documentElement.dataset.theme).toBe("claudinio");
  });

  // ── Legacy migration ──

  it("migrates legacy 'dark' stored value to 'claudinio'", async () => {
    stubGlobals(false, "dark"); // old format
    const mod = await import("./theme") as any;
    __resetState = mod.__resetState;
    expect(mod.preference()).toBe("claudinio");
    expect(mod.theme()).toBe("claudinio");
  });

  it("migrates legacy 'light' stored value to 'claudinio-light'", async () => {
    stubGlobals(true, "light"); // old format
    const mod = await import("./theme") as any;
    __resetState = mod.__resetState;
    expect(mod.preference()).toBe("claudinio-light");
    expect(mod.theme()).toBe("claudinio-light");
  });

  it("migrates legacy 'sepia' stored value to 'claudinio-sepia'", async () => {
    stubGlobals(false, "sepia"); // old format
    const mod = await import("./theme") as any;
    __resetState = mod.__resetState;
    expect(mod.preference()).toBe("claudinio-sepia");
    expect(mod.theme()).toBe("claudinio-sepia");
    expect(document.documentElement.dataset.theme).toBe("claudinio-sepia");
  });

  // ── New theme IDs ──

  it("reads 'dracula' from localStorage and resolves correctly", async () => {
    stubGlobals(false, "dracula");
    const mod = await import("./theme") as any;
    __resetState = mod.__resetState;
    expect(mod.preference()).toBe("dracula");
    expect(mod.theme()).toBe("dracula");
    expect(document.documentElement.dataset.theme).toBe("dracula");
  });

  it("reads 'nord' from localStorage and resolves correctly", async () => {
    stubGlobals(true, "nord");
    const mod = await import("./theme") as any;
    __resetState = mod.__resetState;
    expect(mod.preference()).toBe("nord");
    expect(mod.theme()).toBe("nord");
  });

  it("reads 'tokyo-night' from localStorage and resolves correctly", async () => {
    stubGlobals(false, "tokyo-night");
    const mod = await import("./theme") as any;
    __resetState = mod.__resetState;
    expect(mod.preference()).toBe("tokyo-night");
    expect(mod.theme()).toBe("tokyo-night");
  });

  it("reads 'everforest' from localStorage and resolves correctly", async () => {
    stubGlobals(false, "everforest");
    const mod = await import("./theme") as any;
    __resetState = mod.__resetState;
    expect(mod.preference()).toBe("everforest");
    expect(mod.theme()).toBe("everforest");
  });

  // ── Preference defaults ──

  it("preference() defaults to 'system' with no stored value", async () => {
    stubGlobals(false);
    const mod = await import("./theme") as any;
    __resetState = mod.__resetState;
    expect(mod.preference()).toBe("system");
  });

  it("setThemePreference persists new themes to localStorage", async () => {
    const { store } = stubGlobals(false);
    const mod = await import("./theme") as any;
    __resetState = mod.__resetState;

    mod.setThemePreference("catppuccin");
    expect(mod.preference()).toBe("catppuccin");
    expect(store["claudinio_theme"]).toBe("catppuccin");
    expect(mod.theme()).toBe("catppuccin");

    mod.setThemePreference("gruvbox-light");
    expect(mod.preference()).toBe("gruvbox-light");
    expect(store["claudinio_theme"]).toBe("gruvbox-light");
    expect(mod.theme()).toBe("gruvbox-light");
  });

  it("cycleTheme() cycles system → claudinio → claudinio-light → claudinio-sepia → system", async () => {
    stubGlobals(false);
    const mod = await import("./theme") as any;
    __resetState = mod.__resetState;

    mod.setThemePreference("system");
    expect(mod.preference()).toBe("system");

    mod.cycleTheme();
    expect(mod.preference()).toBe("claudinio");

    mod.cycleTheme();
    expect(mod.preference()).toBe("claudinio-light");

    mod.cycleTheme();
    expect(mod.preference()).toBe("claudinio-sepia");

    mod.cycleTheme();
    expect(mod.preference()).toBe("system");
  });

  it("setThemePreference persists to localStorage", async () => {
    const { store } = stubGlobals(false);
    const mod = await import("./theme") as any;
    __resetState = mod.__resetState;

    mod.setThemePreference("claudinio-sepia");
    expect(mod.preference()).toBe("claudinio-sepia");
    expect(store["claudinio_theme"]).toBe("claudinio-sepia");

    mod.setThemePreference("claudinio");
    expect(mod.preference()).toBe("claudinio");
    expect(store["claudinio_theme"]).toBe("claudinio");
  });

  it("setThemePreference('system') removes stored override", async () => {
    const { store } = stubGlobals(false, "dracula");
    const mod = await import("./theme") as any;
    __resetState = mod.__resetState;

    expect(mod.preference()).toBe("dracula");
    expect(mod.theme()).toBe("dracula");

    mod.setThemePreference("system");
    expect(mod.preference()).toBe("system");
    expect(store["claudinio_theme"]).toBe("system");
    expect(mod.theme()).toBe("claudinio"); // matches: false → claudinio
  });

  it("resolvedTheme() returns the same value as theme()", async () => {
    stubGlobals(true, "nord");
    const mod = await import("./theme") as any;
    __resetState = mod.__resetState;
    expect(mod.resolvedTheme()).toBe(mod.theme());
    expect(mod.resolvedTheme()).toBe("nord");
  });

  // ── ALL_THEMES and themeMetadata ──

  it("ALL_THEMES contains all 15 theme IDs", async () => {
    stubGlobals(false);
    const mod = await import("./theme") as any;
    __resetState = mod.__resetState;
    expect(mod.ALL_THEMES.length).toBe(15);
    expect(mod.ALL_THEMES).toContain("claudinio");
    expect(mod.ALL_THEMES).toContain("dracula");
    expect(mod.ALL_THEMES).toContain("everforest");
  });

  it("themeMetadata has entries for all 15 themes with a label and previewColors", async () => {
    stubGlobals(false);
    const mod = await import("./theme") as any;
    __resetState = mod.__resetState;

    for (const id of mod.ALL_THEMES) {
      const meta = mod.themeMetadata[id];
      expect(meta).toBeDefined();
      expect(typeof meta.label).toBe("string");
      expect(meta.label.length).toBeGreaterThan(0);
      expect(meta.category).toMatch(/^(dark|light)$/);
      expect(meta.previewColors.length).toBe(5);
    }
  });

  it("resolvePreference('system', true) returns 'claudinio' (dark)", async () => {
    stubGlobals(false);
    const mod = await import("./theme") as any;
    __resetState = mod.__resetState;
    expect(mod.resolvePreference("system", true)).toBe("claudinio");
    expect(mod.resolvePreference("system", false)).toBe("claudinio-light");
    expect(mod.resolvePreference("dracula", true)).toBe("dracula");
    expect(mod.resolvePreference("everforest", false)).toBe("everforest");
  });
});
