import { Component, For, Show, type Accessor, type Setter } from "solid-js";
import ThemePicker from "../ThemePicker";
import { Icon } from "../Icon";

interface SettingsGeneralProps {
  keepAwake: Accessor<boolean>;
  setKeepAwake: Setter<boolean>;
  preferredIde: Accessor<string>;
  setPreferredIde: Setter<string>;
  codeIntelEnabled: Accessor<boolean>;
  setCodeIntelEnabled: Setter<boolean>;
  availableIdes: Accessor<string[]>;
}

export const SettingsGeneral: Component<SettingsGeneralProps> = (props) => {
  return (
    <>
      {/* Theme */}
      <label class="mb-1 block text-xs text-ink-muted">{"Theme"}</label>
      <div class="mb-4">
        <ThemePicker />
      </div>

      <div class="space-y-1">
        {/* Keep awake */}
        <label class="group flex cursor-pointer items-start gap-3 border-l-2 border-transparent py-2 pl-3 pr-1 transition-colors has-[:checked]:border-accent hover:border-accent/30">
          <div class="mt-0.5 flex shrink-0 items-center gap-2">
            <input
              type="checkbox"
              checked={props.keepAwake()}
              onChange={(e) => props.setKeepAwake(e.currentTarget.checked)}
              class="h-3.5 w-3.5 rounded border-border-subtle bg-surface-0 text-accent focus:ring-accent"
            />
            <Icon name="coffee-cup" class="h-4 w-4 text-ink-faint" stroke />
          </div>
          <div class="min-w-0">
            <span class="text-sm font-medium text-ink">{"Keep awake while working"}</span>
            <span class="block text-[11px] leading-relaxed text-ink-faint">{"Prevents the system from sleeping while a session is running (display can still turn off)."}</span>
          </div>
        </label>

        {/* Code intelligence */}
        <label class="group flex cursor-pointer items-start gap-3 border-l-2 border-transparent py-2 pl-3 pr-1 transition-colors has-[:checked]:border-accent hover:border-accent/30">
          <div class="mt-0.5 flex shrink-0 items-center gap-2">
            <input
              type="checkbox"
              checked={props.codeIntelEnabled()}
              onChange={(e) => props.setCodeIntelEnabled(e.currentTarget.checked)}
              class="h-3.5 w-3.5 rounded border-border-subtle bg-surface-0 text-accent focus:ring-accent"
            />
            <Icon name="brain" class="h-4 w-4 text-ink-faint" />
          </div>
          <div class="min-w-0">
            <span class="text-sm font-medium text-ink">{"Code intelligence"}</span>
            <span class="block text-[11px] leading-relaxed text-ink-faint">{"Enables LSP, FTS5 index, and semantic search. Turn off to save CPU and memory when not coding."}</span>
          </div>
        </label>
      </div>

      <hr class="mb-4 mt-4 border-border-subtle" />

      {/* Preferred IDE */}
      <label class="mb-1 block text-xs text-ink-muted">{"Preferred IDE"}</label>
      <Show
        when={props.availableIdes().length > 0}
        fallback={<p class="mb-1 text-[11px] text-ink-faint">{"No supported IDEs detected (VS Code / Cursor)."}</p>}
      >
        <select
          value={props.preferredIde()}
          onChange={(e) => props.setPreferredIde(e.currentTarget.value)}
          class="mb-1 w-full appearance-none rounded-md border border-border-subtle bg-surface-0 p-2 text-sm text-ink focus:border-accent focus:outline-none focus:ring-1 focus:ring-accent"
        >
          <For each={props.availableIdes()}>
            {(ide) => (
              <option value={ide} selected={props.preferredIde() === ide}>
                {ide === "vscode" ? "VS Code" : "Cursor"}
              </option>
            )}
          </For>
        </select>
      </Show>
      <p class="mb-4 text-[11px] text-ink-faint">{"Select which IDE to use when opening files. Auto-detected IDEs appear here."}</p>
    </>
  );
};
