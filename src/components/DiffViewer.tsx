import { onMount, onCleanup, createEffect, type Component } from "solid-js";
import * as monaco from "monaco-editor";
import { theme } from "../lib/theme";
import { defineMonacoThemes } from "../lib/monacoThemes";

export const DiffViewer: Component<{
  original: string;
  modified: string;
  language?: string;
}> = (props) => {
  let containerRef: HTMLDivElement | undefined;
  let editor: monaco.editor.IStandaloneDiffEditor | undefined;

  onMount(() => {
    if (!containerRef) return;
    defineMonacoThemes();
    editor = monaco.editor.createDiffEditor(containerRef, {
      theme: theme() === "dark" ? "claudinio-dark" : "claudinio-light",
      fontSize: 13,
      fontFamily: "'JetBrains Mono', monospace",
      readOnly: true,
      renderSideBySide: true,
      minimap: { enabled: false },
      scrollBeyondLastLine: false,
      automaticLayout: true,
    });
    editor.setModel({
      original: monaco.editor.createModel(props.original, props.language),
      modified: monaco.editor.createModel(props.modified, props.language),
    });
  });

  createEffect(() => {
    const currentTheme = theme();
    if (editor) {
      monaco.editor.setTheme(currentTheme === "dark" ? "claudinio-dark" : "claudinio-light");
    }
  });

  onCleanup(() => {
    editor?.getModel()?.original?.dispose();
    editor?.getModel()?.modified?.dispose();
    editor?.dispose();
  });

  return <div ref={containerRef} class="h-full w-full" />;
};
