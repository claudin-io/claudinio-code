import { createSignal, createMemo, createRoot } from "solid-js";

// ── Theme identity ───────────────────────────────────────────────────
export type ThemeId =
  | "claudinio"
  | "claudinio-light"
  | "claudinio-sepia"
  | "dracula"
  | "nord"
  | "solarized-dark"
  | "solarized-light"
  | "monokai"
  | "one-dark"
  | "catppuccin"
  | "tokyo-night"
  | "gruvbox-dark"
  | "gruvbox-light"
  | "rose-pine"
  | "everforest";

export type ThemePreference = "system" | ThemeId;
export type ResolvedTheme = ThemeId;

// ── Theme metadata ───────────────────────────────────────────────────
export interface ThemeMeta {
  label: string;
  category: "dark" | "light";
  /** 5 representative oklch colours for the preview swatch card */
  previewColors: string[];
}

export const themeMetadata: Record<ThemeId, ThemeMeta> = {
  claudinio: {
    label: "Claudinio",
    category: "dark",
    previewColors: [
      "oklch(0.145 0.015 280)",
      "oklch(0.185 0.018 280)",
      "oklch(0.95 0.01 280)",
      "oklch(0.62 0.19 277)",
      "oklch(0.72 0.17 155)",
    ],
  },
  "claudinio-light": {
    label: "Claudinio Light",
    category: "light",
    previewColors: [
      "oklch(0.98 0.003 280)",
      "oklch(0.91 0.01 280)",
      "oklch(0.18 0.02 280)",
      "oklch(0.50 0.20 277)",
      "oklch(0.58 0.17 155)",
    ],
  },
  "claudinio-sepia": {
    label: "Claudinio Sepia",
    category: "light",
    previewColors: [
      "oklch(0.96 0.015 90)",
      "oklch(0.87 0.025 88)",
      "oklch(0.22 0.03 80)",
      "oklch(0.55 0.16 65)",
      "oklch(0.58 0.17 145)",
    ],
  },
  dracula: {
    label: "Dracula",
    category: "dark",
    previewColors: [
      "oklch(0.14 0.02 325)",
      "oklch(0.19 0.03 325)",
      "oklch(0.94 0.02 325)",
      "oklch(0.72 0.15 335)",
      "oklch(0.65 0.15 145)",
    ],
  },
  nord: {
    label: "Nord",
    category: "dark",
    previewColors: [
      "oklch(0.17 0.015 220)",
      "oklch(0.22 0.02 220)",
      "oklch(0.93 0.01 220)",
      "oklch(0.68 0.10 210)",
      "oklch(0.62 0.08 150)",
    ],
  },
  "solarized-dark": {
    label: "Solarized Dark",
    category: "dark",
    previewColors: [
      "oklch(0.16 0.01 45)",
      "oklch(0.22 0.015 45)",
      "oklch(0.90 0.02 45)",
      "oklch(0.55 0.06 190)",
      "oklch(0.55 0.06 45)",
    ],
  },
  "solarized-light": {
    label: "Solarized Light",
    category: "light",
    previewColors: [
      "oklch(0.96 0.01 45)",
      "oklch(0.88 0.015 45)",
      "oklch(0.18 0.015 45)",
      "oklch(0.45 0.06 190)",
      "oklch(0.45 0.06 45)",
    ],
  },
  monokai: {
    label: "Monokai",
    category: "dark",
    previewColors: [
      "oklch(0.14 0.01 50)",
      "oklch(0.19 0.02 50)",
      "oklch(0.95 0.01 50)",
      "oklch(0.68 0.15 15)",
      "oklch(0.58 0.12 120)",
    ],
  },
  "one-dark": {
    label: "One Dark",
    category: "dark",
    previewColors: [
      "oklch(0.16 0.015 230)",
      "oklch(0.20 0.018 230)",
      "oklch(0.92 0.01 230)",
      "oklch(0.58 0.18 260)",
      "oklch(0.60 0.12 150)",
    ],
  },
  catppuccin: {
    label: "Catppuccin",
    category: "dark",
    previewColors: [
      "oklch(0.14 0.015 350)",
      "oklch(0.18 0.02 350)",
      "oklch(0.93 0.01 350)",
      "oklch(0.68 0.12 10)",
      "oklch(0.58 0.10 150)",
    ],
  },
  "tokyo-night": {
    label: "Tokyo Night",
    category: "dark",
    previewColors: [
      "oklch(0.12 0.02 240)",
      "oklch(0.17 0.025 240)",
      "oklch(0.94 0.01 240)",
      "oklch(0.70 0.15 280)",
      "oklch(0.55 0.10 160)",
    ],
  },
  "gruvbox-dark": {
    label: "Gruvbox Dark",
    category: "dark",
    previewColors: [
      "oklch(0.18 0.02 40)",
      "oklch(0.24 0.025 40)",
      "oklch(0.92 0.02 40)",
      "oklch(0.58 0.12 60)",
      "oklch(0.55 0.08 140)",
    ],
  },
  "gruvbox-light": {
    label: "Gruvbox Light",
    category: "light",
    previewColors: [
      "oklch(0.93 0.02 40)",
      "oklch(0.86 0.025 40)",
      "oklch(0.21 0.025 40)",
      "oklch(0.52 0.12 60)",
      "oklch(0.48 0.08 140)",
    ],
  },
  "rose-pine": {
    label: "Rose Pine",
    category: "dark",
    previewColors: [
      "oklch(0.16 0.015 340)",
      "oklch(0.20 0.02 340)",
      "oklch(0.94 0.01 340)",
      "oklch(0.72 0.10 360)",
      "oklch(0.58 0.08 160)",
    ],
  },
  everforest: {
    label: "Everforest",
    category: "dark",
    previewColors: [
      "oklch(0.17 0.015 160)",
      "oklch(0.21 0.018 160)",
      "oklch(0.88 0.02 160)",
      "oklch(0.58 0.10 50)",
      "oklch(0.60 0.08 150)",
    ],
  },
};

/** All theme IDs in display order */
export const ALL_THEMES: ThemeId[] = Object.keys(themeMetadata) as ThemeId[];

/** Resolve a user preference ("system" delegates to OS) */
export function resolvePreference(
  pref: ThemePreference,
  systemDark: boolean,
): ThemeId {
  if (pref === "system") return systemDark ? "claudinio" : "claudinio-light";
  return pref;
}

// ── Singleton state ─────────────────────────────────────────────────
const STORAGE_KEY = "claudinio_theme";

// Legacy migration: old stored values "dark"/"light"/"sepia" map to new ids
const LEGACY_MAP: Record<string, ThemePreference> = {
  dark: "claudinio",
  light: "claudinio-light",
  sepia: "claudinio-sepia",
};

const CYCLE_ORDER: ThemePreference[] = [
  "system",
  "claudinio",
  "claudinio-light",
  "claudinio-sepia",
];

function createThemeState() {
  // ── read persisted preference (with legacy migration) ────────────
  const stored =
    typeof localStorage !== "undefined"
      ? localStorage.getItem(STORAGE_KEY)
      : null;
  const migrated = stored ? LEGACY_MAP[stored] ?? stored : null;
  const initial: ThemePreference =
    migrated === "system" ||
    ALL_THEMES.includes(migrated as ThemeId)
      ? (migrated as ThemePreference)
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
    if (pref === "system") return systemDark() ? "claudinio" : "claudinio-light";
    return pref;
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

// ── Module-level root (lazy init via createRoot) ─
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

