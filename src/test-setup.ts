/// <reference types="vitest/globals" />
/**
 * Vitest setup file: mocks Tauri modules so tests can run in jsdom.
 * 
 * All `@tauri-apps/*` modules are replaced with minimal stubs that throw
 * when called unless the test explicitly mocks them via `vi.mock`.
 */

// ── Global browser API polyfills ─────────────────────────────────
// jsdom does not implement ResizeObserver. Popover.tsx uses it to
// track its content size, and any test rendering a component that
// contains a Popover will trigger an uncaught exception without this.
vi.stubGlobal(
  "ResizeObserver",
  vi.fn(function MockResizeObserver(callback: ResizeObserverCallback) {
    let observed = false;
    return {
      observe: vi.fn(() => {
        if (observed) return;
        observed = true;
        queueMicrotask(() => {
          callback(
            [{ contentRect: { width: 280, height: 160 } } as ResizeObserverEntry],
            null as unknown as ResizeObserver,
          );
        });
      }),
      disconnect: vi.fn(),
      unobserve: vi.fn(),
    };
  }),
);

// ── @tauri-apps/api/core ───────────────────────────────────────────
vi.mock("@tauri-apps/api/core", () => ({
  invoke: vi.fn().mockRejectedValue(new Error("invoke not mocked")),
  Channel: vi.fn().mockImplementation(function () {
    // Use a closure variable to avoid the duplicate key issue
    let _onmessage: unknown = null;
    return {
      get onmessage() {
        return _onmessage;
      },
      set onmessage(fn: unknown) {
        _onmessage = fn;
      },
    };
  }),
}));

// ── @tauri-apps/api/event ──────────────────────────────────────────
vi.mock("@tauri-apps/api/event", () => ({
  listen: vi.fn().mockResolvedValue(vi.fn()),
}));

// ── @tauri-apps/api/window ─────────────────────────────────────────
vi.mock("@tauri-apps/api/window", () => ({
  getCurrentWindow: vi.fn().mockReturnValue({
    onDragDropEvent: vi.fn().mockResolvedValue(vi.fn()),
  }),
}));

// ── @tauri-apps/plugin-dialog ──────────────────────────────────────
vi.mock("@tauri-apps/plugin-dialog", () => ({
  open: vi.fn().mockRejectedValue(new Error("open not mocked")),
}));

// ── @tauri-apps/plugin-updater ─────────────────────────────────────
vi.mock("@tauri-apps/plugin-updater", () => ({
  check: vi.fn().mockResolvedValue(null),
}));

// ── @tauri-apps/plugin-process ─────────────────────────────────────
vi.mock("@tauri-apps/plugin-process", () => ({
  relaunch: vi.fn().mockResolvedValue(undefined),
}));

// ── @tauri-apps/plugin-opener ──────────────────────────────────────
vi.mock("@tauri-apps/plugin-opener", () => ({
  openPath: vi.fn().mockResolvedValue(undefined),
  openUrl: vi.fn().mockResolvedValue(undefined),
}));

// ── solid-js/web (Portal) ──────────────────────────────────────────
vi.mock("solid-js/web", async () => {
  const actual = await vi.importActual<Record<string, unknown>>("solid-js/web");
  return {
    ...actual,
    Portal: (props: { children: unknown }) => props.children,
  };
});
