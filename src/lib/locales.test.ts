import { describe, it, expect, vi } from "vitest";

// jsdom doesn't provide a functional localStorage — stub it so
// grill-me.ts can be imported without throwing.
vi.stubGlobal("localStorage", {
  getItem: () => null,
  setItem: () => {},
  removeItem: () => {},
  clear: () => {},
  get length() { return 0; },
  key: () => null,
});

// Locale dictionaries are safe as static imports — they only
// `import type` from grill-me, which is erased at compile time.
import enUS from "./locales/en-US";
import ptBR from "./locales/pt-BR";
import esES from "./locales/es-ES";
import frFR from "./locales/fr-FR";
import deDE from "./locales/de-DE";
import itIT from "./locales/it-IT";
import ruRU from "./locales/ru-RU";
import trTR from "./locales/tr-TR";
import arSA from "./locales/ar-SA";
import hiIN from "./locales/hi-IN";
import bnBD from "./locales/bn-BD";
import urPK from "./locales/ur-PK";
import zhCN from "./locales/zh-CN";
import jaJP from "./locales/ja-JP";
import koKR from "./locales/ko-KR";
import viVN from "./locales/vi-VN";
import idID from "./locales/id-ID";
import ptPT from "./locales/pt-PT";

// resolveLocale is a plain function and does not depend on Solid signals,
// so we can import it once at the top of the describe block. We use a
// dynamic import so the localStorage stub is in place before grill-me.ts
// evaluates its module scope.
let resolveLocale: (typeof import("./grill-me"))["resolveLocale"];

beforeAll(async () => {
  resolveLocale = (await import("./grill-me")).resolveLocale;
});

function isStringOrFunction(val: unknown): boolean {
  return typeof val === "function" || typeof val === "string";
}

describe("locale dictionaries — en-US & pt-BR parity", () => {
  describe("key parity", () => {
    it("both exports have the same number of keys", () => {
      const enKeys = Object.keys(enUS);
      const ptKeys = Object.keys(ptBR);
      expect(ptKeys.length).toBe(enKeys.length);
    });

    it("every key in en-US exists in pt-BR", () => {
      for (const key of Object.keys(enUS)) {
        expect(ptBR).toHaveProperty(key);
      }
    });

    it("every key in pt-BR exists in en-US", () => {
      for (const key of Object.keys(ptBR)) {
        expect(enUS).toHaveProperty(key);
      }
    });
  });

  describe("value validity", () => {
    it("all en-US values are strings or functions", () => {
      for (const [key, val] of Object.entries(enUS)) {
        expect(
          isStringOrFunction(val),
          `en-US key "${key}" is ${typeof val}${typeof val === "string" ? ` (length ${val.length})` : ""}`,
        ).toBe(true);
      }
    });

    it("all pt-BR values are strings or functions", () => {
      for (const [key, val] of Object.entries(ptBR)) {
        expect(
          isStringOrFunction(val),
          `pt-BR key "${key}" is ${typeof val}${typeof val === "string" ? ` (length ${val.length})` : ""}`,
        ).toBe(true);
      }
    });
  });

  describe("empty locales (16 new — awaiting translation)", () => {
    const emptyLocales: Record<string, Record<string, unknown>> = {
      "es-ES": esES, "fr-FR": frFR, "de-DE": deDE, "it-IT": itIT,
      "ru-RU": ruRU, "tr-TR": trTR, "ar-SA": arSA, "hi-IN": hiIN,
      "bn-BD": bnBD, "ur-PK": urPK, "zh-CN": zhCN, "ja-JP": jaJP,
      "ko-KR": koKR, "vi-VN": viVN, "id-ID": idID, "pt-PT": ptPT,
    };

    it("all 16 empty locale files export empty dictionaries", () => {
      for (const [code, dict] of Object.entries(emptyLocales)) {
        expect(dict, `${code} dict should be defined`).toBeDefined();
        expect(Object.keys(dict).length, `${code} should have 0 keys`).toBe(0);
      }
    });
  });

  describe("resolveLocale", () => {
    it("returns exact match when locale is supported", () => {
      expect(resolveLocale("pt-BR")).toBe("pt-BR");
      expect(resolveLocale("en-US")).toBe("en-US");
      expect(resolveLocale("ja-JP")).toBe("ja-JP");
    });

    it("matches by language prefix when exact match fails", () => {
      expect(resolveLocale("pt")).toBe("pt-BR"); // first pt-* in SUPPORTED_LOCALES
      expect(resolveLocale("en")).toBe("en-US");
      expect(resolveLocale("es")).toBe("es-ES");
    });

    it("falls back to en-US for unrecognized locales", () => {
      expect(resolveLocale("zz-ZZ")).toBe("en-US");
      expect(resolveLocale("xx")).toBe("en-US");
      expect(resolveLocale("")).toBe("en-US");
    });
  });
});
