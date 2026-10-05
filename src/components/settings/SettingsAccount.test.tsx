import { describe, it, expect, vi, afterEach } from "vitest";
import { render } from "solid-js/web";
import { createSignal } from "solid-js";
import type { ConnectedProviderInfo } from "../../lib/ipc";

vi.mock("../Icon", () => ({
  Icon: (props: { name: string; class?: string }) => (
    <span data-testid={`icon-${props.name}`} class={props.class ?? ""} />
  ),
}));

import { SettingsAccount } from "./SettingsAccount";

let dispose: (() => void) | undefined;
let host: HTMLDivElement | undefined;

afterEach(() => {
  dispose?.();
  host?.remove();
  dispose = undefined;
  host = undefined;
});

const onDisconnectProvider = vi.fn();
const onOpenProviderCatalog = vi.fn();
const onOpenCustomProvider = vi.fn();

function mount(providers: Record<string, ConnectedProviderInfo>) {
  vi.clearAllMocks();
  host = document.createElement("div");
  document.body.appendChild(host);
  const [prov] = createSignal(providers);
  const [apiKey, setApiKey] = createSignal("");
  dispose = render(
    () => (
      <SettingsAccount
        accountLogin={() => "victor"}
        hasApiKey={() => true}
        loggingIn={() => false}
        configApiKey={apiKey}
        setConfigApiKey={setApiKey}
        settingsApiKeyError={() => null}
        doLogin={() => {}}
        doLogout={() => {}}
        openSupportUrl={() => {}}
        providers={prov}
        openrouterConnecting={() => false}
        providerError={() => null}
        onOpenrouterConnect={() => {}}
        onOpenrouterCancel={() => {}}
        onDisconnectProvider={onDisconnectProvider}
        onOpenProviderCatalog={onOpenProviderCatalog}
        onOpenCustomProvider={onOpenCustomProvider}
      />
    ),
    host,
  );
  return host;
}

const row = (el: HTMLElement, id: string) => el.querySelector<HTMLElement>(`[data-provider="${id}"]`)!;
const buttonIn = (el: HTMLElement, label: string) =>
  Array.from(el.querySelectorAll("button")).find((b) => b.textContent === label);

describe("SettingsAccount providers", () => {
  it("offers to add a custom provider next to the catalog", () => {
    const el = mount({});
    buttonIn(el, "Add custom provider…")!.click();
    expect(onOpenCustomProvider).toHaveBeenCalledWith();
    buttonIn(el, "More providers…")!.click();
    expect(onOpenProviderCatalog).toHaveBeenCalled();
  });

  it("a custom provider can be edited or removed from its row", () => {
    const el = mount({
      litellm: { connected: true, baseUrl: "http://localhost:4000/v1", label: "LiteLLM", custom: true },
    });
    const litellm = row(el, "litellm");
    expect(litellm.textContent).toContain("LiteLLM");
    expect(litellm.textContent).toContain("Custom");
    buttonIn(litellm, "Edit")!.click();
    expect(onOpenCustomProvider).toHaveBeenCalledWith("litellm");
    buttonIn(litellm, "Remove")!.click();
    expect(onDisconnectProvider).toHaveBeenCalledWith("litellm");
  });

  it("a catalog provider keeps a plain disconnect and no edit", () => {
    const el = mount({
      deepseek: { connected: true, baseUrl: "https://api.deepseek.com", label: "DeepSeek" },
    });
    const deepseek = row(el, "deepseek");
    expect(deepseek.textContent).not.toContain("Custom");
    expect(buttonIn(deepseek, "Edit")).toBeUndefined();
    buttonIn(deepseek, "Disconnect")!.click();
    expect(onDisconnectProvider).toHaveBeenCalledWith("deepseek");
  });

  it("OpenRouter stays in its own card, not in the list", () => {
    const el = mount({ openrouter: { connected: true, baseUrl: "https://openrouter.ai/api/v1" } });
    expect(el.querySelector('[data-provider="openrouter"]')).toBeNull();
  });
});
