import { Component, For, Show, createMemo, createSignal, onCleanup, onMount, type Accessor } from "solid-js";
import { Icon } from "./Icon";
import {
  probeCustomProvider,
  saveCustomProvider,
  type ConnectedProviderInfo,
  type ProviderProtocol,
} from "../lib/ipc";

interface CustomProviderModalProps {
  /** Id of the custom provider being edited; null when adding a new one. */
  providerId: string | null;
  providers: Accessor<Record<string, ConnectedProviderInfo>>;
  onClose: () => void;
  /** Called after a successful save so the app refreshes provider state and
   * model groups. */
  onChanged: () => Promise<void> | void;
  /** Removes the provider being edited. The app owns this because removing a
   * provider also resets the model slots that pointed at it. */
  onRemove: (providerId: string) => Promise<void> | void;
}

const PROTOCOLS: { id: ProviderProtocol; label: string }[] = [
  { id: "openai", label: "OpenAI-compatible" },
  { id: "anthropic", label: "Anthropic-compatible" },
];

/** One model id per line; commas work too, since that is how people paste them. */
export function parseModelList(text: string): string[] {
  const seen = new Set<string>();
  const out: string[] = [];
  for (const raw of text.split(/[\n,]+/)) {
    const id = raw.trim();
    if (id && !seen.has(id)) {
      seen.add(id);
      out.push(id);
    }
  }
  return out;
}

/** Where chat requests will actually land for a base URL, so a missing or
 * doubled "/v1" is visible before saving rather than as a 404 mid-chat.
 * Mirrors the backend: OpenAI appends to the base as given; the Anthropic
 * client supplies "/v1" itself, with or without one on the base. */
export function requestUrlPreview(baseUrl: string, protocol: ProviderProtocol): string {
  const base = baseUrl.trim().replace(/\/+$/, "");
  if (!base) return "";
  if (protocol === "anthropic") return `${base.replace(/\/v1$/, "")}/v1/messages`;
  return `${base}/chat/completions`;
}

/** A context window as typed: digits, optionally with a "k" ("32k", "128 K").
 * Empty or anything else is null, which the backend reads as "not set". */
export function parseContextWindow(text: string): number | null {
  const m = /^(\d+(?:\.\d+)?)\s*(k)?$/i.exec(text.trim().replace(/[,_\s]/g, ""));
  if (!m) return null;
  const tokens = Math.round(Number(m[1]) * (m[2] ? 1000 : 1));
  return tokens > 0 ? tokens : null;
}

const inputClass =
  "w-full rounded-md border border-border-subtle bg-surface-0 p-2 text-sm text-ink placeholder:text-ink-muted focus:border-accent focus:outline-none focus:ring-1 focus:ring-accent";

/** Add or edit a provider that is not in the models.dev catalog: a localhost
 * server (Ollama, LM Studio, llama.cpp), a company LiteLLM proxy, any endpoint
 * that speaks the OpenAI or Anthropic wire protocol. */
export const CustomProviderModal: Component<CustomProviderModalProps> = (props) => {
  // Read once: the form is a draft of the entry as it was when the modal opened.
  const editingId = props.providerId;
  const existing = editingId ? props.providers()[editingId] : undefined;

  const [name, setName] = createSignal(existing?.label ?? "");
  const [protocol, setProtocol] = createSignal<ProviderProtocol>(
    existing?.protocol === "anthropic" ? "anthropic" : "openai",
  );
  const [baseUrl, setBaseUrl] = createSignal(existing?.baseUrl ?? "");
  const [apiKey, setApiKey] = createSignal("");
  const [modelsText, setModelsText] = createSignal((existing?.models ?? []).join("\n"));
  const [contextText, setContextText] = createSignal(
    existing?.contextWindow ? String(existing.contextWindow) : "",
  );
  const [saving, setSaving] = createSignal(false);
  const [fetching, setFetching] = createSignal(false);
  const [error, setError] = createSignal<string | null>(null);
  const [fetchedCount, setFetchedCount] = createSignal<number | null>(null);

  onMount(() => {
    const onKeyDown = (e: KeyboardEvent) => {
      if (e.key === "Escape") props.onClose();
    };
    document.addEventListener("keydown", onKeyDown);
    onCleanup(() => document.removeEventListener("keydown", onKeyDown));
  });

  const preview = createMemo(() => requestUrlPreview(baseUrl(), protocol()));
  const canSubmit = () => Boolean(name().trim() && baseUrl().trim()) && !saving();

  const doFetchModels = async () => {
    setFetching(true);
    setError(null);
    setFetchedCount(null);
    try {
      const models = await probeCustomProvider(baseUrl().trim(), protocol(), apiKey().trim(), editingId);
      setModelsText(models.join("\n"));
      setFetchedCount(models.length);
    } catch (e) {
      setError(`Could not fetch models: ${String(e)}`);
    } finally {
      setFetching(false);
    }
  };

  const doSave = async () => {
    if (!canSubmit()) return;
    setSaving(true);
    setError(null);
    try {
      await saveCustomProvider({
        providerId: editingId,
        name: name().trim(),
        baseUrl: baseUrl().trim(),
        protocol: protocol(),
        apiKey: apiKey().trim() || null,
        models: parseModelList(modelsText()),
        contextWindow: parseContextWindow(contextText()),
      });
      await props.onChanged();
      props.onClose();
    } catch (e) {
      setError(`Could not save: ${String(e)}`);
    } finally {
      setSaving(false);
    }
  };

  const doRemove = async () => {
    if (!editingId) return;
    await props.onRemove(editingId);
    props.onClose();
  };

  return (
    <div class="fixed inset-0 z-50 flex items-center justify-center bg-black/40" onClick={props.onClose}>
      <div
        class="flex max-h-[85vh] w-[480px] max-w-[92vw] flex-col overflow-hidden rounded-lg border border-border-subtle bg-surface-1 shadow-modal"
        onClick={(e) => e.stopPropagation()}
      >
        <div class="flex items-center justify-between border-b border-border-subtle px-4 py-3">
          <div class="flex items-center gap-2">
            <span class="text-sm font-semibold text-ink">
              {editingId ? "Edit custom provider" : "Add custom provider"}
            </span>
            <span class="rounded border border-amber-500/40 bg-amber-500/10 px-1.5 py-px text-[10px] text-amber-600">
              {"Experimental"}
            </span>
          </div>
          <button onClick={props.onClose} class="text-ink-muted hover:text-ink" title={"Cancel"}>
            <Icon name="x" class="h-4 w-4" />
          </button>
        </div>

        <div class="min-h-0 flex-1 overflow-y-auto p-4">
          <p class="mb-3 text-[11px] text-ink-faint">
            {"Any endpoint that speaks the OpenAI or Anthropic API: a local server (Ollama, LM Studio, llama.cpp), a LiteLLM proxy, a company gateway."}
          </p>

          <label class="mb-1 block text-xs text-ink-muted">{"Name"}</label>
          <input
            type="text"
            data-field="name"
            value={name()}
            onInput={(e) => setName(e.currentTarget.value)}
            placeholder={"LiteLLM"}
            class={`mb-3 ${inputClass}`}
          />

          <label class="mb-1 block text-xs text-ink-muted">{"API format"}</label>
          <div class="mb-3 flex gap-2">
            <For each={PROTOCOLS}>
              {(p) => (
                <button
                  type="button"
                  data-protocol={p.id}
                  aria-pressed={protocol() === p.id}
                  onClick={() => setProtocol(p.id)}
                  class="flex-1 rounded-md border p-2 text-sm transition-colors"
                  classList={{
                    "border-accent bg-accent/10 text-accent": protocol() === p.id,
                    "border-border-subtle bg-surface-0 text-ink hover:bg-surface-2": protocol() !== p.id,
                  }}
                >
                  {p.label}
                </button>
              )}
            </For>
          </div>

          <label class="mb-1 block text-xs text-ink-muted">{"Base URL"}</label>
          <input
            type="text"
            data-field="base-url"
            value={baseUrl()}
            onInput={(e) => setBaseUrl(e.currentTarget.value)}
            placeholder={"http://localhost:4000/v1"}
            class={`mb-1 ${inputClass}`}
          />
          <p class="mb-3 min-h-[1rem] break-all text-[11px] text-ink-faint" data-request-preview>
            <Show when={preview()}>{`Requests go to ${preview()}`}</Show>
          </p>

          <label class="mb-1 block text-xs text-ink-muted">{"API key (optional)"}</label>
          <input
            type="password"
            value={apiKey()}
            onInput={(e) => setApiKey(e.currentTarget.value)}
            placeholder={
              existing?.hasApiKey ? "Leave blank to keep the saved key" : "Leave blank if the server needs none"
            }
            class={`mb-3 ${inputClass}`}
          />

          <div class="mb-1 flex items-center justify-between">
            <label class="block text-xs text-ink-muted">{"Models"}</label>
            <button
              type="button"
              onClick={() => void doFetchModels()}
              disabled={fetching() || !baseUrl().trim()}
              class="text-xs text-accent hover:underline disabled:opacity-50 disabled:hover:no-underline"
            >
              {fetching() ? "Fetching…" : "Fetch models"}
            </button>
          </div>
          <textarea
            value={modelsText()}
            onInput={(e) => setModelsText(e.currentTarget.value)}
            rows={5}
            placeholder={"One model id per line.\nLeave empty to use everything the server lists."}
            class={`mb-1 font-mono text-xs ${inputClass}`}
          />
          <Show when={fetchedCount() !== null}>
            <p class="mb-2 text-xs text-green-500">{`Found ${String(fetchedCount())} models.`}</p>
          </Show>

          <label class="mb-1 mt-2 block text-xs text-ink-muted">{"Context window (optional)"}</label>
          <input
            type="text"
            inputmode="numeric"
            data-field="context-window"
            value={contextText()}
            onInput={(e) => setContextText(e.currentTarget.value)}
            placeholder={"32768"}
            class={`mb-1 ${inputClass}`}
          />
          <p class="mb-3 text-[11px] text-ink-faint">
            {"Tokens the server accepts per request. A session hands off and compacts relative to it; left blank, it assumes 200k unless the server reports otherwise."}
          </p>
          <Show when={error()}>
            <p class="mb-2 text-sm text-red-400">{error()}</p>
          </Show>
        </div>

        <div class="flex items-center gap-2 border-t border-border-subtle px-4 py-3">
          <button
            onClick={() => void doSave()}
            disabled={!canSubmit()}
            class="rounded-md bg-accent px-3 py-1.5 text-sm font-semibold text-white hover:opacity-90 disabled:opacity-50"
          >
            {saving() ? "Saving…" : editingId ? "Save changes" : "Add provider"}
          </button>
          <button
            onClick={props.onClose}
            class="rounded-md border border-border-subtle bg-surface-0 px-3 py-1.5 text-sm text-ink hover:bg-surface-2"
          >
            {"Cancel"}
          </button>
          <Show when={editingId}>
            <button
              onClick={() => void doRemove()}
              class="ml-auto text-xs text-ink-muted hover:text-red-400 hover:underline"
            >
              {"Remove provider"}
            </button>
          </Show>
        </div>
      </div>
    </div>
  );
};
