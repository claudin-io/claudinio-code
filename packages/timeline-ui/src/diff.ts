/// Line diffs, computed here because the transcript does not carry them.
///
/// # Why this exists
///
/// §8 phase 3 ends with "a diff is legible and approvable on a phone held in one
/// hand", and §7 of the threat model rests on a human reading a diff before
/// approving. On a phone a diff that cannot be read produces reflexive approval,
/// which is worse than no remote approval at all — so this is a load-bearing piece
/// rather than presentation.
///
/// Two things forced it to be new code rather than a move:
///
/// - **The transcript has no diff in it.** An `edit_file` tool call carries
///   `old_string`, `new_string` and `path`. The unified diff the desktop shows is
///   computed live and never persisted, so a peer replaying a transcript has to
///   compute one.
/// - **The desktop's `DiffViewer` is Monaco.** A full editor is the wrong answer for
///   a read-only view on a phone, and it would take a 48 kB bundle past two
///   megabytes. The desktop keeps Monaco; this is what the browser and the timeline
///   share.
///
/// Pure, framework-free and separately tested, because a diff that is subtly wrong
/// is worse than no diff: it invites approval of something other than what will
/// happen.

export type LineKind = "context" | "added" | "removed";

export interface DiffLine {
  kind: LineKind;
  text: string;
  /// 1-based, in the old text. Absent on an added line.
  oldNumber?: number;
  /// 1-based, in the new text. Absent on a removed line.
  newNumber?: number;
}

export interface Hunk {
  oldStart: number;
  oldCount: number;
  newStart: number;
  newCount: number;
  lines: DiffLine[];
}

export interface Diff {
  hunks: Hunk[];
  added: number;
  removed: number;
  /// Set when the diff was cut short, with what was left. A phone showing the first
  /// screen of a thousand-line change has to say so — silently stopping is how
  /// someone approves the part they could see.
  truncated?: { hunks: number; lines: number };
  /// Set when the inputs were too large to diff and the whole block is shown as a
  /// replacement instead. Honest about being a fallback rather than pretending to be
  /// a minimal diff.
  wholeBlock?: true;
}

/// Lines of context either side of a change.
export const CONTEXT_LINES = 3;

/// How many diff lines to produce before cutting.
///
/// Sized for a phone: past this nobody is reading, and a longer render is a longer
/// scroll between the change and the approve button.
export const MAX_LINES = 600;

/// Above this many lines on either side, the quadratic table is abandoned.
///
/// An LCS over two 5000-line texts is 25 million cells, which on a phone is a frozen
/// tab. The fallback is honest — the whole block as a replacement — rather than a
/// diff that took ten seconds.
export const MAX_DIFFABLE_LINES = 1500;

export function diffLines(oldText: string, newText: string): Diff {
  const oldLines = splitLines(oldText);
  const newLines = splitLines(newText);

  if (oldText === newText) {
    return { hunks: [], added: 0, removed: 0 };
  }

  if (oldLines.length > MAX_DIFFABLE_LINES || newLines.length > MAX_DIFFABLE_LINES) {
    return wholeBlockDiff(oldLines, newLines);
  }

  const lines = walk(oldLines, newLines);
  const hunks = intoHunks(lines);
  return summarise(hunks);
}

/// Split into lines, without inventing a trailing empty one.
///
/// `"a\n".split("\n")` is `["a", ""]`, and that phantom line shows up as a spurious
/// change at the end of every file that ends properly. An empty string is genuinely
/// zero lines rather than one.
function splitLines(text: string): string[] {
  if (text === "") return [];
  const lines = text.split("\n");
  if (lines.length > 0 && lines[lines.length - 1] === "") lines.pop();
  return lines;
}

/// Longest common subsequence, walked backwards into a line list.
function walk(oldLines: string[], newLines: string[]): DiffLine[] {
  const n = oldLines.length;
  const m = newLines.length;

  // lcs[i][j] = length of the longest common subsequence of oldLines[i..] and
  // newLines[j..]. One row longer in each direction so the edges need no special case.
  const lcs: number[][] = Array.from({ length: n + 1 }, () => new Array<number>(m + 1).fill(0));
  for (let i = n - 1; i >= 0; i--) {
    for (let j = m - 1; j >= 0; j--) {
      lcs[i][j] =
        oldLines[i] === newLines[j] ? lcs[i + 1][j + 1] + 1 : Math.max(lcs[i + 1][j], lcs[i][j + 1]);
    }
  }

  const out: DiffLine[] = [];
  let i = 0;
  let j = 0;
  while (i < n && j < m) {
    if (oldLines[i] === newLines[j]) {
      out.push({ kind: "context", text: oldLines[i], oldNumber: i + 1, newNumber: j + 1 });
      i++;
      j++;
    } else if (lcs[i + 1][j] >= lcs[i][j + 1]) {
      // Removals before additions at the same position, so a replaced line reads as
      // "this became that" rather than the other way round.
      out.push({ kind: "removed", text: oldLines[i], oldNumber: i + 1 });
      i++;
    } else {
      out.push({ kind: "added", text: newLines[j], newNumber: j + 1 });
      j++;
    }
  }
  while (i < n) out.push({ kind: "removed", text: oldLines[i], oldNumber: ++i });
  while (j < m) out.push({ kind: "added", text: newLines[j], newNumber: ++j });
  return out;
}

/// Group changes into hunks with context, dropping the untouched middle.
function intoHunks(lines: DiffLine[]): Hunk[] {
  const changed = lines
    .map((line, index) => (line.kind === "context" ? -1 : index))
    .filter((index) => index >= 0);
  if (changed.length === 0) return [];

  // Merge runs of change whose context windows touch, so two edits three lines apart
  // read as one hunk rather than two with the same lines in both.
  const ranges: [number, number][] = [];
  for (const index of changed) {
    const from = Math.max(0, index - CONTEXT_LINES);
    const to = Math.min(lines.length - 1, index + CONTEXT_LINES);
    const previous = ranges[ranges.length - 1];
    if (previous && from <= previous[1] + 1) {
      previous[1] = Math.max(previous[1], to);
    } else {
      ranges.push([from, to]);
    }
  }

  return ranges.map(([from, to]) => {
    const slice = lines.slice(from, to + 1);
    const oldNumbers = slice.map((l) => l.oldNumber).filter((n): n is number => n !== undefined);
    const newNumbers = slice.map((l) => l.newNumber).filter((n): n is number => n !== undefined);
    return {
      oldStart: oldNumbers[0] ?? 0,
      oldCount: oldNumbers.length,
      newStart: newNumbers[0] ?? 0,
      newCount: newNumbers.length,
      lines: slice,
    };
  });
}

/// Cut to `MAX_LINES`, reporting what was left rather than stopping quietly.
function summarise(hunks: Hunk[]): Diff {
  let added = 0;
  let removed = 0;
  for (const hunk of hunks) {
    for (const line of hunk.lines) {
      if (line.kind === "added") added++;
      if (line.kind === "removed") removed++;
    }
  }

  let budget = MAX_LINES;
  const kept: Hunk[] = [];
  for (const hunk of hunks) {
    if (budget <= 0) break;
    if (hunk.lines.length <= budget) {
      kept.push(hunk);
      budget -= hunk.lines.length;
    } else {
      kept.push({ ...hunk, lines: hunk.lines.slice(0, budget) });
      budget = 0;
    }
  }

  const shownLines = kept.reduce((n, hunk) => n + hunk.lines.length, 0);
  const allLines = hunks.reduce((n, hunk) => n + hunk.lines.length, 0);
  const diff: Diff = { hunks: kept, added, removed };
  if (shownLines < allLines) {
    diff.truncated = { hunks: hunks.length - kept.length, lines: allLines - shownLines };
  }
  return diff;
}

/// Everything removed, everything added — for inputs too large to diff.
function wholeBlockDiff(oldLines: string[], newLines: string[]): Diff {
  const lines: DiffLine[] = [
    ...oldLines.map((text, i) => ({ kind: "removed" as const, text, oldNumber: i + 1 })),
    ...newLines.map((text, i) => ({ kind: "added" as const, text, newNumber: i + 1 })),
  ];
  const hunk: Hunk = {
    oldStart: oldLines.length > 0 ? 1 : 0,
    oldCount: oldLines.length,
    newStart: newLines.length > 0 ? 1 : 0,
    newCount: newLines.length,
    lines,
  };
  const diff = summarise([hunk]);
  diff.wholeBlock = true;
  return diff;
}

/// A unified-diff header, for a hunk.
///
/// The familiar `@@ -a,b +c,d @@`. Worth keeping the standard form: it is what someone
/// pasting a hunk into a terminal or an issue will expect, and inventing a prettier
/// one buys nothing.
export function hunkHeader(hunk: Hunk): string {
  return `@@ -${hunk.oldStart},${hunk.oldCount} +${hunk.newStart},${hunk.newCount} @@`;
}
