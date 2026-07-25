import { Component, createSignal, createEffect, onCleanup, createMemo, For, Show, type Accessor, type Setter } from "solid-js";
import type { ConnectedProviderInfo, McpServerStatus, ModelGroup } from "../lib/ipc";
import { Icon } from "./Icon";
import { SettingsGeneral } from "./settings/SettingsGeneral";
import { SettingsModels } from "./settings/SettingsModels";
import { SettingsAccount } from "./settings/SettingsAccount";
import { SettingsAgent } from "./settings/SettingsAgent";
import { SettingsMcp } from "./settings/SettingsMcp";
import { SettingsRemote } from "./settings/SettingsRemote";
import { RemotePairing } from "./settings/RemotePairing";
import { RemotePolicyEditor } from "./settings/RemotePolicyEditor";
import { createRemoteSettings } from "../lib/createRemoteSettings";

interface SettingsPanelProps {
  showConfig: Accessor<boolean>;
  setShowConfig: Setter<boolean>;
  configBrainModel: Accessor<string>;
  setConfigBrainModel: Setter<string>;
  configBuilderModel: Accessor<string>;
  setConfigBuilderModel: Setter<string>;
  modelGroups: Accessor<ModelGroup[]>;
  configMaxParallelAgents: Accessor<number>;
  setConfigMaxParallelAgents: Setter<number>;
  configMaxRounds: Accessor<number | null>;
  setConfigMaxRounds: Setter<number | null>;
  configSubMaxRounds: Accessor<number | null>;
  setConfigSubMaxRounds: Setter<number | null>;
  configMaxGoldenCycles: Accessor<number | null>;
  setConfigMaxGoldenCycles: Setter<number | null>;
  configMaxGoldenStalls: Accessor<number | null>;
  setConfigMaxGoldenStalls: Setter<number | null>;
  configHandoffTokens: Accessor<number>;
  setConfigHandoffTokens: Setter<number>;
  configYoloMode: Accessor<boolean>;
  setConfigYoloMode: Setter<boolean>;
  configYoloBlacklist: Accessor<string>;
  setConfigYoloBlacklist: Setter<string>;
  configKeepAwake: Accessor<boolean>;
  setConfigKeepAwake: Setter<boolean>;
  configCodeIntelEnabled: Accessor<boolean>;
  setConfigCodeIntelEnabled: Setter<boolean>;
  configAutoCommitPlan: Accessor<boolean>;
  setConfigAutoCommitPlan: Setter<boolean>;
  configPreferredIde: Accessor<string>;
  setConfigPreferredIde: Setter<string>;
  availableIdes: Accessor<string[]>;
  configPlanSavePath: Accessor<string>;
  setConfigPlanSavePath: Setter<string>;
  workspaceConfigFields: Accessor<Set<string>>;
  accountLogin: Accessor<string | null>;
  hasApiKey: Accessor<boolean>;
  loggingIn: Accessor<boolean>;
  configApiKey: Accessor<string>;
  setConfigApiKey: Setter<string>;
  settingsApiKeyError: Accessor<string | null>;
  configMcpJson: Accessor<string>;
  setConfigMcpJson: Setter<string>;
  mcpJsonError: Accessor<string | null>;
  setMcpJsonError: Setter<string | null>;
  mcpStatuses: Accessor<Record<string, McpServerStatus>>;
  mcpTesting: Accessor<boolean>;
  setMcpTesting: Setter<boolean>;
  easterEggActive: Accessor<boolean>;
  configOverrideBaseUrl: Accessor<string>;
  setConfigOverrideBaseUrl: Setter<string>;
  configOverrideApiKey: Accessor<string>;
  setConfigOverrideApiKey: Setter<string>;
  providers: Accessor<Record<string, ConnectedProviderInfo>>;
  openrouterConnecting: Accessor<boolean>;
  providerError: Accessor<string | null>;
  onOpenrouterConnect: () => void;
  onOpenrouterCancel: () => void;
  onDisconnectProvider: (providerId: string) => void;
  onOpenProviderCatalog: () => void;
  saveConfig: () => Promise<void>;
  doLogin: () => Promise<void>;
  doLogout: () => Promise<void>;
  pickPlanPath: () => Promise<void>;
  addMcpServerTemplate: () => void;
  testAllMcpServers: () => Promise<void>;
  openSupportUrl: () => void;
  /// The workspace a paired browser would drive. `null` when none is open, which
  /// the remote panel reports as a reason rather than a failure.
  activeWorkspace: Accessor<string | null>;
}

type CategoryId = 'general' | 'models' | 'account' | 'agent' | 'mcp' | 'remote';

interface Category {
  id: CategoryId;
  icon: string;
  searchTerms: string[];
}

const CATEGORIES: Category[] = [
  { id: 'general', icon: 'sliders', searchTerms: ["Language","Theme","Keep awake while working","Plan save path","Preferred IDE","Auto-commit plan on finalize","Code intelligence"] },
  { id: 'models', icon: 'brain', searchTerms: ["Brain Model","Builder Model","Max rounds (main agent)","Max rounds (subagents)","Parallel subagents","Max golden cycles","Max golden stalls","Session handoff threshold","Anthropic URL Override","API Key Override"] },
  { id: 'account', icon: 'key', searchTerms: ["Account","Sign in with claudin.io","Sign out","API Key","Support","Providers","More providers\u2026","Connect","Access hundreds of models through one account, via OAuth."] },
  { id: 'agent', icon: 'construction-worker', searchTerms: ["\u26a1 YOLO Mode (auto-approve all)","YOLO Blacklist (comma-separated tool names)"] },
  { id: 'mcp', icon: 'package-process', searchTerms: ["MCP Servers","+ Add server","Test all"] },
  { id: 'remote', icon: 'globe', searchTerms: ["Remote access","Paired browsers","Create device key","Revoke","Revoked","What a paired browser may do"] },
];

function getCategoryLabel(id: CategoryId): string {
  const labels: Record<CategoryId, string> = {
    general: 'General',
    models: 'Models',
    account: 'Account',
    agent: 'Agent',
    mcp: 'MCP',
    remote: 'Remote',
  };
  return labels[id];
}

export const SettingsPanel: Component<SettingsPanelProps> = (props) => {
  const [activeCategory, setActiveCategory] = createSignal<CategoryId>('general');
  const [searchQuery, setSearchQuery] = createSignal('');
  const [panelWidth, setPanelWidth] = createSignal(850);
  const [phase, setPhase] = createSignal<'hidden' | 'entering' | 'visible' | 'exiting'>('hidden');
  let exitTimer: ReturnType<typeof setTimeout> | undefined;

  const remote = createRemoteSettings(props.activeWorkspace);

  /// Remote access is a compile-time feature, so a build can genuinely not have
  /// it. Until the device says otherwise the category is absent rather than
  /// disabled: a greyed-out tab invites someone to look for the switch that
  /// enables it, and there isn't one — it is decided by how the binary was built.
  const offeredCategories = createMemo(() =>
    CATEGORIES.filter((cat) => cat.id !== 'remote' || remote.available() === true),
  );

  const visibleCategories = createMemo(() => {
    const q = searchQuery().toLowerCase().trim();
    if (!q) return offeredCategories();
    return offeredCategories().filter((cat) => {
      if (getCategoryLabel(cat.id).toLowerCase().includes(q)) return true;
      return cat.searchTerms.some((term) => term.toLowerCase().includes(q));
    });
  });

  // Escape key listener
  createEffect(() => {
    if (phase() === 'hidden') return;
    const handler = (e: KeyboardEvent) => {
      if (e.key === 'Escape') props.setShowConfig(false);
    };
    document.addEventListener('keydown', handler);
    onCleanup(() => document.removeEventListener('keydown', handler));
  });

  // Resize handling
  let resizeStartX = 0;
  let resizeStartWidth = 0;

  const onResizeStart = (e: MouseEvent) => {
    e.preventDefault();
    resizeStartX = e.clientX;
    resizeStartWidth = panelWidth();
    document.addEventListener('mousemove', onResizeMove);
    document.addEventListener('mouseup', onResizeEnd);
  };

  const onResizeMove = (e: MouseEvent) => {
    const delta = resizeStartX - e.clientX;
    const newWidth = resizeStartWidth + delta;
    const clamped = Math.max(480, Math.min(newWidth, window.innerWidth * 0.9));
    setPanelWidth(clamped);
  };

  const onResizeEnd = () => {
    document.removeEventListener('mousemove', onResizeMove);
    document.removeEventListener('mouseup', onResizeEnd);
  };

  onCleanup(() => {
    document.removeEventListener('mousemove', onResizeMove);
    document.removeEventListener('mouseup', onResizeEnd);
    if (exitTimer) clearTimeout(exitTimer);
  });

  // Phase state machine: animate enter/exit based on showConfig
  createEffect(() => {
    if (props.showConfig()) {
      if (exitTimer) {
        clearTimeout(exitTimer);
        exitTimer = undefined;
      }
      if (phase() === 'hidden') {
        // Asked on open, not at startup: it is a file read the user has no reason
        // to pay for until they look at settings.
        void remote.probe();
        setPhase('entering');
        requestAnimationFrame(() => {
          setPhase('visible');
        });
      } else if (phase() === 'exiting') {
        // Interrupt exit: go straight to visible
        setPhase('visible');
      }
    } else {
      if (phase() === 'hidden') return;
      setPhase('exiting');
      exitTimer = setTimeout(() => {
        setPhase('hidden');
        setSearchQuery('');
        setActiveCategory('general');
      }, 200);
    }
  });

  return (
    <>
      {phase() !== 'hidden' && (
        <div
          class="settings-panel-overlay"
          classList={{
            'settings-overlay-enter': phase() === 'entering',
            'settings-overlay-exit': phase() === 'exiting',
          }}
          onClick={() => props.setShowConfig(false)}
        >
          <div
            class="settings-panel"
            classList={{
              'settings-panel-enter': phase() === 'entering',
              'settings-panel-exit': phase() === 'exiting',
            }}
            style={{ width: panelWidth() + 'px' }}
            onClick={(e) => e.stopPropagation()}
          >
          <div class="settings-panel-resize-handle" onMouseDown={onResizeStart} />

          <div class="settings-panel-sidebar">
            <div class="settings-search">
              <input
                type="text"
                value={searchQuery()}
                onInput={(e) => setSearchQuery(e.currentTarget.value)}
                placeholder="Search settings..."
              />
            </div>

            <For each={CATEGORIES}>
              {(cat) => (
                <Show when={visibleCategories().some((vc) => vc.id === cat.id)}>
                  <button
                    class="settings-category-item"
                    classList={{ active: activeCategory() === cat.id }}
                    onClick={() => setActiveCategory(cat.id)}
                  >
                    <Icon name={cat.icon as any} class="h-4 w-4 shrink-0" stroke />
                    <span>{getCategoryLabel(cat.id)}</span>
                  </button>
                </Show>
              )}
            </For>
          </div>

          <div class="settings-panel-body">
            <div class="settings-panel-content">
              <Show when={searchQuery() ? true : activeCategory() === 'general'}>
                <SettingsGeneral
                  keepAwake={props.configKeepAwake}
                  setKeepAwake={props.setConfigKeepAwake}
                  planSavePath={props.configPlanSavePath}
                  setPlanSavePath={props.setConfigPlanSavePath}
                  preferredIde={props.configPreferredIde}
                  setPreferredIde={props.setConfigPreferredIde}
                  autoCommitPlan={props.configAutoCommitPlan}
                  setAutoCommitPlan={props.setConfigAutoCommitPlan}
                  codeIntelEnabled={props.configCodeIntelEnabled}
                  setCodeIntelEnabled={props.setConfigCodeIntelEnabled}
                  pickPlanPath={props.pickPlanPath}
                  availableIdes={props.availableIdes}
                />
              </Show>

              <Show when={searchQuery() ? true : activeCategory() === 'models'}>
                <SettingsModels
                  brainModel={props.configBrainModel}
                  setBrainModel={props.setConfigBrainModel}
                  builderModel={props.configBuilderModel}
                  setBuilderModel={props.setConfigBuilderModel}
                  maxParallelAgents={props.configMaxParallelAgents}
                  setMaxParallelAgents={props.setConfigMaxParallelAgents}
                  maxRounds={props.configMaxRounds}
                  setMaxRounds={props.setConfigMaxRounds}
                  subMaxRounds={props.configSubMaxRounds}
                  setSubMaxRounds={props.setConfigSubMaxRounds}
                  maxGoldenCycles={props.configMaxGoldenCycles}
                  setMaxGoldenCycles={props.setConfigMaxGoldenCycles}
                  maxGoldenStalls={props.configMaxGoldenStalls}
                  setMaxGoldenStalls={props.setConfigMaxGoldenStalls}
                  handoffTokens={props.configHandoffTokens}
                  setHandoffTokens={props.setConfigHandoffTokens}
                  workspaceConfigFields={props.workspaceConfigFields}
                  easterEggActive={props.easterEggActive}
                  overrideBaseUrl={props.configOverrideBaseUrl}
                  setOverrideBaseUrl={props.setConfigOverrideBaseUrl}
                  overrideApiKey={props.configOverrideApiKey}
                  setOverrideApiKey={props.setConfigOverrideApiKey}
                  modelGroups={props.modelGroups}
                />
              </Show>

              <Show when={searchQuery() ? true : activeCategory() === 'account'}>
                <SettingsAccount
                  accountLogin={props.accountLogin}
                  hasApiKey={props.hasApiKey}
                  loggingIn={props.loggingIn}
                  configApiKey={props.configApiKey}
                  setConfigApiKey={props.setConfigApiKey}
                  settingsApiKeyError={props.settingsApiKeyError}
                  doLogin={props.doLogin}
                  doLogout={props.doLogout}
                  openSupportUrl={props.openSupportUrl}
                  providers={props.providers}
                  openrouterConnecting={props.openrouterConnecting}
                  providerError={props.providerError}
                  onOpenrouterConnect={props.onOpenrouterConnect}
                  onOpenrouterCancel={props.onOpenrouterCancel}
                  onDisconnectProvider={props.onDisconnectProvider}
                  onOpenProviderCatalog={props.onOpenProviderCatalog}
                />
              </Show>

              <Show when={searchQuery() ? true : activeCategory() === 'agent'}>
                <SettingsAgent
                  yoloMode={props.configYoloMode}
                  setYoloMode={props.setConfigYoloMode}
                  yoloBlacklist={props.configYoloBlacklist}
                  setYoloBlacklist={props.setConfigYoloBlacklist}
                  workspaceConfigFields={props.workspaceConfigFields}
                />
              </Show>

              <Show when={searchQuery() ? true : activeCategory() === 'mcp'}>
                <SettingsMcp
                  configMcpJson={props.configMcpJson}
                  setConfigMcpJson={props.setConfigMcpJson}
                  mcpJsonError={props.mcpJsonError}
                  mcpStatuses={props.mcpStatuses}
                  mcpTesting={props.mcpTesting}
                  onAddServer={props.addMcpServerTemplate}
                  onTestAll={props.testAllMcpServers}
                />
              </Show>

              <Show
                when={
                  remote.available() === true &&
                  (searchQuery() ? true : activeCategory() === 'remote')
                }
              >
                <SettingsRemote
                  deviceKey={remote.deviceKey}
                  policy={remote.policy}
                  pairings={remote.pairings}
                  revoked={remote.revoked}
                  error={remote.error}
                  busy={remote.busy}
                  onCreateIdentity={remote.createIdentity}
                  onRevoke={remote.revoke}
                  onUnrevoke={remote.unrevoke}
                  onRename={remote.rename}
                  policyEditor={
                    <RemotePolicyEditor
                      policy={remote.policy}
                      running={remote.running}
                      busy={remote.busy}
                      activeWorkspace={props.activeWorkspace}
                      onSave={remote.setPolicy}
                      onDisable={remote.disable}
                    />
                  }
                  pairing={
                    <RemotePairing
                      code={remote.code}
                      pending={remote.pending}
                      outcome={remote.outcome}
                      busy={remote.busy}
                      blocked={remote.blocked}
                      onStart={remote.startPairing}
                      onCancel={remote.cancelPairing}
                      onConfirm={remote.confirmPairing}
                    />
                  }
                />
              </Show>
            </div>

            {/* Nothing on the Remote tab is staged: revoking a pairing has
                already happened by the time the footer is looked at, so offering
                Cancel there would promise an undo that does not exist. */}
            <div class="settings-panel-footer">
              <Show
                when={!searchQuery() && activeCategory() === 'remote'}
                fallback={
                  <>
                    <button
                      onClick={() => props.setShowConfig(false)}
                      class="rounded-md border border-border-subtle bg-surface-2 px-3 py-1.5 text-sm text-ink hover:bg-surface-3"
                    >
                      {"Cancel"}
                    </button>
                    <button
                      onClick={() => props.saveConfig()}
                      class="rounded-md bg-accent px-3 py-1.5 text-sm font-medium text-accent-ink hover:bg-accent-hover"
                    >
                      {"Save"}
                    </button>
                  </>
                }
              >
                <button
                  onClick={() => props.setShowConfig(false)}
                  class="rounded-md border border-border-subtle bg-surface-2 px-3 py-1.5 text-sm text-ink hover:bg-surface-3"
                >
                  {"Close"}
                </button>
              </Show>
            </div>
          </div>
        </div>
      </div>
      )}
    </>
  );
};
