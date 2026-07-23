import { Component } from "solid-js";
import { Icon } from "./Icon";
import { THINKING_EFFORTS, type ThinkingEffort } from "../lib/ipc";

const EFFORT_LABEL: Record<ThinkingEffort, string> = {
  low: "Low",
  medium: "Medium",
  high: "High",
  xhigh: "X-High",
  max: "Max",
};

interface ThinkingEffortSliderProps {
  value: () => ThinkingEffort;
  onChange: (v: ThinkingEffort) => void;
  disabled?: () => boolean;
}

/// Compact 5-step slider (low → max) for the chat toolbar. The value is the
/// global thinking-effort setting; changes apply from the next message.
export const ThinkingEffortSlider: Component<ThinkingEffortSliderProps> = (props) => {
  const index = () => THINKING_EFFORTS.indexOf(props.value());
  return (
    <div
      class="flex shrink-0 items-center gap-1.5"
      title={`Thinking effort: ${EFFORT_LABEL[props.value()]} — applies from your next message`}
    >
      <Icon name="thinking-face" class="h-3.5 w-3.5 text-ink-faint" />
      <input
        type="range"
        min="0"
        max={THINKING_EFFORTS.length - 1}
        step="1"
        value={index()}
        disabled={props.disabled?.()}
        onInput={(e) => {
          const i = parseInt(e.currentTarget.value, 10);
          props.onChange(THINKING_EFFORTS[i] ?? "medium");
        }}
        class="h-2 w-20 cursor-pointer appearance-none rounded-lg accent-accent"
        aria-label={"Thinking effort"}
      />
      <span class="w-12 text-[10px] leading-none text-ink-faint">
        {EFFORT_LABEL[props.value()]}
      </span>
    </div>
  );
};
