# App UI kit — reconstructing the Claudinio Code interface

Every frame that shows the product's interface uses **the app's own design system**, not an
approximation. The app is Tailwind v4 + a CSS-variable token layer, and HyperFrames runs the
Tailwind v4 browser runtime — the same engine. So the app's `@theme` block drops straight into
a composition and the **real `class` strings from the SolidJS components resolve unchanged**.

Source of truth: `src/App.css` (`:root` at lines 4–48, `@theme inline` at line 486) and
`src/components/*.tsx` in the repository root. Everything below is copied from there.

---

## 1. Paste this into every frame that renders app UI

Goes **inside `<template>`**, never in `<head>` (the runtime discards everything outside the
template). Add it alongside your frame's own `<style>`.

```html
<style type="text/tailwindcss">
  @font-face {
    font-family: "Inter";
    src: url("assets/fonts/InterVariable.woff2") format("woff2");
    font-weight: 100 900;
    font-display: block;
  }
  @font-face {
    font-family: "JetBrains Mono";
    src: url("assets/fonts/JetBrainsMono-Regular.woff2") format("woff2");
    font-weight: 400;
    font-display: block;
  }
  @font-face {
    font-family: "JetBrains Mono";
    src: url("assets/fonts/JetBrainsMono-Medium.woff2") format("woff2");
    font-weight: 500;
    font-display: block;
  }

  /* ── the app's dark theme, verbatim from src/App.css :root ── */
  @theme {
    --color-surface-0: oklch(0.145 0.015 280);
    --color-surface-1: oklch(0.17 0.015 280);
    --color-surface-2: oklch(0.185 0.018 280);
    --color-surface-3: oklch(0.23 0.02 280);
    --color-surface: oklch(0.17 0.015 280);
    --color-border-subtle: oklch(0.28 0.02 280);
    --color-border-strong: oklch(0.33 0.02 280);
    --color-ink: oklch(0.95 0.01 280);
    --color-ink-muted: oklch(0.78 0.015 280);
    --color-ink-faint: oklch(0.65 0.02 280);
    --color-ink-subtle: oklch(0.58 0.02 280);
    --color-accent: oklch(0.62 0.19 277);
    --color-accent-strong: oklch(0.562 0.199 276.6);
    --color-accent-ink: oklch(0.99 0 0);
    --color-success: oklch(0.72 0.17 155);
    --color-warning: oklch(0.78 0.15 85);
    --color-danger: oklch(0.68 0.19 25);
    --font-sans: "Inter", ui-sans-serif, system-ui, sans-serif;
    --font-mono: "JetBrains Mono", ui-monospace, "SF Mono", monospace;
  }
</style>
```

With that present, `bg-surface-0`, `text-ink-faint`, `border-border-subtle`, `bg-accent/10`,
`text-accent`, `border-accent/60`, `text-danger`, `font-mono` all resolve exactly as they do in
the running app.

## 2. Scale — read this before you build

The app's real chrome is tiny by video standards: labels are `text-[11px]`, the diff is 13px
JetBrains Mono. A 1920×1080 frame needs its readable text at **≥ 24px effective**. So:

- **Build the UI markup at the app's real pixel sizes** (keep `text-[11px]` etc. — do not
  rewrite the classes), then put the whole surface in a wrapper and
  `transform: scale(N); transform-origin: <the anchor>` to bring it up to video scale.
- **When a surface is the subject** (the approval card, the diff, the transcript column, the
  toggle) build **only that surface**, not the whole window, and scale it **~2.2×–2.6×** so its
  11px labels land at 24–29px. This is what frames 3–6 do.
- **When the whole window is shown** (frame 2's zoom-out landing) it reads as a **shape**, not
  as text. Its interior text being small and unreadable at rest is correct and realistic —
  do not inflate the app's type to make it legible, that is what makes a mock look fake.
  Anything the viewer must actually read in that frame belongs in the video's own type layer
  (`display` / `lead` / the chip), outside the window.

Never mix the two type systems inside one element: the app surface uses the app's tokens and
Inter/JetBrains Mono at app sizes; the video's headline layer uses `frame.md`'s display ramp on
the cream ground. The contrast between them is deliberate.

## 3. Real markup — copied from the components

Class strings below are verbatim from `src/components/`. Content is representative, not
invented UI.

### Window shell — three panels (`App.tsx`, `FileTree.tsx`, `ChatPanel.tsx`)

```html
<div class="flex h-full w-full overflow-hidden rounded-xl border border-border-subtle bg-surface-0 font-sans">
  <aside class="w-[240px] shrink-0 border-r border-border-subtle bg-surface-1">
    <div class="px-3 py-2 text-[11px] font-semibold uppercase tracking-wider text-ink-faint">Explorer</div>
    <!-- tree rows: -->
    <div class="flex w-full flex-col gap-0.5 px-3 py-1.5 text-left hover:bg-surface-2">
      <span class="text-xs text-ink-muted">src/agent/session.rs</span>
    </div>
  </aside>
  <main class="flex-1 bg-surface-0"><!-- viewer / editor --></main>
  <section class="flex h-full w-[520px] shrink-0 flex-col border-l border-border-subtle bg-surface-0">
    <div class="relative flex items-center justify-between border-b border-border-subtle px-6 py-1.5">
      <span class="text-[11px] font-semibold uppercase tracking-wider text-ink-faint">Chat</span>
      <span class="font-mono text-[11px] text-ink-faint whitespace-nowrap">claudinio-sonnet</span>
    </div>
    <div class="flex flex-1 flex-col overflow-y-auto"><!-- timeline rows --></div>
  </section>
</div>
```

### Timeline — phase divider, thinking, tool call, cost footer (`ChatPanel.tsx`)

```html
<!-- phase divider -->
<div class="my-2 ml-6 text-[11px] text-ink-faint">— plan —</div>

<!-- thinking block -->
<div class="border-l-2 border-accent/60 pl-3">
  <span class="text-xs text-ink-muted">Reading the session loop to find where…</span>
</div>

<!-- tool call row -->
<div class="group flex items-center gap-1.5 rounded-md border border-border-subtle bg-surface-1 px-2 py-1 text-xs text-ink-muted">
  <span class="h-1.5 w-1.5 rounded-full bg-accent"></span>
  <span class="font-mono">semantic_search</span>
  <span class="font-mono text-[10px] text-ink-faint">"session persistence"</span>
</div>

<!-- running spinner (frame 6 stops this mid-rotation) -->
<div class="h-3.5 w-3.5 shrink-0 rounded-full border-2 border-amber-500/30 border-t-amber-500"></div>

<!-- token + cost footer -->
<div class="flex flex-wrap gap-2 border-t border-border-subtle px-6 py-2">
  <span class="font-mono text-[11px] text-ink-faint">12,481 tokens</span>
  <span class="inline-flex items-center gap-1 rounded-full bg-accent/10 px-2 py-0.5 text-[11px] text-accent">$0.04</span>
</div>
```

### Approval card — the gate (`ChatPanel.tsx`, the accent-bordered request strip)

```html
<div class="border-t border-accent/40 bg-accent/10 px-4 py-3.5">
  <div class="flex items-start justify-between gap-4">
    <div class="flex items-start gap-3 min-w-0">
      <span class="font-mono text-[11px] text-accent">bash</span>
      <span class="text-xs text-ink-muted">requires approval</span>
    </div>
  </div>
  <div class="mt-2 rounded-md border border-border-subtle bg-surface-2 px-3 py-2 font-mono text-xs text-ink">
    cargo test --all
  </div>
  <div class="mt-3 flex gap-2 shrink-0">
    <button class="rounded-md bg-accent px-3 py-1.5 text-xs text-accent-ink">Allow</button>
    <button class="rounded-md border border-border-subtle px-3 py-1.5 text-xs text-ink-muted">Deny</button>
  </div>
</div>
```

The danger and warning variants in the same file are
`border-t border-danger/30 bg-danger/5 px-4 py-3` and
`border-t border-amber-500/30 bg-amber-500/5 px-4 py-2.5` — use the amber one for the
"waiting on you" state if a second tone is needed.

### Diff — Monaco's real look (`DiffViewer.tsx`)

`DiffViewer` mounts Monaco with `fontSize: 13`, `fontFamily: "'JetBrains Mono', monospace"`,
`renderSideBySide: true`, `minimap: false`. Monaco itself will not run here, so reproduce its
appearance: two panes, per-line gutter numbers in `text-ink-faint`, removed lines on a
`bg-danger/10` row, added lines on a `bg-success/10` row, everything `font-mono text-[13px]`.

```html
<div class="grid grid-cols-2 gap-px overflow-hidden rounded-lg border border-border-subtle bg-border-subtle">
  <div class="bg-surface-1 p-3 font-mono text-[13px] leading-5">
    <div class="flex gap-3"><span class="text-ink-faint">42</span><span class="text-ink-muted">let mut history = …</span></div>
    <div class="flex gap-3 bg-danger/10"><span class="text-ink-faint">43</span><span class="text-ink">    run_phase(&amp;history)?;</span></div>
  </div>
  <div class="bg-surface-1 p-3 font-mono text-[13px] leading-5">
    <div class="flex gap-3"><span class="text-ink-faint">42</span><span class="text-ink-muted">let mut history = …</span></div>
    <div class="flex gap-3 bg-success/10"><span class="text-ink-faint">43</span><span class="text-ink">    run_phase(&amp;history, &amp;mode)?;</span></div>
  </div>
</div>
```

### Brain / Builder toggle — segmented control (`ChatPanel.tsx` input row)

```html
<div class="flex shrink-0 items-center rounded-md border border-border-subtle bg-surface-0 p-0.5">
  <button class="flex items-center gap-1 rounded px-2 py-1 text-[11px] text-ink-muted hover:bg-surface-2">Brain</button>
  <button class="flex items-center gap-1 rounded bg-accent px-2 py-1 text-[11px] text-accent-ink">Builder</button>
</div>
```

The active segment is the accent fill; the inactive one is bare. The real icons are
`codicon:thinking` and `carbon:tool-box` — draw them as simple inline SVG glyphs or omit them;
do not substitute an emoji.

### Search result rows (frame 4)

```html
<div class="flex w-full flex-col gap-0.5 px-3 py-1.5 text-left hover:bg-surface-2">
  <span class="font-mono text-xs text-ink">src/agent/queue.rs</span>
  <span class="font-mono text-[10px] text-ink-faint">pub fn enqueue(&amp;mut self, msg: Message)</span>
</div>
```

## 4. Rules

1. **Never invent a token.** If a color is not in the `@theme` block above, it does not belong
   on an app surface.
2. **Never restyle the app to match the video.** The app is dark; the film is cream. That
   contrast is the design — it is what makes the product the object in every frame.
3. **Copy class strings, don't paraphrase them.** `text-[11px] text-ink-faint` is the app's
   actual label style; writing `text-sm text-gray-400` breaks the fidelity this kit exists for.
4. **Only real content.** File paths, tool names and numbers must be plausible for this
   repository (`src/agent/session.rs`, `semantic_search`, `cargo test --all`). Never invent a
   product claim or a statistic — the facts are in `capture/extracted/asset-descriptions.md`.
5. **No browser chrome, no terminal, no traffic-light window buttons.** The window is a native
   desktop app.
