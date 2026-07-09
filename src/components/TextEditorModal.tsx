import { onMount, onCleanup, type Component } from "solid-js";
import * as monaco from "monaco-editor";
import { Icon } from "./Icon";

interface TextEditorModalProps {
  initialText: string;
  onClose: (text: string) => void;
}

const TextEditorModal: Component<TextEditorModalProps> = (props) => {
  let editorContainer: HTMLDivElement | undefined;
  let editor: monaco.editor.IStandaloneCodeEditor | undefined;

  const handleClose = () => {
    props.onClose(editor?.getValue() ?? props.initialText);
  };

  onMount(() => {
    if (!editorContainer) return;

    editor = monaco.editor.create(editorContainer, {
      value: props.initialText,
      language: "text",
      theme: "vs-dark",
      automaticLayout: true,
      minimap: { enabled: false },
      scrollBeyondLastLine: false,
      wordWrap: "on",
    });
    editor.focus();

    const onKey = (e: KeyboardEvent) => {
      if (e.key === "Escape") handleClose();
    };
    document.addEventListener("keydown", onKey);

    onCleanup(() => {
      document.removeEventListener("keydown", onKey);
      editor?.dispose();
    });
  });

  return (
    <div
      class="fixed inset-0 z-50 flex items-center justify-center bg-black/40"
      onClick={(e) => {
        if (e.target === e.currentTarget) handleClose();
      }}
    >
      <div class="flex w-[80vw] h-[80vh] flex-col rounded-xl bg-surface-0 shadow-2xl">
        <div class="flex items-center justify-between border-b border-border-subtle px-5 py-3">
          <span class="font-semibold text-ink">Editor</span>
          <button
            onClick={handleClose}
            class="rounded-md p-1 text-ink-faint transition-colors hover:bg-surface-2 hover:text-ink"
          >
            <Icon name="x" class="h-4 w-4" />
          </button>
        </div>
        <div ref={editorContainer} class="flex-1 min-h-0" />
      </div>
    </div>
  );
};

export default TextEditorModal;
