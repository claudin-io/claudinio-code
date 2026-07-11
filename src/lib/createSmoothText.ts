import { createSignal, createEffect, onCleanup } from "solid-js";

/**
 * Balances an odd number of ``` fences by appending a closing one, so a
 * partially-revealed prefix never renders the rest of the message as a
 * giant code block. Everything else (bold/italic/backticks) is left as-is —
 * worst case a one-frame styling flash, not a jarring layout break.
 */
export function balanceMarkdown(prefix: string): string {
  const fenceCount = (prefix.match(/```/g) ?? []).length;
  if (fenceCount % 2 === 1) {
    return prefix.endsWith("\n") ? `${prefix}\`\`\`` : `${prefix}\n\`\`\``;
  }
  return prefix;
}

export interface SmoothTextOptions {
  /** Words per second at zero backlog. */
  baseWps?: number;
  /** Hard cap on words per second, however large the backlog gets. */
  maxWps?: number;
  /** Backlog (in words) that doubles the base rate. */
  backlogScale?: number;
  /** Rate multiplier once `finished()` is true, to drain quickly. */
  finishMultiplier?: number;
}

const DEFAULTS: Required<SmoothTextOptions> = {
  baseWps: 18,
  maxWps: 120,
  backlogScale: 30,
  finishMultiplier: 4.5,
};

/** Finds the end of the next word boundary (whitespace run) at or after `from`. */
function nextWordBoundary(text: string, from: number): number {
  let i = from;
  const len = text.length;
  // Skip any leading whitespace already revealed up to `from`.
  while (i < len && /\s/.test(text[i])) i++;
  // Consume one non-whitespace run.
  while (i < len && !/\s/.test(text[i])) i++;
  // Consume the whitespace that follows it, so the boundary lands right
  // before the next word (keeps trailing spaces attached to the word before).
  while (i < len && /\s/.test(text[i])) i++;
  return i;
}

function countWords(text: string): number {
  const trimmed = text.trim();
  if (!trimmed) return 0;
  return trimmed.split(/\s+/).length;
}

const TICK_MS = 33;

/**
 * Reveals `target()` word-by-word at an adaptive rate: faster when the
 * backlog (unrevealed text) grows, and sprinting once `finished()` flips
 * true so the display catches up instead of dumping instantly.
 */
export function createSmoothText(target: () => string, finished: () => boolean, opts?: SmoothTextOptions) {
  const cfg = { ...DEFAULTS, ...opts };
  const [revealedChars, setRevealedChars] = createSignal(0);

  let timerId: ReturnType<typeof setTimeout> | null = null;
  let lastTickTime: number | null = null;
  let carryFraction = 0;
  // Snapshots are meant to be cumulative (each new target extends the last).
  // Track the previous target so we can detect the exceptional case — retry,
  // new block, or Done text diverging from the streamed preview — and reset
  // instead of rendering a wrong/backwards prefix.
  let priorTarget = "";

  const displayed = () => target().slice(0, revealedChars());
  const isDrained = () => revealedChars() >= target().length;

  function stopLoop() {
    if (timerId !== null) {
      clearTimeout(timerId);
      timerId = null;
    }
    lastTickTime = null;
  }

  function tick() {
    timerId = null;
    const now = Date.now();
    const full = target();
    const revealed = revealedChars();
    if (revealed >= full.length) {
      lastTickTime = null;
      return;
    }
    const dt = lastTickTime === null ? 0 : (now - lastTickTime) / 1000;
    lastTickTime = now;

    const backlogWords = countWords(full.slice(revealed));
    const rate = Math.min(cfg.maxWps, cfg.baseWps * (1 + backlogWords / cfg.backlogScale)) * (finished() ? cfg.finishMultiplier : 1);

    carryFraction += rate * dt;
    let wordsToReveal = Math.floor(carryFraction);
    carryFraction -= wordsToReveal;

    let pos = revealed;
    while (wordsToReveal > 0 && pos < full.length) {
      pos = nextWordBoundary(full, pos);
      wordsToReveal--;
    }
    if (pos !== revealed) setRevealedChars(pos);

    timerId = setTimeout(tick, TICK_MS);
  }

  function startLoopIfNeeded() {
    if (timerId === null && revealedChars() < target().length) {
      carryFraction = 0;
      timerId = setTimeout(tick, TICK_MS);
    }
  }

  createEffect(() => {
    const full = target();
    if (!full.startsWith(priorTarget)) {
      carryFraction = 0;
      setRevealedChars(0);
    }
    priorTarget = full;
    startLoopIfNeeded();
  });

  onCleanup(stopLoop);

  function reset() {
    stopLoop();
    carryFraction = 0;
    setRevealedChars(0);
  }

  function flush() {
    stopLoop();
    setRevealedChars(target().length);
  }

  return { displayed, isDrained, reset, flush };
}
