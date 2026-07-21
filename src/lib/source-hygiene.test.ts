import { describe, it, expect } from "vitest";
import { readFileSync, readdirSync, statSync } from "node:fs";
import { join } from "node:path";

// A NUL byte anywhere in a source file makes git treat the whole file as
// binary: diffs render as "Bin <n> -> <m> bytes", code review tooling can't
// show the change, and `git show`/blame stop being useful. ChatPanel.tsx once
// carried a stray NUL as a cache-key delimiter; this guard keeps it out.
function walk(dir: string, acc: string[] = []): string[] {
  for (const entry of readdirSync(dir)) {
    if (entry === "node_modules" || entry === "dist") continue;
    const p = join(dir, entry);
    const s = statSync(p);
    if (s.isDirectory()) walk(p, acc);
    else if (/\.(ts|tsx|css)$/.test(entry)) acc.push(p);
  }
  return acc;
}

describe("source hygiene", () => {
  it("no source file contains a NUL byte", () => {
    const offenders: string[] = [];
    for (const file of walk(join(process.cwd(), "src"))) {
      if (readFileSync(file).includes(0)) offenders.push(file);
    }
    expect(offenders, `NUL byte(s) found in: ${offenders.join(", ")}`).toEqual([]);
  });
});
