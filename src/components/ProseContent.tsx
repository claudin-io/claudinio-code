import { createEffect, type JSX } from "solid-js";
import { renderMermaid } from "../lib/mermaid";
import { resolvedTheme } from "../lib/theme";
import { openMermaidViewer } from "../lib/mermaidViewer";

// Shared markdown surface: injects pre-rendered HTML (from `marked`) and
// hydrates any ```mermaid placeholders into SVG afterwards.
//
// innerHTML is set imperatively inside the effect rather than via the JSX
// `innerHTML` prop so that the DOM is guaranteed up to date before we hydrate
// diagrams, and so that a theme switch (which does not change `html`) cleanly
// restores the `.mermaid-block` placeholders before re-rendering them in the
// new theme.

interface ProseContentProps {
  html: string;
  class?: string;
  /** While true (streaming), skip mermaid hydration — the source fallback shows. */
  live?: boolean;
  onClick?: (e: MouseEvent & { currentTarget: HTMLDivElement; target: Element }) => void;
}

export function ProseContent(props: ProseContentProps): JSX.Element {
  let el!: HTMLDivElement;
  createEffect(() => {
    const html = props.html;
    el.innerHTML = html;
    // Nothing more to do while streaming, or when there are no diagrams.
    // Only content with a mermaid block touches the theme signal (which lazily
    // boots theme state) and the mermaid runtime — plain markdown stays cheap.
    if (props.live || !el.querySelector(".mermaid-block")) return;
    // Track theme so diagrams re-render when the user switches themes.
    resolvedTheme();
    void renderMermaid(el);
  });

  const handleClick = (e: MouseEvent & { currentTarget: HTMLDivElement; target: Element }) => {
    // A click on a rendered diagram opens the fullscreen zoom/pan/download viewer.
    const block = (e.target as HTMLElement).closest?.(".mermaid-block.mermaid-rendered");
    const svg = block?.querySelector("svg");
    if (svg) {
      e.preventDefault();
      openMermaidViewer(svg.outerHTML);
      return;
    }
    props.onClick?.(e);
  };

  return <div ref={el} class={props.class} onClick={handleClick} />;
}
