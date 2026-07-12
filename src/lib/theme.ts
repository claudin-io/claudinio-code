import { createSignal, createMemo, createRoot } from "solid-js";

export type ThemePreference = "system" | "dark" | "light" | "sepia";
export type ResolvedTheme = "dark" | "light" | "sepia";

const STORAGE_KEY = "claudinio_theme";
const CYCLE_ORDER: ThemePreference[] = ["system", "dark", "light", "sepia"];

function createThemeState() {
  // ── read persisted preference ────────────────────────────────────
  const stored =
    typeof localStorage !== "undefined"
      ? localStorage.getItem(STORAGE_KEY)
      : null;
  const initial: ThemePreference =
    stored === "dark" || stored === "light" || stored === "sepia"
      ? stored
      : "system";

  const [preference, setPreference] = createSignal<ThemePreference>(initial);

  // ── system-level dark/light (only relevant when preference === "system") ──
  const [systemDark, setSystemDark] = createSignal(
    typeof window !== "undefined"
      ? !window.matchMedia("(prefers-color-scheme: light)").matches
      : true,
  );

  // Watch OS preference changes
  if (typeof window !== "undefined") {
    const mql = window.matchMedia("(prefers-color-scheme: light)");
    const handleChange = () => setSystemDark(!mql.matches);
    mql.addEventListener("change", handleChange);
  }

  // ── resolved theme (what HTML actually uses) ─────────────────────
  const resolvedTheme = createMemo<ResolvedTheme>(() => {
    const pref = preference();
    if (pref === "system") return systemDark() ? "dark" : "light";
    return pref; // "dark" | "light" | "sepia"
  });

  // ── keep data-theme in sync with resolvedTheme ──────────────────
  createMemo(() => {
    if (typeof document !== "undefined") {
      document.documentElement.dataset.theme = resolvedTheme();
    }
  });

  // ── public API ───────────────────────────────────────────────────
  const setThemePreference = (next: ThemePreference) => {
    setPreference(next);
    if (typeof localStorage !== "undefined") {
      localStorage.setItem(STORAGE_KEY, next);
    }
  };

  const cycleTheme = () => {
    const current = preference();
    const idx = CYCLE_ORDER.indexOf(current);
    const next = CYCLE_ORDER[(idx + 1) % CYCLE_ORDER.length];
    setThemePreference(next);
  };

  return { preference, resolvedTheme, setThemePreference, cycleTheme };
}

// ── Module-level root (lazy init via createRoot, same pattern as grill-me.ts) ─
let rootState: ReturnType<typeof createThemeState> | undefined;
function initState() {
  if (!rootState) {
    createRoot(() => {
      rootState = createThemeState();
    });
  }
  return rootState!;
}

// ── Backward-compatible export: theme() returns the resolved theme ──
// Existing consumers like DiffViewer call theme() inside createEffect —
// Solid tracks the access to the inner createMemo automatically.
export function theme(): ResolvedTheme {
  return initState().resolvedTheme();
}

// ── New public API ─────────────────────────────────────────────────
export function preference(): ThemePreference {
  return initState().preference();
}

export function resolvedTheme(): ResolvedTheme {
  return initState().resolvedTheme();
}

export function setThemePreference(next: ThemePreference) {
  initState().setThemePreference(next);
}

export function cycleTheme() {
  initState().cycleTheme();
}

/** @internal — testing only: forces a fresh state on next init */
export function __resetState() {
  rootState = undefined;
}

