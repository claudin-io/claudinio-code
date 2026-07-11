import { onMount, onCleanup, createEffect, type Component } from "solid-js";
import * as monaco from "monaco-editor";
import { theme } from "../lib/theme";
import { defineMonacoThemes } from "../lib/monacoThemes";

export const DiffViewer: Component<{
  original: string;
  modified: string;
  language?: string;
  inline?: boolean;
  maxHeight?: string;
}> = (props) => {
  let containerRef: HTMLDivElement | undefined;
  let editor: monaco.editor.IStandaloneDiffEditor | undefined;

  const estimateHeight = (): number => {
    const origLines = props.original.split("\n").length;
    const modLines = props.modified.split("\n").length;
    const maxLines = Math.max(origLines, modLines);
    // ~20px per line at 13px font + ~30px for Monaco's header/gutter padding
    const contentHeight = maxLines * 20 + 30;
    const clamped = Math.max(contentHeight, 100);
    return props.maxHeight ? Math.min(clamped, parseInt(props.maxHeight)) : clamped;
  };

  onMount(() => {
    // containerRef is always set by SolidJS before onMount fires
    defineMonacoThemes();

    // Set container height to match content before creating editor
    if (props.inline) {
      containerRef.style.height = `${estimateHeight()}px`;
    }

    editor = monaco.editor.createDiffEditor(containerRef, {
      theme: theme() === "dark" ? "claudinio-dark" : theme() === "sepia" ? "claudinio-sepia" : "claudinio-light",
      fontSize: 13,
      fontFamily: "'JetBrains Mono', monospace",
      readOnly: true,
      renderSideBySide: !props.inline,
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
    monaco.editor.setTheme(currentTheme === "dark" ? "claudinio-dark" : currentTheme === "sepia" ? "claudinio-sepia" : "claudinio-light");
  });

  onCleanup(() => {
    editor?.getModel()?.original?.dispose();
    editor?.getModel()?.modified?.dispose();
    editor?.dispose();
  });

  return <div ref={containerRef} class="h-full w-full" />;
};
