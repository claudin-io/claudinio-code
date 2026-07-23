import { Component, For, Show, createMemo, type Accessor, type Setter } from "solid-js";
import { Icon } from "../Icon";
import type { ConnectedProviderInfo } from "../../lib/ipc";

interface SettingsAccountProps {
  accountLogin: Accessor<string | null>;
  hasApiKey: Accessor<boolean>;
  loggingIn: Accessor<boolean>;
  configApiKey: Accessor<string>;
  setConfigApiKey: Setter<string>;
  settingsApiKeyError: Accessor<string | null>;
  doLogin: () => void;
  doLogout: () => void;
  openSupportUrl: () => void;
  providers: Accessor<Record<string, ConnectedProviderInfo>>;
  openrouterConnecting: Accessor<boolean>;
  providerError: Accessor<string | null>;
  onOpenrouterConnect: () => void;
  onOpenrouterCancel: () => void;
  onDisconnectProvider: (providerId: string) => void;
  onOpenProviderCatalog: () => void;
}

const experimentalBadge = () => (
  <span class="rounded border border-amber-500/40 bg-amber-500/10 px-1.5 py-px text-[10px] text-amber-600">
    {"Experimental"}
  </span>
);

export const SettingsAccount: Component<SettingsAccountProps> = (props) => {
  const openrouterConnected = () => Boolean(props.providers()["openrouter"]?.connected);
  /** Catalog providers connected via the modal (everything but OpenRouter). */
  const otherProviders = createMemo(() =>
    Object.entries(props.providers())
      .filter(([id, p]) => id !== "openrouter" && p.connected)
      .sort(([a], [b]) => a.localeCompare(b)),
  );

  return (
    <>
      <label class="mb-1 block text-xs text-ink-muted">{"Providers"}</label>

      {/* 1. Claudinio — primary, recommended */}
      <div class="mb-3 rounded-md border border-accent/50 bg-accent/5 p-3">
        <div class="mb-2 flex items-center gap-2">
          <span class="text-sm font-semibold text-ink">Claudinio</span>
          <span class="rounded border border-accent/40 bg-accent/10 px-1.5 py-px text-[10px] font-medium text-accent">
            {"Recommended"}
          </span>
        </div>
        <Show
          when={props.accountLogin() || props.hasApiKey()}
          fallback={
            <div class="space-y-2">
              <button
                onClick={props.doLogin}
                disabled={props.loggingIn()}
                class="w-full rounded-md bg-accent p-2 text-sm font-semibold text-white hover:opacity-90 disabled:opacity-50"
              >
                {props.loggingIn() ? "Waiting for browser sign-in…" : "Sign in with claudin.io"}
              </button>
              <label class="block text-xs text-ink-muted">{"API Key"}</label>
              <input
                type="password"
                value={props.configApiKey()}
                onInput={(e) => props.setConfigApiKey(e.currentTarget.value)}
                placeholder="sk-..."
                class="w-full rounded-md border border-border-subtle bg-surface-0 p-2 text-sm text-ink placeholder:text-ink-muted focus:border-accent focus:outline-none focus:ring-1 focus:ring-accent"
              />
              <Show when={props.settingsApiKeyError()}>
                <div class="text-sm text-red-400">{props.settingsApiKeyError()}</div>
              </Show>
            </div>
          }
        >
          <div class="flex items-center justify-between rounded-md border border-border-subtle bg-surface-0 p-2 text-sm">
            <span class="truncate text-ink">{"Signed In"}</span>
            <button
              onClick={props.doLogout}
              class="ml-2 shrink-0 text-xs text-ink-muted hover:text-ink hover:underline"
            >
              {"Sign out"}
            </button>
          </div>
        </Show>
      </div>

      {/* 2. OpenRouter — secondary */}
      <div class="mb-3 rounded-md border border-border-subtle bg-surface-0 p-3">
        <div class="mb-1 flex items-center gap-2">
          <span class="text-sm font-medium text-ink">OpenRouter</span>
          {experimentalBadge()}
        </div>
        <p class="mb-2 text-[11px] text-ink-faint">{"Access hundreds of models through one account, via OAuth."}</p>
        <Show
          when={openrouterConnected()}
          fallback={
            <div class="flex items-center gap-2">
              <button
                onClick={props.onOpenrouterConnect}
                disabled={props.openrouterConnecting()}
                class="flex-1 rounded-md border border-border-subtle bg-surface-0 p-2 text-sm text-ink hover:bg-surface-2 hover:border-accent/40 transition-colors disabled:opacity-50"
              >
                {props.openrouterConnecting()
                  ? "Connecting…"
                  : "Connect"}
              </button>
              <Show when={props.openrouterConnecting()}>
                <button
                  onClick={props.onOpenrouterCancel}
                  class="shrink-0 text-xs text-ink-muted hover:text-ink hover:underline"
                >
                  {"Cancel"}
                </button>
              </Show>
            </div>
          }
        >
          <div class="flex items-center justify-between rounded-md border border-border-subtle bg-surface-0 p-2 text-sm">
            <span class="flex items-center gap-1.5 text-ink">
              <Icon name="check-circle" class="h-3.5 w-3.5 text-green-500" />
              {"Connected"}
            </span>
            <button
              onClick={() => props.onDisconnectProvider("openrouter")}
              class="ml-2 shrink-0 text-xs text-ink-muted hover:text-ink hover:underline"
            >
              {"Disconnect"}
            </button>
          </div>
        </Show>
      </div>

      <Show when={props.providerError()}>
        <div class="mb-2 text-sm text-red-400">{props.providerError()}</div>
      </Show>

      {/* 3. Other connected catalog providers + the catalog itself */}
      <For each={otherProviders()}>
        {([id, info]) => (
          <div class="mb-2 flex items-center justify-between rounded-md border border-border-subtle bg-surface-0 p-2 text-sm">
            <span class="flex items-center gap-2 truncate text-ink">
              {info.label ?? id}
              {experimentalBadge()}
            </span>
            <button
              onClick={() => props.onDisconnectProvider(id)}
              class="ml-2 shrink-0 text-xs text-ink-muted hover:text-ink hover:underline"
            >
              {"Disconnect"}
            </button>
          </div>
        )}
      </For>

      <div class="mb-3">
        <button
          onClick={props.onOpenProviderCatalog}
          class="flex w-full items-center gap-2 rounded-md border border-border-subtle bg-surface-0 p-2 text-sm text-ink hover:bg-surface-2 hover:border-accent/40 transition-colors"
        >
          <Icon name="layers" class="h-4 w-4 shrink-0" />
          <span>{"More providers…"}</span>
        </button>
      </div>

      {/* Support */}
      <div class="mb-3">
        <button
          onClick={props.openSupportUrl}
          class="flex items-center gap-2 w-full rounded-md border border-border-subtle bg-surface-0 p-2 text-sm text-ink hover:bg-surface-2 hover:border-accent/40 transition-colors"
        >
          <Icon name="speech-balloon-alt" class="h-4 w-4 shrink-0" />
          <span>{"Support"}</span>
        </button>
      </div>
    </>
  );
};
