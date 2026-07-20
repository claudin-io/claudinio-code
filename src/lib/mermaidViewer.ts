// Global store for the fullscreen Mermaid diagram viewer. A single
// <MermaidViewerModal> mounted at the app root subscribes to this signal, so
// any ProseContent surface (chat, plans, tool bodies) can open the viewer by
// handing it the already-rendered SVG markup.

import { createSignal, createRoot } from "solid-js";

const store = createRoot(() => {
  const [svg, setSvg] = createSignal<string | null>(null);
  return { svg, setSvg };
});

/** The SVG currently open in the viewer, or null when closed. */
export const mermaidViewerSvg = store.svg;

export function openMermaidViewer(svg: string): void {
  store.setSvg(svg);
}

export function closeMermaidViewer(): void {
  store.setSvg(null);
}
