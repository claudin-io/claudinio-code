import { createEffect, type JSX } from "solid-js";
import { useTimelineHost } from "./host";

/// Rendered markdown from the transcript.
///
/// # The one rule
///
/// `html` must come from `renderMarkdown` in this package and from nothing else. That
/// function is where DOMPurify runs, and it is the reason the sanitizer lives here
/// rather than in each surface: §10 of the plan names XSS on the web origin as a high
/// risk, and this component renders *model output* on an origin that holds the key to
/// someone's machine. Two copies of a sanitizer is one sanitizer and one liability —
/// the fix goes into whichever the author happened to have open.
///
/// It takes HTML rather than the source text on purpose: the caller chooses between
/// `renderMarkdown` and `renderLiveMarkdown` (which leaves a streaming block alone),
/// and a component that rendered text itself would have to guess.
///
/// # Links
///
/// A link in a transcript points outside this view — to the web, or to a file on the
/// developer's machine. Neither is something this component can act on, so both go to
/// the host: the desktop hands a URL to the OS because opening it in the webview would
/// navigate away from the app, and a browser opens a tab. A file reference is only a
/// link where the host can open one, which the web peer cannot — the file is on the
/// other machine and §2 says it stays there.

interface ProseProps {
  /// Output of `renderMarkdown`. Already sanitized; see above.
  html: string;
  class?: string;
}

export function Prose(props: ProseProps): JSX.Element {
  const host = useTimelineHost();
  let el!: HTMLDivElement;

  // Set imperatively rather than through the JSX `innerHTML` prop, so a later
  // hydration step — mermaid on the desktop — can rely on the DOM being current.
  createEffect(() => {
    el.innerHTML = props.html;
  });

  const onClick = (event: MouseEvent) => {
    const target = event.target as HTMLElement | null;
    const anchor = target?.closest?.("a[data-link-type]") as HTMLAnchorElement | null;
    if (!anchor) return;

    const href = anchor.getAttribute("href");
    if (!href) return;
    const kind = anchor.getAttribute("data-link-type");

    // Prevented in both branches: an unhandled internal link must not navigate this
    // view to a `file:` URL or a relative path, which on the web peer would replace the
    // session with a 404.
    event.preventDefault();

    if (kind === "external") {
      host.openExternalUrl(href);
      return;
    }
    host.openFile?.(href);
  };

  return <div ref={el} class={props.class ?? "cprose"} onClick={onClick} />;
}
