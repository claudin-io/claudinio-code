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
}
