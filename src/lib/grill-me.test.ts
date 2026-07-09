import { describe, it, expect, vi } from "vitest";

// Factory so we can recreate the mock in different scopes
function createLocalStorageMock() {
  const store: Record<string, string> = {};
  return {
    getItem: (key: string) => store[key] ?? null,
    setItem: (key: string, value: string) => {
      store[key] = value;
    },
    removeItem: (key: string) => {
      delete store[key];
    },
    clear: () => {
      Object.keys(store).forEach((k) => delete store[k]);
    },
    get length() {
      return Object.keys(store).length;
    },
    key: (i: number) => Object.keys(store)[i] ?? null,
  };
}

// jsdom doesn't provide a functional localStorage — stub it so the
// module can be imported without throwing.
vi.stubGlobal("localStorage", createLocalStorageMock());

// Mock locale dynamic imports with data that includes function-form entries
// to exercise the branch in t() where typeof val === "function".
vi.mock("./locales/pt-BR", () => ({
  default: {
    "greeting": "Olá",
    "hello.name": "Olá, {0}!",
    "items.count": "Itens: {0} de {1}",
    "farewell": (name: string) => `Tchau, ${name}!`,
  },
}));

vi.mock("./locales/en-US", () => ({
  default: {
    "greeting": "Hello",
    "hello.name": "Hello, {0}!",
  },
}));

// Solid 1.9 schedules effects via MessageChannel, whose callbacks fire in the
// Node "Check" phase — the same phase as setImmediate, and AFTER timers. So we
// flush with setImmediate (not setTimeout/vi.waitFor, which polls in the Timer
// phase and can fail to let the Solid effect run). Each cycle also drains
// microtasks so the loadDict().then(setCurrentDict) handler applies. We poll
// until the expectation holds rather than a fixed count, so it never flakes on
// slow CI; the cap only bounds a genuine hang. grill-me's "latest locale wins"
// guard makes this deterministic even across the shared module state vitest
// reuses between tests in a file.
async function flushUntil(cond: () => boolean, maxCycles = 500) {
  for (let i = 0; i < maxCycles; i++) {
    if (cond()) return;
    await new Promise((r) => setImmediate(r));
    await Promise.resolve();
  }
}

describe("grill-me", () => {
  it("exports FLAGS with correct emoji for each locale", async () => {
    const { FLAGS } = await import("./grill-me");
    expect(FLAGS).toEqual({
      "pt-BR": "🇧🇷",
      "en-US": "🇺🇸",
    });
  });

  it("exports LOCALE_LABELS with correct labels for each locale", async () => {
    const { LOCALE_LABELS } = await import("./grill-me");
    expect(LOCALE_LABELS).toEqual({
      "pt-BR": "PT",
      "en-US": "EN",
    });
  });

  it("t() returns the key when dict is empty for that key", async () => {
    const { t } = await import("./grill-me");
    expect(t("nonexistent.key")).toBe("nonexistent.key");
  });

  it("t() returns the string value when key exists", async () => {
    const { t } = await import("./grill-me");
    expect(t("greeting")).toBe("Olá");
  });

  it("t() interpolates positional args {0}, {1} etc", async () => {
    const { t } = await import("./grill-me");
    expect(t("hello.name", "Mundo")).toBe("Olá, Mundo!");
    expect(t("items.count", "3", "10")).toBe("Itens: 3 de 10");
  });

  it("t() returns function result when val is a function", async () => {
    const { t } = await import("./grill-me");
    expect(t("farewell", "João")).toBe("Tchau, João!");
  });

  it("locale and setLocale are exported and functional", async () => {
    const mod = await import("./grill-me");
    expect(typeof mod.locale).toBe("function");
    expect(typeof mod.setLocale).toBe("function");
    // Should not throw when called
    expect(() => mod.setLocale("en-US")).not.toThrow();
  });

  // ── loadDict cache hit path (lines 49-59) ──────────────────────

  it("loadDict returns cached result when loading the same locale twice", async () => {
    const mod = await import("./grill-me");

    // Await loadDict directly instead of going through setLocale → Solid effect →
    // signal update. That async propagation path is inherently timing-dependent and
    // flakes under CI load; awaiting the promise is deterministic.
    const pt1 = await mod.loadDict("pt-BR");
    expect(pt1["greeting"]).toBe("Olá");

    // First en-US load — not cached yet → dynamic import
    const en = await mod.loadDict("en-US");
    expect(en["greeting"]).toBe("Hello");

    // Second pt-BR load: dictCache.has("pt-BR") is true → cache hit branch
    // (`if (dictCache.has(id)) return dictCache.get(id)!;`). Same object reference
    // proves the cached value was returned rather than re-imported.
    const pt2 = await mod.loadDict("pt-BR");
    expect(pt2).toBe(pt1);
  });

  // ── Proxy get handler: unknown property returns undefined (line 49) ──
  it("Proxy get returns undefined for unknown property access", async () => {
    // __localeProxy is the exported Proxy. Only "locale" and "setLocale" are handled.
    // Any unknown property should trigger `return undefined` (the fallthrough).
    const mod = await import("./grill-me");

    // Access the exported __localeProxy with an unknown key
    const unknownProp = mod.__localeProxy["nonexistentProp" as keyof typeof mod.__localeProxy];
    expect(unknownProp).toBeUndefined();

    // Known properties still work
    expect(typeof mod.__localeProxy.locale).toBe("function");
    expect(typeof mod.__localeProxy.setLocale).toBe("function");
  });

  // ── ensureDictWatcher idempotency (line 80 guard) ──────────────────
  it("ensureDictWatcher is idempotent — second call returns immediately", async () => {
    // ensureDictWatcher uses an `effectStarted` boolean guard.
    // The first call is already made at module init.
    // A second call should hit `if (effectStarted) return;` and do nothing.
    const mod = await import("./grill-me");
    
    // Call it again — should be a no-op since effectStarted is already true.
    // If it tried to create a second effect, it might throw or produce
    // unexpected behavior. Since it's a no-op, locale/setLocale still work.
    expect(() => mod.ensureDictWatcher()).not.toThrow();
    expect(typeof mod.locale).toBe("function");
    expect(typeof mod.setLocale).toBe("function");
  });

  // ── ensureDictWatcher — locale changes still propagate ────────────
  it("ensureDictWatcher locale effect still works after single init", async () => {
    const mod = await import("./grill-me");

    // Warm the dict cache so the effect's loadDict() resolves from cache (a couple
    // of microtasks) rather than a dynamic import whose timing flakes on CI. The
    // effect → signal propagation is then reliably caught by flushUntil.
    await mod.loadDict("pt-BR");
    await mod.loadDict("en-US");

    mod.setLocale("en-US");
    await flushUntil(() => mod.t("greeting") === "Hello");
    expect(mod.t("greeting")).toBe("Hello");

    mod.setLocale("pt-BR");
    await flushUntil(() => mod.t("greeting") === "Olá");
    expect(mod.t("greeting")).toBe("Olá");
  });
});

// ── Coverage gaps: SSR guard, stale-load guards ──────────────────────
//
// These tests cover the remaining branches that the default jsdom / stubbed
// localStorage environment never exercises:
//
//   Line 13  — typeof localStorage !== "undefined" (false)  in createLocaleState
//   Line 21  — typeof localStorage !== "undefined" (false)  in setLocale
//   Line 78  — locale() === initialLocale       (false)  in init .then handler
//   Line 94  — locale() === id                  (false)  in effect .then handler
//
describe("grill-me coverage gaps", () => {
  beforeEach(() => {
    vi.resetModules();
  });

  afterEach(() => {
    // Restore a working localStorage mock (do NOT use unstubAllGlobals — that
    // restores jsdom's broken localStorage, which would fail on next import).
    vi.stubGlobal("localStorage", createLocalStorageMock());
  });

  // ── SSR guard: no localStorage available (lines 13, 21) ──────────
  it("works without localStorage (SSR) — createLocaleState guard", async () => {
    vi.stubGlobal("localStorage", undefined);

    // Use vi.importActual so the top-level code runs fresh with localStorage=undefined
    const mod = await vi.importActual<typeof import("./grill-me")>("./grill-me");

    // Default locale should still be pt-BR (stored ?? "pt-BR" → stored is null)
    expect(mod.locale()).toBe("pt-BR");

    // setLocale must not throw even though localStorage is undefined (line 21 guard)
    expect(() => mod.setLocale("en-US")).not.toThrow();
    expect(mod.locale()).toBe("en-US");

    expect(() => mod.setLocale("pt-BR")).not.toThrow();
    expect(mod.locale()).toBe("pt-BR");
  });

  // ── stored ?? "pt-BR" default when localStorage is present but empty ──
  it("defaults to pt-BR when localStorage has no stored locale", async () => {
    // localStorage mock has no "claudinio_locale" key → stored is null → "pt-BR"
    const mod = await import("./grill-me");
    expect(mod.locale()).toBe("pt-BR");
  });

  // ── initial-load stale guard (line 78) ──────────────────────────────
  //
  // Module init: loadDict(initialLocale).then((d) => {
  //   if (getLocaleState().locale() === initialLocale) setCurrentDict(d);
  // });
  // If locale is changed BEFORE the .then fires, the guard prevents applying
  // a stale dict.
  it("prevents stale initial-load dict from overwriting a newer locale", async () => {
    const mod = await import("./grill-me");

    // Module init: initialLocale = "pt-BR", loadDict("pt-BR") is pending.
    // Change locale before .then fires so guard evaluates to false.
    mod.setLocale("en-US");

    // Flush so the init .then fires with locale() === "en-US" !== "pt-BR"
    // → setCurrentDict is NOT called with pt-BR dict (line 78 false branch).
    await flushUntil(() => mod.t("greeting") === "Hello");

    // en-US dict came from effect (init load was blocked)
    expect(mod.t("greeting")).toBe("Hello");
  });

  // ── effect-load stale guard (line 94) ────────────────────────────────
  //
  // createEffect watches locale(). When it fires:
  //   loadDict(id).then((d) => {
  //     if (getLocaleState().locale() === id) setCurrentDict(d);
  //   });
  it("prevents stale effect-load dict from overwriting a newer locale", async () => {
    const mod = await import("./grill-me");

    await mod.loadDict("pt-BR");

    mod.setLocale("pt-BR");
    await flushUntil(() => mod.t("greeting") === "Olá");
    expect(mod.t("greeting")).toBe("Olá");

    // Switch to en-US — effect fires, starts uncached loadDict("en-US").
    mod.setLocale("en-US");

    // Let Solid schedule the effect body (next Check phase tick)
    await new Promise((r) => setImmediate(r));
    await Promise.resolve();

    // Now switch back to pt-BR (cached) before en-US mock import resolves.
    // Race: en-US .then fires with locale() === "pt-BR" !== "en-US" → BLOCKED (line 94)
    mod.setLocale("pt-BR");
    await flushUntil(() => mod.t("greeting") === "Olá");
    expect(mod.t("greeting")).toBe("Olá");
  });
});
