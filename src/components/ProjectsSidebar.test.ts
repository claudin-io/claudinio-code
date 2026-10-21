import { describe, it, expect } from "vitest";
import { readFileSync } from "node:fs";
import { join } from "node:path";

// The projects sidebar must read as one straight left edge. The app's root
// font-size is 13px, so every rem-based Tailwind step lands on a fraction
// (px-3 → 9.75px, gap-2 → 6.5px, w-4 → 13px). Rows built from fractions sit
// on different subpixel phases and their text rasterises a pixel apart, which
// is what made the column look jagged. Both row lists therefore share the
// `.projects-row*` classes below, whose measures are whole pixels.
const root = join(process.cwd(), "src");
const css = readFileSync(join(root, "App.css"), "utf8");
const app = readFileSync(join(root, "App.tsx"), "utf8");

/// Every `<n>px` value in a declaration block, as numbers.
function pxValues(block: string): number[] {
  return [...block.matchAll(/(-?\d+(?:\.\d+)?)px/g)].map((m) => Number(m[1]));
}

function cssRule(selector: string): string {
  const start = css.indexOf(`${selector} {`);
  expect(start, `${selector} must be defined in App.css`).toBeGreaterThan(-1);
  return css.slice(start, css.indexOf("}", start));
}

describe("projects sidebar rows", () => {
  it("keeps every row measure on a whole pixel", () => {
    for (const selector of [".projects-row", ".projects-row-link", ".projects-row-icon", ".projects-row-name", ".projects-row-path"]) {
      const block = cssRule(selector);
      const fractions = pxValues(block).filter((n) => !Number.isInteger(n));
      expect(fractions, `${selector} has fractional px measures: ${fractions.join(", ")}`).toEqual([]);
    }
  });

  it("gives both text lines a fixed line-height, so rows never inherit a fractional one", () => {
    expect(cssRule(".projects-row-name")).toMatch(/line-height:\s*\d+px/);
    expect(cssRule(".projects-row-path")).toMatch(/line-height:\s*\d+px/);
  });

  it("uses the shared row classes for open workspaces and recent projects alike", () => {
    // One occurrence per list: a row styled inline in only one of the two
    // lists drifts from the other the moment either is touched.
    const rows = app.match(/class="projects-row[ "]/g) ?? [];
    expect(rows.length, "both the open and the recent list must use .projects-row").toBe(2);
    expect(app.match(/projects-row-name/g)?.length).toBe(2);
    expect(app.match(/projects-row-path/g)?.length).toBe(2);
  });

  it("never styles a row or its inner button with the fractional rem steps that caused the drift", () => {
    // gap-2 and px-3 are 6.5px and 9.75px under a 13px root font. The inner
    // button is checked too: it carries the icon-to-text gap, which is what
    // moves the text column, so a fractional step there drifts the rows even
    // when the row element itself is styled correctly.
    // One block per row: from the row element to the end of its clickable
    // button, which wraps the icon, the status dot and both text lines.
    const blocks = [...app.matchAll(/<div\s+class="projects-row[\s\S]*?<\/button>/g)].map((m) => m[0]);
    expect(blocks.length, "could not locate the two row blocks").toBe(2);
    // Any rem-based spacing/size utility inside a row is a fraction here, so
    // the check is generic rather than a list of the ones seen so far.
    // `0` is 0px and `px` is 1px whatever the root font-size, so both are safe;
    // every other step is a multiple of the 13px rem and lands on a fraction.
    const remStep = /\b(?:gap|p|px|py|ps|pe|pt|pb|pl|pr|m|mx|my|w|h|space-[xy]|translate-[xy])-(?!0\b|px\b)\d+(?:\.\d+)?\b/;
    for (const block of blocks) {
      for (const cls of block.match(/class="[^"]*"/g) ?? []) {
        expect(cls, `row still uses a fractional rem step: ${cls}`).not.toMatch(remStep);
      }
    }
  });
});
