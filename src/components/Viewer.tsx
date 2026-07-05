import { createSignal, createResource, Show, onMount, onCleanup, type Component } from "solid-js";
import * as monaco from "monaco-editor";
import { readFile, writeFile } from "../lib/ipc";

export const Viewer: Component<{ path: () => string | null }> = (props) => {
  let containerRef: HTMLDivElement | undefined;
  let editor: monaco.editor.IStandaloneCodeEditor | undefined;
  const [dirty, setDirty] = createSignal(false);
  const [saving, setSaving] = createSignal(false);

  const [content] = createResource(props.path, readFile);

  let currentPath: string | null = null;
  let savedContent = "";
  let currentModel: monaco.editor.ITextModel | null = null;

  const save = async () => {
    if (!currentPath || !editor) return;
    setSaving(true);
    try {
      await writeFile(currentPath, editor.getValue());
      savedContent = editor.getValue();
      setDirty(false);
    } catch (e) {
      console.error("save failed:", e);
    }
    setSaving(false);
  };

  const disposeEditor = () => {
    if (currentModel) {
      currentModel.dispose();
      currentModel = null;
    }
    if (editor) {
      editor.dispose();
      editor = undefined;
    }
    currentPath = null;
    setDirty(false);
  };

  onMount(() => {
    document.addEventListener("keydown", handleKeyDown);
  });

  onCleanup(() => {
    document.removeEventListener("keydown", handleKeyDown);
    disposeEditor();
  });

  const handleKeyDown = (e: KeyboardEvent) => {
    if ((e.metaKey || e.ctrlKey) && e.key === "s") {
      e.preventDefault();
      save();
    }
  };

  const detectLanguage = (path: string): string => {
    if (path.endsWith(".ts") || path.endsWith(".tsx")) return "typescript";
    if (path.endsWith(".rs")) return "rust";
    if (path.endsWith(".py")) return "python";
    if (path.endsWith(".swift")) return "swift";
    if (path.endsWith(".js") || path.endsWith(".jsx")) return "javascript";
    if (path.endsWith(".css")) return "css";
    if (path.endsWith(".json")) return "json";
    if (path.endsWith(".html")) return "html";
    if (path.endsWith(".md")) return "markdown";
    if (path.endsWith(".toml")) return "ini";
    if (path.endsWith(".yaml") || path.endsWith(".yml")) return "yaml";
    return "plaintext";
  };

  createResource(
    () => content(),
    (text) => {
      const path = props.path();
      if (!path || !text) return;
      if (!containerRef) return;

      if (path !== currentPath) {
        disposeEditor();
        currentPath = path;

        const lang = detectLanguage(path);
        currentModel = monaco.editor.createModel(text, lang);
        savedContent = text;
        editor = monaco.editor.create(containerRef, {
          model: currentModel,
          theme: "vs-dark",
          fontSize: 13,
          fontFamily: "'JetBrains Mono', 'Fira Code', monospace",
          minimap: { enabled: true, scale: 1 },
          scrollBeyondLastLine: false,
          automaticLayout: true,
          wordWrap: "off",
          tabSize: 2,
          renderWhitespace: "selection",
          cursorBlinking: "smooth",
          smoothScrolling: true,
        });

        editor.onDidChangeModelContent(() => {
          const val = editor?.getValue() ?? "";
          setDirty(val !== savedContent);
        });
      } else if (currentModel && text !== currentModel.getValue()) {
        currentModel.setValue(text);
      }
    },
  );

  return (
    <div class="flex h-full flex-col bg-surface-0">
      <Show
        when={props.path()}
        fallback={
          <div class="flex h-full items-center justify-center text-sm text-ink-muted">
            Selecione um arquivo para editar
          </div>
        }
      >
        <div class="flex items-center justify-between border-b border-border-subtle bg-surface-1 px-3 py-1">
          <span class="truncate text-xs text-ink-muted">{props.path()}</span>
          <div class="flex items-center gap-2">
            <Show when={dirty()}>
              <span class="text-[10px] text-yellow-400">não salvo</span>
            </Show>
            <button
              onClick={save}
              disabled={!dirty() || saving()}
              class="rounded border border-border-subtle bg-surface-2 px-2 py-0.5 text-[11px] hover:border-accent disabled:opacity-40"
            >
              {saving() ? "Salvando…" : "Salvar"}
            </button>
          </div>
        </div>
        <div ref={containerRef} class="flex-1" />
      </Show>
    </div>
  );
};
