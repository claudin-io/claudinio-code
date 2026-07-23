import { Component, Show, createSignal, type Accessor, type Setter } from "solid-js";
import { Icon } from "../Icon";
import { ModelSelect } from "../ModelSelect";
import type { ModelGroup } from "../../lib/ipc";

interface SettingsModelsProps {
  brainModel: Accessor<string>;
  setBrainModel: Setter<string>;
  builderModel: Accessor<string>;
  setBuilderModel: Setter<string>;
  maxParallelAgents: Accessor<number>;
  setMaxParallelAgents: Setter<number>;
  maxRounds: Accessor<number | null>;
  setMaxRounds: Setter<number | null>;
  subMaxRounds: Accessor<number | null>;
  setSubMaxRounds: Setter<number | null>;
  maxGoldenCycles: Accessor<number | null>;
  setMaxGoldenCycles: Setter<number | null>;
  maxGoldenStalls: Accessor<number | null>;
  setMaxGoldenStalls: Setter<number | null>;
  handoffTokens: Accessor<number>;
  setHandoffTokens: Setter<number>;
  workspaceConfigFields: Accessor<Set<string>>;
  easterEggActive: Accessor<boolean>;
  overrideBaseUrl: Accessor<string>;
  setOverrideBaseUrl: Setter<string>;
  overrideApiKey: Accessor<string>;
  setOverrideApiKey: Setter<string>;
  modelGroups: Accessor<ModelGroup[]>;
}

export const SettingsModels: Component<SettingsModelsProps> = (props) => {
  const [showAdvanced, setShowAdvanced] = createSignal(false);

  const sourceBadge = (field: string) => (
    <Show
      when={props.workspaceConfigFields().has(field)}
      fallback={
        <span class="rounded border border-border-subtle bg-surface-2 px-1.5 py-px text-[10px] text-ink-faint">
          {"Local"}
        </span>
      }
    >
      <span class="rounded border border-accent/40 bg-accent/10 px-1.5 py-px text-[10px] font-medium text-accent">
        {"Workspace"}
      </span>
    </Show>
  );

  const workspaceDisabledClass = (field: string) => ({
    "bg-surface-2 text-ink-muted pointer-events-none": props.workspaceConfigFields().has(field),
    "bg-surface-0": !props.workspaceConfigFields().has(field),
  });

  return (
    <>
      {/* 1. Brain Model + Builder Model */}
      <div class="grid grid-cols-2 gap-x-4 mb-4">
        {/* Brain model */}
        <div>
          <div class="flex items-center gap-2 mb-1">
            <label class="block text-xs text-ink-muted">{"Brain Model"}</label>
            {sourceBadge("brain_model")}
          </div>

          <Show
            when={props.easterEggActive()}
            fallback={
              <ModelSelect
                value={props.brainModel}
                onChange={(m) => props.setBrainModel(m)}
                groups={props.modelGroups}
                disabled={() => props.workspaceConfigFields().has("brain_model")}
              />
            }
          >
            <input
              type="text"
              value={props.brainModel()}
              onInput={(e) => props.setBrainModel(e.currentTarget.value)}
              placeholder="claudius"
              class="w-full rounded-md border border-border-subtle bg-surface-0 p-2 text-sm text-ink placeholder:text-ink-muted focus:border-accent focus:outline-none focus:ring-1 focus:ring-accent"
            />
          </Show>
        </div>

        {/* Builder model */}
        <div>
          <div class="flex items-center gap-2 mb-1">
            <label class="block text-xs text-ink-muted">{"Builder Model"}</label>
            {sourceBadge("builder_model")}
          </div>

          <Show
            when={props.easterEggActive()}
            fallback={
              <ModelSelect
                value={props.builderModel}
                onChange={(m) => props.setBuilderModel(m)}
                groups={props.modelGroups}
                disabled={() => props.workspaceConfigFields().has("builder_model")}
              />
            }
          >
            <input
              type="text"
              value={props.builderModel()}
              onInput={(e) => props.setBuilderModel(e.currentTarget.value)}
              placeholder="claudinio"
              class="w-full rounded-md border border-border-subtle bg-surface-0 p-2 text-sm text-ink placeholder:text-ink-muted focus:border-accent focus:outline-none focus:ring-1 focus:ring-accent"
            />
          </Show>
        </div>
      </div>

      {/* 2. Max Parallel Agents slider */}
      <div class="mb-4">
        <div class="flex items-center gap-2 mb-1">
          <label class="block text-xs text-ink-muted">
            {"Parallel subagents"}: {props.maxParallelAgents()}
          </label>
          {sourceBadge("max_parallel_agents")}
        </div>

        <div class="flex items-center gap-2">
          <span class="text-[10px] text-ink-faint w-10 text-right">{"slower"}</span>
          <input
            type="range"
            min="1"
            max="8"
            step="1"
            value={props.maxParallelAgents()}
            onInput={(e) => props.setMaxParallelAgents(parseInt(e.currentTarget.value, 10) || 4)}
            disabled={props.workspaceConfigFields().has("max_parallel_agents")}
            class="flex-1 h-2 rounded-lg appearance-none cursor-pointer accent-accent"
            classList={{
              "opacity-50 cursor-not-allowed": props.workspaceConfigFields().has("max_parallel_agents"),
            }}
          />
          <span class="text-[10px] text-ink-faint w-10">{"faster"}</span>
        </div>

        <p class="mt-1 mb-0 text-[11px] text-ink-faint">{"Higher = faster, but uses more of your hourly rate limit. Lower = slower, but conserves it."}</p>
      </div>

      <hr class="mb-4 border-border-subtle" />

      {/* 3. Max Rounds + Sub Max Rounds */}
      <div class="grid grid-cols-2 gap-x-4 gap-y-2 mb-4">
        <div>
          <div class="flex items-center gap-2 mb-1">
            <label class="block text-xs text-ink-muted">{"Max rounds (main agent)"}</label>
            {sourceBadge("max_rounds")}
          </div>

          <input
            type="number"
            min="0"
            value={props.maxRounds() ?? ""}
            onInput={(e) => {
              const v = e.currentTarget.value;
              props.setMaxRounds(v === "" ? null : Math.max(1, parseInt(v, 10) || 1));
            }}
            placeholder={"Unlimited (default)"}
            class="mb-1 w-full rounded-md border border-border-subtle p-2 text-sm text-ink placeholder:text-ink-muted focus:border-accent focus:outline-none focus:ring-1 focus:ring-accent"
            classList={workspaceDisabledClass("max_rounds")}
            disabled={props.workspaceConfigFields().has("max_rounds")}
          />
          <p class="mb-0 text-[11px] text-ink-faint">{"Leave empty for unlimited. Sets how many tool calls the agent may make per submission."}</p>
        </div>

        <div>
          <div class="flex items-center gap-2 mb-1">
            <label class="block text-xs text-ink-muted">{"Max rounds (subagents)"}</label>
            {sourceBadge("sub_max_rounds")}
          </div>

          <input
            type="number"
            min="0"
            value={props.subMaxRounds() ?? ""}
            onInput={(e) => {
              const v = e.currentTarget.value;
              props.setSubMaxRounds(v === "" ? null : Math.max(1, parseInt(v, 10) || 1));
            }}
            placeholder={"Unlimited (default)"}
            class="mb-1 w-full rounded-md border border-border-subtle p-2 text-sm text-ink placeholder:text-ink-muted focus:border-accent focus:outline-none focus:ring-1 focus:ring-accent"
            classList={workspaceDisabledClass("sub_max_rounds")}
            disabled={props.workspaceConfigFields().has("sub_max_rounds")}
          />
          <p class="mb-0 text-[11px] text-ink-faint">{"Leave empty for unlimited. Sets the limit per subagent."}</p>
        </div>

        {/* 4. Max Golden Cycles + Max Golden Stalls */}
        <div>
          <label class="mb-1 block text-xs text-ink-muted">{"Max golden cycles"}</label>
          <input
            type="number"
            min="0"
            value={props.maxGoldenCycles() ?? ""}
            onInput={(e) => {
              const v = e.currentTarget.value;
              props.setMaxGoldenCycles(v === "" ? null : Math.max(0, parseInt(v, 10) || 0));
            }}
            placeholder="5"
            class="mb-1 w-full rounded-md border border-border-subtle bg-surface-0 p-2 text-sm text-ink placeholder:text-ink-muted focus:border-accent focus:outline-none focus:ring-1 focus:ring-accent"
          />
          <p class="mb-0 text-[11px] text-ink-faint">{"How many automatic Brain↔Builder cycles to run while golden goals (<goal> tags) are pending. Empty = 5."}</p>
        </div>

        <div>
          <label class="mb-1 block text-xs text-ink-muted">{"Max golden stalls"}</label>
          <input
            type="number"
            min="0"
            value={props.maxGoldenStalls() ?? ""}
            onInput={(e) => {
              const v = e.currentTarget.value;
              props.setMaxGoldenStalls(v === "" ? null : Math.max(0, parseInt(v, 10) || 0));
            }}
            placeholder="2"
            class="mb-1 w-full rounded-md border border-border-subtle bg-surface-0 p-2 text-sm text-ink placeholder:text-ink-muted focus:border-accent focus:outline-none focus:ring-1 focus:ring-accent"
          />
          <p class="mb-0 text-[11px] text-ink-faint">{"Stop the golden loop after this many cycles without progress. Empty = 2."}</p>
        </div>
      </div>

      {/* 5. Handoff Threshold slider */}
      <div class="mb-4">
        <div class="flex items-center gap-2 mb-1">
          <label class="block text-xs text-ink-muted">
            {"Session handoff threshold"}
            <span class="ml-2 font-mono text-[11px] text-ink-faint">
              {Math.round(props.handoffTokens() / 1000)}k tokens
            </span>
          </label>
          {sourceBadge("handoff_context_tokens")}
        </div>

        <div class="flex items-center gap-2">
          <span class="text-[10px] text-ink-faint w-10 text-right">120k</span>
          <input
            type="range"
            min="120000"
            max="256000"
            step="8000"
            value={props.handoffTokens()}
            onInput={(e) => props.setHandoffTokens(parseInt(e.currentTarget.value, 10))}
            disabled={props.workspaceConfigFields().has("handoff_context_tokens")}
            class="flex-1 h-2 rounded-lg appearance-none cursor-pointer handoff-slider"
            classList={{
              "opacity-50 cursor-not-allowed": props.workspaceConfigFields().has("handoff_context_tokens"),
            }}
          />
          <span class="text-[10px] text-ink-faint w-10">256k</span>
        </div>

        {/* Context rot risk bar */}
        <div class="mt-1 flex items-center gap-2">
          <span class="text-[10px] text-ink-faint">{"lower risk"}</span>
          <div class="flex-1 flex items-center">
            <div class="flex-1 border-t border-border-subtle"></div>
            <span class="mx-2 text-[10px] text-ink-faint whitespace-nowrap">{"Context Rot Risk"}</span>
            <div class="flex-1 border-t border-border-subtle"></div>
          </div>
          <span class="text-[10px] text-ink-faint">{"higher risk"}</span>
        </div>
      </div>

      <hr class="mb-4 border-border-subtle" />

      {/* 6. Advanced (collapsed subsection, easter egg) */}
      <Show when={props.easterEggActive()}>
        <button
          type="button"
          onClick={() => setShowAdvanced((v) => !v)}
          class="mb-3 flex w-full items-center gap-2 text-xs text-ink-muted hover:text-ink"
        >
          <Icon
            name="chevron-down"
            class={`h-3 w-3 shrink-0 transition-transform ${showAdvanced() ? "rotate-180" : ""}`}
          />
          <span>Advanced</span>
        </button>

        <Show when={showAdvanced()}>
          <div class="mb-4">
            <label class="mb-1 block text-xs text-ink-muted">{"Anthropic URL Override"}</label>
            <input
              type="text"
              value={props.overrideBaseUrl()}
              onInput={(e) => props.setOverrideBaseUrl(e.currentTarget.value)}
              placeholder="https://api.claudin.io"
              class="w-full rounded-md border border-border-subtle bg-surface-0 p-2 text-sm text-ink placeholder:text-ink-muted focus:border-accent focus:outline-none focus:ring-1 focus:ring-accent"
            />
            <p class="mt-1 mb-3 text-[11px] text-ink-faint">{"Overrides the API endpoint for LLM calls only. Leave empty to use default."}</p>

            <label class="mb-1 block text-xs text-ink-muted">{"API Key Override"}</label>
            <input
              type="password"
              value={props.overrideApiKey()}
              onInput={(e) => props.setOverrideApiKey(e.currentTarget.value)}
              placeholder="sk-ant-..."
              class="w-full rounded-md border border-border-subtle bg-surface-0 p-2 text-sm text-ink placeholder:text-ink-muted focus:border-accent focus:outline-none focus:ring-1 focus:ring-accent"
            />
            <p class="mt-1 text-[11px] text-ink-faint">{"Overrides the API key for LLM calls only. Leave empty to use the signed-in key."}</p>
          </div>
        </Show>
      </Show>
    </>
  );
};
