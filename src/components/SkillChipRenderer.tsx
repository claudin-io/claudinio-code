import { createMemo, For, type Component } from "solid-js";

interface Block {
  type: "text" | "markup" | "chip";
  value: string;
}

interface SkillChipRendererProps {
  text: string;
}

/**
 * Renders `<skill>name</skill>` as a styled inline chip.
 * The raw `<skill>` and `</skill>` markup is hidden; plain text is visible.
 *
 * This is an overlay on a transparent-text textarea. Small wrapping
 * differences between the overlay (inline chips) and the textarea
 * (raw monospace-like text) are acceptable for a chat input.
 */
export const SkillChipRenderer: Component<SkillChipRendererProps> = (props) => {
  const blocks = createMemo<Block[]>(() => {
    const t = props.text;
    const out: Block[] = [];
    let i = 0;

    while (i < t.length) {
      if (t.slice(i, i + 7) === "<skill>") {
        const ci = t.indexOf("</skill>", i + 7);
        if (ci !== -1) {
          out.push({ type: "chip", value: t.slice(i + 7, ci) });
          i = ci + 8;
          continue;
        }
      }
      if (t.slice(i, i + 8) === "</skill>") {
        out.push({ type: "markup", value: "</skill>" });
        i += 8;
        continue;
      }
      if (t.slice(i, i + 7) === "<skill>") {
        out.push({ type: "markup", value: "<skill>" });
        i += 7;
        continue;
      }
      const nx = t.indexOf("<", i + 1);
      if (nx === -1) {
        out.push({ type: "text", value: t.slice(i) });
        i = t.length;
      } else {
        out.push({ type: "text", value: t.slice(i, nx) });
        i = nx;
      }
    }
    return out;
  });

  return (
    <For each={blocks()}>
      {(b) => {
        if (b.type === "chip") {
          return (
            <span class="inline-flex items-center gap-1 rounded-md bg-accent/15 pl-1.5 pr-2 py-[1px] text-[12px] font-medium text-accent select-none leading-[18px]">
              <svg class="h-3 w-3 shrink-0" viewBox="0 0 16 16" fill="none" stroke="currentColor" stroke-width="1.5" stroke-linecap="round" stroke-linejoin="round">
                <rect x="1" y="3" width="14" height="10" rx="2" />
                <path d="M5 7v2" />
                <path d="M8 6v3" />
                <path d="M11 8v1" />
              </svg>
              {b.value}
            </span>
          );
        }
        if (b.type === "markup") {
          return <span class="text-transparent select-none">{b.value}</span>;
        }
        return <span class="text-ink whitespace-pre-wrap">{b.value}</span>;
      }}
    </For>
  );
};
