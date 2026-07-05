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
      "editor.background": "#141210",
      "editor.foreground": "#ece8e3",
      "editor.lineHighlightBackground": "#1c1917",
      "editor.selectionBackground": "#35302a",
      "editor.inactiveSelectionBackground": "#26221e",
      "editorCursor.foreground": "#d97757",
      "editorLineNumber.foreground": "#6e675f",
      "editorLineNumber.activeForeground": "#9c948a",
      "editorGutter.background": "#141210",
      "editor.selectionHighlightBackground": "#35302a44",
      "diffEditor.insertedTextBackground": "#7fb06922",
      "diffEditor.removedTextBackground": "#e5735f22",
      "diffEditor.insertedLineBackground": "#7fb06911",
      "diffEditor.removedLineBackground": "#e5735f11",
      "scrollbarSlider.background": "#35302a88",
      "scrollbarSlider.hoverBackground": "#4a433b88",
      "scrollbarSlider.activeBackground": "#4a433b",
    },
  });

  monaco.editor.defineTheme("claudinio-light", {
    base: "vs",
    inherit: true,
    rules: [],
    colors: {
      "editor.background": "#faf8f5",
      "editor.foreground": "#2b2620",
      "editor.lineHighlightBackground": "#f3f0ea",
      "editor.selectionBackground": "#d6cfc3",
      "editor.inactiveSelectionBackground": "#eae5dd",
      "editorCursor.foreground": "#c65f3f",
      "editorLineNumber.foreground": "#a69a8c",
      "editorLineNumber.activeForeground": "#7a7166",
      "editorGutter.background": "#faf8f5",
      "editor.selectionHighlightBackground": "#d6cfc344",
      "diffEditor.insertedTextBackground": "#5f8f4f22",
      "diffEditor.removedTextBackground": "#c94d3a22",
      "diffEditor.insertedLineBackground": "#5f8f4f11",
      "diffEditor.removedLineBackground": "#c94d3a11",
      "scrollbarSlider.background": "#d6cfc388",
      "scrollbarSlider.hoverBackground": "#b8ae9e88",
      "scrollbarSlider.activeBackground": "#b8ae9e",
    },
  });
}
