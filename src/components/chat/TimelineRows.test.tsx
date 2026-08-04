import { describe, it, expect, vi, afterEach } from "vitest";
import { render } from "solid-js/web";

// TimelineRows pulls in DiffViewer, which pulls in monaco — it needs browser
// APIs jsdom does not implement. Same stub the other viewer tests use.
vi.mock("monaco-editor", () => ({
  editor: { create: vi.fn(), createDiffEditor: vi.fn(), defineTheme: vi.fn() },
}));

import { QualityRow } from "./TimelineRows";
import type { QualityVerdictData } from "../../lib/ipc";

let dispose: (() => void) | undefined;
let host: HTMLDivElement | undefined;

afterEach(() => {
  dispose?.();
  host?.remove();
  dispose = undefined;
  host = undefined;
});

function mount(quality: QualityVerdictData): HTMLDivElement {
  host = document.createElement("div");
  document.body.appendChild(host);
  dispose = render(() => <QualityRow quality={quality} />, host);
  return host;
}

const verdict = (over: Partial<QualityVerdictData> = {}): QualityVerdictData => ({
  pass: true,
  summary: "all good",
  trigger: "tool",
  layers: [
    { layer: "tests", stack: "rust", status: "pass", summary: "12 passed, 0 failed" },
  ],
  ...over,
});

describe("QualityRow", () => {
  it("shows a passing run as passed", () => {
    const el = mount(verdict());
    expect(el.textContent).toContain("Checks passed");
    expect(el.textContent).toContain("12 passed, 0 failed");
  });

  it("shows a failing run as failed", () => {
    const el = mount(
      verdict({
        pass: false,
        layers: [
          { layer: "tests", stack: "js", status: "fail", summary: "9 passed, 3 failed" },
        ],
      }),
    );
    expect(el.textContent).toContain("Checks failed");
    expect(el.textContent).toContain("9 passed, 3 failed");
  });

  it("marks a harness-enforced run so the user knows who verified it", () => {
    const el = mount(verdict({ trigger: "harness" }));
    expect(el.textContent).toContain("verified by the harness");
  });

  it("does not credit the harness when the agent asked for the run", () => {
    const el = mount(verdict({ trigger: "tool" }));
    expect(el.textContent).not.toContain("verified by the harness");
  });

  it("distinguishes a layer that could not run from one that passed", () => {
    // "we did not measure this" must never read like a green check — that is
    // the whole difference between evidence and the appearance of evidence.
    const el = mount(
      verdict({
        layers: [
          { layer: "tests", stack: "rust", status: "pass", summary: "3 passed" },
          {
            layer: "coverage",
            stack: "rust",
            status: "unavailable",
            summary: "coverage tooling is not installed",
          },
        ],
      }),
    );
    expect(el.textContent).toContain("coverage tooling is not installed");
    const marks = Array.from(el.querySelectorAll("span")).map((s) => s.textContent?.trim());
    expect(marks).toContain("✓");
    expect(marks).toContain("–");
  });

  it("lists every layer and stack that ran", () => {
    const el = mount(
      verdict({
        layers: [
          { layer: "tests", stack: "rust", status: "pass", summary: "a" },
          { layer: "tests", stack: "js", status: "pass", summary: "b" },
          { layer: "coverage", stack: "js", status: "fail", summary: "c" },
        ],
      }),
    );
    expect(el.textContent).toContain("tests (rust)");
    expect(el.textContent).toContain("tests (js)");
    expect(el.textContent).toContain("coverage (js)");
  });
});
