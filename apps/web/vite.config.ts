import { resolve } from "node:path";
import { defineConfig } from "vite";
import solid from "vite-plugin-solid";

export default defineConfig({
  plugins: [solid()],

  resolve: {
    alias: {
      // The generated protocol bindings, shared rather than copied.
      //
      // `sas.ts` above all: the device and the browser must derive the same three
      // words, or pairing asks people to compare strings that can never agree. A
      // second copy of that word list breaks the day one of them is edited, and it
      // breaks by looking like an attack.
      "@claudinio/protocol": resolve(
        __dirname,
        "../../src-tauri/crates/claudinio-protocol/bindings",
      ),
    },
  },

  build: {
    // No inline assets. The CSP has no `'unsafe-inline'`, so anything Vite decided to
    // inline as a data: URI in a <style> or <script> would be blocked at runtime —
    // and the failure would be a blank page rather than a build error.
    assetsInlineLimit: 0,
    // For the service worker's precache list, which is read out of it by
    // `vite.sw.config.ts` after this build finishes. A worker that globbed `dist/assets`
    // instead would precache whatever an interrupted build left behind.
    manifest: true,
    // A source map for a page that holds a session key is a source map that describes
    // how to use it. The bundle is small and the tests are where debugging happens.
    sourcemap: false,
    target: "es2022",
  },

  server: {
    // Only for `pnpm dev` against a local relay. Production is a static bundle behind
    // whatever serves app.claudin.io.
    port: 1430,
    strictPort: true,
  },
});
