import { describe, it, expect } from "vitest";
import enUS from "./locales/en-US";
import ptBR from "./locales/pt-BR";

function isStringOrFunction(val: unknown): boolean {
  return typeof val === "function" || typeof val === "string";
}

describe("locale dictionaries", () => {
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
});
