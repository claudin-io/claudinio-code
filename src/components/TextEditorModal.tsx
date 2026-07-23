import { onMount, onCleanup, createSignal, Show, type Component } from "solid-js";
import * as monaco from "monaco-editor";
import { Icon } from "./Icon";

interface TextEditorModalProps {
  initialText: string;
  onClose: (text: string) => void;
  onEnhance?: (text: string) => Promise<string>;
}

const TextEditorModal: Component<TextEditorModalProps> = (props) => {
  let editorContainer: HTMLDivElement | undefined;
  let editor: monaco.editor.IStandaloneCodeEditor | undefined;
  const [isEnhancing, setIsEnhancing] = createSignal(false);

  const handleClose = () => {
    props.onClose(editor?.getValue() ?? props.initialText);
  };

  const handleEnhance = async () => {
    if (!props.onEnhance || !editor) return;
    setIsEnhancing(true);
    try {
      const result = await props.onEnhance(editor.getValue());
      editor.setValue(result);
      editor.focus();
    } catch (err) {
      console.error("enhance failed", err);
    } finally {
      setIsEnhancing(false);
    }
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
          <span class="font-semibold text-ink">{"Editor"}</span>
          <div class="flex items-center gap-1">
            <Show when={props.onEnhance}>
              <button
                onClick={handleEnhance}
                disabled={isEnhancing()}
                class="rounded-md p-1 text-ink-faint transition-colors hover:bg-surface-2 hover:text-accent disabled:opacity-50"
                title={isEnhancing() ? "Enhancing..." : "Enhance prompt"}
              >
                <Show when={!isEnhancing()} fallback={<Icon name="loader" class="h-4 w-4 animate-spin" />}>
                  <Icon name="magic-button-outline" class="h-4 w-4" />
                </Show>
              </button>
            </Show>
            <button
              onClick={handleClose}
              class="rounded-md p-1 text-ink-faint transition-colors hover:bg-surface-2 hover:text-ink"
            >
              <Icon name="x" class="h-4 w-4" />
            </button>
          </div>
        </div>
        <div ref={editorContainer} class="flex-1 min-h-0" />
      </div>
    </div>
  );
};

export default TextEditorModal;
