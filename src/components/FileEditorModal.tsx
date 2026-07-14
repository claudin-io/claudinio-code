import { createSignal, createEffect, onMount, onCleanup, type Component } from "solid-js";
import * as monaco from "monaco-editor";
import { Icon } from "./Icon";
import { t } from "../lib/grill-me";
import { readFile, writeFile } from "../lib/ipc";
import { defineMonacoThemes, getMonacoTheme } from "../lib/monacoThemes";
import { theme } from "../lib/theme";

interface FileEditorModalProps {
  filePath: string;
  rootPath: string;
  onClose: () => void;
}

export function detectLanguage(filePath: string): string {
  const ext = filePath.match(/\.([^.]+)$/)?.[1]?.toLowerCase();
  const map: Record<string, string> = {
    ts: "typescript",
    mts: "typescript",
    tsx: "typescript",
    js: "javascript",
    mjs: "javascript",
    jsx: "javascript",
    json: "json",
    md: "markdown",
    css: "css",
    html: "html",
    py: "python",
    rs: "rust",
    go: "go",
  };
  return ext && ext in map ? map[ext] : "plaintext";
}

export function getBasename(path: string): string {
  return path.replace(/\\/g, "/").split("/").pop() ?? path;
}

export function getRelativePath(filePath: string, rootPath: string): string {
  const normFile = filePath.replace(/\\/g, "/");
  const normRoot = rootPath.replace(/\\/g, "/").replace(/\/$/, "");
  if (normFile.startsWith(normRoot + "/")) {
    return normFile.slice(normRoot.length + 1);
  }
  return normFile;
}

const FileEditorModal: Component<FileEditorModalProps> = (props) => {
  let editorContainer: HTMLDivElement | undefined;
  let editor: monaco.editor.IStandaloneCodeEditor | undefined;
  let activeDisposables: monaco.IDisposable[] = [];
  let mounted = true;

  const [originalContent, setOriginalContent] = createSignal("");
  const [isDirty, setIsDirty] = createSignal(false);
  const [_loading, setLoading] = createSignal(true);

  const basename = () => getBasename(props.filePath);
  const relativePath = () => getRelativePath(props.filePath, props.rootPath);
  const language = () => detectLanguage(props.filePath);

  const handleSave = async () => {
    if (!editor) return;
    const content = editor.getValue();
    await writeFile(props.filePath, content);
    setOriginalContent(content);
    setIsDirty(false);
  };

  const handleClose = () => {
    if (isDirty()) {
      const confirmed = window.confirm(t("fileEditor.unsavedMessage"));
      if (!confirmed) return;
    }
    props.onClose();
  };

  const disposeEditor = () => {
    activeDisposables.forEach((d) => d.dispose());
    activeDisposables = [];
    editor?.dispose();
    editor = undefined;
  };

  const initEditor = async (filePath: string) => {
    disposeEditor();
    setIsDirty(false);

    defineMonacoThemes();

    if (!mounted) return;
    if (!editorContainer) return;

    const content = await readFile(filePath);
    if (!mounted) return;
    setOriginalContent(content);

    const monacoTheme = getMonacoTheme(theme());

    editor = monaco.editor.create(editorContainer, {
      value: content,
      language: language(),
      theme: monacoTheme,
      automaticLayout: true,
      minimap: { enabled: true },
      scrollBeyondLastLine: false,
      wordWrap: "off",
      fontSize: 13,
      tabSize: 2,
    });

    editor.focus();

    const disposable = editor.onDidChangeModelContent(() => {
      setIsDirty(editor!.getValue() !== originalContent());
    });
    activeDisposables.push(disposable);

    setLoading(false);
  };

  createEffect(() => {
    const fp = props.filePath;
    const rp = props.rootPath;
    if (fp && rp) {
      initEditor(fp);
    }
  });

  // Reactive theme switching: keep Monaco in sync when user changes theme
  createEffect(() => {
    const currentTheme = theme();
    if (editor) {
      monaco.editor.setTheme(getMonacoTheme(currentTheme));
    }
  });

  // Keyboard events (runs once on mount, cleaned up on unmount)
  onMount(() => {
    const onKey = (e: KeyboardEvent) => {
      if ((e.ctrlKey || e.metaKey) && e.key === "s") {
        e.preventDefault();
        handleSave();
      }
      if (e.key === "Escape") {
        handleClose();
      }
    };
    document.addEventListener("keydown", onKey);
    onCleanup(() => {
      document.removeEventListener("keydown", onKey);
    });
  });

  onCleanup(() => {
    mounted = false;
    disposeEditor();
  });

  return (
    <div
      class="fixed inset-0 z-50 flex items-center justify-center bg-black/40"
      onClick={(e) => {
        if (e.target === e.currentTarget) handleClose();
      }}
    >
      <div class="flex w-[80vw] h-[80vh] flex-col rounded-xl bg-surface-0 shadow-2xl">
        <div class="flex items-center justify-between border-b border-border-subtle px-5 py-3 gap-3">
          <div class="flex items-center gap-2 min-w-0">
            <span class="font-semibold text-ink truncate">{basename()}</span>
            {isDirty() && <span class="text-accent font-bold">*</span>}
            <span class="hidden sm:inline text-xs text-ink-faint truncate">
              {relativePath()}
            </span>
          </div>
          <div class="flex items-center gap-2 shrink-0">
            <button
              onClick={handleSave}
              class="flex items-center gap-1.5 rounded-md px-3 py-1.5 text-sm font-medium bg-accent text-accent-ink transition-colors hover:bg-accent-hover"
            >
              <Icon name="check" class="h-3.5 w-3.5" />
              {t("fileEditor.save")}
            </button>
            <button
              onClick={handleClose}
              class="rounded-md p-1.5 text-ink-faint transition-colors hover:bg-surface-2 hover:text-ink"
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

export default FileEditorModal;
