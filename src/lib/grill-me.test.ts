import { describe, it, expect, vi } from "vitest";

// jsdom doesn't provide a functional localStorage — stub it so the
// module can be imported without throwing.
vi.stubGlobal(
  "localStorage",
  (() => {
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
  })(),
);

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

    // Solid 1.9 uses MessageChannel for scheduling. In Node.js, MessageChannel
    // events fire in the "Check" phase (same as setImmediate), AFTER timers.
    // The Solid effect fires → loadDict() → .then(setCurrentDict). The number of
    // flush cycles this takes is not fixed: under CI load (parallel workers) it
    // can take many more than a handful, so a fixed loop count flakes. Instead,
    // poll — flush Check phase + microtasks — until the expectation holds.
    async function settle(until?: () => boolean) {
      for (let i = 0; i < 200; i++) {
        if (until?.()) return;
        await new Promise((r) => setImmediate(r));
        await Promise.resolve();
      }
    }

    // Flush any pending effects from the previous test first
    // (vitest caches modules across tests)
    await settle();

    // Force to pt-BR and wait for the effect + import to settle
    mod.setLocale("pt-BR");
    await settle(() => mod.t("greeting") === "Olá");
    expect(mod.t("greeting")).toBe("Olá");

    // Switch to en-US — first load, not cached yet → dynamic import
    mod.setLocale("en-US");
    await settle(() => mod.t("greeting") === "Hello");
    expect(mod.t("greeting")).toBe("Hello");

    // Switch back to pt-BR — second load: dictCache.has("pt-BR") is true → cache hit
    // This exercises the `if (dictCache.has(id)) return dictCache.get(id)!;` branch
    mod.setLocale("pt-BR");
    await settle(() => mod.t("greeting") === "Olá");
    expect(mod.t("greeting")).toBe("Olá");
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

    async function settle(until?: () => boolean) {
      for (let i = 0; i < 200; i++) {
        if (until?.()) return;
        await new Promise((r) => setImmediate(r));
        await Promise.resolve();
      }
    }

    mod.setLocale("pt-BR");
    await settle(() => mod.t("greeting") === "Olá");
    expect(mod.t("greeting")).toBe("Olá");

    mod.setLocale("en-US");
    await settle(() => mod.t("greeting") === "Hello");
    expect(mod.t("greeting")).toBe("Hello");

    mod.setLocale("pt-BR");
    await settle(() => mod.t("greeting") === "Olá");
    expect(mod.t("greeting")).toBe("Olá");
  });
});
