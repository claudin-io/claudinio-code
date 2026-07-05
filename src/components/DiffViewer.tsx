import { onMount, onCleanup, type Component } from "solid-js";
import * as monaco from "monaco-editor";

export const DiffViewer: Component<{
  original: string;
  modified: string;
  language?: string;
}> = (props) => {
  let containerRef: HTMLDivElement | undefined;
  let editor: monaco.editor.IStandaloneDiffEditor | undefined;

  onMount(() => {
    if (!containerRef) return;
    editor = monaco.editor.createDiffEditor(containerRef, {
      theme: "vs-dark",
      fontSize: 13,
      fontFamily: "'JetBrains Mono', 'Fira Code', monospace",
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

  onCleanup(() => {
    editor?.getModel()?.original?.dispose();
    editor?.getModel()?.modified?.dispose();
    editor?.dispose();
  });

  return <div ref={containerRef} class="h-full w-full" />;
};
