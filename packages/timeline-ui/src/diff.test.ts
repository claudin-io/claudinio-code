import { describe, it, expect } from "vitest";
import {
  CONTEXT_LINES,
  MAX_DIFFABLE_LINES,
  MAX_LINES,
  diffLines,
  hunkHeader,
  type Diff,
} from "./diff";

/// The diff as a unified-diff-shaped string, which is far easier to read in a failure
/// than a tree of objects.
function render(diff: Diff): string {
  return diff.hunks
    .map((hunk) =>
      [
        hunkHeader(hunk),
        ...hunk.lines.map((line) => {
          const sign = line.kind === "added" ? "+" : line.kind === "removed" ? "-" : " ";
          return `${sign}${line.text}`;
        }),
      ].join("\n"),
    )
    .join("\n");
}

const lines = (...items: string[]) => items.join("\n") + "\n";

describe("diffLines", () => {
  it("finds nothing to show when nothing changed", () => {
    const diff = diffLines("a\nb\n", "a\nb\n");
    expect(diff.hunks).toEqual([]);
    expect(diff.added).toBe(0);
    expect(diff.removed).toBe(0);
  });

  it("shows a changed line as a removal then an addition", () => {
    const diff = diffLines(lines("a", "b", "c"), lines("a", "B", "c"));

    expect(render(diff)).toBe(["@@ -1,3 +1,3 @@", " a", "-b", "+B", " c"].join("\n"));
    expect(diff.added).toBe(1);
    expect(diff.removed).toBe(1);
  });

  /// Removals before additions at the same position, so a replaced line reads "this
  /// became that" rather than the reverse.
  it("puts the removal before the addition", () => {
    const diff = diffLines("old\n", "new\n");
    expect(diff.hunks[0].lines.map((l) => l.kind)).toEqual(["removed", "added"]);
  });

  it("counts an insertion without a removal", () => {
    const diff = diffLines(lines("a", "c"), lines("a", "b", "c"));

    expect(render(diff)).toBe(["@@ -1,2 +1,3 @@", " a", "+b", " c"].join("\n"));
    expect(diff.added).toBe(1);
    expect(diff.removed).toBe(0);
  });

  it("counts a deletion without an addition", () => {
    const diff = diffLines(lines("a", "b", "c"), lines("a", "c"));

    expect(diff.removed).toBe(1);
    expect(diff.added).toBe(0);
  });

  it("handles an empty original", () => {
    const diff = diffLines("", lines("a", "b"));
    expect(diff.added).toBe(2);
    expect(diff.removed).toBe(0);
  });

  it("handles everything being deleted", () => {
    const diff = diffLines(lines("a", "b"), "");
    expect(diff.removed).toBe(2);
    expect(diff.added).toBe(0);
  });

  /// `"a\n".split("\n")` is `["a", ""]`, and that phantom line shows up as a spurious
  /// change at the end of every properly terminated file.
  it("does not invent a change from a trailing newline", () => {
    expect(diffLines("a\n", "a\n").hunks).toEqual([]);
    expect(diffLines("a", "a").hunks).toEqual([]);
  });

  /// But a file that gained or lost its final newline genuinely changed, and pretending
  /// otherwise hides a real edit.
  it("still sees a line added after a trailing newline", () => {
    const diff = diffLines("a\n", "a\nb\n");
    expect(diff.added).toBe(1);
  });

  it("treats an empty string as no lines rather than one", () => {
    expect(diffLines("", "").hunks).toEqual([]);
    expect(diffLines("", "a\n").added).toBe(1);
  });

  // ── line numbers ─────────────────────────────────────────────────────────

  /// The numbers are how someone finds the change in the file afterwards. An added line
  /// has no place in the old file and a removed one has none in the new, and showing a
  /// number for either would be a lie.
  it("numbers lines against the side they exist in", () => {
    const diff = diffLines(lines("a", "b", "c"), lines("a", "x", "b", "c"));
    const added = diff.hunks[0].lines.find((l) => l.kind === "added");

    expect(added?.newNumber).toBe(2);
    expect(added?.oldNumber).toBeUndefined();
  });

  it("keeps old and new numbering in step through a change", () => {
    const diff = diffLines(lines("a", "b", "c", "d"), lines("a", "c", "d"));
    const context = diff.hunks[0].lines.filter((l) => l.kind === "context");

    expect(context.map((l) => [l.oldNumber, l.newNumber])).toEqual([
      [1, 1],
      [3, 2],
      [4, 3],
    ]);
  });

  // ── hunks and context ────────────────────────────────────────────────────

  it("drops the untouched middle of a long file", () => {
    const before = Array.from({ length: 60 }, (_, i) => `line ${i}`);
    const after = [...before];
    after[5] = "changed early";
    after[55] = "changed late";

    const diff = diffLines(before.join("\n") + "\n", after.join("\n") + "\n");

    expect(diff.hunks).toHaveLength(2);
    // Each hunk is the change plus context either side, not sixty lines.
    for (const hunk of diff.hunks) {
      expect(hunk.lines.length).toBeLessThanOrEqual(CONTEXT_LINES * 2 + 2);
    }
  });

  /// Two edits close together must read as one hunk. Two hunks sharing the same lines
  /// is how a diff starts looking like more changed than did.
  it("merges changes whose context overlaps", () => {
    const before = Array.from({ length: 30 }, (_, i) => `line ${i}`);
    const after = [...before];
    after[10] = "one";
    after[12] = "two";

    const diff = diffLines(before.join("\n") + "\n", after.join("\n") + "\n");
    expect(diff.hunks).toHaveLength(1);
  });

  it("gives each hunk a header with both ranges", () => {
    const before = Array.from({ length: 40 }, (_, i) => `line ${i}`);
    const after = [...before];
    after[20] = "changed";

    const diff = diffLines(before.join("\n") + "\n", after.join("\n") + "\n");
    expect(hunkHeader(diff.hunks[0])).toMatch(/^@@ -\d+,\d+ \+\d+,\d+ @@$/);
  });

  it("does not run off the start or the end for a change at the edge", () => {
    const first = diffLines(lines("a", "b", "c"), lines("A", "b", "c"));
    expect(first.hunks[0].oldStart).toBe(1);

    const last = diffLines(lines("a", "b", "c"), lines("a", "b", "C"));
    const lastLine = last.hunks[0].lines[last.hunks[0].lines.length - 1];
    expect(lastLine.kind).toBe("added");
  });

  // ── limits ───────────────────────────────────────────────────────────────

  /// A phone showing the first screen of a thousand-line change has to say so.
  /// Stopping quietly is how someone approves the part they could see.
  it("says how much it cut rather than stopping quietly", () => {
    const before = Array.from({ length: 1000 }, (_, i) => `line ${i}`);
    const after = before.map((line, i) => (i % 2 === 0 ? `${line} changed` : line));

    const diff = diffLines(before.join("\n") + "\n", after.join("\n") + "\n");
    const shown = diff.hunks.reduce((n, hunk) => n + hunk.lines.length, 0);

    expect(shown).toBeLessThanOrEqual(MAX_LINES);
    expect(diff.truncated).toBeDefined();
    expect(diff.truncated!.lines).toBeGreaterThan(0);
  });

  /// The counts must be of the whole change, not of what fitted. A diff that reported
  /// "12 added" because that is all it drew would understate what is about to happen.
  it("counts the whole change even when it only shows part", () => {
    const before = Array.from({ length: 1000 }, (_, i) => `line ${i}`);
    const after = before.map((line) => `${line} changed`);

    const diff = diffLines(before.join("\n") + "\n", after.join("\n") + "\n");
    expect(diff.added).toBe(1000);
    expect(diff.removed).toBe(1000);
  });

  it("does not report truncation when everything fitted", () => {
    const diff = diffLines(lines("a", "b"), lines("a", "c"));
    expect(diff.truncated).toBeUndefined();
  });

  /// An LCS over two five-thousand-line texts is twenty-five million cells, which on a
  /// phone is a frozen tab. The fallback is honest about being one.
  it("falls back to a whole-block replacement when the input is too large", () => {
    const size = MAX_DIFFABLE_LINES + 1;
    const before = Array.from({ length: size }, (_, i) => `line ${i}`).join("\n") + "\n";
    const after = before.replace("line 0", "changed");

    const diff = diffLines(before, after);

    expect(diff.wholeBlock).toBe(true);
    expect(diff.added).toBe(size);
    expect(diff.removed).toBe(size);
    expect(diff.truncated).toBeDefined();
  });

  it("does not claim a whole-block fallback for an ordinary diff", () => {
    expect(diffLines(lines("a"), lines("b")).wholeBlock).toBeUndefined();
  });

  /// Identical inputs short-circuit before the size check, so an unchanged large file
  /// is free rather than a whole-block replacement of itself.
  it("shows nothing for two identical large files", () => {
    const text = Array.from({ length: MAX_DIFFABLE_LINES + 10 }, (_, i) => `l${i}`).join("\n");
    const diff = diffLines(text, text);

    expect(diff.hunks).toEqual([]);
    expect(diff.wholeBlock).toBeUndefined();
  });

  // ── the shapes real edits take ───────────────────────────────────────────

  /// What an `edit_file` tool call actually carries: two snippets, not two files. The
  /// transcript has `old_string` and `new_string` and no diff at all.
  it("diffs the snippets an edit_file call carries", () => {
    const diff = diffLines(
      '  const timeout = 30;\n  return fetch(url, { timeout });\n',
      '  const timeout = 60;\n  return fetch(url, { timeout, retries: 2 });\n',
    );

    expect(diff.added).toBe(2);
    expect(diff.removed).toBe(2);
    expect(render(diff)).toContain("-  const timeout = 30;");
    expect(render(diff)).toContain("+  const timeout = 60;");
  });

  it("keeps leading whitespace, which is often the whole change", () => {
    const diff = diffLines("if (x) {\n", "  if (x) {\n");
    const added = diff.hunks[0].lines.find((l) => l.kind === "added");

    expect(added?.text).toBe("  if (x) {");
  });

  it("handles windows line endings without treating every line as changed", () => {
    // The carriage returns end up in the text, which is honest: they *are* a
    // difference, and hiding it would hide a real change to the file.
    const diff = diffLines("a\r\nb\r\n", "a\r\nB\r\n");
    expect(diff.added).toBe(1);
    expect(diff.removed).toBe(1);
  });
});
