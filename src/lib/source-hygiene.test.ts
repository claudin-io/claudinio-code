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

  /// `packages/` is shared with the web peer, and `apps/web` *is* the web peer.
  /// Neither has Tauri, and neither has `src/`.
  ///
  /// This is checked rather than left to the build for a specific reason: every
  /// `@tauri-apps/*` module is mocked in `test-setup.ts`, so an import that would
  /// fail in production passes silently in tests. Without this the failure surfaces
  /// at deploy, and the cheapest-looking fix at that point is to copy a file — which
  /// is how a sanitizer ends up existing twice and diverging.
  it("nothing in packages/ or apps/ imports Tauri", () => {
    const offenders: string[] = [];
    const roots = ["packages", "apps"].map((dir) => join(process.cwd(), dir));
    for (const file of roots.flatMap((root) => walk(root))) {
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
