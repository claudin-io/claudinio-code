/* @refresh reload */
import { render } from "solid-js/web";
import App from "./App";

// Suppress the default WebView2 (Chromium) context menu on Windows.
// Editable elements and selected text keep native behavior (copy/paste/spellcheck).
// Existing custom context menus (App.tsx, FileTree.tsx) call preventDefault() themselves.
if (!import.meta.env.DEV) {
  document.addEventListener("contextmenu", (e) => {
    const t = e.target as HTMLElement | null;
    if (t?.closest("input, textarea, [contenteditable='true'], [contenteditable='']")) return;
    if (window.getSelection()?.toString()) return;
    e.preventDefault();
  });
}

render(() => <App />, document.getElementById("root") as HTMLElement);
