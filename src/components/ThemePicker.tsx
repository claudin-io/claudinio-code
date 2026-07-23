import { For, Show, type Component } from "solid-js";
import {
  preference,
  resolvedTheme,
  setThemePreference,
  themeMetadata,
  ALL_THEMES,
  type ThemeId,
  type ThemeMeta,
} from "../lib/theme";
import { Icon } from "./Icon";

/** The "system" option rendered as the first card */
const SYSTEM_META: ThemeMeta = {
  label: "System",
  category: "dark",
  previewColors: [
    "oklch(0.145 0.015 280)",
    "oklch(0.95 0.01 280)",
    "oklch(0.98 0.003 280)",
    "oklch(0.18 0.02 280)",
    "oklch(0.50 0.20 277)",
  ],
};

function getThemeMeta(id: ThemeId): ThemeMeta {
  return themeMetadata[id] ?? SYSTEM_META;
}

const ThemePicker: Component = () => {
  return (
    <div class="grid grid-cols-4 gap-2">
      {/* System card */}
      <button
        onClick={() => setThemePreference("system")}
        class={`group relative flex flex-col items-start gap-1.5 rounded-lg border p-2.5 text-left transition-all duration-120 ${
          preference() === "system"
            ? "border-accent-strong ring-1 ring-accent-strong"
            : "border-border-subtle hover:border-border-strong hover:bg-surface-2"
        } bg-surface-1`}
      >
        {/* Preview swatches — half dark, half light gradient for "system" */}
        <div class="flex gap-0.5 overflow-hidden rounded-[4px]">
          <div class="h-3 w-4 rounded-l-[3px]" style="background: oklch(0.145 0.015 280)" />
          <div class="h-3 w-4" style="background: oklch(0.18 0.02 280)" />
          <div class="h-3 w-4" style="background: linear-gradient(90deg, oklch(0.145 0.015 280) 50%, oklch(0.98 0.003 280) 50%)" />
          <div class="h-3 w-4" style="background: linear-gradient(90deg, oklch(0.95 0.01 280) 50%, oklch(0.18 0.02 280) 50%)" />
          <div class="h-3 w-4 rounded-r-[3px]" style="background: linear-gradient(90deg, oklch(0.62 0.19 277) 50%, oklch(0.50 0.20 277) 50%)" />
        </div>
        {/* Label */}
        <span class="text-[11px] font-medium leading-none text-ink">
          {"System"}
        </span>
        {/* Badge showing resolved theme */}
        <span class="text-[9px] leading-none text-ink-faint">
          → {getThemeMeta(resolvedTheme()).label}
        </span>
        {/* Check icon when selected */}
        <Show when={preference() === "system"}>
          <span class="absolute right-1.5 top-1.5 flex h-4 w-4 items-center justify-center rounded-full bg-accent-strong text-[10px] text-accent-ink">
            <Icon name="check" class="h-2.5 w-2.5" />
          </span>
        </Show>
      </button>

      {/* Theme cards */}
      <For each={ALL_THEMES}>
        {(id) => {
          const meta = getThemeMeta(id);
          return (
            <button
              onClick={() => setThemePreference(id)}
              class={`group relative flex flex-col items-start gap-1.5 rounded-lg border p-2.5 text-left transition-all duration-120 ${
                preference() === id
                  ? "border-accent-strong ring-1 ring-accent-strong"
                  : "border-border-subtle hover:border-border-strong hover:bg-surface-2"
              } bg-surface-1`}
            >
              {/* Preview swatches — 5 representative colours */}
              <div class="flex gap-0.5 overflow-hidden rounded-[4px]">
                <For each={meta.previewColors}>
                  {(color, i) => (
                    <div
                      class={`h-3 ${i() === 0 ? "w-4 rounded-l-[3px]" : i() === 4 ? "w-4 rounded-r-[3px]" : "w-4"}`}
                      style={`background: ${color}`}
                    />
                  )}
                </For>
              </div>
              {/* Name */}
              <span class="text-[11px] font-medium leading-none text-ink">
                {meta.label}
              </span>
              {/* Category hint */}
              <span class="text-[9px] leading-none text-ink-faint">
                {meta.category === "dark" ? "🌙" : "☀️"}
              </span>
              {/* Check icon when selected */}
              <Show when={preference() === id}>
                <span class="absolute right-1.5 top-1.5 flex h-4 w-4 items-center justify-center rounded-full bg-accent-strong text-[10px] text-accent-ink">
                  <Icon name="check" class="h-2.5 w-2.5" />
                </span>
              </Show>
            </button>
          );
        }}
      </For>
    </div>
  );
};

export default ThemePicker;