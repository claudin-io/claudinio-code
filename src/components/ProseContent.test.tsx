import { describe, it, expect, beforeAll, vi } from "vitest";
import { render } from "solid-js/web";

// The real Tauri webview has matchMedia; jsdom does not. The theme system
// (read synchronously inside ProseContent's effect) boots via matchMedia, so
// polyfill it to mirror production. Its absence is exactly the kind of failure
// that used to blank the whole chat — see the resilience guards below.
beforeAll(() => {
  if (!window.matchMedia) {
    // @ts-expect-error minimal test polyfill
    window.matchMedia = (q: string) => ({
      matches: false, media: q, onchange: null,
      addEventListener() {}, removeEventListener() {},
      addListener() {}, removeListener() {}, dispatchEvent() { return false; },
    });
  }
});

// Control the (otherwise ~2-3 MB, lazily imported) mermaid runtime so these
// tests stay fast and can simulate a broken diagram / broken runtime.
const renderSpy = vi.fn(async (_id: string, _src: string) => ({ svg: "<svg><g/></svg>" }));
const initializeSpy = vi.fn(() => {});
vi.mock("mermaid", () => ({
  default: {
    initialize: (...a: unknown[]) => initializeSpy(...a),
    render: (id: string, src: string) => renderSpy(id, src),
  },
}));

// What ChatPanel's marked `code` renderer emits for a ```mermaid fence.
const mermaidBlock = (src: string) =>
  `<div class="mermaid-block" data-mermaid="${encodeURIComponent(src)}">` +
  `<pre class="mermaid-src"><code>diagram</code></pre></div>`;

async function renderTriplet(midHtml: string) {
  const { ProseContent } = await import("./ProseContent");
  const host = document.createElement("div");
  document.body.appendChild(host);
  let threw: unknown = null;
  try {
    render(() => (
      <div>
        <ProseContent class="a" html={"<p>BEFORE</p>"} />
        <ProseContent class="b" html={midHtml} />
        <ProseContent class="c" html={"<p>AFTER</p>"} />
      </div>
    ), host);
  } catch (e) {
    threw = e;
  }
  // Let the void renderMermaid() promise settle.
  await new Promise((r) => setTimeout(r, 50));
  const text = host.textContent ?? "";
  const html = host.innerHTML;
  host.remove();
  return { threw, text, html };
}

describe("ProseContent resilience", () => {
  it("injects plain markdown html", async () => {
    const { threw, text } = await renderTriplet("<p>MIDDLE</p>");
    expect(threw).toBeNull();
    expect(text).toContain("BEFORE");
    expect(text).toContain("MIDDLE");
    expect(text).toContain("AFTER");
  });

  it("a rendered mermaid diagram never blanks sibling messages", async () => {
    renderSpy.mockResolvedValueOnce({ svg: "<svg id='ok'><g/></svg>" });
    const { threw, text, html } = await renderTriplet(mermaidBlock("flowchart TD\nA-->B"));
    expect(threw).toBeNull();
    // The sibling AFTER survives regardless of what the diagram did.
    expect(text).toContain("BEFORE");
    expect(text).toContain("AFTER");
    expect(html).toContain("<svg");
  });

  it("a diagram whose render THROWS is contained (source fallback kept, siblings intact)", async () => {
    renderSpy.mockRejectedValueOnce(new Error("bad diagram"));
    const { threw, text, html } = await renderTriplet(mermaidBlock("not a diagram"));
    expect(threw).toBeNull();
    expect(text).toContain("BEFORE");
    expect(text).toContain("AFTER");
    // Fallback <pre class="mermaid-src"> stays visible, marked as errored.
    expect(html).toContain("mermaid-src");
    expect(html).toContain("mermaid-error");
  });

  it("a broken mermaid RUNTIME (initialize throws) is contained", async () => {
    initializeSpy.mockImplementationOnce(() => { throw new Error("runtime boom"); });
    const { threw, text } = await renderTriplet(mermaidBlock("flowchart TD\nA-->B"));
    // renderMermaid swallows an ensureMermaid() failure — no throw propagates,
    // siblings still render.
    expect(threw).toBeNull();
    expect(text).toContain("BEFORE");
    expect(text).toContain("AFTER");
  });

  it("non-string html is coerced instead of throwing", async () => {
    // @ts-expect-error deliberately wrong type to exercise the guard
    const { threw, text } = await renderTriplet(undefined);
    expect(threw).toBeNull();
    expect(text).toContain("BEFORE");
    expect(text).toContain("AFTER");
  });
});
