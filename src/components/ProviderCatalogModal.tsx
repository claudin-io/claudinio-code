import { Component, For, Show, createMemo, createSignal, onCleanup, onMount, type Accessor } from "solid-js";
import { t } from "../lib/grill-me";
import { Icon } from "./Icon";
import {
  connectProvider,
  disconnectProvider,
  fetchProviderCatalog,
  openExternalUrl,
  type CatalogProvider,
  type ConnectedProviderInfo,
} from "../lib/ipc";

interface ProviderCatalogModalProps {
  providers: Accessor<Record<string, ConnectedProviderInfo>>;
  onClose: () => void;
  /** Called after a successful connect/disconnect so the app refreshes
   * provider state and model groups. */
  onChanged: () => Promise<void> | void;
}

function formatPricePerMtok(v: number | null | undefined): string {
  if (v == null) return "—";
  return `$${v}`;
}

function formatContext(v: number | null | undefined): string {
  if (v == null) return "—";
  return v >= 1000 ? `${Math.round(v / 1000)}k` : String(v);
}

/** Full models.dev catalog: pick a provider, paste an API key, connect.
 * OpenRouter is excluded — it has its own OAuth card in Settings › Account. */
export const ProviderCatalogModal: Component<ProviderCatalogModalProps> = (props) => {
  const [catalog, setCatalog] = createSignal<CatalogProvider[]>([]);
  const [loading, setLoading] = createSignal(true);
  const [loadError, setLoadError] = createSignal<string | null>(null);
  const [query, setQuery] = createSignal("");
  const [selectedId, setSelectedId] = createSignal<string | null>(null);
  const [apiKey, setApiKey] = createSignal("");
  const [baseUrl, setBaseUrl] = createSignal("");
  const [connecting, setConnecting] = createSignal(false);
  const [connectError, setConnectError] = createSignal<string | null>(null);
  const [connectSuccess, setConnectSuccess] = createSignal<number | null>(null);

  const load = async (force: boolean) => {
    setLoading(true);
    setLoadError(null);
    try {
      const result = await fetchProviderCatalog(force);
      setCatalog((result.providers ?? []).filter((p) => p.id !== "openrouter"));
    } catch (e) {
      setLoadError(String(e));
    } finally {
      setLoading(false);
    }
  };

  onMount(() => {
    void load(false);
    const onKeyDown = (e: KeyboardEvent) => {
      if (e.key === "Escape") props.onClose();
    };
    document.addEventListener("keydown", onKeyDown);
    onCleanup(() => document.removeEventListener("keydown", onKeyDown));
  });

  const isConnected = (id: string) => Boolean(props.providers()[id]?.connected);

  const filtered = createMemo(() => {
    const q = query().trim().toLowerCase();
    const list = q
      ? catalog().filter(
          (p) => p.id.toLowerCase().includes(q) || p.name.toLowerCase().includes(q),
        )
      : catalog();
    // Connected providers surface first, then alphabetical (catalog order).
    return [...list].sort((a, b) => Number(isConnected(b.id)) - Number(isConnected(a.id)));
  });

  const selected = createMemo(() => catalog().find((p) => p.id === selectedId()) ?? null);

  const select = (p: CatalogProvider) => {
    setSelectedId(p.id);
    setApiKey("");
    setBaseUrl(props.providers()[p.id]?.baseUrl ?? p.api);
    setConnectError(null);
    setConnectSuccess(null);
  };

  const doConnect = async () => {
    const p = selected();
    if (!p) return;
    setConnecting(true);
    setConnectError(null);
    setConnectSuccess(null);
    try {
      const models = await connectProvider(p.id, apiKey(), baseUrl() || undefined);
      setConnectSuccess(models.length);
      setApiKey("");
      await props.onChanged();
    } catch (e) {
      setConnectError(String(e));
    } finally {
      setConnecting(false);
    }
  };

  const doDisconnect = async () => {
    const p = selected();
    if (!p) return;
    setConnectError(null);
    setConnectSuccess(null);
    try {
      await disconnectProvider(p.id);
      await props.onChanged();
    } catch (e) {
      setConnectError(String(e));
    }
  };

  return (
    <div class="fixed inset-0 z-50 flex items-center justify-center bg-black/40" onClick={props.onClose}>
      <div
        class="flex h-[70vh] w-[720px] max-w-[92vw] flex-col overflow-hidden rounded-lg border border-border-subtle bg-surface-1 shadow-modal"
        onClick={(e) => e.stopPropagation()}
      >
        {/* Header */}
        <div class="flex items-center justify-between border-b border-border-subtle px-4 py-3">
          <div class="flex items-center gap-2">
            <span class="text-sm font-semibold text-ink">{t("providerCatalog.title")}</span>
            <span class="rounded border border-amber-500/40 bg-amber-500/10 px-1.5 py-px text-[10px] text-amber-600">
              {t("settings.providers.experimental")}
            </span>
          </div>
          <button onClick={props.onClose} class="text-ink-muted hover:text-ink" title={t("app.config.cancel")}>
            <Icon name="x" class="h-4 w-4" />
          </button>
        </div>

        <Show
          when={!loading()}
          fallback={
            <div class="flex flex-1 items-center justify-center text-sm text-ink-muted">
              {t("providerCatalog.loading")}
            </div>
          }
        >
          <Show
            when={!loadError()}
            fallback={
              <div class="flex flex-1 flex-col items-center justify-center gap-3">
                <p class="text-sm text-red-400">{t("providerCatalog.loadError")}</p>
                <button
                  onClick={() => void load(true)}
                  class="rounded-md border border-border-subtle bg-surface-0 px-3 py-1.5 text-sm text-ink hover:bg-surface-2"
                >
                  {t("providerCatalog.retry")}
                </button>
              </div>
            }
          >
            <div class="flex min-h-0 flex-1">
              {/* Left: search + provider list */}
              <div class="flex w-56 shrink-0 flex-col border-r border-border-subtle">
                <div class="border-b border-border-subtle p-2">
                  <input
                    type="text"
                    value={query()}
                    onInput={(e) => setQuery(e.currentTarget.value)}
                    placeholder={t("providerCatalog.search")}
                    class="w-full rounded border border-border-subtle bg-surface-0 px-2 py-1 text-sm text-ink placeholder:text-ink-muted focus:border-accent focus:outline-none"
                  />
                </div>
                <div class="min-h-0 flex-1 overflow-y-auto py-1">
                  <Show
                    when={filtered().length > 0}
                    fallback={<p class="px-3 py-2 text-xs text-ink-faint">{t("providerCatalog.empty")}</p>}
                  >
                    <For each={filtered()}>
                      {(p) => (
                        <button
                          type="button"
                          onClick={() => select(p)}
                          class="flex w-full items-center justify-between gap-2 px-3 py-1.5 text-left text-sm hover:bg-surface-2"
                          classList={{
                            "bg-surface-2 text-accent": selectedId() === p.id,
                            "text-ink": selectedId() !== p.id,
                          }}
                        >
                          <span class="truncate">{p.name}</span>
                          <Show when={isConnected(p.id)}>
                            <Icon name="check-circle" class="h-3.5 w-3.5 shrink-0 text-green-500" />
                          </Show>
                        </button>
                      )}
                    </For>
                  </Show>
                </div>
              </div>

              {/* Right: provider detail + connect form */}
              <div class="min-h-0 flex-1 overflow-y-auto p-4">
                <Show
                  when={selected()}
                  fallback={<p class="text-sm text-ink-faint">{t("providerCatalog.pickProvider")}</p>}
                >
                  {(p) => (
                    <>
                      <div class="mb-1 flex items-center gap-2">
                        <span class="text-sm font-semibold text-ink">{p().name}</span>
                        <Show when={isConnected(p().id)}>
                          <span class="flex items-center gap-1 text-[11px] text-green-500">
                            <Icon name="check-circle" class="h-3 w-3" />
                            {t("settings.providers.connected")}
                          </span>
                        </Show>
                      </div>
                      <Show when={p().doc}>
                        <button
                          onClick={() => openExternalUrl(p().doc!)}
                          class="mb-2 text-xs text-accent hover:underline"
                        >
                          {t("providerCatalog.docs")}
                        </button>
                      </Show>
                      <Show when={p().env.length > 0}>
                        <p class="mb-3 text-[11px] text-ink-faint">
                          {t("providerCatalog.envHint", p().env[0])}
                        </p>
                      </Show>

                      <label class="mb-1 block text-xs text-ink-muted">{t("providerCatalog.baseUrl")}</label>
                      <input
                        type="text"
                        value={baseUrl()}
                        onInput={(e) => setBaseUrl(e.currentTarget.value)}
                        class="mb-3 w-full rounded-md border border-border-subtle bg-surface-0 p-2 text-sm text-ink focus:border-accent focus:outline-none focus:ring-1 focus:ring-accent"
                      />

                      <label class="mb-1 block text-xs text-ink-muted">{t("providerCatalog.apiKey")}</label>
                      <input
                        type="password"
                        value={apiKey()}
                        onInput={(e) => setApiKey(e.currentTarget.value)}
                        placeholder={t("providerCatalog.apiKeyPlaceholder")}
                        class="mb-3 w-full rounded-md border border-border-subtle bg-surface-0 p-2 text-sm text-ink placeholder:text-ink-muted focus:border-accent focus:outline-none focus:ring-1 focus:ring-accent"
                      />

                      <div class="mb-3 flex items-center gap-2">
                        <button
                          onClick={() => void doConnect()}
                          disabled={connecting() || !apiKey().trim()}
                          class="rounded-md bg-accent px-3 py-1.5 text-sm font-semibold text-white hover:opacity-90 disabled:opacity-50"
                        >
                          {connecting() ? t("settings.providers.connecting") : t("settings.providers.connect")}
                        </button>
                        <Show when={isConnected(p().id)}>
                          <button
                            onClick={() => void doDisconnect()}
                            class="rounded-md border border-border-subtle bg-surface-0 px-3 py-1.5 text-sm text-ink hover:bg-surface-2"
                          >
                            {t("settings.providers.disconnect")}
                          </button>
                        </Show>
                      </div>

                      <Show when={connectSuccess() !== null}>
                        <p class="mb-2 text-sm text-green-500">
                          {t("providerCatalog.connectSuccess", String(connectSuccess()))}
                        </p>
                      </Show>
                      <Show when={connectError()}>
                        <p class="mb-2 text-sm text-red-400">{t("providerCatalog.connectError", connectError()!)}</p>
                      </Show>

                      <p class="mb-1 text-xs text-ink-muted">
                        {t("providerCatalog.models", String(p().models.length))}
                      </p>
                      <div class="overflow-hidden rounded-md border border-border-subtle">
                        <For each={p().models.slice(0, 50)}>
                          {(m) => (
                            <div class="flex items-center justify-between gap-2 border-b border-border-subtle bg-surface-0 px-2 py-1 text-[11px] last:border-b-0">
                              <span class="truncate text-ink">{m.id}</span>
                              <span class="shrink-0 font-mono text-ink-faint">
                                {formatContext(m.context)} · {formatPricePerMtok(m.costInput)}/
                                {formatPricePerMtok(m.costOutput)}
                              </span>
                            </div>
                          )}
                        </For>
                      </div>
                    </>
                  )}
                </Show>
              </div>
            </div>
          </Show>
        </Show>
      </div>
    </div>
  );
};
