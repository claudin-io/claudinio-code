/// A second build, for the service worker alone.
///
/// Three things force it to be separate rather than another entry in `vite.config.ts`.
///
/// 1. **It has to be a classic script.** Module service workers are still not supported
///    everywhere — and a worker that fails to register in one browser turns "offered,
///    never required" into "works on my phone". The app is `format: "es"`; one Rollup
///    build cannot emit both.
/// 2. **Its filename must be exactly `sw.js`, at the root.** A worker's scope is the
///    directory it is served from, so a hashed name under `/assets/` could only ever
///    control `/assets/`.
/// 3. **It needs the app's asset list, which only exists after the app is built.** So
///    this reads `dist/.vite/manifest.json` and injects it. Ordering is in the `build`
///    script: the app first, then this, with `emptyOutDir` off.
///
/// The alternative was `vite-plugin-pwa`, which does all of the above and a great deal
/// more. What it also does is generate the worker, and the worker is the one file on this
/// origin that can intercept every request forever. Forty lines that can be read in full
/// beat a generated one that can't.

import { createHash } from "node:crypto";
import { readFileSync, rmSync } from "node:fs";
import { resolve } from "node:path";
import { defineConfig } from "vite";

const dist = resolve(__dirname, "dist");

/// This build's assets, from Vite's manifest.
///
/// The manifest lists every emitted chunk and stylesheet with its hashed name. Reading it
/// rather than globbing `dist/assets` means the list is what the app actually references:
/// a stale file left behind by an interrupted build would be globbed in, precached, and
/// then never used.
function assets(): string[] {
  const path = resolve(dist, ".vite/manifest.json");
  let raw: string;
  try {
    raw = readFileSync(path, "utf8");
  } catch {
    throw new Error(
      `No ${path}. The app has to be built first — run \`pnpm build\`, which does both in order.`,
    );
  }

  const manifest = JSON.parse(raw) as Record<string, { file?: string; css?: string[] }>;
  const files = new Set<string>();
  for (const entry of Object.values(manifest)) {
    if (entry.file) files.add(`/${entry.file}`);
    for (const css of entry.css ?? []) files.add(`/${css}`);
  }

  // Sorted so the cache name below depends on the content of the build and not on the
  // order Vite happened to write its manifest in.
  return [...files].sort();
}

const list = assets();

/// The cache name, derived from the list.
///
/// Every name in it ends in a content hash, so this changes when any asset does and
/// stays put when none did. That is what makes `activate`'s "delete everything but mine"
/// both correct and cheap: a redeploy of identical bytes does not evict a working cache.
const cacheName = `claudinio-remote-${createHash("sha256").update(list.join("\n")).digest("hex").slice(0, 12)}`;

export default defineConfig({
  plugins: [
    {
      // The manifest is input to this build, not something to serve. Removing it once it
      // has been read leaves `dist/` as exactly the set of files that get deployed —
      // which is also what makes `prova-real/pwa.mjs` able to check that every precached
      // URL is a file that is really there.
      name: "claudinio-drop-the-build-manifest",
      closeBundle() {
        rmSync(resolve(dist, ".vite"), { recursive: true, force: true });
      },
    },
  ],

  define: {
    __CACHE_NAME__: JSON.stringify(cacheName),
    __ASSETS__: JSON.stringify(list),
  },

  build: {
    outDir: "dist",
    // The app's output is already there and this must land beside it.
    emptyOutDir: false,
    sourcemap: false,
    target: "es2022",
    lib: {
      entry: resolve(__dirname, "src/sw.ts"),
      formats: ["iife"],
      name: "claudinioServiceWorker",
      fileName: () => "sw.js",
    },
  },
});
