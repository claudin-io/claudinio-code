import { createSignal, createMemo, For, Show, type Component } from "solid-js";
import { Icon } from "./Icon";
import {
  activeOps,
  lastFinishedOp,
  formatBytes,
  formatDuration,
  type NetOp,
} from "../lib/networkActivity";

/** Name + explanation for one network operation's source, shown on hover. */
export const NET_SOURCE_NAME: Record<string, string> = {
  llm_stream: "Model response",
  llm_classify: "Turn-completion check",
  llm_one_shot: "One-off model call",
  list_models: "Model list",
  auth: "Authentication",
  provider_catalog: "Provider catalog",
  skills_index: "Skills registry",
  skill_fetch: "Skill download",
  embedding_model_download: "Embedding model download",
  web_search: "Web search",
  mcp: "MCP server",
};

const NET_SOURCE_WHY: Record<string, string> = {
  llm_stream:
    "Streaming the agent's response from the model API. Runs until the turn finishes — including golden-loop cycles and subagents.",
  llm_classify: "Small model call that checks whether the agent's reply is really finished.",
  llm_one_shot: "Single non-streaming model request (enhancement, compaction, evaluation).",
  list_models: "Fetching the list of available models from the API.",
  auth: "Signing in or validating your API key.",
  provider_catalog: "Fetching the models.dev provider catalog.",
  skills_index: "Fetching the remote skills index.",
  skill_fetch: "Downloading a skill definition.",
  embedding_model_download:
    "Downloading the local semantic-search model (only when it's not cached yet).",
  web_search: "The agent is running a web search.",
  mcp: "Connecting to a remote MCP server.",
};

const sourceName = (op: NetOp) => NET_SOURCE_NAME[op.source] ?? op.source;
const sourceWhy = (op: NetOp) => NET_SOURCE_WHY[op.source] ?? "";

/**
 * Status-bar globe that lights up whenever the backend has an open network
 * connection. Hovering shows which subsystem is using the network and why —
 * so a forgotten agent run streaming in the background is visible at a glance.
 */
export const NetworkIndicator: Component<{
  placement?: 'top' | 'bottom';
  workspace?: string;
  onClick?: () => void;
}> = (props) => {
  const placement = () => props.placement ?? 'bottom';
  const [hovered, setHovered] = createSignal(false);
  const ops = activeOps;
  const active = createMemo(() => ops().length > 0);

  return (
    <div
      class="relative"
      onMouseEnter={() => setHovered(true)}
      onMouseLeave={() => setHovered(false)}
    >
      <button
        class="flex items-center gap-1 rounded px-2 py-1 text-[11px] hover:bg-surface-2"
        onClick={() => props.onClick?.()}
        classList={{
          "text-accent": active(),
          "text-ink-faint": !active(),
        }}
        aria-label={"Network activity"}
      >
        <Icon name="globe" class={"h-3.5 w-3.5" + (active() ? " animate-pulse" : "")} />
      </button>

      {/* Hover tooltip: one row per active connection (origin + why). */}
      <Show when={hovered()}>
        <div class="absolute right-0 z-50 w-80 rounded-lg border border-border-subtle bg-surface-1 p-3 shadow-modal" classList={{ 'top-full mt-1': placement() === 'bottom', 'bottom-full mb-1': placement() === 'top' }}>
          <p class="mb-2 text-[10px] font-semibold uppercase tracking-wider text-ink-faint">
            {"Network activity"}
          </p>
          <Show
            when={active()}
            fallback={
              <div>
                <p class="text-[12px] text-ink-muted">{"No network activity"}</p>
                <Show when={lastFinishedOp()}>
                  {(op) => (
                    <p class="mt-1 text-[11px] text-ink-faint">
                      {`Last: ${sourceName(op())}`}
                      <Show when={op().detail}> — {op().detail}</Show>
                    </p>
                  )}
                </Show>
              </div>
            }
          >
            <div class="space-y-2">
              <For each={ops()}>
                {(op) => (
                  <div class="border-l-2 border-accent/40 pl-2">
                    <p class="text-[12px] font-medium text-ink">
                      {sourceName(op)}
                      <span class="ml-1 font-normal text-ink-faint">
                        {formatDuration(op.elapsedMs)}
                        <Show when={op.bytes > 0}> · {formatBytes(op.bytes)}</Show>
                      </span>
                    </p>
                    <Show when={op.detail}>
                      <p class="break-words text-[11px] text-ink-muted">{op.detail}</p>
                    </Show>
                    <p class="text-[11px] leading-snug text-ink-faint">{sourceWhy(op)}</p>
                  </div>
                )}
              </For>
            </div>
          </Show>
        </div>
      </Show>
    </div>
  );
};
