import { createEffect, createSignal, onCleanup, Show, type JSX } from "solid-js";
import { exportFile, exportFileBytes } from "../lib/ipc";
import { mermaidViewerSvg, closeMermaidViewer } from "../lib/mermaidViewer";

// Fullscreen viewer for a rendered Mermaid diagram: scroll/buttons to zoom,
// drag to pan, fit/reset, and download as SVG or PNG. Mounted once at app root.

const MIN_SCALE = 0.1;
const MAX_SCALE = 8;
const clamp = (v: number, lo: number, hi: number) => Math.min(hi, Math.max(lo, v));

// Natural size of the diagram, read from the SVG viewBox (falls back to bbox).
function svgSize(svgEl: SVGSVGElement): { w: number; h: number } {
  const vb = svgEl.viewBox?.baseVal;
  if (vb && vb.width && vb.height) return { w: vb.width, h: vb.height };
  const r = svgEl.getBoundingClientRect();
  return { w: r.width || 800, h: r.height || 600 };
}

export function MermaidViewerModal(): JSX.Element {
  let viewport!: HTMLDivElement;
  let stage!: HTMLDivElement;

  const [scale, setScale] = createSignal(1);
  const [tx, setTx] = createSignal(0);
  const [ty, setTy] = createSignal(0);

  const apply = () => {
    if (stage) stage.style.transform = `translate(${tx()}px, ${ty()}px) scale(${scale()})`;
  };

  const svgEl = (): SVGSVGElement | null => stage?.querySelector("svg") ?? null;

  const fit = () => {
    const el = svgEl();
    if (!el || !viewport) return;
    const { w, h } = svgSize(el);
    const cw = viewport.clientWidth - 48;
    const ch = viewport.clientHeight - 48;
    const s = clamp(Math.min(cw / w, ch / h), MIN_SCALE, 1);
    setScale(s);
    setTx((viewport.clientWidth - w * s) / 2);
    setTy((viewport.clientHeight - h * s) / 2);
    apply();
  };

  const reset = () => {
    const el = svgEl();
    if (!el || !viewport) return;
    const { w, h } = svgSize(el);
    setScale(1);
    setTx((viewport.clientWidth - w) / 2);
    setTy((viewport.clientHeight - h) / 2);
    apply();
  };

  // Zoom by `factor` around a viewport-relative point (cx, cy).
  const zoomAt = (factor: number, cx: number, cy: number) => {
    const prev = scale();
    const next = clamp(prev * factor, MIN_SCALE, MAX_SCALE);
    if (next === prev) return;
    const ratio = next / prev;
    setTx(cx - (cx - tx()) * ratio);
    setTy(cy - (cy - ty()) * ratio);
    setScale(next);
    apply();
  };

  const zoomCenter = (factor: number) =>
    viewport && zoomAt(factor, viewport.clientWidth / 2, viewport.clientHeight / 2);

  const onWheel = (e: WheelEvent) => {
    e.preventDefault();
    const rect = viewport.getBoundingClientRect();
    zoomAt(e.deltaY < 0 ? 1.12 : 1 / 1.12, e.clientX - rect.left, e.clientY - rect.top);
  };

  // Drag to pan.
  let dragging = false;
  let lastX = 0;
  let lastY = 0;
  const onPointerDown = (e: PointerEvent) => {
    dragging = true;
    lastX = e.clientX;
    lastY = e.clientY;
    (e.currentTarget as HTMLElement).setPointerCapture(e.pointerId);
  };
  const onPointerMove = (e: PointerEvent) => {
    if (!dragging) return;
    setTx(tx() + (e.clientX - lastX));
    setTy(ty() + (e.clientY - lastY));
    lastX = e.clientX;
    lastY = e.clientY;
    apply();
  };
  const onPointerUp = () => {
    dragging = false;
  };

  // Load a fresh diagram whenever the store changes, then fit it.
  createEffect(() => {
    const svg = mermaidViewerSvg();
    if (!svg || !stage) return;
    stage.innerHTML = svg;
    const el = svgEl();
    if (el) {
      el.style.maxWidth = "none";
      el.removeAttribute("width");
      el.removeAttribute("height");
    }
    // Defer fit until layout has the real viewport size.
    requestAnimationFrame(fit);
  });

  const onKey = (e: KeyboardEvent) => {
    if (mermaidViewerSvg() && e.key === "Escape") closeMermaidViewer();
  };
  document.addEventListener("keydown", onKey);
  onCleanup(() => document.removeEventListener("keydown", onKey));

  // Build a standalone SVG string with explicit size for export.
  const exportSvgString = (): { svg: string; w: number; h: number } | null => {
    const el = svgEl();
    if (!el) return null;
    const { w, h } = svgSize(el);
    const clone = el.cloneNode(true) as SVGSVGElement;
    clone.setAttribute("xmlns", "http://www.w3.org/2000/svg");
    clone.setAttribute("width", String(w));
    clone.setAttribute("height", String(h));
    clone.style.maxWidth = "none";
    return { svg: `<?xml version="1.0" encoding="UTF-8"?>\n${clone.outerHTML}`, w, h };
  };

  const downloadSvg = async () => {
    const out = exportSvgString();
    if (!out) return;
    await exportFile("diagram.svg", "SVG", "svg", out.svg);
  };

  const downloadPng = async () => {
    const out = exportSvgString();
    if (!out) return;
    const scaleFactor = 2; // crisper raster
    const b64svg = btoa(unescape(encodeURIComponent(out.svg)));
    const img = new Image();
    const dataUrl = await new Promise<string | null>((resolve) => {
      img.onload = () => {
        const canvas = document.createElement("canvas");
        canvas.width = Math.round(out.w * scaleFactor);
        canvas.height = Math.round(out.h * scaleFactor);
        const ctx = canvas.getContext("2d");
        if (!ctx) return resolve(null);
        // Fill with the viewer background so themed (light-on-dark) diagrams
        // stay legible in the exported PNG.
        const bg = viewport ? getComputedStyle(viewport).backgroundColor : "";
        ctx.fillStyle = bg && bg !== "rgba(0, 0, 0, 0)" ? bg : "#ffffff";
        ctx.fillRect(0, 0, canvas.width, canvas.height);
        ctx.setTransform(scaleFactor, 0, 0, scaleFactor, 0, 0);
        ctx.drawImage(img, 0, 0);
        resolve(canvas.toDataURL("image/png"));
      };
      img.onerror = () => resolve(null);
      img.src = `data:image/svg+xml;base64,${b64svg}`;
    });
    if (!dataUrl) return;
    await exportFileBytes("diagram.png", "PNG", "png", dataUrl.split(",")[1]);
  };

  return (
    <Show when={mermaidViewerSvg()}>
      <div
        class="fixed inset-0 z-[110] flex flex-col bg-surface-0"
        onClick={(e) => {
          if (e.target === e.currentTarget) closeMermaidViewer();
        }}
      >
        {/* Toolbar */}
        <div class="pointer-events-none absolute right-4 top-4 z-10 flex gap-2">
          <TBtn label="Ajustar" onClick={fit}>{iconFit}</TBtn>
          <TBtn label="Reset (100%)" onClick={reset}>{iconReset}</TBtn>
          <TBtn label="Baixar SVG" onClick={downloadSvg}>{iconDownload}<span class="ml-1 text-[10px] font-semibold">SVG</span></TBtn>
          <TBtn label="Baixar PNG" onClick={downloadPng}>{iconDownload}<span class="ml-1 text-[10px] font-semibold">PNG</span></TBtn>
          <TBtn label="Fechar" onClick={closeMermaidViewer}>{iconClose}</TBtn>
        </div>
        <div class="pointer-events-none absolute bottom-4 right-4 z-10 flex flex-col gap-2">
          <TBtn label="Aproximar" onClick={() => zoomCenter(1.2)}>{iconPlus}</TBtn>
          <TBtn label="Afastar" onClick={() => zoomCenter(1 / 1.2)}>{iconMinus}</TBtn>
        </div>

        {/* Pan/zoom viewport */}
        <div
          ref={viewport}
          class="relative flex-1 overflow-hidden bg-surface-0"
          style={{ cursor: "grab", "touch-action": "none" }}
          onWheel={onWheel}
          onPointerDown={onPointerDown}
          onPointerMove={onPointerMove}
          onPointerUp={onPointerUp}
        >
          <div ref={stage} style={{ "transform-origin": "0 0", "will-change": "transform" }} />
        </div>
      </div>
    </Show>
  );
}

// ── Tiny toolbar button + inline icons (Icon.tsx has no zoom/download glyphs) ──
function TBtn(props: { label: string; onClick: () => void; children: JSX.Element }): JSX.Element {
  return (
    <button
      title={props.label}
      aria-label={props.label}
      onClick={props.onClick}
      class="pointer-events-auto inline-flex items-center rounded-md border border-border-subtle bg-surface-1 px-2.5 py-2 text-ink-muted shadow-sm transition-colors hover:bg-surface-2 hover:text-ink active:scale-95"
    >
      {props.children}
    </button>
  );
}

const svgAttrs = {
  width: "16",
  height: "16",
  viewBox: "0 0 24 24",
  fill: "none",
  stroke: "currentColor",
  "stroke-width": "2",
  "stroke-linecap": "round" as const,
  "stroke-linejoin": "round" as const,
};

const iconPlus = (
  <svg {...svgAttrs}><line x1="12" y1="5" x2="12" y2="19" /><line x1="5" y1="12" x2="19" y2="12" /></svg>
);
const iconMinus = (
  <svg {...svgAttrs}><line x1="5" y1="12" x2="19" y2="12" /></svg>
);
const iconReset = (
  <svg {...svgAttrs}><path d="M3 12a9 9 0 1 0 3-6.7L3 8" /><path d="M3 3v5h5" /></svg>
);
const iconFit = (
  <svg {...svgAttrs}><path d="M8 3H5a2 2 0 0 0-2 2v3" /><path d="M21 8V5a2 2 0 0 0-2-2h-3" /><path d="M3 16v3a2 2 0 0 0 2 2h3" /><path d="M16 21h3a2 2 0 0 0 2-2v-3" /></svg>
);
const iconDownload = (
  <svg {...svgAttrs}><path d="M21 15v4a2 2 0 0 1-2 2H5a2 2 0 0 1-2-2v-4" /><polyline points="7 10 12 15 17 10" /><line x1="12" y1="15" x2="12" y2="3" /></svg>
);
const iconClose = (
  <svg {...svgAttrs}><line x1="18" y1="6" x2="6" y2="18" /><line x1="6" y1="6" x2="18" y2="18" /></svg>
);

export default MermaidViewerModal;
