import { Component, For, Show, type Accessor, type Setter } from "solid-js";
import type { McpServerStatus } from "../../lib/ipc";

interface SettingsMcpProps {
  configMcpJson: Accessor<string>;
  setConfigMcpJson: Setter<string>;
  mcpJsonError: Accessor<string | null>;
  mcpStatuses: Accessor<Record<string, McpServerStatus>>;
  mcpTesting: Accessor<boolean>;
  onAddServer: () => void;
  onTestAll: () => void;
}

export const SettingsMcp: Component<SettingsMcpProps> = (props) => {
  return (
    <>
      {/* Header */}
      <div class="mb-2 flex items-center justify-between">
        <span class="text-sm font-medium text-ink">{"MCP Servers"}</span>
        <div class="flex gap-2">
          <button
            onClick={props.onAddServer}
            class="rounded-md border border-border-subtle bg-surface-2 px-2 py-1 text-xs text-ink hover:bg-surface-3"
          >
            {"+ Add server"}
          </button>
          <button
            onClick={props.onTestAll}
            disabled={props.mcpTesting()}
            class="rounded-md border border-border-subtle bg-surface-2 px-2 py-1 text-xs text-ink hover:bg-surface-3 disabled:opacity-50"
          >
            {props.mcpTesting() ? "Testing…" : "Test all"}
          </button>
        </div>
      </div>

      {/* JSON Editor */}
      <textarea
        value={props.configMcpJson()}
        onInput={(e) => props.setConfigMcpJson(e.currentTarget.value)}
        rows={10}
        spellcheck={false}
        class="mb-1 w-full rounded-md border border-border-subtle bg-surface-0 p-2 font-mono text-xs text-ink placeholder:text-ink-muted focus:border-accent focus:outline-none focus:ring-1 focus:ring-accent"
        classList={{ "border-red-500": !!props.mcpJsonError() }}
      />
      <Show when={props.mcpJsonError()}>
        <p class="mb-2 text-[11px] text-red-500">{props.mcpJsonError()}</p>
      </Show>

      {/* Hint */}
      <p class="mb-3 text-[11px] text-ink-faint">{"Edit the \"mcp\" block as JSON, keyed by server name. Example: { \"context7\": { \"type\": \"remote\", \"url\": \"https://mcp.context7.com/mcp\", \"headers\": { \"CONTEXT7_API_KEY\": \"...\" } } }"}</p>

      {/* Status list */}
      <Show when={Object.keys(props.mcpStatuses()).length > 0}>
        <div class="mb-4 space-y-1.5">
          <For each={Object.entries(props.mcpStatuses())}>
            {([name, status]) => (
              <div class="flex items-center gap-2 rounded-md border border-border-subtle bg-surface-1 px-2 py-1.5 text-xs">
                <span
                  class="h-2 w-2 shrink-0 rounded-full"
                  classList={{
                    "bg-green-500": status.connected,
                    "bg-red-500": !status.connected,
                  }}
                />
                <span class="font-medium text-ink">{name}</span>
                <span class="text-ink-faint">
                  {status.connected
                    ? `${String(status.toolCount)} tools`
                    : (status.error ?? "not connected")}
                </span>
              </div>
            )}
          </For>
        </div>
      </Show>
    </>
  );
};
