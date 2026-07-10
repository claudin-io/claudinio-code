import { describe, it, expect, vi } from "vitest";

vi.mock("monaco-editor", () => ({}));
vi.mock("./Icon", () => ({ Icon: () => null }));
vi.mock("../lib/grill-me", () => ({ t: (k: string) => k }));
vi.mock("../lib/ipc", () => ({ readFile: vi.fn(), writeFile: vi.fn() }));
vi.mock("../lib/monacoThemes", () => ({ defineMonacoThemes: vi.fn() }));

import {
  detectLanguage,
  getBasename,
  getRelativePath,
} from "./FileEditorModal";

describe("detectLanguage", () => {
  it("detects TypeScript from .ts", () => {
    expect(detectLanguage("file.ts")).toBe("typescript");
  });

  it("detects TypeScript from .mts", () => {
    expect(detectLanguage("file.mts")).toBe("typescript");
  });

  it("detects TypeScript from .tsx", () => {
    expect(detectLanguage("file.tsx")).toBe("typescript");
  });

  it("detects JavaScript from .js", () => {
    expect(detectLanguage("file.js")).toBe("javascript");
  });

  it("detects JavaScript from .mjs", () => {
    expect(detectLanguage("file.mjs")).toBe("javascript");
  });

  it("detects JavaScript from .jsx", () => {
    expect(detectLanguage("file.jsx")).toBe("javascript");
  });

  it("detects JSON from .json", () => {
    expect(detectLanguage("file.json")).toBe("json");
  });

  it("detects Markdown from .md", () => {
    expect(detectLanguage("file.md")).toBe("markdown");
  });

  it("detects CSS from .css", () => {
    expect(detectLanguage("file.css")).toBe("css");
  });

  it("detects HTML from .html", () => {
    expect(detectLanguage("file.html")).toBe("html");
  });

  it("detects Python from .py", () => {
    expect(detectLanguage("file.py")).toBe("python");
  });

  it("detects Rust from .rs", () => {
    expect(detectLanguage("file.rs")).toBe("rust");
  });

  it("detects Go from .go", () => {
    expect(detectLanguage("file.go")).toBe("go");
  });

  it("falls back to plaintext for unknown extension", () => {
    expect(detectLanguage("file.unknown")).toBe("plaintext");
  });

  it("falls back to plaintext when there is no extension", () => {
    expect(detectLanguage("noext")).toBe("plaintext");
  });
});

describe("getBasename", () => {
  it("extracts basename from a Unix path", () => {
    expect(getBasename("/home/user/file.ts")).toBe("file.ts");
  });

  it("extracts basename from a Windows path with backslashes", () => {
    expect(getBasename("C:\\Users\\test\\file.js")).toBe("file.js");
  });

  it("returns the filename itself when given just a filename", () => {
    expect(getBasename("file.json")).toBe("file.json");
  });

  it("returns empty string for a path ending with a trailing slash", () => {
    expect(getBasename("/home/user/")).toBe("");
  });
});

describe("getRelativePath", () => {
  it("returns path relative to root", () => {
    expect(getRelativePath("/root/sub/file.ts", "/root")).toBe("sub/file.ts");
  });

  it("returns just filename when file is directly under root", () => {
    expect(getRelativePath("/root/file.ts", "/root")).toBe("file.ts");
  });

  it("returns the absolute path when file is outside root", () => {
    expect(getRelativePath("/other/path.ts", "/root")).toBe("/other/path.ts");
  });

  it("handles root path with a trailing slash", () => {
    expect(getRelativePath("/root/sub/file.ts", "/root/")).toBe("sub/file.ts");
  });
});
