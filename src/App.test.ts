import { describe, it, expect, beforeEach, vi } from "vitest";

// App.tsx imports several modules as side-effects that execute
// browser APIs (window.matchMedia, localStorage.getItem) at module
// scope. Mock them to prevent the import cascade from throwing
// before any test runs.
vi.mock("./lib/theme", () => ({}));
vi.mock("./lib/grill-me", () => ({
  t: (key: string) => key,
  locale: () => "en-US" as const,
  setLocale: vi.fn(),
}));
vi.mock("monaco-editor", () => ({}));
vi.mock("./lib/monacoThemes", () => ({ defineMonacoThemes: vi.fn() }));

// The jsdom environment in this project has a broken localStorage (empty
// Object). Replace it with a proper Map-backed implementation so all
// localStorage-dependent functions work correctly.
const lsStore = new Map<string, string>();
const lsMock: Storage = {
  getItem: (k: string) => lsStore.get(k) ?? null,
  setItem: (k: string, v: string) => { lsStore.set(k, v); },
  removeItem: (k: string) => { lsStore.delete(k); },
  clear: () => lsStore.clear(),
  get length() { return lsStore.size; },
  key: (i: number) => [...lsStore.keys()][i] ?? null,
};
Object.defineProperty(window, "localStorage", { value: lsMock, writable: false });

import {
  loadRecent,
  saveRecent,
  loadOpenWorkspaces,
  saveOpenWorkspaces,
  addRecent,
} from "./App";

beforeEach(() => {
  lsStore.clear();
});

// ── loadRecent ──────────────────────────────────────────────────────

describe("loadRecent", () => {
  it("returns empty array when localStorage has no key", () => {
    expect(loadRecent()).toEqual([]);
  });

  it("parses valid JSON array", () => {
    localStorage.setItem("claudinio_recent_projects", '["/a","/b"]');
    expect(loadRecent()).toEqual(["/a", "/b"]);
  });

  it("returns empty array on invalid JSON (corrupted)", () => {
    localStorage.setItem("claudinio_recent_projects", "not-json");
    expect(loadRecent()).toEqual([]);
  });
});

// ── saveRecent ──────────────────────────────────────────────────────

describe("saveRecent", () => {
  it("stores JSON stringified array in localStorage", () => {
    saveRecent(["/x", "/y"]);
    expect(localStorage.getItem("claudinio_recent_projects")).toBe('["/x","/y"]');
  });
});

// ── loadOpenWorkspaces ──────────────────────────────────────────────

describe("loadOpenWorkspaces", () => {
  it("returns empty array when no key", () => {
    expect(loadOpenWorkspaces()).toEqual([]);
  });

  it("parses valid JSON array", () => {
    localStorage.setItem("claudinio_open_workspaces", '["/ws1","/ws2"]');
    expect(loadOpenWorkspaces()).toEqual(["/ws1", "/ws2"]);
  });

  it("returns empty array on invalid JSON", () => {
    localStorage.setItem("claudinio_open_workspaces", "{bad json");
    expect(loadOpenWorkspaces()).toEqual([]);
  });
});

// ── saveOpenWorkspaces ──────────────────────────────────────────────

describe("saveOpenWorkspaces", () => {
  it("stores JSON stringified array in localStorage", () => {
    saveOpenWorkspaces(["/alpha", "/beta"]);
    expect(localStorage.getItem("claudinio_open_workspaces")).toBe(
      '["/alpha","/beta"]',
    );
  });
});

// ── addRecent ───────────────────────────────────────────────────────

describe("addRecent", () => {
  it("adds path to front of list", () => {
    const projects: string[] = ["/old"];
    const getter = () => projects;
    const setter = (v: string[]) => {
      projects.length = 0;
      projects.push(...v);
    };

    addRecent(getter, setter, "/new");

    expect(projects).toEqual(["/new", "/old"]);
    expect(localStorage.getItem("claudinio_recent_projects")).toBe(
      '["/new","/old"]',
    );
  });

  it("deduplicates existing path (moves it to front)", () => {
    const projects: string[] = ["/a", "/b", "/c"];
    const getter = () => projects;
    const setter = (v: string[]) => {
      projects.length = 0;
      projects.push(...v);
    };

    addRecent(getter, setter, "/b");

    expect(projects).toEqual(["/b", "/a", "/c"]);
    expect(localStorage.getItem("claudinio_recent_projects")).toBe(
      '["/b","/a","/c"]',
    );
  });

  it("caps list at 10 items (discards oldest)", () => {
    const initial = Array.from({ length: 10 }, (_, i) => `/path-${i}`);
    const projects: string[] = [...initial];
    const getter = () => projects;
    const setter = (v: string[]) => {
      projects.length = 0;
      projects.push(...v);
    };

    addRecent(getter, setter, "/new-path");

    expect(projects).toHaveLength(10);
    // New item should be at front
    expect(projects[0]).toBe("/new-path");
    // The oldest (last in the original list, /path-9) should be discarded
    expect(projects).not.toContain("/path-9");
    expect(projects).toContain("/path-0");
  });
});
