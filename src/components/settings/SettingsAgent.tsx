import { Component, Show, type Accessor, type Setter } from "solid-js";

interface SettingsAgentProps {
  yoloMode: Accessor<boolean>;
  setYoloMode: Setter<boolean>;
  yoloBlacklist: Accessor<string>;
  setYoloBlacklist: Setter<string>;
  workspaceConfigFields: Accessor<Set<string>>;
}

export const SettingsAgent: Component<SettingsAgentProps> = (props) => {
  return (
    <>
      <label class="mb-2 flex cursor-pointer items-center gap-2">
        <input
          type="checkbox"
          checked={props.yoloMode()}
          onChange={(e) => props.setYoloMode(e.currentTarget.checked)}
          class="h-4 w-4 rounded border-border-subtle bg-surface-0 text-accent focus:ring-accent"
          disabled={props.workspaceConfigFields().has("yolo_mode")}
        />
        <span class="text-sm font-medium text-ink">{"⚡ YOLO Mode (auto-approve all)"}</span>
        <span class="text-[11px] text-ink-faint">{"Auto-approves tool calls except those in the blacklist below."}</span>
        <Show when={props.workspaceConfigFields().has("yolo_mode")}>
          <span class="rounded border border-accent/40 bg-accent/10 px-1.5 py-px text-[10px] font-medium text-accent">
            {"Workspace"}
          </span>
        </Show>
        <Show when={!props.workspaceConfigFields().has("yolo_mode")}>
          <span class="rounded border border-border-subtle bg-surface-2 px-1.5 py-px text-[10px] text-ink-faint">
            {"Local"}
          </span>
        </Show>
      </label>

      <Show when={props.yoloMode()}>
        <label class="mb-1 block text-xs text-ink-muted">{"YOLO Blacklist (comma-separated tool names)"}</label>
        <textarea
          value={props.yoloBlacklist()}
          onInput={(e) => props.setYoloBlacklist(e.currentTarget.value)}
          placeholder="edit_file, bash"
          rows={2}
          class="mb-4 w-full rounded-md border border-border-subtle bg-surface-0 p-2 text-sm text-ink placeholder:text-ink-muted focus:border-accent focus:outline-none focus:ring-1 focus:ring-accent"
          classList={{
            "bg-surface-2 text-ink-muted pointer-events-none": props.workspaceConfigFields().has("yolo_blacklist"),
          }}
          disabled={props.workspaceConfigFields().has("yolo_blacklist")}
        />
        <p class="-mt-3 mb-4 text-[11px] text-ink-faint">{"These tools still require manual approval even with YOLO on. Ex: edit_file, bash"}</p>
      </Show>
    </>
  );
};
