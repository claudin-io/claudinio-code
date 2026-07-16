import { createSignal, createEffect, onMount, onCleanup, Show, type Component } from "solid-js";
import * as monaco from "monaco-editor";
import { convertFileSrc } from "@tauri-apps/api/core";
import { Icon } from "./Icon";
import { t } from "../lib/grill-me";
import { readFile, openExternal } from "../lib/ipc";
import { defineMonacoThemes, getMonacoTheme } from "../lib/monacoThemes";
import { detectLanguage } from "./FileEditorModal";
import { theme } from "../lib/theme";

type ContentType = "text" | "image" | "video" | "audio";

interface ContentViewerModalProps {
  contentType: ContentType;
  filePath: string;
  title: string;
  workspace: string;
  onClose: () => void;
  onCursorLineChange?: (line: number) => void;
}

function resolvePath(filePath: string, workspace: string): string {
  let p = filePath.replace(/^file:\/\//, "");
  if (!p.startsWith("/")) {
    const ws = workspace.replace(/\\/g, "/").replace(/\/$/, "");
    p = ws + "/" + p.replace(/^\.\//, "");
  }
  return p;
}

const ContentViewerModal: Component<ContentViewerModalProps> = (props) => {
  const [loading, setLoading] = createSignal(props.contentType === "text");
  const [error, setError] = createSignal<string | null>(null);
  const [content, setContent] = createSignal("");

  let editorContainer: HTMLDivElement | undefined;
  let editor: monaco.editor.IStandaloneCodeEditor | undefined;
  let mounted = true;

  const resolvedPath = () => resolvePath(props.filePath, props.workspace);
  const mediaSrc = () => convertFileSrc(resolvedPath());

  // Load text content
  onMount(async () => {
    if (props.contentType !== "text") return;
    try {
      setLoading(true);
      setError(null);
      const text = await readFile(resolvedPath());
      if (!mounted) return;
      setContent(text);
    } catch (err) {
      if (!mounted) return;
      setError(t("contentViewer.error"));
    } finally {
      if (mounted) setLoading(false);
    }
  });

  onMount(() => {
    const onKey = (e: KeyboardEvent) => {
      if (e.key === "Escape") props.onClose();
    };
    document.addEventListener("keydown", onKey);
    onCleanup(() => document.removeEventListener("keydown", onKey));
  });

  onCleanup(() => {
    mounted = false;
    editor?.dispose();
  });

  // Monaco editor setup
  createEffect(() => {
    if (props.contentType !== "text" || loading() || error() || !content() || !editorContainer) return;

    defineMonacoThemes();
    const currentTheme = theme();
    const monacoTheme = getMonacoTheme(currentTheme);

    const lang = detectLanguage(props.filePath);

    editor = monaco.editor.create(editorContainer, {
      value: content(),
      language: lang,
      theme: monacoTheme,
      readOnly: true,
      wordWrap: "on",
      minimap: { enabled: false },
      fontSize: 13,
      lineNumbers: "on",
      scrollBeyondLastLine: false,
      automaticLayout: true,
    });

    // Report cursor position changes (for --goto support)
    const cursorDisposable = editor.onDidChangeCursorPosition((e) => {
      props.onCursorLineChange?.(e.position.lineNumber);
    });
    onCleanup(() => cursorDisposable.dispose());

    onCleanup(() => editor?.dispose());
  });

  const handleBackdrop = (e: MouseEvent) => {
    if (e.target === e.currentTarget) props.onClose();
  };

  return (
    <div
      class="fixed inset-0 z-50 flex items-center justify-center bg-black/40"
      onClick={handleBackdrop}
    >
      <div class="flex w-[80vw] h-[80vh] flex-col overflow-hidden rounded-xl bg-surface-0 shadow-2xl">
        {/* Header */}
        <div class="flex items-center justify-between border-b border-border-subtle px-4 py-3">
          <span class="truncate text-sm font-medium text-ink">{props.title}</span>
          <button
            onClick={props.onClose}
            class="flex h-7 w-7 items-center justify-center rounded-md text-ink-muted hover:bg-surface-2 hover:text-ink"
            title={t("contentViewer.close")}
          >
            <Icon name="x" class="h-4 w-4" />
          </button>
        </div>

        {/* Content area */}
        <div class="flex flex-1 overflow-hidden">
          <Show when={props.contentType === "text"}>
            <Show when={loading()}>
              <div class="flex flex-1 items-center justify-center">
                <div class="flex items-center gap-2 text-ink-muted">
                  <Icon name="loader" class="h-4 w-4 animate-spin" />
                  <span class="text-sm">{t("contentViewer.loading")}</span>
                </div>
              </div>
            </Show>
            <Show when={error()}>
              <div class="flex flex-1 flex-col items-center justify-center gap-2">
                <Icon name="alert-circle" class="h-6 w-6 text-red-400" />
                <span class="text-sm text-red-400">{error()}</span>
              </div>
            </Show>
            <Show when={!loading() && !error() && content()}>
              <div ref={editorContainer} class="h-full w-full" />
            </Show>
          </Show>

          <Show when={props.contentType === "image"}>
            <div class="flex flex-1 items-center justify-center bg-surface-5">
              <img
                src={mediaSrc()}
                alt={props.title}
                class="max-h-full max-w-full object-contain"
              />
            </div>
          </Show>

          <Show when={props.contentType === "video"}>
            <div class="flex flex-1 items-center justify-center">
              <video controls class="max-h-full max-w-full">
                <source src={mediaSrc()} />
              </video>
            </div>
          </Show>

          <Show when={props.contentType === "audio"}>
            <div class="flex flex-1 items-center justify-center">
              <audio controls class="w-full max-w-md">
                <source src={mediaSrc()} />
              </audio>
            </div>
          </Show>
        </div>

        {/* Footer with Open Externally for images */}
        <Show when={props.contentType === "image"}>
          <div class="flex items-center justify-end border-t border-border-subtle px-4 py-2">
            <button
              onClick={() => openExternal(resolvedPath())}
              class="flex items-center gap-1.5 rounded-md bg-surface-2 px-3 py-1.5 text-xs text-ink-muted hover:bg-surface-3 hover:text-ink"
            >
              <Icon name="external-link" class="h-3.5 w-3.5" />
              {t("contentViewer.openExternally")}
            </button>
          </div>
        </Show>
      </div>
    </div>
  );
};

export default ContentViewerModal;
