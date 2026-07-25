import { describe, it, expect } from "vitest";
import { renderMarkdown, renderLiveMarkdown } from "./markdown";

// These are the vectors that were live before sanitization was added. Every
// chat surface (assistant text, tool bodies, subagent reports, commit summaries)
// renders through renderMarkdown into innerHTML, and the webview always has
// window.__TAURI_INTERNALS__.invoke — so script execution here reaches the IPC
// surface. Any of these coming back means the whole chain is open again.
describe("renderMarkdown — XSS containment", () => {
  it("strips event-handler attributes from raw HTML", () => {
    const html = renderMarkdown('<img src=x onerror="alert(1)">');
    expect(html).not.toContain("onerror");
    expect(html).not.toContain("alert(1)");
  });

  it("drops <svg> entirely (onload vector)", () => {
    const html = renderMarkdown("<svg onload=alert(1)></svg>");
    expect(html.toLowerCase()).not.toContain("<svg");
    expect(html).not.toContain("onload");
  });

  it("drops <script> blocks", () => {
    const html = renderMarkdown("<script>alert(1)</script>");
    expect(html.toLowerCase()).not.toContain("<script");
    expect(html).not.toContain("alert(1)");
  });

  it("removes javascript: hrefs", () => {
    const html = renderMarkdown('<a href="javascript:alert(1)">click</a>');
    expect(html).not.toContain("javascript:");
  });

  it("removes javascript: hrefs written as markdown links", () => {
    const html = renderMarkdown("[click](javascript:alert(1))");
    expect(html).not.toContain("javascript:");
  });

  it("survives attribute-breakout attempts in link href and title", () => {
    const html = renderMarkdown('[x](foo.md "a\\" onmouseover=alert(1) x=\\"")');
    expect(html).not.toContain("onmouseover");
  });

  it("strips iframes and objects", () => {
    const html = renderMarkdown('<iframe src="http://evil"></iframe><object data="x"></object>');
    expect(html.toLowerCase()).not.toContain("<iframe");
    expect(html.toLowerCase()).not.toContain("<object");
  });

  it("strips style attributes used for overlaying the approval UI", () => {
    const html = renderMarkdown('<div style="position:fixed;inset:0">x</div>');
    expect(html).not.toContain("style=");
  });

  it("strips id attributes (DOM clobbering)", () => {
    const html = renderMarkdown('<div id="documentElement">x</div>');
    expect(html).not.toContain("id=");
  });

  it("blocks data: URIs", () => {
    const html = renderMarkdown('<img src="data:text/html;base64,PHNjcmlwdD4=">');
    expect(html).not.toContain("data:text/html");
  });

  it("applies the same guarantees while streaming", () => {
    const html = renderLiveMarkdown('<img src=x onerror="alert(1)">');
    expect(html).not.toContain("onerror");
  });
});

// The sanitizer must not eat the markup the chat depends on. A too-tight
// allowlist would silently blank code blocks and diagrams instead of failing.
describe("renderMarkdown — rendering is preserved", () => {
  it("keeps headings, emphasis and lists", () => {
    const html = renderMarkdown("# Title\n\nsome **bold** and *italic*\n\n- one\n- two");
    expect(html).toContain("<h1>Title</h1>");
    expect(html).toContain("<strong>bold</strong>");
    expect(html).toContain("<em>italic</em>");
    expect(html).toContain("<li>one</li>");
  });

  it("keeps tables", () => {
    const html = renderMarkdown("| a | b |\n|---|---|\n| 1 | 2 |");
    expect(html).toContain("<table>");
    expect(html).toContain("<td>1</td>");
  });

  it("keeps the highlighted code-block wrapper and language label", () => {
    const html = renderMarkdown("```ts\nconst x: number = 1;\n```");
    expect(html).toContain('class="code-block"');
    expect(html).toContain('class="code-lang-label"');
    expect(html).toContain('class="hljs"');
    expect(html).toContain("hljs-keyword");
  });

  it("keeps the mermaid placeholder and its encoded source", () => {
    const html = renderMarkdown("```mermaid\ngraph TD;\nA-->B;\n```");
    expect(html).toContain('class="mermaid-block"');
    expect(html).toContain("data-mermaid=");
    expect(html).toContain(encodeURIComponent("graph TD;\nA-->B;"));
    expect(html).toContain('class="mermaid-src"');
  });

  it("keeps data-link-type so link clicks still route", () => {
    const html = renderMarkdown("[readme](./README.md)");
    expect(html).toContain('data-link-type="file"');
    expect(html).toContain('href="./README.md"');
  });

  it("classifies external, image and video links", () => {
    expect(renderMarkdown("[x](https://example.com)")).toContain('data-link-type="external"');
    expect(renderMarkdown("[x](./a.png)")).toContain('data-link-type="image"');
    expect(renderMarkdown("[x](./a.mp4)")).toContain('data-link-type="video"');
  });

  it("keeps file:// links (the viewer strips the prefix)", () => {
    const html = renderMarkdown("[x](file:///tmp/a.md)");
    expect(html).toContain("file:///tmp/a.md");
  });

  it("keeps absolute workspace paths", () => {
    const html = renderMarkdown("[x](/Users/me/project/src/main.rs)");
    expect(html).toContain('href="/Users/me/project/src/main.rs"');
  });
});
