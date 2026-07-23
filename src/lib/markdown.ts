// The single place markdown becomes HTML in this app.
//
// Everything rendered in the chat is untrusted: model output, file contents the
// agent quotes back, subagent reports, web_search results. `marked` deliberately
// passes raw HTML through (it dropped its own sanitizer in v5 and points at
// DOMPurify instead), and the result is handed to `innerHTML` in ProseContent.
// Without the sanitize step below, an `<img src=x onerror=...>` in any file the
// agent reads executes script in the webview — and Tauri always exposes
// `window.__TAURI_INTERNALS__.invoke`, so that script reaches the IPC surface.
// `<script>` tags inserted via innerHTML do NOT run; event-handler attributes do.
//
// Nothing outside this module may call `marked.parse` directly.

import { marked } from "marked";
import DOMPurify from "dompurify";
import hljs from "highlight.js";

// The live streaming block re-parses the whole message every smooth-text tick
// (~30/s), which used to re-highlight every code block each time. Completed
// blocks hit this cache instead; only the still-growing block re-highlights.
const highlightCache = new Map<string, string>();
const HIGHLIGHT_CACHE_MAX = 300;

// True while parsing the live streaming message (see renderLiveMarkdown):
// unlabeled blocks skip hljs.highlightAuto there — it tries every registered
// grammar and is by far the worst per-tick cost.
let parsingLiveMessage = false;

export function escapeHtml(text: string): string {
  return text
    .replace(/&/g, "&amp;")
    .replace(/</g, "&lt;")
    .replace(/>/g, "&gt;");
}

// Allowlist, not denylist: only these tags survive sanitization. Covers what
// `marked` emits plus the wrappers the custom renderers below produce.
//
// `svg` is deliberately absent. Mermaid diagrams are not in the sanitized HTML —
// the renderer emits an empty `.mermaid-block` placeholder and `renderMermaid`
// injects the SVG afterwards, from mermaid's own `securityLevel: "strict"`
// output. Keeping svg out closes the `<svg onload=...>` vector entirely.
const ALLOWED_TAGS = [
  "p", "div", "span", "br", "hr",
  "h1", "h2", "h3", "h4", "h5", "h6",
  "ul", "ol", "li", "dl", "dt", "dd",
  "blockquote", "pre", "code", "kbd", "samp", "var",
  "em", "strong", "b", "i", "u", "s", "del", "ins", "mark", "sub", "sup", "small",
  "a", "img",
  "table", "thead", "tbody", "tfoot", "tr", "th", "td", "caption",
];

// `id` is excluded on purpose: an attacker-controlled id can clobber `document`
// properties. `style` is excluded so injected markup cannot position an
// invisible overlay over the approval buttons.
const ALLOWED_ATTR = [
  "href", "title", "class", "src", "alt", "width", "height",
  "align", "colspan", "rowspan", "start", "reversed",
  // Consumed by ProseContent to hydrate diagrams and route link clicks.
  "data-mermaid", "data-link-type",
];

// DOMPurify's default, plus `file:` — the chat links to workspace files and
// `handleLinkClick` strips the `file://` prefix before opening the viewer.
// Everything else falls through to the relative-path branch. `javascript:`,
// `data:` and `vbscript:` match none of the alternatives and are dropped.
const ALLOWED_URI_REGEXP =
  /^(?:(?:(?:f|ht)tps?|file|mailto|tel|callto|sms|cid|xmpp):|[^a-z]|[a-z+.\-]+(?:[^a-z+.\-:]|$))/i;

// DOMPurify exempts a handful of media tags (`img`, `video`, `source`, …) from
// ALLOWED_URI_REGEXP so that inline `data:` payloads keep working, and the list
// is not configurable downwards. Nothing in the chat needs inline data URIs —
// images come from workspace paths — so strip them back out. The one place the
// app does build a `data:image/svg+xml` URL (MermaidViewerModal) constructs the
// element in JS and never passes through here.
//
// The hook is global to the DOMPurify singleton, which is fine: this module is
// the only caller of `sanitize` in the app.
DOMPurify.addHook("afterSanitizeAttributes", (node) => {
  const src = node.getAttribute?.("src");
  if (src && /^\s*data:/i.test(src)) node.removeAttribute("src");
});

function sanitize(html: string): string {
  return DOMPurify.sanitize(html, {
    ALLOWED_TAGS,
    ALLOWED_ATTR,
    ALLOWED_URI_REGEXP,
    // Only the two data-* attributes listed above get through.
    ALLOW_DATA_ATTR: false,
    ALLOW_ARIA_ATTR: false,
    // Defence in depth: these are already excluded by the allowlist.
    FORBID_TAGS: ["style", "script", "iframe", "object", "embed", "form", "svg", "math"],
    FORBID_ATTR: ["style", "id", "target", "formaction", "srcset"],
  });
}

marked.use({
  renderer: {
    code({ text, lang }) {
      // lang + text joined by a separator no fence language token can
      // contain, so "js"+"xA" and "jsx"+"A" never collide as one key. Must
      // not be a NUL byte: that makes git treat this whole file as binary.
      const key = `${lang ?? ""}\x1f${text}`;
      if (lang === "mermaid") {
        // Emit a placeholder carrying the (encoded) source. ProseContent
        // hydrates it into an SVG after injection; the visible <pre> is the
        // fallback shown while streaming and on any render error. This runs
        // inside marked.parse() during render, so it must not throw:
        // encodeURIComponent rejects lone surrogates (possible in streamed or
        // model-authored source), which would otherwise blank the message.
        let encoded: string;
        try {
          encoded = encodeURIComponent(text);
        } catch {
          // Fall back to a plain code block — no diagram, but nothing breaks.
          return `<div class="code-block"><pre class="hljs"><code>${escapeHtml(text)}</code></pre></div>`;
        }
        return `<div class="mermaid-block" data-mermaid="${encoded}">`
          + `<pre class="mermaid-src"><code>${escapeHtml(text)}</code></pre></div>`;
      }
      let highlighted = highlightCache.get(key);
      if (highlighted === undefined) {
        if (lang && hljs.getLanguage(lang)) {
          highlighted = hljs.highlight(text, { language: lang }).value;
        } else if (parsingLiveMessage) {
          // Don't cache: once the message completes, the final render
          // auto-detects (one-time cost) and caches that result.
          return `<div class="code-block"><pre class="hljs"><code>${escapeHtml(text)}</code></pre></div>`;
        } else {
          highlighted = hljs.highlightAuto(text).value;
        }
        if (highlightCache.size >= HIGHLIGHT_CACHE_MAX) highlightCache.clear();
        highlightCache.set(key, highlighted);
      }
      const label = lang
        ? `<span class="code-lang-label">${escapeHtml(lang)}</span>`
        : "";
      return `<div class="code-block">${label}<pre class="hljs"><code>${highlighted}</code></pre></div>`;
    },
    link({ href, title, text }) {
      const clean = href.split('?')[0].split('#')[0];
      const ext = clean.split('.').pop()?.toLowerCase();
      let dataType = 'file';
      if (href.match(/^https?:\/\//)) {
        dataType = 'external';
      } else if (ext && ['png', 'jpg', 'jpeg', 'gif', 'webp', 'svg'].includes(ext)) {
        dataType = 'image';
      } else if (ext && ['mp4', 'webm', 'mov'].includes(ext)) {
        dataType = 'video';
      } else if (ext && ['mp3', 'wav', 'ogg', 'flac'].includes(ext)) {
        dataType = 'audio';
      }
      // href/title are interpolated raw here; the sanitize() pass is what makes
      // that safe (attribute-breakout attempts do not survive re-parsing).
      const titleAttr = title ? ` title="${title}"` : '';
      return `<a href="${href}"${titleAttr} data-link-type="${dataType}">${text}</a>`;
    },
  },
});

/** Markdown → sanitized HTML. The only supported way to render agent content. */
export function renderMarkdown(text: string): string {
  return sanitize(marked.parse(text, { async: false }) as string);
}

/**
 * Same as `renderMarkdown`, for the block still being streamed: unlabeled code
 * fences skip `hljs.highlightAuto`, which dominates the per-tick cost.
 */
export function renderLiveMarkdown(text: string): string {
  parsingLiveMessage = true;
  try {
    return renderMarkdown(text);
  } finally {
    parsingLiveMessage = false;
  }
}
