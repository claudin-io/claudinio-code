import * as monaco from "monaco-editor";

let defined = false;

export function defineMonacoThemes() {
  if (defined) return;
  defined = true;

  monaco.editor.defineTheme("claudinio-dark", {
    base: "vs-dark",
    inherit: true,
    rules: [],
    colors: {
      "editor.background": "#090910",
      "editor.foreground": "#edeef5",
      "editor.lineHighlightBackground": "#0e0f16",
      "editor.selectionBackground": "#272833",
      "editor.inactiveSelectionBackground": "#11121b",
      "editorCursor.foreground": "#6d74f5",
      "editorLineNumber.foreground": "#8c8e9c",
      "editorLineNumber.activeForeground": "#b5b7c1",
      "editorGutter.background": "#090910",
      "editor.selectionHighlightBackground": "#27283340",
      "diffEditor.insertedTextBackground": "#22c37326",
      "diffEditor.removedTextBackground": "#f75d5926",
      "diffEditor.insertedLineBackground": "#22c37314",
      "diffEditor.removedLineBackground": "#f75d5914",
      "scrollbarSlider.background": "#2728338c",
      "scrollbarSlider.hoverBackground": "#3334408c",
      "scrollbarSlider.activeBackground": "#333440",
    },
  });

  monaco.editor.defineTheme("claudinio-light", {
    base: "vs",
    inherit: true,
    rules: [],
    colors: {
      "editor.background": "#f8f8fa",
      "editor.foreground": "#10111a",
      "editor.lineHighlightBackground": "#edeef2",
      "editor.selectionBackground": "#bbbdcb",
      "editor.inactiveSelectionBackground": "#e0e1e8",
      "editorCursor.foreground": "#4d4bd1",
      "editorLineNumber.foreground": "#666875",
      "editorLineNumber.activeForeground": "#40414d",
      "editorGutter.background": "#f8f8fa",
      "editor.selectionHighlightBackground": "#bbbdcb40",
      "diffEditor.insertedTextBackground": "#00964a26",
      "diffEditor.removedTextBackground": "#c2172526",
      "diffEditor.insertedLineBackground": "#00964a14",
      "diffEditor.removedLineBackground": "#c2172514",
      "scrollbarSlider.background": "#bbbdcb8c",
      "scrollbarSlider.hoverBackground": "#9b9dae8c",
      "scrollbarSlider.activeBackground": "#9b9dae",
    },
  });

  monaco.editor.defineTheme("claudinio-sepia", {
    base: "vs",
    inherit: true,
    rules: [],
    colors: {
      "editor.background": "#f6f2e7",
      "editor.foreground": "#22190a",
      "editor.lineHighlightBackground": "#eae4d6",
      "editor.selectionBackground": "#bbb098",
      "editor.inactiveSelectionBackground": "#dbd4c2",
      "editorCursor.foreground": "#ae5700",
      "editorLineNumber.foreground": "#7d7361",
      "editorLineNumber.activeForeground": "#554c3b",
      "editorGutter.background": "#f6f2e7",
      "editor.selectionHighlightBackground": "#bbb09840",
      "diffEditor.insertedTextBackground": "#1d933026",
      "diffEditor.removedTextBackground": "#c2172526",
      "diffEditor.insertedLineBackground": "#1d933014",
      "diffEditor.removedLineBackground": "#c2172514",
      "scrollbarSlider.background": "#bbb0988c",
      "scrollbarSlider.hoverBackground": "#9f90778c",
      "scrollbarSlider.activeBackground": "#9f9077",
    },
  });

  monaco.editor.defineTheme("claudinio-dracula", {
    base: "vs-dark",
    inherit: true,
    rules: [],
    colors: {
      "editor.background": "#19112a",
      "editor.foreground": "#e2d9f3",
      "editor.lineHighlightBackground": "#1e1531",
      "editor.selectionBackground": "#2d2245",
      "editor.inactiveSelectionBackground": "#201839",
      "editorCursor.foreground": "#bb8cf2",
      "editorLineNumber.foreground": "#7e7191",
      "editorLineNumber.activeForeground": "#a899b5",
      "editorGutter.background": "#19112a",
      "editor.selectionHighlightBackground": "#2d224540",
      "diffEditor.insertedTextBackground": "#22c37326",
      "diffEditor.removedTextBackground": "#f75d5926",
      "diffEditor.insertedLineBackground": "#22c37314",
      "diffEditor.removedLineBackground": "#f75d5914",
      "scrollbarSlider.background": "#2d22458c",
      "scrollbarSlider.hoverBackground": "#3a2d5a8c",
      "scrollbarSlider.activeBackground": "#3a2d5a",
    },
  });

  monaco.editor.defineTheme("claudinio-nord", {
    base: "vs-dark",
    inherit: true,
    rules: [],
    colors: {
      "editor.background": "#1f262e",
      "editor.foreground": "#d8dee9",
      "editor.lineHighlightBackground": "#242b34",
      "editor.selectionBackground": "#2f3843",
      "editor.inactiveSelectionBackground": "#262d38",
      "editorCursor.foreground": "#81a1c1",
      "editorLineNumber.foreground": "#747b86",
      "editorLineNumber.activeForeground": "#9ca3af",
      "editorGutter.background": "#1f262e",
      "editor.selectionHighlightBackground": "#2f384340",
      "diffEditor.insertedTextBackground": "#22c37326",
      "diffEditor.removedTextBackground": "#f75d5926",
      "diffEditor.insertedLineBackground": "#22c37314",
      "diffEditor.removedLineBackground": "#f75d5914",
      "scrollbarSlider.background": "#2f38438c",
      "scrollbarSlider.hoverBackground": "#3d47548c",
      "scrollbarSlider.activeBackground": "#3d4754",
    },
  });

  monaco.editor.defineTheme("claudinio-solarized-dark", {
    base: "vs-dark",
    inherit: true,
    rules: [],
    colors: {
      "editor.background": "#1f1c14",
      "editor.foreground": "#d3cbb7",
      "editor.lineHighlightBackground": "#242018",
      "editor.selectionBackground": "#2f2b20",
      "editor.inactiveSelectionBackground": "#262219",
      "editorCursor.foreground": "#d3af6a",
      "editorLineNumber.foreground": "#7a725f",
      "editorLineNumber.activeForeground": "#a0977f",
      "editorGutter.background": "#1f1c14",
      "editor.selectionHighlightBackground": "#2f2b2040",
      "diffEditor.insertedTextBackground": "#22c37326",
      "diffEditor.removedTextBackground": "#f75d5926",
      "diffEditor.insertedLineBackground": "#22c37314",
      "diffEditor.removedLineBackground": "#f75d5914",
      "scrollbarSlider.background": "#2f2b208c",
      "scrollbarSlider.hoverBackground": "#3d382b8c",
      "scrollbarSlider.activeBackground": "#3d382b",
    },
  });

  monaco.editor.defineTheme("claudinio-solarized-light", {
    base: "vs",
    inherit: true,
    rules: [],
    colors: {
      "editor.background": "#fbf5e8",
      "editor.foreground": "#554835",
      "editor.lineHighlightBackground": "#f0eadd",
      "editor.selectionBackground": "#ddd6c4",
      "editor.inactiveSelectionBackground": "#ece5d6",
      "editorCursor.foreground": "#af7e2e",
      "editorLineNumber.foreground": "#8a7f6a",
      "editorLineNumber.activeForeground": "#5f5543",
      "editorGutter.background": "#fbf5e8",
      "editor.selectionHighlightBackground": "#ddd6c440",
      "diffEditor.insertedTextBackground": "#00964a26",
      "diffEditor.removedTextBackground": "#c2172526",
      "diffEditor.insertedLineBackground": "#00964a14",
      "diffEditor.removedLineBackground": "#c2172514",
      "scrollbarSlider.background": "#ddd6c48c",
      "scrollbarSlider.hoverBackground": "#c3bbaa8c",
      "scrollbarSlider.activeBackground": "#c3bbaa",
    },
  });

  monaco.editor.defineTheme("claudinio-monokai", {
    base: "vs-dark",
    inherit: true,
    rules: [],
    colors: {
      "editor.background": "#16130e",
      "editor.foreground": "#eae2d6",
      "editor.lineHighlightBackground": "#1b1812",
      "editor.selectionBackground": "#2b2620",
      "editor.inactiveSelectionBackground": "#1e1a15",
      "editorCursor.foreground": "#e6db74",
      "editorLineNumber.foreground": "#736b5e",
      "editorLineNumber.activeForeground": "#9e9482",
      "editorGutter.background": "#16130e",
      "editor.selectionHighlightBackground": "#2b262040",
      "diffEditor.insertedTextBackground": "#22c37326",
      "diffEditor.removedTextBackground": "#f75d5926",
      "diffEditor.insertedLineBackground": "#22c37314",
      "diffEditor.removedLineBackground": "#f75d5914",
      "scrollbarSlider.background": "#2b26208c",
      "scrollbarSlider.hoverBackground": "#39332b8c",
      "scrollbarSlider.activeBackground": "#39332b",
    },
  });

  monaco.editor.defineTheme("claudinio-one-dark", {
    base: "vs-dark",
    inherit: true,
    rules: [],
    colors: {
      "editor.background": "#181a24",
      "editor.foreground": "#d4d7e3",
      "editor.lineHighlightBackground": "#1d1f2a",
      "editor.selectionBackground": "#2c2e3c",
      "editor.inactiveSelectionBackground": "#202230",
      "editorCursor.foreground": "#61afef",
      "editorLineNumber.foreground": "#6f7280",
      "editorLineNumber.activeForeground": "#9295a3",
      "editorGutter.background": "#181a24",
      "editor.selectionHighlightBackground": "#2c2e3c40",
      "diffEditor.insertedTextBackground": "#22c37326",
      "diffEditor.removedTextBackground": "#f75d5926",
      "diffEditor.insertedLineBackground": "#22c37314",
      "diffEditor.removedLineBackground": "#f75d5914",
      "scrollbarSlider.background": "#2c2e3c8c",
      "scrollbarSlider.hoverBackground": "#3b3d4d8c",
      "scrollbarSlider.activeBackground": "#3b3d4d",
    },
  });

  monaco.editor.defineTheme("claudinio-catppuccin", {
    base: "vs-dark",
    inherit: true,
    rules: [],
    colors: {
      "editor.background": "#1e1a20",
      "editor.foreground": "#e0d7e0",
      "editor.lineHighlightBackground": "#231f26",
      "editor.selectionBackground": "#2f2a32",
      "editor.inactiveSelectionBackground": "#262128",
      "editorCursor.foreground": "#cba6f7",
      "editorLineNumber.foreground": "#716773",
      "editorLineNumber.activeForeground": "#958998",
      "editorGutter.background": "#1e1a20",
      "editor.selectionHighlightBackground": "#2f2a3240",
      "diffEditor.insertedTextBackground": "#22c37326",
      "diffEditor.removedTextBackground": "#f75d5926",
      "diffEditor.insertedLineBackground": "#22c37314",
      "diffEditor.removedLineBackground": "#f75d5914",
      "scrollbarSlider.background": "#2f2a328c",
      "scrollbarSlider.hoverBackground": "#3d37418c",
      "scrollbarSlider.activeBackground": "#3d3741",
    },
  });

  monaco.editor.defineTheme("claudinio-tokyo-night", {
    base: "vs-dark",
    inherit: true,
    rules: [],
    colors: {
      "editor.background": "#12151f",
      "editor.foreground": "#cbd2e6",
      "editor.lineHighlightBackground": "#171a25",
      "editor.selectionBackground": "#292d3e",
      "editor.inactiveSelectionBackground": "#1d2130",
      "editorCursor.foreground": "#7aa2f7",
      "editorLineNumber.foreground": "#63677a",
      "editorLineNumber.activeForeground": "#888da0",
      "editorGutter.background": "#12151f",
      "editor.selectionHighlightBackground": "#292d3e40",
      "diffEditor.insertedTextBackground": "#22c37326",
      "diffEditor.removedTextBackground": "#f75d5926",
      "diffEditor.insertedLineBackground": "#22c37314",
      "diffEditor.removedLineBackground": "#f75d5914",
      "scrollbarSlider.background": "#292d3e8c",
      "scrollbarSlider.hoverBackground": "#373c508c",
      "scrollbarSlider.activeBackground": "#373c50",
    },
  });

  monaco.editor.defineTheme("claudinio-gruvbox-dark", {
    base: "vs-dark",
    inherit: true,
    rules: [],
    colors: {
      "editor.background": "#1d1b14",
      "editor.foreground": "#d4be98",
      "editor.lineHighlightBackground": "#222018",
      "editor.selectionBackground": "#2d2a20",
      "editor.inactiveSelectionBackground": "#232117",
      "editorCursor.foreground": "#d79921",
      "editorLineNumber.foreground": "#746e5e",
      "editorLineNumber.activeForeground": "#99927d",
      "editorGutter.background": "#1d1b14",
      "editor.selectionHighlightBackground": "#2d2a2040",
      "diffEditor.insertedTextBackground": "#22c37326",
      "diffEditor.removedTextBackground": "#f75d5926",
      "diffEditor.insertedLineBackground": "#22c37314",
      "diffEditor.removedLineBackground": "#f75d5914",
      "scrollbarSlider.background": "#2d2a208c",
      "scrollbarSlider.hoverBackground": "#3b372b8c",
      "scrollbarSlider.activeBackground": "#3b372b",
    },
  });

  monaco.editor.defineTheme("claudinio-gruvbox-light", {
    base: "vs",
    inherit: true,
    rules: [],
    colors: {
      "editor.background": "#faf0dd",
      "editor.foreground": "#3c3836",
      "editor.lineHighlightBackground": "#efe5d2",
      "editor.selectionBackground": "#dad0ba",
      "editor.inactiveSelectionBackground": "#e9dfc9",
      "editorCursor.foreground": "#af6000",
      "editorLineNumber.foreground": "#857b65",
      "editorLineNumber.activeForeground": "#59513e",
      "editorGutter.background": "#faf0dd",
      "editor.selectionHighlightBackground": "#dad0ba40",
      "diffEditor.insertedTextBackground": "#00964a26",
      "diffEditor.removedTextBackground": "#c2172526",
      "diffEditor.insertedLineBackground": "#00964a14",
      "diffEditor.removedLineBackground": "#c2172514",
      "scrollbarSlider.background": "#dad0ba8c",
      "scrollbarSlider.hoverBackground": "#c0b7a38c",
      "scrollbarSlider.activeBackground": "#c0b7a3",
    },
  });

  monaco.editor.defineTheme("claudinio-rose-pine", {
    base: "vs-dark",
    inherit: true,
    rules: [],
    colors: {
      "editor.background": "#1b1721",
      "editor.foreground": "#e0def4",
      "editor.lineHighlightBackground": "#201c27",
      "editor.selectionBackground": "#2d2838",
      "editor.inactiveSelectionBackground": "#221e2c",
      "editorCursor.foreground": "#c4a7e7",
      "editorLineNumber.foreground": "#6f687b",
      "editorLineNumber.activeForeground": "#948ba0",
      "editorGutter.background": "#1b1721",
      "editor.selectionHighlightBackground": "#2d283840",
      "diffEditor.insertedTextBackground": "#22c37326",
      "diffEditor.removedTextBackground": "#f75d5926",
      "diffEditor.insertedLineBackground": "#22c37314",
      "diffEditor.removedLineBackground": "#f75d5914",
      "scrollbarSlider.background": "#2d28388c",
      "scrollbarSlider.hoverBackground": "#3b354a8c",
      "scrollbarSlider.activeBackground": "#3b354a",
    },
  });

  monaco.editor.defineTheme("claudinio-everforest", {
    base: "vs-dark",
    inherit: true,
    rules: [],
    colors: {
      "editor.background": "#1c221f",
      "editor.foreground": "#c5c9bc",
      "editor.lineHighlightBackground": "#212724",
      "editor.selectionBackground": "#2d332d",
      "editor.inactiveSelectionBackground": "#222823",
      "editorCursor.foreground": "#a7c080",
      "editorLineNumber.foreground": "#6f7668",
      "editorLineNumber.activeForeground": "#939a88",
      "editorGutter.background": "#1c221f",
      "editor.selectionHighlightBackground": "#2d332d40",
      "diffEditor.insertedTextBackground": "#22c37326",
      "diffEditor.removedTextBackground": "#f75d5926",
      "diffEditor.insertedLineBackground": "#22c37314",
      "diffEditor.removedLineBackground": "#f75d5914",
      "scrollbarSlider.background": "#2d332d8c",
      "scrollbarSlider.hoverBackground": "#3b423b8c",
      "scrollbarSlider.activeBackground": "#3b423b",
    },
  });
}

import type { ThemeId } from "./theme";

/** Maps a ThemeId to the corresponding Monaco editor theme name */
export function getMonacoTheme(t: ThemeId): string {
  // Themes already prefixed with "claudinio-" map by replacing "claudinio-" with "claudinio-"
  // (they keep the same name, e.g. "claudinio-light" → "claudinio-light")
  if (t.startsWith("claudinio-")) return t;
  // Legacy default: "claudinio" maps to "claudinio-dark"
  if (t === "claudinio") return "claudinio-dark";
  return `claudinio-${t}`;
}
