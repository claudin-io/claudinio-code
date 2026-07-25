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

  /// `packages/` is shared with the web peer at `app.claudin.io`, which has no
  /// Tauri and no `src/`. An import of either compiles fine here and breaks only
  /// over there — and at that point the cheapest-looking fix is to copy the file,
  /// which is how a sanitizer ends up existing twice and diverging.
  ///
  /// The rule is stated in the package README. It is checked here because a
  /// README does not fail a build.
  it("nothing in packages/ reaches into the app", () => {
    const offenders: string[] = [];
    for (const file of walk(join(process.cwd(), "packages"))) {
      const body = readFileSync(file, "utf8");
      // Covers `from "…"` and `import("…")`, which is every way a module
      // specifier names something outside the file.
      for (const [, spec] of body.matchAll(/(?:from|import)\s*\(?\s*["']([^"']+)["']/g)) {
        if (spec.startsWith("@tauri-apps") || spec.includes("../src/")) {
          offenders.push(`${file} imports ${spec}`);
        }
      }
    }
    expect(offenders, offenders.join("\n")).toEqual([]);
  });
});
