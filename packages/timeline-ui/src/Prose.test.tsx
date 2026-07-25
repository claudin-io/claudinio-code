import { describe, it, expect, vi, afterEach } from "vitest";
import { render } from "solid-js/web";
import { Prose } from "./Prose";
import { TimelineHostProvider, type TimelineHost } from "./host";
import { renderMarkdown } from "./markdown";

const flush = () => new Promise((r) => setTimeout(r, 10));

let dispose: (() => void) | undefined;
let host: HTMLDivElement | undefined;

afterEach(() => {
  dispose?.();
  host?.remove();
  dispose = undefined;
  host = undefined;
});

async function mount(html: string, provided?: Partial<TimelineHost>) {
  host = document.createElement("div");
  document.body.appendChild(host);

  const openExternalUrl = vi.fn();
  const openFile = vi.fn();
  const hostValue: TimelineHost = {
    openExternalUrl,
    ...(provided?.openFile === undefined ? {} : { openFile: provided.openFile }),
    ...provided,
  };

  dispose = render(
    () => (
      <TimelineHostProvider value={hostValue}>
        <Prose html={html} />
      </TimelineHostProvider>
    ),
    host,
  );
  await flush();
  return { root: host, openExternalUrl, openFile };
}

const clickLink = (root: HTMLElement, index = 0) => {
  const link = root.querySelectorAll("a")[index];
  const event = new MouseEvent("click", { bubbles: true, cancelable: true });
  link.dispatchEvent(event);
  return event;
};

describe("Prose", () => {
  it("renders the html it was given", async () => {
    const { root } = await mount("<p>hello <em>there</em></p>");

    expect(root.querySelector("p")?.textContent).toBe("hello there");
    expect(root.querySelector("em")).toBeTruthy();
  });

  /// Rendering markdown through this package's own renderer is the point: it is where
  /// DOMPurify runs, and the desktop and the web peer must share it rather than each
  /// keeping a sanitizer.
  it("renders what the shared markdown renderer produces", async () => {
    const { root } = await mount(renderMarkdown("# Title\n\nsome **bold** text\n"));

    expect(root.textContent).toContain("Title");
    expect(root.querySelector("strong")?.textContent).toBe("bold");
  });

  /// The reason the sanitizer is shared. Model output reaches this component on an
  /// origin that holds the key to someone's machine.
  it("does not execute script the model wrote", async () => {
    const { root } = await mount(renderMarkdown('<img src=x onerror="globalThis.pwned=1">'));

    expect(root.innerHTML).not.toContain("onerror");
    expect((globalThis as Record<string, unknown>).pwned).toBeUndefined();
  });

  // ── links ────────────────────────────────────────────────────────────────

  /// A link goes to the host: the desktop hands it to the OS, because opening it in the
  /// webview would navigate away from the app, and a browser opens a tab.
  it("hands an external link to the host", async () => {
    const { root, openExternalUrl } = await mount(
      renderMarkdown("[the docs](https://example.com/docs)"),
    );

    clickLink(root);
    expect(openExternalUrl).toHaveBeenCalledWith("https://example.com/docs");
  });

  /// Navigating this view to the link would replace the session with whatever is at the
  /// other end — on the web peer, losing the connection along with it.
  it("stops the click from navigating", async () => {
    const { root } = await mount(renderMarkdown("[the docs](https://example.com/docs)"));

    const event = clickLink(root);
    expect(event.defaultPrevented).toBe(true);
  });

  it("hands a file reference to the host that can open one", async () => {
    const openFile = vi.fn();
    const { root } = await mount(renderMarkdown("see `src/main.rs`\n\n[src/main.rs](src/main.rs)"), {
      openFile,
    });

    clickLink(root);
    expect(openFile).toHaveBeenCalledWith("src/main.rs");
  });

  /// The web peer has no `openFile`: the file is on the developer's machine and §2 says
  /// it stays there. The click must do nothing rather than navigate to a relative path
  /// that is not there.
  it("does nothing for a file reference when the host cannot open one", async () => {
    const { root, openExternalUrl } = await mount(renderMarkdown("[src/main.rs](src/main.rs)"));

    const event = clickLink(root);
    expect(event.defaultPrevented).toBe(true);
    expect(openExternalUrl).not.toHaveBeenCalled();
  });

  it("ignores a click that is not on a link", async () => {
    const { root, openExternalUrl } = await mount("<p>just text</p>");

    root.querySelector("p")!.dispatchEvent(new MouseEvent("click", { bubbles: true }));
    expect(openExternalUrl).not.toHaveBeenCalled();
  });

  /// The markdown renderer tags links it recognises. One without a tag was not produced
  /// by it, so this must not treat it as a destination the host should open.
  it("ignores a link the renderer did not tag", async () => {
    const { root, openExternalUrl } = await mount('<p><a href="https://example.com">x</a></p>');

    const event = clickLink(root);
    expect(openExternalUrl).not.toHaveBeenCalled();
    expect(event.defaultPrevented).toBe(false);
  });

  // ── the default host ─────────────────────────────────────────────────────

  /// Without a provider the default is the browser's behaviour, so a missing provider
  /// degrades to something correct rather than to a link that silently does nothing.
  it("opens a link in a new tab with no provider at all", async () => {
    host = document.createElement("div");
    document.body.appendChild(host);
    const open = vi.fn();
    vi.stubGlobal("open", open);

    dispose = render(
      () => <Prose html={renderMarkdown("[docs](https://example.com/docs)")} />,
      host,
    );
    await flush();

    clickLink(host);
    expect(open).toHaveBeenCalledWith("https://example.com/docs", "_blank", "noopener,noreferrer");
    vi.unstubAllGlobals();
  });

  /// `window.open("javascript:…")` executes in this origin. The default checks the
  /// scheme itself rather than trusting that the renderer only ever tags http links —
  /// a default whose safety depends on a file two directories away is a default that
  /// breaks when someone edits the other file.
  it("the default refuses a scheme that is not web", async () => {
    host = document.createElement("div");
    document.body.appendChild(host);
    const open = vi.fn();
    vi.stubGlobal("open", open);

    dispose = render(
      () => (
        <Prose html='<p><a href="javascript:globalThis.pwned=1" data-link-type="external">x</a></p>' />
      ),
      host,
    );
    await flush();

    clickLink(host);
    expect(open).not.toHaveBeenCalled();
    expect((globalThis as Record<string, unknown>).pwned).toBeUndefined();
    vi.unstubAllGlobals();
  });
});
