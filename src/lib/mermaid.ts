// Lazy Mermaid integration.
//
// Mermaid is ~2-3 MB, so it is imported dynamically the first time a diagram
// actually needs to render — it never enters the main bundle or startup path.
// This module is the ONLY place that imports mermaid.
//
// The code-fence renderer (lib/markdown.ts) emits a placeholder
// <div class="mermaid-block" data-mermaid="<encoded source>"> for every
// ```mermaid fence; renderMermaid() walks a container after injection and
// replaces each placeholder's contents with the rendered SVG.
//
// The SVG bypasses the markdown sanitizer — it is written straight to
// `innerHTML` below — so it is mermaid's own `securityLevel: "strict"` that has
// to hold. Do not relax that setting. (Note that "injected via innerHTML" is NOT
// itself a safety property: <script> does not execute that way, but event-handler
// attributes like onerror/onload do. Sanitizing is what makes the markdown path
// safe; see lib/markdown.ts.)

import { resolvedTheme, themeMetadata } from "./theme";

type MermaidModule = typeof import("mermaid").default;

let mermaidPromise: Promise<MermaidModule> | undefined;
// The mermaid `theme` the singleton was last initialized with, so we know when
// a theme switch requires re-initialization + re-render.
let initializedTheme: string | undefined;
// Monotonic id source for mermaid.render (ids must be unique per call).
let renderSeq = 0;

/** Map the app's resolved theme onto a mermaid built-in theme. */
export function currentMermaidTheme(): "dark" | "default" {
  const id = resolvedTheme();
  return themeMetadata[id]?.category === "dark" ? "dark" : "default";
}

/** Import mermaid once and (re-)initialize it for the current theme. */
async function ensureMermaid(): Promise<MermaidModule> {
  if (!mermaidPromise) {
    mermaidPromise = import("mermaid").then((m) => m.default);
  }
  const mermaid = await mermaidPromise;
  const theme = currentMermaidTheme();
  if (initializedTheme !== theme) {
    mermaid.initialize({
      startOnLoad: false,
      // Diagram source is untrusted model output — keep it sandboxed so
      // click-handlers / scripts in a diagram cannot execute.
      securityLevel: "strict",
      theme,
    });
    initializedTheme = theme;
  }
  return mermaid;
}

/**
 * Find every unrendered `.mermaid-block` placeholder inside `container` and
 * replace its contents with the rendered SVG. Malformed diagrams fall back to
 * their source (already present as `.mermaid-src`) — a bad diagram never breaks
 * the surrounding message. Safe to call repeatedly: rendered nodes are marked
 * with `data-mermaid-theme` and skipped unless the theme changed.
 */
export async function renderMermaid(container: HTMLElement): Promise<void> {
  const nodes = container.querySelectorAll<HTMLElement>(".mermaid-block[data-mermaid]");
  if (nodes.length === 0) return;

  const theme = currentMermaidTheme();
  // Collect nodes that still need rendering for the current theme.
  const pending: HTMLElement[] = [];
  nodes.forEach((node) => {
    if (node.dataset.mermaidTheme !== theme) pending.push(node);
  });
  if (pending.length === 0) return;

  // Loading the (lazy, ~2-3 MB) module can fail — a chunk load error offline,
  // for instance. Swallow it: the `.mermaid-src` fallback already shows the
  // source, and a diagram must never break the surrounding message.
  let mermaid: MermaidModule;
  try {
    mermaid = await ensureMermaid();
  } catch {
    pending.forEach((node) => {
      node.dataset.mermaidTheme = theme;
      node.classList.add("mermaid-error");
    });
    return;
  }

  for (const node of pending) {
    const encoded = node.dataset.mermaid ?? "";
    let source = "";
    try {
      source = decodeURIComponent(encoded);
    } catch {
      source = encoded;
    }
    try {
      const id = `mermaid-svg-${renderSeq++}`;
      const { svg } = await mermaid.render(id, source);
      node.innerHTML = svg;
      node.dataset.mermaidTheme = theme;
      node.classList.add("mermaid-rendered");
    } catch {
      // Leave the `.mermaid-src` fallback in place; mark as handled for this
      // theme so we don't retry the same broken source every effect run.
      node.dataset.mermaidTheme = theme;
      node.classList.add("mermaid-error");
    }
  }
}
