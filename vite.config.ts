/// <reference types="vitest" />
import { defineConfig } from "vite";
import solid from "vite-plugin-solid";
import tailwindcss from "@tailwindcss/vite";

// @ts-expect-error process is a nodejs global
const host = process.env.TAURI_DEV_HOST;
// @ts-expect-error process is a nodejs global
const isTest = process.env.VITEST === "true" || process.env.NODE_ENV === "test";

// https://vite.dev/config/
export default defineConfig(async () => ({
  // Under Vitest, disable Solid's HMR runtime. It injects an `@solid-refresh`
  // import that vite-plugin-solid resolves via fileURLToPath("file:///@solid-refresh").
  // On POSIX that yields "/@solid-refresh" and is harmless, but on Windows it is
  // not a valid file URL (no drive letter) and throws, so every JSX test suite
  // fails to load. HMR is meaningless in tests anyway.
  plugins: [solid(isTest ? { hot: false } : {}), tailwindcss()],

  optimizeDeps: {
    // Monaco editor loads workers via `new Worker(new URL(...))` at runtime.
    // Pre-bundling these workers breaks them, so exclude Monaco entirely.
    exclude: ["monaco-editor"],
  },

  // Vite options tailored for Tauri development and only applied in `tauri dev` or `tauri build`
  //
  // 1. prevent Vite from obscuring rust errors
  clearScreen: false,
  build: {
    chunkSizeWarningLimit: 8000,
  },
  // 2. tauri expects a fixed port, fail if that port is not available
  server: {
    port: 1420,
    strictPort: true,
    host: host || false,
    hmr: host
      ? {
          protocol: "ws",
          host,
          port: 1421,
        }
      : undefined,
    watch: {
      // 3. tell Vite to ignore watching `src-tauri` and SQLite db files
      ignored: ["**/src-tauri/**", "**/.claudinio_index.db*"],
    },
  },

  // https://vitest.dev/config/
  test: {
    environment: "jsdom",
    globals: true,
    include: ["src/**/*.test.{ts,tsx}"],
    setupFiles: ["src/test-setup.ts"],
  },
}));
